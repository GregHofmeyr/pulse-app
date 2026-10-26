# Call view and quality-of-life batch: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** make the voice call view the place streams live: stream tiles in the grid, a full view, a mini-preview while
you are elsewhere, right-click menus, instant join feedback with stages, a clickable connection panel, and a few
comforts (shift-click emoji, emoji freeze while live, unsent/failed messages that look and recover properly).

**Architecture:**
- **Core (Rust):** the `VoiceManager` emits a new `Stage` voice event at each join step. A new `voice/linkstats.rs`
  turns the stats the voice loop already samples into ping/jitter/loss/transport plus a 60 s ping history. It reports
  every second only while the UI watches it (`watch_voice_link`), on the same pattern as `watch_voice_meters`.
- **UI state:** `voice.svelte.ts` gains the stage, the optimistic "joining" entry and the failed join. The stream store
  turns `watching` into a list capped by `MAX_WATCHED = 1`, and it owns one `<video>` element per watch. Grid tiles,
  the full view and the mini-preview borrow that element through one attachment (`hostVideo`), so it moves without
  restarting.
- **UI components:** `VoicePanel` becomes a grid of `UserTile`/`StreamTile` or a `FullView` (today's `StreamViewer`,
  renamed). It reuses one `ParticipantMenu` with the channel list. The status block becomes `VoiceStatus` with a
  `LinkStatsPanel`.
- **Pure logic** goes in small `lib/*.ts` modules with vitest tests: tile order, menu contents, stage reducer,
  thresholds, unsent restore.

**Tech Stack:**
- Rust: Tauri 2 and the livekit/libwebrtc stats types (`livekit::webrtc::stats`).
- UI: Svelte 5 (runes, `{@attach}`), livekit-client `2.22.4`, and vitest + jsdom with `mount()`.

**Spec:** `docs/superpowers/specs/2026-10-26-call-view-qol-design.md`. Executors read the spec and this plan.

## Global Constraints

- **Public repo:**
  - No IPs, secrets, emails, friends' names or the owner's other projects in code, comments, commits, logs or test data.
  - Test users are `alex` and `sam`.
- **Checks:**
  - TDD throughout.
  - `just check` stays green after every task.
  - Any Rust change must also pass `cargo xwin clippy -p pulse-client --all-targets --target x86_64-pc-windows-msvc -- -D warnings`.
- **Join stage copy (UI), exact:**
  - `Requesting access`, `Connecting to voice`, `Starting audio`, `Connected`.
  - `Reconnecting…` (with an attempt number: `Reconnecting… (attempt 2)`), `Disconnected`.
  - The greyed entry reads `Joining…`.
- **Greyed/unsent opacity:** the joining entry and unsent messages render at **60 %** opacity.
- **Status thresholds:** each value is coloured green/amber/red.

  | Value | Green | Amber | Red |
  |---|---|---|---|
  | Ping (ms) | 0–99 | 100–199 | 200+ |
  | Jitter (ms) | 0–19 | 20–49 | 50+ |
  | Loss (%) | ≤ 1 | > 1 and ≤ 5 | > 5 |
- **Stats cadence:**
  - The panel refreshes every **1 s**, and only while it is open (and the window is focused).
  - The ping sparkline covers the last **60 s**.
  - Loss is measured over the last **5 s**.
  - With the panel closed, nothing beyond today's 5 s `voice: path` sampling runs.
- **Stats values:**
  - Ping is the WebRTC RTT on your own LiveKit connection.
  - Jitter and loss come from the remote-inbound stats for the audio you send: loss % = lost ÷ (lost + received).
  - Connection type is `UDP`, `TCP` or `TURN relay`.
  - Voice server is the LiveKit host name.
  - **Copy details** adds the app version.
- **Watch cap:**
  - `MAX_WATCHED = 1`. At the cap, a new Watch first stops the oldest watch.
  - Grid previews and the mini-preview always ask for the low layer.
  - Full view asks for the high layer under **Auto** and the low layer under **Data saver**.
- **Volumes:**
  - The per-person volume slider runs 0–200 % in steps of 5, as today.
  - The stream volume slider uses the same range.
  - **Mute for me** is local only and persists per person.
- **Remembered per device (guarded localStorage, the `prefs.ts`/`voiceui.ts` pattern):**
  - "Show non-streaming participants", per signed-in user: `pulse.callgrid.everyone.<userId>`, default on.
  - Mini-preview corner: `pulse.stream.minicorner`, default bottom-right.
  - Local mutes: `pulse.localmutes`.
  - Unsent messages, per signed-in user: `pulse.unsent.<userId>`.
- **Inline message errors** auto-hide after **5 s** (`INLINE_ERROR_MS = 5_000`).
- **Linux:**
  - Watching stays Windows-only (`watchDisabledReason` is unchanged): stream tiles on Linux show the reason in place
    of Watch.
  - Go Live stays Windows-only.
- **No protocol or server changes** in this push. Other people see you join through the existing `VoiceJoined`.

## Review Focus

1. **The watched stream ends while you are in full view or on the mini-preview.** The full view returns to the grid,
   the mini-preview disappears, and the "<name> stopped streaming" notice shows. Pinned by Task 7 test
   `stream_end_closes_full_view_and_mini_preview`.
2. **Leaving voice or switching channel while the mini-preview plays.** The watch ends (a stream never outlives your
   voice channel) and the mini-preview goes with it, with no orphan `<video>` left in the DOM. Pinned by Task 7 test
   `leaving_voice_drops_the_mini_preview_and_its_element`.
3. **Clicking a voice channel twice, or clicking another while a join is in flight.** You see one greyed entry, in the
   newest channel only. The superseded join's `cancelled` rejection is not shown as a failure with Retry. Pinned by
   Task 3 test `a_superseded_join_is_not_a_failure`.
4. **The stats panel is open and the window loses focus (left open for hours).** The 1 s polling stops and resumes on
   focus. Pinned by Task 4 test `unfocused_window_stops_link_polling`.
5. **Unsent messages restored after a restart for a channel that is gone** (or already in memory). Gone channels'
   messages are dropped; a nonce already present is not duplicated. Pinned by Task 11 test
   `restore_drops_gone_channels_and_known_nonces`.

---

## File map

