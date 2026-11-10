#!/usr/bin/env bash
# Self-test for publish-models.sh (`just deploy-check` and CI run it). Needs bash and sha256sum. The route check is
# stubbed and the "server" is a temp dir (PUBLISH_MODELS_DIR), so nothing touches the network or a real server.
# `check && pass … || fail …` throughout: pass only echoes, so it never falls through to fail.
# shellcheck disable=SC2015
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
failed=0

pass() { echo "ok   $*"; }
fail() {
  echo "FAIL $*"
  failed=1
}

# shellcheck source=/dev/null
PUBLISH_MODELS_LIB=1 source "$here/publish-models.sh"
set +e # the library turned it on; each case checks its own results

# --- the real pins: the app's hash and the libDF rev are where the script reads them
PUBLISH_MODELS_DIR='' PUBLISH_MODELS_SHA='' pinned_sha | grep -Eq '^[0-9a-f]{64}$' && pass "pinned_sha reads LOW_DELAY_SHA256" || fail "pinned_sha: '$(pinned_sha)'"
libdf_rev | grep -Eq '^[0-9a-f]{40}$' && pass "libdf_rev reads Cargo.toml's rev" || fail "libdf_rev: '$(libdf_rev)'"

model=DeepFilterNet3_ll_onnx.tar.gz # the library's name, which the publish uses
head -c 300000 /dev/urandom >"$tmp/model"
good=$(sha256sum "$tmp/model" | cut -d' ' -f1)
# shellcheck disable=SC2329 # replaces the library's: publish_main calls it
route_code() { echo 401; } # the live route, guarded

# run ARGS…: publish_main in a subshell (it exits on a refusal) against a fresh fake server, status in $?
run() {
  (
    export PUBLISH_MODELS_DIR=$tmp/srv PUBLISH_MODELS_SRC=$tmp/model PUBLISH_MODELS_SHA=$good
    "$@"
    publish_main
  ) >"$tmp/out" 2>&1
}

# --- a good publish lands exactly the file, with nothing left in .incoming
rm -rf "$tmp/srv"
if run true; then
  [ "$(sha256sum "$tmp/srv/models/$model" | cut -d' ' -f1)" = "$good" ] && pass "publishes the file" || fail "the published file differs"
  [ ! -e "$tmp/srv/models/.incoming" ] && pass "nothing left in .incoming" || fail ".incoming left behind"
  grep -q "published $model" "$tmp/out" && pass "says published" || fail "no 'published' line: $(cat "$tmp/out")"
else
  fail "a good publish was refused: $(cat "$tmp/out")"
fi

# --- again: safe, same file
run true && [ "$(sha256sum "$tmp/srv/models/$model" | cut -d' ' -f1)" = "$good" ] && pass "re-running is safe" || fail "re-running: $(cat "$tmp/out")"

# --- a file that isn't the pinned one is refused before anything is uploaded
rm -rf "$tmp/srv"
if run export PUBLISH_MODELS_SHA=0000000000000000000000000000000000000000000000000000000000000000; then
  fail "published a file with the wrong hash"
else
  [ ! -e "$tmp/srv/models/$model" ] && pass "refuses a wrong hash, uploads nothing" || fail "uploaded despite a wrong hash"
fi

# --- a cut-short upload never goes live, and an earlier good copy stays
rm -rf "$tmp/srv"
run true
cat >"$tmp/shortcp" <<'EOF'
#!/usr/bin/env bash
head -c 1000 "$1" >"$2"
EOF
chmod +x "$tmp/shortcp"
if run export PUBLISH_MODELS_CP="$tmp/shortcp"; then
  fail "a cut-short upload was accepted"
else
  [ "$(sha256sum "$tmp/srv/models/$model" | cut -d' ' -f1)" = "$good" ] && pass "a cut-short upload leaves the good copy" || fail "a cut-short upload replaced the good copy"
  [ ! -e "$tmp/srv/models/.incoming/$model" ] && pass "the cut-short copy is deleted" || fail "the cut-short copy stayed in .incoming"
fi

# --- no live route: refused before anything
rm -rf "$tmp/srv"
if run eval 'route_code() { echo 404; }'; then
  fail "published without a live route"
else
  [ ! -e "$tmp/srv/models/$model" ] && grep -q "deploy the server first" "$tmp/out" && pass "refuses without a live route" || fail "no-route refusal: $(cat "$tmp/out")"
fi

# --- a missing source file is refused
rm -rf "$tmp/srv"
if run export PUBLISH_MODELS_SRC="$tmp/nope"; then fail "published a missing file"; else pass "refuses a missing source"; fi

exit "$failed"
