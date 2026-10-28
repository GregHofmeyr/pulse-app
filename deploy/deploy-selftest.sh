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

# expect_clips ok|refused NAMES FILE-LINES: check_clips_env on a .env holding the lines. NAMES (refused only) are the
# variables the message must name, space separated; for ok the message must stay empty.
expect_clips() {
  local want=$1 names=$2 lines=$3 got out n
  printf '%s' "$lines" >"$tmp/.env"
  if out=$(check_clips_env "$tmp/.env" 2>&1); then got=ok; else got=refused; fi
  if [ "$got" != "$want" ]; then
    echo "FAIL clips want $want, got $got: ${lines//$'\n'/ }"
    failed=1
    return
  fi
  for n in $names; do
    if [[ $out != *"$n"* ]]; then
      echo "FAIL clips message doesn't name $n: $out"
      failed=1
      return
    fi
  done
  echo "ok   clips $want${names:+ (names $names)}: ${lines//$'\n'/ }"
}
cb=CLIPS_R2_BUCKET cid=CLIPS_R2_ACCESS_KEY_ID csec=CLIPS_R2_SECRET_ACCESS_KEY
expect_clips ok '' ''
expect_clips ok '' "R2_ACCOUNT_ID=<account-id>
$cb=
$cid=
$csec=
"
expect_clips ok '' "R2_ACCOUNT_ID=acct
$cb=pulse-clips
$cid=kid
$csec=sec
"
expect_clips refused "$cb" "R2_ACCOUNT_ID=acct
$cb=
$cid=kid
$csec=sec
"
expect_clips refused "$cid" "R2_ACCOUNT_ID=acct
$cb=pulse-clips
$cid=
$csec=sec
"
expect_clips refused "$csec" "R2_ACCOUNT_ID=acct
$cb=pulse-clips
$cid=kid
"
expect_clips refused "$cid $csec" "R2_ACCOUNT_ID=acct
$cb=pulse-clips
"
expect_clips refused "R2_ACCOUNT_ID" "$cb=pulse-clips
$cid=kid
$csec=sec
"
expect_clips refused "R2_ACCOUNT_ID" "R2_ACCOUNT_ID=
$cb=pulse-clips
$cid=kid
$csec=sec
"
# the secret's value is never echoed
printf 'R2_ACCOUNT_ID=acct\n%s=b\n%s=\n%s=topsecretvalue\n' "$cb" "$cid" "$csec" >"$tmp/.env"
if out=$(check_clips_env "$tmp/.env" 2>&1) || [[ $out == *topsecretvalue* ]]; then
  echo "FAIL clips check echoed a secret or accepted a half set"
  failed=1
else
  echo "ok   clips message never echoes values"
fi
if check_clips_env "$tmp/missing.env" 2>/dev/null; then
  echo "FAIL check_clips_env accepted a missing .env"
  failed=1
else
  echo "ok   clips on a missing .env is refused"
fi
# --check-env runs it too (a good node ip and key, half-filled clips)
printf 'LIVEKIT_NODE_IP=203.0.113.10\nPULSE_TOTP_KEY=%s\nR2_ACCOUNT_ID=acct\n%s=b\n' "$key" "$cb" >"$tmp/halfclips.env"
if bash "$here/deploy.sh" --check-env "$tmp/halfclips.env" 2>/dev/null; then
  echo "FAIL --check-env accepted half-filled clips settings"
  failed=1
else
  echo "ok   --check-env refuses half-filled clips settings"
fi
# the shipped example: clips blanks are all-empty (clips off), and compose passes them through
if check_clips_env "$here/env.example"; then
  echo "ok   env.example's clips block is all blank"
else
  echo "FAIL env.example's clips block isn't all blank"
  failed=1
fi
for v in CLIPS_R2_BUCKET CLIPS_R2_ACCESS_KEY_ID CLIPS_R2_SECRET_ACCESS_KEY; do
  if ! grep -q "^$v=\$" "$here/env.example" || ! grep -qF "\${$v:-}" "$here/compose.yml"; then
    echo "FAIL $v missing from env.example or compose.yml"
    failed=1
  fi
done
# shellcheck disable=SC2016 # a literal compose expression
endpoint='PULSE_CLIPS_S3_ENDPOINT: https://${R2_ACCOUNT_ID:-unset}.r2.cloudflarestorage.com'
if ! grep -qF "$endpoint" "$here/compose.yml"; then
  echo "FAIL compose.yml doesn't pass the clips endpoint"
  failed=1
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

