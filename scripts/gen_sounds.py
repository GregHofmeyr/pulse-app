#!/usr/bin/env python3
"""Generate Pulse's UI sounds, one directory per set (into client/src-tauri/sounds, from any cwd).
Output is checked in."""
import math, os, struct, wave

RATE = 22050
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "client/src-tauri/sounds")

def tone(freqs, dur, gap=0.0, vol=0.22, bright=0.0):
    """Notes in sequence; each note glides from f0 to f1 with a soft envelope. `bright`: a 2nd harmonic at that
    fraction of the note, which carries better on small speakers."""
    out = []
    for f0, f1 in freqs:
        n = int(RATE * dur)
        phase = 0.0
        for i in range(n):
            t = i / n
            f = f0 + (f1 - f0) * t
            phase += 2 * math.pi * f / RATE
            env = min(1.0, i / (RATE * 0.008)) * min(1.0, (n - i) / (RATE * 0.04))
            out.append((math.sin(phase) + bright * math.sin(2 * phase)) * env * vol)
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

# Classic's join before #219: the ringtones are made at its level (sounds.rs RING_RMS checks it).
RING_RMS = active_rms(tone([(523, 523), (784, 784)], 0.09, vol=0.34))
# Join and leave are the events you most need to notice (#219): the loudest of the set, a little brighter, and no
# longer mirror images: join climbs high (E5 -> B5), leave drops into a low register (B4 -> E4).
JOIN_LEAVE_RMS = 0.27

CLASSIC = {
    "join": normalise(tone([(659, 659), (988, 988)], 0.09, vol=0.34, bright=0.25), JOIN_LEAVE_RMS),
    "leave": normalise(tone([(494, 494), (330, 330)], 0.1, vol=0.34, bright=0.25), JOIN_LEAVE_RMS),
    "mute": tone([(440, 330)], 0.08),
    "unmute": tone([(330, 440)], 0.08),
    "deafen": tone([(392, 294), (330, 247)], 0.07, gap=0.02),
    "undeafen": tone([(247, 330), (294, 392)], 0.07, gap=0.02),
    # a soft, bright two-note pop for new DMs/mentions (distinct from join/leave)
    "message": tone([(880, 880), (1320, 1320)], 0.08, gap=0.02, vol=0.42),
    # someone goes live / a stream ends: a rising / falling three-note run
    "live": tone([(587, 587), (784, 784), (1175, 1175)], 0.07, vol=0.30),
    "live_end": tone([(1175, 1175), (784, 784), (587, 587)], 0.07, vol=0.30),
    # a badge unlocked: a rising major chime, brighter than "live"
    "badge": tone([(784, 784), (988, 988), (1568, 1568)], 0.08, gap=0.015, vol=0.30),
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
    # leave falls from the base into the register below, not back down join's notes (#219)
    "join": ((1, 1.5), 1.0), "leave": ((1, 0.6667), 1.0),
    "unmute": ((1, 1.25), 0.8), "mute": ((1.25, 1), 0.8),
    "undeafen": ((0.5, 0.6667), 1.0), "deafen": ((0.6667, 0.5), 1.0),
    "live": ((1, 1.6667), 1.0), "live_end": ((1.6667, 1), 1.0),
    "message": ((1.5, 2.0), None),
    # the one three-note event: a rising major chime
    "badge": ((1, 1.25, 2.0), 0.8),
}
# the message notes are this long whatever the variant
MESSAGE_NOTE = 0.06

def twostep(variant, event):
    base, partials, note, attack = TWOSTEP[variant]
    mults, scale = TWOSTEP_PAIRS[event]
    dur = MESSAGE_NOTE if scale is None else note * scale
    gap = [0.0] * int(RATE * 0.025)
    out = _note(base * mults[0], dur, partials, attack)
    for m in mults[1:]:
        out += gap + _note(base * m, dur, partials, attack)
    return out

# Minimal: one note at base * multiple
MINIMAL_MULT = {
    "join": 1.5, "leave": 1.0, "unmute": 1.3333, "mute": 1.1225, "undeafen": 0.9,
    "deafen": 0.75, "live": 2.0, "live_end": 1.6818, "message": 1.8877,
}

# badge: the set's one note, three times, rising, each cut short (the set's sounds stay <= 0.2 s)
MINIMAL_BADGE = (1.5, 1.8877, 2.2449)
MINIMAL_BADGE_NOTE = 0.055

def minimal(variant, event):
    if event == "badge":
        gap = [0.0] * int(RATE * 0.01)
        out = _minimal_note(variant, MINIMAL_BADGE[0], MINIMAL_BADGE_NOTE)
        for m in MINIMAL_BADGE[1:]:
            out += gap + _minimal_note(variant, m, MINIMAL_BADGE_NOTE)
        return out
    return _minimal_note(variant, MINIMAL_MULT[event])

