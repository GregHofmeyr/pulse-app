# Volume Mixer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Settings → Voice & Audio → Volume mixer: a slider for everything Pulse plays, grouped in sections with
masters, multiplied into the existing audio paths (issue #44).

**Architecture:** One versioned per-device record `pulse.mixer` (UI, localStorage) holds every notch as a percent on
the same dB curve the per-person sliders use (`percent_to_gain`). App sounds and the soundboard get their gain
computed per play; Voices and Streams are pushed to the core (`set_call_mix`), where the voice mixer multiplies a
per-kind gain into each source's per-person gain. Clips stay in the webview (`<video>.volume`).

**Tech Stack:** Rust (Tauri 2 core, cpal mixer), Svelte 5 + vitest.

**Spec:** `docs/superpowers/specs/2026-11-03-volume-mixer-design.md`

## Builder's decisions

- **One curve everywhere.** Every notch is a percent on `percent_to_gain` (0 % = silence, 1 % = −40 dB, 100 % = 0 dB,
  200 % = +12 dB). TS mirrors it as `percentToGain` (tests pin the same anchors).
- **Migration keeps loudness, not digits.** UI sounds and clips were *linear* volumes; they migrate through
  `gainToPercent` (old 50 % linear → 85 %, which is the same −6 dB). The soundboard's `volume_pct` was already on the
  dB curve, so it carries over as is. Old keys are left in place (harmless, downgrade-safe); `pulse.sounds` keeps
  holding the chosen set.
- **Safe maximum:** the final gain of any path is capped at `MAX_GAIN` = `percent_to_gain(200)` (+12 dB, ≈3.981):
  200 % × 200 % is no louder than one slider at 200 %. Everything that can go above 0 dB is soft-limited: the call
  mixer already is; out-of-call soundboard already is; UI sounds now are too (identity below the knee, so at ≤100 %
  they are bit-for-bit what they were).
- **Streams are rendered by the core** (`stream/audio.rs` pushes `SourceKind::Stream` into the call's `MixerInput`),
  so Streams is a per-kind gain there, like Voices.
- **Speaking rings ignore the Voices notch:** the meter stays on the per-person gain (a listener-wide "Voices 0 %"
  shouldn't hide who's talking; "Mute for me" still darkens a ring as before).
- **Clips: 0–100 %.** A `<video>` element's volume can't exceed 1. Boosting needs WebAudio routing of a cross-origin
  loopback-HTTP element through WebKitGTK/GStreamer (the renderer-abort risk class) — deferred to a parked issue. The
  clip player's slider and the Clips notch are the same percent; the player's mute toggle is kept in the record
  (`clipsMuted`).
- **Media has no master** (one notch, as the spec's table shows).
- **Soundboard ▶ test** plays the first sound the popover would show (favourites → frequent → first server); disabled
  when there are none. Previews (popover ▶, upload dialog) keep their rule: if the effective soundboard level is 0
  they play at 100 % × 100 %.
- **"Preview" of a sound set** plays each event at its section master (the per-event notches are ignored, as the
  per-event switches were).

## Global Constraints

- Prefs key `pulse.mixer`, `v: 1`. Notches 0–200, default 100, integers; Clips 0–100.
- Sections and labels (exact): **Calls** (Voices, Streams, Soundboard) · **Call sounds** (Join, Leave, Mute, Unmute,
  Deafen, Undeafen) · **Stream sounds** (Someone goes live, A stream ends) · **Notifications** (Message ding, Badge
  unlocked) · **Media** (Clips).
- Colour tokens only (`--accent-text`, `--focus-ring`, `--shadow-*` …); no new raw colours.
- Server and protocol untouched.

## Review Focus

1. Corrupt / partial `pulse.mixer` (missing section, string values, NaN, v: 2) → defaults per field, never a crash.
2. Legacy `pulse.sounds` with `volume_pct: 0` → all three sound masters 0 (not 1 %).
3. A Calls change while in a call applies live; one made before joining applies on join (core keeps it with no session).
4. Moving the popover slider updates the open mixer's Soundboard notch (same reactive state), and vice versa.
5. Master 200 % × item 200 % → gain capped at `MAX_GAIN`, both in TS (`mixGain`) and the core (`sounds::gain`,
   `soundboard::gain`, mixer per-kind).

---

### Task 1: Core gain maths and per-kind mixer gain

**Files:** `client/src-tauri/src/voice/mixer.rs`, `src/sounds.rs`, `src/soundboard/mod.rs`

**Produces:** `mixer::MAX_GAIN: f32`, `mixer::combined_gain(pcts: &[u16]) -> f32`,
`MixerInput::set_kind_gain(kind, gain)`, `MixerInput::effective_gain(source) -> Option<f32>` (test helper),
`soundboard::gain(calls_pct, soundboard_pct, sound_pct)`, `sounds::gain` capped at `MAX_GAIN`.

- [ ] Tests: `combined_gain(&[200, 200]) == MAX_GAIN`, `combined_gain(&[100, 50]) == percent_to_gain(50)`,
  `combined_gain(&[0, 200]) == 0`; `kind_gain_multiplies_person_gain` (person 0.5 × Voice kind 0.5 → 0.25 sent to the
  speakers, Stream sources untouched); `kind_gain_set_before_sources_and_kept_across_clear`;
  `kind_gain_is_capped`; mixed output of a source at 0.5×0.5 is 0.25 × input; `soundboard::gain(100, 200, 100)` is
  `percent_to_gain(200)`, `gain(50, 100, 100) == percent_to_gain(50)`, `gain(200,200,200) == MAX_GAIN`;
  `sounds::gain(Some(9.0)) == MAX_GAIN`; UI sounds are soft-limited (`shape(0.9, 2.0) == soft_limit(1.8)`, quiet audio
  untouched).
- [ ] Run, see them fail; implement; pass; commit `feat(core): per-kind mixer gain and capped combined gains (#44)`.

### Task 2: Core plumbing — `set_call_mix` and soundboard commands

**Files:** `src/voice/mod.rs`, `src/commands.rs`, `src/lib.rs`

**Produces:** command `set_call_mix { callsPercent, voicesPercent, streamsPercent }`; `soundboard_play` /
`soundboard_preview` take `calls_percent: u16`. `VoiceManager::set_call_mix(calls, voices, streams)` stores kind gains
(kept across sessions) and applies to the live session; a new session applies them right after `mix_in.clear()`.

- [ ] Test (voice/mod.rs, existing session-free test style): `call_mix_is_kept_without_a_session` — set before any
  session, read back via a `kind_gains()` accessor = `combined_gain(&[calls, voices])` per kind.
- [ ] Implement, pass, commit `feat(core): set_call_mix command (#44)`.

### Task 3: `lib/mixer.ts` — prefs, maths, migration

**Files:** create `client/ui/src/lib/mixer.ts`, `mixer.test.ts`

**Produces:** `type MixerPrefs`, `MIXER_KEY = 'pulse.mixer'`, `MIXER_SECTIONS` (ids, labels, master flag, notch keys +
labels + `max` + `test` kind), `defaultMixer()`, `percentToGain`, `gainToPercent`, `MAX_GAIN`, `mixGain(...pcts)`,
`normalizeMixer(raw): MixerPrefs | null` (null when not a v1 record), `migrateMixer(legacy)`, `loadMixer()`,
`saveMixer(p)`, `soundSectionOf(name)`, `soundGain(p, name)`, `soundMasterGain(p, name)`, `resetSection(p, id)`.

- [ ] Tests: anchors (`percentToGain(0)=0`, `(100)=1`, `(200)≈3.981`, `(1)≈0.0105`); `gainToPercent` round-trips and
  `gainToPercent(0.5) === 85`, `(0) === 0`; defaults all 100 / clipsMuted false; migration: sounds `{volume_pct: 50,
  off: ['join','badge']}` → masters 85, join 0, badge 0; `volume_pct: 0` → masters 0; soundboard 40 → 40; clipVolume
  `{volume: 0.5, muted: true}` → clips 85, clipsMuted true; nothing stored → defaults; persistence round-trip;
  corrupt record (strings, NaN, 999, missing sections) → per-field defaults/clamps; `v: 2` → migrated from legacy;
  `mixGain(200, 200) === MAX_GAIN`; `soundGain` = master × item, 0 when either is 0; `resetSection` resets only that
  section.
- [ ] Implement, pass, commit `feat(ui): volume mixer prefs and migration (#44)`.

### Task 4: Wire the paths (UI sounds, soundboard, clips, Calls)

**Files:** create `lib/mixer.svelte.ts` (`mixer` $state, `setNotch(section, key, pct)`, `resetMixerSection(id)`,
`pushCallMix()`); modify `lib/voiceui.ts` (`playSound`/`previewSet` use the mixer; `invokeSound(name, set, gain)`;
`playSoundAt(name, gain)` for test buttons), `lib/store.svelte.ts` (sound decision log), `lib/soundboard.ts`
(drop `loadSoundboardPrefs`/`saveSoundboardPrefs`; `previewPercents(calls, soundboard)`), `lib/soundboard.svelte.ts`,
`lib/tauri.ts` (`soundboardCmd.play(hash, callsPct, soundboardPct, soundPct, output)`, `preview(..., callsPct, ...)`),
`lib/clipPlayback.svelte.ts` (volume = Clips percent, `elementVolume(pct)`), `components/ClipPlayer.svelte` (0–100
slider), `components/Soundboard.svelte` + `SoundDialog.svelte`, `lib/voice.svelte.ts` (push call mix before join).

- [ ] Tests first: `playSound('join')` with Call sounds 50 × Join 200 invokes `play_sound` with
  `volume = mixGain(50, 200)`; with Join 0 nothing is invoked; message uses Notifications; `setNotch('calls', …)`
  invokes `set_call_mix`; the soundboard listener passes calls + soundboard percents; popover slider writes
  `mixer.calls.soundboard` (max 200); clip slider writes `mixer.media.clips` and sets `el.volume =
  min(1, percentToGain(pct))`; join pushes `set_call_mix` before `join_voice`.
- [ ] Update existing tests that pinned the old keys; implement; pass; commit
  `feat(ui): apply the mixer in every audio path (#44)`.

### Task 5: Settings → Volume mixer UI

**Files:** create `components/VolumeMixer.svelte`, `VolumeMixer.test.ts`; modify `SoundSettings.svelte` (+test: the
volume slider and switches move to the mixer), `VoiceSettings.svelte` (mount it after Sounds).

- [ ] Tests: five section headings in order; every notch has a labelled range (0–200, Clips 0–100) showing `NN%`;
  moving a slider saves `pulse.mixer`; ▶ on Join plays `join` at master × item; ▶ Soundboard plays the first sound via
  `soundboard_play`, disabled with no sounds; per-section "Reset to defaults" resets only that section; the popover
  slider and the mixer stay in sync (shared state).
- [ ] Implement with existing tokens; pass; commit `feat(ui): Volume mixer settings section (#44)`.

### Task 6: Review, merge main, final checks

- [ ] Fresh reviewer pass; fix Critical/Important test-first.
- [ ] Open the parked issue(s) for deferrals (clip boost > 100 %).
- [ ] `git merge main`; fmt, clippy `-D warnings`, `cargo test -p pulse-client --lib`, `pnpm check`, `pnpm test`;
  final commit `Closes #44`.