# Hardening in Caddyfile and compose.yml (SECURITY.md section 5). Each check_* takes the file so the drift
# tests below can hand it a broken copy.
# caddy_global FILE: the global options block (from the first line to the first '}' in column 0).
caddy_global() { sed -n '1,/^}/p' "$1"; }
# caddy_site FILE NAME: one site block, from its "NAME {" line to the next '}' in column 0.
caddy_site() { awk -v name="$2 {" '$0 == name { on = 1 } on { print } on && /^}/ { exit }' "$1"; }
# check_caddy_admin FILE: the admin API is off or on a unix socket under /config (the caddy container's own
# config volume), never the default TCP localhost:2019 that every host-network container could reach.
check_caddy_admin() {
  caddy_global "$1" | grep -Eq '^[[:space:]]*admin[[:space:]]+(off|unix//config/[^[:space:]]+)[[:space:]]*$'
}
# check_caddy_twirp FILE: the voice site refuses LiveKit's server API (/twirp/*) before proxying anything.
check_caddy_twirp() {
  caddy_site "$1" "voice.{\$PULSE_DOMAIN}" |
    awk '/^[[:space:]]*respond \/twirp\/\* 404[[:space:]]*$/ { blocked = 1 } /reverse_proxy/ { exit !blocked } END { if (!blocked) exit 1 }'
}
# compose_service FILE NAME: one service's block (its "  NAME:" line up to the next 2-space-indented key).
compose_service() {
  awk -v name="  $2:" '$0 == name { on = 1; print; next } on && /^  [^ #]/ { exit } on { print }' "$1"
}
# check_compose_hardening FILE: every service drops all capabilities and has a pids and memory ceiling; all but
# caddy set no-new-privileges; caddy gets back NET_BIND_SERVICE only.
check_compose_hardening() {
  local svc block pids bad=0
  for svc in pulse-server livekit caddy; do
    block=$(compose_service "$1" "$svc")
    if [ -z "$block" ]; then echo "$svc: no such service" >&2; bad=1; continue; fi
    grep -Eq '^ +cap_drop: \[ALL\]$' <<<"$block" || { echo "$svc: cap_drop [ALL] missing" >&2; bad=1; }
    grep -Eq '^ +pids_limit: [0-9]+$' <<<"$block" || { echo "$svc: pids_limit missing" >&2; bad=1; }
    # the server's tokio blocking pool alone can reach 512 threads; LiveKit runs many goroutine threads too
    if [ "$svc" != caddy ]; then
      pids=$(sed -n 's/^ *pids_limit: *\([0-9]*\)$/\1/p' <<<"$block")
      [ "${pids:-0}" -ge 2048 ] || { echo "$svc: pids_limit ${pids:-?} is below 2048" >&2; bad=1; }
    fi
    grep -Eq '^ +mem_limit: [0-9]+[mg]$' <<<"$block" || { echo "$svc: mem_limit missing" >&2; bad=1; }
    if [ "$svc" = caddy ]; then
      grep -Eq '^ +cap_add: \[NET_BIND_SERVICE\]$' <<<"$block" || { echo "caddy: cap_add [NET_BIND_SERVICE] missing" >&2; bad=1; }
    else
      grep -Eq '^ +security_opt: \["no-new-privileges:true"\]$' <<<"$block" || { echo "$svc: no-new-privileges missing" >&2; bad=1; }
      if grep -Eq '^ +cap_add:' <<<"$block"; then echo "$svc: unexpected cap_add" >&2; bad=1; fi
    fi
  done
  return "$bad"
}
# check_caddy_config_volume FILE: the admin socket's directory is the caddy service's /config volume.
check_caddy_config_volume() {
  compose_service "$1" caddy | grep -Eq '^ +- \./caddy/config:/config$'
}
# result STATUS DESCRIPTION: STATUS 0 is a pass.
result() {
  if [ "$1" -eq 0 ]; then echo "ok   $2"; else echo "FAIL $2"; failed=1; fi
}
caddyfile=$root/deploy/Caddyfile
compose=$root/deploy/compose.yml
rc=0; check_caddy_admin "$caddyfile" || rc=$?
result "$rc" "Caddy admin API on a unix socket under /config (or off)"
rc=0; check_caddy_twirp "$caddyfile" || rc=$?
result "$rc" "Caddy refuses /twirp/* on the voice site"
rc=0; check_compose_hardening "$compose" || rc=$?
result "$rc" "compose: caps dropped, no-new-privileges, pids and memory ceilings"
rc=0; check_caddy_config_volume "$compose" || rc=$?
result "$rc" "compose: caddy mounts its /config volume (the admin socket's home)"
# and the checks catch drift: each must refuse a copy broken by the given sed expression.
# refused DESCRIPTION: the previous check (rc) must have failed.
refused() { if [ "$rc" -ne 0 ]; then echo "ok   drift caught: $1"; else echo "FAIL drift not caught: $1"; failed=1; fi; }
drift=$tmp/drift
sed 's#^\([[:space:]]*admin\) .*#\1 localhost:2019#' "$caddyfile" >"$drift"
rc=0; check_caddy_admin "$drift" || rc=$?; refused "admin back on TCP"
sed '/^[[:space:]]*admin /d' "$caddyfile" >"$drift"
rc=0; check_caddy_admin "$drift" || rc=$?; refused "no admin line"
sed '/respond \/twirp/d' "$caddyfile" >"$drift"
rc=0; check_caddy_twirp "$drift" || rc=$?; refused "/twirp/ not blocked"
sed '/respond \/twirp/d; s#^\([[:space:]]*\)reverse_proxy 127.0.0.1:7880$#&\n\1respond /twirp/* 404#' "$caddyfile" >"$drift"
rc=0; check_caddy_twirp "$drift" || rc=$?; refused "/twirp/ blocked after the proxy"
sed '/^  livekit:/,/^  caddy:/{/cap_drop/d}' "$compose" >"$drift"
rc=0; check_compose_hardening "$drift" 2>/dev/null || rc=$?; refused "livekit keeps its capabilities"
sed '/^  pulse-server:/,/^  livekit:/{/security_opt/d}' "$compose" >"$drift"
rc=0; check_compose_hardening "$drift" 2>/dev/null || rc=$?; refused "server without no-new-privileges"
sed '/^  pulse-server:/,/^  livekit:/{/mem_limit/d}' "$compose" >"$drift"
rc=0; check_compose_hardening "$drift" 2>/dev/null || rc=$?; refused "server without a memory limit"
sed '/^  pulse-server:/,/^  livekit:/{s/pids_limit: .*/pids_limit: 512/}' "$compose" >"$drift"
rc=0; check_compose_hardening "$drift" 2>/dev/null || rc=$?; refused "server pids limit back at 512"
sed '/^  livekit:/,/^  caddy:/{s/pids_limit: .*/pids_limit: 512/}' "$compose" >"$drift"
rc=0; check_compose_hardening "$drift" 2>/dev/null || rc=$?; refused "livekit pids limit back at 512"
# caddy is the last service: its block runs to the end of the file
sed '/^  caddy:/,/^NOMATCH/{/pids_limit/d}' "$compose" >"$drift"
rc=0; check_compose_hardening "$drift" 2>/dev/null || rc=$?; refused "caddy without a pids limit"
sed '/^  caddy:/,/^NOMATCH/{/cap_add/d}' "$compose" >"$drift"
rc=0; check_compose_hardening "$drift" 2>/dev/null || rc=$?; refused "caddy without NET_BIND_SERVICE"
sed '/^  caddy:/,/^NOMATCH/{/caddy\/config:/d}' "$compose" >"$drift"
rc=0; check_caddy_config_volume "$drift" || rc=$?; refused "caddy without its /config volume"
# reload_caddy: a failed reload warns on stderr but doesn't stop the deploy; a good one prints nothing.
# docker is stubbed: each case defines what `docker compose exec ... caddy reload` returns.
# shellcheck disable=SC2329 # invoked by reload_caddy
docker() { echo "docker output that must stay hidden"; echo "docker error that must stay hidden" >&2; return "$docker_rc"; }
docker_rc=1
rc=0; reload_caddy >"$tmp/out" 2>"$tmp/err" || rc=$?
if [ "$rc" -eq 0 ] && [ ! -s "$tmp/out" ] && grep -q 'WARNING: caddy reload failed' "$tmp/err" &&
  ! grep -q 'must stay hidden' "$tmp/err"; then
  echo "ok   a failed caddy reload warns on stderr and the deploy goes on"
