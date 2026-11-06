#!/usr/bin/env bash
# Self-test for publish-update.sh (`just deploy-check` and CI run it). Needs bash, jq, minisign and sha256sum. gh and
# curl are stubbed and the "server" is a temp dir (PUBLISH_UPDATE_DIR), so nothing touches the network or a real
# server. The Tauri signature case also needs client/ui/node_modules (skipped with a note without it).
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
PUBLISH_UPDATE_LIB=1 source "$here/publish-update.sh"
set +e # the library turned it on; each case checks its own results

# --- version_of
[ "$(version_of "$tmp/Pulse_0.2.1_x64-setup.exe" 2>/dev/null)" = 0.2.1 ] && pass "version_of 0.2.1" || fail "version_of 0.2.1"
for bad in Pulse_0.2_x64-setup.exe evil.exe Pulse_0.2.1_x64-setup.exe.sig "Pulse_0.2.1_x64-setup.exe;rm" ""; do
  if version_of "$bad" >/dev/null 2>&1; then fail "version_of accepted '$bad'"; else pass "version_of refuses '$bad'"; fi
done

# --- verify_sig, with minisign keys made here and wrapped in base64 as Tauri does
mkdir -p "$tmp/keys"
minisign -G -W -p "$tmp/keys/a.pub" -s "$tmp/keys/a.key" >/dev/null
minisign -G -W -p "$tmp/keys/b.pub" -s "$tmp/keys/b.key" >/dev/null
head -c 100000 /dev/urandom >"$tmp/file.exe"
minisign -S -s "$tmp/keys/a.key" -m "$tmp/file.exe" -x "$tmp/file.raw.sig" >/dev/null
base64 -w0 "$tmp/file.raw.sig" >"$tmp/file.exe.sig"
pub_a=$(base64 -w0 "$tmp/keys/a.pub")
pub_b=$(base64 -w0 "$tmp/keys/b.pub")
verify_sig "$tmp/file.exe" "$tmp/file.exe.sig" "$pub_a" >/dev/null 2>&1 && pass "verify_sig: good signature" || fail "verify_sig refused a good signature"
cp "$tmp/file.exe" "$tmp/flipped.exe"
printf '\x00' | dd of="$tmp/flipped.exe" bs=1 seek=500 conv=notrunc status=none
cmp -s "$tmp/file.exe" "$tmp/flipped.exe" && printf '\x01' | dd of="$tmp/flipped.exe" bs=1 seek=500 conv=notrunc status=none
if verify_sig "$tmp/flipped.exe" "$tmp/file.exe.sig" "$pub_a" >/dev/null 2>&1; then fail "verify_sig accepted a changed file"; else pass "verify_sig refuses a changed file"; fi
if verify_sig "$tmp/file.exe" "$tmp/file.exe.sig" "$pub_b" >/dev/null 2>&1; then fail "verify_sig accepted another key"; else pass "verify_sig refuses another key"; fi
if verify_sig "$tmp/file.exe" "$tmp/file.exe.sig" "not base64 !!" >/dev/null 2>&1; then fail "verify_sig accepted a junk key"; else pass "verify_sig refuses a junk key"; fi

# --- tauri_signature_verifies: a real `tauri signer` signature (Linux end to end)
tauri_js="$here/../client/ui/node_modules/@tauri-apps/cli/tauri.js"
if [ -f "$tauri_js" ] && command -v node >/dev/null; then
  head -c 5000 /dev/urandom >"$tmp/dummy"
  node "$tauri_js" signer generate --ci -p test -w "$tmp/k" >/dev/null 2>&1
  node "$tauri_js" signer sign -f "$tmp/k" -p test "$tmp/dummy" >/dev/null 2>&1
  if [ -f "$tmp/dummy.sig" ] && verify_sig "$tmp/dummy" "$tmp/dummy.sig" "$(cat "$tmp/k.pub")" >/dev/null 2>&1; then
    pass "tauri_signature_verifies"
  else
    fail "tauri_signature_verifies: verify_sig refused a tauri signer signature"
  fi
else
  echo "skip tauri_signature_verifies (no client/ui/node_modules or node: run pnpm install in client/ui)"
