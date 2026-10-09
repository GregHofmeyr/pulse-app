#!/usr/bin/env bash
# Restore from an encrypted backup. Runs on your machine (needs Tailscale + the age private key).
#   restore.sh drill [OBJ]   download, decrypt, integrity-check locally; the server is untouched
#   restore.sh apply OBJ     …then replace the live database (pulse-server stops for a few seconds)
set -euo pipefail
PROD=${PULSE_PROD:-pulse@pulse}
KEY=${PULSE_BACKUP_KEY:-$HOME/.config/pulse/backup-age.key}
mode=${1:-}
name=${2:-}
case $mode in
  drill) ;;
  apply) [[ -n $name ]] || { echo "apply needs an explicit backup name" >&2; exit 2; } ;;
  *) echo "usage: restore.sh drill [OBJ] | restore.sh apply OBJ" >&2; exit 2 ;;
esac
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

[[ -n $name ]] || name=$(ssh "$PROD" '/srv/pulse/backup.sh latest')
echo "backup: $name"
# shellcheck disable=SC2029 # $name is meant to expand here, on this machine
ssh "$PROD" "/srv/pulse/backup.sh fetch '$name'" >"$work/backup.age"
age -d -i "$KEY" -o "$work/pulse.db" "$work/backup.age"
check=$(sqlite3 "$work/pulse.db" 'PRAGMA integrity_check;')
[[ $check == ok ]] || { echo "integrity check failed: $check" >&2; exit 1; }
echo "integrity ok: $(sqlite3 "$work/pulse.db" 'SELECT COUNT(*) FROM users;') users, $(sqlite3 "$work/pulse.db" 'SELECT COUNT(*) FROM messages;') messages"
[[ $mode == drill ]] && exit 0

scp -q "$work/pulse.db" "$PROD:/srv/pulse/restore.db"
ssh "$PROD" 'set -e; cd /srv/pulse
  docker compose stop pulse-server
  ts=$(date -u +%Y%m%dT%H%M%SZ)
  for f in data/pulse.db data/pulse.db-wal data/pulse.db-shm; do [ -e "$f" ] && mv "$f" "$f.pre-restore-$ts"; done
  mv restore.db data/pulse.db
  docker compose start pulse-server
  echo "restored; the previous database was kept as data/pulse.db.pre-restore-$ts"'
