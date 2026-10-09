#!/usr/bin/env bash
# Encrypted database backups to Cloudflare R2 (deploy/README.md, "Backups"). Runs as `pulse`.
#   backup.sh [run]      snapshot → age-encrypt → upload → prune   (pulse-backup.service, just deploy)
#   backup.sh latest     print the newest daily backup's object name
#   backup.sh fetch OBJ  write backup OBJ (still encrypted) to stdout
set -euo pipefail
cd /srv/pulse
set -a
# shellcheck source=/dev/null
. ./.env
set +a
export RCLONE_CONFIG_R2_TYPE=s3 RCLONE_CONFIG_R2_PROVIDER=Cloudflare RCLONE_CONFIG_R2_NO_CHECK_BUCKET=true \
  RCLONE_CONFIG_R2_ACCESS_KEY_ID="${R2_ACCESS_KEY_ID:-}" RCLONE_CONFIG_R2_SECRET_ACCESS_KEY="${R2_SECRET_ACCESS_KEY:-}" \
  RCLONE_CONFIG_R2_ENDPOINT="https://${R2_ACCOUNT_ID:-unset}.r2.cloudflarestorage.com"
remote="r2:${R2_BUCKET:-unset}"

case "${1:-run}" in
  latest) rclone lsf "$remote/daily" | sort | tail -n1 | sed 's|^|daily/|'; exit 0 ;;
  fetch) rclone cat "$remote/${2:?object name}"; exit 0 ;;
  run) ;;
  *) echo "usage: backup.sh [run|latest|fetch OBJ]" >&2; exit 2 ;;
esac

if [[ -z ${AGE_RECIPIENT:-} || -z ${R2_BUCKET:-} ]]; then
  echo "backups not configured yet (AGE_RECIPIENT / R2_* empty in .env): skipping" >&2
  exit 0
fi
if ! docker compose ps --status running --services | grep -qx pulse-server; then
  echo "pulse-server isn't running: nothing to back up" >&2
  exit 0
fi

ts=$(date -u +%Y%m%dT%H%M%SZ)
snap="data/backup-$ts.db"
# The plaintext snapshot must never outlive this script, whatever happens.
trap 'rm -f "$snap" "$snap.age"' EXIT
docker compose exec -T pulse-server pulse-app-server backup "/data/backup-$ts.db"
age -r "$AGE_RECIPIENT" -o "$snap.age" "$snap"
rm -f "$snap"
rclone copyto "$snap.age" "$remote/daily/pulse-$ts.db.age"
if [[ $(date -u +%u) == 7 ]]; then rclone copyto "$snap.age" "$remote/weekly/pulse-$ts.db.age"; fi
rclone delete "$remote/daily" --min-age 14d
rclone delete "$remote/weekly" --min-age 56d
echo "backed up pulse-$ts.db.age"
