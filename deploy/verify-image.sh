#!/usr/bin/env bash
# Runs on the deploying machine, before anything touches the server (`just deploy TAG`, or `just verify-image TAG`
# alone): resolve pulse-server:TAG to its manifest digest without pulling the image, check that digest carries a build
# provenance attestation signed by this repo's release-server.yml workflow, and that a <sha7> tag holds that commit.
# Prints the verified digest (sha256:…) on stdout and nothing else; everything else goes to stderr. Exit 1 = refused.
# Needs curl, jq and an authenticated gh (`gh auth login`); never ssh.
set -euo pipefail

gh_repo=GregHofmeyr/pulse-app
registry_repo=greghofmeyr/pulse-server
image=ghcr.io/$registry_repo
signer_workflow=$gh_repo/.github/workflows/release-server.yml
index_types='application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json'
manifest_types='application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json'

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

# Sourced by the self-test with VERIFY_IMAGE_LIB=1: define the functions, run nothing.
[ -n "${VERIFY_IMAGE_LIB:-}" ] && return 0

tag=${1:?usage: verify-image.sh TAG}
is_tag "$tag" || refuse "'$tag' is not an image tag"
for tool in curl jq gh; do
  command -v "$tool" >/dev/null || refuse "$tool is not installed"
done

# Anonymous pull token (the package is public): enough to read manifests, and no local docker login needed.
token=$(curl -fsS "https://ghcr.io/token?scope=repository:$registry_repo:pull" | jq -r .token) ||
  refuse "could not get a registry token from ghcr.io"
# registry_get PATH ACCEPT: GET a registry path (follows the blob redirect)
registry_get() {
  curl -fsSL -H "Authorization: Bearer $token" -H "Accept: $2" "https://ghcr.io/v2/$registry_repo/$1"
}

# The digest of what the tag names, as the registry serves it: the multi-arch index, the same thing the server's
# docker pull resolves the tag to (and the subject the publish job attests).
digest=$(curl -fsSI -H "Authorization: Bearer $token" -H "Accept: $index_types, $manifest_types" \
  "https://ghcr.io/v2/$registry_repo/manifests/$tag" | tr -d '\r' | sed -n 's/^docker-content-digest: *//Ip' | tail -n1) ||
  refuse "$image:$tag not found in the registry (mistyped sha, or its image workflow hasn't finished)"
is_digest "$digest" || refuse "the registry gave no usable digest for $image:$tag: '$digest'"
echo "verifying $image:$tag = $digest" >&2

# The attestation: signed by Sigstore with a certificate GitHub issued to release-server.yml in this repo, on a
# GitHub-hosted runner, with this exact digest as its subject.
if ! gh attestation verify "oci://$image@$digest" --repo "$gh_repo" --signer-workflow "$signer_workflow" \
  --deny-self-hosted-runners >&2; then
  refuse "$image:$tag ($digest) has no valid build provenance from $signer_workflow.
  Images built before provenance was added have none: to roll back to one of those on purpose, run
  \`just deploy-unverified $tag\` (deploy/README.md, section 7)."
fi

# A <sha7> tag must hold that commit. The workflow labels every platform image with the commit it checked out; the
# labels sit in content the attested digest covers, so they are as trustworthy as the attestation.
if [[ $tag =~ ^[0-9a-f]{7}$ ]]; then
  index=$(registry_get "manifests/$digest" "$index_types") || refuse "could not read the index $digest"
  platforms=$(jq -r '.manifests[] | select(.platform.os != "unknown") | .digest' <<<"$index")
  [ -n "$platforms" ] || refuse "the index $digest lists no platform images"
  for m in $platforms; do
    config=$(registry_get "manifests/$m" "$manifest_types" | jq -r .config.digest) || refuse "could not read $m"
    rev=$(registry_get "blobs/$config" '*/*' | jq -r '.config.Labels["org.opencontainers.image.revision"] // ""') ||
      refuse "could not read the config of $m"
    [[ $rev == "$tag"* ]] || refuse "$image:$tag is labelled with commit '${rev:-none}', not $tag"
  done
  echo "commit label matches $tag on every platform" >&2
fi

echo "verified: $image:$tag = $digest" >&2
echo "$digest"
