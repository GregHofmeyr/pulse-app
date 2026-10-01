#!/usr/bin/env python3
"""Generate Pulse's UI sounds (tiny sine chirps). Run from repo root; output is checked in."""
import math, struct, wave

RATE = 22050
OUT = "client/ui/public/sounds"

def tone(freqs, dur, gap=0.0, vol=0.22):
    """Notes in sequence; each note glides from f0 to f1 with a soft envelope."""
    out = []
    for f0, f1 in freqs:
        n = int(RATE * dur)
        phase = 0.0
        for i in range(n):
            t = i / n
            f = f0 + (f1 - f0) * t
            phase += 2 * math.pi * f / RATE
            env = min(1.0, i / (RATE * 0.008)) * min(1.0, (n - i) / (RATE * 0.04))
            out.append(math.sin(phase) * env * vol)
        out += [0.0] * int(RATE * gap)
    return out

SOUNDS = {
    "join": tone([(523, 523), (784, 784)], 0.09, vol=0.34),
    "leave": tone([(784, 784), (523, 523)], 0.09, vol=0.34),
    "mute": tone([(440, 330)], 0.08),
    "unmute": tone([(330, 440)], 0.08),
    "deafen": tone([(392, 294), (330, 247)], 0.07, gap=0.02),
    "undeafen": tone([(247, 330), (294, 392)], 0.07, gap=0.02),
}

for name, samples in SOUNDS.items():
    with wave.open(f"{OUT}/{name}.wav", "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(b"".join(struct.pack("<h", int(max(-1, min(1, s)) * 32767)) for s in samples))
print("ok")