| Area | Files |
|---|---|
| Core stages | `client/src-tauri/src/voice/mod.rs` |
| Core link stats | create `client/src-tauri/src/voice/linkstats.rs`; modify `voice/mod.rs`, `commands.rs`, `lib.rs` |
| Voice UI state | create `client/ui/src/lib/voicestage.ts`; modify `lib/voice.svelte.ts`, `lib/voiceui.ts`, `lib/selectors.ts` |
| Status block | create `components/VoiceStatus.svelte`, `components/LinkStatsPanel.svelte`, `lib/voicelink.ts`; modify `routes/Shell.svelte` |
| Watch list | modify `lib/stream/stream.svelte.ts`, `lib/stream/room.ts`, `lib/stream/viewer.ts`; create `lib/stream/videoHost.ts` |
| Grid | create `lib/callgrid.ts`, `components/UserTile.svelte`, `components/StreamTile.svelte`; modify `components/VoicePanel.svelte`, `components/LiveCard.svelte`, `lib/stream/prefs.ts` |
| Full view / mini | rename `components/StreamViewer.svelte` → `components/FullView.svelte`; create `components/MiniPreview.svelte`; modify `lib/stream/viewer.svelte.ts`, `components/StreamToast.svelte`, `components/LivePopover.svelte`, `routes/Shell.svelte` |
| Menus | create `lib/participantMenu.ts`, `components/ParticipantMenu.svelte`, `lib/stream/golive.svelte.ts`; modify `components/ChannelList.svelte`, `components/UserTile.svelte`, `components/StreamTile.svelte` |
| Emoji | modify `components/EmojiPicker.svelte`, `EmojiPopover.svelte`, `Composer.svelte`, `MessageItem.svelte`, `ReactionRow.svelte`, `lib/animate.svelte.ts`, `app.css` |
| Messages | create `lib/unsent.ts`; modify `lib/state.ts`, `lib/store.svelte.ts`, `components/MessageItem.svelte`, `components/TextChannel.svelte`, `components/ReactionRow.svelte` |

Component tests sit next to their component (`components/X.test.ts`, `// @vitest-environment jsdom`, Tauri modules
mocked as in `routes/Shell.test.ts`). Pure modules are tested in `lib/x.test.ts`.

---

### Task 1: Core: join stages

**Files:**
- Modify: `client/src-tauri/src/voice/mod.rs` (`VoiceEvent`, `join_inner`, `connect`, `rejoin_attempts`, `report_publish`).
- Test: the unit tests in `voice/mod.rs`; `client/src-tauri/tests/voice_it.rs`.

**Interfaces:**
- Produces:
  - `#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)] #[serde(rename_all = "snake_case")] pub enum JoinStage { RequestingAccess, Connecting, StartingAudio, Connected, Reconnecting }`.
  - `VoiceEvent::Stage { stage: JoinStage, attempt: Option<u32> }`. On the wire it is
    `{"kind":"stage","stage":"starting_audio","attempt":null}`.
- When each stage is emitted:
  - `RequestingAccess`: in a user join (`rejoin_of == None`), right after the existing
    `emit_state(.., Connecting)` and before the token is taken or fetched.
  - `Connecting`: in `connect`, once the voice token is in hand (prefetched or fetched). Not emitted on a rejoin.
  - `StartingAudio`: in `join_inner`'s `Ok` arm, just before the existing `emit_state(.., Connected)`. Emitted for joins
    and rejoins.
  - `Connected`: in `report_publish`'s `Ok` arm, next to `Mic { live: true }`, under the same `alive` check.
  - `Reconnecting` with `attempt: Some(attempt + 1)`: in `rejoin_attempts`, before each `join_inner` call (after the
    backoff sleep).
- `Disconnected` and LiveKit's own `RoomEvent::Reconnecting` stay as today's `State` events. The UI derives those
  two stages from `State` (Task 3).

- [ ] **Step 1: Failing unit tests** in `voice/mod.rs`. Each collects `Stage`/`State` events from the sink into a
  `Vec<String>` (tags like `stage:requesting_access`, `state:disconnected`):
  - `a_failed_token_request_stops_after_requesting_access`. `sam` joins a voice channel of `alex`'s server without
    being a member, so the token request is refused. Expect
    `["state:connecting", "stage:requesting_access", "state:disconnected"]`, and no `stage:connecting`.
  - `a_failed_connect_stops_after_connecting`. Use `testing::spawn_with` with `livekit_url = "ws://127.0.0.1:1"`.
    Expect the order `stage:requesting_access` < `stage:connecting` < `state:disconnected`, and no
    `stage:starting_audio`.
  - `rejoin_attempts_are_numbered`. Same unreachable LiveKit, `vm.rejoin_unit = Duration::from_millis(1)`,
    `simulate_drop`. Expect `stage:reconnecting:1`, `stage:reconnecting:2`, … up to `rejoin::MAX_ATTEMPTS`, then
    `state:disconnected`.
  - `stage_event_serialises_for_the_ui`: `{"kind":"stage","stage":"reconnecting","attempt":2}`.
- [ ] **Step 2: Run** `cargo test -p pulse-client voice::tests` → FAIL (no `Stage` variant).
- [ ] **Step 3: Implement** the emissions listed above.
- [ ] **Step 4: Extend `voice_it.rs`.** Add `stages_arrive_in_order` (ignored, real LiveKit, modelled on
  `connected_shows_before_the_mic_is_published`). Expect `requesting_access`, `connecting`, `starting_audio`,
  `connected`, in that order, with `state:connected` between `starting_audio` and `connected`.
- [ ] **Step 5: Run** `cargo test -p pulse-client` → PASS; `just voice-it` → PASS (rerun once on the known cold-start
  flake).
- [ ] **Step 6: Commit** `feat(voice): report join stages to the UI`.

### Task 2: Core: link stats for the status panel

**Files:**
- Create: `client/src-tauri/src/voice/linkstats.rs` (with its unit tests).
- Modify:
  - `voice/mod.rs`: `VoiceManager` gets `link: Arc<linkstats::LinkState>`. The stats task in `connect` uses
    `run_ticks`. The server host is stored on join, and history is cleared on leave.
  - `commands.rs`: the `watch_voice_link` command.
  - `lib.rs`: register the command.

**Interfaces:**
- Produces, in `linkstats.rs`:
  - Constants: `pub const LINK_EVERY: Duration = 1s`, `pub const HISTORY: Duration = 60s`,
    `pub const LOSS_WINDOW: Duration = 5s`.
  - `#[serde(rename_all = "snake_case")] pub enum Transport { Udp, Tcp, Turn }`.
  - `pub struct LinkSample { pub rtt_ms: Option<f64>, pub jitter_ms: Option<f64>, pub loss_pct: Option<f64>, pub transport: Option<Transport> }`.
  - `pub struct LossWindow`: a `VecDeque<(Instant, lost: i64, received: u64, sent: u64)>` trimmed to `LOSS_WINDOW`.
  - `pub fn sample(publisher: &[RtcStats], loss: &mut LossWindow, now: Instant) -> LinkSample`. Its rules:
    - **RTT:** the nominated, succeeded candidate pair's `current_round_trip_time` × 1000 (the rule in
      `netstats::summarize`).
    - **Jitter:** the audio `RemoteInboundRtp`'s `received.jitter` × 1000.
    - **Loss:** Δlost ÷ (Δlost + Δreceived) over the window. libwebrtc can leave remote-inbound `packets_received` at
      0; when Δreceived is 0, use the audio `OutboundRtp` Δpackets_sent as the denominator. `None` when the
      denominator is 0.
    - **Transport:** the pair's local candidate (matched by `rtc.id == local_candidate_id`). `candidate_type` Relay →
      `Turn`; otherwise protocol `"tcp"` → `Tcp`; otherwise `Udp`.
  - `pub struct LinkHistory`: `push(now: Instant, rtt_ms: f64)` and
    `points(now: Instant) -> Vec<(u32 /*ms ago*/, f64)>`. Holds only the last `HISTORY`.
  - `pub struct LinkState`:
    - Fields: `watching: AtomicBool`, `wake: Notify`, `history: Mutex<LinkHistory>`, `server: Mutex<Option<String>>`.
    - Methods: `set_watching(&self, on: bool)` (on → `wake.notify_one()`) and `clear(&self)`.
  - `#[derive(Serialize)] pub struct LinkReport`, with the fields `rtt_ms`, `jitter_ms`, `loss_pct`, `transport`,
    `server: Option<String>`, `version: &'static str` (`env!("CARGO_PKG_VERSION")`) and `history: Vec<(u32, f64)>`.
  - `pub struct Tick { pub log: bool, pub report: bool }`.
  - `pub async fn run_ticks<F, Fut>(link: Arc<LinkState>, mut on_tick: F) where F: FnMut(Tick) -> Fut, Fut: Future<Output = ()>`.
    It fires `{log: true, report: watching}` every `netstats::WINDOW`. While `watching`, it also fires
    `{log: false, report: true}` every `LINK_EVERY`. It wakes on `wake` when watching turns on, so the first report
    is immediate.