def _minimal_note(variant, m, short=None):
    """One note of the variant; `short`: this long instead, with a quick release so it doesn't click."""
    if variant == "soft":
        n = int(RATE * (short or 0.12))
        return [math.sin(2 * math.pi * 880 * m * i / RATE) * 0.5 * (1 - math.cos(2 * math.pi * i / n))
                for i in range(n)]
    release = 0.012 if short else None
    if variant == "wood":
        return _note(587 * m, short or 0.12, [(1, 1.0), (2.76, 0.3)], 0.002, release=release,
                     decay=[0.025, 0.025])
    return _note(1319 * m, short or 0.18, [(1, 1.0), (2.76, 0.4), (5.4, 0.15)], 0.002, release=release,
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

RING_OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "client/src-tauri/ringtone/ring.wav")

def ringtone():
    """The incoming-call ring (#50): one cycle, which the client loops. Classic's soft sines: two quick trills of a
    rising pair (E5-A5), a short breath between them, then a pause before the next cycle. Normalised to RING_RMS
    (Classic's join as it was), so it sits with the set; the volume mixer's Ringing notch takes it from there. One ringtone for every set."""
    trill = []
    for _ in range(4):
        trill += tone([(659, 659)], 0.09) + tone([(880, 880)], 0.09)
    cycle = trill + [0.0] * int(RATE * 0.2) + trill + [0.0] * int(RATE * 1.2)
    return normalise(cycle, RING_RMS)

# The other ringtones you can pick (#174). Each is one cycle the client loops, ending in a pause, normalised to
# Classic's join like the original (which stays `ring.wav`, the "classic" ringtone).

def _mix(length_s, notes):
    """Sum `(start s, samples)` into a buffer `length_s` long (notes ring over each other)."""
    out = [0.0] * int(RATE * length_s)
    for start, samples in notes:
        at = int(RATE * start)
        for i, v in enumerate(samples[: len(out) - at]):
            out[at + i] += v
    return out

def ring_marimba():
    """Soft mallets: a rising-and-falling pentatonic figure (G4 C5 E5 G5 E5 C5), played twice, each note a warm wooden
    strike (fundamental plus the marimba's 4th partial, quick decay) that rings into the next."""
    notes = [392, 523, 659, 784, 659, 523]
    step = 0.14
    phrase = [(i * step, _note(f, 0.45, [(1, 1.0), (4, 0.18), (10, 0.03)], 0.003, release=0.05,
                               decay=[0.18, 0.05, 0.015])) for i, f in enumerate(notes)]
    second = [(s + 0.95, n) for s, n in phrase]
    cycle = _mix(2.95, phrase + second)
    return normalise(cycle, RING_RMS)

def _pulse(freq, dur, harmonics=(1, 3, 5, 7), vol=1.0):
    """A soft square: the first few odd harmonics only (no aliasing fizz), clean attack and release."""
    n = int(RATE * dur)
    out = []
    for i in range(n):
        t = i / RATE
        v = sum(math.sin(2 * math.pi * freq * k * t) / k for k in harmonics if freq * k < RATE / 2.5)
        env = min(1.0, t / 0.004) * min(1.0, (dur - t) / 0.012)
        out.append(v * env * vol)
    return out

def ring_chiptune():
    """Retro handheld: a quick square-wave arpeggio up a major chord (C5 E5 G5 C6), twice, then a bright answer
    (G5 C6) and a pause. Odd harmonics only, so it stays round rather than buzzy on loop."""
    up = [523, 659, 784, 1047]
    notes, t = [], 0.0
    for _ in range(2):
        for f in up:
            notes.append((t, _pulse(f, 0.065)))
            t += 0.075
        t += 0.06
    t += 0.08
    for f in (784, 1047):
        notes.append((t, _pulse(f, 0.12)))
        t += 0.14
    cycle = _mix(t + 0.9, notes)
    return normalise(cycle, RING_RMS)

def ring_phone():
    """The two-tone classic phone: two tones together (400 + 450 Hz, the old double ring), ring-ring, then a pause.
    Soft edges and a light 3rd harmonic so it reads as a bell-less desk phone rather than a test tone."""
    def burst(dur):
        n = int(RATE * dur)
        out = []
        for i in range(n):
            t = i / RATE
            v = 0.0
            for f in (400, 450):
                v += math.sin(2 * math.pi * f * t) + 0.15 * math.sin(2 * math.pi * 3 * f * t)
            env = min(1.0, t / 0.015) * min(1.0, (dur - t) / 0.03)
            out.append(v * env)
        return out
    cycle = _mix(2.4, [(0.0, burst(0.4)), (0.6, burst(0.4))])
    return normalise(cycle, RING_RMS)

RINGTONE_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "client/src-tauri/ringtone")
# ringtone id -> generator; "classic" is ring.wav (above)
RINGTONES = {"marimba": ring_marimba, "chiptune": ring_chiptune, "phone": ring_phone}

def main():
    for set_id, sounds in SETS.items():
        for name, samples in sounds.items():
            write_wav(f"{OUT}/{set_id}/{name}.wav", samples)
    write_wav(RING_OUT, ringtone())
    for ring_id, make in RINGTONES.items():
        write_wav(f"{RINGTONE_DIR}/{ring_id}.wav", make())
    print("ok")

if __name__ == "__main__":
    main()
