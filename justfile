set dotenv-load := true

# everything CI runs
check: fmt-check clippy test types-check ui-check fetch-ffmpeg-test

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

# the pinned LGPL ffmpeg the Windows installer bundles, into client/src-tauri/ffmpeg/ (verified by SHA-256; Windows
# builds and `cargo xwin` need it, Linux builds don't)
fetch-ffmpeg:
    bash scripts/fetch-ffmpeg.sh

# offline self-test of scripts/fetch-ffmpeg.sh (fake archives, wrong and right SHA-256)
fetch-ffmpeg-test:
    bash scripts/fetch-ffmpeg-test.sh

dev-livekit:
    docker compose up -d livekit

# uploads go to ./data/media (gitignored) unless PULSE_MEDIA_DIR says otherwise; the server's own default is /data/media
dev-server:
    PULSE_MEDIA_DIR="${PULSE_MEDIA_DIR:-data/media}" \
    PULSE_CLIPS_S3_ENDPOINT="${PULSE_CLIPS_S3_ENDPOINT:-http://127.0.0.1:7895}" \
    PULSE_CLIPS_S3_BUCKET="${PULSE_CLIPS_S3_BUCKET:-pulse-clips-test}" \
    PULSE_CLIPS_S3_KEY_ID="${PULSE_CLIPS_S3_KEY_ID:-FAKEKEYID-not-a-real-key}" \
    PULSE_CLIPS_S3_SECRET="${PULSE_CLIPS_S3_SECRET:-fake-secret-not-a-real-secret-0000}" \
    cargo run -p pulse-server --bin pulse-app-server -- serve

# a fake R2 (SigV4-checking, on disk) at 127.0.0.1:7895; prints the bucket and keys to configure
dev-s3:
    cargo run -p pulse-server --features testing --example fake-s3 -- 127.0.0.1:7895 data/fake-s3

ui-check:
    cd client/ui && pnpm install --frozen-lockfile && pnpm check && pnpm test

# On Linux the UI comes from http://localhost here, and WebKitGTK won't show pulse-media images on an http page.
# tauri.dev.conf.json's CSP replaces tauri.linux.conf.json's, so Linux adds its own copy (loopback clip video) on top.
# hot-reloading client (on Linux avatars, banners and emoji are blank: use `just linux-client` for those)
dev-client:
    cd client/src-tauri && cargo tauri dev --config tauri.dev.conf.json {{ if os() == "linux" { "--config tauri.dev.linux.conf.json" } else { "" } }}

# Built like an install (a tauri:// page, so media shows) but as a debug build, so clips use the system ffmpeg.
# WebKitGTK plays video through GStreamer: without autoaudiosink (gst-plugins-good) any video crashes the page.
# the Linux client against the live server, for testing on Linux
linux-client:
    @gst-inspect-1.0 autoaudiosink >/dev/null 2>&1 || echo "warning: GStreamer has no autoaudiosink (install gst-plugins-good): videos will crash the page" >&2
    cd client/src-tauri && VITE_DEFAULT_SERVER=https://pulsechat.co.za cargo tauri build --debug --no-bundle
    PULSE_DEV_FFMPEG_DIR=/usr/bin ./target/debug/pulse-app

# print a single-use invite for the local dev server
invite:
    cargo run -q -p pulse-server --bin pulse-app-server -- create-invite

# real LiveKit → webhook → voice_sessions, end to end
voice-smoke:
    ./scripts/voice-smoke.sh

# the voice tests that open this machine's real mic and speakers (they can be heard): voice-it skips them
real_device_tests := "--skip mic_test_queue_on_real_devices --skip real_devices_are_not_reopened_without_a_reason --skip reopening_one_side_leaves_the_other_alone"

# voice integration test against the dev LiveKit (real WebRTC, fake peer); no real audio devices
voice-it: dev-livekit
    cargo test -p pulse-client --test voice_it --test voice_outage_it --test stream_it -- --ignored --nocapture {{ real_device_tests }}
    cargo test -p pulse-server --test livekit_ice --test stream_liveness_it -- --ignored --nocapture

# the real-device voice probes on their own (mic and speakers open; PULSE_IT_INPUT / PULSE_IT_OUTPUT pick devices)
voice-it-devices: dev-livekit
    cargo test -p pulse-client --test voice_it -- --ignored --nocapture --exact mic_test_queue_on_real_devices real_devices_are_not_reopened_without_a_reason reopening_one_side_leaves_the_other_alone

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
    bash deploy/backup-selftest.sh

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

# disk use on the live server (and this machine's build caches); run it daily. Warns at 80%.
disk:
    #!/usr/bin/env bash
    set -euo pipefail
    out=$(ssh {{prod}} 'df -h / | tail -1; docker system df')
    echo "$out"
    used=$(awk 'NR==1 {sub("%", "", $5); print $5}' <<<"$out")
    if [ "$used" -ge 80 ]; then echo "WARNING: server disk ${used}% full" >&2; exit 1; fi
    du -sh target ../pulse-app-*/target 2>/dev/null || true

