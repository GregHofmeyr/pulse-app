# Speaking Rings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** let people pick a speaking ring style (11) and colour (8 + profile colour) that everyone in a call sees, with
three voice-reactive styles fed by real per-speaker loudness, without costing a repaint per frame.

**Architecture:** two optional profile fields (`ring_style`, `ring_color`) ride on `User` exactly as typing text (#652)
does. The Tauri core moves the meter reads and `SpeakingTracker` onto one 70 ms task that also emits
`VoiceEvent::SpeakingLevels` (only while someone speaks and a UI tile watches). The UI draws every ring, Classic
included, with one `SpeakingRing.svelte` inside a reserved 1.32 D box; levels are written straight onto elements as
`--lvl` and smoothed by a compositor `scale` transition, never through Svelte state.

**Tech Stack:** Rust (axum, sqlx/SQLite, ts-rs), Tauri 2 core (tokio), Svelte 5 + vitest.

**Spec:** `docs/superpowers/specs/2026-11-26-speaking-rings-design.md` (all of §11 is settled; section numbers below
are the spec's).

## Global Constraints

- Default is Classic green: `ring_style`/`ring_color` absent on `User` when Classic/green, and `"classic"`/`"green"`
  store NULL (§6.4). Nothing changes for anyone who doesn't opt in (§1).
- Keys, verbatim (§6.2): styles `classic, glow, pulse, ripple, sweep, orbit, halo, viewfinder, liquid, dots, bars`;
  colours `green, blue, gold, pink, cyan, orange, white, purple, profile`. Reactive = `liquid, dots, bars`.
- Error copy (§6.4): `"Unknown speaking ring."`, `"Unknown ring colour."` (400).
- Motion rules (§8.1): ring `@keyframes` touch only `transform`/`opacity`; ring `transition`s name only
  `opacity`/`scale`; no `filter` anywhere in ring CSS; `--lvl` read only inside `scale:` and never animated or
  transitioned; animations only under `data-ring="on"`; no permanent `will-change`.
- Ring box (§3.3): every style at every frame stays inside a 1.32 D square centred on the avatar (0.16 D outset).
- Cap (§8.3): at most 8 animated rings, first 8 to start; an animating ring never drops to still for a newcomer.
- Levels (§5.2): 70 ms tick, `ring_level = clamp((20·log10(rms) + 44) / 30, 0, 1)`, instant attack, release
  `max(new, shown × 0.7)`, 0.05 steps, wire shape `{"kind":"speaking_levels","levels":{"<id>":0.45}}`.
- Copy (§7): tab "Speaking ring"; subtitle "How your avatar lights up when you talk. Everyone in the call sees it.";
  groups "Still or looping" / "Follows your voice"; captions "Preview · talking" / "Preview · still (reduced motion is
  on)"; profile swatch tip "Your name colour, from Name style" (+ " You haven't set one, so it's green."); note "With
  reduced motion on, everyone sees the still version of each ring. In big calls some rings show still to keep things
  smooth."; voice toggle "Show everyone's rings as Classic" / "Shows every speaking ring as the plain ring. Lighter on
  slower PCs, and calmer if motion distracts you."; localStorage key `pulse.classicRings`.
- Greg's UI rule: nothing spills out of its container or jumps. Every UI task carries a test for it.
- Repo rules (`CLAUDE.md`): every commit references `#651`; the last commit of Phase 4 says `Closes #651`; any new
  deferral gets a GitHub issue before its phase merges; test people are alex, sam, kim and jo only.
- Migrations are immutable: the new one is `server/migrations/0085_speaking_ring.sql` (0084 is the last on `main`
  today; take the next free number at build time).
- Protocol types change → `just gen-types` and commit `client/ui/src/lib/protocol/*` in the same commit.
- Local runs (low-load): `flock target/.cargo-serial nice -n 10 ionice -c3 cargo test -j 2 -p <crate> <filter>` and
  `cd client/ui && pnpm vitest run <file>`; targeted tests only, the full suite runs on CI.

## Review Focus

1. **Someone's tile mounts mid-call while others already talk** (joins, scrolls into view, layout swap). Expect: the
   ring shows at once at the `.5` fallback, the next frame takes over, and the cap ranks the newcomer by when they
   started speaking, not when the tile mounted. → Task 6 test `a_tile_mounted_mid_speech_ranks_by_start_order`.
2. **The webview reloads mid-call with reactive rings on screen.** The old page's "unwatch" never comes. Expect: the
   core stops levels until the new page watches again. → Task 3 test `a_reloaded_page_stops_ring_levels`.
3. **The window is hidden (minimised to tray) during a call.** Expect: no level events while hidden; they resume on
   return. → Task 6 test `a_hidden_document_unwatches_and_returns`.
4. **A pale profile colour on a light theme** (a pastel name colour, Light/Pastel theme). Expect: the ring still reads
   (≥ 3:1 against `--bg-1`), as names do. → Task 5 test `profile_colour_is_made_readable_on_the_theme`.
5. **A watching tile unmounts while its person talks** (they leave, stage reflows). Expect: no leaked element in the
   registry, the watch count drops, and the last unmount sends exactly one `watch_voice_rings(false)`. → Task 6 test
   `unmounting_the_last_watcher_unwatches_once`.

## Phases and parallelism

| Phase | Tasks | Size | Depends on |
|---|---|---|---|
| 1 Server and protocol | 1–2 | S (~300 lines) | – |
| 2 Levels pipeline in the core | 3–4 | S/M (~350 lines) | – |
| 3 The ring in the UI | 5–11 | L (~1300 lines) | 1, 2 |
| 4 Profile settings picker | 12–13 | M (~450 lines) | 3 |

Phases 1 and 2 are independent and can run side by side. Each phase is its own PR to `main`. Phases 3 and 4 should go
out in the same client release (Phase 3 alone shows rings nobody can pick yet; it still ships safely, everyone is
Classic). Inside Phase 3, Tasks 5 and 6 can run in parallel; 7 needs 5 and 6; 8–10 need 7; 11 needs 7.

---

## Phase 1: Server and protocol

### Task 1: Migration, keys, protocol fields and save

Follows `a8567d1a` (typing text) field for field.

**Files:**
- Create: `server/migrations/0085_speaking_ring.sql`
- Modify: `protocol/src/rest.rs` (`User`, `UpdateProfileRequest`), `server/src/profiles/mod.rs` (`RING_STYLES`,
  `RING_COLORS`, `Validated`, `validate`, new test module), `server/src/profiles/routes.rs` (`save`),
  `server/src/profiles/users.rs` (`USER_COLS`, `UserRow`, `into_user`, tests), `server/src/blocks.rs`
  (`emptied_user`), `client/ui/src/lib/protocol/User.ts`, `client/ui/src/lib/protocol/UpdateProfileRequest.ts`
  (generated)
- Modify (compiler-guided, every literal that has `typing_text: None` today): `protocol/src/rest.rs` tests,
  `protocol/src/gateway.rs` tests, `server/src/auth/routes.rs` (`finish_register`), `server/tests/names.rs`,
  `server/tests/private_servers.rs`, `server/tests/profiles.rs` (`request()` and the whole-profile literal),
  `server/tests/blocks.rs`, `client/src-tauri/src/api.rs`, `client/src-tauri/src/commands.rs`,
  `client/src-tauri/src/gateway.rs`
- Test: `server/tests/profiles.rs`, `server/src/profiles/mod.rs`, `server/src/profiles/users.rs`

**Interfaces:**
- Produces (protocol, both on `User` and `UpdateProfileRequest`, each
  `#[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)]`):
  - `ring_style: Option<String>`; `User` doc: "the speaking ring's style (#651); absent: Classic (and from older
    servers; older clients ignore it)". Request doc: "absent keeps the current one (older clients don't send it)".
  - `ring_color: Option<String>`; `User` doc: "absent: green". Request doc as above.
- Produces (server): `pub const RING_STYLES: [&str; 11]`, `pub const RING_COLORS: [&str; 9]` (§6.2, verbatim order);
  `Validated { ring_style: Option<Option<String>>, ring_color: Option<Option<String>> }` (`None` keep, `Some(None)`
  NULL, `Some(Some(k))` store).

- [ ] **Step 1: Write the failing tests**
  - `server/src/profiles/mod.rs`, new `mod ring_tests`: `ring_keys_match_the_ui_list` asserts both arrays equal the
    literal lists above (doc comment: "A literal copy in client/ui/src/lib/speakingRing.test.ts").
  - `server/src/profiles/users.rs`: `ring_fields_are_read_back_and_unknown_or_default_keys_are_none`: row
    `ring_style = Some("bars")` → `Some("bars")`; `Some("sparkle")` → `None`; `Some("classic")` → `None`; same shape
    for `ring_color` (`"purple"` kept, `"red"` → `None`, `"green"` → `None`).
  - `server/tests/profiles.rs`:
    - `speaking_ring_is_saved_kept_and_reset`: as `typing_text_is_saved_kept_and_cleared`. Set `bars`/`purple` → the
      response, `/me`, Ready's `people` and sam's `ProfileUpdated` all carry them; a save with both `None` keeps them;
      a save with `classic`/`green` → `body["user"].get("ring_style").is_none()` and same for `ring_color`.
    - `each_ring_key_is_accepted`: loop every `RING_STYLES` and `RING_COLORS` entry → 200.
    - `an_unknown_ring_key_is_refused`: `ring_style: "sparkle"` → `refused(.., "Unknown speaking ring.")`;
      `ring_color: "red"` → `refused(.., "Unknown ring colour.")`; the stored value is unchanged afterwards.
- [ ] **Step 2: Run them to verify they fail**
  Run: `flock target/.cargo-serial nice -n 10 ionice -c3 cargo test -j 2 -p pulse-server ring`
  Expected: compile error, no field `ring_style` on `User`.
- [ ] **Step 3: Implement**
  - Migration (§6.1 verbatim, header comment `-- #651: the speaking ring others see. NULL is the default (Classic,
    green).`).
  - `validate`: a present key not in its list → `bad("Unknown speaking ring.")` / `bad("Unknown ring colour.")`;
    `"classic"` and `"green"` map to `Some(None)`.
  - `save`: `ring_style = CASE WHEN ? THEN ? ELSE ring_style END, ring_color = CASE WHEN ? THEN ? ELSE ring_color END`
    bound as `v.ring_style.is_some()`, `v.ring_style.flatten()` (as `typing_text`).
  - `users.rs`: `u.ring_style, u.ring_color` in `USER_COLS`; `UserRow` fields; `into_user` keeps a key only if it is
    in its list and not the default.
  - `emptied_user`: both to `None`; extend its doc line ("…, the plain "is typing" (#652) and the plain green ring
    (#651)").
  - Every compiler-flagged literal gets `ring_style: None, ring_color: None`.
- [ ] **Step 4: Run tests and regenerate bindings**
  Run: the Step 2 command, then `cargo test -j 2 -p pulse-server --test profiles --test names`, then `just gen-types`.
  Expected: PASS; `git diff client/ui/src/lib/protocol` shows only the two new optional fields in `User.ts` and
  `UpdateProfileRequest.ts`.
- [ ] **Step 5: Commit**
  ```bash
  git add server/migrations/0085_speaking_ring.sql protocol/src server/src server/tests client/src-tauri/src/api.rs \
    client/src-tauri/src/commands.rs client/src-tauri/src/gateway.rs client/ui/src/lib/protocol
  git commit -m "Profiles: speaking ring style and colour (#651)"
  ```

### Task 2: A blocked viewer never gets your ring

Follows `c15e31ae`.

**Files:**
- Test: `server/tests/blocks.rs`

**Interfaces:**
- Consumes: Task 1's fields; `emptied_user` already resets them.

- [ ] **Step 1: Write the test** `a_blocked_viewer_never_gets_your_speaking_ring`, copied from
  `a_blocked_viewer_never_gets_your_typing_text`: alex saves `ring_style: "orbit"`, `ring_color: "gold"`, blocks sam
  (`"line"`). Sam sees `None` for both in Ready's members and people, the members list, a `ProfileUpdated` after a
  second save, and `GET /users/{id}/profile`; kim sees `Some("orbit")`/`Some("gold")` in each.
- [ ] **Step 2: Run it**
  Run: `flock target/.cargo-serial nice -n 10 ionice -c3 cargo test -j 2 -p pulse-server --test blocks speaking_ring`
  Expected: PASS (Task 1 already empties them). If it fails, the path that leaks is a missing `emptied_user` call: fix
  it there, not in the test.
- [ ] **Step 3: Commit**
  ```bash
  git add server/tests/blocks.rs
  git commit -m "Blocks: a blocked viewer never gets the blocker's speaking ring (#651)"
  ```

---

## Phase 2: Levels pipeline in the core

All in `client/src-tauri/src/voice/`. `mixer.rs` (`MixerInput::take_peer_levels`), `meter.rs` (`Meter::take`) and
`devices.rs` (`Shared::take_sent_level`) keep their code: only their doc comments gain "one caller: the ring tick in
`mod.rs`" (both readers are read-and-reset, §5.2.1). The other callers stay `VoiceManager::mixer_levels()` and the
mixer's own tests.

### Task 3: `ring_level`, `RingLevels` and the watch flag

**Files:**
- Create: `client/src-tauri/src/voice/ringlevels.rs` (named so, not `rings.rs`: `voice/rings.rs` is the audio ring
  buffers)
- Modify: `voice/mod.rs` (`mod ringlevels;`, `VoiceManager::watch_rings`, `reset_page_views`), `voice/controls.rs`
  (`Controls`), `voice/speaking.rs` (`SpeakingTracker::current`), `commands.rs` (`watch_voice_rings`), `lib.rs`
  (register it beside `commands::watch_voice_meters`)
- Test: `voice/ringlevels.rs`, `voice/controls.rs`, `voice/speaking.rs`, `voice/mod.rs` tests

**Interfaces:**
- Produces:
  - `pub fn ring_level(rms: f32) -> f32` (0 for `rms <= 0`).
  - `#[derive(Default)] pub struct RingLevels { shown: HashMap<String, f32>, last_sent: Option<BTreeMap<String, f32>> }`
  - `impl RingLevels { pub fn frame(&mut self, levels: &[(String, f32)], speaking: &[String], watched: bool) -> Option<BTreeMap<String, f32>> }`:
    updates `shown` for every id in `levels` (instant attack, `max(new, shown × 0.7)` release, rounded to 0.05),
    drops ids not in `speaking`, then: unwatched → `last_sent = None`, return `None`; `speaking` empty → `Some({})`
    once if `last_sent` was a non-empty map, else `None`; otherwise the map of speaking ids, `None` if equal to
    `last_sent`.
  - `SpeakingTracker::current(&self) -> &[String]`.
  - `Controls`: `#[serde(skip)] ring_view: bool`; `pub fn watch_rings(&mut self, on: bool)`, `pub fn rings_shown(&self) -> bool`;
    `reset_meters()` also clears `ring_view` (so `reset_page_views` resets it).
  - `VoiceManager::watch_rings(&self, on: bool)`.
  - `#[tauri::command] pub fn watch_voice_rings(voice: State<'_, VoiceManager>, on: bool)` (doc: "a reactive ring
    appeared (`on`) or the last one went: levels are sent only while one is shown (#651)").

- [ ] **Step 1: Write the failing tests**
  - `ringlevels.rs`:
    - `ring_level_maps_the_speaking_threshold_to_zero_and_loud_speech_to_one`: `ring_level(SPEAKING_RMS) == 0.0`;
      `ring_level(10f32.powf(-14.0 / 20.0)) == 1.0`; `ring_level(0.0) == 0.0`; `ring_level(1e-9) == 0.0`.
    - `ring_level_is_monotonic`: over `rms` from 0.001 to 1.0 in ×1.1 steps, never decreases.
    - `levels_rise_at_once_and_fall_by_0_7_per_tick_in_0_05_steps`: id at level 1.0 then silent: frames give 1.0,
      0.7, 0.5 (0.49 rounded), 0.35, …; every emitted value × 20 is an integer.
    - `only_speaking_ids_are_sent`: levels for `a`, `b`; speaking `[a]` → map has only `a`.
    - `nothing_is_sent_in_silence`: speaking empty from the start → `None` for 10 frames.
    - `one_empty_frame_when_the_last_speaker_stops`: a frame with `a`, then speaking empty → `Some({})`, then `None`.
    - `nothing_while_unwatched_and_the_first_watched_frame_is_sent`: unwatched → `None`; watched with the same levels
      → `Some(..)` even though it equals what was sent before unwatching.
    - `an_unchanged_frame_is_skipped`.
  - `controls.rs`: `rings_are_shown_only_while_watched` (on/off flag, not a counter: on, on, off → not shown) and
    `a_reloaded_page_forgets_ring_watching` (`watch_rings(true); reset_meters()` → not shown).
  - `speaking.rs`: `current_is_the_last_reported_set`.
  - `voice/mod.rs`: `a_reloaded_page_stops_ring_levels` beside `a_reloaded_page_stops_link_watching`
    (`vm.watch_rings(true); vm.reset_page_views();` → `!rings_shown()`).
- [ ] **Step 2: Run them to verify they fail**
  Run: `flock target/.cargo-serial nice -n 10 ionice -c3 cargo test -j 2 -p pulse-app voice::ringlevels voice::controls voice::speaking reloaded`
  Expected: compile errors (no `ringlevels`, no `watch_rings`).
- [ ] **Step 3: Implement** the interfaces above. `ring_level` is `((20.0 * rms.log10() + 44.0) / 30.0).clamp(0.0, 1.0)`
  behind the `rms <= 0` guard. Quantise as `(v * 20.0).round() / 20.0`.
- [ ] **Step 4: Run tests to verify they pass** (Step 2 command). Expected: PASS, and the existing `speaking.rs`
  tests unchanged.
- [ ] **Step 5: Commit**
  ```bash
  git add client/src-tauri/src/voice/ringlevels.rs client/src-tauri/src/voice/mod.rs client/src-tauri/src/voice/controls.rs \
    client/src-tauri/src/voice/speaking.rs client/src-tauri/src/commands.rs client/src-tauri/src/lib.rs
  git commit -m "Voice: ring levels and the watch_voice_rings gate (#651)"
  ```

### Task 4: The 70 ms ring task and `VoiceEvent::SpeakingLevels`

**Files:**
- Modify: `client/src-tauri/src/voice/mod.rs` (`VoiceEvent`, the join path's "levels for the UI meter + speaking ring"
  task near `tasks.push(tokio::spawn(` after `shared.mic_silence.lock().unwrap().restart()`), doc comments in
  `voice/mixer.rs` (`take_peer_levels`), `voice/devices.rs` (`take_sent_level`)
- Test: `voice/mod.rs` tests

**Interfaces:**
- Consumes: Task 3's `RingLevels::frame`, `SpeakingTracker::current`, `Controls::rings_shown`.
- Produces: `VoiceEvent::SpeakingLevels { levels: BTreeMap<String, f32> }` (doc: "per-speaker loudness 0..1 for the
  reactive rings (#651): only for people speaking, only while a ring watches; one `{}` when the last one stops").

- [ ] **Step 1: Write the failing test** `speaking_levels_event_serialises_for_the_ui` beside
  `muted_talk_event_serialises_for_the_ui`: `{ "U1": 0.45, "U2": 1.0 }` → exactly
  `{"kind":"speaking_levels","levels":{"U1":0.45,"U2":1.0}}`.
- [ ] **Step 2: Run it** (`cargo test -j 2 -p pulse-app speaking_levels`). Expected: FAIL, no variant.
- [ ] **Step 3: Implement**
  - Add the variant.
  - Remove the three ring lines (`take_peer_levels`, `levels.push((me_id…, take_sent_level()))`,
    `speaking.update(..)` and its `Speaking` emit) and the `SpeakingTracker` from the 100 ms task. Everything else
    stays on it.
  - Spawn a second task in the same block, pushed to the same `tasks`, with its own clones of `shared`, `events`,
    `alive` and `me_id`: `tokio::time::interval(Duration::from_millis(70))`, `MissedTickBehavior::Skip`, break when
    `!alive`. Each tick: take the levels as before, `speaking.update` → emit `Speaking` on change, then
    `ring.frame(&levels, speaking.current(), devices::relock(&shared.controls).rings_shown())` → emit
    `SpeakingLevels` when `Some`. Comment it "speaking ring + its levels, ~14 Hz (#651); the only reader of the
    read-and-reset voice meters".
- [ ] **Step 4: Run the voice tests that cover the session tasks**
  Run: `flock target/.cargo-serial nice -n 10 ionice -c3 cargo test -j 2 -p pulse-app voice::`
  Expected: PASS, including the existing mixer and speaking tests (no test reads `take_peer_levels` concurrently with
  a session except via `mixer_levels`, as before).
- [ ] **Step 5: Commit**
  ```bash
  git add client/src-tauri/src/voice/mod.rs client/src-tauri/src/voice/mixer.rs client/src-tauri/src/voice/devices.rs
  git commit -m "Voice: speaking ring on its own 70 ms tick, with per-speaker levels (#651)"
  ```

---

## Phase 3: The ring in the UI

### Task 5: Palette tokens, keys and resolving a ring

**Files:**
- Modify: `client/ui/src/app.css` (every theme block: `dark`/`:root`, `midnight`, `oled`, `sunset`, `ember`, `plum`,
  `pastel`, `light`; **not** the `light` tint blocks, which `themeCss.test.ts` limits to surface tokens)
- Create: `client/ui/src/lib/speakingRing.ts`, `client/ui/src/lib/speakingRing.test.ts`
- Test: `client/ui/src/themeCss.test.ts`

**Interfaces:**
- Produces (`lib/speakingRing.ts`):
  - `export type RingStyle = 'classic' | 'glow' | 'pulse' | 'ripple' | 'sweep' | 'orbit' | 'halo' | 'viewfinder' | 'liquid' | 'dots' | 'bars'`
  - `export type RingColor = 'green' | 'blue' | 'gold' | 'pink' | 'cyan' | 'orange' | 'white' | 'purple' | 'profile'`
  - `export const RING_STYLES: { key: RingStyle; label: string; reactive: boolean }[]` (labels: Classic, Soft glow,
    Pulse, Ripple, Sweep, Orbit, Halo, Viewfinder, Liquid, Wave of dots, Bars round the ring)
  - `export const RING_COLORS: { key: RingColor; label: string }[]` (Green, Pulse blue, Gold, Pink, Cyan, Orange,
    White, Purple, Profile colour)
  - `export type RingPlace = 'stage' | 'strip' | 'band' | 'list' | 'preview'`
  - `export function ringStyleOf(u: User | null): RingStyle` (unknown/absent → `'classic'`)
  - `export function ringColorOf(u: User | null): RingColor` (unknown/absent → `'green'`)
  - `export function speakCss(u: User | null, color: RingColor, bg: string): string` → `var(--speak-<key>)`, or for
    `profile` `readable(u.style.colors[0], [bg], 3)` when it parses, else `var(--speak-green)`
  - `export type RingState = 'off' | 'on' | 'still'`
  - `export function effectiveStyle(style: RingStyle, o: { classicForAll: boolean }): RingStyle`
  - `export function ringState(o: { lit: boolean; place: RingPlace; style: RingStyle; reducedMotion: boolean; streaming: boolean; animating: boolean }): RingState`
    (`animating` = within the cap; `list` is always `still` when lit)
  - `export function watchesLevels(style: RingStyle, o: { reducedMotion: boolean; streaming: boolean; place: RingPlace }): boolean`
    (reactive effective style, not reduced motion, not streaming, place not `list`/`preview`)

- [ ] **Step 1: Write the failing tests**
  - `speakingRing.test.ts`:
    - `the_eleven_styles_and_nine_colours_keyed_as_the_server_keys_them` (doc: "A literal copy of RING_STYLES /
      RING_COLORS in server/src/profiles/mod.rs: change one side and both tests fail").
    - `unknown_absent_or_emptied_users_are_classic_green`: `ringStyleOf(null)`, a user with `ring_style: 'sparkle'`,
      a user with no fields (what a blocked viewer gets, §6.4) → `'classic'`/`'green'`.
    - `profile_colour_is_the_first_name_colour_or_green`: colours `['#ffc4a3', '#ff7fae']` on `#1b1c21` → `#ffc4a3`
      (already readable); `[]` → `var(--speak-green)`.
    - `profile_colour_is_made_readable_on_the_theme` (Review Focus 4): `['#ffe58f']` on `#f4f4f6` → result has
      `contrast(result, '#f4f4f6') >= 3`.
    - `ring_state_table`: `lit:false` → `off`; lit + `classic` + all clear → `on`; reduced motion → `still`;
      streaming → `still`; `animating:false` → `still`; `place:'list'` → `still`.
    - `classic_for_all_forces_the_style_but_keeps_the_colour` (`effectiveStyle('bars', {classicForAll:true}) === 'classic'`).
    - `only_visible_reactive_effective_styles_watch_levels`.
  - `themeCss.test.ts`, new `describe('speaking ring colours (#651)')`:
    - `every_theme_declares_every_speak_token`: `--speak-green … --speak-purple` in every block (the existing
      "same colour tokens" test then also passes).
    - `every_speak_colour_reaches_3_to_1_on_its_themes_bg_1`: for every `VARIANTS` entry (Light with each tint),
      `contrast(tokens(theme, tint)('--speak-<k>'), tokens(theme, tint)('--bg-1')) >= 3` for all eight keys.
    - `green_is_todays_ring_and_white_is_ink_on_light_themes`: `--speak-green` is `var(--ok)` in every block;
      `--speak-white` is `var(--text)` in `pastel` and `light`.
- [ ] **Step 2: Run them to verify they fail**
  Run: `cd client/ui && pnpm vitest run src/lib/speakingRing.test.ts src/themeCss.test.ts`. Expected: FAIL.
- [ ] **Step 3: Implement**
  - Tokens: dark blocks use the §3.4 hexes (`--speak-blue: #8b9cff`, `--speak-gold: #e8b04a`, `--speak-pink: #f06fb0`,
    `--speak-cyan: #4fc8e8`, `--speak-orange: #f0884f`, `--speak-white: #e8e9ed`, `--speak-purple: #b07cf0`,
    `--speak-green: var(--ok)`). The two light themes (`pastel` and `light`, both light surfaces) get deeper shades of
    each hue, picked so the contrast test passes against `light` with every tint and against `pastel`.
  - `lib/speakingRing.ts`: the interfaces above (`readable` from `./color`).
- [ ] **Step 4: Run tests to verify they pass** (Step 2 command, plus `src/colorTokens.test.ts`). Expected: PASS.
- [ ] **Step 5: Commit**
  ```bash
  git add client/ui/src/app.css client/ui/src/lib/speakingRing.ts client/ui/src/lib/speakingRing.test.ts client/ui/src/themeCss.test.ts
  git commit -m "Rings: palette tokens on every theme, and resolving a person's ring (#651)"
  ```

### Task 6: Watching, applying levels, the cap and the Classic setting

**Files:**
- Create: `client/ui/src/lib/ringLevels.svelte.ts`, `client/ui/src/lib/ringLevels.svelte.test.ts`,
  `client/ui/src/lib/speakingRing.svelte.ts`, `client/ui/src/lib/speakingRing.svelte.test.ts`
- Modify: `client/ui/src/lib/voiceui.ts` (`VoiceEvent` union), `client/ui/src/lib/voice.svelte.ts` (the
  `voice://event` listener), `client/ui/src/lib/animate.svelte.ts` (export `observe`)

**Interfaces:**
- Produces (`lib/speakingRing.svelte.ts`):
  - `export const ringPrefs = $state({ classic: savedClassic() })`; `export function setClassicRings(on: boolean): void`
    (localStorage `pulse.classicRings`, read and written in try/catch like `setAnimateMode`)
  - `export const RING_CAP = 8`
  - `export function nextOrder(prev: string[], lit: Set<string>): string[]` (keep `prev` order minus ids no longer
    lit, append new ids in sorted order)
  - `export function isAnimating(id: string): boolean` (rank in the current order `< RING_CAP`), backed by a
    module-level `$derived.by` over `voice.speaking ∪ soundboard.playing` that remembers its previous order
- Produces (`lib/ringLevels.svelte.ts`):
  - `export function register(userId: string, el: HTMLElement): () => void` (the `{@attach}` target; returns cleanup)
  - `export function setWatching(el: HTMLElement, on: boolean): void` (counted per element; the module invokes
    `watch_voice_rings({ on })` only on 0 → 1 and 1 → 0)
  - `export function apply(levels: Record<string, number>): void` (`el.style.setProperty('--lvl', String(v))` on each
    listed user's elements; `{}` sets `0` on every registered element; then the speed buckets)
  - `export type Bucket = { rate: 0.75 | 1 | 1.5; pending: 0.75 | 1 | 1.5 | null; since: number }`
  - `export function nextBucket(b: Bucket, lvl: number, now: number): Bucket` (< .35 → 0.75, < .7 → 1, else 1.5; a
    new bucket must hold 250 ms before `rate` changes)
  - On a rate change for an element with `data-style="dots"`: `el.getAnimations({ subtree: true }).forEach((a) => a.updatePlaybackRate(rate))`,
    guarded by `typeof el.getAnimations === 'function'`.
- Produces (union in `voiceui.ts`): `| { kind: 'speaking_levels'; levels: Record<string, number> }`.

- [ ] **Step 1: Write the failing tests**
  - `speakingRing.svelte.test.ts`:
    - `the_first_eight_to_start_animate_and_a_newcomer_never_bumps_one`: ids 1–9 start in order → 1–8 animate, 9
      still; 3 stops → 9 animates; 10 starts → 1,2,4–9 still animate, 10 still.
    - `a_tile_mounted_mid_speech_ranks_by_start_order` (Review Focus 1): order is built from the lit set, not from
      registration; mounting a tile for id 9 after 1–9 started still reports `isAnimating('9') === false`.
    - `soundboard_lit_people_count_toward_the_cap` (`lightRing` from `soundboard.svelte`).
    - `the_classic_setting_persists_and_survives_blocked_storage` (throwing `localStorage` → default off, no throw).
  - `ringLevels.svelte.test.ts` (mock `@tauri-apps/api/core` `invoke`):
    - `one_watch_invoke_per_transition`: two elements watch → one `watch_voice_rings {on:true}`; one stops → none;
      the other stops → one `{on:false}`.
    - `unmounting_the_last_watcher_unwatches_once` (Review Focus 5): the cleanup from `register` also drops its
      watch; afterwards `apply({ U1: .5 })` touches no element.
    - `apply_sets_lvl_on_the_right_elements_without_touching_state`: two users, three elements → `--lvl` set on the
      matching ones only; wrap in `$effect.root` with an `$effect` reading `voice.speaking` and `voice.stage` and
      assert it ran once.
    - `an_empty_frame_zeroes_every_element`.
    - `speed_buckets_hold_250ms_before_switching` (`nextBucket` table: .2 then .9 for 200 ms → still 0.75; at 260 ms →
      1.5; a flicker back under 250 ms resets `pending`).
    - `a_hidden_document_unwatches_and_returns` (Review Focus 3): with one watching element, dispatch
      `visibilitychange` with `visibilityState: 'hidden'` → `{on:false}`; `'visible'` → `{on:true}`.
  - `voice.svelte` listener (in `ringLevels.svelte.test.ts`, by emitting through the mocked `listen`):
    `speaking_levels_never_reaches_reduceStage_or_state`: `voice.stage` is the same object before and after.
- [ ] **Step 2: Run them to verify they fail**
  Run: `cd client/ui && pnpm vitest run src/lib/ringLevels.svelte.test.ts src/lib/speakingRing.svelte.test.ts`.
  Expected: FAIL (modules missing).
- [ ] **Step 3: Implement**
  - In the listener, before `voice.stage = reduceStage(voice.stage, e)`:
    `if (e.kind === 'speaking_levels') { apply(e.levels); return }`.
  - `ringLevels` listens to `document`'s `visibilitychange` once; the effective watch is
    `count > 0 && visibilityState === 'visible'`, and the invoke fires only when that changes.
  - Export `observe` from `animate.svelte.ts` unchanged (the ring calls it per element).
- [ ] **Step 4: Run tests to verify they pass** (Step 2 command, plus `src/lib/voice.svelte.test.ts` if present).
  Expected: PASS.
- [ ] **Step 5: Commit**
  ```bash
  git add client/ui/src/lib/ringLevels.svelte.ts client/ui/src/lib/ringLevels.svelte.test.ts client/ui/src/lib/speakingRing.svelte.ts \
    client/ui/src/lib/speakingRing.svelte.test.ts client/ui/src/lib/voiceui.ts client/ui/src/lib/voice.svelte.ts client/ui/src/lib/animate.svelte.ts
  git commit -m "Rings: levels straight onto the tiles, watching only what's visible, and the 8-ring cap (#651)"
  ```

### Task 7: `SpeakingRing.svelte`: all eleven styles

**Files:**
- Create: `client/ui/src/components/SpeakingRing.svelte`, `client/ui/src/components/SpeakingRing.test.ts`,
  `client/ui/src/components/SpeakingRingCss.test.ts`

**Interfaces:**
- Consumes: Task 5 (`ringStyleOf`, `ringColorOf`, `speakCss`, `effectiveStyle`, `ringState`, `watchesLevels`),
  Task 6 (`register`, `setWatching`, `isAnimating`, `ringPrefs`), `observe` and `motion` from `animate.svelte.ts`,
  `userOf`, `nameBg('side')`.
- Produces: `<SpeakingRing userId d speaking place style? color? />` with props
  `{ userId: string; d: number; speaking: boolean; place: RingPlace; style?: RingStyle; color?: RingColor }`
  (`style`/`color` override, preview only). Root: `<span class="ring-box" data-style={eff} data-ring={state}
  style:--d="{d}px" style:--speak={css} aria-hidden="true">`, `position: absolute; inset: 0` (exactly the avatar
  box, so the §4 insets apply verbatim), `pointer-events: none`, **no** `z-index`, `opacity`, `transform` or
  `overflow` on the root (it must not form a stacking context or clip).
  Layers behind the avatar (`.back`: Liquid, Halo) have `z-index: -1`; ring layers (`.front`) `z-index: 1`. Hosts make
  their avatar button a stacking context and give the badge `z-index: 2` (Tasks 8–9).

Everything the §4 CSS lists goes in this one stylesheet, sized in `calc(var(--d) * k)`, insets exactly as §4 writes
them (relative to the avatar box). Wave of dots' dots use `background: var(--speak)` (the spec omits it).

- [ ] **Step 1: Write the failing CSS guard test** `SpeakingRingCss.test.ts` (compile with `svelte/compiler` as
  `FullProfileCss.test.ts` does):
  - `every_keyframes_body_sets_only_transform_and_opacity`: for every `@keyframes` block, every declaration's
    property is in `{transform, opacity}`.
  - `every_transition_names_only_opacity_or_scale`: every `transition:` value's property names ⊂ `{opacity, scale}`.
  - `no_filter_box_shadow_animation_or_will_change`: no `filter`, no `backdrop-filter`, no `will-change`; `box-shadow`
    appears only in the Classic rule and never next to a `transition`.
  - `lvl_is_read_only_inside_scale`: every `var(--lvl` sits in a `scale:` declaration; `--lvl` is never declared,
    transitioned or animated.
  - `animations_run_only_when_on`: every `animation:` declaration other than `none` is in a rule whose selector
    contains `[data-ring="on"]` (or `='on'`).
  - `every_style_stays_inside_the_ring_box` (no-spill guard): a table `{ style, layerSelector, keyframes, lvlPeak }`
    for the nine non-trivial styles; parse the layer's negative `inset` multiplier `k` (as `calc(var(--d) * -k)`), the
    largest `scale(...)` in its keyframes and its `scale:` at `--lvl: 1`; assert `(0.5 + k) × maxScale ≤ 0.66 + 1e-3`
    (the layer's half-size in D, times its peak, inside the 0.66 D half of the ring box). Bars: the layer is at
    `-.16`, the bar's outer end at `top: 0`, and the product of keyframe and `scale:` peaks in Y is `≤ 1`.
- [ ] **Step 2: Write the failing DOM test** `SpeakingRing.test.ts` (jsdom, `mount`):
  - `off_on_still_follow_speaking_and_motion`: `speaking:false` → `data-ring="off"`; true → `"on"`;
    `motion.reducedMotion = true` → `"still"`; `motion.streaming = true` → `"still"`.
  - `starting_to_speak_creates_no_dom`: a `MutationObserver({ childList: true, subtree: true })` on the root sees no
    records when `speaking` flips true and false; only attributes change.
  - `layer_counts_per_style`: `dots` at `d: 96` → 24 dots, at `d: 34` → 12; `bars` at 96 → 44, at 34 → 22; `orbit` → 5;
    `ripple` → 2 layers; `liquid` → 3 blobs.
  - `bar_delays_and_still_profile_are_computed_at_render`: bar `i` has `animation-delay` `-((sin(i·.9)+1)·.25)s` and a
    still `transform: scaleY(.35 + .4·(sin(i·.9)+1)/2)` (within 1e-3).
  - `going_off_clears_lvl`: set `--lvl: .9` on the root, flip `speaking` false → `--lvl` removed (so a later
    soundboard-lit ring runs at the `.5` fallback, §11.7).
  - `classic_for_all_renders_classic_in_the_persons_colour`: `ringPrefs.classic = true`, user `bars`/`purple` →
    `data-style="classic"`, `--speak: var(--speak-purple)`.
  - `only_reactive_visible_rings_watch` (spy on `setWatching`): `bars` → true; `pulse` → false; `bars` with reduced
    motion → false; `place: 'preview'` → false.
- [ ] **Step 3: Run both to verify they fail**
  Run: `cd client/ui && pnpm vitest run src/components/SpeakingRing.test.ts src/components/SpeakingRingCss.test.ts`.
  Expected: FAIL (no component).
- [ ] **Step 4: Implement the component**
  - State: `lit = speaking`, `eff = effectiveStyle(style ?? ringStyleOf(user), { classicForAll: ringPrefs.classic })`
    (the preview ignores `ringPrefs`), `state = ringState({ lit, place, style: eff, reducedMotion: motion.reducedMotion,
    streaming: motion.streaming, animating: place === 'preview' || isAnimating(userId) })`.
  - `{@attach}` on the root: `register(userId, el)` (not for `preview`), `observe(el, (v) => setWatching(el, v && watchesLevels(...)))`,
    returning both cleanups; an `$effect` re-runs `setWatching` when `watchesLevels` changes.
  - Classic: `box-shadow: 0 0 0 Apx var(--ring), 0 0 0 Bpx var(--speak)` when lit, A/B 4/7 for `stage`/`preview`, 2/4
    otherwise; no transition.
  - The other ten styles exactly as §4.2–4.11 (keyframe names `ring-pulse`, `ring-ripple`, `ring-spin`, `ring-halo`,
    `ring-snap`, `ring-b1`–`ring-b3`, `ring-dot`, `ring-bar`), each with its still version under `[data-ring="still"]`.
  - Below `d < 48` render 12 dots / 22 bars.
- [ ] **Step 5: Run tests to verify they pass** (Step 3 command). Expected: PASS.
- [ ] **Step 6: Commit**
  ```bash
  git add client/ui/src/components/SpeakingRing.svelte client/ui/src/components/SpeakingRing.test.ts client/ui/src/components/SpeakingRingCss.test.ts
  git commit -m "Rings: one SpeakingRing component draws all eleven styles, compositor-only (#651)"
  ```

### Task 8: The voice stage and the strip (`UserTile`, `tileScale`)

Voice channels, DM calls and group calls all render their stage through `VoicePanel.svelte` → `UserTile` (stage), and
the full view's strip through `FullView.svelte` → `UserTile compact`.

**Files:**
- Modify: `client/ui/src/lib/stagelayout.ts` (`tileScale`), `client/ui/src/components/UserTile.svelte`
- Test: `client/ui/src/lib/stagelayout.test.ts`, `client/ui/src/components/UserTile.ring.test.ts`,
  `client/ui/src/components/UserTile.scale.test.ts`, `client/ui/src/components/VoicePanel.test.ts` (existing avatar
  width check must still pass)

**Interfaces:**
- Consumes: Task 7's `SpeakingRing`.
- Produces: `tileScale(h)` keeps its return type `{ avatar, font, badge, icon }`; `avatar = Math.min(clamp(round(h × .5),
  48, 160), Math.floor((h − 8 − 10 − nameRow(font)) / 1.32))` with `nameRow(font) = Math.ceil(font × 1.4)` (tune the
  factor against the stage tests; `StreamTile`'s placeholder avatar shrinks with it, which is fine).

- [ ] **Step 1: Write the failing tests**
  - `stagelayout.test.ts`: `the_ring_box_and_name_fit_every_tile_height`: for `h` from 108 (a `TILE_MIN_W` tile) to
    810 step 1, `1.32 × avatar + 10 + Math.ceil(font × 1.4) <= h − 8`; and `avatar` never grows above the old
    `clamp(round(h × .5), 48, 160)`.
  - `UserTile.ring.test.ts` (keep the soundboard test, it now asserts on `.ring-box[data-ring]` as well as
    `.tile.speaking`):
    - `the_tile_draws_its_ring_through_speaking_ring`: a `.ring-box` inside `.avatar-btn`, `--d` equal to the avatar
      size; no `box-shadow` on `.av`.
    - `the_ring_box_is_reserved_whatever_the_style` (no jump): the compiled CSS for `.avatar-btn` has
      `margin: calc(var(--d) * .16)` and `.compact .avatar-btn` `margin: max(6px, calc(var(--d) * .16))`; flipping
      `voice.speaking` changes no `style` attribute on `.avatar-btn` or `.tile`.
    - `the_tile_clips_on_purpose`: compiled `.tile` has `overflow: hidden`.
    - `the_border_lights_in_the_ring_colour`: compiled `.tile.speaking` has `border-color: var(--speak)`, and `.tile`
      carries `--speak` from `speakCss`.
    - `the_badge_stays_above_the_ring`: compiled `.badge` has `z-index: 2`, `.avatar-btn` has `isolation: isolate`.
  - `UserTile.scale.test.ts`: the 84px fixed-size case is unchanged.
- [ ] **Step 2: Run them to verify they fail**
  Run: `cd client/ui && pnpm vitest run src/lib/stagelayout.test.ts src/components/UserTile.ring.test.ts src/components/UserTile.scale.test.ts src/components/VoicePanel.test.ts`.
- [ ] **Step 3: Implement**
  - `UserTile`: `style:--d` on `.avatar-btn` (the same px as `Avatar`'s size); `<SpeakingRing userId={entry.userId}
    d={size} speaking={lit} place={compact ? 'strip' : 'stage'} />` after `<Avatar>`, before the badges; `style:--speak`
    on `.tile`.
  - Remove `.tile.speaking :global(.av)` and `.compact.speaking :global(.av)`; `.tile.speaking { border-color:
    var(--speak) }`; `.tile { overflow: hidden }` (the game backdrop already uses `border-radius: inherit`).
- [ ] **Step 4: Run tests to verify they pass** (Step 2 command). Expected: PASS.
- [ ] **Step 5: Commit**
  ```bash
  git add client/ui/src/lib/stagelayout.ts client/ui/src/lib/stagelayout.test.ts client/ui/src/components/UserTile.svelte \
    client/ui/src/components/UserTile.ring.test.ts client/ui/src/components/UserTile.scale.test.ts
  git commit -m "Rings: the stage and strip tiles reserve the ring box and draw SpeakingRing (#651)"
  ```

### Task 9: The call band and the channel list

**Files:**
- Modify: `client/ui/src/components/CallBand.svelte`, `client/ui/src/components/ChannelList.svelte`
- Test: `client/ui/src/components/CallBand.test.ts`, `client/ui/src/components/ChannelList.test.ts`

**Interfaces:**
- Consumes: Task 7's `SpeakingRing`, Task 5's `speakCss`, `ringColorOf`.

- [ ] **Step 1: Write the failing tests**
  - `CallBand.test.ts`:
    - `band_avatars_draw_speaking_ring_at_34px`: each `.who` holds a `.ring-box` with `--d: 34px`,
      `place` band (Classic 2/4 shadow when lit).
    - `the_band_leaves_room_for_the_ring_box` (no spill): compiled `.people` has `gap: 12px` (≥ 2 × .16 × 34 = 10.9);
      `.who` has `isolation: isolate` and `.flag` `z-index: 2`.
  - `ChannelList.test.ts`: `occupant_ring_is_static_classic_in_the_persons_colour`: a speaking occupant's `.ring`
    has `--speak` from `speakCss` (`var(--speak-gold)` for a `gold` user); compiled `.ring.speaking` uses
    `var(--speak)`, has no `transition` and no `animation`.
- [ ] **Step 2: Run them to verify they fail**
  Run: `cd client/ui && pnpm vitest run src/components/CallBand.test.ts src/components/ChannelList.test.ts`.
- [ ] **Step 3: Implement**: CallBand: `<SpeakingRing ... d={34} place="band" />` in each roster `.who` (not in the
  ringing caller's placeholder), drop `.who.speaking :global(.av)`, `.people { gap: 12px }`. ChannelList: keep its own
  `.ring.speaking` rule with `var(--ok)` → `var(--speak)`, and `style:--speak={speakCss(user, ringColorOf(user), nameBg('side'))}`
  on the `.ring` span.
- [ ] **Step 4: Run tests to verify they pass** (Step 2 command). Expected: PASS.
- [ ] **Step 5: Commit**
  ```bash
  git add client/ui/src/components/CallBand.svelte client/ui/src/components/CallBand.test.ts client/ui/src/components/ChannelList.svelte client/ui/src/components/ChannelList.test.ts
  git commit -m "Rings: the call band and the channel list's occupants (#651)"
  ```

### Task 10: "Show everyone's rings as Classic"

**Files:**
- Modify: `client/ui/src/components/VoiceSettings.svelte`
- Test: `client/ui/src/components/VoiceSettings.test.ts`

**Interfaces:**
- Consumes: Task 6's `ringPrefs`, `setClassicRings`.

- [ ] **Step 1: Write the failing test** `the_classic_rings_toggle_persists_and_turns_rings_classic`: a row after the
  audio toggles, `button.switch[aria-label="Show everyone's rings as Classic"]`, `aria-pressed="false"` by default;
  a click → `ringPrefs.classic === true`, `localStorage['pulse.classicRings'] === 'true'`, and `voiceApi.setAudioConfig` is not
  called for it (it never reaches the core).
- [ ] **Step 2: Run it** (`pnpm vitest run src/components/VoiceSettings.test.ts`). Expected: FAIL.
- [ ] **Step 3: Implement** one more `.row` in the existing `.toggles` block, after the `{#each toggles}` loop, with
  the §7.2 label and help, bound to `ringPrefs.classic` / `setClassicRings`.
- [ ] **Step 4: Run it to verify it passes.**
- [ ] **Step 5: Commit**
  ```bash
  git add client/ui/src/components/VoiceSettings.svelte client/ui/src/components/VoiceSettings.test.ts
  git commit -m "Rings: Voice settings can show everyone's ring as Classic (#651)"
  ```

### Task 11: The dev stress harness and the acceptance figure

**Files:**
- Create: `client/ui/src/lib/ringStress.ts`
- Modify: `client/ui/src/main.ts` (one `if (import.meta.env.DEV) void import('./lib/ringStress').then((m) => m.install())`)
- Test: `client/ui/src/lib/ringStress.test.ts`

**Interfaces:**
- Produces: `export function install(): void` defines `window.__ringStress(n: number, style: RingStyle): () => void`
  that adds `n` fake speakers (`voice.speaking`, fake `RosterEntry`s on the current stage, users with `ring_style:
  style`) and feeds `apply()` a synthetic random walk every 70 ms; the returned function stops and removes them.

- [ ] **Step 1: Write the failing test** `the_harness_is_dev_only_and_cleans_up`: `install()` then `__ringStress(8,
  'bars')` → `voice.speaking.size === 8`, `apply` called within 100 ms (fake timers); calling the stop function →
  `voice.speaking.size === 0` and no timer left (`vi.getTimerCount() === 0`). A second check greps `main.ts` for the
  `import.meta.env.DEV` guard around the import.
- [ ] **Step 2: Run it to verify it fails**, implement, run it to verify it passes.
- [ ] **Step 3: Measure (manual, on the owner's Windows PC)**: a dev build, `__ringStress(8, 'bars')` vs
  `__ringStress(8, 'classic')`, 10 s performance trace and Paint flashing (§8.2). Record the CPU delta and "no
  frame-rate Paint on the tiles" in the Phase 3 PR description. If the PR can't be measured before merge, open an
  issue for it first (repo rule).
- [ ] **Step 4: Commit**
  ```bash
  git add client/ui/src/lib/ringStress.ts client/ui/src/lib/ringStress.test.ts client/ui/src/main.ts
  git commit -m "Rings: a dev-only stress harness for the performance check (#651)"
  ```

---

## Phase 4: Profile settings picker

### Task 12: The draft carries the ring

**Files:**
- Modify: `client/ui/src/lib/profileDraft.ts` (`Draft`, `draftFrom`, `toRequest`, `previewUser`)
- Test: `client/ui/src/lib/profileDraft.test.ts`

**Interfaces:**
- Consumes: Task 5's `ringStyleOf`, `ringColorOf`, Phase 1's generated `User`/`UpdateProfileRequest`.
- Produces: `Draft.ringStyle: RingStyle`, `Draft.ringColor: RingColor`; `toRequest` always sends `ring_style` and
  `ring_color`; `previewUser` sets them on the returned `User` (omitting each when it's the default, as the server
  does).

- [ ] **Step 1: Write the failing tests**
  - `ring_fields_round_trip`: a profile with `ring_style: 'dots'`, `ring_color: 'cyan'` → draft `dots`/`cyan` →
    `toRequest` sends both; a profile without them → `classic`/`green`, and `toRequest` still sends both.
  - `an_unknown_ring_key_drafts_as_the_default` (`'sparkle'` → `classic`).
  - `changing_the_ring_makes_the_draft_dirty` (`isDirty`, `editsSince` pick it up with no other change).
  - `preview_user_carries_the_draft_ring_and_drops_defaults`.
- [ ] **Step 2: Run them to verify they fail** (`pnpm vitest run src/lib/profileDraft.test.ts`).
- [ ] **Step 3: Implement** the interfaces above.
- [ ] **Step 4: Run them to verify they pass.**
- [ ] **Step 5: Commit**
  ```bash
  git add client/ui/src/lib/profileDraft.ts client/ui/src/lib/profileDraft.test.ts
  git commit -m "Profiles: the draft carries the speaking ring (#651)"
  ```

### Task 13: The "Speaking ring" tab

**Files:**
- Modify: `client/ui/src/components/ProfileSettings.svelte` (`Tab`, `TABS`, a new panel after `name`),
  `client/ui/src/lib/speakingRing.ts` (`envelopeStep`)
- Test: `client/ui/src/components/ProfileSettings.test.ts`, `client/ui/src/lib/speakingRing.test.ts`,
  new `client/ui/src/components/ProfileSettingsCss.test.ts`

**Interfaces:**
- Consumes: Task 7's `SpeakingRing` (`place="preview"`, `style`/`color` overrides), Task 12's draft fields,
  `radioKeys` from `lib/actions`, `motion` from `animate.svelte.ts`.
- Produces: `export type EnvelopeState = { lvl: number; pause: number }`;
  `export function envelopeStep(s: EnvelopeState, r: number): EnvelopeState` (`r` in [0, 1); a cheap random walk
  that sometimes enters a pause of several ticks at 0).

- [ ] **Step 1: Write the failing tests**
  - `speakingRing.test.ts`: `the_preview_envelope_stays_in_range_and_pauses`: 300 steps with a seeded PRNG → every
    `lvl` in [0, 1], at least one run of ≥ 3 consecutive zeros, and at least one value ≥ .7.
  - `ProfileSettings.test.ts`:
    - `the_speaking_ring_tab_follows_name_style` (tab order About you, Card, Name style, Speaking ring, Badges; tab
      keys still move through them).
    - `style_and_colour_are_radiogroups_with_roving_tabindex`: `[role=radiogroup][aria-label="Ring style"]` holds 11
      radios in two labelled groups ("Still or looping" 8, "Follows your voice" 3), exactly one `tabindex=0`;
      ArrowRight from Viewfinder moves to Liquid and sets the draft; `[aria-label="Ring colour"]` holds 9 swatches with
      `aria-label`s from `RING_COLORS`.
    - `the_profile_swatch_tip_says_where_the_colour_comes_from`: with a name colour → tip "Your name colour, from Name
      style"; without → tip ends "You haven't set one, so it's green."; its swatch background is the `speakCss`
      result.
    - `picking_a_ring_saves_it`: pick `bars` + `purple`, Save → `invoke('update_profile', { request })` (via `api.updateProfile`) has
      `ring_style: 'bars'`, `ring_color: 'purple'`.
    - `the_preview_animates_only_while_the_tab_is_shown`: fake timers; on the ring tab with `bars`, the preview's
      wrapper gets `--lvl` within 70 ms; switching to another tab clears the interval (`vi.getTimerCount()` back to
      its baseline); with `motion.reducedMotion` the caption is "Preview · still (reduced motion is on)" and no
      interval runs.
  - `ProfileSettingsCss.test.ts` (Greg's UI rule; compile as `FullProfileCss.test.ts`):
    - `the_style_grid_never_spills`: `.ring-grid` is `grid-template-columns: repeat(3, minmax(0, 1fr))`; `.ring-chip`
      has `min-width: 0`, `overflow: hidden`, `text-overflow: ellipsis`, `white-space: nowrap`.
    - `the_preview_never_shrinks_and_the_row_stacks_instead`: `.ring-row` has `flex-wrap: wrap`; `.ring-preview` has
      `flex: 0 0 200px`, `width: 200px`, `height: 200px`; `.ring-swatches` has `flex-wrap: wrap`; the 96px avatar's
      ring box (127px) fits the 200px panel (assert the avatar size constant ≤ 200 / 1.32).
- [ ] **Step 2: Run them to verify they fail**
  Run: `cd client/ui && pnpm vitest run src/lib/speakingRing.test.ts src/components/ProfileSettings.test.ts src/components/ProfileSettingsCss.test.ts`.
- [ ] **Step 3: Implement** the tab per §7.1: heading and subtitle; `.ring-row` (preview panel `--bg-1`, 1px
  `--bg-3` border, `--radius-xl`, avatar 96px, `<SpeakingRing place="preview" speaking d={96} style={draft.ringStyle}
  color={draft.ringColor} userId={me.id} />` inside a wrapper whose `--lvl` the envelope interval (70 ms, only while
  `tab === 'ring'` and not reduced motion) writes with `style.setProperty`); the two groups inside one radiogroup
  (`onkeydown={radioKeys}`); the colour radiogroup of 28px round swatches drawn with `speakCss` (so the light theme's
  White shows as ink); the muted note.
- [ ] **Step 4: Run them to verify they pass** (Step 2 command).
- [ ] **Step 5: Commit**
  ```bash
  git add client/ui/src/components/ProfileSettings.svelte client/ui/src/components/ProfileSettings.test.ts \
    client/ui/src/components/ProfileSettingsCss.test.ts client/ui/src/lib/speakingRing.ts client/ui/src/lib/speakingRing.test.ts
  git commit -m "Profiles: pick your speaking ring, with a live preview (#651)

  Closes #651"
  ```
