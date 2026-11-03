# Volume mixer (per-category volume)

Issue #44. From a friend's wishlist (2026-10-22); shaped by the owner on 2026-11-03: "if it can possibly be changed /
customized, we can add a notch for it", in easy-to-read sections. **Status: approved by the owner 2026-11-03.**

## Why

Pulse's own sounds and audio can't be balanced against each other. Call sounds are far too loud ("have to lower the
system volume for it, which is annoying") while notifications are about right, and the only fix today is the OS mixer,
which turns everything down together.

## Goals

- One place, **Settings → Voice & Audio → Volume mixer**, with a slider for everything Pulse plays that can be
  adjusted, grouped in sections.
- Each section has a **master** slider; the items under it multiply with it; and existing per-person / per-stream
  volumes multiply with those (effective = section × item × person). Nothing already set is lost.
- Per device, like today's sound settings.

## Non-goals

- The microphone's input level (that's in Voice settings already).
- Anything outside Pulse; never touches the OS mixer.

## Sections and notches

| Section | Notches |
|---|---|
| **Calls** (master) | **Voices** (everyone talking; × each person's own volume) · **Streams** (stream audio; × each stream's volume) · **Soundboard** (others' sounds; same setting as the popover's speaker slider; × per-person "Mute soundboard") |
| **Call sounds** (master) | **Join** · **Leave** · **Mute** · **Unmute** · **Deafen** · **Undeafen** |
| **Stream sounds** (master) | **Someone goes live** · **A stream ends** |
| **Notifications** (master) | **Message ding** · **Badge unlocked** |
| **Media** | **Clips** (clip playback in chat; same setting as a clip's own volume control) |

- Range 0–200 % for every notch, 100 % default; 0 % is off. The existing per-sound on/off switches in Sounds become
  "0 %" (migrated: an "off" sound starts at 0 %). The existing overall sound volume (`SoundPrefs.volume_pct`) becomes
  the starting value of all three sound masters.
- Each notch has a ▶ test button where it makes sense (the sound sections and Soundboard), playing that sound at the
  resulting level.
- New sounds get a notch when they're added (e.g. the ringtone with ringing calls, #50; join sounds, #121).
- A "Reset to defaults" button per section.

## How it gets made (builder's call where not stated)

- Prefs: one versioned per-device record (e.g. `pulse.mixer`) holding every notch; migration from `pulse.sounds`
  (`volume_pct`, `off`), `pulse.soundboard` and `pulse.clipVolume`; the soundboard popover and clip players read and
  write the same values.
- Voices and streams: the gain is applied where audio is already scaled per person/stream (the core's voice mixer;
  the stream audio path), as one extra multiplier; no change for people who don't touch it (100 %).
- App sounds: the per-sound gain is applied where UI sounds are played (`sounds.rs` / the UI sound player), section ×
  item × today's set.
- Clamp the final gain to a safe maximum and soft-limit (the mixer already soft-limits) so 200 % × 200 % can't clip
  badly.

## Testing

- Prefs: defaults, migration from today's keys, persistence, versioning.
- Gain maths: section × item × person, clamps, 0 = off.
- Each audio path applies its gain (core tests for voices/streams/soundboard where they exist; UI tests for sounds and
  clips).
- UI: sections render, sliders and test buttons work, reset per section, popover and clip slider stay in sync.
- Manual (owner): turn Call sounds right down and leave Notifications alone; check a call's join/leave chirps are
  quiet while a message ding isn't; Voices and Streams sliders affect a live call and a stream; Clips affects
  playback; the soundboard popover slider and the mixer's Soundboard notch move together.
