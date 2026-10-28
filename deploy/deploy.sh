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

# check_node_ip FILE: LIVEKIT_NODE_IP must be a public IPv4 in canonical dotted form. LiveKit silently ignores a
# malformed NODE_IP and then advertises the VM's private address: calls would connect and carry no audio.
# Private, loopback, link-local, CGNAT (Tailscale), 0/8, multicast and reserved (224/3: 224/4, 240/4 and
# 255.255.255.255) addresses are refused for the same reason.
check_node_ip() {
  local ip a b
  if [ ! -r "$1" ]; then
    echo "$1 not found (or not readable): create it with LIVEKIT_NODE_IP set (deploy/README.md, section 6)" >&2
    return 1
  fi
  ip=$(sed -n 's/^LIVEKIT_NODE_IP=//p' "$1" | tail -n1)
  # each octet 0-255, no leading zeros
  local octet='(25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9][0-9]|[0-9])'
  if [[ $ip =~ ^$octet\.$octet\.$octet\.$octet$ ]]; then
    IFS=. read -r a b _ <<<"$ip"
    if ! { [ "$a" -eq 0 ] || [ "$a" -eq 10 ] || [ "$a" -eq 127 ] || [ "$a" -ge 224 ] ||
      { [ "$a" -eq 169 ] && [ "$b" -eq 254 ]; } ||
      { [ "$a" -eq 172 ] && [ "$b" -ge 16 ] && [ "$b" -le 31 ]; } ||
      { [ "$a" -eq 192 ] && [ "$b" -eq 168 ]; } ||
      { [ "$a" -eq 100 ] && [ "$b" -ge 64 ] && [ "$b" -le 127 ]; }; }; then
      return 0
    fi
  fi
  echo "LIVEKIT_NODE_IP in $1 must be the server's public IPv4 (deploy/README.md, section 6); got '$ip'" >&2
  return 1
}

# check_totp_key FILE: PULSE_TOTP_KEY must be exactly 64 hex characters. The server refuses to start without a
# usable key once anyone has 2FA on, so catch it before anything is copied. The value is never printed.
check_totp_key() {
  local key
  if [ ! -r "$1" ]; then
    echo "$1 not found (or not readable): see deploy/README.md, \"First deploy after the two-factor push\"" >&2
    return 1
  fi
  key=$(sed -n 's/^PULSE_TOTP_KEY=//p' "$1" | tail -n1)
  if [[ $key =~ ^[0-9a-fA-F]{64}$ ]]; then
    return 0
  fi
  echo "PULSE_TOTP_KEY in $1 must be 64 hex characters (deploy/README.md, \"First deploy after the two-factor push\")" >&2
  return 1
}

# reload_caddy: apply the Caddyfile just copied. Quiet when it works. A failure doesn't stop the deploy (the
# running Caddy keeps its previous config, and the server update is what matters), but it says so on stderr.
# reload finds the admin API (a unix socket, see the Caddyfile's global block) from --config.
reload_caddy() {
  if ! docker compose exec -T caddy caddy reload --config /etc/caddy/Caddyfile >/dev/null 2>&1; then
    echo "WARNING: caddy reload failed; Caddy is still running its previous config." >&2
    echo "  Check it with: docker compose exec caddy caddy reload --config /etc/caddy/Caddyfile" >&2
  fi
  return 0
}

# prune_old_images: keep the newest 3 server images (the live one and two to roll back to) and remove the rest, so
# each deploy's ~160 MB image doesn't pile up on the disk. Quiet; an image still in use simply isn't removed.
prune_old_images() {
  local old
  old=$(docker images --no-trunc --format '{{.ID}}' "$repo" | awk '!seen[$0]++' | tail -n +4)
  # shellcheck disable=SC2086 # one id per word
  [ -z "$old" ] || docker rmi -f $old >/dev/null 2>&1 || true
  docker image prune -f >/dev/null 2>&1 || true
}

# Sourced by the self-test with DEPLOY_SH_LIB=1: define the functions, run nothing.
[ -n "${DEPLOY_SH_LIB:-}" ] && return 0

# `just deploy` runs this first, piped over ssh, before it copies anything to the server: a missing or bad
# LIVEKIT_NODE_IP or PULSE_TOTP_KEY stops the deploy while /srv/pulse is still exactly as it was.
if [ "${1:-}" = --check-env ]; then
  check_node_ip "${2:-/srv/pulse/.env}"
  check_totp_key "${2:-/srv/pulse/.env}"
  exit
fi

tag=${1:?usage: deploy.sh TAG}
cd /srv/pulse
check_node_ip .env
check_totp_key .env
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
reload_caddy
for _ in $(seq 1 30); do
  if curl -fsS http://127.0.0.1:7890/health >/dev/null; then
    rev=$(docker inspect --format '{{index .Config.Labels "org.opencontainers.image.revision"}}' \
      "$(docker compose ps -q pulse-server)")
    echo "healthy: $tag (commit ${rev:0:7}, ${image#*@})"
    prune_old_images
    df -h / | awk 'NR==2 {print "disk: " $5 " used, " $4 " free"}'
    exit 0
  fi
  sleep 1
done
echo "server not healthy after 30 s" >&2
docker compose logs --tail 50 pulse-server
exit 1