fi

# --- signed_version (#232): the version a signature binds, from minisign's trusted comment
minisign -S -s "$tmp/keys/a.key" -m "$tmp/file.exe" -x "$tmp/v.raw.sig" -t "$(printf 'timestamp:1700000000\tfile:Pulse_0.2.2_x64-setup.exe\tversion:0.2.2')" >/dev/null
base64 -w0 "$tmp/v.raw.sig" >"$tmp/v.sig"
[ "$(signed_version "$tmp/v.sig")" = 0.2.2 ] && pass "signed_version reads version:0.2.2" || fail "signed_version: '$(signed_version "$tmp/v.sig")'"
[ -z "$(signed_version "$tmp/file.exe.sig")" ] && pass "signed_version: none in a plain minisign signature" || fail "signed_version found one in a plain signature"
echo 'not base64 !!' >"$tmp/junk.sig"
[ -z "$(signed_version "$tmp/junk.sig")" ] && pass "signed_version: none in junk" || fail "signed_version read junk"
# the real thing: `tauri signer sign --app-version` (what `tauri build` does, by the CLI's own help) writes the same
if [ -f "$tauri_js" ] && command -v node >/dev/null && [ -f "$tmp/k" ]; then
  node "$tauri_js" signer sign -f "$tmp/k" -p test --app-version 0.2.2 "$tmp/dummy" >/dev/null 2>&1
  if [ "$(signed_version "$tmp/dummy.sig")" = 0.2.2 ] && verify_sig "$tmp/dummy" "$tmp/dummy.sig" "$(cat "$tmp/k.pub")" >/dev/null 2>&1; then
    pass "tauri signer --app-version: signed_version 0.2.2 and it verifies"
  else
    fail "tauri signer --app-version: signed_version '$(signed_version "$tmp/dummy.sig")'"
  fi
  node "$tauri_js" signer sign -f "$tmp/k" -p test "$tmp/dummy" >/dev/null 2>&1
  [ -z "$(signed_version "$tmp/dummy.sig")" ] && pass "tauri signer without --app-version binds none" || fail "tauri signer without --app-version: '$(signed_version "$tmp/dummy.sig")'"
else
  echo "skip tauri signer --app-version (no client/ui/node_modules or node)"
fi

# --- the app's own key: what a real publish checks against (base64 of a minisign public key file)
app_pub=$(jq -r '.plugins.updater.pubkey // empty' "$here/../client/src-tauri/tauri.conf.json")
if [ -n "$app_pub" ] && printf '%s' "$app_pub" | base64 -d 2>/dev/null | grep -q '^untrusted comment: minisign public key'; then
  pass "tauri.conf.json carries the updater's public key"
else
  fail "tauri.conf.json has no readable plugins.updater.pubkey"
fi

# --- newer_than
check_newer() {
  local want=$1 new=$2 old=$3 got
  if newer_than "$new" "$old"; then got=yes; else got=no; fi
  [ "$got" = "$want" ] && pass "newer_than $new '$old' → $want" || fail "newer_than $new '$old' want $want got $got"
}
check_newer yes 0.2.1 0.2.0
check_newer no 0.2.1 0.2.1
check_newer no 0.2.0 0.2.1
check_newer yes 0.10.0 0.9.0
check_newer yes 0.2.0 ""

# --- write_manifest: refuses_foreign_url and manifest_shape
if write_manifest 0.2.1 "https://example.com/updates/Pulse_0.2.1_x64-setup.exe" "$tmp/file.exe.sig" "n" "$tmp/m.json" 2>/dev/null; then
  fail "refuses_foreign_url: accepted example.com"
else
  pass "refuses_foreign_url: example.com"
fi
for u in "http://$PULSE_DOMAIN/updates/x" "https://$PULSE_DOMAIN.evil.example/updates/x" "https://$PULSE_DOMAIN@evil.example/x" "https://$PULSE_DOMAIN:444/x"; do
  if write_manifest 0.2.1 "$u" "$tmp/file.exe.sig" "n" "$tmp/m.json" 2>/dev/null; then fail "refuses_foreign_url: accepted $u"; else pass "refuses_foreign_url: $u"; fi
