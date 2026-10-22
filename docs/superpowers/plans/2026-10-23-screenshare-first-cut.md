# Screen share, first cut: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Windows streamer can Go Live in a server voice channel with H.264 hardware encode and stream audio, and the
other people in that voice channel can see who is live, watch in the main area and hear the stream audio, with enough
quiet diagnostics in the log that a two-PC test with a friend answers the open performance questions.

**Architecture:** The spec's two-room layout: the voice room `<channel>` is unchanged, and a stream room `<channel>~s`
holds `<uid>#v` (livekit-client in WebView2: screen video, publish and watch) and `<uid>#a` (Rust: stream audio from
WASAPI process loopback, published as `ScreenshareAudio`, and watched streams' audio played through the voice mixer).
The server keeps live-stream state in memory, hands out stream-room tickets only to voice members, broadcasts
`StreamStarted/Updated/Ended` and `ViewersChanged`, and follows the `~s` room through the LiveKit webhook.

**Tech Stack:** Rust (axum, sqlx/SQLite, livekit-api, livekit 0.7 Rust SDK, `wasapi` 0.26, `windows` 0.62), Tauri 2,
Svelte 5, `livekit-client` 2.22.4, vitest, LiveKit server v1.13.

**Spec:** `docs/superpowers/specs/2026-10-15-screenshare-design.md` (read it; this plan builds a first cut of §2, §3,
§4 and §5's voice-member rules). The throwaway spike lives on branch `screenshare-spike` (worktree
`~/Projects/pulse-app-spike`); this plan names the files to lift from it. Read them with
`git -C ~/Projects/pulse-app show screenshare-spike:<path>`; never edit that branch.

## Scope of this cut

**In:** the stream room and its tickets (voice members only), stream state and gateway events, the `~s` webhook, the
strict-H.264 codec list shipped in `livekit.yaml`, voice-ticket hardening (spec §2 lists it with the server phase and it
is one line), Go Live (picker, dialog, H.264 simulcast publish, card, Stop), stream audio (process loopback that
excludes every Pulse process, publish, mute, playback through the mixer), LIVE badge, popover with preview and Watch,
the 4 s toast, watching in the main area with Auto / Data saver and a stream-volume slider, quiet diagnostics.

**Out (next round, say so in review):** ghost mode and everything in spec §5.3–5.4 about ghosts, pop-out (`#v2`),
auto step-down (§6.2), health dots and slowdown watch (§6.3–6.4), upscaling, minimise pause, Linux viewing, DM/group
streams, the upload test and bitrate cap (§3.1.4), live settings changes without restarting (§3.3; this cut changes
settings by Stop + Go Live), side-by-side multi-stream tiles (one watched stream at a time), Fullscreen and the
collapsed chat strip, the renderer-crash and reconnect-loop rows of §7, A/V sync correction (§2; logged only).

## Global Constraints

- PUBLIC repo: no IPs, secrets, emails or real people's names in code, comments, commits or logs ("the owner", "a
  friend"). The domain `pulsechat.co.za` is already public in the repo and may appear.
- Rooms: voice `<channel_id>`, stream `<channel_id>~s`. Identities: `<uid>#v` (JS), `<uid>#a` (Rust). No other
  stream-room identity exists in this cut.
- Every stream-room ticket: room `<channel_id>~s`, TTL 10 min, `can_publish_data: false`, `can_subscribe: true`.
  `#v` → `can_publish_sources: ["screen_share"]`; `#a` → `["screen_share_audio"]`.
- Voice tickets: `can_publish_sources: ["microphone"]` (spec §2 hardening).
- Bitrates (spec §3.2), kbps: 720p 1500/3500/5500, 1080p 2500/5000/8000, 1440p 4000/7500/12000 for 15/30/60 fps. Low
  simulcast layer always on: 640×360, 30 fps, 500 kbps.
- 30/60 fps: `contentHint 'motion'`, `degradationPreference 'maintain-framerate'`; 15 fps: `'detail'`,
  `'maintain-resolution'`. Settings go in `screenShareEncoding` / `screenShareSimulcastLayers`, codec `h264`.
- Viewer: `adaptiveStream: false`, `dynacast: true`, `autoSubscribe: false`, SDP answers carry `rtx-time=300`.
- First-time dialog defaults: 720p, 60 fps, stream audio on. Last choice remembered per device.
- Picker: window grows to at least 640 px tall while the picker is open, then restores.
- Toast: "<name> is live" with **Watch**, only to people in that voice channel, dismissed after 4 s.
- Stream end reasons: `stopped`, `left_voice`, `crashed`, `timed_out` (this cut emits all but `crashed`).
- Streamer's `#v` gone from `~s` → status `reconnecting`; no `#v` back within 60 s → `StreamEnded{timed_out}`.
- Previews: 480×270 WebP, quality 0.7, every 15 s while live; server keeps the latest per stream in memory, max 100 KB.
- Log lines for this feature use the tracing target `share` and start `share: ` followed by one JSON object.
- No debug panel, no `VITE_SPIKE`, no `spike` cargo feature, no beep/tone/test-pattern code in this branch.
- `livekit-client` exact version `2.22.4` (validated in the spike; do not float it).

## Review Focus

1. **A still screen** (desktop, a paused game, a document) drops capture to ~2 fps and WebRTC counts freezes. Nothing
   may call that a problem: `fpsNote` returns `'still'` and the overlay shows it neutrally (Task 12 test
   `still_screen_is_not_a_problem`).
2. **Two Pulse instances on one PC** (two profiles, or the viewer instance next to the streamer): stream audio must not
   capture the other instance's playback, or it feeds back (seen in the spike). Task 8 test
   `another_pulse_switches_to_per_session_include` and Task 9's rescan.
3. **Stream ticket requested a moment after joining voice**, before LiveKit's join webhook reaches the server: the
   server answers 403. The client retries 403 three times, 500 ms apart, before showing an error (Task 6 test
   `stream_ticket_retries_a_403_while_voice_join_lands`).
4. **The streamer leaves voice or switches channel while live**, or closes the shared window: the stream ends for
   everyone, the `#a` capture thread stops, and nobody keeps a dangling LIVE badge (Task 3 test
   `leaving_voice_ends_your_stream`, Task 10 test `leave_voice_closes_the_stream_connection`, Task 13's track `ended` handler and `onVoiceChanged`).
5. **The server restarts while someone is live**: streams are in memory, so they vanish. On the next `Ready` a live
   client re-announces itself and the badge returns within seconds (Task 13 test `reannounces_when_ready_lacks_my_stream`).

---

## File map

Server and protocol:
- `protocol/src/rest.rs`, `protocol/src/gateway.rs`, `protocol/src/lib.rs`: stream types, events, `Ready.streams`.
- `server/src/streams/mod.rs` (new): `pub mod state; pub mod routes; pub mod webhook;`.
- `server/src/streams/state.rs` (new): `StreamState`, pure in-memory rules.
- `server/src/streams/routes.rs` (new): tickets, live/stop, watch/unwatch, previews.
- `server/src/streams/webhook.rs` (new): `~s` room handling, called from `voice/routes.rs` `webhook`.
- `server/src/voice/routes.rs`: microphone-only voice tickets; route `~s` webhooks; end streams on voice leave.
- `server/src/state.rs`, `server/src/config.rs`, `server/src/testing.rs`, `server/src/lib.rs`, `server/src/gateway/socket.rs`.
- `server/tests/streams.rs` (new).
- `deploy/livekit.yaml`, `docker/livekit.yaml`: the codec list. `deploy/README.md`: one line on restarting LiveKit.

Client Rust (`client/src-tauri/`):
- `src/api.rs`: stream REST calls.
- `src/stream/mod.rs` (new): Tauri commands, `StreamAudio` manager state.
- `src/stream/plan.rs` (new): pure capture plan.
- `src/stream/summer.rs` (new): pure multi-capture summing.
- `src/stream/procs.rs` (new, Windows bodies, stubs elsewhere): Pulse pids, audio-session pids, window pid.
- `src/stream/loopback.rs` (new, lifted from the spike): WASAPI process-loopback capture.
- `src/stream/audio.rs` (new): the `#a` room: publish, mute, watch subscriptions into the mixer.
- `src/voice/mod.rs`: `mixer_input()`; `src/lib.rs`, `src/commands.rs`: registration, leave hook.
- `tests/stream_it.rs` (new): real-LiveKit isolation tests.
- `Cargo.toml`: `wasapi` as a Windows dependency; `windows` features.
- `tauri.conf.json` (CSP), `tauri.dev.conf.json` (new, dev CSP overlay), `justfile` `dev-client`.

Client UI (`client/ui/src/`):
- `lib/state.ts`, `lib/selectors.ts`: `streams` in `AppState`.
- `lib/stream/encodings.ts`, `sdp.ts`, `stats.ts`, `prefs.ts`, `platform.ts` (pure, tested).
- `lib/stream/room.ts`: `StreamRoom` (the `#v` connection). `lib/stream/diag.svelte.ts`: sampler + overlay state.
- `lib/stream/stream.svelte.ts`: the store and `streamApi`.
- `components/GoLiveDialog.svelte`, `LiveCard.svelte`, `LiveBadge.svelte`, `LivePopover.svelte`, `StreamToast.svelte`,
  `StreamViewer.svelte`, `StreamDiagOverlay.svelte`, `AdvancedSettings.svelte`; edits to `VoicePanel.svelte`,
  `ChannelList.svelte`, `Settings.svelte`, `routes/Shell.svelte`, `app.css`.

## Spike pieces: lift vs rewrite

| Spike path (branch `screenshare-spike`) | Here | What to change |
|---|---|---|
| `.github/workflows/release-server.yml` (commit `c1405a2`) | cherry-pick as is | Task 0 |
| `server/src/spike.rs` | rewrite as `server/src/streams/routes.rs` `ticket` | no env gate, no `v2`, typed role, voice-member rule kept |
| `server/tests/spike.rs` | rewrite into `server/tests/streams.rs` | keep the `TokenVerifier` claim checks |
| `deploy/livekit.codecs.yaml` | fold into `deploy/livekit.yaml` and `docker/livekit.yaml` | drop the file and the `spike-codecs`/`spike-server` recipes; no `PULSE_SPIKE` in `compose.yml` |
| `client/src-tauri/src/spike/loopback.rs` | lift `wasapi_capture`, `process_alive`, the `NativeAudioSource` publish, the `Sub` drop guard and the subscribe loop into `stream/loopback.rs` and `stream/audio.rs` | drop the tone, the onset detector and `spike_beep_watch`; capture target comes from `plan.rs` |
| `client/src-tauri/src/spike/mod.rs` `win::processes`, `win::window_pid` | lift into `stream/procs.rs` | drop `os_stats`, `webview_args`, `spike_log` |
| `client/src-tauri/src/spike/onset.rs` | not carried | |
| `client/src-tauri/src/voice/mod.rs` `spike_mixer` | rename `mixer_input`, return the generation too | |
| `client/src-tauri/src/lib.rs` `handlers!` macro, `ProcessFailed` hook, WebView2 args | not carried (plain `generate_handler!` gets the new commands) | |
| `client/src-tauri/tauri.spike.conf.json` | not carried; CSP narrowed in `tauri.conf.json` | Task 11 |
| `client/ui/src/spike/encodings.ts` + test | lift to `lib/stream/encodings.ts` | add low layer + `resolutionsFor` |
| `client/ui/src/spike/sdp.ts` + test | lift to `lib/stream/sdp.ts` | add idempotent `installRtxTime()` (always on) |
| `client/ui/src/spike/rooms.ts` | rewrite as `lib/stream/room.ts` | one connection publishes and watches; `pick` without size constraints |
| `client/ui/src/spike/frames.ts` + test | lift to `lib/stream/frames.ts` | unchanged |
| `client/ui/src/spike/sampler.ts`, `SpikePanel.svelte` `sampleOut`/`sampleIn` | rewrite as `lib/stream/stats.ts` (pure) + `diag.svelte.ts` | per-interval deltas (the spike's encode ms was cumulative) |
| `SpikePanel.svelte`, `SpikePopout.svelte`, `log.ts`, `testpattern.html`, `App.svelte`/`main.ts` spike hooks | not carried | |

---

### Task 0: Branch, CI fix

**Files:** `.github/workflows/release-server.yml`

- [ ] **Step 1:** Worktree (already created by the planner if this file exists in it):
  `git -C ~/Projects/pulse-app worktree add ~/Projects/pulse-app-share -b screenshare-v1 origin/main`
- [ ] **Step 2:** `git -C ~/Projects/pulse-app-share cherry-pick c1405a2`. Expected: applies cleanly (only
  `release-server.yml`; manual runs off `main` publish `:<sha7>` only).
- [ ] **Step 3:** `just check` → PASS (baseline).
- [ ] **Step 4:** Push the branch.

### Task 1: Protocol types

**Files:** Modify `protocol/src/rest.rs`, `protocol/src/gateway.rs`, `protocol/src/lib.rs`; regenerate
`client/ui/src/lib/protocol/*.ts`.

**Interfaces — Produces** (all `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]`, snake_case enums):
- `rest::StreamSettings { height: u16, fps: u8, audio: bool }` (height ∈ {720,1080,1440}, fps ∈ {15,30,60})
- `rest::StreamStatus { Live, Reconnecting }`
- `rest::StreamInfo { channel_id: ChannelId, user_id: UserId, settings: StreamSettings, status: StreamStatus, viewers: Vec<UserId>, started_at: String }`
- `rest::StreamEndReason { Stopped, LeftVoice, Crashed, TimedOut }`
- `rest::StreamRole { V, A }` (serialised `"v"`, `"a"`)
- `rest::StreamTicketRequest { role: StreamRole }`, `rest::GoLiveRequest { settings: StreamSettings }`,
  `rest::WatchRequest { streamer: UserId }`
- `gateway::Event::StreamStarted { stream: StreamInfo }`, `StreamUpdated { stream: StreamInfo }`,
  `StreamEnded { channel_id, user_id, reason: StreamEndReason }`, `ViewersChanged { channel_id, user_id, viewers: Vec<UserId> }`
  — all four return `Some(channel_id)` from `Event::channel_id()` and `None` from `only_for()`.
- `gateway::Ready.streams: Vec<StreamInfo>` with `#[serde(default)]`.
- `PROTOCOL_VERSION` stays 1 (old clients ignore unknown events: `client/src-tauri/src/gateway.rs:499`).

- [ ] **Step 1: Failing tests** in `protocol/src/gateway.rs` `mod tests`:
  - `ready_without_streams_parses` — a `Ready` JSON with no `streams` key deserialises to `streams == []`.
  - `stream_events_are_channel_scoped` — each of the four events: `channel_id() == Some(c)`, `only_for() == None`.
  - `stream_role_is_a_letter` — `serde_json::to_string(&StreamRole::V) == "\"v\""`.
- [ ] **Step 2:** `cargo test -p pulse-protocol` → FAIL (types missing).
- [ ] **Step 3:** Add the types; add every new rest type to `export_all`.
- [ ] **Step 4:** `cargo test -p pulse-protocol && just gen-types` → PASS; new `.ts` files appear.
- [ ] **Step 5:** Fix compile errors elsewhere (`Ready { .. }` constructions in `server/src/gateway/socket.rs` and
  client tests get `streams: vec![]` for now). `cargo check --workspace` → OK.
- [ ] **Step 6:** Commit `feat(protocol): stream types, events and Ready.streams`.

### Task 2: Server stream state (pure)

**Files:** Create `server/src/streams/mod.rs`, `server/src/streams/state.rs`; modify `server/src/lib.rs`,
`server/src/state.rs` (`AppState.streams: StreamState`), `server/src/testing.rs`.

**Interfaces — Produces** (`StreamState: Clone + Default`, `Arc<Mutex<..>>` inside like `VoiceState`):
- `fn start(&self, channel: ChannelId, user: UserId, settings: StreamSettings, now: String) -> Started` where
  `enum Started { New(StreamInfo), Updated(StreamInfo) }` (re-POST while live = update, keeps viewers and `started_at`,
  sets status `Live`).
- `fn end(&self, channel: ChannelId, user: UserId) -> Option<StreamInfo>`
- `fn end_any_for(&self, user: UserId) -> Vec<StreamInfo>` (all streams by `user`, any channel)
- `fn set_status(&self, channel: ChannelId, user: UserId, status: StreamStatus) -> Option<StreamInfo>` (None if unchanged or absent)
- `fn add_viewer(&self, channel, streamer, viewer) -> Option<Vec<UserId>>` (None if not live or unchanged)
- `fn remove_viewer(&self, channel, streamer, viewer) -> Option<Vec<UserId>>`
- `fn remove_viewer_everywhere(&self, channel: ChannelId, viewer: UserId) -> Vec<(UserId, Vec<UserId>)>`
- `fn get(&self, channel, user) -> Option<StreamInfo>`, `fn all(&self) -> Vec<StreamInfo>`
- `fn set_preview(&self, channel, user, bytes: Bytes) -> bool` (false if not live), `fn preview(&self, channel, user) -> Option<(Bytes, String /*etag*/)>`; etag = first 16 hex of a hash of the bytes. Ending a stream drops its preview.
- `fn bump_epoch(&self, channel, user) -> u64` and `fn epoch(&self, channel, user) -> Option<u64>` (used by Task 4's timer to tell whether the stream it is waiting on is still the same reconnect episode).

- [ ] **Step 1: Failing tests** in `state.rs` `mod tests`:
  `start_twice_is_an_update_that_keeps_viewers`, `end_drops_preview`, `viewer_add_is_idempotent`,
  `remove_viewer_everywhere_reports_each_stream`, `set_status_unchanged_is_none`, `end_any_for_covers_all_channels`.
- [ ] **Step 2:** `cargo test -p pulse-server streams::state` → FAIL.
- [ ] **Step 3:** Implement. Keep `AppState.streams` constructed wherever `VoiceState::default()` is (`main.rs`, `testing.rs`).
- [ ] **Step 4:** Tests PASS.
- [ ] **Step 5:** Commit `feat(server): in-memory stream state`.

### Task 3: Server routes: tickets, go live, stop, watch; voice hardening; Ready

**Files:** Create `server/src/streams/routes.rs`, `server/tests/streams.rs`; modify `server/src/lib.rs` (merge
router), `server/src/voice/routes.rs`, `server/src/gateway/socket.rs` (`build_ready` fills `streams` from
`s.streams.all()` filtered to channels the user may see — reuse the same visibility filter `voice` uses).

**Interfaces — Produces** (all bearer-authed, JSON):
- `POST /streams/{channel}/ticket` body `StreamTicketRequest` → `VoiceTokenResponse` (url = `cfg.livekit_url`).
- `POST /streams/{channel}/live` body `GoLiveRequest` → `StreamInfo`; publishes `StreamStarted` (new) or `StreamUpdated` (re-POST).
- `DELETE /streams/{channel}/live` → 204; publishes `StreamEnded{stopped}`; 404 if not live.
- `POST /streams/{channel}/watch` body `WatchRequest` → 204; publishes `ViewersChanged` if changed.
- `POST /streams/{channel}/unwatch` body `WatchRequest` → 204; same.
- Shared guard `async fn require_voice_member(s: &AppState, me: UserId, ch: ChannelId) -> AppResult<Channel>`:
  `channel_for` → 404/403 as today; `ch.server_id` must be `Some` and `kind == Voice` (else 400 "streaming is only in
  server voice channels"); server membership (`is_server_member`) else 403; `s.voice.members(ch).contains(&me)` else 403.
- Validation: `settings.height ∈ {720,1080,1440}`, `fps ∈ {15,30,60}` else 400. Watch: streamer live in that channel
  else 404; `streamer == me` → 400.
- `voice/routes.rs` `token`: add `can_publish_sources: vec!["microphone".into()]`.
- `voice/routes.rs` `webhook`, voice-room `participant_left` with `Left::Removed`: `s.streams.end_any_for(user)` →
  `StreamEnded{left_voice}` each; `remove_viewer_everywhere(channel, user)` → `ViewersChanged` each.
- Logging: `tracing::info!(user, channel, role, "stream ticket")`, `"stream live"`, `"stream ended"` with reason.

- [ ] **Step 1: Failing tests** in `server/tests/streams.rs` (use `testing::spawn`, `register`, `create_server`, the
  spike test's `lounge` helper, `app.voice.join(...)` to stand in for the webhook, and `TokenVerifier` for claims):
  - `tickets_name_only_the_stream_room` — for `v` and `a`: `claims.video.room == "{ch}~s"`, `claims.sub == "{uid}#v"/"#a"`,
    sources `["screen_share"]` / `["screen_share_audio"]`, `can_publish_data == Some(false)`.
  - `tickets_need_you_in_that_voice_channel` — not in voice → 403; in voice elsewhere → 403.
  - `tickets_refuse_text_and_dm_channels` — text channel → 400; DM → 400.
  - `voice_tickets_publish_microphone_only` — `/voice/{ch}/token` claims `can_publish_sources == ["microphone"]`.
  - `go_live_broadcasts_stream_started` — a second user's gateway socket (existing helpers in `server/tests/gateway.rs`)
    receives `StreamStarted` with the settings.
  - `go_live_twice_is_stream_updated` and `go_live_rejects_odd_settings` (height 900 → 400, fps 45 → 400).
  - `stop_broadcasts_stopped_and_404s_when_not_live`.
  - `watch_adds_a_viewer_and_unwatch_removes_it` — `ViewersChanged` both times; watching yourself → 400; watching
    someone not live → 404; watching when not in voice → 403.
  - `leaving_voice_ends_your_stream` — post a signed voice-room `participant_left` webhook (build it the way
    `server/tests/voice.rs` does) → `StreamEnded{left_voice}`; and a viewer who leaves voice is removed (`ViewersChanged`).
  - `ready_lists_live_streams` — connect a fresh gateway socket after go-live: `ready.streams.len() == 1`.
- [ ] **Step 2:** `cargo test -p pulse-server --test streams` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** `cargo test -p pulse-server` → all PASS (existing voice tests must still pass with the hardening).
- [ ] **Step 5:** Commit `feat(server): stream tickets, go live/stop/watch, microphone-only voice tickets`.

### Task 4: Server: the `~s` room webhook

**Files:** Create `server/src/streams/webhook.rs`; modify `server/src/voice/routes.rs` (`webhook`: before parsing the
room as a `ChannelId`, `if let Some(ch) = room.name.strip_suffix("~s")` → `streams::webhook::handle(&s, ch, &ev)` and
return), `server/src/config.rs` + `testing.rs` (`stream_media_timeout: Duration`, prod 60 s, test 300 ms).

**Interfaces — Produces:**
- `pub fn parse_identity(identity: &str) -> Option<(UserId, StreamRole)>` (`"<ulid>#v"` / `"<ulid>#a"`, anything else None).
- `pub async fn handle(s: &AppState, channel: &str, ev: &WebhookEvent) -> AppResult<()>`. Rules (only `#v` matters for
  state; `#a` events are logged and ignored):
  - `participant_left`, identity is a live streamer → `set_status(Reconnecting)` → `StreamUpdated`; `bump_epoch`;
    spawn a task: sleep `cfg.stream_media_timeout`, then if `epoch` unchanged and still `Reconnecting` → `end` →
    `StreamEnded{timed_out}`.
  - `participant_joined`, identity is a streamer in `Reconnecting` → `set_status(Live)` → `StreamUpdated`; `bump_epoch`.
  - `participant_left`, identity is anyone → `remove_viewer_everywhere(channel, user)` → `ViewersChanged` each.
  - Never touches `voice_sessions` or `VoiceState`.

- [ ] **Step 1: Failing tests** (`server/tests/streams.rs`, signed webhooks as in `server/tests/voice.rs`):
  - `parse_identity_accepts_v_and_a_only` (unit, in `webhook.rs`): `"X#v"` ok, `"X#a"` ok, `"X"`, `"X#v2"`, `"g_x#v"` None.
  - `streamer_drop_marks_reconnecting_then_times_out` — `StreamUpdated{status: reconnecting}`, then after ~300 ms
    `StreamEnded{timed_out}`.
  - `streamer_back_in_time_stays_live` — left then joined within the timeout → `StreamUpdated{live}`, no `StreamEnded`
    after 600 ms.
  - `stream_room_webhooks_never_touch_voice` — `participant_joined` for `{uid}#v` in `{ch}~s` → no `VoiceJoined`, no
    row in `voice_sessions`, `app.voice.members(ch)` unchanged.
  - `viewer_leaving_the_stream_room_is_unlisted`.
- [ ] **Step 2:** FAIL. **Step 3:** Implement. **Step 4:** `cargo test -p pulse-server` PASS.
- [ ] **Step 5:** Commit `feat(server): follow the stream room through the LiveKit webhook`.

### Task 5: Server: previews

**Files:** Modify `server/src/streams/routes.rs`, `server/tests/streams.rs`.

**Interfaces — Produces:**
- `PUT /streams/{channel}/preview`, body raw bytes, `Content-Type: image/webp` (else 415), ≤ 100 KB (else 413; set a
  `DefaultBodyLimit` of 100 KB on this route), caller must be live in that channel (else 404) → 204.
- `GET /streams/{channel}/{user}/preview` → 200 `image/webp` with `ETag`, 304 on matching `If-None-Match`, 404 if no
  image yet or not live. The caller needs the channel visible (`channel_for`) and server membership, but not voice: the popover shows to
  anyone who can see the channel, even though only voice members can watch in this cut.

- [ ] **Step 1: Failing tests:** `preview_round_trips_with_etag`, `preview_needs_you_live`, `preview_rejects_big_or_non_webp`,
  `preview_is_gone_after_stop`, `preview_needs_server_membership`.
- [ ] **Step 2:** FAIL. **Step 3:** Implement. **Step 4:** PASS.
- [ ] **Step 5:** Commit `feat(server): stream preview images`.

### Task 6: Client Rust: API calls and plain commands

**Files:** Modify `client/src-tauri/src/api.rs`; create `client/src-tauri/src/stream/mod.rs`; modify
`client/src-tauri/src/lib.rs` (`pub mod stream;`, register commands).

**Interfaces — Produces:**
- `Api::stream_ticket(&self, token, channel: ChannelId, role: StreamRole) -> Result<VoiceTokenResponse, ApiError>`
- `Api::go_live(&self, token, channel, settings: StreamSettings) -> Result<StreamInfo, ApiError>`
- `Api::stop_live(&self, token, channel) -> Result<(), ApiError>`
- `Api::watch_stream / unwatch_stream(&self, token, channel, streamer: UserId) -> Result<(), ApiError>`
- `Api::put_preview(&self, token, channel, webp: Vec<u8>) -> Result<(), ApiError>`
- `Api::get_preview(&self, token, channel, user) -> Result<Option<Vec<u8>>, ApiError>` (404 → `None`)
- `stream::ticket_with_retry(api, token, channel, role) -> Result<VoiceTokenResponse, ApiError>`: on `Forbidden`, retry
  3 times 500 ms apart (Review Focus 3).
- Tauri commands (camelCase args from JS):
  `stream_ticket(channel_id, role) -> VoiceTokenResponse`, `stream_go_live(channel_id, settings) -> StreamInfo`,
  `stream_stop(channel_id)`, `stream_watch(channel_id, streamer)`, `stream_unwatch(channel_id, streamer)`,
  `stream_preview_put(channel_id, webp_base64: String)`, `stream_preview_get(channel_id, user_id) -> Option<String>`
  (a `data:image/webp;base64,...` URL; `img-src data:` is already allowed), `stream_log(line: String)` (logs
  `tracing::info!(target: "share", "share: {line}")`, line capped at 4 KB),
  `stream_picker_window(open: bool)` (on open: remember the inner size and grow to at least 640 logical px tall; on
  close: restore; done in Rust so no new window capability is needed).

- [ ] **Step 1: Failing tests** in `api.rs` `mod tests` against `pulse_server::testing` (pattern: `voice_token_after_join`):
  `stream_ticket_after_join`, `go_live_then_stop_round_trip`, `preview_put_then_get`,
  `stream_ticket_retries_a_403_while_voice_join_lands` (spawn a task that calls `app.voice.join` after 700 ms; the
  call succeeds).
- [ ] **Step 2:** `cargo test -p pulse-client --lib api::` → FAIL. **Step 3:** Implement. **Step 4:** PASS.
- [ ] **Step 5:** Commit `feat(client): stream REST calls and commands`.

### Task 7: LiveKit codec config and the isolation integration test

**Files:** Modify `deploy/livekit.yaml`, `docker/livekit.yaml`, `deploy/README.md`; create
`client/src-tauri/tests/stream_it.rs`; modify `justfile` (`voice-it` also runs `--test stream_it`).

- [ ] **Step 1:** Append to both YAML files, verbatim from the spike's `deploy/livekit.codecs.yaml` (validated
  2026-10-22), with a comment that the list is LiveKit's full default plus two strict H.264 entries, must never be
  trimmed (voice RED, video RTX), and why (Chromium skips NVIDIA's encoder for Constrained Baseline, crbug 1088650):
  ```yaml
  room:
    enabled_codecs:
      - mime: audio/PCMU
      - mime: audio/PCMA
      - mime: audio/opus
      - mime: audio/red
      - mime: video/VP8
      - mime: video/H264
      - mime: video/H264
        fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=4d001f"
      - mime: video/H264
        fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f"
      - mime: video/VP9
      - mime: video/AV1
      - mime: video/H265
      - mime: video/rtx
  ```
- [ ] **Step 2:** `just deploy-check` → OK; `docker compose restart livekit && docker compose logs --tail 20 livekit`
  shows no config error.
- [ ] **Step 3: Write the integration tests** in `stream_it.rs` (`#[ignore]`, dev LiveKit, server from
  `pulse_server::testing` with `livekit_url = ws://127.0.0.1:7880` and the dev secret, as `voice_it.rs` does):
  - `stream_room_never_sees_voice_tracks` — a bot publishes a tone into the voice room (`bot_publishes_tone`, lifted
    from `voice_it.rs` but without `--exit-after-publish` racing: keep it publishing ~5 s); a Rust `Room` connected
    with alex's `#a` stream ticket (`auto_subscribe: true`) sees zero remote participants and no `TrackSubscribed`
    for 3 s. This is the spec's most important test.
  - `v_ticket_cannot_publish_audio` — a Rust `Room` on a `#v` ticket: `publish_track` of an audio track with source
    `Microphone` fails, and with `ScreenshareAudio` fails.
  - `a_ticket_publishes_screenshare_audio` — the same with an `#a` ticket and `ScreenshareAudio` succeeds.
- [ ] **Step 4:** `just voice-it` → the three new tests PASS (plus the existing ones).
- [ ] **Step 5:** `deploy/README.md`: under the `livekit.yaml` note, "the codec list needs a LiveKit restart to apply;
  `just deploy` copies the file but does not restart LiveKit".
- [ ] **Step 6:** Commit `feat(deploy): strict H.264 codec list; stream-room isolation tests`.

Note for the owner: once this branch merges, `just deploy` from `main` keeps copying the codec list. Before the merge,
deploying `main` would copy the old `livekit.yaml` back; it only takes effect on the next LiveKit restart.

### Task 8: Rust: capture plan and summing (pure)

**Files:** Create `client/src-tauri/src/stream/plan.rs`, `client/src-tauri/src/stream/summer.rs`.

**Interfaces — Produces:**
- `#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub enum Capture { Include(u32), Exclude(u32) }`
- `pub struct PlanInput<'a> { pub own_pid: u32, pub pulse_pids: &'a [u32] /* every Pulse process, own included, incl. WebView2 children */, pub session_pids: &'a [u32] /* processes with an active render session */, pub window_pid: Option<u32> }`
- `pub fn capture_plan(i: &PlanInput) -> Vec<Capture>`:
  1. `window_pid` is `Some(p)` and `p` is not a Pulse pid → `[Include(p)]` (window share, spec §3.3).
  2. else, the only Pulse root is `own_pid` (no other Pulse instance) → `[Exclude(own_pid)]` (what the spike validated).
  3. else → `Include(p)` for each `session_pids` entry not in `pulse_pids`, deduplicated, sorted (another Pulse
     instance exists: EXCLUDE takes one process tree, so exclude-own would capture the other instance's playback).
- `pub fn plan_diff(old: &[Capture], new: &[Capture]) -> (Vec<Capture> /*start*/, Vec<Capture> /*stop*/)`
- `pub struct Summer` — sums 10 ms stereo i16 chunks from N captures into one stream:
  `fn new(lag_cap_ms: u32) -> Self` (use 30), `fn push(&mut self, id: Capture, chunk: &[i16])`,
  `fn remove(&mut self, id: Capture)`, `fn pop(&mut self) -> Option<Vec<i16>>` (a 960-sample chunk once the fullest
  capture has one; a capture behind by more than `lag_cap_ms` contributes zeros for its gap; saturating add).

- [ ] **Step 1: Failing tests:**
  - `window_share_includes_the_window_process`
  - `window_owned_by_pulse_falls_back_to_screen_rules`
  - `alone_excludes_own_tree`
  - `another_pulse_switches_to_per_session_include` (own 10, other Pulse 20 and its child 21, sessions [21, 30, 31,
    30] → `[Include(30), Include(31)]`)
  - `another_pulse_with_no_other_audio_is_an_empty_plan` (no capture, the stream audio track sends silence)
  - `diff_starts_new_and_stops_gone`
  - `single_capture_passes_through_unchanged`, `two_captures_sum_and_saturate`, `lagging_capture_is_zero_filled_after_cap`,
    `removed_capture_stops_contributing`
- [ ] **Step 2:** `cargo test -p pulse-client --lib stream::` → FAIL. **Step 3:** Implement. **Step 4:** PASS.
- [ ] **Step 5:** Commit `feat(client): stream-audio capture plan that excludes every Pulse process`.

### Task 9: Rust: Windows process discovery and loopback capture

**Files:** Create `client/src-tauri/src/stream/procs.rs`, `client/src-tauri/src/stream/loopback.rs`; modify
`client/src-tauri/Cargo.toml`:
- `[target.'cfg(windows)'.dependencies] wasapi = "0.26"` (non-optional; 0.26.0 matches windows/windows-core 0.62.2 in
  `Cargo.lock`).
- `windows` features add `Win32_System_ProcessStatus`, `Win32_System_Diagnostics_ToolHelp`, `Win32_Media_Audio`,
  `Win32_System_Com`.

**Interfaces — Produces:**
- `procs::pulse_pids() -> Vec<u32>`: every process whose exe name equals ours (`std::env::current_exe` file name) plus
  their descendant trees (WebView2 children), from the spike's `win::processes()`.
- `procs::audio_session_pids() -> Vec<u32>`: pids of render sessions on the default render endpoint whose state is
  Active (`IAudioSessionManager2::GetSessionEnumerator` → `IAudioSessionControl2::GetProcessId`; skip pid 0, the system
  sounds session).
- `procs::window_pid(title: &str) -> Option<u32>`: the spike's `win::window_pid`.
- All three return empty / `None` off Windows.
- `loopback::Engine::start(plan: Vec<Capture>, sink: impl FnMut(Vec<i16>) + Send + 'static) -> Engine`: one capture
  thread per `Capture` (the spike's `wasapi_capture`, `include` flag from the variant, 48 kHz stereo i16,
  `EventsShared { autoconvert: true, buffer_duration_hns: 200_000 }`), feeding a shared `Summer`; a sender thread pops
  10 ms chunks into `sink`. `Engine::apply(&self, plan: Vec<Capture>)` uses `plan_diff`. `Engine::stop(self)` joins
  every thread. An `Include` target that exits ends its own thread only (log `capture_ended`).
- Every start/stop/error logs one `share:` line: `{"ev":"capture_started","plan":[...]}`,
  `{"ev":"capture_ended","capture":..,"error":..}`, `{"ev":"plan_changed","start":[..],"stop":[..]}`.
- The 2 s rescan lives in `audio.rs` (Task 10): `capture_plan(&PlanInput { .. })` from fresh `procs` calls, then
  `engine.apply(..)`; skipped entirely when the plan came from a window pid.

- [ ] **Step 1:** Lift the spike tests that apply: `process_loopback_says_it_is_windows_only_elsewhere` (non-Windows)
  and `capture_ends_cleanly_when_the_process_exits` (Windows). Add `pulse_pids_includes_ourselves` (Windows:
  contains `std::process::id()`) and `off_windows_discovery_is_empty` (non-Windows).
- [ ] **Step 2:** FAIL. **Step 3:** Implement. **Step 4:** `cargo test -p pulse-client --lib stream::` PASS on Linux;
  `cargo clippy --workspace --all-targets -- -D warnings` clean; on Windows the release workflow (Task 17) runs the
  Windows-only tests.
- [ ] **Step 5:** Commit `feat(client): WASAPI process-loopback capture engine`.

### Task 10: Rust: the `#a` connection (publish, mute, watch, lifecycle)

**Files:** Create `client/src-tauri/src/stream/audio.rs`; modify `stream/mod.rs` (commands), `voice/mod.rs`
(`spike_mixer` equivalent), `commands.rs` (`leave_voice` and `join_voice` call `StreamAudio::leave`), `lib.rs`
(`app.manage(stream::audio::StreamAudio::default())`).

**Interfaces:**
- Consumes: `capture_plan`, `Engine` (Tasks 8–9), `ticket_with_retry` (Task 6), `MixerInput::{add_source, push_in,
  remove_source, set_gain}`, `SourceKind::Stream`.
- Produces:
  - `VoiceManager::mixer_input(&self) -> Option<(Arc<MixerInput>, u64 /*generation*/)>`
  - `StreamAudio` (one `tokio::sync::Mutex` serialises every operation, as the spike's third field did):
    - `async fn ensure_connected(&self, api, token, channel) -> Result<(), String>`: connects `#a` once per channel
      (`RoomOptions { auto_subscribe: false, ..Default::default() }`), spawns the event task.
    - `async fn start_publishing(&self, window_title: Option<String>) -> Result<(), String>`: `NativeAudioSource::new(
      AudioSourceOptions::default(), 48_000, 2, 100)`, publish with `source: ScreenshareAudio, dtx: false, red: false`,
      start `Engine` with the plan, start the 2 s rescan.
    - `async fn stop_publishing(&self)`, `async fn set_muted(&self, muted: bool)` (track mute; capture keeps running).
    - `async fn watch(&self, streamer: UserId, on: bool)`: subscribe/unsubscribe `<streamer>#a`'s
      `ScreenshareAudio` publication, now or when it is published later (`TrackPublished`).
    - `async fn leave(&self)`: stop everything, close the room.
  - Received stream audio: downmix to mono (the mixer is mono; spec's stereo playback waits for a stereo mixer) and
    `push_in(generation, sid, ..)`; mixer owner = the streamer's `<uid>` (the part before `#`), kind `Stream`. The
    `Sub` drop guard from the spike removes the source.
  - Commands: `stream_audio_start(channel_id, window_title: Option<String>)`, `stream_audio_stop()`,
    `stream_audio_mute(muted: bool)`, `stream_audio_watch(channel_id, streamer, on: bool)`,
    `set_stream_volume(user_id, pct: u16)` (→ `set_gain(user_id, SourceKind::Stream, percent_to_gain(pct))`).
  - `share:` lines every 5 s while publishing: `{"ev":"audio_out","captures":n,"chunks":k,"silentChunks":s}`; while
    watching: `{"ev":"audio_in","streamer":..,"frames":k}`.

- [ ] **Step 1: Failing tests** (unit tests with fakes where the room isn't needed; the rest in `stream_it.rs`):
  - `received_stream_audio_lands_in_the_mixer_as_stream_kind` (feed a fake frame through the push helper; mixer has
    one `Stream` source owned by the streamer uid, and `take_peer_levels` stays empty — no speaking ring).
  - `stream_volume_only_touches_stream_kind`.
  - `stream_it.rs` (ignored): `published_stream_audio_reaches_a_watcher` (Linux CI can't capture, so publish through
    the same `NativeAudioSource` path with synthetic frames via a `#[cfg(test)]`/`pub(crate)` hook that bypasses
    `Engine`), and `leave_voice_closes_the_stream_connection` (after `leave`, the server's next webhook marks the `#a`
    participant gone; the room is closed).
- [ ] **Step 2:** FAIL. **Step 3:** Implement. **Step 4:** `cargo test -p pulse-client` PASS; `just voice-it` PASS.
- [ ] **Step 5:** Commit `feat(client): stream audio connection through the voice mixer`.

### Task 11: UI groundwork: dependency, CSP, state

**Files:** `client/ui/package.json` (`"livekit-client": "2.22.4"`), `pnpm-lock.yaml`,
`client/src-tauri/tauri.conf.json`, `client/src-tauri/tauri.dev.conf.json` (new), `justfile` (`dev-client` passes
`--config tauri.dev.conf.json`), `client/ui/src/lib/state.ts`, `client/ui/src/lib/selectors.ts`.

- [ ] **Step 1: CSP.** Shipped `tauri.conf.json` becomes:
  `default-src 'self' ipc: http://ipc.localhost; img-src 'self' data:; media-src 'self' mediastream: blob:; style-src 'self' 'unsafe-inline'; font-src 'self'; connect-src ipc: http://ipc.localhost wss://voice.pulsechat.co.za https://voice.pulsechat.co.za`.
  `tauri.dev.conf.json` overrides `app.security.csp` with the same plus `ws://127.0.0.1:7880 http://127.0.0.1:7880 ws://localhost:7880 http://localhost:7880`.
  No `wss: https:` wildcards, no `worker-src` unless a CSP violation in the console proves it is needed (then add
  `worker-src 'self' blob:` and say why in the commit). A login to another server gets voice but streaming fails with a
  CSP error: `StreamRoom.connect` (Task 13) maps it to "Streaming isn't available on this server".
- [ ] **Step 2: Failing tests** in `lib/state.test.ts`:
  `ready_fills_streams`, `stream_started_adds_and_updated_replaces`, `stream_ended_removes`,
  `viewers_changed_updates_viewers_only`, `voice_left_does_not_touch_streams` (the server sends `StreamEnded`).
  `AppState.streams: Record<string /*channelId*/, Record<string /*userId*/, StreamInfo>>`.
  Selectors: `liveIn(s, channelId): Record<string, StreamInfo>`, `isLive(s, channelId, userId): boolean`.
- [ ] **Step 3:** `pnpm --dir client/ui test` → FAIL. **Step 4:** Implement. **Step 5:** `just ui-check` PASS.
- [ ] **Step 6:** Commit `feat(ui): livekit-client, narrowed CSP, stream state`.

### Task 12: UI pure helpers

**Files:** Create in `client/ui/src/lib/stream/`: `encodings.ts` (+test, lifted), `sdp.ts` (+test, lifted),
`frames.ts` (+test, lifted), `stats.ts` (+test), `prefs.ts` (+test), `platform.ts`.

**Interfaces — Produces:**
- `type Res = 720 | 1080 | 1440`, `type Fps = 15 | 30 | 60`
- `encodingFor(res: Res, fps: Fps): { maxBitrate: number; maxFramerate: Fps; contentHint: 'motion' | 'detail'; degradation: 'maintain-framerate' | 'maintain-resolution' }`
- `LOW_LAYER = { width: 640, height: 360, maxBitrate: 500_000, maxFramerate: 30 }`
- `resolutionsFor(sourceHeight: number): Res[]` (never above the source; a source under 720 still offers `[720]` and
  the capture stays at its native size)
- `addRtxTime(sdp: string, ms: number): string`; `installRtxTime(): void` (patches
  `RTCPeerConnection.prototype.setLocalDescription` once, always 300 ms on answers)
- `withDeadline`, `frameLoop` (unchanged from the spike)
- `stats.ts`:
  - `type OutboundSummary = { captureFps: number | null; layers: { rid: string | null; w: number; h: number; fps: number; kbps: number; encoder: string; powerEfficient: boolean | null; encodeMs: number | null; qlr: string }[]; rttMs: number | null; note: FpsNote }`
  - `outboundSummary(prev: RTCStatsReport | null, cur: RTCStatsReport, dtMs: number, targetFps: number): OutboundSummary`
    (`encodeMs` = Δ`totalEncodeTime`·1000 / Δ`framesEncoded` over the interval; `captureFps` from the `media-source`
    report's `framesPerSecond`; full layer = widest)
  - `type InboundSummary = { fps: number; w: number; h: number; kbps: number; decoder: string; powerEfficient: boolean | null; freezes: number; freezeMs: number; lost: number; jitterMsPerFrame: number | null; nacks: number; plis: number; stillLikely: boolean }` (all Δ over the interval)
  - `inboundSummary(prev, cur, dtMs): InboundSummary`; `stillLikely = fps <= 5 && lost === 0`
  - `type FpsNote = 'ok' | 'still' | 'capture-limited' | 'encoder-limited' | 'network-limited'`
  - `fpsNote(captureFps: number | null, sentFps: number, targetFps: number, qlr: string): FpsNote`, in order:
    sent ≥ 0.85·target → `ok`; `qlr === 'cpu'` → `encoder-limited`; `qlr === 'bandwidth'` → `network-limited`;
    capture ≤ 5 → `still`; capture < 0.85·target → `capture-limited`; else `encoder-limited`.
  - `isHardwareEncoder(impl: string, powerEfficient: boolean | undefined): boolean` — `powerEfficient` when defined,
    else false iff `impl` matches `/OpenH264|libvpx|libaom|software/i`.
- `prefs.ts`: `loadGoLive(): { res: Res; fps: Fps; audio: boolean }` (default `{720, 60, true}`), `saveGoLive(v)`,
  `loadDiagOverlay(): boolean` (default false), `saveDiagOverlay(on)`, `loadStreamVolumes()/saveStreamVolumes()` —
  same guarded localStorage helpers as `lib/voiceui.ts`.
- `platform.ts`: `isWindows(): boolean` (`navigator.userAgent.includes('Windows')`).

- [ ] **Step 1: Failing tests:** the spike's `encodings.test.ts`, `sdp.test.ts`, `frames.test.ts` (moved), plus
  `bitrates_match_the_spec_table` (all nine cells), `fifteen_fps_is_detail`, `resolutions_never_exceed_the_source`
  (1080 source → `[720, 1080]`), `encode_ms_is_per_interval_not_cumulative`, `capture_fps_comes_from_media_source`,
  `still_screen_is_not_a_problem` (`fpsNote(2, 2, 60, 'none') === 'still'`), `game_capture_short_of_target_is_capture_limited`
  (`fpsNote(40, 40, 60, 'none') === 'capture-limited'`), `cpu_limited_wins_over_capture`, `inbound_still_is_flagged_not_counted_as_trouble`,
  `nvidia_simulcast_adapter_is_hardware` (`'SimulcastEncoderAdapter (MediaFoundationVideoEncodeAccelerator, MediaFoundationVideoEncodeAccelerator)'`, `true` → true),
  `openh264_is_software`, `go_live_defaults_are_720p60_with_audio`.
- [ ] **Step 2:** FAIL. **Step 3:** Implement. **Step 4:** `just ui-check` PASS.
- [ ] **Step 5:** Commit `feat(ui): stream encodings, SDP and stats helpers`.

### Task 13: UI: the `#v` connection, the store, the diagnostics sampler

**Files:** Create `client/ui/src/lib/stream/room.ts`, `stream.svelte.ts` (+ `stream.test.ts` for its pure parts),
`diag.svelte.ts`; modify `client/ui/src/lib/tauri.ts` (wrappers for Task 6/10 commands).

**Interfaces:**
- `class StreamRoom` (`room.ts`):
  - `static async connect(channelId: string): Promise<StreamRoom>` — `installRtxTime()`, `new Room({ adaptiveStream: false, dynacast: true })`, ticket via `stream_ticket` role `v`, `connect(url, token, { autoSubscribe: false })`; logs `{"ev":"connect","ms":..}`.
  - `static async pick(): Promise<{ track: LocalVideoTrack; label: string; surface: string; width: number; height: number }>` — `createScreenTracks({ audio: false, selfBrowserSurface: 'exclude' })` on an unconnected `Room` (spike trick), no size constraints; wraps `stream_picker_window(true/false)` around it.
  - `async publish(track: LocalVideoTrack, res: Res, fps: Fps): Promise<void>` — `applyConstraints({ height: { max: res }, frameRate: { max: fps } })`, `mediaStreamTrack.contentHint`, `publishTrack(track, { source: ScreenShare, videoCodec: 'h264', simulcast: true, screenShareEncoding: {maxBitrate, maxFramerate}, screenShareSimulcastLayers: [new VideoPreset(640, 360, 500_000, 30)], degradationPreference })`.
  - `async unpublish(): Promise<void>`
  - `async watch(streamer: string, el: HTMLVideoElement): Promise<void>` — the spike's `watch` (15 s deadlines); logs `{"ev":"first_frame","ms":..}`.
  - `unwatch(): void`, `setQuality(q: 'auto' | 'saver'): void` (`setVideoQuality(LOW | HIGH)`), `localStats() / remoteStats(): Promise<RTCStatsReport | undefined>`, `async disconnect(): Promise<void>`.
- `stream.svelte.ts`:
  - `export const stream = $state({ room: null as StreamRoom | null, live: null as null | { channelId: string; res: Res; fps: Fps; audio: boolean; audioMuted: boolean; label: string; encoderWarning: boolean }, watching: null as null | { channelId: string; streamer: string; quality: 'auto' | 'saver' }, error: '' })`
  - `streamApi.goLive(channelId, track, label, settings)`: connect if needed → `publish` → `stream_go_live` → `stream_audio_start` if `audio` (window title = `label` when the surface is `window`) → start preview timer (every 15 s: draw the local track into a 480×270 canvas, `toBlob('image/webp', 0.7)`, `stream_preview_put`) → start diagnostics; track `ended` → `stop()`.
  - `streamApi.stop()`: reverse order; `stream_stop` errors are logged, never block the local stop.
  - `streamApi.watch(channelId, streamer, el)`, `streamApi.unwatch()`, `streamApi.setQuality(q)`, `streamApi.setAudioMuted(m)`.
  - `streamApi.onVoiceChanged(channelId: string | null)`: leaving or switching voice → stop live, unwatch, disconnect.
  - `streamApi.onStreamEnded(channelId, userId, reason)`: if watching that streamer → unwatch, notice "<name> stopped streaming".
  - pure: `needsReannounce(meId: string, live: { channelId: string } | null, readyStreams: StreamInfo[]): boolean`; on every `Ready`, if true → `stream_go_live` again with the current settings.
- `diag.svelte.ts`: `startDiagnostics(kind: 'out' | 'in', getReport: () => Promise<RTCStatsReport | undefined>, targetFps: number): () => void` — samples every 5 s (2 s while the overlay is on), logs one `share:` line (`{"ev":"out",...OutboundSummary}` / `{"ev":"in",...InboundSummary}`), keeps the latest in `export const diag = $state({ out: null as OutboundSummary | null, in: null as InboundSummary | null })`. Each 30 s on the streamer, `isHardwareEncoder` on the full layer sets `stream.live.encoderWarning` and logs `{"ev":"encoder","impl":..,"powerEfficient":..}` when it changes (spec §3.2: 5 s after publish, then every 30 s).

- [ ] **Step 1: Failing tests** (`stream.test.ts`): `reannounces_when_ready_lacks_my_stream`,
  `no_reannounce_when_ready_has_it`, `no_reannounce_when_not_live`.
- [ ] **Step 2:** FAIL. **Step 3:** Implement. **Step 4:** `just ui-check` PASS.
- [ ] **Step 5:** Commit `feat(ui): stream connection, store and quiet diagnostics`.

### Task 14: UI: streamer (Go Live, dialog, card)

**Files:** Create `components/GoLiveDialog.svelte`, `components/LiveCard.svelte`; modify `components/VoicePanel.svelte`,
`app.css`.

- [ ] **Step 1:** `VoicePanel`: a **Go Live** button when `inHere && voice.connection === 'connected' && !stream.live`.
  Off Windows: disabled with tooltip "Streaming from Linux isn't supported yet". Click → `StreamRoom.pick()` → open the
  dialog. Picker cancel (`NotAllowedError`) → nothing, no error.
- [ ] **Step 2:** `GoLiveDialog`: live preview of the picked track (`<video muted autoplay>`), source label,
  **Change source** (stops the old track, picks again), Resolution radio from `resolutionsFor(height)`, Frame rate
  15/30/60, **Stream audio** checkbox, **Go Live**, **Cancel** (stops the track). Initial values from `loadGoLive()`,
  clamped by `resolutionsFor`; Go Live saves them. Errors show inline with **Retry** (spec §7).
- [ ] **Step 3:** `LiveCard` (inside the voice panel while live): "You're live", local preview (attach the local track;
  **Hide preview** remembered in prefs), `{res}p{fps}`, viewer names from `liveIn(...)[me].viewers`, the encoder
  warning "Your GPU encoder isn't being used, so this will cost game FPS" when `encoderWarning`, **Mute stream audio**
  (only if audio on), **Stop**.
- [ ] **Step 4:** While live, `document.body.classList.add('streaming')`; `app.css`: `.streaming *, .streaming *::before, .streaming *::after { animation-play-state: paused !important; transition: none !important; }` (spec §3.3).
- [ ] **Step 5: Manual check (Linux dev, two profiles, local LiveKit):** Go Live is disabled with the tooltip; no
  console CSP errors on load. (Windows behaviour is verified in Tasks 17–18.)
- [ ] **Step 6:** `just ui-check` PASS. Commit `feat(ui): Go Live dialog and live card`.

### Task 15: UI: viewer (badge, popover, toast, watching)

**Files:** Create `components/LiveBadge.svelte`, `LivePopover.svelte`, `StreamToast.svelte`, `StreamViewer.svelte`;
modify `VoicePanel.svelte`, `ChannelList.svelte`, `routes/Shell.svelte`, `lib/state.ts` event hook (call
`streamApi.onStreamEnded`), `lib/voice.svelte.ts` (call `streamApi.onVoiceChanged` on channel change).

- [ ] **Step 1:** `LiveBadge`: red "LIVE" pill next to the name in `VoicePanel` tiles and `ChannelList` occupant rows
  when `isLive`. Hover (300 ms) or click opens `LivePopover`.
- [ ] **Step 2:** `LivePopover`: preview image from `stream_preview_get` (fetched on open, then every 15 s while open;
  a grey box with "No preview yet" on `null`), `{height}p{fps}`, viewer count, **Watch**. Watch is disabled with
  "Join voice to watch" unless `voice.channelId === channelId`, and with "Watching from Linux isn't supported yet" off
  Windows. Hidden for your own stream.
- [ ] **Step 3:** `StreamToast`: on `StreamStarted` where `voice.channelId === channel_id` and `user_id !== me`, show
  "<name> is live" + **Watch** for 4 s (stacked if two arrive). Nobody else gets it.
- [ ] **Step 4:** `StreamViewer` in `Shell`'s `<main>`, replacing the voice panel and text channel while
  `stream.watching` is set: `<video autoplay playsinline>` filling the area (`object-fit: contain`), the streamer's
  name, **Quality** select Auto / Data saver, stream **Volume** 0–200 % (→ `set_stream_volume`, remembered per streamer
  with `saveStreamVolumes`), **Close** (unwatch). On watch: `stream_watch` + `stream_audio_watch(on)`; on close: the
  reverse. Watching someone else switches.
- [ ] **Step 5:** `StreamEnded` for the watched streamer closes the viewer with the notice "<name> stopped
  streaming"; `StreamUpdated{reconnecting}` overlays "Reconnecting…" on the frozen last frame.
- [ ] **Step 6:** `just ui-check` PASS. Commit `feat(ui): LIVE badge, popover, toast and the stream viewer`.

### Task 16: UI: diagnostics overlay and Settings → Advanced

**Files:** Create `components/StreamDiagOverlay.svelte`, `components/AdvancedSettings.svelte`; modify
`components/Settings.svelte` (`Section` gains `'advanced'`, label "Advanced"), `StreamViewer.svelte`, `LiveCard.svelte`.

- [ ] **Step 1:** `AdvancedSettings`: one checkbox "Stream diagnostics" with the hint "Shows live numbers on streams
  you share or watch. They are always written to the log for bug reports." (`loadDiagOverlay/saveDiagOverlay`).
- [ ] **Step 2:** `StreamDiagOverlay` (small monospace box, top-left of the viewer video and of the live card's
  preview), shown only when the toggle is on. Streamer: capture fps, per layer `w×h fps kbps encodeMs`, encoder +
  `powerEfficient`, `qlr`, RTT, note. Viewer: `w×h fps kbps`, decoder, freezes/loss/jitter per interval, connect ms,
  first-frame ms. Note wording: `ok` → "OK", `still` → "Still picture (normal)", `capture-limited` → "Capture is below
  target", `encoder-limited` → "Encoder can't keep up", `network-limited` → "Upload is limiting".
- [ ] **Step 3:** `just ui-check` PASS. Commit `feat(ui): stream diagnostics overlay in Settings → Advanced`.

### Task 17: Test build and deploy (OWNER-GATED)

Nothing in this task runs without the owner's explicit OK for that step.

- [ ] **Step 1:** `just check` and `just voice-it` PASS on the branch head; push.
- [ ] **Step 2 (owner OK):** Run `release-server` manually on `screenshare-v1` → image `:<sha7>` only (Task 0's fix
  keeps `:latest` on main).
- [ ] **Step 3 (owner OK):** `just deploy <sha7>` (copies the new `livekit.yaml`), then
  `ssh pulse@pulse 'cd /srv/pulse && docker compose restart livekit && docker compose logs --tail 20 livekit'`. Voice
  calls drop for a few seconds; do it when nobody is in voice. Check: existing voice still works (RED negotiated),
  `just smoke` passes.
- [ ] **Step 4 (owner OK):** Run `release-windows` manually on `screenshare-v1`; the Windows job runs
  `cargo test --release -p pulse-client --lib stream::` (add that step, `continue-on-error: true` for the loopback test
  on runners without an audio endpoint). Download the installer for both PCs.
- [ ] **Step 5:** Rollback note in the PR description: `just deploy <previous sha>`; the codec list is harmless to
  leave on (it is a superset of the default).

### Task 18: Friend test protocol (two PCs)

Run by the owner with a friend; both on the Task 17 build, both in the same voice channel. Each result goes into a
bug report (Report bug → it carries the log tail with the `share:` lines) titled `screenshare v1: <step>`. Enable
Settings → Advanced → Stream diagnostics on both PCs.

- [ ] **1. Baseline game FPS:** streamer runs a game (one competitive, one co-op) in borderless fullscreen with the
  game's own FPS counter; note average FPS over 60 s with Pulse in voice but not streaming.
- [ ] **2. Streaming cost:** Go Live (window share of the game) at 720p60, then 1080p60, then 1440p60; 60 s each; note
  the game's FPS and the overlay's capture fps / sent fps / encode ms / encoder. Answers: capture fps vs encode fps vs
  game fps (the one-PC 33–43 fps result), and whether hardware encode is used on the friend's GPU.
- [ ] **3. Screen share vs window share:** repeat 1080p60 once with a whole-screen share.
- [ ] **4. Still content:** share a desktop with nothing moving for 30 s: the overlay says "Still picture (normal)",
  the viewer sees no error.
- [ ] **5. Stream audio:** game audio heard by the viewer; the streamer's voice-chat audio (the viewer talking) is
  NOT in the stream (no echo of the viewer's own voice); Mute stream audio works; stream volume slider works.
- [ ] **6. Watch quality:** viewer notes connect and first-frame times from the overlay, freezes per minute in
  motion, then switches to Data saver (picture drops to 360p) and back to Auto.
- [ ] **7. Ends:** streamer clicks Stop (viewer sees "stopped streaming"); Go Live again and leave voice (stream ends);
  Go Live again and close the shared window (stream ends).
- [ ] **8. Toast and badge:** a third person (or the owner's second profile on the viewer PC, which also exercises
  Review Focus 2) joins voice while live: LIVE badge, popover preview, Watch works; the toast appears for people in
  voice only.
- [ ] **9. A/V sync (rough):** watch something with clear hits (a gunshot, a clap); note whether sound lags the picture.
- [ ] **10.** Collect `just reports` and summarise the numbers per step in
  `docs/superpowers/research/screenshare-v1-friend-test.md` (no names: "the owner", "a friend", GPUs by model).
