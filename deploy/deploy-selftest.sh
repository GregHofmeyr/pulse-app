#!/usr/bin/env bash
# Self-test for deploy.sh's and verify-image.sh's helpers (`just deploy-check` and CI run it). Needs bash and jq;
# docker, curl and gh are stubbed, nothing touches the network.
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

# --- image provenance: deploy.sh TAG DIGEST|--unverified, deploy/verify-image.sh, and the justfile's deploy recipes ---
d64=sha256:$(printf 'a%.0s' {1..64})
other64=sha256:$(printf 'b%.0s' {1..64})
# pass DESCRIPTION / fail DESCRIPTION
pass() { echo "ok   $1"; }
fail() { echo "FAIL $1"; failed=1; }

# check_args TAG PIN (deploy.sh)
expect_args() {
  local want=$1 got; shift
  if check_args "$@" 2>/dev/null; then got=ok; else got=refused; fi
  if [ "$got" = "$want" ]; then pass "args $want: $*"; else fail "args want $want, got $got: $*"; fi
}
expect_args ok latest "$d64"
expect_args ok a5959d1 --unverified
expect_args refused latest --unverified
expect_args refused v1 --unverified
expect_args refused latest
expect_args refused latest ''
expect_args refused '' "$d64"
expect_args refused latest "${d64:0:70}"
expect_args refused latest "${d64}0"
expect_args refused latest "SHA256:${d64#sha256:}"
expect_args refused latest "sha256:$(printf 'A%.0s' {1..64})"
expect_args refused latest "$d64;reboot"
expect_args refused latest unverified
expect_args refused 'latest;reboot' "$d64"
expect_args refused '-latest' "$d64"
# deploy.sh itself stops on bad arguments before it goes near /srv/pulse (not present here: a cd would say so)
for args in "latest" "latest sha256:abc" "x;y --unverified" "latest --unverified"; do
  # shellcheck disable=SC2086 # split on purpose
  if out=$(bash "$here/deploy.sh" $args 2>&1); then
    fail "deploy.sh accepted: $args"
  elif [[ $out == *"No such file"* || $out == *"/srv/pulse"* ]]; then
    fail "deploy.sh got past its argument check: $args: $out"
  else
    pass "deploy.sh refuses before touching anything: $args"
  fi
done

# pull_image with docker stubbed. The stub answers `image inspect` only for exactly stub_ref (anything else is "No such
# image", as docker says), with inspect_out as the RepoDigests and stub_rev as the revision label.
# shellcheck disable=SC2329 # invoked by pull_image
docker() {
  echo "PULSE_IMAGE=${PULSE_IMAGE:-} $*" >>"$tmp/docker.log"
  case $1 in
    compose) return "$pull_rc" ;;
    image)
      if [ "$2" != inspect ] || [ "$3" != --format ] || [ "$5" != "$stub_ref" ]; then
        echo "Error: No such image: ${5:-}" >&2
        return 1
      fi
      case $4 in
        *RepoDigests*) [ "${#inspect_out[@]}" -eq 0 ] || printf '%s\n' "${inspect_out[@]}" ;;
        *image.revision*) echo "$stub_rev" ;;
        *) return 1 ;;
      esac ;;
    tag) return 0 ;;
    *) return 1 ;;
  esac
}
# expect_pull ok|refused WANT-OUTPUT DESCRIPTION TAG PIN
expect_pull() {
  local want=$1 want_out=$2 desc=$3 got out
  shift 3
  : >"$tmp/docker.log"
  if out=$(pull_image "$@" 2>/dev/null); then got=ok; else got=refused; fi
  if [ "$got" = "$want" ] && [ "$out" = "$want_out" ]; then
    pass "pull $want: $desc"
  else
    fail "pull $desc: want $want '$want_out', got $got '$out'"
  fi
}
# docker_saw LINE: the stub was called with exactly LINE
docker_saw() { grep -qxF "$1" "$tmp/docker.log"; }
# shellcheck disable=SC2154 # repo comes from the sourced deploy.sh
stub_ref=$repo@$d64 pull_rc=0 stub_rev=abc1234def0 inspect_out=("$repo@$other64" "$repo@$d64")
expect_pull ok "$repo:latest@$d64" "a verified digest is pulled and pinned exactly" latest "$d64"
if docker_saw "PULSE_IMAGE=$repo@$d64 compose pull -q" && docker_saw "PULSE_IMAGE= tag $repo@$d64 $repo:latest"; then
  pass "pull by digest (never the tag), then tagged locally as the tag"
