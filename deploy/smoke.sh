#!/usr/bin/env bash
# shellcheck disable=SC2329 # the helpers below run indirectly, through check()
# Checks a live Pulse server from the outside. Usage: smoke.sh [domain]
set -uo pipefail
DOMAIN=${1:-pulsechat.co.za}
ip=$(dig +short A "$DOMAIN" @1.1.1.1 | tail -n1)
fail=0

check() { # name command...
  local name=$1
  shift
  if "$@"; then echo "ok    $name"; else echo "FAIL  $name"; fail=1; fi
}
port_open() { timeout 3 bash -c "</dev/tcp/$ip/$1" 2>/dev/null; }
port_closed() { ! port_open "$1"; }
health_ok() { [[ $(curl -fsS "https://$DOMAIN/health") == ok ]]; }
voice_answers() { curl -fsS -o /dev/null "https://voice.$DOMAIN/"; }
webhook_hidden() { [[ $(curl -s -o /dev/null -w '%{http_code}' -X POST "https://$DOMAIN/livekit/webhook") == 404 ]]; }

echo "$DOMAIN → ${ip:-no A record}"
check "https://$DOMAIN/health" health_ok
check "https://voice.$DOMAIN answers" voice_answers
check "webhook hidden from the internet" webhook_hidden
check "port 7890 closed" port_closed 7890
check "port 7880 closed" port_closed 7880
check "LiveKit TCP fallback 7881 open" port_open 7881
exit $fail
