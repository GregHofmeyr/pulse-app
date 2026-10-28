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

## Fonts

All bundled fonts come from the [Fontsource](https://fontsource.org) npm packages and are licensed under the
[SIL Open Font License 1.1](https://openfontlicense.org). Only the woff2 files named below are bundled,
unmodified (`client/ui/src/fonts.css`, `client/ui/src/nameFonts.css`). The full licence text is at the end of this section; each notice below names its copyright holder.

### Onest (`@fontsource/onest` 5.3.1)

Copyright 2021 The Onest Project Authors (https://github.com/simpals/onest). SIL Open Font License 1.1. Bundled: the weights 400 to 700 of the latin, latin-ext and symbols woff2 files only, unmodified.

### Cormorant Garamond (`@fontsource/cormorant-garamond` 5.3.0)

Copyright 2015 The Cormorant Project Authors (github.com/CatharsisFonts/Cormorant). SIL Open Font License 1.1. Bundled: weight 600 of the latin and latin-ext woff2 files only, unmodified.

### Fredoka (`@fontsource/fredoka` 5.3.0)

Copyright 2016 The Fredoka Project Authors (https://github.com/hafontia/Fredoka-One). SIL Open Font License 1.1. Bundled: weight 500 of the latin and latin-ext woff2 files only, unmodified.

### Press Start 2P (`@fontsource/press-start-2p` 5.3.0)

Copyright 2012 The Press Start 2P Project Authors, with Reserved Font Name "Press Start 2P". SIL Open Font License 1.1. Bundled: weight 400 of the latin and latin-ext woff2 files only, unmodified.

### Pacifico (`@fontsource/pacifico` 5.3.0)

Copyright 2018 The Pacifico Project Authors (https://github.com/googlefonts/Pacifico). SIL Open Font License 1.1. Bundled: weight 400 of the latin and latin-ext woff2 files only, unmodified.

### Caveat (`@fontsource/caveat` 5.3.0)

Copyright 2014 The Caveat Project Authors (https://github.com/googlefonts/caveat). SIL Open Font License 1.1. Bundled: weight 600 of the latin and latin-ext woff2 files only, unmodified.

### Space Grotesk (`@fontsource/space-grotesk` 5.3.0)

Copyright 2020 The Space Grotesk Project Authors (https://github.com/floriankarsten/space-grotesk). SIL Open Font License 1.1. Bundled: weight 600 of the latin and latin-ext woff2 files only, unmodified.

### Bricolage Grotesque (`@fontsource/bricolage-grotesque` 5.3.0)

Copyright 2022 The Bricolage Grotesque Project Authors (https://github.com/ateliertriay/bricolage). SIL Open Font License 1.1. Bundled: weight 600 of the latin and latin-ext woff2 files only, unmodified.

### Orbitron (`@fontsource/orbitron` 5.3.0)

Copyright 2018 The Orbitron Project Authors (https://github.com/theleagueof/orbitron). SIL Open Font License 1.1. Bundled: weight 600 of the latin woff2 files only, unmodified.

### SIL Open Font License 1.1 (applies to every font above)

```text
-----------------------------------------------------------
SIL OPEN FONT LICENSE Version 1.1 - 26 February 2007
-----------------------------------------------------------

PREAMBLE
The goals of the Open Font License (OFL) are to stimulate worldwide
development of collaborative font projects, to support the font creation
efforts of academic and linguistic communities, and to provide a free and
open framework in which fonts may be shared and improved in partnership
with others.

The OFL allows the licensed fonts to be used, studied, modified and
redistributed freely as long as they are not sold by themselves. The
fonts, including any derivative works, can be bundled, embedded,
redistributed and/or sold with any software provided that any reserved
names are not used by derivative works. The fonts and derivatives,
however, cannot be released under any other type of license. The
requirement for fonts to remain under this license does not apply
to any document created using the fonts or their derivatives.

DEFINITIONS
"Font Software" refers to the set of files released by the Copyright
Holder(s) under this license and clearly marked as such. This may
include source files, build scripts and documentation.

"Reserved Font Name" refers to any names specified as such after the
copyright statement(s).

"Original Version" refers to the collection of Font Software components as
distributed by the Copyright Holder(s).

"Modified Version" refers to any derivative made by adding to, deleting,
or substituting -- in part or in whole -- any of the components of the
Original Version, by changing formats or by porting the Font Software to a
new environment.

"Author" refers to any designer, engineer, programmer, technical
writer or other person who contributed to the Font Software.

PERMISSION & CONDITIONS
Permission is hereby granted, free of charge, to any person obtaining
a copy of the Font Software, to use, study, copy, merge, embed, modify,
redistribute, and sell modified and unmodified copies of the Font
Software, subject to the following conditions:

1) Neither the Font Software nor any of its individual components,
in Original or Modified Versions, may be sold by itself.

2) Original or Modified Versions of the Font Software may be bundled,
redistributed and/or sold with any software, provided that each copy
contains the above copyright notice and this license. These can be
included either as stand-alone text files, human-readable headers or
in the appropriate machine-readable metadata fields within text or
binary files as long as those fields can be easily viewed by the user.

3) No Modified Version of the Font Software may use the Reserved Font
Name(s) unless explicit written permission is granted by the corresponding
Copyright Holder. This restriction only applies to the primary font name as
presented to the users.

4) The name(s) of the Copyright Holder(s) or the Author(s) of the Font
Software shall not be used to promote, endorse or advertise any
Modified Version, except to acknowledge the contribution(s) of the
Copyright Holder(s) and the Author(s) or with their explicit written
permission.

5) The Font Software, modified or unmodified, in part or in whole,
must be distributed entirely under this license, and must not be
distributed under any other license. The requirement for fonts to
remain under this license does not apply to any document created
using the Font Software.

TERMINATION
This license becomes null and void if any of the above conditions are
not met.

DISCLAIMER
THE FONT SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT
OF COPYRIGHT, PATENT, TRADEMARK, OR OTHER RIGHT. IN NO EVENT SHALL THE
COPYRIGHT HOLDER BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
INCLUDING ANY GENERAL, SPECIAL, INDIRECT, INCIDENTAL, OR CONSEQUENTIAL
DAMAGES, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF THE USE OR INABILITY TO USE THE FONT SOFTWARE OR FROM
OTHER DEALINGS IN THE FONT SOFTWARE.
```
