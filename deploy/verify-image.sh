#!/usr/bin/env bash
# Runs on the deploying machine, before anything touches the server: `just deploy TAG` / `just verify-image TAG`
# (verify-image.sh TAG), or `just deploy-branch SHA7` (verify-image.sh --branch SHA7).
#
# Resolves pulse-server:TAG to its multi-arch index digest without pulling the image, then checks that the digest carries
# a build provenance attestation whose signing certificate names this repo's release-server.yml:
#   - TAG mode: run on refs/heads/main (both the workflow identity and the source ref), on a GitHub-hosted runner;
#   - --branch mode: run on any branch (a hand-run build), which it prints; only <sha7> tags.
# For a <sha7> tag every platform image must also be labelled with that commit (the certificate records the commit the
# run started on, not necessarily the one it built: see release-server.yml). Every registry body is hashed and compared
# with the digest it was fetched by before anything in it is trusted.
# Prints the verified digest (sha256:…) on stdout and nothing else; everything else goes to stderr. Exit 1 = refused.
# Needs curl, jq, git, sha256sum (or shasum) and a logged-in gh (`gh auth login`). Never ssh.
set -euo pipefail

gh_repo=GregHofmeyr/pulse-app
registry_repo=greghofmeyr/pulse-server
image=ghcr.io/$registry_repo
workflow_path=.github/workflows/release-server.yml
# the certificate's SubjectAlternativeName: the signing workflow at the ref it ran from
identity_prefix='^https://github\.com/GregHofmeyr/pulse-app/\.github/workflows/release-server\.yml@'
main_identity="${identity_prefix}refs/heads/main\$"
branch_identity="${identity_prefix}refs/heads/.+\$"
index_types='application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json'
manifest_types='application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json'
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

refuse() {
  {
    echo
    echo "REFUSED: $*"
    echo "Nothing was deployed and the server was not contacted."
  } >&2
  exit 1
}

# is_digest VALUE: a canonical sha256 digest, the only form deploy.sh accepts
is_digest() { [[ $1 =~ ^sha256:[0-9a-f]{64}$ ]]; }
# is_tag VALUE: a registry tag (it also ends up in a remote command line, so nothing a shell would interpret)
is_tag() { [[ $1 =~ ^[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}$ ]]; }
# is_sha7 VALUE: a commit tag, the only kind release-server.yml publishes besides latest
is_sha7() { [[ $1 =~ ^[0-9a-f]{7}$ ]]; }

# check_ship_args TAG PIN: what `just _ship` accepts before it puts both into a remote command line. PIN is a digest, or
# --unverified with a <sha7> TAG (never a moving tag).
check_ship_args() {
  if ! is_tag "${1:-}"; then
    echo "'${1:-}' is not an image tag" >&2
    return 1
  fi
  if [ "${2:-}" = --unverified ]; then
    is_sha7 "$1" && return 0
    echo "an unverified deploy takes a 7-character commit, not '$1'" >&2
    return 1
  fi
  if ! is_digest "${2:-}"; then
    echo "'${2:-}' is not a sha256 digest" >&2
    return 1
  fi
}

# check_unverified_args TAG CONFIRM: `just deploy-unverified` takes a <sha7> (never latest) and CONFIRM=<that sha7>.
check_unverified_args() {
  if ! is_sha7 "${1:-}"; then
    echo "deploy-unverified takes a 7-character commit (an image from before provenance), not '${1:-}'" >&2
    return 1
  fi
  if [ "${2:-}" != "$1" ]; then
    echo "deploy-unverified needs CONFIRM=$1 set: CONFIRM=$1 just deploy-unverified $1" >&2
    return 1
  fi
}

