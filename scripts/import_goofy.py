#!/usr/bin/env python3
"""Build the Goofy sound set from Kenney's CC0 packs (see THIRD_PARTY.md).

Usage, from the repo root:  python3 -I scripts/import_goofy.py <kenney-root>
<kenney-root> holds one unzipped pack per subdirectory (digital-audio/, ...). Output is checked in.
Needs ffmpeg on PATH.
"""
import os, subprocess, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gen_sounds as g

OUT = "client/src-tauri/sounds"

# event -> (pack-relative path, start seconds, max length seconds)
MAPPING = {
    "join": ("digital-audio/Audio/powerUp6.ogg", 0.0, 0.45),
    "leave": ("digital-audio/Audio/phaserDown1.ogg", 0.0, 0.45),
    "mute": ("digital-audio/Audio/highDown.ogg", 0.0, 0.40),
    "unmute": ("digital-audio/Audio/highUp.ogg", 0.0, 0.40),
    "deafen": ("digital-audio/Audio/phaseJump3.ogg", 0.0, 0.45),
    "undeafen": ("digital-audio/Audio/phaseJump2.ogg", 0.0, 0.45),
    "message": ("digital-audio/Audio/pepSound3.ogg", 0.0, 0.40),
    "live": ("interface-sounds/Audio/confirmation_004.ogg", 0.0, 0.50),
    "live_end": ("digital-audio/Audio/phaserDown2.ogg", 0.0, 0.31),
}

FADE_IN, FADE_OUT = 0.005, 0.020


def decode(path):
    raw = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", path, "-f", "s16le", "-ac", "1", "-ar", str(g.RATE), "-"],
        check=True, capture_output=True,
    ).stdout
    return [int.from_bytes(raw[i:i + 2], "little", signed=True) / 32768 for i in range(0, len(raw) - 1, 2)]


def build(event, root):
    rel, start, length = MAPPING[event]
    path = os.path.join(root, rel)
    if not os.path.isfile(path):
        sys.exit(f"missing source for {event}: {path}")
    s = decode(path)[int(start * g.RATE):int((start + length) * g.RATE)]
    nin, nout = int(FADE_IN * g.RATE), int(FADE_OUT * g.RATE)
    for i in range(min(nin, len(s))):
        s[i] *= i / nin
    for i in range(min(nout, len(s))):
        s[len(s) - 1 - i] *= i / nout
    _, ref = g.read_wav(os.path.join(OUT, "classic", f"{event}.wav"))
    return g.normalise(s, g.active_rms(ref))


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    for event in MAPPING:
        g.write_wav(os.path.join(OUT, "goofy", f"{event}.wav"), build(event, sys.argv[1]))


if __name__ == "__main__":
    main()
