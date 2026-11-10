#!/usr/bin/env bash
# Publishes the models the app downloads on first use (#77): `just publish-models`, run on the deploying machine after
# `just deploy` (the server's /models route must be live). Today that's one file: DeepFilterNet3's low-delay model
# ("Strong (low delay)" noise suppression), exactly as the pinned libDF checkout ships it.
#
#   1. the route must be live: /models/<name> answers 401 without a session;
#   2. the model comes from the libDF git checkout the client builds against (found with `cargo metadata`, at the rev
#      client/src-tauri/Cargo.toml pins), and its SHA-256 must be the one the app pins (LOW_DELAY_SHA256 in
#      client/src-tauri/src/models.rs; the app refuses any other file);
#   3. it goes up to models/.incoming/ first, is hashed there, and only then moved into place, so a cut-off upload never
#      goes live;
#   4. checks: the server's copy hashes the same, the route still answers 401 without a session, and, with
#      PUBLISH_MODELS_TOKEN=<a session token>, that the route serves exactly these bytes.
# Re-running it is safe (the same file again). Needs cargo, jq, sha256sum, curl, ssh, scp.
#
# PUBLISH_MODELS_DIR=DIR (the self-test) uses DIR as the server's updates dir instead of ssh/scp; with it,
# PUBLISH_MODELS_SRC and PUBLISH_MODELS_SHA stand in for the checkout's file and the app's hash.
set -euo pipefail

prod=pulse@pulse
# /srv/pulse/data is the server container's /data (deploy/compose.yml); the server reads /data/updates/models
srv_models=/srv/pulse/data/updates/models
PULSE_DOMAIN=${PULSE_DOMAIN:-pulsechat.co.za}
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$here/..
# the one name the server's allowlist serves (server/src/models.rs) and the app asks for (client/src-tauri/src/models.rs)
model=DeepFilterNet3_ll_onnx.tar.gz

refuse() {
  {
    echo
    echo "REFUSED: $*"
  } >&2
  exit 1
}

# pinned_sha: the SHA-256 the app checks the model against
pinned_sha() {
  if [ -n "${PUBLISH_MODELS_DIR:-}" ] && [ -n "${PUBLISH_MODELS_SHA:-}" ]; then
    echo "$PUBLISH_MODELS_SHA"
    return
  fi
  # rustfmt may wrap the constant onto two lines (`= ` then the string on the next line), so read the
  # declaration and the line after it and take the first 64-hex string.
  grep -A1 '^pub const LOW_DELAY_SHA256: &str' "$root/client/src-tauri/src/models.rs" | grep -oE '[0-9a-f]{64}' | head -n1
}

# libdf_rev: the DeepFilterNet git rev the client builds against
libdf_rev() {
  sed -n 's/^deep_filter = .*rev = "\([0-9a-f]\{40\}\)".*/\1/p' "$root/client/src-tauri/Cargo.toml"
}

# model_source: the model file in that rev's cargo checkout (cargo fetches it if it isn't there yet)
model_source() {
  if [ -n "${PUBLISH_MODELS_DIR:-}" ] && [ -n "${PUBLISH_MODELS_SRC:-}" ]; then
    echo "$PUBLISH_MODELS_SRC"
    return
  fi
  local rev manifest
  rev=$(libdf_rev)
  [ -n "$rev" ] || {
    echo "no deep_filter git rev in client/src-tauri/Cargo.toml" >&2
    return 1
  }
  manifest=$(cargo metadata --format-version 1 --manifest-path "$root/client/src-tauri/Cargo.toml" |
    jq -r --arg rev "$rev" '.packages[] | select(.name == "deep_filter" and ((.source // "") | contains($rev))) | .manifest_path' |
    head -n 1)
  [ -n "$manifest" ] || {
    echo "cargo metadata names no deep_filter at $rev" >&2
    return 1
  }
  # <checkout>/libDF/Cargo.toml → <checkout>/models/
  echo "$(dirname "$(dirname "$manifest")")/models/$model"
}

