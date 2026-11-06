#!/usr/bin/env bash
# Publishes a Windows build as the app update everyone gets (#82): `just publish-update RUN_ID`, run on the deploying
# machine after `just deploy` (the server's /updates route must be live) and a green release-windows run.
#
#   1. the run must be release-windows, successful, on main or a v* tag;
#   2. its pulse-windows-installer artifact must hold exactly one Pulse_X.Y.Z_x64-setup.exe and its .sig, and the
#      signature must verify (minisign) against the public key in client/src-tauri/tauri.conf.json; a version the
#      signature binds must be X.Y.Z, and one is required when the app requires it (requireSignedVersion, #232);
#   3. X.Y.Z must be newer than what the server offers now (a rerun of an old run is refused; a latest.json that's
#      there but can't be read refuses too, since it can't be told apart from a newer one);
#   4. the installer goes up first (to .incoming/, then moved into place), the manifest last, so latest.json never
#      names a missing file; then everything but this installer and the one before it is deleted;
#   5. checks: the server's copy hashes the same, the manifest parses, and the route answers 401 without a session.
# Nothing is uploaded unless 1-3 and the route check pass, and the download is deleted whatever happens. Needs gh
# (logged in), jq, minisign, curl, ssh, scp.
#
# PUBLISH_UPDATE_DIR=DIR (the self-test) uses DIR as the server's updates dir instead of ssh/scp.
set -euo pipefail

gh_repo=GregHofmeyr/pulse-app
prod=pulse@pulse
# /srv/pulse/data is the server container's /data (deploy/compose.yml); the server reads /data/updates
srv_updates=/srv/pulse/data/updates
PULSE_DOMAIN=${PULSE_DOMAIN:-pulsechat.co.za}
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

refuse() {
  {
    echo
    echo "REFUSED: $*"
  } >&2
  exit 1
}

