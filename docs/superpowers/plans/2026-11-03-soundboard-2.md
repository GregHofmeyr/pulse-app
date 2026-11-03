# Soundboard round 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Issue #138 (and #122): a Server settings → Soundboard page, the chat emoji picker (custom emoji too) in the
add/edit dialog, trimmer play/pause with a playhead, independent handle dragging, "who played it" rings, and a
measured, smaller click → audible delay.

**Architecture:** Server and protocol gain a sound's custom emoji (`emoji_id`, migration 0017) and the sound's length on
`SoundPlayed`. The core gets a de-duplicating sound loader with a warm-up command, timestamps along the play path, a
stoppable trimmer playback and a clip start-delay probe in the mixer. The UI adds `SoundboardSettings.svelte`, reworks
`SoundDialog.svelte` (picker, play/pause, drag rules in `soundTrim.ts`), and a "playing" set in `soundboard.svelte.ts`
that rings unite with the real speaking set at render time only.

**Tech Stack:** Rust (axum, sqlx/SQLite, Tauri 2, cpal), ts-rs bindings, Svelte 5, vitest.

**Spec:** `docs/superpowers/specs/2026-11-03-soundboard-design.md`, issue #138, issue #122.

**Status: built, reviewed and merged with main on the branch (2026-11-03). Ring chosen over the emoji-beside-name fallback.**

## Global Constraints

