# Stream sound you can count on: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A three-way sound choice when going live, a tiered way of finding the shared window's app, and stream controls that always say what is being heard.

**Architecture:** All tier logic is pure Rust in `stream/plan.rs` (fake processes, sessions and window rows in, a capture plan and a tier out). `stream/procs.rs` gathers the real inputs on Windows (window by handle, process table with image paths, audio sessions). `stream/audio.rs` keeps the sharer's choice in shared state that the 2 s rescan reads for every share, and reports a status the UI polls. The UI turns the "Stream audio" checkbox into a "Sound" radio fieldset like Resolution and Frame rate, probes the picked window for the greyed-out state and the live hint, and shows the source and a "no sound yet" note in the live card.

**Tech Stack:** Rust (Tauri 2, windows 0.62, wasapi), Svelte 5, vitest.

**Spec:** `docs/superpowers/specs/2026-11-06-stream-audio-design.md`

## Global Constraints

- Choices, in order: **This app · Everything except Pulse · No sound**; a screen share offers only the last two.
- Default: This app when identified, otherwise Everything except Pulse. This app is greyed out only when the app can't be identified, tooltip "Can't tell which app owns this window"; never because it is silent.
- Hint beside This app: "Playing sound" / "Not playing sound right now".
- Controls: "Sound: <App>", "Sound: everything except Pulse", "No sound"; last-resort tier says so.
- "No sound from this app yet" after 20 s with nothing audible, with a one-click switch to Everything except Pulse; never an auto-switch.
- Window shares rescan like screen shares (every 2 s).
- Privacy: never capture other apps unless chosen, or the last-resort tier applies (and the controls say so).
- Look and feel: reuse `fieldset`/`legend`/radio markup of GoLiveDialog and the `.meta`/`.btn btn-secondary ghost` of LiveCard; colours only from tokens.
- Linux keeps today's behaviour (probe says unidentified; capture is a no-op there).

## Review Focus

- A parent that is the shell (explorer.exe) has a session: Include(parent) is tree-mode and would capture every app. Test: `related_never_takes_the_shell_as_a_launcher`.
- Two windows with the same title from two different apps: must stay unidentified. Test: `title_owner_is_none_for_two_different_apps`.
- Two windows with the same title from one app (two Chrome windows): identified as that app. Test: `title_owner_takes_one_app_with_two_processes`.
- A UWP window is owned by ApplicationFrameHost.exe, not the app. Test: `frame_host_window_resolves_to_its_app`.
- A music app's helper process (child of Spotify.exe) in the last-resort tier. Test: `last_resort_skips_music_app_helpers`.

---

### Task 1: Pure tier logic (plan.rs)

**Files:** Modify `client/src-tauri/src/stream/plan.rs` (+ tests)

**Interfaces (produces):**
- `pub enum Sound { App, Everything }` (serde lowercase; "No sound" never reaches Rust: nothing is published)
- `pub struct Proc { pub pid: u32, pub parent: u32, pub exe: String }` (`exe`: image path when known, else name)
- `pub enum Source { Screen, Window(Vec<u32>) }` (owner pids; empty = not identified)
- `pub enum Tier { App, Related, Everything, LastResort }` (serde camelCase)
- `PlanInput { own_pid, pulse_pids, pulse_roots, session_pids, procs, source, sound }`
- `pub fn capture_plan(&PlanInput) -> (Vec<Capture>, Tier)`
- `pub fn playing(&PlanInput) -> bool` (owners' tree or related family has a session)
- `pub const SKIP_ALWAYS_ON: &[&str]` (Spotify, Apple Music/iTunes, foobar2000, MusicBee, AIMP, Winamp, TIDAL, Deezer, Amazon Music, Discord, DiscordPTB, DiscordCanary)
- `pub fn title_owner(rows: &[WindowRow], title, pulse) -> Vec<u32>`; `pub fn app_name(exe) -> String`; `pub fn frame_host_app(owner_exe, child_pids) -> Option<u32>`

Rules: Everything/Screen → today's rule. App + owners → Include(owners); if the owners' trees have no session, also Include each session process in the family (same exe as an owner and their trees, or an owner's parent unless it is a shell/system host or Pulse) → Related. App + no owners → per-session Include skipping Pulse and skip-list processes (or their descendants) → LastResort.

- [ ] Tests first (names above plus `app_tier_includes_the_owner`, `related_adds_same_exe_session`, `related_adds_launcher_parent`, `everything_ignores_owner`, `skip_list_is_named_and_case_insensitive`, `app_name_from_path`); run, see fail; implement; run, see pass; commit.

### Task 2: Windows gathering (procs.rs)

**Files:** Modify `client/src-tauri/src/stream/procs.rs`

- `pub fn processes() -> Vec<Proc>` (Toolhelp + QueryFullProcessImageNameW).
- `pub fn resolve_window(hwnd: Option<u64>, title: Option<&str>, pulse, procs) -> (Vec<u32>, How)` where `How = Handle | Title | None`: handle first (IsWindow, GetWindowThreadProcessId, ApplicationFrameHost child), else `title_owner`.
- Off Windows everything returns empty / `None`. Replace `window_pid`.
- [ ] Update the off-Windows test; xwin clippy in Task 6; commit.

### Task 3: Shared choice, rescans, status, switch (audio.rs, mod.rs, lib.rs)

- `pub struct WindowRef { hwnd: Option<u64>, title: String }` (serde camelCase).
- `start_publishing(window: Option<WindowRef>, sound: Sound, muted)`; window shares now always publish and always rescan.
- `set_sound(Sound)`: re-plans now, no republish; resets "heard".
- `status() -> Option<SoundStatus { tier, app: Option<String>, heard: bool, ms: u64 }>`; Outlet counts audible chunks.
- Commands: `stream_audio_start(channelId, window, sound, muted)`, `stream_audio_probe(window) -> Probe { identified, app, playing }`, `stream_audio_sound(sound)`, `stream_audio_status()`.
- [ ] Test `a_switch_resets_heard` on the status bookkeeping; client tests + clippy; commit.

### Task 4: Picker sound choice (UI)

**Files:** `client/ui/src/lib/stream/room.ts` (`Picked.hwnd` from `getSettings().deviceId` `window:<hwnd>:<n>`; pure `windowHandle(deviceId)`), `prefs.ts` (`GoLive.sound: 'app' | 'everything'`, clamp), new `client/ui/src/lib/stream/sound.ts` (`soundChoices(surface)`, `defaultSound(stored, surface, identified)`, `soundLabel(status, audio)`, `showNoSoundYet(status)`), `tauri.ts`, `GoLiveDialog.svelte` (Sound fieldset, probe poll every 1.5 s for a window share).
- [ ] Tests: `windowHandle` parsing; defaults and greying; dialog renders three choices for a window and two for a screen, greys This app with the tooltip when the probe says unidentified, shows the hint; Go Live carries `sound`. Commit.

### Task 5: Live card source and note (UI)

**Files:** `stream.svelte.ts` (`audioStart(channelId, window, sound, muted)`, status poll every 2 s while live with audio into `stream.sound`, `streamApi.switchSound('everything')`), `LiveCard.svelte` (meta `· Sound: …`, the note with "Use everything except Pulse").
- [ ] Tests: label per tier; note after 20 s unheard, not when heard, not for Everything; button calls `stream_audio_sound`. Update streamFlow expectations. Commit.

### Task 6: Verify, review, merge

- [ ] Client crate `cargo t` + clippy `-D warnings`; xwin clippy; vitest + svelte-check. Review the branch, fix findings. Open issues for deferrals. Rebase, rerun, push to main with `Closes #215`, delete branch.