# provenance_status SHA7 [GIT_DIR]: "predates" when that commit doesn't contain the first commit that made
# release-server.yml attest (its image can't have an attestation), "has" when it does, "unknown" when the commit isn't
# in the checkout. Found from history rather than a recorded sha, so it survives a squash or rebase merge. The history
# searched is origin/main's (what release-server.yml builds from), not HEAD's: a stale or unrelated local branch must not
# make a post-provenance commit look older. No origin/main (or not fetched) → "unknown", which offers no bypass.
provenance_status() {
  local dir=${2:-$here/..} commit first
  commit=$(git -C "$dir" rev-parse --verify --quiet "$1^{commit}" 2>/dev/null) || { echo unknown; return 0; }
  git -C "$dir" rev-parse --verify --quiet refs/remotes/origin/main >/dev/null 2>&1 || { echo unknown; return 0; }
  first=$(git -C "$dir" log refs/remotes/origin/main --format=%H --reverse -S'actions/attest-build-provenance@' \
    -- "$workflow_path" 2>/dev/null | head -n1 || true)
  if [ -z "$first" ]; then
    echo unknown
  elif git -C "$dir" merge-base --is-ancestor "$first" "$commit"; then
    echo has
  else
    echo predates
  fi
}

# refusal_hint TAG MODE: what to do about an image without a valid attestation. The unverified bypass is offered only
# for a commit from before provenance; anything newer is exactly what the check is for.
refusal_hint() {
  local tag=$1 mode=$2
  if ! is_sha7 "$tag"; then
    echo "  '$tag' is a moving tag: if a release-server run is still going, wait for it; otherwise deploy a specific"
    echo "  build with \`just deploy <sha7>\`."
    return 0
  fi
  case $(provenance_status "$tag" "${VERIFY_IMAGE_GIT_DIR:-}") in
    predates)
      echo "  Commit $tag is from before release-server.yml attested its images, so it has none. To roll back to it on"
      echo "  purpose: CONFIRM=$tag just deploy-unverified $tag   (deploy/README.md, section 7)" ;;
    has)
      echo "  Commit $tag already has the attesting workflow, so its image should verify. Don't deploy it unverified:"
      echo "  find out why (did its release-server run fail at the attest step?)."
      if [ "$mode" = main ]; then
        echo "  If it is a hand-run branch build, \`just deploy-branch $tag\` checks it against its branch."
      fi ;;
    *)
      echo "  Commit $tag isn't in this checkout (git fetch?), so whether it predates provenance can't be told." ;;
  esac
}

# sha256_of FILE: the file's sha256 as sha256:<hex>
sha256_of() {
  local sum
  if command -v sha256sum >/dev/null; then sum=$(sha256sum "$1"); else sum=$(shasum -a 256 "$1"); fi
  echo "sha256:${sum%% *}"
}

# header_value FILE NAME: one HTTP header's value (NAME lower case), without parameters or the CR. awk with tolower
# rather than sed's GNU-only I flag, so it runs on macOS too.
header_value() {
  awk -v name="$2" '{ sub(/\r$/, "") } index($0, ":") { k = tolower(substr($0, 1, index($0, ":") - 1)); v = substr($0, index($0, ":") + 1); sub(/^ +/, "", v); sub(/ *;.*$/, "", v); if (k == name) out = v } END { print out }' "$1"
}

# Sourced by the self-test and the justfile with VERIFY_IMAGE_LIB=1: define the functions, run nothing.
[ -n "${VERIFY_IMAGE_LIB:-}" ] && return 0

mode=main
if [ "${1:-}" = --branch ]; then mode=branch; shift; fi
tag=${1:?usage: verify-image.sh [--branch] TAG}
is_tag "$tag" || refuse "'$tag' is not an image tag"
if [ "$mode" = branch ] && ! is_sha7 "$tag"; then
  refuse "deploy-branch takes the 7-character commit of a hand-run branch build, not '$tag'"
fi
for tool in curl jq gh git; do
  command -v "$tool" >/dev/null || refuse "$tool is not installed (or not on PATH)"
done
command -v sha256sum >/dev/null || command -v shasum >/dev/null || refuse "neither sha256sum nor shasum is installed"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Anonymous pull token (the package is public): enough to read manifests, and no local docker login needed.
token=$(curl -fsS "https://ghcr.io/token?scope=repository:$registry_repo:pull" | jq -r '.token // ""') ||
  refuse "could not get a registry token from ghcr.io"
[ -n "$token" ] || refuse "ghcr.io gave no registry token"