- Produces, in `voice/mod.rs`:
  - `VoiceEvent::Link(linkstats::LinkReport)`, which serialises with `"kind":"link"`.
  - `pub fn watch_link(&self, on: bool)`.
  - Every tick pushes the RTT into the history. A `report` tick emits `Link`. A `log` tick keeps today's
    `voice: path` line unchanged.
  - The server is `reqwest::Url::parse(&vt.url)` → `host_str()`, stored when the room connects.
- Produces, in `commands.rs`: `#[tauri::command] pub fn watch_voice_link(voice: State<'_, VoiceManager>, on: bool)`.

- [ ] **Step 1: Failing tests** in `linkstats.rs`. Build `RtcStats` fixtures the way `netstats.rs` tests do:
  - `rtt_from_the_active_pair_only`: two pairs, only the nominated succeeded one counts. 0.042 s gives `Some(42.0)`.
  - `jitter_from_remote_inbound_audio`: jitter 0.018 gives `Some(18.0)`.
  - `loss_over_the_window`: samples at t0 (lost 0, received 100) and t0+1 s (lost 2, received 198) give
    `loss_pct == Some(2.0)`. A sample older than 5 s drops out of the window.
  - `loss_falls_back_to_packets_sent`: received stays 0 and sent goes 0→100 with lost 0→1, giving `Some(1.0)`.
  - `transport_turn_tcp_udp`: local candidate type relay gives `Turn`, host/tcp gives `Tcp`, srflx/udp gives `Udp`.
  - `history_keeps_sixty_seconds`: push at 0, 30 and 61 s. At t = 61 s, `points` returns 2 entries with ms-ago
    `[31000, 0]`.
  - `run_ticks_only_reports_while_watched` (`#[tokio::test(start_paused = true)]`):
    - 20 s unwatched gives 4 ticks, all `report: false`.
    - `set_watching(true)` gives an immediate report tick, then one per second (≥ 9 report ticks in the next 10 s).
    - `set_watching(false)` gives no `log: false` ticks in the next 10 s.
  - `link_event_serialises_for_the_ui`: `kind == "link"`, `transport == "turn"`, and `history` is an array of
    `[ms, rtt]` pairs.
- [ ] **Step 2: Run** `cargo test -p pulse-client linkstats` → FAIL.
- [ ] **Step 3: Implement** `linkstats.rs`. Then rewire the stats task in `connect` onto `run_ticks`, one
  `room.get_stats()` per tick. Add `watch_link`, the command and its registration. Clear the link state in `leave`.
- [ ] **Step 4: Run** `cargo test -p pulse-client`, `just clippy`, and the xwin clippy → PASS.
- [ ] **Step 5: Commit** `feat(voice): ping, jitter, loss and connection type for the status panel`.

### Task 3: UI: voice stages and the optimistic join

**Files:**
- Create: `client/ui/src/lib/voicestage.ts` and `lib/voicestage.test.ts`.
- Modify:
  - `lib/voiceui.ts`: export the `VoiceEvent` union, moved here from `voice.svelte.ts`, with the `stage` and `link`
    variants added.
  - `lib/voice.svelte.ts`, `lib/selectors.ts`, `lib/selectors.test.ts`.
  - `components/ChannelList.svelte`, plus a test in `components/ChannelList.test.ts`.

**Interfaces:**
- Produces, in `voicestage.ts`:
  - `type Stage = 'requesting_access' | 'connecting' | 'starting_audio' | 'connected' | 'reconnecting' | 'disconnected'`.
  - `type StageState = { stage: Stage; attempt: number | null }`.
  - `reduceStage(cur: StageState, e: VoiceEvent): StageState`. Its rules:
    - A `stage` event sets the stage directly.
    - `state` disconnected → `disconnected`.
    - `state` reconnecting with no stage event → `reconnecting`. The attempt is kept if the stage was already
      reconnecting, else `null`.
    - `state` connected while reconnecting → `connected` (LiveKit resumed). Otherwise `state` connected leaves the
      stage alone.
    - `mic` with an error while the stage is `starting_audio` → `connected`. That is microphone trouble, not a failed
      join.
  - `stageLabel(s: StageState): string`, using the exact copy from Global Constraints.
  - `isJoining(stage: Stage): boolean`: true for `requesting_access` and `connecting`.
- Produces, in `voice.svelte.ts` (new fields on `voice`):
  - `stage: StageState`.
  - `joining: string | null`: set on click, cleared at `starting_audio` or on failure.
  - `leaving: string | null`: the channel just left, until the server's list drops you.
  - `joinFailed: { channelId: string; error: string } | null`.
  - `link: LinkReport | null`.
- Produces, in `voiceApi`:
  - `join(channelId)` sets `joining`, `leaving` (the previous `voice.channelId`) and stage `requesting_access`
    synchronously, before `invoke`. A rejection whose text is `cancelled` is ignored. Any other rejection sets
    `joinFailed` and clears `joining`.
  - `retry(): Promise<void>` re-joins `joinFailed.channelId`.
- Produces, in `selectors.ts`:
  - `type SelfVoice = { channelId: string | null; joining: boolean; leaving: string | null }`.
  - `type RosterEntry = Occupant & { pending: boolean }`.
  - `callRoster(s: AppState, serverId: string | null, channelId: string, me: SelfVoice): RosterEntry[]`. Its rules:
    - In `me.channelId`, you are appended with `pending: me.joining` if the server hasn't listed you yet. If it has,
      `pending` is `me.joining`.
    - In `me.leaving`, you are filtered out.
    - Other people always have `pending: false`.
- Consumes: Task 1's `stage` event shape and Task 2's `link` event shape.