# remote 'CMD': runs CMD in the server's models dir (only fixed names ever go into CMD)
remote() {
  if [ -n "${PUBLISH_MODELS_DIR:-}" ]; then
    mkdir -p "$PUBLISH_MODELS_DIR/models"
    (cd "$PUBLISH_MODELS_DIR/models" && bash -c "$1")
  else
    # shellcheck disable=SC2029 # expanded here on purpose: a fixed path and a fixed name
    ssh "$prod" "mkdir -p $srv_models && cd $srv_models && $1"
  fi
}

# upload LOCAL NAME: copies LOCAL to NAME in the server's models dir
upload() {
  if [ -n "${PUBLISH_MODELS_DIR:-}" ]; then
    # the self-test can swap the copy for one that cuts the file short
    "${PUBLISH_MODELS_CP:-cp}" "$1" "$PUBLISH_MODELS_DIR/models/$2"
  else
    scp -q "$1" "$prod:$srv_models/$2"
  fi
}

# route_code: what /models/<model> answers without a session (401 = live and guarded)
route_code() {
  curl -s -o /dev/null -w '%{http_code}' --max-time 20 "https://$PULSE_DOMAIN/models/$model" || true
}

# served_sha TOKEN: the SHA-256 of what the route serves with that session ('' = it didn't)
served_sha() {
  local out
  out=$(mktemp)
  if curl -sf --max-time 300 -H "Authorization: Bearer $1" -o "$out" "https://$PULSE_DOMAIN/models/$model"; then
    sha256sum "$out" | cut -d' ' -f1
  fi
  rm -f "$out"
}

publish_main() {
  local code src want got remote_sha
  code=$(route_code)
  [ "$code" = 401 ] || refuse "https://$PULSE_DOMAIN/models/$model answers $code without a session, not 401: deploy the server first"

  want=$(pinned_sha)
  [[ $want =~ ^[0-9a-f]{64}$ ]] || refuse "no LOW_DELAY_SHA256 in client/src-tauri/src/models.rs"
  src=$(model_source) || refuse "couldn't find the libDF checkout"
  [ -f "$src" ] || refuse "no $model at $src"
  got=$(sha256sum "$src" | cut -d' ' -f1)
  [ "$got" = "$want" ] || refuse "$src hashes $got, not the $want the app pins"

  echo "publishing $model (sha256 $want)"
  remote 'mkdir -p .incoming'
  upload "$src" ".incoming/$model" || {
    remote "rm -f .incoming/$model"
    refuse "couldn't upload $model"
  }
  remote_sha=$(remote "sha256sum .incoming/$model" | cut -d' ' -f1)
  if [ "$remote_sha" != "$want" ]; then
    remote "rm -f .incoming/$model"
    refuse "the server's copy of $model hashes ${remote_sha:-nothing}, not $want; nothing changed"
  fi
  remote "mv -f .incoming/$model $model && rmdir .incoming 2>/dev/null || true"
  remote_sha=$(remote "sha256sum $model" | cut -d' ' -f1)
  [ "$remote_sha" = "$want" ] || refuse "the server's $model hashes ${remote_sha:-nothing} after the move"

  code=$(route_code)
  [ "$code" = 401 ] || refuse "https://$PULSE_DOMAIN/models/$model answers $code without a session, not 401"
  if [ -n "${PUBLISH_MODELS_TOKEN:-}" ]; then
    got=$(served_sha "$PUBLISH_MODELS_TOKEN")
    [ "$got" = "$want" ] || refuse "the route served ${got:-nothing} (sha256) with that session, not $want"
    echo "the route serves it (sha256 checked)"
  else
    echo "note: set PUBLISH_MODELS_TOKEN=<a session token> to check the route serves it too"
  fi
  echo "published $model"
}

if [ -z "${PUBLISH_MODELS_LIB:-}" ]; then
  publish_main "$@"
fi
