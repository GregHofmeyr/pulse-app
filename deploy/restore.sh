#!/usr/bin/env bash
# Restore from an encrypted backup. Runs on your machine (needs Tailscale + the age private key).
#   restore.sh drill [OBJ]         download, decrypt, integrity-check the database locally; the server is untouched
#   restore.sh apply OBJ           …then replace the live database (pulse-server stops for a few seconds)
#   restore.sh media-drill [OBJ]   download, decrypt and list a media backup locally; the server is untouched
#   restore.sh media-apply OBJ     …then unpack it over the live media (the server keeps running)
set -euo pipefail
PROD=${PULSE_PROD:-pulse@pulse}
KEY=${PULSE_BACKUP_KEY:-$HOME/.config/pulse/backup-age.key}
mode=${1:-}
name=${2:-}
usage() { echo "usage: restore.sh drill [OBJ] | apply OBJ | media-drill [OBJ] | media-apply OBJ" >&2; exit 2; }
case $mode in
  drill | media-drill) ;;
  apply | media-apply) [[ -n $name ]] || { echo "$mode needs an explicit backup name" >&2; exit 2; } ;;
  *) usage ;;
esac
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

newest=latest
if [[ $mode == media-* ]]; then newest=latest-media; fi
# shellcheck disable=SC2029 # $newest is meant to expand here, on this machine
[[ -n $name ]] || name=$(ssh "$PROD" "/srv/pulse/backup.sh $newest")
echo "backup: $name"
# shellcheck disable=SC2029 # $name is meant to expand here, on this machine
ssh "$PROD" "/srv/pulse/backup.sh fetch '$name'" >"$work/backup.age"

if [[ $mode == media-* ]]; then
  age -d -i "$KEY" -o "$work/media.tgz" "$work/backup.age"
  tar -tzf "$work/media.tgz" >"$work/list"
  # Everything must unpack under media/: a tampered archive can't write elsewhere on the server.
  if grep -qvE '^media(/|$)' "$work/list" || grep -qE '(^|/)\.\.(/|$)' "$work/list"; then
    echo "archive has paths outside media/; refusing" >&2
    exit 1
  fi
  echo "archive ok: $(grep -c '\.webp$' "$work/list") media files"
  [[ $mode == media-drill ]] && exit 0
  scp -q "$work/media.tgz" "$PROD:/srv/pulse/media.tgz"
  # Unpacks over data/media: files in the backup are put back, newer uploads it doesn't have stay.
  # The server keeps running: files are content-addressed, so an overwrite writes identical bytes.
  ssh "$PROD" 'set -e; cd /srv/pulse
    trap "rm -f media.tgz" EXIT
    tar -xzf media.tgz -C data
    echo "media restored"'
  exit 0
fi

age -d -i "$KEY" -o "$work/pulse.db" "$work/backup.age"
check=$(sqlite3 "$work/pulse.db" 'PRAGMA integrity_check;')
[[ $check == ok ]] || { echo "integrity check failed: $check" >&2; exit 1; }
echo "integrity ok: $(sqlite3 "$work/pulse.db" 'SELECT COUNT(*) FROM users;') users, $(sqlite3 "$work/pulse.db" 'SELECT COUNT(*) FROM messages;') messages"
[[ $mode == drill ]] && exit 0

scp -q "$work/pulse.db" "$PROD:/srv/pulse/restore.db"
ssh "$PROD" 'set -e; cd /srv/pulse
  # Whatever fails below, the server comes back up.
  trap "docker compose start pulse-server" EXIT
  docker compose stop pulse-server
  ts=$(date -u +%Y%m%dT%H%M%SZ)
  for f in data/pulse.db data/pulse.db-wal data/pulse.db-shm; do [ -e "$f" ] && mv "$f" "$f.pre-restore-$ts"; done
  mv restore.db data/pulse.db
  echo "restored; the previous database was kept as data/pulse.db.pre-restore-$ts"'
