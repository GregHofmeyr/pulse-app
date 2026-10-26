#!/usr/bin/env bash
# Runs on the server (`just deploy TAG`): back up, pull TAG, switch to it, wait until healthy.
set -euo pipefail
repo=ghcr.io/greghofmeyr/pulse-server

# set_env FILE KEY VALUE: replace the KEY= line, or append it when absent (older servers have no PULSE_IMAGE line).
set_env() {
  local file=$1 key=$2 value=$3
  if grep -q "^${key}=" "$file"; then
    # '|' as the sed delimiter: image references contain '/', never '|'
    sed -i "s|^${key}=.*|${key}=${value}|" "$file"
  else
    # a file without a trailing newline would glue KEY onto its last line
    if [ -s "$file" ] && [ -n "$(tail -c1 "$file")" ]; then
      echo >>"$file"
    fi
    printf '%s=%s\n' "$key" "$value" >>"$file"
  fi
}

# check_node_ip FILE: LIVEKIT_NODE_IP must be a plain IPv4. LiveKit silently ignores a malformed NODE_IP and
# then advertises the VM's private address: calls would connect and carry no audio.
check_node_ip() {
  local ip
  ip=$(sed -n 's/^LIVEKIT_NODE_IP=//p' "$1" | tail -n1)
  if ! [[ $ip =~ ^([0-9]{1,3}\.){3}[0-9]{1,3}$ ]]; then
    echo "LIVEKIT_NODE_IP in $1 must be the server's public IPv4 (deploy/README.md, section 6)" >&2
    return 1
  fi
}

# Sourced by the self-test with DEPLOY_SH_LIB=1: define the functions, run nothing.
[ -n "${DEPLOY_SH_LIB:-}" ] && return 0

tag=${1:?usage: deploy.sh TAG}
cd /srv/pulse
check_node_ip .env
./backup.sh run
# Pull first: a mistyped tag fails here and leaves .env and the running server untouched.
# PULSE_IMAGE from the shell wins over .env, so this pulls TAG and not the previously pinned digest.
PULSE_IMAGE=$repo:$tag docker compose pull -q
# Pin to the digest of what was just pulled, so compose runs exactly this image even if the tag moves.
digest=$(docker image inspect --format '{{range .RepoDigests}}{{println .}}{{end}}' "$repo:$tag" | grep "^$repo@sha256:" | head -n1 || true)
if [ -z "$digest" ]; then
  echo "no registry digest for $repo:$tag; refusing to deploy an unpinned image" >&2
  exit 1
fi
image=$repo:$tag@${digest#*@}
set_env .env PULSE_TAG "$tag"
set_env .env PULSE_IMAGE "$image"
echo "pinned: $image"
docker compose up -d --remove-orphans
docker compose exec -T caddy caddy reload --config /etc/caddy/Caddyfile >/dev/null 2>&1 || true
for _ in $(seq 1 30); do
  if curl -fsS http://127.0.0.1:7890/health >/dev/null; then
    rev=$(docker inspect --format '{{index .Config.Labels "org.opencontainers.image.revision"}}' \
      "$(docker compose ps -q pulse-server)")
    echo "healthy: $tag (commit ${rev:0:7}, ${image#*@})"
    exit 0
  fi
  sleep 1
done
echo "server not healthy after 30 s" >&2
docker compose logs --tail 50 pulse-server
exit 1