else
  fail "pull by digest: docker saw $(tr '\n' '|' <"$tmp/docker.log")"
fi
if docker_saw "PULSE_IMAGE= tag $repo@$d64 $repo:abc1234"; then
  pass "the deployed image is also tagged with its own commit (so a moved latest doesn't leave it dangling)"
else
  fail "no commit tag: docker saw $(tr '\n' '|' <"$tmp/docker.log")"
fi
inspect_out=("$repo@$other64")
expect_pull refused "" "the pulled image isn't known by the verified digest" latest "$d64"
if grep -q ' tag ' "$tmp/docker.log"; then fail "a mismatched image was tagged"; fi
pull_rc=1 inspect_out=("$repo@$d64")
expect_pull refused "" "a failed pull doesn't fall through to a local copy (digest)" latest "$d64"
stub_ref=$repo:a5959d1
expect_pull refused "" "a failed pull doesn't fall through to a local copy (--unverified)" a5959d1 --unverified
pull_rc=0 inspect_out=("$repo@$other64" "other/repo@$d64") stub_rev=a5959d1ffff
expect_pull ok "$repo:a5959d1@$other64" "--unverified pins whatever the tag resolves to" a5959d1 --unverified
if docker_saw "PULSE_IMAGE=$repo:a5959d1 compose pull -q" && docker_saw "PULSE_IMAGE= tag $repo:a5959d1 $repo:a5959d1"; then
  pass "--unverified pulls the tag and tags it with its commit"
else
  fail "--unverified pull: docker saw $(tr '\n' '|' <"$tmp/docker.log")"
fi
inspect_out=()
expect_pull refused "" "--unverified without a registry digest" a5959d1 --unverified
stub_ref=$repo@$d64 inspect_out=("$repo@$d64") stub_rev=
expect_pull ok "$repo:latest@$d64" "an image without a revision label still deploys" latest "$d64"
if grep -q " tag $repo@$d64 $repo:[0-9a-f]\{7\}$" "$tmp/docker.log"; then fail "tagged a commit from no label"; fi
# the stub really is exact: inspecting a different ref than the one pulled fails
stub_ref=$repo:latest inspect_out=("$repo@$d64")
expect_pull refused "" "inspect by any other ref than the pulled digest finds nothing" latest "$d64"
unset -f docker
# prune_old_images then keeps the newest 3 by image id: with every deployed image carrying its commit tag, the stub
# lists what docker would after deploys of c1..c5 (latest moved each time, no image left untagged).
# shellcheck disable=SC2329 # invoked by prune_old_images
docker() {
  case $1 in
    images) printf '%s\n' id5 id5 id4 id3 id2 id1 ;; # id5 is both latest and c5
    rmi) shift 2; echo "$*" >"$tmp/removed" ;;
    image) echo prune >>"$tmp/pruned" ;;
  esac
}
rm -f "$tmp/removed" "$tmp/pruned"
prune_old_images
if [ "$(cat "$tmp/removed" 2>/dev/null)" = "id2 id1" ]; then
  pass "prune keeps the 3 newest commit-tagged images and removes the rest"
else
  fail "prune removed '$(cat "$tmp/removed" 2>/dev/null)', want 'id2 id1'"
fi
unset -f docker