done
[ ! -e "$tmp/m.json" ] && pass "a refused manifest writes nothing" || fail "a refused manifest left a file"
url="https://$PULSE_DOMAIN/updates/Pulse_0.2.1_x64-setup.exe"
if write_manifest 0.2.1 "$url" "$tmp/file.exe.sig" "Pulse 0.2.1" "$tmp/m.json"; then
  if jq -e --arg url "$url" --arg sig "$(cat "$tmp/file.exe.sig")" '
      .version == "0.2.1" and .notes == "Pulse 0.2.1"
      and (.pub_date | test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"))
      and .platforms["windows-x86_64"].url == $url
      and .platforms["windows-x86_64"].signature == $sig' "$tmp/m.json" >/dev/null; then
    pass "manifest_shape"
  else
    fail "manifest_shape: $(cat "$tmp/m.json")"
  fi
else
  fail "manifest_shape: write_manifest refused our own origin"
fi

# --- the fake server and the stubs
srv="$tmp/srv"
mkdir -p "$tmp/bin" "$tmp/artifact"
# gh run view → $FAKE_RUN; gh run download → copies $FAKE_ARTIFACT (and logs it)
cat >"$tmp/bin/gh" <<'EOF'
#!/usr/bin/env bash
case "$1 $2" in
  "run view") printf '%s\n' "$FAKE_RUN" ;;
  "run download")
    echo download >>"$LOG"
    while [ $# -gt 0 ]; do case "$1" in -D) dest=$2; shift ;; esac; shift; done
    cp "$FAKE_ARTIFACT"/* "$dest"/
    ;;
  *) echo "unexpected gh $*" >&2; exit 1 ;;
esac
EOF
# curl … -w '%{http_code}' URL → $FAKE_CODE (401 = the route is live and guarded)
cat >"$tmp/bin/curl" <<'EOF'
#!/usr/bin/env bash
echo "curl $*" >>"$LOG"
printf '%s' "${FAKE_CODE:-401}"
EOF
chmod +x "$tmp/bin/gh" "$tmp/bin/curl"
mkdir -p "$tmp/tmpdirs"
export LOG="$tmp/log" FAKE_ARTIFACT="$tmp/artifact" PATH="$tmp/bin:$PATH" PUBLISH_UPDATE_DIR="$srv" TMPDIR="$tmp/tmpdirs"
export PUBLISH_UPDATE_PUBKEY="$pub_a"
good_run='{"workflowName":"release-windows","conclusion":"success","headBranch":"main"}'

# artifact VERSION [SIGNED]: the CI download for VERSION, signed with key a for version SIGNED (default VERSION, as
# `tauri build` binds it; '-' = a signature that binds none)
artifact() {
  local signed=${2:-$1} comment
  rm -rf "${FAKE_ARTIFACT:?}"/*
  head -c 300000 /dev/urandom >"$FAKE_ARTIFACT/Pulse_$1_x64-setup.exe"
  comment=$(printf 'timestamp:1700000000\tfile:Pulse_%s_x64-setup.exe' "$1")
  [ "$signed" = - ] || comment+=$(printf '\tversion:%s' "$signed")
  minisign -S -s "$tmp/keys/a.key" -m "$FAKE_ARTIFACT/Pulse_$1_x64-setup.exe" -x "$tmp/raw.sig" -t "$comment" >/dev/null
  base64 -w0 "$tmp/raw.sig" >"$FAKE_ARTIFACT/Pulse_$1_x64-setup.exe.sig"
  echo 'collect logs' >"$FAKE_ARTIFACT/collect-logs.ps1"
}
# serve VERSIONS…: a fresh server dir holding those installers, the manifest naming the last
serve() {
  rm -rf "$srv"
  mkdir -p "$srv"
  local v
  for v in "$@"; do echo "old $v" >"$srv/Pulse_${v}_x64-setup.exe"; done
  if [ $# -gt 0 ]; then
    v=${!#}
    jq -n --arg v "$v" --arg u "https://$PULSE_DOMAIN/updates/Pulse_${v}_x64-setup.exe" \
      '{version: $v, platforms: {"windows-x86_64": {url: $u, signature: "x"}}}' >"$srv/latest.json"
  fi
}
# publish → ok|refused (output in $tmp/out)
publish() {
  if (publish_main 4242) >"$tmp/out" 2>&1; then echo ok; else echo refused; fi
}
offered() { jq -r .version "$srv/latest.json" 2>/dev/null; }
files() { (cd "$srv" && find . -type f | sed 's|^\./||' | LC_ALL=C sort | tr '\n' ' '); }

# --- a first publish on an empty server, then the next one
export FAKE_RUN=$good_run
serve
artifact 0.2.0
[ "$(publish)" = ok ] && pass "first publish" || fail "first publish: $(cat "$tmp/out")"
[ "$(offered)" = 0.2.0 ] && pass "manifest names 0.2.0" || fail "manifest after first publish: $(offered)"
cmp -s "$srv/Pulse_0.2.0_x64-setup.exe" "$FAKE_ARTIFACT/Pulse_0.2.0_x64-setup.exe" && pass "installer uploaded as built" || fail "installer differs on the server"
grep -q '^published 0.2.0$' "$tmp/out" && pass "prints published 0.2.0" || fail "no 'published 0.2.0': $(cat "$tmp/out")"
[ "$(files)" = "Pulse_0.2.0_x64-setup.exe latest.json " ] && pass "no leftovers (.incoming, .tmp, .sig)" || fail "server files: $(files)"
jq -e --arg s "$(cat "$FAKE_ARTIFACT/Pulse_0.2.0_x64-setup.exe.sig")" '.platforms["windows-x86_64"].signature == $s' "$srv/latest.json" >/dev/null &&
  pass "manifest carries the artifact's signature" || fail "manifest signature"

# --- refuses_same_or_older: served 0.2.1
for v in 0.2.1 0.2.0; do
  serve 0.2.1
  artifact "$v"
  [ "$(publish)" = refused ] && pass "refuses_same_or_older: $v over 0.2.1" || fail "published $v over 0.2.1"
  [ "$(files)" = "Pulse_0.2.1_x64-setup.exe latest.json " ] && pass "nothing uploaded for $v" || fail "uploaded on refusal: $(files)"
done
serve 0.2.1
artifact 0.2.2
[ "$(publish)" = ok ] && [ "$(offered)" = 0.2.2 ] && pass "0.2.2 over 0.2.1" || fail "0.2.2 over 0.2.1: $(cat "$tmp/out")"

# --- keeps_two_versions: 0.2.0, 0.2.1, 0.2.2 served, publishing 0.2.3 keeps 0.2.2 and 0.2.3
serve 0.2.0 0.2.1 0.2.2
artifact 0.2.3
[ "$(publish)" = ok ] || fail "publish 0.2.3: $(cat "$tmp/out")"
[ "$(files)" = "Pulse_0.2.2_x64-setup.exe Pulse_0.2.3_x64-setup.exe latest.json " ] && pass "keeps_two_versions" || fail "keeps_two_versions: $(files)"
# and prune on its own: manifest 0.2.2, three installers → 0.2.1 and 0.2.2 remain (0.10.0 sorts as a version, not text)
serve 0.2.0 0.2.1 0.2.2
prune >/dev/null 2>&1
[ "$(files)" = "Pulse_0.2.1_x64-setup.exe Pulse_0.2.2_x64-setup.exe latest.json " ] && pass "prune keeps the manifest's and the one before" || fail "prune: $(files)"
serve 0.9.0 0.9.1 0.10.0
prune >/dev/null 2>&1
[ "$(files)" = "Pulse_0.10.0_x64-setup.exe Pulse_0.9.1_x64-setup.exe latest.json " ] && pass "prune sorts by version" || fail "prune by version: $(files)"

# --- wrong_run_refused: refused before anything is downloaded
for run in '{"workflowName":"ci","conclusion":"success","headBranch":"main"}' \
  '{"workflowName":"release-windows","conclusion":"failure","headBranch":"main"}' \
  '{"workflowName":"release-windows","conclusion":"success","headBranch":"some-branch"}'; do
  export FAKE_RUN=$run
  serve 0.2.1
  artifact 0.2.2
  : >"$LOG"
  [ "$(publish)" = refused ] && pass "wrong_run_refused: $run" || fail "accepted run $run"
  grep -q download "$LOG" && fail "downloaded a wrong run: $run" || pass "no download for $run"
done
export FAKE_RUN='{"workflowName":"release-windows","conclusion":"success","headBranch":"v0.2.2"}'
serve 0.2.1
artifact 0.2.2
[ "$(publish)" = ok ] && pass "a v* tag run publishes" || fail "tag run: $(cat "$tmp/out")"
export FAKE_RUN=$good_run
for id in abc "4242;ls" ""; do
  if (publish_main "$id") >/dev/null 2>&1; then fail "accepted run id '$id'"; else pass "refuses run id '$id'"; fi
done

# --- a bad signature in the artifact: nothing uploaded
serve 0.2.1
artifact 0.2.2
minisign -S -s "$tmp/keys/b.key" -m "$FAKE_ARTIFACT/Pulse_0.2.2_x64-setup.exe" -x "$tmp/raw.sig" >/dev/null
base64 -w0 "$tmp/raw.sig" >"$FAKE_ARTIFACT/Pulse_0.2.2_x64-setup.exe.sig"
[ "$(publish)" = refused ] && pass "another key's signature refused" || fail "published with another key's signature"
[ "$(files)" = "Pulse_0.2.1_x64-setup.exe latest.json " ] && pass "nothing uploaded on a bad signature" || fail "uploaded on bad sig: $(files)"
# no .sig, or two installers
artifact 0.2.2
rm "$FAKE_ARTIFACT/Pulse_0.2.2_x64-setup.exe.sig"
[ "$(publish)" = refused ] && pass "missing .sig refused" || fail "published without a .sig"
artifact 0.2.2
cp "$FAKE_ARTIFACT/Pulse_0.2.2_x64-setup.exe" "$FAKE_ARTIFACT/Pulse_0.2.3_x64-setup.exe"
[ "$(publish)" = refused ] && pass "two installers refused" || fail "published from two installers"

# --- corrupt_upload_never_goes_live: the server's copy of the installer differs (a cut-off scp) → refused before the
# manifest names it; the old version stays served and nothing new is left in place
cat >"$tmp/bin/badcp" <<'EOF'
#!/usr/bin/env bash
# copies only the first 1000 bytes of an installer (any other file whole)
case "$1" in *.exe) head -c 1000 "$1" >"$2" ;; *) cp "$1" "$2" ;; esac
EOF
chmod +x "$tmp/bin/badcp"
serve 0.2.0 0.2.1
artifact 0.2.2
export PUBLISH_UPDATE_CP=badcp
[ "$(publish)" = refused ] && pass "corrupt_upload_never_goes_live: refused" || fail "published a corrupt upload: $(cat "$tmp/out")"
export PUBLISH_UPDATE_CP=cp
[ "$(offered)" = 0.2.1 ] && pass "the old version stays served" || fail "served after a corrupt upload: $(offered)"
[ "$(files)" = "Pulse_0.2.0_x64-setup.exe Pulse_0.2.1_x64-setup.exe latest.json " ] && pass "nothing new left, nothing pruned" || fail "after a corrupt upload: $(files)"

# --- failed_manifest_move_refused: the manifest can't be moved into place → refused, and the new installer isn't pruned
cat >"$tmp/bin/mv" <<'EOF'
#!/usr/bin/env bash
case "$*" in *latest.json.tmp*) echo "mv: read-only" >&2; exit 1 ;; *) exec /usr/bin/mv "$@" ;; esac
EOF
chmod +x "$tmp/bin/mv"
serve 0.2.1
artifact 0.2.2
[ "$(publish)" = refused ] && pass "failed_manifest_move_refused" || fail "a failed manifest move passed: $(cat "$tmp/out")"
rm "$tmp/bin/mv"
[ "$(offered)" = 0.2.1 ] && pass "the old manifest stays" || fail "manifest after a failed move: $(offered)"
[ -f "$srv/Pulse_0.2.1_x64-setup.exe" ] && pass "the served installer is kept" || fail "pruned the served installer: $(files)"

# --- signed versions (#232): the version the signature binds must be the installer's
serve 0.2.1
artifact 0.2.2 0.2.1
[ "$(publish)" = refused ] && grep -q "is for version 0.2.1, not 0.2.2" "$tmp/out" && pass "refuses a signature for another version" || fail "signed 0.2.1 as 0.2.2: $(cat "$tmp/out")"
[ "$(files)" = "Pulse_0.2.1_x64-setup.exe latest.json " ] && pass "nothing uploaded for a mismatched signed version" || fail "uploaded on mismatch: $(files)"
artifact 0.2.2 -
export PUBLISH_UPDATE_REQUIRE_SIGNED_VERSION=true
[ "$(publish)" = refused ] && pass "requireSignedVersion: refuses a signature that binds none" || fail "requireSignedVersion published an unbound signature: $(cat "$tmp/out")"
[ "$(files)" = "Pulse_0.2.1_x64-setup.exe latest.json " ] && pass "nothing uploaded for an unbound signature" || fail "uploaded unbound: $(files)"
export PUBLISH_UPDATE_REQUIRE_SIGNED_VERSION=false
[ "$(publish)" = ok ] && grep -q "binds no version" "$tmp/out" && pass "without requireSignedVersion an unbound signature publishes, with a note" || fail "unbound, not required: $(cat "$tmp/out")"
unset PUBLISH_UPDATE_REQUIRE_SIGNED_VERSION
# the real switch: tauri.conf.json's plugins.updater.requireSignedVersion. On since 0.2.1: release-windows' signatures
# bind the version (`tauri build` signs with it: the first signed run's .sig says version:0.2.0), so the app refuses a
# manifest that pairs a version with another release's installer, or with an older signature that binds none (#232).
want=$(jq -r '.plugins.updater.requireSignedVersion // false' "$here/../client/src-tauri/tauri.conf.json")
[ "$want" = true ] && pass "the app requires signed versions (requireSignedVersion)" || fail "requireSignedVersion is off in tauri.conf.json"
[ "$(require_signed_version)" = "$want" ] && pass "require_signed_version follows tauri.conf.json ($want)" || fail "require_signed_version: $(require_signed_version), conf $want"

# --- unreadable_manifest_refused (#233): a latest.json that's there but can't be read isn't "nothing served"
for junk in 'not json' '{"platforms":{}}' '{"version":"latest"}' ''; do
  serve 0.2.1
  printf '%s' "$junk" >"$srv/latest.json"
  artifact 0.2.0
  [ "$(publish)" = refused ] && pass "unreadable_manifest_refused: '$junk'" || fail "published over an unreadable manifest '$junk': $(cat "$tmp/out")"
  [ "$(files)" = "Pulse_0.2.1_x64-setup.exe latest.json " ] && pass "nothing uploaded over '$junk'" || fail "uploaded over '$junk': $(files)"
done
# an empty server dir still means nothing is served yet
serve
artifact 0.2.0
[ "$(publish)" = ok ] && pass "no manifest at all still publishes" || fail "first publish after the manifest fix: $(cat "$tmp/out")"

# --- the download is deleted, refused or not (#233)
[ -z "$(ls -A "$TMPDIR")" ] && pass "download_dir_removed: no temp dirs left after all the publishes above" || fail "temp dirs left: $(ls -A "$TMPDIR")"

# --- the route must be live and guarded
serve 0.2.1
artifact 0.2.2
export FAKE_CODE=404
[ "$(publish)" = refused ] && pass "refused when the route isn't live (404)" || fail "published with no route"
[ "$(files)" = "Pulse_0.2.1_x64-setup.exe latest.json " ] && pass "nothing uploaded without the route" || fail "uploaded without route: $(files)"
export FAKE_CODE=200
[ "$(publish)" = refused ] && pass "refused when the route is open (200)" || fail "published to an unguarded route"
export FAKE_CODE=401

exit "$failed"
