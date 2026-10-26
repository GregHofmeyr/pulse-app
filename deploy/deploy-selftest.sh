#!/usr/bin/env bash
# Self-test for deploy.sh's helpers (`just deploy-check` and CI run it). Needs nothing but bash.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=/dev/null
DEPLOY_SH_LIB=1 source "$here/deploy.sh"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
failed=0

# expect ok|refused LINE: check_node_ip on a .env holding LINE
expect() {
  local want=$1 line=$2 got
  printf '%s\n' "$line" >"$tmp/.env"
  if check_node_ip "$tmp/.env" 2>/dev/null; then got=ok; else got=refused; fi
  if [ "$got" = "$want" ]; then
    echo "ok   $want: $line"
  else
    echo "FAIL want $want, got $got: $line"
    failed=1
  fi
}

expect ok 'LIVEKIT_NODE_IP=203.0.113.10'
expect ok 'LIVEKIT_NODE_IP=8.8.4.4'
expect ok 'LIVEKIT_NODE_IP=100.63.255.255'
expect ok 'LIVEKIT_NODE_IP=100.128.0.1'
expect ok 'LIVEKIT_NODE_IP=172.15.0.1'
expect ok 'LIVEKIT_NODE_IP=172.32.0.1'
expect refused 'LIVEKIT_NODE_IP='
expect refused 'PULSE_TAG=latest'
expect refused 'LIVEKIT_NODE_IP=voice.example.com'
expect refused 'LIVEKIT_NODE_IP=203.0.113.10 '
expect refused 'LIVEKIT_NODE_IP=1.2.3'
expect refused 'LIVEKIT_NODE_IP=1.2.3.4.5'
expect refused 'LIVEKIT_NODE_IP=256.1.1.1'
expect refused 'LIVEKIT_NODE_IP=203.0.113.010'
expect refused 'LIVEKIT_NODE_IP=2001:db8::1'
expect refused 'LIVEKIT_NODE_IP=0.1.2.3'
expect refused 'LIVEKIT_NODE_IP=10.0.0.5'
expect refused 'LIVEKIT_NODE_IP=127.0.0.1'
expect refused 'LIVEKIT_NODE_IP=169.254.169.254'
expect refused 'LIVEKIT_NODE_IP=172.16.0.1'
expect refused 'LIVEKIT_NODE_IP=172.31.255.255'
expect refused 'LIVEKIT_NODE_IP=192.168.1.1'
expect refused 'LIVEKIT_NODE_IP=100.64.0.1'
expect refused 'LIVEKIT_NODE_IP=100.127.255.255'
# --check-env mode (what `just deploy` runs over ssh before copying anything): exit status only.
printf 'LIVEKIT_NODE_IP=203.0.113.10\n' >"$tmp/good.env"
printf 'LIVEKIT_NODE_IP=\n' >"$tmp/bad.env"
if bash "$here/deploy.sh" --check-env "$tmp/good.env"; then
  echo "ok   --check-env accepts"
else
  echo "FAIL --check-env refused a good .env"
  failed=1
fi
if bash "$here/deploy.sh" --check-env "$tmp/bad.env" 2>/dev/null; then
  echo "FAIL --check-env accepted a bad .env"
  failed=1
else
  echo "ok   --check-env refuses"
fi
exit "$failed"