# --- verify-image.sh helpers ---
# shellcheck source=/dev/null
VERIFY_IMAGE_LIB=1 source "$here/verify-image.sh"
# expect_fn ok|refused FUNCTION ARGS...
expect_fn() {
  local want=$1 fn=$2 got; shift 2
  if "$fn" "$@" 2>/dev/null; then got=ok; else got=refused; fi
  if [ "$got" = "$want" ]; then pass "$fn $want: $*"; else fail "$fn want $want, got $got: $*"; fi
}
# _ship's checks (the last thing before a remote command line)
expect_fn ok check_ship_args latest "$d64"
expect_fn ok check_ship_args a5959d1 --unverified
expect_fn refused check_ship_args latest --unverified
expect_fn refused check_ship_args latest
expect_fn refused check_ship_args latest "${d64}0"
expect_fn refused check_ship_args latest "$d64 x"
expect_fn refused check_ship_args 'a;b' "$d64"
expect_fn refused check_ship_args '' "$d64"
expect_fn refused check_ship_args "\$(id)" --unverified
# deploy-unverified: a <sha7>, never latest, and CONFIRM must repeat it
expect_fn ok check_unverified_args a5959d1 a5959d1
expect_fn refused check_unverified_args a5959d1 ''
expect_fn refused check_unverified_args a5959d1 f15d174
expect_fn refused check_unverified_args latest latest
expect_fn refused check_unverified_args A5959D1 A5959D1
expect_fn refused check_unverified_args a5959d12 a5959d12
# header_value: case-insensitive names, CR stripped, parameters dropped
printf 'HTTP/2 200\r\nContent-Type: application/vnd.oci.image.index.v1+json; charset=utf-8\r\nDOCKER-CONTENT-DIGEST: %s\r\n\r\n' "$d64" >"$tmp/h"
if [ "$(header_value "$tmp/h" docker-content-digest)" = "$d64" ] &&
  [ "$(header_value "$tmp/h" content-type)" = application/vnd.oci.image.index.v1+json ] &&
  [ -z "$(header_value "$tmp/h" location)" ]; then
  pass "header_value reads headers in any case, without CR or parameters"
else
  fail "header_value: '$(header_value "$tmp/h" docker-content-digest)' '$(header_value "$tmp/h" content-type)'"
fi
# provenance_status on a throwaway history: A (no attest) -> B (adds the attest step) -> C, and D branched off A
g=$tmp/git
git init -q -b main "$g"
gitc() { git -C "$g" -c user.name=selftest -c user.email=selftest -c commit.gpgsign=false "$@"; }
mkdir -p "$g/.github/workflows"
echo build >"$g/.github/workflows/release-server.yml"
gitc add -A && gitc commit -q -m A && cA=$(gitc rev-parse --short=7 HEAD)
gitc checkout -q -b side && echo side >"$g/x" && gitc add -A && gitc commit -q -m D && cD=$(gitc rev-parse --short=7 HEAD)
gitc checkout -q main
echo '      - uses: actions/attest-build-provenance@0000000000000000000000000000000000000000 # v4' >>"$g/.github/workflows/release-server.yml"
gitc add -A && gitc commit -q -m B && cB=$(gitc rev-parse --short=7 HEAD)
echo c >"$g/y" && gitc add -A && gitc commit -q -m C && cC=$(gitc rev-parse --short=7 HEAD)
for want in "predates $cA" "predates $cD" "has $cB" "has $cC" "unknown 0000000"; do
  got=$(provenance_status "${want#* }" "$g")
  if [ "$got" = "${want%% *}" ]; then pass "provenance_status: ${want%% *} for $(
    case ${want#* } in "$cA") echo "a commit before the attest step" ;; "$cD") echo "a side branch without it" ;;
      "$cB") echo "the commit that adds it" ;; "$cC") echo "a later commit" ;; *) echo "a commit not in the checkout" ;; esac)"
  else fail "provenance_status ${want#* }: want ${want%% *}, got $got"; fi
done

# --- verify-image.sh end to end, with curl, gh and ssh stubbed (STUB_* steer them) and real jq/git ---
# The registry stub serves bodies from $blobs by digest; store BODY puts one there and prints its digest.
bin=$tmp/bin blobs=$tmp/blobs
mkdir -p "$bin" "$blobs"
store() { printf '%s' "$1" >"$tmp/body" && d=$(sha256_of "$tmp/body") && mv "$tmp/body" "$blobs/$d" && echo "$d"; }
# image REV1 REV2: an index of two platform images (plus an attestation manifest) labelled REV1, REV2; prints its digest
image() {
  local c1 c2 m1 m2 att
  c1=$(store "{\"config\":{\"Labels\":{\"org.opencontainers.image.revision\":\"$1\"}}}")
  c2=$(store "{\"config\":{\"Labels\":{\"org.opencontainers.image.revision\":\"$2\"},\"x\":2}}")
  m1=$(store "{\"config\":{\"digest\":\"$c1\"}}")
  m2=$(store "{\"config\":{\"digest\":\"$c2\"},\"x\":2}")
  att=$(store '{"attestation":true}')
  store "{\"mediaType\":\"application/vnd.oci.image.index.v1+json\",\"manifests\":[{\"digest\":\"$m1\",\"platform\":{\"os\":\"linux\"}},{\"digest\":\"$att\",\"platform\":{\"os\":\"unknown\"}},{\"digest\":\"$m2\",\"platform\":{\"os\":\"linux\"}}]}"
}
cat >"$bin/curl" <<'EOF'
#!/usr/bin/env bash
out= accept= head=0 url=
while [ $# -gt 0 ]; do
  case $1 in
    -o) out=$2; shift ;;
    -H) case $2 in Accept:*) accept=${2#Accept: } ;; esac; shift ;;
    -fsSI) head=1 ;;
    https://*) url=$1 ;;
  esac
  shift
