#!/usr/bin/env bash
# Self-test for scripts/fetch-ffmpeg.sh, offline: fake archives served from file:// URLs.
#   bash scripts/fetch-ffmpeg-test.sh
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
fetch="$here/fetch-ffmpeg.sh"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# A fake BtbN archive: one top-level folder with bin/, include/, lib/, doc/ and LICENSE.txt.
top="$work/src/ffmpeg-n8.0-fake-win64-lgpl-shared-8.0"
mkdir -p "$top/bin" "$top/include/libavcodec" "$top/lib" "$top/doc" "$top/presets"
for f in ffmpeg.exe ffprobe.exe ffplay.exe avcodec-62.dll avutil-60.dll; do echo "$f" > "$top/bin/$f"; done
echo header > "$top/include/libavcodec/avcodec.h"
echo lib > "$top/lib/avcodec.lib"
echo doc > "$top/doc/ffmpeg.html"
echo preset > "$top/presets/libvpx-720p.ffpreset"
echo "GNU LESSER GENERAL PUBLIC LICENSE" > "$top/LICENSE.txt"
zip="$work/fake.zip"
(cd "$work/src" && python3 -I -m zipfile -c "$zip" "$(basename "$top")")
good_sha=$(sha256sum "$zip" | cut -d' ' -f1)

# 1. Wrong SHA-256: exit 1, nothing extracted, an existing folder left alone.
dest="$work/out/ffmpeg"
mkdir -p "$dest"
echo keep > "$dest/old.txt"
if FFMPEG_URL="file://$zip" FFMPEG_SHA256="$(printf '0%.0s' {1..64})" FFMPEG_DEST="$dest" \
    bash "$fetch" >"$work/log" 2>&1; then
    fail "a wrong SHA-256 was accepted"
else
    code=$?
fi
[ "$code" -eq 1 ] || fail "wrong SHA-256: exit $code, want 1"
grep -q "SHA-256 mismatch" "$work/log" || fail "wrong SHA-256: no mismatch message"
[ "$(ls -A "$dest")" = "old.txt" ] || fail "wrong SHA-256: the folder changed: $(ls -A "$dest")"
[ ! -e "$dest.pinned" ] || fail "wrong SHA-256: a pin marker was written"

# 1b. Without an explicit pin, a file:// archive is checked against the real pin (and fails the same way).
if FFMPEG_URL="file://$zip" FFMPEG_DEST="$dest" bash "$fetch" >"$work/log" 2>&1; then
    fail "the fake archive matched the real pin"
fi
[ "$(ls -A "$dest")" = "old.txt" ] || fail "real pin: the folder changed"

# 2. A pinned SHA-256 override is refused for a non-file:// URL (real downloads always use the pin).
if FFMPEG_URL="https://example.invalid/x.zip" FFMPEG_SHA256="$good_sha" FFMPEG_DEST="$dest" \
    bash "$fetch" >"$work/log" 2>&1; then
    fail "an overridden pin was accepted for a network URL"
fi
grep -q "only for file://" "$work/log" || fail "override refusal: no message"

# 3. Right SHA-256: the folder is replaced by exactly ffmpeg.exe, ffprobe.exe, the DLLs and LICENSE.txt.
FFMPEG_URL="file://$zip" FFMPEG_SHA256="$good_sha" FFMPEG_DEST="$dest" bash "$fetch" >"$work/log" 2>&1 \
    || { cat "$work/log" >&2; fail "a matching archive was refused"; }
got=$(cd "$dest" && find . -mindepth 1 | LC_ALL=C sort | tr '\n' ' ')
want="./LICENSE.txt ./avcodec-62.dll ./avutil-60.dll ./ffmpeg.exe ./ffprobe.exe "
[ "$got" = "$want" ] || fail "extracted set: got '$got', want '$want'"
[ "$(cat "$dest.pinned")" = "$good_sha" ] || fail "pin marker doesn't hold the SHA-256"

# 4. Idempotent: same pin and files present -> no download (a URL that can't be read is never touched).
FFMPEG_URL="file://$work/missing.zip" FFMPEG_SHA256="$good_sha" FFMPEG_DEST="$dest" bash "$fetch" >"$work/log" 2>&1 \
    || { cat "$work/log" >&2; fail "an up-to-date folder was fetched again"; }
grep -q "already" "$work/log" || fail "up to date: no message"

# 5. A pin marker without the files is not "up to date".
rm "$dest/ffprobe.exe"
if FFMPEG_URL="file://$work/missing.zip" FFMPEG_SHA256="$good_sha" FFMPEG_DEST="$dest" \
    bash "$fetch" >"$work/log" 2>&1; then
    fail "a folder missing ffprobe.exe counted as up to date"
fi

# 6. No unzip, 7z only, under MSYS (cygpath present): 7z gets the cygpath -w paths. PATH holds only stubs and the
# tools the script needs, so neither unzip nor python3 can be picked first.
stub="$work/stub"
mkdir -p "$stub"
for t in bash curl sha256sum cut mktemp rm mkdir cp mv cat dirname; do ln -s "$(command -v "$t")" "$stub/$t"; done
py=$(command -v python3)
cat > "$stub/cygpath" <<STUB
#!$(command -v bash)
[ "\$1" = "-w" ] || exit 2
printf 'WIN:%s\n' "\$2"
STUB
cat > "$stub/7z" <<STUB
#!$(command -v bash)
# Fake native 7z: logs its arguments, then extracts only if both paths came through cygpath.
printf '%s\n' "\$@" > "$work/7z.args"
out=; zip=
for a in "\$@"; do
    case "\$a" in -oWIN:*) out=\${a#-oWIN:} ;; WIN:*) zip=\${a#WIN:} ;; esac
done
[ -n "\$out" ] && [ -n "\$zip" ] || { echo "7z got a non-native path" >&2; exit 2; }
"$py" -I -m zipfile -e "\$zip" "\$out"
STUB
chmod +x "$stub/cygpath" "$stub/7z"
dest7="$work/out7/ffmpeg"
PATH="$stub" FFMPEG_URL="file://$zip" FFMPEG_SHA256="$good_sha" FFMPEG_DEST="$dest7" "$stub/bash" "$fetch" \
    >"$work/log" 2>&1 || { cat "$work/log" >&2; fail "the 7z path (MSYS) failed"; }
grep -qx -- '-oWIN:.*/x' "$work/7z.args" || fail "7z output dir wasn't converted: $(cat "$work/7z.args")"
grep -qx 'WIN:.*/ffmpeg.zip' "$work/7z.args" || fail "7z archive path wasn't converted: $(cat "$work/7z.args")"
[ -f "$dest7/ffmpeg.exe" ] && [ -f "$dest7/LICENSE.txt" ] || fail "the 7z path extracted nothing"

echo "fetch-ffmpeg self-test: ok"
