#!/usr/bin/env bash
# Runs on the server (`just deploy TAG`): back up, pull TAG, switch to it, wait until healthy.
set -euo pipefail
tag=${1:?usage: deploy.sh TAG}
cd /srv/pulse
./backup.sh run
# Pull first: a mistyped tag fails here and leaves .env and the running server untouched.
PULSE_TAG=$tag docker compose pull -q
sed -i "s/^PULSE_TAG=.*/PULSE_TAG=$tag/" .env
docker compose up -d --remove-orphans
docker compose exec -T caddy caddy reload --config /etc/caddy/Caddyfile >/dev/null 2>&1 || true
for _ in $(seq 1 30); do
  if curl -fsS http://127.0.0.1:7890/health >/dev/null; then
    rev=$(docker inspect --format '{{index .Config.Labels "org.opencontainers.image.revision"}}' \
      "$(docker compose ps -q pulse-server)")
    echo "healthy: $tag (commit ${rev:0:7})"
    exit 0
  fi
  sleep 1
done
echo "server not healthy after 30 s" >&2
docker compose logs --tail 50 pulse-server
exit 1