# version_of FILE: X.Y.Z from .../Pulse_X.Y.Z_x64-setup.exe, else fails
version_of() {
  local base=${1##*/}
  if [[ $base =~ ^Pulse_([0-9]+\.[0-9]+\.[0-9]+)_x64-setup\.exe$ ]]; then
    echo "${BASH_REMATCH[1]}"
  else
    echo "not an installer name: '$base'" >&2
    return 1
  fi
}

# verify_sig FILE SIGFILE PUBKEY_B64: Tauri wraps both minisign files in base64; minisign checks the signature
verify_sig() {
  local file=$1 sigfile=$2 pubkey=$3 d rc=0
  d=$(mktemp -d)
  if printf '%s' "$pubkey" | base64 -d >"$d/key.pub" 2>/dev/null && base64 -d <"$sigfile" >"$d/file.sig" 2>/dev/null; then
    minisign -V -q -p "$d/key.pub" -x "$d/file.sig" -m "$file" || rc=$?
  else
    echo "the signature or public key isn't base64" >&2
    rc=1
  fi
  rm -rf "$d"
  return "$rc"
}

# signed_version SIGFILE: the version a Tauri signature binds ('' = none). `tauri build` (and `tauri signer sign
# --app-version`) write it into minisign's trusted comment as `version:X.Y.Z`, which the global signature covers, so
# it only counts once verify_sig has passed. The app's updater refuses a manifest whose version differs from it, and,
# with requireSignedVersion, a signature without one (#232).
signed_version() {
  local comment field v
  local -a fields=()
  comment=$(base64 -d <"$1" 2>/dev/null | sed -n 's/^trusted comment: //p') || return 0
  IFS=$'\t' read -r -a fields <<<"${comment%%$'\n'*}" || true
  for field in "${fields[@]}"; do
    if [[ $field == version:* ]]; then
      v=${field#version:}
      echo "${v#v}"
      return 0
    fi
  done
}

# require_signed_version: "true" when the app refuses a signature that binds no version
# (plugins.updater.requireSignedVersion in tauri.conf.json)
require_signed_version() {
  if [ -n "${PUBLISH_UPDATE_DIR:-}" ] && [ -n "${PUBLISH_UPDATE_REQUIRE_SIGNED_VERSION:-}" ]; then
    echo "$PUBLISH_UPDATE_REQUIRE_SIGNED_VERSION"
  else
    jq -r '.plugins.updater.requireSignedVersion // false' "$here/../client/src-tauri/tauri.conf.json"
  fi
}

# newer_than NEW OLD: NEW is a higher version than OLD (an empty OLD = nothing served yet)
newer_than() {
  local new=$1 old=$2
  [ -z "$old" ] && return 0
  [ "$new" = "$old" ] && return 1
  [ "$(printf '%s\n%s\n' "$old" "$new" | sort -V | tail -n 1)" = "$new" ]
}

# write_manifest VERSION URL SIGFILE NOTES OUT: the updater's latest.json; the URL must be on our own origin (the app
# sends its session token with the download, and refuses another host anyway)
write_manifest() {
  local version=$1 url=$2 sigfile=$3 notes=$4 out=$5
  if [[ ! $url =~ ^https://${PULSE_DOMAIN//./\\.}/[^@]*$ ]]; then
    echo "the download URL must be on https://$PULSE_DOMAIN/: $url" >&2
    return 1
  fi
  jq -n --arg v "$version" --arg n "$notes" --arg d "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg u "$url" \
    --rawfile s "$sigfile" \
    '{version: $v, notes: $n, pub_date: $d, platforms: {"windows-x86_64": {url: $u, signature: ($s | rtrimstr("\n"))}}}' \
    >"$out"
}

# remote 'CMD': runs CMD in the server's updates dir (only names checked by version_of ever go into CMD)
remote() {
  if [ -n "${PUBLISH_UPDATE_DIR:-}" ]; then
    (cd "$PUBLISH_UPDATE_DIR" && bash -c "$1")
  else
    # shellcheck disable=SC2029 # expanded here on purpose: a fixed path and checked names
    ssh "$prod" "mkdir -p $srv_updates && cd $srv_updates && $1"
  fi
}

# upload LOCAL NAME: copies LOCAL to NAME in the server's updates dir
upload() {
  if [ -n "${PUBLISH_UPDATE_DIR:-}" ]; then
    # the self-test can swap the copy for one that cuts the file short
    "${PUBLISH_UPDATE_CP:-cp}" "$1" "$PUBLISH_UPDATE_DIR/$2"
  else
    scp -q "$1" "$prod:$srv_updates/$2"
  fi
}

# served_version: what the server's latest.json offers now ('' = nothing published yet). Fails when it can't tell: no
# answer from the server, or a latest.json that's there but unreadable or names no X.Y.Z (#233: taking that for
# "nothing" would let any version, even an older one, replace it).
served_version() {
  local raw v
  raw=$(remote 'if [ -e latest.json ]; then cat latest.json; else echo "<no manifest>"; fi') || {
    echo "couldn't read the server's latest.json" >&2
    return 1
  }
  [ "$raw" = "<no manifest>" ] && return 0
  v=$(jq -r '.version // empty' <<<"$raw" 2>/dev/null) || v=
  if [[ ! $v =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "the server's latest.json doesn't name a version" >&2
    return 1
  fi
  echo "$v"
}

# prune: keeps the installer the manifest names and the next-highest version; deletes every other Pulse_*.exe
prune() {
  local current keep_prev="" v name
  current=$(remote 'cat latest.json' | jq -r '.platforms["windows-x86_64"].url')
  current=$(version_of "${current##*/}") || refuse "the manifest names no installer, so nothing was pruned"
  local -a versions=() doomed=()
  while IFS= read -r name; do
    v=$(version_of "$name" 2>/dev/null) && versions+=("$v")
  done < <(remote 'ls -1A')
  for v in $(printf '%s\n' "${versions[@]}" | sort -V); do
    newer_than "$current" "$v" && keep_prev=$v
  done
  for v in "${versions[@]}"; do
    [ "$v" = "$current" ] || [ "$v" = "$keep_prev" ] || doomed+=("Pulse_${v}_x64-setup.exe")
  done
  if [ ${#doomed[@]} -gt 0 ]; then
    remote "rm -f -- ${doomed[*]}"
    echo "pruned: ${doomed[*]}"
  fi
}

# route_code: what /updates/latest.json answers without a session (401 = live and guarded)
route_code() {
  curl -s -o /dev/null -w '%{http_code}' --max-time 20 "https://$PULSE_DOMAIN/updates/latest.json" || true
}

publish_main() {
  local run_id=${1:-} info dl exe sig version served name pubkey signed local_sha remote_sha code
  [[ $run_id =~ ^[0-9]+$ ]] || refuse "a run id is digits (gh run list -w release-windows), got '$run_id'"

  info=$(gh run view "$run_id" --repo "$gh_repo" --json workflowName,conclusion,headBranch) ||
    refuse "couldn't look up run $run_id"
  [ "$(jq -r .workflowName <<<"$info")" = release-windows ] || refuse "run $run_id isn't release-windows: $info"
  [ "$(jq -r .conclusion <<<"$info")" = success ] || refuse "run $run_id didn't succeed: $info"
  [[ $(jq -r .headBranch <<<"$info") =~ ^(main|v[0-9][0-9A-Za-z.-]*)$ ]] || refuse "run $run_id isn't from main or a v* tag: $info"

  code=$(route_code)
  [ "$code" = 401 ] || refuse "https://$PULSE_DOMAIN/updates/latest.json answers $code without a session, not 401: deploy the server first"

  dl=$(mktemp -d)
  # on EXIT too: a refusal exits, and the download must go with it (#233)
  # shellcheck disable=SC2064 # $dl is fixed now
  trap "rm -rf '$dl'" RETURN EXIT
  gh run download "$run_id" --repo "$gh_repo" -n pulse-windows-installer -D "$dl" ||
    refuse "couldn't download run $run_id's pulse-windows-installer"
  local -a exes=()
  shopt -s nullglob
  exes=("$dl"/*.exe)
  shopt -u nullglob
  [ ${#exes[@]} -eq 1 ] || refuse "expected exactly one installer in the artifact, found ${#exes[@]}"
  exe=${exes[0]}
  sig=$exe.sig
  version=$(version_of "$exe") || refuse "unexpected installer name"
  [ -f "$sig" ] || refuse "the artifact has no ${exe##*/}.sig (is release-windows signing?)"
  name=${exe##*/}

  # the self-test's own key only goes with its fake server dir: a real publish always checks the app's key
  if [ -n "${PUBLISH_UPDATE_DIR:-}" ] && [ -n "${PUBLISH_UPDATE_PUBKEY:-}" ]; then
    pubkey=$PUBLISH_UPDATE_PUBKEY
  else
    pubkey=$(jq -r '.plugins.updater.pubkey // empty' "$here/../client/src-tauri/tauri.conf.json")
  fi
  [ -n "$pubkey" ] || refuse "no plugins.updater.pubkey in tauri.conf.json"
  verify_sig "$exe" "$sig" "$pubkey" || refuse "$name's signature doesn't verify against tauri.conf.json's public key"
  signed=$(signed_version "$sig")
  if [ -n "$signed" ]; then
    [ "$signed" = "$version" ] || refuse "$name's signature is for version $signed, not $version"
  elif [ "$(require_signed_version)" = true ]; then
    refuse "$name's signature binds no version, and the app requires one (requireSignedVersion): it would refuse this update"
  else
    echo "note: $name's signature binds no version (#232)" >&2
  fi

  served=$(served_version) || refuse "can't tell what the server offers now, so nothing was published"
  newer_than "$version" "$served" || refuse "$version isn't newer than the $served the server offers now (the updater never downgrades: publish a higher version)"

  echo "publishing $version (run $run_id; the server offers ${served:-nothing} now)"
  local_sha=$(sha256sum "$exe" | cut -d' ' -f1)
  remote 'mkdir -p .incoming'
  upload "$exe" ".incoming/$name" || { remote "rm -f .incoming/$name"; refuse "couldn't upload $name"; }
  # the server's copy is checked while it's still in .incoming: a cut-off upload never goes live
  remote_sha=$(remote "sha256sum .incoming/$name" | cut -d' ' -f1)
  if [ "$local_sha" != "$remote_sha" ]; then
    remote "rm -f .incoming/$name"
    refuse "the server's copy of $name doesn't match the download (sha256 ${remote_sha:-none}); nothing changed"
  fi
  remote "mv -f .incoming/$name $name"
  write_manifest "$version" "https://$PULSE_DOMAIN/updates/$name" "$sig" "Pulse $version" "$dl/latest.json"
  upload "$dl/latest.json" latest.json.tmp || refuse "couldn't upload latest.json; the server still offers ${served:-nothing}"
  remote 'mv -f latest.json.tmp latest.json' || refuse "couldn't put latest.json in place; the server still offers ${served:-nothing}"
  remote 'rmdir .incoming 2>/dev/null || true'
  # prune only once the server really offers this version (it keeps this installer and the one before)
  [ "$(served_version 2>/dev/null)" = "$version" ] || refuse "the server's latest.json doesn't offer $version after the move; nothing was pruned"
  prune

  remote 'cat latest.json' | jq -e '.platforms["windows-x86_64"].signature | length > 0' >/dev/null ||
    refuse "the server's latest.json doesn't parse"
  code=$(route_code)
  [ "$code" = 401 ] || refuse "https://$PULSE_DOMAIN/updates/latest.json answers $code without a session, not 401"
  echo "published $version"
}

if [ -z "${PUBLISH_UPDATE_LIB:-}" ]; then
  publish_main "$@"
fi
