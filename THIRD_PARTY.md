# Third-party notices

## Twemoji graphics (`@twemoji/svg` 15.0.0)

The standard emoji images are Twemoji. Copyright 2020 Twitter, Inc and other contributors.
Graphics licensed under [CC-BY 4.0](https://creativecommons.org/licenses/by/4.0/).
Source: https://github.com/jdecked/twemoji. The SVGs are copied unmodified into the UI at build time.

## emojibase-data 15.0.0

Emoji names, keywords, groups and skin tones. Copyright Miles Johnson. MIT licence.
Source: https://github.com/milesj/emojibase.

## Goofy sound set (Kenney "Digital Audio" and "Interface Sounds")

The Goofy UI sounds are clips from Kenney's [Digital Audio](https://kenney.nl/assets/digital-audio) and
[Interface Sounds](https://kenney.nl/assets/interface-sounds) packs, by Kenney Vleugels (https://kenney.nl).
Both are released under [CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/) (public domain dedication).
The clips were trimmed, resampled to 22 050 Hz mono and normalised in loudness; `scripts/import_goofy.py` rebuilds them.

## FFmpeg (Windows installer only)

The Windows installer ships `ffmpeg.exe`, `ffprobe.exe` and their DLLs (libavcodec, libavformat, libavfilter,
libavdevice, libavutil, libswscale, libswresample) in its `ffmpeg/` folder, to cut, re-encode and probe clips.
FFmpeg is a trademark of Fabrice Bellard; see https://ffmpeg.org. It is licensed under the GNU Lesser General Public
License: this build is configured with `--enable-version3` and without GPL parts, so it is **LGPL-3.0-or-later**
(the DLLs' own licence string says "LGPL version 3 or later", and `LICENSE.txt` is the LGPL v3). The licence
text ships as `ffmpeg/LICENSE.txt`. Pulse runs `ffmpeg.exe` and `ffprobe.exe` as separate processes and never
links to the libraries. Linux builds don't include it.

- Binaries: BtbN's FFmpeg-Builds `win64-lgpl-shared` build from the FFmpeg 8.0 release branch, release
  [`autobuild-2026-02-28-12-59`](https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-02-28-12-59),
  asset `ffmpeg-n8.0.1-66-g27b8d1a017-win64-lgpl-shared-8.0.zip`, SHA-256
  `e7b1087c310cf8b91f5467b8ada6d7e47ce26f2777efa2317c7cc271087e5100` (pinned and checked in
  `scripts/fetch-ffmpeg.sh`; the same value as the release's `checksums.sha256` and a fresh `sha256sum`).
- Corresponding source: FFmpeg commit
  [`27b8d1a017de62b51c0f3a039327b5cc2d7ea13f`](https://github.com/FFmpeg/FFmpeg/commit/27b8d1a017de62b51c0f3a039327b5cc2d7ea13f)
  on the `release/8.0` branch (`git describe`: `n8.0.1-66-g27b8d1a017`, 66 commits after tag `n8.0.1`), built
  with the build scripts at BtbN/FFmpeg-Builds commit
  [`c482a075076c29edf6dd9c5a7c9d3113d0babed6`](https://github.com/BtbN/FFmpeg-Builds/tree/c482a075076c29edf6dd9c5a7c9d3113d0babed6)
  (the commit the release tag points at; it names the exact versions of the LGPL libraries it compiles in).
- Size (measured on the pinned build): the archive is 83.4 MB; the bundled files are 195.4 MB (186.4 MiB)
  installed; an LZMA stream of them (`xz --format=lzma`, preset 9, 8 MiB dictionary, NSIS's default) is 48.5 MB,
  the estimated growth of the installer. Not yet measured on a real installer (no Windows build here): compare the
  first `release-windows` installer with the previous one.
- H.264/AAC encoders: the encoder names in `avcodec-62.dll` show `h264_nvenc`, `h264_amf`, `h264_qsv`, `h264_mf`,
  `libopenh264` and `aac` (no `libx264`: it is GPL). Confirm from the first `release-windows` run, whose
  "ffmpeg encoders" step prints `ffmpeg.exe -hide_banner -encoders` filtered to `h264|aac`.