- [ ] **Step 1: Failing tests:**
  - `voicestage.test.ts`:
    - `stages_follow_the_core_in_order`.
    - `livekit_reconnect_then_resume_is_connected`.
    - `rejoin_attempt_is_kept_through_state_reconnecting`.
    - `mic_error_while_starting_audio_counts_as_connected`.
    - `labels_match_the_spec_copy`: `stageLabel({stage:'reconnecting',attempt:2}) === 'Reconnecting… (attempt 2)'`.
  - `selectors.test.ts`:
    - `roster_adds_you_greyed_while_joining`.
    - `roster_marks_you_pending_when_the_server_already_lists_you`.
    - `roster_drops_you_from_the_channel_you_are_leaving`.
    - `roster_never_marks_others_pending`.
  - `ChannelList.test.ts`:
    - `joining_shows_a_greyed_entry_with_spinner_at_once`: after `voiceApi.join('V1')`, with `invoke` pending, the
      occupant row has class `pending`, its text is `Joining…`, and it has a `[role=progressbar]` or `.spinner`.
    - `failed_join_removes_the_entry`: `invoke` rejects `'couldn't connect to voice: x'`, so the row is gone and
      `voice.joinFailed.channelId === 'V1'`.
    - `a_superseded_join_is_not_a_failure`: the first join rejects `'cancelled'` after a second `join('V2')`, so
      `voice.joinFailed === null` and there is one pending row, in V2.
- [ ] **Step 2: Run** `pnpm -C client/ui test` → the new tests FAIL.
- [ ] **Step 3: Implement.** `ChannelList` renders `callRoster(...)` instead of `voiceOccupants`. A pending row gets
  `class:pending` (opacity .6), a small CSS spinner, and the label `Joining…` in place of the flags. The event
  listener runs every event through `reduceStage`. It clears `joining` once the stage is not `isJoining`. It clears
  `leaving` once `app.state.voice[leaving]` no longer has you.
- [ ] **Step 4: Run** `just ui-check` → PASS.
- [ ] **Step 5: Commit** `feat(ui): instant join feedback with stages`.

### Task 4: UI: the clickable voice status and stats panel

**Files:**
- Create:
  - `lib/voicelink.ts` and `lib/voicelink.test.ts`.
  - `components/VoiceStatus.svelte` and `components/LinkStatsPanel.svelte`.
  - `components/VoiceStatus.test.ts`.
- Modify: `routes/Shell.svelte`, which replaces its inline `.vc` block with `<VoiceStatus />`.

**Interfaces:**
- Produces, in `voicelink.ts`:
  - `type Level = 'good' | 'warn' | 'bad'`.
  - `pingLevel(ms: number): Level`, `jitterLevel(ms: number): Level` and `lossLevel(pct: number): Level`, using the
    thresholds in Global Constraints.
  - `transportLabel(t: 'udp' | 'tcp' | 'turn'): string`, which gives `UDP`, `TCP` or `TURN relay`.
  - `sparklinePath(points: [number, number][], w: number, h: number): string`. It returns an SVG path; x is
    `60 000 − msAgo` scaled to `w`.
  - `detailsText(r: LinkReport, channelName: string): string`. One plain-text block of `key: value` lines: Pulse
    version, channel, voice server, connection type, ping, jitter and loss (`—` for a missing value).
- `VoiceStatus` shows when `voice.channelId || voice.joining || voice.joinFailed`:
  - While joining or reconnecting it shows `stageLabel(voice.stage)` with the warn style.
  - Once the stage is `connected` or `starting_audio`, it becomes a `<button aria-haspopup="dialog">` that opens
    `LinkStatsPanel`.
  - `joinFailed` shows the error and a **Retry** button (`voiceApi.retry()`).
  - The leave button stays as today.
- `LinkStatsPanel` (`role="dialog" aria-label="Connection details"`):
  - On mount it calls `voiceApi.watchLink(true)`, and on destroy `watchLink(false)`.
  - An `$effect` on `windowFocus.focused` turns watching off while unfocused and back on when focus returns.
  - It renders the three coloured values, the sparkline, server, type and **Copy details**
    (`navigator.clipboard.writeText(detailsText(...))`).
  - Esc or a click outside closes it.