# back up, then run image TAG (a 7-char commit or `latest`) once its provenance (built by release-server.yml on main) checks out; prints the commit that's live
deploy $tag="latest":
    #!/usr/bin/env bash
    set -euo pipefail
    # Checked here, before anything is copied or ssh'd: a refusal leaves the server untouched. The server then pulls
    # and pins exactly this digest; it never looks the tag up again.
    digest=$(bash deploy/verify-image.sh "$tag")
    just _ship "$tag" "$digest"

# run a hand-run branch build SHA7 once its provenance (built by release-server.yml on some branch) checks out; prints the branch
deploy-branch $tag:
    #!/usr/bin/env bash
    set -euo pipefail
    digest=$(bash deploy/verify-image.sh --branch "$tag")
    just _ship "$tag" "$digest"

# check TAG's provenance (main builds) without deploying: prints the verified digest; never contacts the server
verify-image $tag="latest":
    bash deploy/verify-image.sh "$tag"

# run image SHA7 WITHOUT checking provenance: only for builds from before provenance existed. Needs CONFIRM=SHA7.
deploy-unverified $tag:
    #!/usr/bin/env bash
    set -euo pipefail
    VERIFY_IMAGE_LIB=1 source deploy/verify-image.sh
    check_unverified_args "$tag" "${CONFIRM:-}"
    cat >&2 <<EOF
    ########################################################################################################
    WARNING: deploying pulse-server:$tag WITHOUT checking its build provenance.
    Nothing proves this image was built by release-server.yml from this repo. Use this only to roll back
    to a build from before provenance existed (deploy/README.md, section 7).
    ########################################################################################################
    EOF
    just _ship "$tag" --unverified

# Private: the shared second half of deploy, deploy-branch and deploy-unverified. PIN is a sha256 digest (which this
# recipe does NOT verify: deploy and deploy-branch do, before calling it) or --unverified. Don't run it directly.
_ship $tag $pin:
    #!/usr/bin/env bash
    set -euo pipefail
    # both end up in a remote command line: check them here too
    VERIFY_IMAGE_LIB=1 source deploy/verify-image.sh
    check_ship_args "$tag" "$pin"
    # Checks the server's .env (LIVEKIT_NODE_IP, PULSE_TOTP_KEY) with this deploy.sh before anything is copied, so a
    # refusal leaves /srv/pulse exactly as it was.
    ssh {{prod}} 'bash -s -- --check-env' < deploy/deploy.sh
    scp deploy/compose.yml deploy/Caddyfile deploy/livekit.yaml deploy/backup.sh deploy/deploy.sh {{prod}}:/srv/pulse/
    ssh {{prod}} "/srv/pulse/deploy.sh $tag $pin"

# outside-in checks of the live server
smoke:
    ./deploy/smoke.sh pulsechat.co.za

# write, read and delete one small object in the clips bucket from the live server (run after filling the CLIPS_R2_* settings)
clips-check:
    ssh {{prod}} 'cd /srv/pulse && docker compose exec -T pulse-server pulse-app-server clips-check'

# each server's clip storage and the estimated monthly cost (all servers together; never shown in the app)
clips-storage:
    ssh {{prod}} 'cd /srv/pulse && docker compose exec -T pulse-server pulse-app-server clip-storage'

# a single-use invite on the live server (valid 24 h)
invite-prod:
    ssh {{prod}} 'cd /srv/pulse && docker compose exec -T pulse-server pulse-app-server create-invite'

# turn off two-factor for a locked-out user on the live server (the name goes into a remote shell, so it is checked here first; passed as $username, never pasted into the script)
reset-2fa $username:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ "$username" =~ ^[A-Za-z0-9_.]{2,32}$ ]] || { echo "reset-2fa: username must be 2-32 characters of letters, digits, _ or ." >&2; exit 1; }
    # `run`, not `exec`: works with the server stopped (it refuses to start on a wrong key). The image's entrypoint is the binary.
    ssh {{prod}} 'cd /srv/pulse && docker compose run --rm --no-deps -T pulse-server reset-2fa '"$username"

# set a new password for a user on the live server and sign them out everywhere (the password is typed here, not echoed, and goes over ssh on stdin, never in a command line)
reset-password $username:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ "$username" =~ ^[A-Za-z0-9_.]{2,32}$ ]] || { echo "reset-password: username must be 2-32 characters of letters, digits, _ or ." >&2; exit 1; }
    read -rsp "New password for $username: " pw; echo
    read -rsp "Again: " again; echo
    [[ "$pw" == "$again" ]] || { echo "reset-password: the passwords don't match" >&2; exit 1; }
    # printf is a shell builtin, so the password is never in a process's arguments.
    printf '%s\n' "$pw" | ssh {{prod}} 'cd /srv/pulse && docker compose run --rm --no-deps -T pulse-server reset-password '"$username"

# sign a user out everywhere on the live server (deletes all their sessions; their password stays)
revoke-sessions $username:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ "$username" =~ ^[A-Za-z0-9_.]{2,32}$ ]] || { echo "revoke-sessions: username must be 2-32 characters of letters, digits, _ or ." >&2; exit 1; }
    ssh {{prod}} 'cd /srv/pulse && docker compose run --rm --no-deps -T pulse-server revoke-sessions '"$username"

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