# fetch DIGEST ACCEPT FILE: GET manifests/DIGEST (or a blob: pass blobs/DIGEST as the path) into FILE and check the
# body hashes to DIGEST, so nothing read from it can have been swapped in transit or by the registry.
fetch() {
  local path=$1 accept=$2 file=$3 want=${1#*/}
  curl -fsSL -H "Authorization: Bearer $token" -H "Accept: $accept" -o "$file" \
    "https://ghcr.io/v2/$registry_repo/$path" || refuse "could not read $path from the registry"
  [ "$(sha256_of "$file")" = "$want" ] || refuse "the registry's $path doesn't hash to $want"
}

# The digest of what the tag names, as the registry serves it: the multi-arch index, the same thing the server's
# docker pull resolves the tag to (and the subject the publish job attests).
curl -fsSI -H "Authorization: Bearer $token" -H "Accept: $index_types, $manifest_types" \
  "https://ghcr.io/v2/$registry_repo/manifests/$tag" >"$work/head" ||
  refuse "$image:$tag not found in the registry (mistyped sha, or its image workflow hasn't finished)"
digest=$(header_value "$work/head" docker-content-digest)
type=$(header_value "$work/head" content-type)
is_digest "$digest" || refuse "the registry gave no usable digest for $image:$tag: '$digest'"
case $type in
  application/vnd.oci.image.index.v1+json | application/vnd.docker.distribution.manifest.list.v2+json) ;;
  *) refuse "$image:$tag is a single image ($type), not the multi-arch index release-server.yml publishes" ;;
esac
echo "verifying $image:$tag = $digest" >&2

# The attestation: signed by Sigstore with a certificate GitHub issued to release-server.yml in this repo (on main, or
# for --branch on any branch), on a GitHub-hosted runner, with this exact digest as its subject. --cert-identity-regex
# is anchored at both ends; gh's --signer-workflow would match the workflow on any ref, and gh allows only one of them.
if [ "$mode" = main ]; then
  identity=(--cert-identity-regex "$main_identity" --source-ref refs/heads/main)
else
  identity=(--cert-identity-regex "$branch_identity" --format json)
fi
if ! gh attestation verify "oci://$image@$digest" --repo "$gh_repo" "${identity[@]}" --deny-self-hosted-runners \
  >"$work/verify.out"; then
  refuse "$image:$tag ($digest) has no valid build provenance from $gh_repo/$workflow_path$([ "$mode" = main ] && echo " on main").
$(refusal_hint "$tag" "$mode")"
fi
if [ "$mode" = branch ]; then
  branch=$(jq -r '[.[].verificationResult.signature.certificate.sourceRepositoryRef // empty] | unique | join(", ")' \
    "$work/verify.out" 2>/dev/null || true)
  echo "built by $workflow_path from branch: ${branch:-unknown}" >&2
else
  cat "$work/verify.out" >&2
fi

# The index itself, hashed against the digest: it must be an index, and for a <sha7> tag each platform image must carry
# that commit. The labels sit in content the attested digest covers, so they are as trustworthy as the attestation.
fetch "manifests/$digest" "$index_types" "$work/index"
platforms=$(jq -r '.manifests[]? | select(.platform.os != "unknown") | .digest' "$work/index")
[ -n "$platforms" ] || refuse "the index $digest lists no platform images"
if is_sha7 "$tag"; then
  for m in $platforms; do
    is_digest "$m" || refuse "the index lists a malformed digest '$m'"
    fetch "manifests/$m" "$manifest_types" "$work/manifest"
    config=$(jq -r '.config.digest // ""' "$work/manifest")
    is_digest "$config" || refuse "$m has no usable config digest"
    fetch "blobs/$config" '*/*' "$work/config"
    rev=$(jq -r '.config.Labels["org.opencontainers.image.revision"] // ""' "$work/config")
    [[ $rev == "$tag"* ]] || refuse "$image:$tag is labelled with commit '${rev:-none}', not $tag"
  done
  echo "commit label matches $tag on every platform" >&2
fi

echo "verified: $image:$tag = $digest" >&2
echo "$digest"
