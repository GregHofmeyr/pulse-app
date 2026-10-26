#!/usr/bin/env python3
"""Generate Pulse's UI sounds, one directory per set (into client/src-tauri/sounds, from any cwd).
Output is checked in."""
import math, os, struct, wave

RATE = 22050
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "client/src-tauri/sounds")

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


def _note(freq, dur, partials, attack, release=0.03, decay=None):
    """One note: summed partials (multiple of freq, amplitude), linear attack/release
    (`release=None`: no release ramp), optionally each partial decaying exponentially
    (tau seconds per partial)."""
    n = int(RATE * dur)
    out = []
    for i in range(n):
        t = i / RATE
        v = 0.0
        for k, (mult, amp) in enumerate(partials):
            a = amp * (math.exp(-t / decay[k]) if decay else 1.0)
            v += a * math.sin(2 * math.pi * freq * mult * t)
        env = min(1.0, t / attack)
        if release is not None:
            env *= min(1.0, (dur - t) / release)
        out.append(v * env)
    return out

# Two-step timbres: base Hz, partials, note length (s), attack (s)
TWOSTEP = {
    "bright": (1047, [(1, 1.0), (2, 0.25)], 0.07, 0.004),
    "warm": (659, [(1, 1.0), (3, 0.2), (5, 0.08)], 0.085, 0.010),
    "deep": (220, [(1, 1.0), (2, 0.5), (3, 0.3)], 0.10, 0.006),
}
# event -> (first, second) multiples of the base, note length scale (None: MESSAGE_NOTE)
TWOSTEP_PAIRS = {
    "join": ((1, 1.5), 1.0), "leave": ((1.5, 1), 1.0),
    "unmute": ((1, 1.25), 0.8), "mute": ((1.25, 1), 0.8),
    "undeafen": ((0.5, 0.6667), 1.0), "deafen": ((0.6667, 0.5), 1.0),
    "live": ((1, 1.6667), 1.0), "live_end": ((1.6667, 1), 1.0),
    "message": ((1.5, 2.0), None),
}
# the message notes are this long whatever the variant
MESSAGE_NOTE = 0.06

def twostep(variant, event):
    base, partials, note, attack = TWOSTEP[variant]
    (m1, m2), scale = TWOSTEP_PAIRS[event]
    dur = MESSAGE_NOTE if scale is None else note * scale
    gap = [0.0] * int(RATE * 0.025)
    return _note(base * m1, dur, partials, attack) + gap + _note(base * m2, dur, partials, attack)

# Minimal: one note at base * multiple
MINIMAL_MULT = {
    "join": 1.5, "leave": 1.0, "unmute": 1.3333, "mute": 1.1225, "undeafen": 0.9,
    "deafen": 0.75, "live": 2.0, "live_end": 1.6818, "message": 1.8877,
}

def minimal(variant, event):
    m = MINIMAL_MULT[event]
    if variant == "soft":
        n = int(RATE * 0.12)
        return [math.sin(2 * math.pi * 880 * m * i / RATE) * 0.5 * (1 - math.cos(2 * math.pi * i / n))
                for i in range(n)]
    if variant == "wood":
        return _note(587 * m, 0.12, [(1, 1.0), (2.76, 0.3)], 0.002, release=None, decay=[0.025, 0.025])
    return _note(1319 * m, 0.18, [(1, 1.0), (2.76, 0.4), (5.4, 0.15)], 0.002, release=None,
                 decay=[0.060, 0.030, 0.015])

def _scaled(make, minus_db=0.0):
    """Every event of a set, normalised to Classic's active RMS for that event (minus `minus_db`)."""
    k = 10 ** (-minus_db / 20)
    return {name: normalise(make(name), active_rms(CLASSIC[name]) * k) for name in CLASSIC}

# set id -> {event name: samples}. Classic is the loudness reference and is NOT normalised.
SETS = {"classic": CLASSIC}
for _v in TWOSTEP:
    SETS[f"twostep-{_v}"] = _scaled(lambda e, v=_v: twostep(v, e))
for _v in ("soft", "wood", "glass"):
    SETS[f"minimal-{_v}"] = _scaled(lambda e, v=_v: minimal(v, e), minus_db=6.0)

def main():
    for set_id, sounds in SETS.items():
        for name, samples in sounds.items():
            write_wav(f"{OUT}/{set_id}/{name}.wav", samples)
    print("ok")

if __name__ == "__main__":
    main()