done
echo "curl head=$head accept=$accept $url" >>"$STUB_LOG"
case $url in
  */token*) [ -z "${STUB_TOKEN_FAIL:-}" ] || exit 22; echo "${STUB_TOKEN_BODY:-{\"token\":\"t\"\}}" ;;
  */manifests/* | */blobs/*)
    if [ "$head" = 1 ]; then
      [ -n "$STUB_DIGEST" ] || exit 22
      printf 'HTTP/2 200\r\ncontent-type: %s\r\nDocker-Content-Digest: %s\r\n\r\n' \
        "${STUB_TYPE:-application/vnd.oci.image.index.v1+json}" "$STUB_DIGEST"
    else
      d=${url##*/}
      [ -f "$STUB_BLOBS/$d" ] || exit 22
      cat "$STUB_BLOBS/$d" >"${out:-/dev/stdout}"
      if [ "$d" = "${STUB_CORRUPT:-}" ]; then printf ' ' >>"${out:-/dev/stdout}"; fi
    fi ;;
  *) exit 22 ;;
esac
EOF
cat >"$bin/gh" <<'EOF'
#!/usr/bin/env bash
echo "gh $*" >>"$STUB_LOG"
echo "gh verification output" >&2
[ "$STUB_GH_RC" = 0 ] || exit "$STUB_GH_RC"
case " $* " in
  *" --format json "*) echo '[{"verificationResult":{"signature":{"certificate":{"sourceRepositoryRef":"refs/heads/spike"}}}}]' ;;
  *) echo "Verification succeeded!" ;;
