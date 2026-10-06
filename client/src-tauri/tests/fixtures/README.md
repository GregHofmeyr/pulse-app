# Test fixtures

`speech_48k_mono_s16le.raw` — 3 s of continuous speech, raw signed 16-bit little-endian, 48 kHz, mono.
Excerpt (from 2:05.9) of "The Art of War" chapters 1–2, LibriVox recording, **public domain**:
https://archive.org/details/art_of_war_librivox
(`art_of_war_01-02_sun_tzu_64kb.mp3`; `ffmpeg -ss 125.9 -t 3 -i … -ac 1 -ar 48000 -f s16le …`)