- New protocol fields are `#[serde(default)]`; no protocol version bump. Bindings regenerated (`cargo test -p pulse-protocol export_bindings`) with no diff afterwards.
- Migration: `server/migrations/0017_sound_custom_emoji.sql` (0016 is the soundboard's).
- A sound span is at most 6 s (`MAX_SPAN_MS = 6000`), handles at least `STEP_MS = 100` apart.
- Colour tokens only (`var(--…)`); no new raw colours (`colorTokens.test.ts`, `themeCss.test.ts`).
- Ring "who played" never writes `voice.speaking` (The Doos and speaking-based logic read it).
- Test people alex/sam/kim/jo; generated audio only.
- Stay out of ProfileCard, MemberMenu, NicknameDialog, ConversationList, BadgeToast and the mic-silent notice;
  ParticipantMenu untouched.

## Review Focus

1. A custom emoji from a server the uploader is not in, or a deleted one → 400 (server test).
2. Setting the Unicode emoji clears a custom one and vice versa; both at once → 400 (server test).
3. A custom emoji deleted after the sound was made → tile falls back to the soundboard icon (UI test).
4. Two plays of the same uncached sound fetch and decode once (core test); a play during a warm-up waits for its fetch
   and then reads the disk cache.
5. Warm-up never evicts a decoded sound (core test); its memory stays within `CACHE_ENTRIES`.

## As built (deviations, decided while building)

- Measuring showed `opus-decoder` needs ~0.8 s to decode a 6 s sound even optimised (ffmpeg/libopus: ~30 ms), and a
  first play waited for all of it. So decoding streams: `soundboard/pcm.rs` (`Pcm`, lock-free, one writer),
  `decode::decode_streaming` (resolves after a 100 ms head start), and the mixer plays `ClipSamples::Stream` as far as
  decoded. `LoadTiming.decode_ms` became `start_ms`.
- Warm-up therefore only fetches (fills the media disk cache) and decodes nothing: decoding a call's worth of sounds
  ahead would burn ~0.8 s of CPU per sound on every join, for a gain of ~30 ms.
- `[profile.dev.package.opus-decoder]` is optimised in dev builds (`just linux-client` is a debug build).

---

### Task 1: Custom emoji on sounds (server + protocol)

**Files:** `protocol/src/rest.rs`, `protocol/src/gateway.rs`, `server/migrations/0017_sound_custom_emoji.sql`,
`server/src/sounds/{mod,routes}.rs`, `server/tests/sounds.rs`, `client/src-tauri/src/api.rs`, `commands.rs`,
`client/src-tauri/tests/soundboard_e2e.rs`, `client/ui/src/lib/protocol/*` (generated).

**Interfaces — Produces:**
- `Sound.emoji_id: Option<String>` (serde default), `CreateSoundRequest.emoji_id: Option<String>` (serde default),
  `UpdateSoundRequest.emoji_id: Option<Option<String>>` (absent keeps, null clears, like `emoji`).
- `Event::SoundPlayed.duration_ms: u32` (serde default; 0 from an old server).
- Rule: a stored sound has at most one of `emoji`/`emoji_id`; setting one to a value clears the other; both set in one
  request → 400 "a sound has one emoji". `emoji_id` must be a live (`deleted_at IS NULL`) custom emoji of a server the
  caller is a member of, else 400 "that emoji isn't available".
- Tauri `create_sound(server_id, name, emoji, emoji_id, media_hash, volume)`; `Api::create_sound` takes a
  `&CreateSoundRequest`.

- [ ] Tests (server/tests/sounds.rs): `custom_emoji_on_a_sound` (create with emoji_id from own server; from another
  server alex is in; refuses a server alex isn't in, a deleted one, both fields; update emoji clears emoji_id and back);
  `played_carries_the_length` (SoundPlayed.duration_ms equals the sound's). Run → fail to compile/fail. Capture.
- [ ] Implement; migration `ALTER TABLE sounds ADD COLUMN emoji_id TEXT;` (no FK: custom emoji are soft-deleted).
- [ ] Regenerate bindings; server + protocol tests pass. Commit.

### Task 2: Loader, warm-up and timings (core)

**Files:** `client/src-tauri/src/soundboard/mod.rs`, `commands.rs`, `lib.rs`, `voice/mixer.rs`.

**Interfaces — Produces:**
- `Cache::load<F, Fut>(&self, hash, fetch: F) -> Result<(Arc<[f32]>, LoadTiming), String>` where `fetch` is called at
  most once per hash at a time (a per-hash async lock); `LoadTiming { cached: bool, fetch_ms: f64, decode_ms: f64 }`.
- `Cache::warm<F, Fut>(&self, hashes: &[String], fetch)`: in order, fetch each not already decoded (fills the disk
  cache); decodes nothing (see "As built").
- Command `soundboard_warm(hashes: Vec<String>)` (spawns, returns at once; at most 200 hashes).
- `soundboard_play(..., sound_id: Option<String>)`: logs `soundboard: play` with `cached`, `fetch_ms`, `decode_ms`,
  `click_to_queued_ms` (when this session clicked `sound_id` within 10 s, recorded by `play_sound_to_call`, which also
  logs `rest_ms`).
- Mixer: `Clip` carries `queued: Instant`; the callback stores the first-mix delay in µs in an `AtomicU64`;
  `MixerInput::last_clip_start_us() -> u64`.

- [ ] Tests: `two_loads_of_one_sound_fetch_once`, `warm_fills_only_free_slots`, `warm_skips_failures`,
  `load_reports_cached_on_the_second_play`, `first_play_vs_cached_timing` (prints decode ms of a 6 s ffmpeg sine and the
  cached path; asserts cached < 1 ms), mixer `a_clip_records_its_start_delay`. Fail first. Implement. Commit.

### Task 3: Trimmer play/pause (core)

**Files:** `client/src-tauri/src/soundboard/{mod,encode}.rs`, `commands.rs`, `voice/mixer.rs`, `sounds.rs`.

**Interfaces — Produces:**
- `encode::decode_span(ffmpeg, file, from_ms, to_ms) -> Result<Vec<f32>, String>`: 48 kHz mono f32, at most 6 s.
- A stop flag: `MixerInput::play_clip_stoppable(samples, gain, stop: Arc<AtomicBool>)` and
  `sounds::play_samples_stoppable(sound, gain, output, stop)`; a set flag ends the clip on the next pull.
- `soundboard::Trimming` state: one current stop flag; `soundboard_trim_play(path, from_ms, to_ms, calls_percent,
  soundboard_percent, output) -> u32` (ms that will play; stops the previous one) and `soundboard_trim_stop()`.

- [ ] Tests: `decode_span_is_the_span` (ffmpeg sine, length within 20 ms), mixer `a_stopped_clip_goes_quiet`,
  `trimming_replaces_and_stops`. Fail first, implement, commit.

### Task 4: Trim maths (UI)

**Files:** `client/ui/src/lib/soundTrim.ts`, `soundTrim.test.ts`.

**Interfaces — Produces:** `moveStart(span, ms, dur)`: start ∈ [max(0, end − 6000), end − 100], end unchanged;
`moveEnd`: end ∈ [start + 100, min(dur, start + 6000)], start unchanged; `moveSpan(span, deltaMs, dur)`: both move,
clamped to [0, dur], length kept; `playFrom(span, at: number | null)`: `at` when start ≤ at < end else start.

- [ ] Rewrite the "drags the other along" tests to "stops at 6 s", add `moveSpan`/`playFrom` tests; fail; implement; commit.

### Task 5: Sound dialog: picker, play/pause, drag (UI)

**Files:** `client/ui/src/components/SoundDialog.svelte`, `SoundDialog.test.ts`, `lib/tauri.ts`,
`components/SoundEmoji.svelte` (new: renders a sound's emoji: custom image via `stillSrc`/`animatedEmoji`, Unicode text,
or the soundboard icon), `Soundboard.svelte` (tiles use `SoundEmoji`).

- Emoji: a button showing the current choice opens `EmojiPopover`; a Std pick sets `{emoji: u, emoji_id: null}`, a
  custom pick `{emoji: null, emoji_id: id}`; a "Remove emoji" button clears both. Save sends both.
- Play/pause button (aria-label "Play selection" / "Pause") beside Preview; a playhead line in the waveform
  (`data-testid="playhead"`) driven by `requestAnimationFrame` from the resolved start; at the end handle it stops and
  resets. Handle or selection drags, and closing, stop playback.
- Dragging the selection body (`.sel` rect, role none) moves the span with `moveSpan`.
- [ ] Tests: picker opens and a custom pick is saved as `emoji_id`; a Unicode pick saved as `emoji`; play calls
  `trimPlay(path, start, end, …)`, pause calls `trimStop`; playing from a paused playhead starts there; tile renders a
  custom emoji image and falls back when the emoji is gone. Fail, implement, commit.

### Task 6: Server settings → Soundboard (UI)

**Files:** `components/SoundboardSettings.svelte` (new), `SoundboardSettings.test.ts`, `ServerSettings.svelte`.

- Section "Soundboard" after "Emoji". Rows sorted by name: emoji (`SoundEmoji`), name, added by (`displayName`),
  plays, length ("2.4 s"), buttons Preview / Edit / Remove (inline confirm "Remove {name}?" → Remove / Cancel). "Add
  sound" opens `SoundDialog` add mode; Edit opens edit mode. Empty: "No sounds yet."
- [ ] Tests: lists the server's sounds only with the columns; preview plays locally; remove asks then deletes; add/edit
  open the dialog. Fail, implement, commit.

### Task 7: Who played it + warm-up on join (UI)

**Files:** `lib/soundboard.svelte.ts`, `soundboard.svelte.test.ts`, `lib/soundboard.ts` (pure `warmOrder`),
`components/UserTile.svelte`, `components/ChannelList.svelte` call site in `routes/Shell.svelte`.

- `soundboard.playing: Set<string>`: after a heard `SoundPlayed` resolves `play`, add `user_id` until
  `duration_ms` (event, else the stored sound's, else 2000) has passed; overlapping plays extend. Cleared on leaving.
- Rings: `UserTile` `class:speaking={voice.speaking.has(id) || soundboard.playing.has(id)}`; Shell passes
  `speaking={ringSet(voice.speaking, soundboard.playing)}`. `voice.speaking` is never written.
- `warmOrder(state, memberOf): string[]`: favourites, frequent, then the rest of member servers' sounds, unique hashes.
  On joining a call (`voice.channelId` null → id) call `soundboardCmd.warm(warmOrder(...))`; on `SoundCreated` while in
  a call warm that one.
- [ ] Tests: ring set lights and clears on timers (fake timers); muted person not lit; `voice.speaking` untouched;
  warm order; warm on join. Fail, implement, commit.

### Task 8: Review, merge, final checks

- [ ] One whole-branch review (superpowers:requesting-code-review); fix Critical/Important test-first.
- [ ] `git merge main`; fmt, clippy, server + client tests, bindings no diff, `pnpm check`, `pnpm test`. Issues for
  anything deferred. Final commit `Closes #138`, `Closes #122`.