- Produces, in `voiceApi`: `watchLink(on: boolean)`, which invokes `watch_voice_link`. `voice.link` is filled from
  `link` events (Task 3's listener).
- Consumes: Task 3's `voice.stage`, `voice.link` and `voiceApi.retry`, and Task 2's `watch_voice_link`.

- [ ] **Step 1: Failing tests:**
  - `voicelink.test.ts`:
    - `ping_thresholds`: 99 → good, 100 → warn, 199 → warn, 200 → bad.
    - `jitter_thresholds`: 19 / 20 / 49 / 50.
    - `loss_thresholds`: 1 → good, 1.1 → warn, 5 → warn, 5.1 → bad.
    - `details_text_lists_every_value_and_the_version`.
    - `sparkline_spans_sixty_seconds`.
  - `VoiceStatus.test.ts`:
    - `shows_the_stage_while_joining`.
    - `connected_becomes_a_button_that_opens_the_panel`, which also asserts `invoke('watch_voice_link', {on:true})`.
    - `closing_the_panel_stops_link_polling` (`{on:false}`).
    - `unfocused_window_stops_link_polling`: set `windowFocus.focused = false`, flush, expect `{on:false}`; set it
      true and expect `{on:true}` again.
    - `failed_join_shows_retry`, which also checks that the click calls `join_voice` for that channel.
    - `values_are_coloured_by_level`: a `link` event with rtt 150 gives `.warn` on the ping value.
- [ ] **Step 2: Run** them → FAIL.
- [ ] **Step 3: Implement.** Also make `VoicePanel`'s header pill show `stageLabel(voice.stage)`.
- [ ] **Step 4: Run** `just ui-check` → PASS.
- [ ] **Step 5: Commit** `feat(ui): clickable connection status with ping, jitter and loss`.

### Task 5: UI: watching becomes a list the store owns

**Files:**
- Modify:
  - `lib/stream/stream.svelte.ts` and `lib/stream/room.ts`.
  - `lib/stream/viewer.ts` and `lib/stream/viewer.test.ts`.
  - `lib/stream/streamFlow.test.ts`: update its `watch(…, el)` calls and the fake room's methods.
- Create: `lib/stream/videoHost.ts` and `lib/stream/videoHost.test.ts`.

**Interfaces:**
- Produces, in `viewer.ts`:
  - `type View = 'preview' | 'full'`.
  - `type Quality = 'auto' | 'saver'`.
  - `layerFor(view: View, quality: Quality): 'low' | 'high'`: `full` + `auto` gives `high`; anything else gives `low`.
  - `admit<T>(list: T[], next: T, max: number): { list: T[]; dropped: T[] }`. The oldest entries go first.
- Produces, in `stream.svelte.ts`:
  - `export const MAX_WATCHED = 1`.
  - `type Watching = { channelId: string; streamer: string; quality: Quality; view: View }`.
  - `stream.watching: Watching[]` (was `Watching | null`).
  - `findWatch(channelId: string, streamer: string): Watching | undefined`.
  - `videoOf(channelId: string, streamer: string): HTMLVideoElement | null`. Not reactive: the store creates one
    `<video autoplay playsinline muted>` per watch and drops it, `srcObject` cleared, when the watch ends.
  - `streamApi.watch(channelId, streamer): Promise<void>` (no element argument). A second call for the same stream is a
    no-op. At the cap it ends the dropped watches first.
  - `streamApi.unwatch(channelId: string, streamer: string)` and `streamApi.unwatchAll()`.
  - `streamApi.setQuality(channelId, streamer, q: Quality)` and `streamApi.setView(channelId, streamer, view: View)`.
    Both apply `room.setLayer(streamer, layerFor(view, quality))`.
  - `streamApi.setStreamVolume(streamer: string, pct: number)`. It saves through `saveStreamVolumes` and calls
    `streamCmd.setVolume`. It is also applied once each watch is up (moved out of `StreamViewer`).
  - `onStreamEnded`, `onVoiceChanged`, `onAudioClosed` and `onReady` act per entry of the list. Generations become
    per watch key (`Map<string, ReturnType<typeof generation>>`).
  - Inbound diagnostics follow the entry whose `view` is `full`.
- Produces, in `room.ts`:
  - `StreamRoom` holds `watched: Map<string, Watched>`.
  - `watch(streamer, el)` is unchanged in shape, but no longer unwatches other streamers.
  - `unwatch(streamer?: string)`: no argument means all.
  - `setLayer(streamer, layer: 'low' | 'high')` replaces `setQuality`. A new watch starts on `low`.
  - `remoteStats(streamer)`.
  - The `TrackPublished`/`Reconnected` handlers loop over the map.
- Produces, in `videoHost.ts`:
  - `hostVideo(channelId: string, streamer: string): Attachment<HTMLElement>`. It appends `videoOf(...)` into the
    node, then calls `void el.play().catch(() => {})` if paused.
  - Its cleanup removes the element only if `el.parentNode === node`, because the next host may already have taken it.

- [ ] **Step 1: Failing tests:**
  - `viewer.test.ts`:
    - `preview_is_always_low_full_follows_quality`.
    - `admit_drops_the_oldest_at_the_cap`.
  - `streamFlow.test.ts`:
    - `watching_a_second_stream_at_the_cap_stops_the_first`: `unwatch` for the first, an `audio_watch off` for it, and
      `stream.watching.length === 1`.
    - `a_new_watch_starts_on_the_low_layer` (`room.setLayer('other', 'low')`).
    - `full_view_asks_for_high_under_auto_and_low_under_saver`.
    - `the_watch_owns_one_video_element_until_it_ends`: `videoOf` is the same element across `setView`, and `null`
      after `unwatch`.
    - Update the existing watch tests to the new signatures.
  - `videoHost.test.ts`:
    - `moving_between_hosts_keeps_the_element`: mount host A, then B, then unmount A. The element is in B, and A's
      cleanup didn't remove it.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.** Keep `StreamViewer` compiling for now: it calls `watch(c, s)` and hosts the element with
  `hostVideo` until Task 7 replaces it.
- [ ] **Step 4: Run** `just ui-check` → PASS.
- [ ] **Step 5: Commit** `refactor(ui): watches are a capped list with store-owned video`.

### Task 6: UI: the call grid with stream tiles

**Files:**
- Create:
  - `lib/callgrid.ts` and `lib/callgrid.test.ts`.
  - `components/UserTile.svelte` and `components/StreamTile.svelte`.
  - `components/VoicePanel.test.ts`.
- Modify:
  - `components/VoicePanel.svelte`: the grid and header toggle; the `LiveCard` below the grid goes.
  - `components/LiveCard.svelte`: rendered inside the own stream tile; keeps hide-preview, audio mute, Stop, viewers
    and the encoder warning.
  - `lib/stream/prefs.ts`.

**Interfaces:**
- Produces, in `callgrid.ts`:
  - `type Tile = { kind: 'user'; entry: RosterEntry } | { kind: 'stream'; streamer: string; mine: boolean }`.
  - `gridTiles(a: { roster: RosterEntry[]; streams: Record<string, StreamInfo>; meId: string; showEveryone: boolean }): Tile[]`.
    Its rules:
    - Each stream tile sits right after its streamer's user tile.
    - A stream whose streamer has no user tile goes at the end.
    - `showEveryone: false` keeps only stream tiles.
  - `nobodyStreaming(tiles: Tile[], showEveryone: boolean): boolean`.
- Produces, in `prefs.ts`: `loadShowEveryone(userId: string): boolean` (default `true`) and
  `saveShowEveryone(userId: string, on: boolean)`.
- `UserTile` props:
  - `{ entry: RosterEntry; channelId: string; compact?: boolean; onMenu: (e: MouseEvent | KeyboardEvent, anchor: HTMLElement) => void }`.
  - It renders the avatar, the speaking ring, the badges, the bars, and the volume label.
  - A pending entry is at .6 opacity with a spinner and `Joining…`.
  - `compact` is for the full-view strip.
  - An avatar click opens the menu (it replaces today's volume popover).
  - The `LiveBadge` leaves grid tiles; the stream tile replaces it.
- `StreamTile` props:
  - `{ channelId: string; serverId: string | null; streamer: string; mine: boolean; onMenu: …; onFull: (k: { channelId: string; streamer: string }) => void }`.
  - `VoicePanel` passes a no-op `onFull` until Task 7 wires it to `openFull`.
  - **Mine:** `LiveCard` content.
  - **Not watching:** the avatar on a dimmed backdrop, a red `LIVE` tag, and a **Watch** button. In a voice channel
    you're not in, the button reads **Join & watch** (Task 7 wires it). On Linux it shows the
    `watchDisabledReason` text instead. The eye icon + viewer count is hidden at 0.
  - **Watching:**
    - The tile hosts the video (`{@attach hostVideo(...)}`).
    - Its hover toolbar has stream volume, quality (`Select`, Auto / Data saver), **Stop watching** and **Full view**.
    - Clicking the preview calls `onFull`.
  - A click anywhere on a not-watching tile watches (one click).
- In `VoicePanel`:
  - The header gets an eye toggle (`aria-pressed`, label `Show non-streaming participants`).
  - With the toggle off and nobody live, the grid shows `Nobody's streaming — show everyone` with a button that turns
    the toggle on.
  - The grid is CSS grid, 16:9 tiles (`aspect-ratio: 16 / 9`), 1/2/3 columns by width (container query).
- Consumes: Task 3's `callRoster`, and Task 5's `findWatch`, `streamApi.watch/unwatch/setQuality/setStreamVolume` and
  `hostVideo`.

- [ ] **Step 1: Failing tests:**
  - `callgrid.test.ts`:
    - `stream_tile_follows_its_streamer`: roster `[alex, sam]` with sam live gives `user alex, user sam, stream sam`.
    - `orphan_stream_goes_last`.
    - `toggle_off_shows_only_streams`.
    - `nobody_streaming_with_toggle_off_is_empty_state`.
  - `VoicePanel.test.ts`, with the store seeded so `sam` is live in `V1` and you are in `V1`:
    - `not_watching_tile_shows_live_watch_and_viewer_count`: the count is hidden when `viewers` is empty, and shows
      `2` for two viewers.
    - `one_click_watches_the_stream`, which also asserts `stream_watch` was invoked.
    - `watching_tile_has_volume_quality_stop_and_full_view`.
    - `own_stream_tile_shows_the_local_preview_and_hide_toggle`.
    - `toggle_hides_user_tiles_and_is_remembered_per_user`, checking localStorage key `pulse.callgrid.everyone.U1`.
    - `empty_state_button_shows_everyone_again`.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `just ui-check` → PASS.
- [ ] **Step 5: Commit** `feat(ui): stream tiles in the call grid`.

### Task 7: UI: full view, mini-preview, and watching from elsewhere

**Files:**
- Rename `components/StreamViewer.svelte` to `components/FullView.svelte` (`git mv`), then modify it.
- Create: `components/MiniPreview.svelte`; tests `components/FullView.test.ts` and `components/MiniPreview.test.ts`.
- Modify:
  - `lib/stream/viewer.svelte.ts`: `viewing` is replaced by `callView`.
  - `lib/stream/prefs.ts`.
  - `components/VoicePanel.svelte`, `components/StreamToast.svelte` and `components/LivePopover.svelte`.
  - `routes/Shell.svelte` and `routes/Shell.test.ts`.

**Interfaces:**
- Produces, in `viewer.svelte.ts`:
  - `type WatchKey = { channelId: string; streamer: string }`.
  - `callView = $state({ full: null as WatchKey | null, goTo: null as string | null, joinThenWatch: null as WatchKey | null })`.
  - `openFull(k: WatchKey)` sets `full` and `streamApi.setView(.., 'full')`. `closeFull()` sets the view back to
    `preview` and `full = null`.
  - `watchStream(channelId, streamer)` (toast and popover):
    - In that voice channel: `streamApi.watch`, then `openFull`, then `goTo = channelId`.
    - Otherwise: `joinAndWatch`.
  - `joinAndWatch(channelId, streamer)` sets `joinThenWatch`, `goTo`, then calls `voiceApi.join(channelId)`.
  - `pendingWatchReady(p: WatchKey | null, v: { channelId: string | null; connection: string }): boolean` (pure, in
    `viewer.ts`). An `$effect` in Shell runs `watch` + `openFull` once it is true, then clears `joinThenWatch`. A
    failed join clears it too.
- Produces, in `prefs.ts`:
  - `type Corner = 'tl' | 'tr' | 'bl' | 'br'`.
  - `loadMiniCorner(): Corner` (default `'br'`) and `saveMiniCorner(c: Corner)`.
- `FullView` props: `{ channelId: string; streamer: string; serverId: string | null }`.
  - **Bar:** **Back** (top-left), the name, quality, stream volume, **Stop watching**.
  - **Stage:** `hostVideo`, `StreamDiagOverlay kind="in"`, and the `Reconnecting…` overlay as today.
  - **Strip:** compact `UserTile`s along the bottom, with speaking rings.
  - **Esc** (window keydown, ignored while focus is in an input, textarea or menu) calls `closeFull()`. The stream
    keeps playing as a preview.
- In `VoicePanel`: when `callView.full` is in this channel and still in `stream.watching`, the grid area renders
  `FullView`.
- `MiniPreview`:
  - Shown by Shell when `stream.watching.length > 0` and the main area is not that watch's voice channel.
  - It is a fixed box in the main area's corner `loadMiniCorner()`, hosting the video.
  - Dragging it snaps to the nearest corner and saves it.
  - A click sets `goTo` to the channel, keeping `callView.full` as it was (full view if you left from there).
  - Its **✕** (`aria-label="Stop watching"`) calls `streamApi.unwatch`.
- In `Shell`:
  - An `$effect` consumes `callView.goTo`: it selects the channel's server and channel, then clears `goTo`.
  - Shell no longer renders `StreamViewer`.
  - It clears `callView.full` when that key leaves `stream.watching`.
- `StreamToast` and `LivePopover`:
  - **Watch** calls `watchStream`.
  - For a stream in another voice channel (Windows), the button reads **Join & watch** and calls `joinAndWatch`.
- Consumes: Task 5 (`setView`, `hostVideo`, `findWatch`), Task 6 (`UserTile compact`, `StreamTile`'s `openFull`
  hook), and Task 3 (`voiceApi.join`, `voice.joinFailed`).

- [ ] **Step 1: Failing tests:**
  - `FullView.test.ts`:
    - `back_and_esc_return_to_the_grid_and_keep_watching`: `callView.full === null`, the watch is still in the list,
      and `setLayer` was called with low.
    - `strip_shows_speaking_rings`.
  - `MiniPreview.test.ts`:
    - `shows_when_watching_and_away_from_the_call`.
    - `click_returns_to_the_call_view_in_full_view_if_you_left_from_there`.
    - `x_stops_watching`.
    - `drag_snaps_and_remembers_the_corner`.
  - `Shell.test.ts`:
    - `stream_end_closes_full_view_and_mini_preview`: a `StreamEnded` event for the watched stream leaves
      `.mini-preview` and `FullView` gone, with the notice text `sam stopped streaming`.
    - `leaving_voice_drops_the_mini_preview_and_its_element`: after the `state` event with `channel_id: null`, there
      is no `.mini-preview` and `document.querySelectorAll('video').length === 0`.
    - `toast_watch_opens_the_call_view_in_full_view`.
    - `join_and_watch_joins_then_opens_the_stream`: `join_voice` is invoked. After a `voice://event`
      `state` event (`connection: 'connected'`) for that channel, `stream_watch` is invoked and full view shows.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `just ui-check` → PASS.
- [ ] **Step 5: Commit** `feat(ui): full view, mini-preview, and watch from anywhere`.

### Task 8: UI: right-click menus

**Files:**
- Create:
  - `lib/participantMenu.ts` and `lib/participantMenu.test.ts`.
  - `components/ParticipantMenu.svelte` and `components/ParticipantMenu.test.ts`.
  - `lib/stream/golive.svelte.ts`.
- Modify:
  - `lib/voiceui.ts`: local mutes.
  - `lib/voice.svelte.ts`: `setLocalMute`, and re-apply it on join.
  - `components/VoicePanel.svelte`: Go Live goes through `golive.svelte.ts`. The `GoLiveDialog` render moves to Shell.
  - `components/UserTile.svelte`, `components/StreamTile.svelte` and `components/ChannelList.svelte`: the occupant
    rows become focusable, with the menu on right-click.
  - `routes/Shell.svelte`: passes `onMessage={messageUser}` and `onVoiceSettings={() => openSettings('voice')}` down
    through `VoicePanel` and `ChannelList`.

**Interfaces:**
- Produces, in `participantMenu.ts`:
  - `type MenuItemId = 'volume' | 'local-mute' | 'message' | 'stream-volume' | 'watch' | 'stop-watching' | 'full-view' | 'mute' | 'deafen' | 'go-live' | 'stop-streaming' | 'voice-settings'`.
  - `type MenuCtx = { self: boolean; live: boolean; watching: boolean; full: boolean; inCall: boolean; canGoLive: boolean; streamingSelf: boolean }`.
  - `menuItems(ctx: MenuCtx): MenuItemId[]`. Its rules:
    - **Other:** `volume`, `local-mute`, `message`. If live, also `stream-volume` (only while watching), then `watch` or
      `stop-watching`, then `full-view` (watching and not full).
    - **Self:** `mute`, `deafen`. Then `go-live` (`canGoLive`) or `stop-streaming` (`streamingSelf`). Then
      `voice-settings`.
- Produces, in `voiceui.ts`: `loadLocalMutes(): Record<string, true>` and `saveLocalMutes(m)`, key `pulse.localmutes`.
- Produces, in `voiceApi`: `setLocalMute(userId: string, on: boolean)`. On sends `set_peer_volume` 0. Off sends the
  remembered volume (default 100). It persists, and `join` re-applies local mutes after volumes.
- Produces, in `golive.svelte.ts`:
  - `goLiveUi = $state({ picked: null as Picked | null, channelId: null as string | null, error: '' })`.
  - `pickSource(channelId: string): Promise<void>`: moved from `VoicePanel`, with the same cancel rule.
  - `closeGoLive()`.
  - The leave/lose-connection cleanup effect moves with it, into Shell.
- `ParticipantMenu` props:
  - `{ channelId: string; serverId: string | null; userId: string; x: number; y: number; opener: HTMLElement; onClose: () => void; onMessage: (userId: string) => void; onVoiceSettings: () => void }`.
  - It is `role="menu"`, `position: fixed`, clamped to the viewport, and focuses the first item.
  - **Keys:** Up/Down move between items, Enter picks, Esc closes and returns focus to `opener`. On a focused slider,
    Left/Right adjust it.
  - A click outside closes it.
  - The slider rows are labelled `User volume` / `Stream volume` and show their value.
- Openers: `contextmenu` on tiles and channel-list occupants, and `keydown` with `ContextMenu`, or `F10` with
  `shiftKey`, on a focused tile or row.

- [ ] **Step 1: Failing tests:**
  - `participantMenu.test.ts`:
    - `other_not_live`: `['volume','local-mute','message']`.
    - `other_live_not_watching`: `[…,'watch']`.
    - `other_live_watching_in_grid`: `[…,'stream-volume','stop-watching','full-view']`.
    - `self_in_call_can_go_live`: `['mute','deafen','go-live','voice-settings']`.
    - `self_streaming`: `stop-streaming` in place of `go-live`.
  - `ParticipantMenu.test.ts`:
    - `right_click_on_a_tile_opens_the_menu`.
    - `shift_f10_and_context_menu_key_open_it`.
    - `arrows_move_enter_picks_esc_closes_and_restores_focus`.
    - `mute_for_me_persists_and_sends_zero_volume`: `invoke('set_peer_volume', {userId:'U2', percent:0})`, and
      localStorage `pulse.localmutes` contains `U2`.
    - `message_calls_on_message`.
    - `channel_list_occupants_use_the_same_menu`.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `just ui-check` → PASS.
- [ ] **Step 5: Commit** `feat(ui): right-click menus for people and streams`.

### Task 9: UI: shift-click picks several emoji

**Files:**
- Modify:
  - `components/EmojiPicker.svelte` and `components/EmojiPopover.svelte`.
  - `components/Composer.svelte`, `components/MessageItem.svelte` and `components/ReactionRow.svelte`.
- Test: `EmojiPicker.test.ts`, `Composer.test.ts` and `ReactionRow.test.ts`.

**Interfaces:**
- `onPick: (e: Std | CustomEmoji, opts: { keep: boolean }) => void` on both `EmojiPicker` and `EmojiPopover`.
  - `keep` is `true` for a cell click with `shiftKey` or Enter with `shiftKey`.
  - After a `keep` pick the picker focuses its search box.
- In `Composer`:
  - A `keep` pick inserts at the remembered selection, advances it, and leaves the picker open, without focusing the
    textarea.
  - A plain pick behaves as today.
- In `MessageItem`'s smile and `ReactionRow`'s add chip:
  - A `keep` pick toggles: add if not `me`, remove if `me`. The picker stays open.
  - A plain pick adds and closes, as today.
- "Frequently used" needs no change. Reactions count each add on the server. Composer emoji count when the message is
  sent: one use per distinct emoji per message, the existing server rule.

- [ ] **Step 1: Failing tests:**
  - `EmojiPicker.test.ts`:
    - `shift_click_reports_keep`.
    - `shift_enter_reports_keep_and_focuses_search`.
  - `Composer.test.ts`:
    - `shift_click_inserts_and_keeps_the_picker_open`: two shift-clicks put both emoji in the text in order, and the
      picker is still in the DOM.
    - `plain_click_inserts_and_closes`.
  - `ReactionRow.test.ts`:
    - `shift_click_toggles_and_keeps_the_picker_open`: shift-clicking an emoji you already reacted with invokes
      `remove_reaction`.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `just ui-check` → PASS.
- [ ] **Step 5: Commit** `feat(ui): shift-click to pick several emoji`.

### Task 10: UI: emoji freeze while live, and no animation while unfocused

**Files:**
- Modify:
  - `lib/animate.svelte.ts` and `lib/animate.test.ts`.
  - `routes/Shell.svelte`: the existing `.streaming` effect also sets `motion.streaming`. A new effect toggles
    `body.unfocused` from `windowFocus.focused`.
  - `app.css`: `.unfocused *` gets the same rule as `.streaming *`.
- Test: `components/AnimatedEmoji.test.ts`.

**Interfaces:**
- `shouldAnimate(s: { mode; focused; visible; hovered; reducedMotion; streaming: boolean })`: `streaming` true returns
  false.
- `motion.streaming: boolean`, default `false`.

- [ ] **Step 1: Failing tests:**
  - `animate.test.ts`: `streaming_shows_the_still_frame`.
  - `AnimatedEmoji.test.ts`: `going_live_freezes_and_stopping_resumes`. Set `motion.streaming = true` and the img src
    is the still; set it false and the src is the animated one.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `just ui-check` → PASS.
- [ ] **Step 5: Commit** `feat(ui): animated emoji freeze while you stream`.

### Task 11: UI: unsent messages that look real and survive a restart

**Files:**
- Create: `lib/unsent.ts` and `lib/unsent.test.ts`.
- Modify:
  - `lib/state.ts` and `lib/state.test.ts`.
  - `lib/store.svelte.ts`.
  - `components/MessageItem.svelte`, `components/TextChannel.svelte` and `components/TextChannel.test.ts`.

**Interfaces:**
- `Pending` gains `created_at: string`, set in `send`. A restored entry without it uses the restore time.
- Produces, in `state.ts`:
  - `removePending(s: AppState, channel: string, nonce: string): AppState`.
  - `restoreUnsent(s: AppState, saved: Record<string, Pending[]>): AppState`. Its rules:
    - Every entry comes back `failed`.
    - Channels not in `s.channels` are skipped.
    - Nonces already pending are skipped.
- Produces, in `unsent.ts`:
  - `loadUnsent(userId: string): Record<string, Pending[]>`: guarded parse; anything malformed gives `{}`.
  - `saveUnsent(userId: string, pending: AppState['pending'])`: empty channels are omitted.
- In `store.svelte.ts`:
  - The first `Ready` of the app session applies `restoreUnsent(app.state, loadUnsent(me.id))` after `applyReady`.
  - An `$effect.root` saves `app.state.pending` whenever its identity changes.
  - Restored messages are never resent automatically.
- `MessageItem` gets `pending?: Pending`, `onRetry?: () => void` and `onDiscard?: () => void`.
  - A pending message renders through the normal path (avatar, name, markdown, emoji), using a `Message` built by
    `pendingMessage(p: Pending, channelId: string, meId: string): Message` in `state.ts`.
  - The wrapper is at `opacity: .6`. No actions or reactions show.
  - Under it: `Sending…`, or `Failed to send` in red with **Retry** and **Discard**.
- `TextChannel` renders pending entries with `MessageItem`. **Discard** calls `removePending`. **Retry** keeps
  today's `retry`.

- [ ] **Step 1: Failing tests:**
  - `state.test.ts`:
    - `restore_marks_everything_failed`.
    - `restore_drops_gone_channels_and_known_nonces`.
    - `remove_pending_drops_one`.
    - `pending_message_renders_as_you`.
  - `unsent.test.ts`:
    - `round_trips_per_user`.
    - `malformed_storage_is_empty`.
    - `empty_channels_are_not_saved`.
  - `TextChannel.test.ts`:
    - `unsent_message_looks_like_a_message_faded`: name `alex`, the markdown rendered, and the class `unsent`.
    - `failed_shows_retry_and_discard`.
    - `discard_removes_it_and_retry_resends_with_a_new_nonce`.
    - `survives_reload`: seed localStorage, run the store's Ready path, and expect the message shown as failed with
      no `send_message` invoke.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `just ui-check` → PASS.
- [ ] **Step 5: Commit** `feat(ui): unsent messages render in full and survive a restart`.

### Task 12: UI: errors on the message that failed

**Files:**
- Modify:
  - `components/MessageItem.svelte`.
  - `components/TextChannel.svelte`: `onEdit`/`onDelete` pass the API promise through un-caught, and `onError` is no
    longer passed down for reactions.
  - `components/ReactionRow.svelte`.
- Test: `components/MessageItem.test.ts` (create).

**Interfaces:**
- `MessageItem`:
  - `onEdit: (content: string) => Promise<void>` and `onDelete: () => Promise<void>` now reject on failure.
  - `export const INLINE_ERROR_MS = 5_000` (module script).
- **A failed edit** keeps the textarea open with your text. It shows the error under it (`role="alert"`) until the
  next save or Esc.
- **A failed delete or reaction** sets `inlineError` on that message (`role="alert"`, small, red). It clears after
  `INLINE_ERROR_MS`.
- `ReactionRow` reports through the `onError` that `MessageItem` gives it.
- The channel-wide `error` line stays for history loading, renames and mutes.

- [ ] **Step 1: Failing tests** (fake timers):
  - `failed_edit_keeps_the_box_and_text_with_the_error`.
  - `failed_delete_shows_inline_error_that_hides_after_5s`.
  - `failed_reaction_shows_inline_error_not_the_banner`.
  - `successful_edit_closes_the_box`.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `just ui-check` → PASS.
- [ ] **Step 5: Commit** `feat(ui): failed edits, deletes and reactions show on the message`.

### Task 13: Final verification and the owner's checklist

- [ ] **Step 1: Run the checks.**
  - `just check` → green.
  - `cargo xwin clippy -p pulse-client --all-targets --target x86_64-pc-windows-msvc -- -D warnings` → clean.
  - `just voice-it` → PASS. Rerun once on the known cold-start flake and report it if it recurs.
  - Paste the tail of each output in the report.
- [ ] **Step 2: Linux smoke test.** Use `just dev-server` and `just dev-client` with two accounts (`alex`, `sam`):
  - The join shows `Joining…` greyed at once and the stages in the status block.
  - The stats panel opens, updates every second, and **Copy details** works.
  - Stream tiles show the Linux reason instead of Watch.
  - The menus and their keyboard handling work.
  - Shift-click emoji works.
  - An unsent message is shown, and is still there after a restart: stop the dev server, send, then restart the client.
  - Inline errors appear on the message.
  - Record what was checked.
- [ ] **Step 3: Write the manual checklist for the owner** into the final report (not a file).
  - **Windows, two PCs, real network:**
    - Join feel and the stage text on a slow join.
    - One click watches from the grid; a second click gives full view; Back and Esc return to the grid, still playing.
    - Data saver vs Auto in full view (diagnostics overlay resolution).
    - Leave to a text channel: the mini-preview plays with audio, drags to each corner, and the corner is remembered
      after restart. Clicking it returns; ✕ stops.
    - Toast Watch and popover Watch open full view.
    - **Join & watch** from another channel.
    - The own stream tile and its hide toggle.
    - Right-click menus on tiles and the channel list, including Shift+F10.
    - **Mute for me** survives a restart.
    - Compare the stats panel against Settings → Advanced diagnostics.
    - Emoji frozen while live.
    - An hour idle in a call with a preview open: memory flat in Task Manager, and no CPU while the window is
      unfocused with the stats panel open.
    - Messages: open channel at the bottom has no unread-dot flash.
  - **Linux:**
    - Join stages, the stats panel, the menus (no Watch or Go Live, reasons shown), shift-click emoji, unsent and failed
      messages.
- [ ] **Step 4:** Nothing to commit unless verification found fixes. Each fix gets its own TDD loop and commit.

---

## Self-review notes

- **Spec coverage:**
  - §2.1 → T6. §2.2 → T7. §2.2a → T5, T7. §2.3 → T8. §2.4 → T6. §2.5 → T5.
  - §3 → T1, T3. §4 → T2, T4.
  - §5.1 → T9. §5.2 → T10. §6 → T11, T12.
  - §7 tests are spread across the tasks; the long-session checks are in T4 and T10, and the manual run in T13.
  - Anti-goals: the stats only poll while the panel is open (T2 and T4); previews use the low layer (T5); nothing
    animates while unfocused (T10).
- **Builder choices (spec §8 and gaps):**
  - Join failures are only failures of the voice connection. A device that can't open at join still fails the join
    (Retry), as today. "Mic trouble" means `Mic` errors after the room is up.
  - Unsent messages are stored in their own key, because composer drafts are not persisted today.
  - Loss falls back to packets sent when remote-inbound lacks `packets_received`.
  - Loss boundaries: ≤ 1 % green, ≤ 5 % amber.
  - `Disconnected` and LiveKit-internal `Reconnecting` stay `State` events and are not duplicated as stages.
- **Not in this push:** multi-watch (cap stays 1), camera tiles, View profile, pop-out, attachments (rule only).