else
  echo "FAIL a failed caddy reload: rc=$rc, stdout '$(cat "$tmp/out")', stderr '$(cat "$tmp/err")'"
  failed=1
fi
docker_rc=0
rc=0; reload_caddy >"$tmp/out" 2>"$tmp/err" || rc=$?
if [ "$rc" -eq 0 ] && [ ! -s "$tmp/out" ] && [ ! -s "$tmp/err" ]; then
  echo "ok   a good caddy reload is quiet"
else
  echo "FAIL a good caddy reload: rc=$rc, stdout '$(cat "$tmp/out")', stderr '$(cat "$tmp/err")'"
  failed=1
fi
# prune_old_images keeps the 3 newest server images (docker lists newest first; a tag pair shares one id)
# shellcheck disable=SC2329 # invoked by prune_old_images
docker() {
  case $1 in
    images) printf '%s\n' id5 id4 id4 id3 id2 id1 ;;
    rmi) shift 2; echo "$*" >"$tmp/removed" ;;
  esac
}
rm -f "$tmp/removed"
prune_old_images
if [ "$(cat "$tmp/removed" 2>/dev/null)" = "id2 id1" ]; then
  echo "ok   prune keeps the 3 newest images"
else
  echo "FAIL prune removed '$(cat "$tmp/removed" 2>/dev/null)', want 'id2 id1'"
  failed=1
fi
unset -f docker

exit "$failed"
