# Soundboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In a call, play a short per-server sound that everyone in the call hears, each listener at their own
soundboard volume (issue #43).

**Architecture:** The server stores each sound as an Ogg Opus file in the disk media store (new media kind `sound`)
plus a `sounds` row, and relays `SoundPlayed` only to the people in the player's call. Every client fetches the file
through the core's verified `pulse-media` cache, decodes it in pure Rust (`opus-decoder`) and plays it through the
voice mixer (so it respects the output device and deafen). Uploads are trimmed and loudness-normalised by the core's
ffmpeg (as clips are) before they leave the machine.

**Tech Stack:** Rust (axum + SQLite server, `pulse-protocol`, Tauri 2 core, cpal mixer, `opus-decoder` 0.1), Svelte 5.

**Spec:** `docs/superpowers/specs/2026-11-03-soundboard-design.md` (and `gh issue view 43 --comments`).

## Builder's decisions (where the spec leaves the call to us)

- **Boost is a stored playback gain**, not baked into the file: `sounds.volume` (percent, 10–200, default 100,
  `percent_to_gain` curve, 200 % = +12 dB). The upload dialog's boost slider sets it and Edit can change it later
  without re-encoding. The file itself is loudness-normalised (two-pass `loudnorm`, I=-16, TP=-1.5, LRA=11).
- **Hard delete.** Nothing references a sound after it is gone (no messages carry it), so the row goes (favourites
  and usage cascade) and media cleanup reclaims the file. No `deleted_at`.
- **Play is REST** (`POST /sounds/{id}/play`), the server finds the caller's current call itself (`VoiceState`), so a
  client can't aim a sound at a call it isn't in. The player hears it from the relayed event like everyone else.
- **Ogg Opus parsing lives in `pulse-protocol`** (`pulse_protocol::sound`): the server validates with it and the core
  demuxes with it — one parser, one set of limits.
- **Playback outside a call** (preview in the upload dialog) goes through the UI-sound path (`sounds.rs`, one-shot
  cpal stream on the chosen output) since there is no mixer then.
- **Emoji on a sound:** Unicode only (custom-emoji glyphs deferred to an issue).

## Global Constraints

- Migration file: `server/migrations/0016_soundboard.sql` (badges takes 0015; renumber if main differs at merge).
- Sound length ≤ **6 s** in the client trim; server accepts ≤ **6.2 s** (`MAX_DURATION_MS = 6200`), > 0.
- File: Ogg Opus, 1–2 channels (client sends mono, `-b:a 64k`, 48 kHz); server size cap `MAX_BYTES = 256 * 1024`.
- Names 1–32 characters after trimming, no control characters. Emoji optional: 1–16 chars, no ASCII letters/digits,
  no whitespace or control chars.
- Upload formats in the picker: `mp3`, `ogg`, `wav`, `m4a`.
- Rate limits: plays 10 per second per person (`Rate::new(10, 10.0)`, `sound_play_user`); create/edit/delete/
  favourite `Rate::per_hour(120)` (`sound_user`); the media upload uses the existing `upload_user`.
- "Tidy up?" shows past **24** sounds in a server, listing the 5 least-played.
- Frequently used: top **9** (3 rows of the 3-column grid), decayed like `emoji_usage` (14-day half-life).
- Soundboard volume: 0–100 %, default 100, per device in localStorage `pulse.soundboard`; 0 = off (nothing fetched
  or played). Per-person soundboard mute in localStorage `pulse.soundmutes`.
- New protocol fields `#[serde(default)]`. No protocol version bump (unknown events are ignored by old clients).
- Colours only via `app.css` tokens (`colorTokens.test.ts`); text colours `--accent-text` etc.
- Build throttle on every cargo command: `CARGO_BUILD_JOBS=4 flock ~/Projects/pulse-app/target/.cargo-serial nice -n 19 ionice -c3 cargo …`, tests `-- --test-threads=4`. pnpm via `nice -n 19` with node from `~/.config/nvm/versions/node/v22.22.3/bin`.
- Test audio is generated (sine/silence via code or `ffmpeg -f lavfi`), never downloaded. Test people alex/sam/kim/jo.
- Commits reference #43, end with the Co-Authored-By / Claude-Session lines.

## Review Focus

1. A listener who is deafened, not in that call, or has soundboard volume 0 hears nothing (server sends only to the
   call; client also checks `voice.channelId === channel_id`; mixer master 0 when deafened). Tests: Task 3 relay test
   (outsider gets no event), Task 7 `shouldPlay` cases.
2. Spam clicking a tile: the 11th play in a second is a quiet 429, not an error banner. Tests: Task 3 rate-limit test,
   Task 8 tile click swallows a 429.
3. Odd upload input: a silent file (loudnorm on silence must not fail or emit NaN), a stereo 44.1 kHz source, a file
   that isn't audio. Tests: Task 5 `silence_encodes`, `stereo_44k_becomes_mono_48k`, `not_audio_is_refused`.
4. A sound deleted while it's on screen or mid-play: tile vanishes on `SoundDeleted`; a stale click's 404 is quiet;
   an in-flight `SoundPlayed` still plays (it carries the hash). Tests: Task 2 delete test, Task 7 reducer test.
5. A hostile upload body (not Ogg, wrong CRC, too long by granule, OpusHead missing) is a 400 and never stored. Tests:
   Task 1 parser cases, Task 2 upload validation.

---

### Task 1: Protocol — Ogg Opus parser and wire types

**Files:**
- Create: `protocol/src/sound.rs`
- Modify: `protocol/src/lib.rs` (module + `export_all`), `protocol/src/rest.rs`, `protocol/src/gateway.rs`
- Generated: `client/ui/src/lib/protocol/*.ts` (`cargo test -p pulse-protocol export_bindings`)

**Interfaces — Produces:**
- `pulse_protocol::sound::{MAX_DURATION_MS: u32 = 6200, MAX_BYTES: usize = 256*1024, MAX_NAME: usize = 32, OggOpus, SoundFileError, parse_ogg_opus(bytes: &[u8]) -> Result<OggOpus<'_>, SoundFileError>, clean_name(&str) -> Option<String>, valid_emoji(&str) -> bool}`
- `OggOpus<'a> { channels: u8, pre_skip: u16, packets: Vec<&'a [u8]> /* audio packets only, header packets excluded; a packet spanning pages is joined — store as Vec<Cow<'a,[u8]>> */, duration_ms: u32 }` where duration = (last granule − pre_skip) / 48.
- `rest::Sound { id: String, server_id: ServerId, name: String, emoji: Option<String>, media_hash: String, volume: u16, duration_ms: u32, added_by: Option<UserId>, created_at: String, plays: u32 }`
- `rest::CreateSoundRequest { name, emoji: Option<String>, media_hash, volume: u16 (serde default 100) }`, `rest::UpdateSoundRequest { name: Option<String>, emoji: Option<Option<String>> (serde default; `null` clears), volume: Option<u16> }`
- `Ready` gains `#[serde(default)] sounds: Vec<Sound>`, `sound_favourites: Vec<String>`, `frequent_sounds: Vec<String>`.
- `Event` gains `SoundCreated { sound }`, `SoundUpdated { sound }`, `SoundDeleted { server_id, sound_id }` (no channel → everyone), `SoundPlayed { channel_id, user_id, sound_id, media_hash, volume }` (`channel_id()` = Some), `SoundFavourites { user_id, favourites: Vec<String> }` and `FrequentSounds { user_id, frequent: Vec<String> }` (both `only_for`).

- [ ] **Step 1: failing tests** in `sound.rs`: build Ogg pages in the test (helper writing `OggS` pages with a correct CRC; OpusHead with channels=1, pre_skip=312; OpusTags; audio pages with granule positions). Cases: `parses_a_mono_stream` (channels 1, packets count, duration_ms = (granule−312)/48); `refuses_not_ogg`, `refuses_bad_crc`, `refuses_missing_opus_head`, `refuses_too_long` (granule over 6.2 s), `refuses_empty_audio` (duration 0), `joins_a_packet_split_across_pages` (lacing 255 continuation). `clean_name`: `"  airhorn "`→`"airhorn"`, `""`/33 chars/`"a\u{7}"` → None; `valid_emoji`: `"📯"` ok, `"ab"`, `" "`, 17 chars → false.
- [ ] **Step 2:** run `… cargo test -p pulse-protocol sound -- --test-threads=4`, expect compile failure.
- [ ] **Step 3:** implement (Ogg CRC-32 poly `0x04c11db7`, init 0, no reflection, CRC field zeroed while summing). Add the wire types above, `Event::channel_id`/`only_for` arms, export_all entries for `Sound`, `CreateSoundRequest`, `UpdateSoundRequest`.
- [ ] **Step 4:** tests pass; run export_bindings; the generated `.ts` files appear.
- [ ] **Step 5:** commit `soundboard: Ogg Opus parser and wire types (#43)`.

### Task 2: Server — table, media kind, upload validation, CRUD, Ready

**Files:**
- Create: `server/migrations/0016_soundboard.sql`, `server/src/sounds/mod.rs`, `server/src/sounds/routes.rs`, `server/tests/sounds.rs`
- Modify: `server/src/lib.rs` (module + router merge), `server/src/media/mod.rs` (`MediaKind::Sound` "sound", disk; `put_sound`; cleanup keeps `sounds.media_hash`), `server/src/media/routes.rs` (`POST /media/sound`; `fetch` serves the row's `content_type`), `server/src/limits.rs` (`sound_user`, `sound_play_user` in config, defaults, `unlimited`), `server/src/gateway/socket.rs` (Ready fields)

Migration:
```sql
CREATE TABLE sounds (id TEXT PRIMARY KEY, server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
  name TEXT NOT NULL, emoji TEXT, media_hash TEXT NOT NULL REFERENCES media(hash), volume INTEGER NOT NULL DEFAULT 100,
  duration_ms INTEGER NOT NULL, added_by TEXT REFERENCES users(id), created_at TEXT NOT NULL,
  plays INTEGER NOT NULL DEFAULT 0) STRICT;
CREATE INDEX sounds_server ON sounds(server_id);
CREATE TABLE sound_favourites (user_id TEXT NOT NULL REFERENCES users(id), sound_id TEXT NOT NULL REFERENCES sounds(id) ON DELETE CASCADE,
  created_at TEXT NOT NULL, PRIMARY KEY (user_id, sound_id)) STRICT;
CREATE TABLE sound_usage (user_id TEXT NOT NULL REFERENCES users(id), sound_id TEXT NOT NULL REFERENCES sounds(id) ON DELETE CASCADE,
  score REAL NOT NULL, last_used_at TEXT NOT NULL, PRIMARY KEY (user_id, sound_id)) STRICT;
```

**Interfaces — Produces:** `media::put_sound(db, dir, bytes: &[u8], by) -> AppResult<MediaInfo>` (content_type `audio/ogg`, width/height 0, frames 0); `sounds::{for_ready(db) -> Vec<Sound>, load(db, id) -> Option<Sound>, FREQUENT: usize = 9}`; routes `POST /media/sound` (body ≤ MAX_BYTES + 4 KiB, `parse_ogg_opus` then store; duration from the parser), `POST /servers/{id}/sounds` (CreateSoundRequest → Sound; media row must be kind `sound`; `duration_ms` comes from re-reading and parsing the stored file, ≤ 256 KiB), `GET /servers/{id}/sounds` (fresh plays), `PATCH /sounds/{id}`, `DELETE /sounds/{id}` (all: member of the sound's server, else 403; missing 404; volume outside 10..=200 → 400).

- [ ] **Step 1: failing tests** in `server/tests/sounds.rs` (generate valid Ogg Opus bytes with a test helper that writes pages with OpusHead/OpusTags and a few dummy audio packets — the server never decodes): `upload_accepts_a_short_ogg_opus` (200, MediaInfo content_type `audio/ogg`; `GET /media/{hash}` returns `audio/ogg` and the same bytes); `upload_refuses_not_ogg_too_long_and_too_big` (400/400/413); `member_creates_and_everyone_sees_it` (201/200 Sound, `SoundCreated` reaches another user's gateway, Ready carries it); `create_rejects_bad_name_emoji_volume_and_non_sound_media` (an emoji upload's hash → 400); `non_member_cannot_create_edit_or_delete` (403); `anyone_in_the_server_edits_and_deletes` (sam edits alex's sound; `SoundUpdated`; sam deletes; `SoundDeleted`; Ready no longer has it); `deleted_sound_media_is_cleaned_up` (`media::cleanup` with zero grace removes it once deleted, keeps it while live).
- [ ] **Step 2:** run `… cargo test -p pulse-server --test sounds -- --test-threads=4` → fails.
- [ ] **Step 3:** implement as above, following `emoji/` (write lock via `begin_write`, publish after commit, `writes_ok`).
- [ ] **Step 4:** `--test sounds` and `--test media --test emoji` pass.
- [ ] **Step 5:** commit `soundboard: server sounds table, upload validation and CRUD (#43)`.

### Task 3: Server — play relay, plays, frequently used, favourites, rate limit

**Files:** Modify `server/src/sounds/{mod,routes}.rs`, `server/src/gateway/hub.rs` (`publish_to`), `server/src/gateway/socket.rs`; Test `server/tests/sounds.rs`.

**Interfaces — Produces:** `Hub::publish_to(&self, db, event, only: &HashSet<UserId>)` — the normal `audience_for` result intersected with `only` (privacy choke point stays). Routes: `POST /sounds/{id}/play` → 204 (429 over `sound_play_user`; 409 "you're not in a call" when `s.voice.room_flags(me)` is None; 403 non-member; 404 missing); increments `plays`, records `sound_usage` (decay like `emoji::record_use_at`), publishes `SoundPlayed` to `s.voice.members(channel)` and `FrequentSounds` to the player. `PUT /sounds/{id}/favourite`, `DELETE /sounds/{id}/favourite` → 204 + `SoundFavourites` to own sessions. `sounds::frequent(db, user, n) -> Vec<String>` (skips sounds of servers the user left), `sounds::favourites(db, user) -> Vec<String>` (oldest first).

- [ ] **Step 1: failing tests:** `play_reaches_only_the_call` (alex+sam in the call via `app.state().voice.join`, kim online not in it: alex plays → alex and sam get `SoundPlayed{sound_id, media_hash, volume, user_id: alex}`, kim gets nothing within 300 ms); `play_works_while_muted` (alex's flags muted); `play_outside_a_call_is_409`; `play_from_another_server_you_are_in_works` and `non_member_play_is_403`; `plays_are_counted` (`GET /servers/{id}/sounds` plays == 3 after 3 plays); `frequent_and_favourites_are_per_user` (Ready of alex has frequent [id], sam's empty; favourite → event to alex only; Ready carries it; unfavourite clears); `eleventh_play_in_a_second_is_429` (config with `sound_play_user = Rate::new(10, 10.0)`: 10×204 then 429).
- [ ] **Step 2:** run, fails. **Step 3:** implement. **Step 4:** `--test sounds`, `--test gateway`, `--test privacy` pass.
- [ ] **Step 5:** commit `soundboard: play relay to the call, plays, frequent and favourites (#43)`.

### Task 4: Core — decode and play through the mixer

**Files:** Create `client/src-tauri/src/soundboard/mod.rs`, `client/src-tauri/src/soundboard/decode.rs`; Modify `client/src-tauri/Cargo.toml` (`opus-decoder = "0.1"`), `client/src-tauri/src/lib.rs` (`mod soundboard`), `client/src-tauri/src/voice/mixer.rs` (`Cmd::Clip`, `MixerInput::play_clip(&self, samples: Arc<[f32]>, gain: f32)`, `Mixer` keeps `clips: Vec<Clip>` with capacity 16, at most `MAX_CLIPS = 32` at once, mixed before `soft_limit`, scaled by `master`), `client/src-tauri/src/sounds.rs` (`play_samples(sound: Sound, volume: f32, output: Option<String>)` sharing `play`'s device logic; `play_on` takes `&Sound`).

**Interfaces — Produces:** `soundboard::decode::decode(bytes: &[u8]) -> Result<Arc<[f32]>, String>` (48 kHz mono, stereo downmixed, pre-skip dropped); `soundboard::Cache` (LRU of 24 decoded sounds by hash: `get_or_decode(hash, bytes)`); `soundboard::gain(soundboard_pct: u16, sound_pct: u16) -> f32` = `percent_to_gain(min(sb,100)) * percent_to_gain(clamp(sound,10,200))`.

- [ ] **Step 1: failing tests:** decode test encodes a generated 1 s 440 Hz sine with system ffmpeg (skip with a printed note when `PULSE_DEV_FFMPEG_DIR`/`ffmpeg` isn't present — same convention as clips tests) and asserts length ≈ 48 000 ± 960 and RMS ≈ that of the input (± 20 %); `decode_refuses_garbage`. Mixer: `a_clip_plays_once_at_its_gain` (`play_clip([0.5;480], 0.5)` → first pull ≈ 0.25, next pull 0); `overlapping_clips_add` (two clips → sum, soft-limited); `deafened_master_silences_clips` (master 0 → 0); `clips_survive_clear` is NOT wanted: `clear()` drops clips (channel switch). `gain(0, x) == 0`, `gain(100,100) == 1`.
- [ ] **Step 2:** run `… cargo test -p pulse-client --lib soundboard mixer -- --test-threads=4` → fails.
- [ ] **Step 3:** implement. **Step 4:** pass. **Step 5:** commit `soundboard: decode Ogg Opus and mix sounds into the call (#43)`.

### Task 5: Core — upload pipeline (waveform, trim, loudnorm, encode)

**Files:** Create `client/src-tauri/src/soundboard/encode.rs`; reuse `clips::ffmpeg::{command, output, locate, dev_override, Os}`.

**Interfaces — Produces:** `encode::waveform(ffmpeg: &Path, file: &Path) -> Result<Waveform, String>` with `Waveform { duration_ms: u32, peaks: Vec<f32> /* 200 buckets, 0..=1 */ }` (decode `-t 600 -ac 1 -ar 8000 -f s16le pipe:1`; source over 50 MB refused); `encode::encode(ffmpeg: &Path, file: &Path, start_ms: u32, end_ms: u32) -> Result<Vec<u8>, String>` — clamps the span to ≤ 6000 ms, pass 1 `loudnorm=I=-16:TP=-1.5:LRA=11:print_format=json` (parse the JSON block from stderr), pass 2 `loudnorm=…:measured_I=…:measured_TP=…:measured_LRA=…:measured_thresh=…:offset=…:linear=true`, `-ac 1 -ar 48000 -c:a libopus -b:a 64k -f ogg pipe:1`; measured `-inf` (silence) → skip loudnorm. Result checked with `parse_ogg_opus` before returning. User-facing errors: `"Pulse couldn't read that audio file."`, `"Sounds can be at most 6 seconds."` (never for a clamped span).

- [ ] **Step 1: failing tests** (fixtures generated with ffmpeg `-f lavfi -i sine=…`, skipped without ffmpeg): `waveform_has_200_peaks_and_the_duration`; `encode_trims_to_the_span` (10 s source, 2000–5000 → parse duration 3000 ± 40 ms); `encode_clamps_to_six_seconds`; `a_quiet_clip_comes_out_louder` (sine at −40 dBFS → decoded RMS ≥ 10× the input's); `silence_encodes`; `stereo_44k_becomes_mono_48k` (OggOpus channels 1); `not_audio_is_refused`.
- [ ] **Step 2–4:** fail, implement, pass. **Step 5:** commit `soundboard: trim and loudness-normalise uploads with ffmpeg (#43)`.

### Task 6: Core — API, commands and playback wiring

**Files:** Modify `client/src-tauri/src/api.rs`, `client/src-tauri/src/commands.rs`, `client/src-tauri/src/lib.rs` (handler list + managed `soundboard::Cache` and `soundboard::Prepared`), `client/src-tauri/src/soundboard/mod.rs`.

**Interfaces — Produces (Tauri commands, all prefixed `soundboard_` except CRUD):**
- `soundboard_play(hash: String, soundboard_percent: u16, sound_percent: u16, output: Option<String>)` — media via `media::serve(core.media_session(), "/<hash>")`, decode via cache, then the call's mixer (`VoiceManager::mixer_input`) else `sounds::play_samples`. Returns `Result<(), String>`; nothing is fetched when the gain is 0.
- `soundboard_support() -> ClipSupport` (same rule as clips; reason copy `"Adding sounds needs Pulse's audio tools (Windows), or ffmpeg on Linux."`), `soundboard_pick_file() -> Option<String>` (filter `mp3, ogg, wav, m4a`), `soundboard_open(path) -> Waveform`, `soundboard_preview(path, start_ms, end_ms, sound_percent, soundboard_percent, output)` (encodes, keeps the bytes in `Prepared` keyed by (path,start,end), plays locally), `soundboard_upload(path, start_ms, end_ms) -> MediaInfo` (reuses `Prepared` or encodes; `POST /media/sound`).
- `create_sound(server_id, name, emoji, media_hash, volume) -> Sound`, `update_sound(id, name, emoji, volume) -> Sound`, `delete_sound(id)`, `list_sounds(server_id) -> Vec<Sound>`, `play_sound_to_call(id)`, `set_sound_favourite(id, on)`.
- Api: `upload_sound_media`, `create_sound`, `update_sound`, `delete_sound`, `list_sounds`, `play_sound_to_call`, `set_sound_favourite` (paths as in Tasks 2–3).

- [ ] **Step 1: failing test** `client/src-tauri/tests/soundboard_e2e.rs` against the in-process test server (pattern: `tests/emoji_e2e.rs`): upload generated Ogg bytes via `Api::upload_sound_media`, create, list, favourite, play while in a call (`app.state().voice.join`), delete. Unit test in commands/soundboard: `prepared_is_reused_for_the_same_span`.
- [ ] **Step 2–4:** fail, implement, pass (`cargo test -p pulse-client --lib` and `--test soundboard_e2e`).
- [ ] **Step 5:** commit `soundboard: core commands for playing, uploading and managing sounds (#43)`.

### Task 7: UI — state, prefs and the listener

**Files:** Modify `client/ui/src/lib/state.ts` (fields `sounds: Record<string, Sound>`, `soundFavourites: string[]`, `frequentSounds: string[]`; Ready + the six events), `client/ui/src/lib/tauri.ts` (wrappers for Task 6); Create `client/ui/src/lib/soundboard.ts` (+ test), `client/ui/src/lib/soundboard.svelte.ts`.

**Interfaces — Produces** (`soundboard.ts`, pure): `loadSoundboardPrefs(): { volume_pct: number }`, `saveSoundboardPrefs`, `loadSoundMutes(): Record<string, true>`, `saveSoundMutes`; `shouldPlay(e: {channel_id, user_id}, ctx: { myChannel: string | null, volume_pct: number, mutes: Record<string,true> }): boolean`; `soundSections(s: AppState, memberOf: Set<string>, servers: Server[]): Section[]` with `Section = { id: 'favourites' | 'frequent' | `server:${id}`, label, sounds: Sound[], server?: Server }` (favourites in favourite order, frequent ≤ 9, each member server's sounds by name; empty favourites/frequent omitted, an empty server section kept so "Add sound" shows); `searchSounds(sounds: Sound[], q: string): Sound[]` (case-insensitive name contains, prefix matches first); `tidyCandidates(sounds: Sound[]): Sound[]` (5 least plays, then oldest). `soundboard.svelte.ts`: `soundboard = $state({ volume_pct, mutes })`, `startSoundboardListening()` (onAppEvent `SoundPlayed` → `shouldPlay` → `invoke('soundboard_play', { hash, soundboardPercent, soundPercent, output })` with the output device from `loadAudioConfig()`), `setSoundboardVolume`, `setSoundMute(userId, on)`. Wire `startSoundboardListening` where `startVoiceListening` starts.

- [ ] **Step 1: failing tests** `soundboard.test.ts`: `shouldPlay` (other call → false, volume 0 → false, muted person → false, own sound in my call → true); `soundSections` order and frequent cap and non-member server excluded; `searchSounds` prefix first; `tidyCandidates`; prefs round-trip and a corrupt localStorage value falls back. `state.test.ts`: Ready fills sounds; `SoundDeleted` removes the sound and its favourite; `SoundFavourites`/`FrequentSounds` replace lists.
- [ ] **Step 2–4:** `nice -n 19 pnpm test` fail → implement → pass; `pnpm check` 0/0.
- [ ] **Step 5:** commit `soundboard: UI state, prefs and playback on SoundPlayed (#43)`.

### Task 8: UI — the soundboard popover and call controls

**Files:** Create `client/ui/src/components/Soundboard.svelte` (+ `Soundboard.test.ts`), `client/ui/src/components/SoundboardPopover.svelte`; Modify `client/ui/src/components/VoicePanel.svelte` (a `.ctl` button `aria-label="Soundboard"` after Deafen, `data-menu-toggle`), `client/ui/src/components/Icon.svelte` (`soundboard`, `star`, `volume` icons), `client/ui/src/lib/participantMenu.ts` + `ParticipantMenu.svelte` (`'sound-mute'` item after `'local-mute'`: "Mute soundboard" / "Unmute soundboard"), `client/ui/src/app.css` (any new tokens).

Behaviour: top row = search `placeholder="Find a sound"` + a speaker button (`aria-label="Soundboard volume"`) toggling a 0–100 slider; left rail buttons: Favourites (star), Frequently used (clock), one per member server (icon or initial) — click scrolls to the section; main = sections of a 3-column grid of tiles (`.tile`, emoji + name; a small `.preview` button `aria-label="Preview {name}"` that calls `soundboard_play` locally only and never the server); tile click → `play_sound_to_call(id)`, a 429 ignored, other errors shown inline; right-click tile → menu Favourite/Unfavourite, Edit, Delete (confirm "Delete {name}?"); each server section ends with an "Add sound" tile (opens Task 9's dialog), and past 24 sounds a "Tidy up?" link (Task 9). Escape closes; outside click closes (`clickOutside`), popover placed with `popoverStyle` above the button.

- [ ] **Step 1: failing tests** `Soundboard.test.ts` (mock invoke): sections render with 3-column tiles; typing in search filters and hides sections; rail click scrolls (`scrollIntoView` called with the section); tile click invokes `play_sound_to_call` and not `soundboard_play`; preview invokes `soundboard_play` and not `play_sound_to_call`; context menu Favourite invokes `set_sound_favourite {on:true}`; Delete asks first, then `delete_sound`; volume slider writes `pulse.soundboard`; "Add sound" shown per member server; "Tidy up?" only past 24. `participantMenu.test.ts`: `'sound-mute'` for others, not for self.
- [ ] **Step 2–4:** fail → implement → pass; `pnpm check` 0/0; colour test passes.
- [ ] **Step 5:** commit `soundboard: the soundboard popover in the call controls (#43)`.

### Task 9: UI — add/edit dialog and tidy up

**Files:** Create `client/ui/src/components/SoundDialog.svelte` (+ test), `client/ui/src/components/SoundTidy.svelte`; Modify `Soundboard.svelte`.

Add mode: if `soundboard_support().can_send` is false show its reason. "Choose a file" → `soundboard_pick_file` → `soundboard_open` → SVG waveform of `peaks` with two draggable handles (pointer + arrow keys, 100 ms steps; the span never exceeds 6000 ms, initial span = first min(6 s, duration)); fields Name (1–32, required), Emoji (optional, a text input of one emoji), Boost slider 10–200 % (default 100); "Preview" → `soundboard_preview`; "Save" → `soundboard_upload` then `create_sound`. Edit mode (from the tile menu): name, emoji, boost only → `update_sound`. Tidy up: fetches `list_sounds(server)`, lists `tidyCandidates` with play counts and a Delete button each (`delete_sound`, no extra confirm — the button says "Delete").

- [ ] **Step 1: failing tests** `SoundDialog.test.ts`: span is clamped to 6 s when a handle is dragged past it; Save disabled without a name or file; Save calls `soundboard_upload` with the span then `create_sound` with name/emoji/volume; edit mode calls `update_sound` and never uploads; unsupported install shows the reason. Tidy: lists the 5 least played.
- [ ] **Step 2–4:** fail → implement → pass; `pnpm check` 0/0.
- [ ] **Step 5:** commit `soundboard: add, edit and tidy up sounds (#43)`.

### Task 10: Finish

- [ ] Fresh reviewer pass (superpowers:requesting-code-review); fix Critical/Important test-first.
- [ ] `git merge main`; renumber the migration if 0016 is taken.
- [ ] Final checks (fmt; clippy `--workspace --all-targets -D warnings`; `cargo test -p pulse-server`; `cargo test -p pulse-client --lib`; bindings no diff; `pnpm check`; `pnpm test`).
- [ ] Open issues for deferrals (custom-emoji glyphs on sounds; join sounds already deferred by the spec — check it has an issue; per-category volume #44 hook-up).
- [ ] Final commit `Closes #43`.
