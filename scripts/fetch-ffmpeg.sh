#!/usr/bin/env bash
# Fetch the pinned LGPL ffmpeg build that the Windows installer bundles (tauri.windows.conf.json) into
# client/src-tauri/ffmpeg/: ffmpeg.exe, ffprobe.exe, their DLLs and LICENSE.txt. Linux builds never need it.
#   bash scripts/fetch-ffmpeg.sh        (or `just fetch-ffmpeg`)
#
# Re-pinning: pick a BtbN autobuild release (https://github.com/BtbN/FFmpeg-Builds/releases) that has an
# ffmpeg-n8.0.*-win64-lgpl-shared-8.0.zip asset. Take its SHA-256 from that release's checksums.sha256, check it
# against `sha256sum` of a fresh download (both must match), and update all three values below together, plus the
# FFmpeg source tag and BtbN commit in THIRD_PARTY.md. If BtbN prunes the pinned release, the Windows release build
# fails here until it is re-pinned.
set -euo pipefail

FFMPEG_TAG=autobuild-2026-02-28-12-59
FFMPEG_ASSET=ffmpeg-n8.0.1-66-g27b8d1a017-win64-lgpl-shared-8.0.zip
FFMPEG_SHA256_PIN=e7b1087c310cf8b91f5467b8ada6d7e47ce26f2777efa2317c7cc271087e5100

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
url="${FFMPEG_URL:-https://github.com/BtbN/FFmpeg-Builds/releases/download/$FFMPEG_TAG/$FFMPEG_ASSET}"
dest="${FFMPEG_DEST:-$repo/client/src-tauri/ffmpeg}"
# Outside the folder: the installer bundles everything in it (`ffmpeg/*` matches dotfiles too).
marker="$dest.pinned"

# The self-test (scripts/fetch-ffmpeg-test.sh) serves fake archives from file:// URLs with their own hash.
# Anything fetched over the network is always checked against the pin above.
if [ -n "${FFMPEG_SHA256:-}" ]; then
    case "$url" in
        file://*) sha="$FFMPEG_SHA256" ;;
        *) echo "fetch-ffmpeg: FFMPEG_SHA256 overrides the pin only for file:// URLs" >&2; exit 1 ;;
    esac
else
    sha="$FFMPEG_SHA256_PIN"
fi

if [ -f "$marker" ] && [ "$(cat "$marker")" = "$sha" ] \
    && [ -f "$dest/ffmpeg.exe" ] && [ -f "$dest/ffprobe.exe" ] && [ -f "$dest/LICENSE.txt" ]; then
    echo "fetch-ffmpeg: $dest already holds $sha"
    exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "fetch-ffmpeg: downloading $url"
curl -fsSL --retry 3 -o "$tmp/ffmpeg.zip" "$url"
actual=$(sha256sum "$tmp/ffmpeg.zip" | cut -d' ' -f1)
if [ "$actual" != "$sha" ]; then
    echo "fetch-ffmpeg: SHA-256 mismatch for $url: expected $sha, got $actual. Not extracting it; re-pin only after" \
        "verifying the new archive (see the top of this script)." >&2
    exit 1
fi

mkdir "$tmp/x"
if command -v unzip >/dev/null; then
    unzip -q "$tmp/ffmpeg.zip" -d "$tmp/x"
elif command -v 7z >/dev/null; then
    7z x -bd -y -o"$tmp/x" "$tmp/ffmpeg.zip" >/dev/null
else
    python3 -I -m zipfile -e "$tmp/ffmpeg.zip" "$tmp/x"
fi

shopt -s nullglob
tops=("$tmp"/x/*/)
[ "${#tops[@]}" -eq 1 ] || { echo "fetch-ffmpeg: expected one top-level folder in the archive" >&2; exit 1; }
top=${tops[0]%/}
dlls=("$top"/bin/*.dll)
for f in "$top/bin/ffmpeg.exe" "$top/bin/ffprobe.exe" "$top/LICENSE.txt"; do
    [ -f "$f" ] || { echo "fetch-ffmpeg: ${f#"$top"/} is missing from the archive" >&2; exit 1; }
done
[ "${#dlls[@]}" -gt 0 ] || { echo "fetch-ffmpeg: no DLLs in the archive's bin/" >&2; exit 1; }

# Only what the app runs: no ffplay, include, lib, doc or presets.
stage="$tmp/stage"
mkdir "$stage"
cp "$top/bin/ffmpeg.exe" "$top/bin/ffprobe.exe" "${dlls[@]}" "$top/LICENSE.txt" "$stage/"

rm -rf "$dest" "$marker"
mkdir -p "$(dirname "$dest")"
mv "$stage" "$dest"
printf '%s\n' "$sha" > "$marker"
echo "fetch-ffmpeg: $dest ready ($FFMPEG_ASSET)"
