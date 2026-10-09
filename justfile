set dotenv-load := true

# everything CI runs
check: fmt-check clippy test types-check ui-check

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

gen-types:
    cargo test -p pulse-protocol export_bindings

types-check: gen-types
    git diff --exit-code -- client/ui/src/lib/protocol

dev-livekit:
    docker compose up -d livekit

dev-server:
    cargo run -p pulse-server --bin pulse-app-server -- serve

ui-check:
    cd client/ui && pnpm install --frozen-lockfile && pnpm check && pnpm test

dev-client:
    cd client/src-tauri && cargo tauri dev

# print a single-use invite for the local dev server
invite:
    cargo run -q -p pulse-server --bin pulse-app-server -- create-invite

# real LiveKit → webhook → voice_sessions, end to end
voice-smoke:
    ./scripts/voice-smoke.sh

# voice integration test against the dev LiveKit (real WebRTC, fake peer)
voice-it: dev-livekit
    cargo test -p pulse-client --test voice_it -- --ignored --nocapture

# validate the deploy/ files the same way CI does (needs Docker)
deploy-check:
    sed -e 's/^PULSE_UID=$/PULSE_UID=1000/' -e 's/^PULSE_GID=$/PULSE_GID=1000/' \
        -e 's/^ACME_EMAIL=$/ACME_EMAIL=ci@example.com/' -e 's/^LIVEKIT_SECRET=$/LIVEKIT_SECRET=x/' \
        deploy/env.example > /tmp/pulse-ci.env
    docker compose --env-file /tmp/pulse-ci.env -f deploy/compose.yml config -q
    docker run --rm -e PULSE_DOMAIN=example.com -e ACME_EMAIL=ci@example.com \
        -v "$PWD/deploy/Caddyfile:/etc/caddy/Caddyfile:ro" caddy:2.10 caddy validate --config /etc/caddy/Caddyfile
    docker run --rm -v "$PWD:/mnt" -w /mnt koalaman/shellcheck:stable deploy/*.sh

# --- production (pulsechat.co.za); see deploy/README.md ---
prod := "pulse@pulse"

# set up / re-run setup on a server. First time: `just provision ubuntu@<public-ip> ~/.ssh/pulse-oracle.key`
provision host="ubuntu@pulse" key="":
    ssh {{ if key != "" { "-i " + key } else { "" } }} {{host}} 'rm -rf /tmp/pulse-deploy && mkdir -p /tmp/pulse-deploy'
    scp {{ if key != "" { "-i " + key } else { "" } }} deploy/setup.sh deploy/backup.sh deploy/compose.yml deploy/Caddyfile deploy/livekit.yaml deploy/env.example deploy/pulse-backup.service deploy/pulse-backup.timer {{host}}:/tmp/pulse-deploy/
    ssh -t {{ if key != "" { "-i " + key } else { "" } }} {{host}} 'sudo bash /tmp/pulse-deploy/setup.sh'

# close public SSH (only works over Tailscale, which is the point)
close-public-ssh:
    just provision ubuntu@pulse
    ssh ubuntu@pulse 'sudo bash /tmp/pulse-deploy/setup.sh --close-ssh'

# back up, then run image TAG (a 7-char commit or `latest`) and wait for it to be healthy
deploy tag="latest":
    scp deploy/compose.yml deploy/Caddyfile deploy/livekit.yaml deploy/backup.sh {{prod}}:/srv/pulse/
    ssh {{prod}} 'set -e; cd /srv/pulse; ./backup.sh run; \
        sed -i "s/^PULSE_TAG=.*/PULSE_TAG={{tag}}/" .env; \
        docker compose pull -q; docker compose up -d --remove-orphans; \
        docker compose exec -T caddy caddy reload --config /etc/caddy/Caddyfile >/dev/null 2>&1 || true; \
        for i in $(seq 1 30); do curl -fsS http://127.0.0.1:7890/health >/dev/null && echo "healthy: {{tag}}" && exit 0; sleep 1; done; \
        echo "server not healthy after 30 s" >&2; docker compose logs --tail 50 pulse-server; exit 1'

# outside-in checks of the live server
smoke:
    ./deploy/smoke.sh pulsechat.co.za

# a single-use invite on the live server (valid 24 h)
invite-prod:
    ssh {{prod}} 'cd /srv/pulse && docker compose exec -T pulse-server pulse-app-server create-invite'

# run a backup right now
backup-now:
    ssh {{prod}} '/srv/pulse/backup.sh run'

# download + decrypt + check a backup locally (latest if NAME is empty); the server is untouched
restore-drill name="":
    ./deploy/restore.sh drill {{name}}

# replace the live database with backup NAME (e.g. daily/pulse-20261010T010000Z.db.age)
restore name:
    ./deploy/restore.sh apply {{name}}

# live server logs
logs service="pulse-server":
    ssh {{prod}} 'cd /srv/pulse && docker compose logs --tail 100 -f {{service}}'
