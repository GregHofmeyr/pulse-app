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

# uploads go to ./data/media (gitignored) unless PULSE_MEDIA_DIR says otherwise; the server's own default is /data/media
dev-server:
    PULSE_MEDIA_DIR="${PULSE_MEDIA_DIR:-data/media}" cargo run -p pulse-server --bin pulse-app-server -- serve

ui-check:
    cd client/ui && pnpm install --frozen-lockfile && pnpm check && pnpm test

dev-client:
    cd client/src-tauri && cargo tauri dev --config tauri.dev.conf.json

# print a single-use invite for the local dev server
invite:
    cargo run -q -p pulse-server --bin pulse-app-server -- create-invite

# real LiveKit → webhook → voice_sessions, end to end
voice-smoke:
    ./scripts/voice-smoke.sh

# voice integration test against the dev LiveKit (real WebRTC, fake peer)
voice-it: dev-livekit
    cargo test -p pulse-client --test voice_it --test voice_outage_it --test stream_it -- --ignored --nocapture
    cargo test -p pulse-server --test livekit_ice -- --ignored --nocapture

# This checkout's dev LiveKit then listens on 17880-17882 and 13478 instead of 7880-7882 and 3478, and sends its
# webhooks to 17890, where nothing listens (the tests' servers use ephemeral ports and don't need them), not to
# the other checkout's dev server on 7890.
# `docker compose down` afterwards; a plain `just dev-livekit` moves it back.
# voice-it beside another checkout's dev LiveKit (alternate ports)
voice-it-alt:
    PULSE_DEV_LIVEKIT_PORT=17880 PULSE_DEV_LIVEKIT_TCP_PORT=17881 PULSE_DEV_LIVEKIT_UDP_PORT=17882 \
        PULSE_DEV_LIVEKIT_TURN_PORT=13478 PULSE_DEV_WEBHOOK_PORT=17890 just voice-it

# libwebrtc log lines per second in a two-person call (decides whether to filter at the source)
webrtc-log-volume: dev-livekit
    cargo test -p pulse-client --test webrtc_log_volume -- --ignored --nocapture

# validate the deploy/ files the same way CI does (needs Docker)
deploy-check:
    sed -e 's/^PULSE_UID=$/PULSE_UID=1000/' -e 's/^PULSE_GID=$/PULSE_GID=1000/' \
        -e 's/^ACME_EMAIL=$/ACME_EMAIL=ci@example.com/' -e 's/^LIVEKIT_SECRET=$/LIVEKIT_SECRET=x/' \
        -e 's/^LIVEKIT_NODE_IP=$/LIVEKIT_NODE_IP=203.0.113.10/' \
        -e 's/^PULSE_TOTP_KEY=$/PULSE_TOTP_KEY=0000000000000000000000000000000000000000000000000000000000000000/' \
        deploy/env.example > /tmp/pulse-ci.env
    docker compose --env-file /tmp/pulse-ci.env -f deploy/compose.yml config -q
    caddy_image=$(docker compose --env-file /tmp/pulse-ci.env -f deploy/compose.yml config --images | grep '^caddy:') && \
        docker run --rm -e PULSE_DOMAIN=example.com -e ACME_EMAIL=ci@example.com \
        -v "$PWD/deploy/Caddyfile:/etc/caddy/Caddyfile:ro" "$caddy_image" caddy validate --config /etc/caddy/Caddyfile
    docker run --rm -v "$PWD:/mnt" -w /mnt koalaman/shellcheck:v0.11.0 deploy/*.sh
    bash deploy/deploy-selftest.sh

# --- production (pulsechat.co.za); see deploy/README.md ---
prod := "pulse@pulse"

# set up / re-run setup on a server. First time: `just provision ubuntu@<public-ip> ~/.ssh/pulse-oracle.key`
provision host="ubuntu@pulse" key="":
    ssh {{ if key != "" { "-i " + key } else { "" } }} {{host}} 'rm -rf /tmp/pulse-deploy && mkdir -p /tmp/pulse-deploy'
    scp {{ if key != "" { "-i " + key } else { "" } }} deploy/setup.sh deploy/backup.sh deploy/deploy.sh deploy/compose.yml deploy/Caddyfile deploy/livekit.yaml deploy/env.example deploy/pulse-backup.service deploy/pulse-backup.timer {{host}}:/tmp/pulse-deploy/
    ssh -t {{ if key != "" { "-i " + key } else { "" } }} {{host}} 'sudo bash /tmp/pulse-deploy/setup.sh'

# close public SSH (only works over Tailscale, which is the point). First disable key expiry for
# `pulse` in the Tailscale admin console, or the server drops off the tailnet after 180 days.
close-public-ssh:
    just provision ubuntu@pulse
    ssh ubuntu@pulse 'sudo bash /tmp/pulse-deploy/setup.sh --close-ssh'

# back up, then run image TAG (a 7-char commit or `latest`); prints the commit that's live
deploy tag="latest":
    # Checks the server's .env (LIVEKIT_NODE_IP) with this deploy.sh before anything is copied, so a refusal
    # leaves /srv/pulse exactly as it was.
    ssh {{prod}} 'bash -s -- --check-env' < deploy/deploy.sh
    scp deploy/compose.yml deploy/Caddyfile deploy/livekit.yaml deploy/backup.sh deploy/deploy.sh {{prod}}:/srv/pulse/
    ssh {{prod}} '/srv/pulse/deploy.sh {{tag}}'

# outside-in checks of the live server
smoke:
    ./deploy/smoke.sh pulsechat.co.za

# a single-use invite on the live server (valid 24 h)
invite-prod:
    ssh {{prod}} 'cd /srv/pulse && docker compose exec -T pulse-server pulse-app-server create-invite'

# turn off two-factor for a locked-out user on the live server (the name goes into a remote shell, so it is checked here first; passed as $username, never pasted into the script)
reset-2fa $username:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ "$username" =~ ^[A-Za-z0-9_.]{2,32}$ ]] || { echo "reset-2fa: username must be 2-32 characters of letters, digits, _ or ." >&2; exit 1; }
    ssh {{prod}} 'cd /srv/pulse && docker compose exec -T pulse-server pulse-app-server reset-2fa '"$username"

# newest bug/crash reports on the live server (`new` = not looked at yet)
reports:
    ssh {{prod}} 'cd /srv/pulse && docker compose exec -T pulse-server pulse-app-server reports'

# one report in full; add --log for the attached app log
report id *flags:
    ssh {{prod}} 'cd /srv/pulse && docker compose exec -T pulse-server pulse-app-server report {{id}} {{flags}}'

# run a backup right now
backup-now:
    ssh {{prod}} '/srv/pulse/backup.sh run'

# download + decrypt + check a backup locally (latest if NAME is empty); the server is untouched
restore-drill name="":
    ./deploy/restore.sh drill {{name}}

# replace the live database with backup NAME (e.g. daily/pulse-20261010T010000Z.db.age)
restore name:
    ./deploy/restore.sh apply {{name}}

# download + decrypt + list a media backup locally (latest if NAME is empty); the server is untouched
restore-media-drill name="":
    ./deploy/restore.sh media-drill {{name}}

# unpack media backup NAME over the live media (e.g. daily/pulse-media-20261010T010000Z.tgz.age)
restore-media name:
    ./deploy/restore.sh media-apply {{name}}

# live server logs
logs service="pulse-server":
    ssh {{prod}} 'cd /srv/pulse && docker compose logs --tail 100 -f {{service}}'

# dependency policy: advisories, licences, bans, sources (needs `cargo install cargo-deny --locked`)
deny:
    cargo deny check
