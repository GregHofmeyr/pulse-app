#!/usr/bin/env bash
# Encrypted database and media backups to Cloudflare R2 (deploy/README.md, "Backups"). Runs as `pulse`.
#   backup.sh [run]      snapshot db + tar media → age-encrypt → upload → prune   (pulse-backup.service, just deploy)
#   backup.sh latest     print the newest daily database backup's object name
#   backup.sh latest-media  print the newest daily media backup's object name
#   backup.sh fetch OBJ  write backup OBJ (still encrypted) to stdout
set -euo pipefail
cd /srv/pulse
set -a
# shellcheck source=/dev/null
. ./.env
set +a
# rclone 1.60.1 sends HEAD ?versionId= after each PUT and R2 answers 501, so the first attempt "fails" though the upload succeeded.
export RCLONE_CONFIG_R2_TYPE=s3 RCLONE_CONFIG_R2_PROVIDER=Cloudflare RCLONE_CONFIG_R2_NO_CHECK_BUCKET=true RCLONE_CONFIG_R2_NO_HEAD=true \
  RCLONE_CONFIG_R2_ACCESS_KEY_ID="${R2_ACCESS_KEY_ID:-}" RCLONE_CONFIG_R2_SECRET_ACCESS_KEY="${R2_SECRET_ACCESS_KEY:-}" \
  RCLONE_CONFIG_R2_ENDPOINT="https://${R2_ACCOUNT_ID:-unset}.r2.cloudflarestorage.com"
remote="r2:${R2_BUCKET:-unset}"

# Newest daily object matching grep pattern $1; $2 names it in the message when there is none.
newest() {
  local name
  name=$(rclone lsf "$remote/daily" | { grep -E "$1" || true; } | sort | tail -n1)
  if [[ -z $name ]]; then
    echo "no $2 backup found in $remote/daily" >&2
    return 1
  fi
  echo "daily/$name"
}

case "${1:-run}" in
  latest) newest '\.db\.age$' database; exit $? ;;
  latest-media) newest '^pulse-media-' media; exit $? ;;
  fetch) rclone cat "$remote/${2:?object name}"; exit 0 ;;
  run) ;;
  *) echo "usage: backup.sh [run|latest|latest-media|fetch OBJ]" >&2; exit 2 ;;
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
media="data/media-$ts.tgz"
trap 'rm -f "$snap" "$snap.age" "$media" "$media.age"' EXIT
docker compose exec -T pulse-server pulse-app-server backup "/data/backup-$ts.db"
age -r "$AGE_RECIPIENT" -o "$snap.age" "$snap"
rm -f "$snap"
rclone copyto "$snap.age" "$remote/daily/pulse-$ts.db.age"
if [[ $(date -u +%u) == 7 ]]; then rclone copyto "$snap.age" "$remote/weekly/pulse-$ts.db.age"; fi
rclone delete "$remote/daily" --min-age 14d
rclone delete "$remote/weekly" --min-age 56d
echo "backed up pulse-$ts.db.age"

# Uploaded emoji live outside the database; no media directory yet means nothing was ever uploaded.
if [[ -d data/media ]]; then
  # tar exits 1 when a file changed while it was read (an upload during the backup): the archive is
  # still written and the database backup above already succeeded, so that only warrants a warning.
  tar_rc=0
  tar -C data -czf "$media" media || tar_rc=$?
  if [[ $tar_rc -eq 1 ]]; then
    echo "warning: media changed while being archived; the backup may miss the newest uploads" >&2
  elif [[ $tar_rc -ne 0 ]]; then
    echo "media archive failed (tar exit $tar_rc)" >&2
    exit "$tar_rc"
  fi
  age -r "$AGE_RECIPIENT" -o "$media.age" "$media"
  rm -f "$media"
  rclone copyto "$media.age" "$remote/daily/pulse-media-$ts.tgz.age"
  if [[ $(date -u +%u) == 7 ]]; then rclone copyto "$media.age" "$remote/weekly/pulse-media-$ts.tgz.age"; fi
  echo "backed up pulse-media-$ts.tgz.age"
fi
