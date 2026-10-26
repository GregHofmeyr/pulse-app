#!/usr/bin/env python3
"""Generate Pulse's UI sounds, one directory per set. Run from repo root; output is checked in."""
import math, os, struct, wave

RATE = 22050
OUT = "client/src-tauri/sounds"

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

def write_wav(path, samples):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(b"".join(struct.pack("<h", int(max(-1, min(1, s)) * 32767)) for s in samples))

def read_wav(path):
    """-> (rate, samples as floats in -1..1); mono PCM16 only."""
    with wave.open(path, "rb") as w:
        assert w.getnchannels() == 1 and w.getsampwidth() == 2, path
        raw = w.readframes(w.getnframes())
        return w.getframerate(), [v / 32767 for (v,) in struct.iter_unpack("<h", raw)]

def active_rms(samples):
    """RMS from the first to the last sample with |s| >= 0.02 * peak."""
    peak = max((abs(s) for s in samples), default=0.0)
    idx = [i for i, s in enumerate(samples) if abs(s) >= 0.02 * peak]
    if not idx:
        return 0.0
    seg = samples[idx[0]:idx[-1] + 1]
    return math.sqrt(sum(s * s for s in seg) / len(seg))

def normalise(samples, target_rms, peak=0.98):
    """Scale to `target_rms` active RMS, then pull down if that would exceed `peak`."""
    cur = active_rms(samples)
    if cur == 0.0:
        return list(samples)
    k = target_rms / cur
    top = max(abs(s) for s in samples) * k
    if top > peak:
        k *= peak / top
    return [s * k for s in samples]

CLASSIC = {
    "join": tone([(523, 523), (784, 784)], 0.09, vol=0.34),
    "leave": tone([(784, 784), (523, 523)], 0.09, vol=0.34),
    "mute": tone([(440, 330)], 0.08),
    "unmute": tone([(330, 440)], 0.08),
    "deafen": tone([(392, 294), (330, 247)], 0.07, gap=0.02),
    "undeafen": tone([(247, 330), (294, 392)], 0.07, gap=0.02),
    # a soft, bright two-note pop for new DMs/mentions (distinct from join/leave)
    "message": tone([(880, 880), (1320, 1320)], 0.08, gap=0.02, vol=0.42),
    # someone goes live / a stream ends: a rising / falling three-note run
    "live": tone([(587, 587), (784, 784), (1175, 1175)], 0.07, vol=0.30),
    "live_end": tone([(1175, 1175), (784, 784), (587, 587)], 0.07, vol=0.30),
}

# set id -> {event name: samples}. Classic is the loudness reference and is NOT normalised.
SETS = {"classic": CLASSIC}

def main():
    for set_id, sounds in SETS.items():
        for name, samples in sounds.items():
            write_wav(f"{OUT}/{set_id}/{name}.wav", samples)
    print("ok")

if __name__ == "__main__":
    main()