esac
EOF
# shellcheck disable=SC2016 # expanded by the stub, not here
printf '#!/bin/sh\necho "ssh $*" >>"$STUB_LOG"; exit 1\n' >"$bin/ssh"
cp "$bin/ssh" "$bin/scp"
chmod +x "$bin/curl" "$bin/gh" "$bin/ssh" "$bin/scp"
export STUB_LOG=$tmp/stub.log STUB_BLOBS=$blobs
good=$(image abc1234def abc1234def)
# expect_verify ok|refused DESCRIPTION ARGS... [-- NAME=VALUE...]: verify-image.sh ARGS with the stubs; on ok stdout must
# be the index digest (STUB_DIGEST), on refusal empty with a REFUSED line. Never any ssh.
expect_verify() {
  local want=$1 desc=$2 got out args=() envs=()
  shift 2
  while [ $# -gt 0 ] && [ "$1" != -- ]; do args+=("$1"); shift; done
  [ $# -eq 0 ] || { shift; envs=("$@"); }
  : >"$STUB_LOG"
  if out=$(env PATH="$bin:$PATH" STUB_DIGEST="$good" STUB_GH_RC=0 VERIFY_IMAGE_GIT_DIR="$g" "${envs[@]}" \
    bash "$here/verify-image.sh" "${args[@]}" 2>"$tmp/verify.err"); then got=ok; else got=refused; fi
  if [ "$got" != "$want" ]; then
    fail "verify $desc: want $want, got $got: $(tr '\n' ' ' <"$tmp/verify.err")"
  elif [ "$want" = ok ] && [ "$out" != "$good" ]; then
    fail "verify $desc: stdout '$out', want only the digest"
  elif [ "$want" = refused ] && { [ -n "$out" ] || ! grep -q '^REFUSED: ' "$tmp/verify.err"; }; then
    fail "verify $desc: a refusal must print nothing on stdout and say REFUSED; stdout '$out'"
  elif grep -q '^ssh' "$STUB_LOG"; then
    fail "verify $desc: ran ssh"
  else
    pass "verify $want: $desc"
  fi
}
# err_has TEXT / err_lacks TEXT: about the last expect_verify's stderr
err_has() { if grep -qF -- "$1" "$tmp/verify.err"; then pass "  ...says: $1"; else fail "  ...doesn't say '$1': $(cat "$tmp/verify.err")"; fi; }
err_lacks() { if grep -qF -- "$1" "$tmp/verify.err"; then fail "  ...says '$1': $(cat "$tmp/verify.err")"; else pass "  ...doesn't say: $1"; fi; }
log_has() { if grep -qxF -- "$1" "$STUB_LOG"; then pass "  ...called: $2"; else fail "  ...no call '$1'; log: $(cat "$STUB_LOG")"; fi; }
no_gh() { if grep -q '^gh' "$STUB_LOG"; then fail "  ...gh ran"; else pass "  ...gh never ran"; fi; }

main_re='^https://github\.com/GregHofmeyr/pulse-app/\.github/workflows/release-server\.yml@refs/heads/main$'
branch_re='^https://github\.com/GregHofmeyr/pulse-app/\.github/workflows/release-server\.yml@refs/heads/.+$'
expect_verify ok "an attested tag gives its index digest" latest
log_has "gh attestation verify oci://$repo@$good --repo GregHofmeyr/pulse-app --cert-identity-regex $main_re --source-ref refs/heads/main --deny-self-hosted-runners" \
  "gh with this digest, this repo, release-server.yml anchored to main, source ref main, GitHub-hosted runners"
if grep '^curl head=1 ' "$STUB_LOG" | grep -qF 'application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json'; then
  pass "  ...the HEAD request accepts the index types"
else
  fail "  ...the HEAD request's Accept lacks the index types: $(grep '^curl head=1' "$STUB_LOG")"
fi
expect_verify refused "no main attestation, latest" latest -- STUB_GH_RC=1
err_has "moving tag"
err_lacks "deploy-unverified"
expect_verify refused "no attestation, a commit from before provenance" "$cA" -- STUB_GH_RC=1
err_has "CONFIRM=$cA just deploy-unverified $cA"
expect_verify refused "no attestation, a side-branch commit without the attest step" "$cD" -- STUB_GH_RC=1
err_has "just deploy-unverified $cD"
expect_verify refused "no attestation, a commit that has the attest step" "$cC" -- STUB_GH_RC=1
err_lacks "deploy-unverified"
err_has "just deploy-branch $cC"
expect_verify refused "no attestation, a commit not in the checkout" 0000000 -- STUB_GH_RC=1
err_lacks "deploy-unverified"
err_has "isn't in this checkout"
expect_verify refused "a tag the registry doesn't have" f15d174 -- STUB_DIGEST=
no_gh
expect_verify refused "a registry digest that isn't sha256 hex" latest -- STUB_DIGEST=sha256:zz
expect_verify refused "a tag naming a single image, not an index" latest -- STUB_TYPE=application/vnd.oci.image.manifest.v1+json
err_has "single image"
no_gh
expect_verify refused "the registry token fetch fails" latest -- STUB_TOKEN_FAIL=1
no_gh
expect_verify refused "the registry gives no token" latest -- 'STUB_TOKEN_BODY={}'
expect_verify refused "not a tag" 'x;y'
if [ -s "$STUB_LOG" ]; then fail "  ...a bad tag reached the network: $(cat "$STUB_LOG")"; fi
expect_verify ok "a sha7 tag whose images carry that commit" abc1234
# a body that doesn't hash to the digest it was fetched by is never read
expect_verify refused "the index body doesn't match its digest" abc1234 -- "STUB_CORRUPT=$good"
err_has "doesn't hash to"
corrupt_cfg=$(jq -r '.config.digest' "$blobs/$(jq -r '.manifests[0].digest' "$blobs/$good")")
expect_verify refused "a config body doesn't match its digest" abc1234 -- "STUB_CORRUPT=$corrupt_cfg"
other=$(image 0000000def 0000000def)
expect_verify refused "a sha7 tag whose images carry another commit" abc1234 -- "STUB_DIGEST=$other"
mixed=$(image abc1234def 0000000def)
expect_verify refused "a sha7 tag with one platform from another commit" abc1234 -- "STUB_DIGEST=$mixed"
nolabel=$(image '' '')
expect_verify refused "a sha7 tag with no commit label" abc1234 -- "STUB_DIGEST=$nolabel"
# --branch: any branch of this repo's release-server.yml, says which, sha7 only
expect_verify ok "--branch: a hand-run branch build" --branch abc1234
log_has "gh attestation verify oci://$repo@$good --repo GregHofmeyr/pulse-app --cert-identity-regex $branch_re --format json --deny-self-hosted-runners" \
  "gh with release-server.yml on any branch, anchored"
err_has "from branch: refs/heads/spike"
expect_verify refused "--branch: latest" --branch latest
if [ -s "$STUB_LOG" ]; then fail "  ...--branch latest reached the network"; fi
expect_verify refused "--branch: no attestation" --branch "$cA" -- STUB_GH_RC=1
err_lacks "deploy-branch $cA"
# gh missing: a PATH holding only what the script needs before its tool check (and no gh)
nogh=$tmp/nogh
mkdir -p "$nogh"
ln -s "$bin/curl" "$nogh/curl"
for t in jq dirname; do ln -s "$(command -v "$t")" "$nogh/$t"; done
: >"$STUB_LOG"
if out=$(env PATH="$nogh" STUB_DIGEST="$good" "$BASH" "$here/verify-image.sh" latest 2>&1); then
  fail "verify-image.sh ran without gh: $out"
elif [[ $out == *"REFUSED: gh is not installed"* ]] && [ ! -s "$STUB_LOG" ]; then
  pass "verify refused: gh missing, before any network"
else
  fail "verify without gh: $out"
fi

# --- the workflow attests what verify-image.sh checks, and the justfile runs the checks before any ssh ---
release=$root/.github/workflows/release-server.yml
# shellcheck disable=SC2016 # literal workflow expressions
if [ "$(grep -c 'uses: actions/attest-build-provenance@[0-9a-f]\{40\} # v' "$release")" -ge 2 ] &&
  grep -q 'subject-digest: ${{ steps.merge.outputs.digest }}' "$release" &&
  grep -q 'subject-digest: ${{ steps.build.outputs.digest }}' "$release" &&
  [ "$(grep -c '^      id-token: write' "$release")" -eq 2 ] &&
  [ "$(grep -c '^      attestations: write' "$release")" -eq 2 ] &&
  ! sed -n '/^permissions:/,/^[a-z]/p' "$release" | grep -q 'write' &&
  grep -q -- '--metadata-file' "$release" &&
  sed -n '/^concurrency:/,/^[a-z]/p' "$release" | grep -q 'cancel-in-progress: false'; then
  pass "release-server.yml: SHA-pinned attest steps on the pushed digests, id-token+attestations write on both jobs only, one release at a time"
else
  fail "release-server.yml no longer attests what verify-image.sh checks (or its permissions/concurrency drifted)"
fi
# recipe NAME: one justfile recipe's body (its header line up to the next blank line)
recipe() { awk -v name="$1" 'index($0, name) == 1 && $0 ~ /:$/ { on = 1 } on && /^$/ { exit } on { print }' "$root/justfile"; }
# before_ssh NAME CHECK: CHECK appears in recipe NAME before its first ssh, scp or nested just
before_ssh() {
  recipe "$1" | awk -v check="$2" 'index($0, check) { seen = 1 } /(^|[ ;(])(ssh|scp|just) / { exit !seen } END { exit !seen }'
}
# shellcheck disable=SC2016 # literal recipe text
{
  rc=0; before_ssh '_ship ' 'check_ship_args "$tag" "$pin"' || rc=$?
  result "$rc" "justfile: _ship checks its tag and digest before any ssh"
  rc=0; before_ssh 'deploy-unverified ' 'check_unverified_args "$tag" "${CONFIRM:-}"' || rc=$?
  result "$rc" "justfile: deploy-unverified needs a sha7 and CONFIRM before anything else"
  rc=0; before_ssh 'deploy ' 'digest=$(bash deploy/verify-image.sh "$tag")' || rc=$?
  result "$rc" "justfile: deploy verifies (main) before shipping"
  rc=0; before_ssh 'deploy-branch ' 'digest=$(bash deploy/verify-image.sh --branch "$tag")' || rc=$?
  result "$rc" "justfile: deploy-branch verifies (any branch) before shipping"
}

exit "$failed"
