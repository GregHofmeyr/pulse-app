#!/usr/bin/env bash
# Self-test for backup.sh (`just deploy-check` and CI run it). docker, age, rclone and curl are stubbed and log
# what they were asked; nothing touches the network or a real server.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
failed=0

mkdir -p "$tmp/bin"
# docker compose ps → pulse-server running; docker compose exec … backup PATH → writes the snapshot under data/
cat >"$tmp/bin/docker" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  "compose ps"*) echo pulse-server ;;
  *"backup /data/"*)
    last=${!#}
    echo snapshot >"data/${last#/data/}"
    ;;
esac
EOF
# age -r KEY -o OUT IN → OUT
cat >"$tmp/bin/age" <<'EOF'
#!/usr/bin/env bash
cp "$5" "$4"
EOF
# rclone: FAIL_RCLONE=1 fails the upload
cat >"$tmp/bin/rclone" <<'EOF'
#!/usr/bin/env bash
echo "rclone $*" >>"$LOG"
[[ $1 == copyto && ${FAIL_RCLONE:-} == 1 ]] && exit 1
exit 0
EOF
cat >"$tmp/bin/curl" <<'EOF'
#!/usr/bin/env bash
echo "curl ${*: -1}" >>"$LOG"
EOF
chmod +x "$tmp/bin"/*

# a fresh /srv/pulse lookalike; $1 = extra .env lines
setup() {
  rm -rf "$tmp/srv"
  mkdir -p "$tmp/srv/data"
  printf 'AGE_RECIPIENT=age1test\nR2_BUCKET=b\n%s\n' "${1:-}" >"$tmp/srv/.env"
  : >"$tmp/log"
}
run() { PATH="$tmp/bin:$PATH" PULSE_DIR="$tmp/srv" LOG="$tmp/log" BACKUP_LOCK_WAIT=1 bash "$here/backup.sh" run >"$tmp/out" 2>&1; }
leftovers() { find "$tmp/srv/data" -name 'backup-*' -o -name 'media-*' | grep -q .; }
pass() { echo "ok   $1"; }
fail() {
  echo "FAIL $1"
  sed 's/^/     | /' "$tmp/out" "$tmp/log"
  failed=1
}

setup 'BACKUP_PING_URL=https://hc.example/ping/abc'
if run && grep -qx 'curl https://hc.example/ping/abc' "$tmp/log" && ! leftovers; then
  pass "a good backup pings and leaves nothing behind"
else fail "a good backup pings and leaves nothing behind"; fi

setup 'BACKUP_PING_URL=https://hc.example/ping/abc'
if ! FAIL_RCLONE=1 run && grep -qx 'curl https://hc.example/ping/abc/fail' "$tmp/log" &&
  ! grep -qx 'curl https://hc.example/ping/abc' "$tmp/log" && ! leftovers; then
  pass "a failed upload pings /fail and still leaves no plaintext"
else fail "a failed upload pings /fail and still leaves no plaintext"; fi

setup
touch "$tmp/srv/data/backup-20260101T000000Z.db" "$tmp/srv/data/media-20260101T000000Z.tgz.age"
if run && ! leftovers; then
  pass "a killed earlier run's files are swept"
else fail "a killed earlier run's files are swept"; fi

setup
if run && ! grep -q '^curl' "$tmp/log"; then
  pass "no ping URL, no ping"
else fail "no ping URL, no ping"; fi

setup 'BACKUP_PING_URL=https://hc.example/ping/abc'
exec 8>"$tmp/srv/data/.backup.lock"
flock 8
touch "$tmp/srv/data/backup-inflight.db" # the running backup's own snapshot: not ours to sweep
if ! run && ! grep -q 'rclone copyto' "$tmp/log" && grep -qx 'curl https://hc.example/ping/abc/fail' "$tmp/log" &&
  [[ -e $tmp/srv/data/backup-inflight.db ]]; then
  pass "a backup already running isn't overlapped (and its files are left alone)"
else fail "a backup already running isn't overlapped (and its files are left alone)"; fi
exec 8>&-

exit $failed
