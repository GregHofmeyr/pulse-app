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
expect ok 'LIVEKIT_NODE_IP=223.255.255.254'
expect refused 'LIVEKIT_NODE_IP=224.0.0.1'
expect refused 'LIVEKIT_NODE_IP=239.255.255.255'
expect refused 'LIVEKIT_NODE_IP=240.0.0.1'
expect refused 'LIVEKIT_NODE_IP=255.255.255.255'
# expect_totp ok|refused LINES: check_totp_key on a .env holding LINES (empty LINES = an empty file)
expect_totp() {
  local want=$1 lines=$2 got out
  printf '%s' "$lines" >"$tmp/.env"
  if out=$(check_totp_key "$tmp/.env" 2>&1); then got=ok; else got=refused; fi
  if [ "$got" = "$want" ]; then
    echo "ok   totp $want: ${lines:-<empty file>}"
  else
    echo "FAIL totp want $want, got $got: $lines"
    failed=1
  fi
  # the key itself is never echoed
  local val=${lines#PULSE_TOTP_KEY=}
  val=${val%%$'\n'*}
  if [ "${#val}" -ge 8 ] && [[ $out == *"$val"* ]]; then
    echo "FAIL totp message echoed the value"
    failed=1
  fi
}
key=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
expect_totp ok "PULSE_TOTP_KEY=$key
"
expect_totp ok "PULSE_TOTP_KEY=0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF
"
expect_totp refused ''
expect_totp refused "LIVEKIT_NODE_IP=203.0.113.10
"
expect_totp refused "PULSE_TOTP_KEY=
"
expect_totp refused "PULSE_TOTP_KEY=${key:0:63}
"
expect_totp refused "PULSE_TOTP_KEY=${key}0
"
expect_totp refused "PULSE_TOTP_KEY=${key:0:63}g
"
expect_totp refused "PULSE_TOTP_KEY=$key 
"
if out=$(check_totp_key "$tmp/missing.env" 2>&1); then
  echo "FAIL check_totp_key accepted a missing .env"
  failed=1
else
  echo "ok   totp on a missing .env: $out"
fi
# --check-env mode (what `just deploy` runs over ssh before copying anything): exit status only.
if out=$(bash "$here/deploy.sh" --check-env "$tmp/missing.env" 2>&1); then
  echo "FAIL --check-env accepted a missing .env"
  failed=1
elif [[ $out != *"not found"* || $out == *"No such file"* ]]; then
  echo "FAIL --check-env on a missing .env should say so in its own words, said: $out"
  failed=1
else
  echo "ok   --check-env on a missing .env: $out"
fi
printf 'LIVEKIT_NODE_IP=203.0.113.10\nPULSE_TOTP_KEY=%s\n' "$key" >"$tmp/good.env"
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
printf 'LIVEKIT_NODE_IP=203.0.113.10\n' >"$tmp/nokey.env"
if bash "$here/deploy.sh" --check-env "$tmp/nokey.env" 2>/dev/null; then
  echo "FAIL --check-env accepted a .env without PULSE_TOTP_KEY"
  failed=1
else
  echo "ok   --check-env refuses a missing PULSE_TOTP_KEY"
fi

# The three LiveKit copies move together: deploy/compose.yml (prod), docker-compose.yml (dev and
# `just voice-it`) and release-windows.yml (the Windows test kit's binary, by version + SHA-256).
# livekit_pins_agree PROD_COMPOSE DEV_COMPOSE RELEASE_WORKFLOW: the two image refs are identical
# and the workflow's LIVEKIT_VERSION is their tag.
livekit_pins_agree() {
  local prod dev kit
  prod=$(sed -n 's#^ *image: *\(livekit/livekit-server:.*\)$#\1#p' "$1")
  dev=$(sed -n 's#^ *image: *\(livekit/livekit-server:.*\)$#\1#p' "$2")
  kit=$(sed -n 's#^ *LIVEKIT_VERSION: *\([^ ]*\) *$#\1#p' "$3")
  if [ -z "$prod" ] || [ "$prod" != "$dev" ]; then
    echo "LiveKit image differs: prod '$prod', dev '$dev'" >&2
    return 1
  fi
  if [[ $prod != "livekit/livekit-server:v$kit@sha256:"* ]] || [ -z "$kit" ]; then
    echo "release-windows.yml LIVEKIT_VERSION '$kit' isn't the compose tag ($prod)" >&2
    return 1
  fi
}
root=$(cd "$here/.." && pwd)
if livekit_pins_agree "$root/deploy/compose.yml" "$root/docker-compose.yml" "$root/.github/workflows/release-windows.yml"; then
  echo "ok   LiveKit pinned alike in deploy/compose.yml, docker-compose.yml, release-windows.yml"
else
  echo "FAIL LiveKit pins drifted (bump all three together, see deploy/README.md)"
  failed=1
fi
# and the check itself catches drift
sed 's#livekit-server:v[0-9.]*@#livekit-server:v0.0.1@#' "$root/docker-compose.yml" >"$tmp/dev-compose.yml"
sed 's#^\( *LIVEKIT_VERSION:\).*#\1 0.0.1#' "$root/.github/workflows/release-windows.yml" >"$tmp/release.yml"
if livekit_pins_agree "$root/deploy/compose.yml" "$tmp/dev-compose.yml" "$root/.github/workflows/release-windows.yml" 2>/dev/null; then
  echo "FAIL a dev LiveKit image that differs from prod passed"
  failed=1
elif livekit_pins_agree "$root/deploy/compose.yml" "$root/docker-compose.yml" "$tmp/release.yml" 2>/dev/null; then
  echo "FAIL a release-windows LIVEKIT_VERSION that differs from the compose tag passed"
  failed=1
else
  echo "ok   LiveKit drift is caught"
fi
exit "$failed"
