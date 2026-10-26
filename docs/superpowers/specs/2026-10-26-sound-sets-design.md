# Sound sets: design

**Status:** approved by the owner in chat (2026-10-26). Branch `sound-sets`.

## What the user gets

Settings → Voice & Audio gains a **Sounds** section:

- **A list of sound sets.** You pick one whole set (no mixing). Each row has a ▶ preview that plays a short
  sequence of that set's sounds. The default is **Classic**.
- **A sound volume slider**, 0–100 %, applied to every UI sound. 100 % is today's loudness.
- **An on/off switch per event.**
- These are saved per device, like the other audio settings (localStorage).

## Sets

| id | Shown as | How it sounds |
|---|---|---|
| `classic` | Classic | Today's sine chirps, byte-for-byte unchanged. Adds `live`/`live_end` in the same style. |
| `twostep-bright` | Two-step · Bright | Two quick high notes with a light overtone. Discord-style: rising for on, falling for off. |
| `twostep-warm` | Two-step · Warm | The same two-note shape, mid register, rounder tone (soft odd harmonics), gentler attack. |
| `twostep-deep` | Two-step · Deep | The same two-note shape, low register, with enough overtone to carry on laptop speakers. |
| `minimal-soft` | Minimal · Soft | One short, soft sine note per event. Higher for "on" events, lower for "off". |
| `minimal-wood` | Minimal · Wood | One short plucked, woody note (fast decay). |
| `minimal-glass` | Minimal · Glass | One short bell-like note (a slightly inharmonic overtone). |
| `goofy` | Goofy | Fun, short clips from Kenney's CC0 audio packs. |

Two-step direction:
- **Rising:** `join`, `unmute`, `undeafen`, `live`.
- **Falling:** `leave`, `mute`, `deafen`, `live_end`.
- `message` is a quick bright two-note ping in every Two-step variant.
- The intervals tell the pairs apart: join/leave a fifth, mute/unmute a major third (shorter notes), deafen/undeafen a
  fourth an octave lower, live/live_end a sixth.

## Events

The existing 7 events stay: `join`, `leave`, `mute`, `unmute`, `deafen`, `undeafen`, `message`. Two are new:

- **`live`:** someone in your voice channel goes live. It plays exactly where the "<name> is live" toast is shown
  today: same conditions, once per start, never for your own stream, never for a stream re-announced after a server
  restart.
- **`live_end`:** a stream ends (`StreamEnded`) that you are watching, or that is in your current voice channel.
  - Someone else's stream: it plays whatever the reason.
  - Your own stream: it plays only when the server ended it (`timed_out`, `crashed`). It stays silent when you
    stopped it or left voice.
  - It plays once per event, even when both conditions hold.
  - The existing toast and notice behaviour does not change. The toast itself never plays a sound, so nothing
    double-plays.

## Audio rules

- **Format:** files are mono PCM16 WAV at 22 050 Hz, the current rate, at
  `client/src-tauri/sounds/<set>/<name>.wav`.
- **Duration:**
  - Generated sounds are ≤ 0.35 s; Minimal sounds are ≤ 0.2 s.
  - Goofy sounds are ≤ 0.5 s.
  - Every sound is ≥ 0.03 s.
- **Size:** everything under `client/src-tauri/sounds/` totals ≤ 1 200 000 bytes. That keeps the total added
  well under 1.5 MB.
- **Loudness:**
  - The **active RMS** of a sound is the RMS over the samples from the first to the last sample whose
    |s| ≥ 0.02 × peak.
  - Reference = Classic's active RMS for the same event.
  - Two-step must be within ±1.5 dB of the reference.
  - Minimal must be at reference −6 dB, ±1.5 dB.
  - Goofy must be within ±3 dB of the reference. Its clips have a higher crest factor.
  - Peak ≤ 0.98 for every sound.
  - Classic is the reference and is not re-normalised.
- **Reproducible:** `scripts/gen_sounds.py` regenerates Classic, Two-step and Minimal. Regenerating Classic's 7
  existing files yields identical bytes.
- **Goofy import:** `scripts/import_goofy.py <kenney-dir>` rebuilds Goofy from the extracted Kenney packs. It uses a
  checked-in event → file/trim mapping, `ffmpeg` to decode and resample, and the same normaliser as the generator.
  The packs are not checked in.
- **Credit:** `THIRD_PARTY.md` credits Kenney (CC0 1.0) and names the packs used.

## Core

- The Rust core embeds every set (`include_bytes!`). It plays by set + name:
  - `play_sound(name, set?, volume?, output?)`.
  - An unknown or missing `set` falls back to `classic`.
  - An unknown `name` plays nothing.
- `volume` is 0.0–1.0, clamped, and multiplies today's fixed gain (0.6). Missing means 1.0; 0 plays nothing.
- The `message` wake-up lead-in (450 ms of silence) applies to `message` in every set.

## UI

- `voiceui.ts` owns the sound prefs (`pulse.sounds` in localStorage):
  - `{ set, volume_pct, off: SoundName[] }`.
  - Normalising on load drops an unknown set back to `classic`, clamps the volume to 0–100, and keeps only known
    names in `off`. A new event is therefore on by default.
- `playSound(name)` honours the event switch and the volume, and passes set, volume and the chosen output device.
- **Preview:**
  - It plays `join, mute, unmute, live, leave`, 450 ms apart, in that set at the current volume, ignoring the
    event switches.
  - Starting a preview cancels a running one.
  - Leaving the settings pane cancels it too.

## Out of scope

Mixing sounds across sets, custom user sounds, per-event volume, and playing sounds for other people
joining or leaving.
