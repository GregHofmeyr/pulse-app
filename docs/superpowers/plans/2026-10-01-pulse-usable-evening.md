# Pulse "Usable Evening" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Two friends can open Pulse, see their server live, sit in a voice channel together (mute/deafen,
hotkeys, per-user volume, speaking rings, mic test), and chat in a text channel.

**Architecture:** The Rust core gains a gateway client (live events → Tauri events) and a `VoiceManager`
(LiveKit room + our own cpal/APM pipeline, built on the spike's rules). The Svelte UI holds one store that
applies `Ready` and `Event`s through a pure reducer. The server gains voice presence in `Ready`, mute/deafen
broadcast, and a membership requirement for voice tokens. Milestones: **M1 live client core → M2 voice → M3 text.**

**Tech Stack:** Rust (tokio, tokio-tungstenite, livekit 0.9.3, cpal 0.15, tauri 2.12,
tauri-plugin-global-shortcut 2), Svelte 5 + TS, vitest, marked + DOMPurify, Docker LiveKit v1.13.

**Spec:** `docs/superpowers/specs/2026-10-01-pulse-design.md`. **Voice lessons (binding):** `spikes/voice/FINDINGS.md`.

## Global Constraints

- Everything from plan 1's Global Constraints still holds: technical name `pulse-app`; ports LiveKit 7880-7882 / server 7890 / Vite 1420; `audience_for` is the only fan-out path; private channels are 404 to outsiders; no secrets in git; the UI never talks to the network.
- **Voice rules from FINDINGS.md:**
  - (1) Drop a peer's `NativeAudioStream` task on `TrackUnsubscribed` / `ParticipantDisconnected` and before re-subscribing.
  - (2) Device watchdog: no input or output callbacks for 2 s → reopen the device and emit a `voice://device-stalled` event.
  - (3) The per-user volume slider maps 0–200% onto dB, with a soft limiter, never a hard clamp.
  - (4) Manual pipeline only (cpal + APM + `NativeAudioSource`), never `PlatformAudio`.
- Opus publish options: `dtx: true`, `red: true`, `source: Microphone` (spec §6.3).
- One voice connection per client (spec §6.1). Deafen = silence playout + mute mic; undeafen restores the previous mute (spec §6.5).
- DM/group calls are out of scope for this plan (next plan); voice here is **server voice channels only**.
- Markdown is rendered with an allow-list sanitiser; `{@html}` only on DOMPurify output (spec §7, §11).
- Typing: send at most 1 per 3 s, displayed indicator fades after 6 s (spec §5).
- clang ≥ 21 is required to build `webrtc-sys` (CI must install it).

## Review Focus

1. **A voice peer leaving and rejoining repeatedly** → their audio stays at 1× real time, never multiplied or robotic. Pinned in Task 7 (`replacing_peer_drops_old_stream`) and Task 9 (integration rejoin test).
2. **Gateway connection lost mid-session (server restart / Wi-Fi drop)** → the client reconnects with backoff, gets a fresh `Ready` and the UI recovers without a restart; queued messages are sent. Pinned in Task 3 (`reconnects_after_server_restart`) and Task 12 (`outbox_flushes_after_reconnect`).
3. **Malicious markdown in a message** (`<img onerror>`, `javascript:` links, `<script>`) → rendered inert. Pinned in Task 12 (XSS corpus).
4. **Deafen while muted, then undeafen** → you stay muted; deafen while unmuted then undeafen → unmuted. Pinned in Task 7 (controls state-machine table).
5. **A non-member asking for a voice token / a user joining voice in a server they never joined** → refused (403), and the UI offers "Join server" instead. Pinned in Task 2.

---

## File Structure

```
docker/livekit.yaml                       # dev LiveKit: loopback-only, webhook → server (fixes M-8)
docker-compose.yml                        # network_mode host + config file
server/src/voice/routes.rs                # token requires server membership
server/src/gateway/socket.rs              # VoiceState frame → Event::VoiceStateChanged; Ready.voice
protocol/src/gateway.rs                   # VoiceRoom, VoiceState, Event::VoiceStateChanged
client/src-tauri/src/
  gateway.rs                              # GatewayClient: connect/Hello/Ready/events/heartbeat/backoff
  backoff.rs                              # pure backoff schedule
  api.rs                                  # + channels, members, messages, join, voice token; timeouts
  commands.rs                             # + new commands, emits events
  outbox.rs                               # pending sends with nonce + retry
  ipc.rs                                  # Linux unix-socket hotkey IPC (+ CLI mode)
  hotkeys.rs                              # Windows global shortcuts
  voice/mod.rs                            # VoiceManager (LiveKit room orchestration)
  voice/controls.rs                       # mute/deafen state machine (pure)
  voice/mixer.rs                          # per-peer queues, dB gain, soft limiter (pure)
  voice/rx.rs                             # RxTasks: per-peer receive task registry (FINDINGS rule 1)
  voice/meter.rs                          # RMS meter (pure)
  voice/devices.rs                        # cpal capture/playback + APM + watchdog
  voice/mictest.rs                        # local loopback mic test
client/ui/src/lib/
  state.svelte.ts                         # app store; applyReady/applyEvent reducer (pure, vitest)
  markdown.ts                             # marked + DOMPurify render (vitest XSS corpus)
  sounds.ts + public/sounds/*.wav         # generated tones (CC0 by construction)
client/ui/src/routes/
  Shell.svelte                            # layout wired to the store
  ChannelList.svelte  MemberList.svelte  VoicePanel.svelte  VoiceTile.svelte
  TextChannel.svelte  MessageItem.svelte  Composer.svelte  Settings.svelte
```

---

## Milestone 1: live client core

### Task 1: Dev LiveKit with real webhooks

**Files:** Create `docker/livekit.yaml`; Modify `docker-compose.yml`, `.env.example`, `justfile`, `.gitignore`

**Interfaces:**
- Produces: a dev LiveKit on 127.0.0.1:7880 with API key `devkey` and the secret from `.env` (`PULSE_LIVEKIT_SECRET`, at least 32 chars), sending webhooks to `http://127.0.0.1:7890/livekit/webhook`
- Produces: `just voice-smoke`, which proves a real webhook body is accepted

- [ ] **Step 1: Config.**

`docker/livekit.yaml`:
```yaml
port: 7880
bind_addresses: ["127.0.0.1"]
rtc:
  tcp_port: 7881
  udp_port: 7882
  use_external_ip: false
  node_ip: 127.0.0.1
keys:
  devkey: pulse-dev-secret-0123456789abcdefghij
webhook:
  api_key: devkey
  urls: ["http://127.0.0.1:7890/livekit/webhook"]
logging: { level: info }
```
`docker-compose.yml`:
```yaml
services:
  livekit:
    image: livekit/livekit-server:v1.13
    network_mode: host
    volumes: ["./docker/livekit.yaml:/livekit.yaml:ro"]
    command: --config /livekit.yaml
```
`.env.example`: `PULSE_LIVEKIT_SECRET=pulse-dev-secret-0123456789abcdefghij`. Update the spike's hardcoded `"secret"` to read `PULSE_LIVEKIT_SECRET` (default to the same dev value).

- [ ] **Step 2: Smoke recipe.** `just voice-smoke`:
  1. `docker compose up -d livekit`
  2. Start the server in the background on a temp DB.
  3. Create an invite, register via curl, create a server, read the Lounge channel id.
  4. `lk room join --identity <user id> --publish spikes/voice/tone.ogg --exit-after-publish <lounge id>` via the docker CLI.
  5. Assert `sqlite3 <db> "select count(*) from voice_sessions"` is ≥ 1.

  Run it → Expected: count ≥ 1. This also closes plan 1's follow-up ("verify a REAL livekit-server webhook body parses").
- [ ] **Step 3: Commit** `chore(dev): loopback LiveKit with webhooks + voice-smoke`

### Task 2: Server: voice presence in Ready, mute/deafen broadcast, membership for voice

**Files:**
- Modify: `protocol/src/gateway.rs`, `server/src/voice/{routes.rs,state.rs}`, `server/src/gateway/socket.rs`
- Test: `server/tests/voice.rs`, `server/tests/gateway.rs`

**Interfaces:**
- Produces (protocol):
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
pub struct VoiceFlags { pub muted: bool, pub deafened: bool }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct VoiceMember { pub user_id: UserId, pub flags: VoiceFlags }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct VoiceRoom { pub channel_id: ChannelId, pub members: Vec<VoiceMember> }
// Ready gains: pub voice: Vec<VoiceRoom>   (audience-filtered: private rooms only for members)
// ClientFrame gains: VoiceState { flags: VoiceFlags }
// Event gains: VoiceStateChanged { channel_id: ChannelId, user_id: UserId, flags: VoiceFlags }
```
- Produces (server):
  - `VoiceState::set_flags(user, flags) -> Option<ChannelId>`, which returns the room the user is in
  - `VoiceState::rooms() -> Vec<(ChannelId, Vec<(UserId, VoiceFlags)>)>`
  - The voice token for a server voice channel requires server membership (403 otherwise)
- [ ] **Step 1: Failing tests.**
  - `voice.rs::token_requires_server_membership_403`: C never joined the server → 403; after `POST /servers/{id}/join` → 200.
  - `gateway.rs::ready_includes_voice_rooms`: webhook join for A in Lounge → B's `Ready.voice` contains Lounge with A.
  - `gateway.rs::voice_state_frame_broadcasts_flags`:
    1. A is in Lounge (webhook) and sends `{"op":"VoiceState","d":{"flags":{"muted":true,"deafened":false}}}`.
    2. B receives `VoiceStateChanged { channel_id: lounge, user_id: A, flags: { muted: true, .. } }`.
    3. A `VoiceState` from a user **not** in any room is ignored (B receives nothing before a marker).
  - `gateway.rs::ready_hides_private_voice_rooms_from_outsiders`: A in a DM room (webhook) → C's `Ready.voice` has no entry for it.

  Run → FAIL (fields/variants missing).
- [ ] **Step 2: Implement.**
  - `VoiceState` stores `HashMap<ChannelId, HashMap<UserId, VoiceFlags>>`.
  - `build_ready` filters rooms with `channel_for(me, room)`.
  - The socket handles `ClientFrame::VoiceState` → `set_flags` → publish `VoiceStateChanged` through the hub (audience via channel).
  - Regenerate TS (`just gen-types`).

  Run all server tests → PASS.
- [ ] **Step 3: Commit** `feat(server): voice presence in Ready, mute/deafen broadcast, voice needs membership`

### Task 3: Client gateway connection with heartbeat + reconnect

**Files:** Create `client/src-tauri/src/{gateway.rs,backoff.rs}`; Modify `lib.rs`, `commands.rs`, `Cargo.toml`

**Interfaces:**
- Consumes: protocol `ClientFrame`/`ServerFrame`; `Api::base()`
- Produces:
```rust
pub fn backoff_delay(attempt: u32, jitter: f64 /*0..1*/) -> Duration // 1s,2s,4s,8s,16s,30s cap; ±20% jitter
pub enum GatewayUpdate { Ready(Ready), Event(Event), Connection(ConnState) }
#[derive(Serialize, Clone, Copy, PartialEq, Debug)] #[serde(rename_all="snake_case")]
pub enum ConnState { Connecting, Connected, Reconnecting, LoggedOut }
pub struct GatewayHandle { /* send ClientFrame, stop */ }
impl GatewayHandle {
  pub fn spawn(base_url: String, token: String, tx: mpsc::UnboundedSender<GatewayUpdate>) -> Self;
  pub fn send(&self, f: ClientFrame);
  pub fn reconnect_now(&self);          // network-up / window focus
  pub fn stop(&self);
}
```
  - The ws URL is derived from `http(s)://` → `ws(s)://` plus `/gateway`.
  - Heartbeat every 30 s.
  - Close code 4001 → `LoggedOut` (no retry).
  - 1011, 4002, 4003 or any drop → `Reconnecting` with backoff.
  - Tauri wiring: on login/restore, `commands` spawns the gateway and forwards updates as Tauri events `pulse://ready`, `pulse://event`, `pulse://conn`.
- [ ] **Step 1: Failing tests** (`gateway.rs` `#[cfg(test)]`, using `pulse_server::testing`):
  - `backoff_schedule`: attempts 0..7 with jitter 0.5 → 1,2,4,8,16,30,30 s; jitter 0 → −20%, jitter 1 → +20%, always ≤ 36 s.
  - `connects_and_receives_ready_then_events`: spawn, recv `Ready`; send a message via REST; recv `Event::MessageCreated`.
  - `bad_token_reports_logged_out`: an unknown token → `Connection(LoggedOut)` and no further retries within 3 s.
  - `reconnects_after_server_restart`:
    - Needs `testing::spawn_on(addr, db_path)` so the test can drop and re-create the server on the same port and DB. Add it to server/src/testing.rs: `pub async fn spawn_on(addr: SocketAddr, db_url: String) -> TestApp` plus `TestApp::shutdown()` (axum graceful shutdown via a oneshot).
    - Expect `Reconnecting`, then a second `Ready` within 5 s (backoff overridable in tests via a `GatewayHandle::spawn_with(…, base_delay: Duration)` constructor).
- [ ] **Step 2: Implement** with tokio-tungstenite (`features = ["rustls-tls-native-roots"]`). Run → PASS.
- [ ] **Step 3: Commit** `feat(client): gateway client with heartbeat and backoff reconnect`

### Task 4: Client API surface, timeouts, expired-session handling

**Files:** Modify `client/src-tauri/src/{api.rs,commands.rs}`, `client/ui/src/lib/tauri.ts`

**Interfaces:**
- Produces (Api):
  - `join_server(token, ServerId)`, `channels(token, ServerId) -> Vec<Channel>`, `members(token, ServerId) -> Vec<Member>`
  - `messages(token, ChannelId, before: Option<MessageId>) -> Vec<Message>`, `send_message(token, ChannelId, SendMessageRequest) -> Message`
  - `edit_message(token, MessageId, &str) -> Message`, `delete_message(token, MessageId)`
  - `voice_token(token, ChannelId) -> VoiceTokenResponse`
  - The reqwest client gets `connect_timeout(5s)` and `timeout(15s)` (fixes M-6 part 2). `ApiError::Unauthorized` outside login → the command emits `pulse://conn` `logged_out` (fixes M-7).
- Produces (commands): `join_server`, `list_messages`, `send_message`, `edit_message`, `delete_message`, plus the existing ones. `http://` is refused unless the host is `localhost` / `127.0.0.1` / `::1` → `ApiError::Rejected("use https:// for remote servers")` (fixes M-6 part 1).
- [ ] **Step 1: Failing tests** (`api.rs`):
  - `join_then_post_and_page_messages`
  - `edit_and_delete_roundtrip`
  - `voice_token_after_join`
  - `remote_http_refused` (pure fn `check_server_url(&str) -> Result<(), ApiError>`)
  - `timeout_on_unroutable_host`: `10.255.255.1:9` must fail with `Network` in under 7 s
- [ ] **Step 2: Implement**, run → PASS. **Step 3: Commit** `feat(client): full REST surface, timeouts, logged-out handling`

### Task 5: UI store + reducer (vitest)

**Files:** Create `client/ui/src/lib/state.svelte.ts`, `client/ui/src/lib/state.test.ts`; Modify `client/ui/package.json` (add `vitest`, script `test`), `justfile` `ui-check` (add `pnpm test`)

**Interfaces:**
- Consumes: generated protocol types
- Produces:
```ts
export type AppState = {
  me: User | null; servers: Server[]; channels: Record<string, Channel>; members: Record<string, Member[]>;
  dmMembers: Record<string, string[]>; voice: Record<string, VoiceMember[]>;   // channelId -> members
  messages: Record<string, Message[]>;       // channelId -> oldest..newest
  typing: Record<string, Record<string, number>>; // channelId -> userId -> expiresAt(ms)
  conn: 'connecting'|'connected'|'reconnecting'|'logged_out';
}
export function emptyState(): AppState
export function applyReady(s: AppState, r: Ready): AppState
export function applyEvent(s: AppState, e: Event, now: number): AppState   // pure
export const app: { state: AppState }   // $state-backed singleton, fed by Tauri listeners
export function startListening(): Promise<() => void>
```
- [ ] **Step 1: Failing tests** (`state.test.ts`):
  - Ready populates everything.
  - `MessageCreated` appends and dedupes by id (a second identical event doesn't duplicate). It also replaces a pending optimistic message with the same `nonce`.
  - `MessageUpdated` replaces in place.
  - `MessageDeleted` marks deleted and blanks the content.
  - `VoiceJoined`/`VoiceLeft`/`VoiceStateChanged` maintain `voice`.
  - `Typing` sets expiry = now + 6000.
  - `MemberJoined` and `ChannelCreated` update lists idempotently.
  - A new `Ready` (reconnect) **replaces** state but keeps already-loaded message history for channels that still exist.

  Run `pnpm test` → FAIL.
- [ ] **Step 2: Implement.** Run → PASS. **Step 3: Commit** `feat(ui): app store with pure Ready/Event reducer`

### Task 6: Server view UI: channels, members, join

**Files:** Create `ChannelList.svelte`, `MemberList.svelte`; Modify `Shell.svelte`, `App.svelte`

**Interfaces:**
- Consumes: `app.state`, `api.*`
- Produces: a selection model in `Shell.svelte` (`activeServerId`, `activeChannelId`).
  - Voice channels list their occupants, with a mute/deafen icon from `voice` flags.
  - A "Join server" button when `me` isn't in that server's members.
  - A connection banner when `conn !== 'connected'` (🟠 reconnecting).
  - `logged_out` → back to Login.
- [ ] **Step 1: Failing test:** extract pure selectors to `state.svelte.ts` (`channelsFor(state, serverId)` sorted text-then-voice by position; `isMember(state, serverId)`; `voiceOccupants(state, channelId)`) with vitest cases. Run → FAIL.
- [ ] **Step 2: Implement** the selectors and components (match the mockup tokens). `pnpm check` + `pnpm test` → PASS.
- [ ] **Step 3: Commit** `feat(ui): live server view with channels, members, join`

---

## Milestone 2: voice

### Task 7: Voice pure core: controls, mixer, rx registry, meter

**Files:** Create `client/src-tauri/src/voice/{mod.rs,controls.rs,mixer.rs,rx.rs,meter.rs}`

**Interfaces:**
- Produces:
```rust
// controls.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
pub struct Controls { pub muted: bool, pub deafened: bool, muted_before_deafen: bool }
impl Controls { pub fn toggle_mute(&mut self); pub fn toggle_deafen(&mut self); pub fn mic_open(&self) -> bool; pub fn playout_on(&self) -> bool; }
// mixer.rs
pub fn percent_to_gain(pct: u16 /*0..=200*/) -> f32;   // 0→0.0, 100→1.0 (0 dB), 200→ +12 dB (≈3.98); dB-linear curve between
pub fn soft_limit(x: f32) -> f32;                       // identity below 0.8, smooth tanh knee to ±1.0
pub struct Mixer { /* per-peer VecDeque<i16>, per-peer gain, cap ms */ }
impl Mixer {
  pub fn new(sample_rate: u32, cap_ms: u32) -> Self;
  pub fn push(&mut self, peer: &str, samples: &[i16]);
  pub fn set_gain(&mut self, peer: &str, gain: f32);
  pub fn remove(&mut self, peer: &str);
  pub fn mix_into(&mut self, out: &mut [f32], channels: usize, master: f32);   // pulls one sample per frame per peer
  pub fn buffered(&self, peer: &str) -> usize;
}
// rx.rs (FINDINGS rule 1)
pub struct RxTasks(HashMap<String, tokio::task::JoinHandle<()>>);
impl RxTasks { pub fn replace(&mut self, id: String, h: JoinHandle<()>); pub fn drop_for(&mut self, id: &str); pub fn len(&self) -> usize; pub fn clear(&mut self); }
// meter.rs
pub struct Meter; impl Meter { pub fn add(&mut self, v: f32); pub fn take(&mut self) -> (f32 /*rms*/, u64 /*n*/); }
```
- [ ] **Step 1: Failing tests:**
  - **controls table:** start → mute → deafen (muted, deafened) → undeafen → still muted. Start → deafen → muted+deafened → undeafen → unmuted. `toggle_mute` while deafened → undeafens and unmutes (Discord behaviour). `mic_open` false when muted or deafened; `playout_on` false when deafened.
  - **mixer:**
    - `percent_to_gain(100)==1.0`, `percent_to_gain(0)==0.0`, `percent_to_gain(200)` ≈ 3.98 ±0.01, monotonic over 0..=200.
    - `soft_limit` is monotonic, |out| < 1.0 for any input up to 10.0, and identity for |x| ≤ 0.8.
    - Mixing two peers sums them.
    - Gain applies per peer only.
    - The cap drops the *oldest* samples beyond `cap_ms`.
    - An empty peer contributes silence.
  - **`replacing_peer_drops_old_stream`:** replace "a" twice; the first JoinHandle is aborted (`is_finished()` after a yield); `len()==1`. `drop_for("a")` aborts and `len()==0`.
  - **meter:** RMS of a constant 0.5 = 0.5.

  Run `cargo test -p pulse-client voice::` → FAIL.
- [ ] **Step 2: Implement**, run → PASS. **Step 3: Commit** `feat(voice): pure controls, dB mixer with soft limiter, rx registry, meter`

### Task 8: Devices: capture/playback with APM + watchdog

**Files:** Create `client/src-tauri/src/voice/devices.rs`; Modify `Cargo.toml` (livekit 0.9.3, cpal 0.15), `.github/workflows/ci.yml` (install clang 21 + `libasound2-dev`)

**Interfaces:**
- Produces:
```rust
pub struct DeviceInfo { pub name: String, pub is_default: bool }
pub fn list_inputs() -> Vec<DeviceInfo>; pub fn list_outputs() -> Vec<DeviceInfo>;
pub struct AudioIo { /* cpal streams, shared Mixer, APM, meters, watchdog */ }
pub struct AudioConfig { pub input: Option<String>, pub output: Option<String>, pub input_gain_pct: u16 /*50..=400*/, pub sensitivity: f32 /*rms gate 0..0.2*/, pub echo_cancel: bool, pub noise_suppress: bool, pub auto_gain: bool }
impl AudioIo {
  /// Starts capture → (gain → gate → APM) → 10 ms chunks sent on `mic_tx`; playback pulls from `mixer`, feeds APM reverse.
  pub fn start(cfg: &AudioConfig, mixer: Arc<Mutex<Mixer>>, mic_tx: mpsc::UnboundedSender<Vec<i16>>) -> anyhow::Result<Self>;
  pub fn input_rate(&self) -> u32; pub fn output_rate(&self) -> u32;
  pub fn levels(&self) -> (f32 /*mic rms*/, f32 /*speaker rms*/);
  pub fn stalled(&self, now: Instant) -> bool;   // no input or output callback for > 2 s (FINDINGS rule 2)
  /// Test constructor: no cpal; pulls the mixer on a 10 ms timer (used by Task 9 integration test).
  pub fn start_null(rate: u32, mixer: Arc<Mutex<Mixer>>, mic_tx: mpsc::UnboundedSender<Vec<i16>>) -> Self;
}
```
  - The capture path applies `mic_open` from shared `Controls` (sends nothing when closed, which DTX handles).
  - The sensitivity gate holds open for 300 ms after the last above-threshold chunk.
  - Playback outputs silence when `!playout_on`.
- [ ] **Step 1: Failing tests** (pure parts, factored as fns):
  - `gate_opens_on_speech_and_holds_300ms` (`Gate::process(rms, now) -> bool`)
  - `stalled_after_2s_without_callbacks` (a `Watchdog` with an injected `Instant`)
  - `downmix_interleaved_to_mono` (`fn downmix(data: &[f32], ch: usize) -> impl Iterator<Item=f32>`)
- [ ] **Step 2: Implement** (port the spike's capture/playback). `cargo test` → PASS.
  - **Device smoke** (manual, in the ledger): `list_inputs()` shows the earbuds and the laptop mic.
  - CI: install LLVM 21 via `https://apt.llvm.org/llvm.sh 21`, then `CC=clang-21 CXX=clang++-21`.
- [ ] **Step 3: Commit** `feat(voice): cpal capture/playback with APM, gate, watchdog`

### Task 9: VoiceManager: LiveKit orchestration + commands

**Files:** Create `client/src-tauri/src/voice/mod.rs` (`VoiceManager`); Modify `commands.rs`, `lib.rs`

**Interfaces:**
- Consumes: Tasks 3, 4, 7, 8
- Produces:
```rust
pub struct VoiceManager { /* Option<Session{room, source, track, rx: RxTasks, io: AudioIo, channel}> */ }
impl VoiceManager {
  pub async fn join(&self, api: &Api, token: &str, channel: ChannelId, cfg: AudioConfig) -> Result<(), VoiceError>; // leaves any current room first
  pub async fn leave(&self);
  pub fn toggle_mute(&self) -> Controls; pub fn toggle_deafen(&self) -> Controls;   // also sends ClientFrame::VoiceState via gateway
  pub fn set_peer_volume(&self, user: UserId, pct: u16);
  pub async fn switch_devices(&self, cfg: AudioConfig) -> Result<(), VoiceError>;   // reopen AudioIo, keep the room
}
```
  - Tauri events: `voice://state` `{channel_id, connection: "connecting|connected|reconnecting|disconnected", controls}`, `voice://speaking` `[user_id]`, `voice://quality` `{user_id, quality}`, `voice://levels` `{mic, speaker}` (every 100 ms while open), `voice://device-stalled`.
  - Commands: `join_voice`, `leave_voice`, `toggle_mute`, `toggle_deafen`, `set_peer_volume`, `list_audio_devices`, `set_audio_config`.
  - Rules:
    - Publish with `dtx` + `red`.
    - On `TrackSubscribed` → `rx.replace(identity, task)`, where the task pushes into the mixer.
    - On `TrackUnsubscribed`/`ParticipantDisconnected` → `rx.drop_for` + `mixer.remove`.
    - The watchdog tick (every 500 ms) restarts `AudioIo` once and emits `device-stalled` if it's still stalled.
- [ ] **Step 1: Failing integration test** (`#[ignore]`, runs under `just voice-it` with LiveKit up):
  - `receives_peer_audio_at_real_time_across_rejoins`:
    1. A spawned server + VoiceManager join Lounge with a **null audio device** (`AudioIo::start_null(rate)` test constructor that pulls the mixer on a timer and never opens cpal).
    2. The `lk` docker bot joins and publishes `tone.ogg` 3 times.
    3. Assert mixer intake per second stays within 0.9–1.1× the sample rate on every rejoin, and `rx.len() <= 1`.
- [ ] **Step 2: Implement.** `just voice-it` → PASS. **Step 3: Commit** `feat(voice): VoiceManager over LiveKit with spike rules`

### Task 10: Voice UI, mic test, sounds

**Files:** Create `VoicePanel.svelte`, `VoiceTile.svelte`, `Settings.svelte`, `lib/sounds.ts`, `public/sounds/*.wav`, `client/src-tauri/src/voice/mictest.rs`

**Interfaces:**
- Consumes: Task 9 events/commands, the Task 5 store
- Produces:
  - Clicking a voice channel → `join_voice`.
  - A voice-connected card (state colour + channel, leave button).
  - Mute/deafen buttons in the user panel.
  - A tile grid for the open voice channel: speaking ring from `voice://speaking`, quality bars, muted/deafened badges from the store `voice`.
  - Right-click/click on a tile → volume popover (0–200%, dB-curved) → `set_peer_volume`. Values persisted per user in `localStorage` (try/catch).
  - Settings → Voice & Audio: input/output pickers, input gain 50–400%, sensitivity with a live meter from `voice://levels`, the three APM toggles, and **Let's check**.
  - `mictest.rs`: `start_mic_test(cfg)` plays your processed mic back after 300 ms through the output device (no server needed) and emits levels; `stop_mic_test()`.
  - Sounds: join, leave, mute, unmute, deafen and undeafen as generated sine/chirp WAVs (≤ 20 KB each, made by a checked-in `scripts/gen_sounds.py`), played from the UI on the matching events.
- [ ] **Step 1: Failing tests:** vitest for `volumeLabel(pct)` and `sounds.forTransition(prevControls, nextControls) -> SoundName|null` (mute→"mute", etc.); Rust unit test `mictest::delay_line_delays_by_n_samples`. Run → FAIL.
- [ ] **Step 2: Implement.** `pnpm check && pnpm test && cargo test -p pulse-client` → PASS.
- [ ] **Step 3: Ear test (Greg):** join Lounge from two app instances (two accounts). Hear each other, check mute/deafen states on both sides, the speaking ring, the volume popover, and the mic test.
- [ ] **Step 4: Commit** `feat(ui): voice panel, tiles, per-user volume, settings + mic test, sounds`

### Task 11: Global hotkeys

**Files:** Create `client/src-tauri/src/{ipc.rs,hotkeys.rs}`; Modify `main.rs`, `lib.rs`, `Cargo.toml` (`tauri-plugin-global-shortcut = "2"`)

**Interfaces:**
- Produces:
  - `pulse-app --toggle-mute` / `--toggle-deafen` (CLI mode: connects to `$XDG_RUNTIME_DIR/pulse-app.sock`, writes the command line, exits 0; exits 1 with a message if the app isn't running).
  - The app listens on that socket on Linux (0600 permissions).
  - On Windows: `Ctrl+Shift+M` / `Ctrl+Shift+D` via the global-shortcut plugin (configurable later).
  - Both call `VoiceManager::toggle_*` and emit `voice://state`.
- [ ] **Step 1: Failing tests** (`ipc.rs`, unix only):
  - `socket_roundtrip_dispatches_command` (bind in a tempdir, send "toggle-mute\n", the handler gets `Command::ToggleMute`)
  - `unknown_command_ignored`
  - `socket_is_owner_only` (mode 0o600)
- [ ] **Step 2: Implement.** Run → PASS. The Hyprland bind snippet goes in `docs/linux-hotkeys.md` (not advertised in the UI, per spec).
- [ ] **Step 3: Commit** `feat(client): global mute/deafen hotkeys (Hyprland IPC, Windows shortcuts)`

---

## Milestone 3: text channels

### Task 12: Text channel: history, composer, markdown, edit/delete, replies, typing, outbox

**Files:** Create `TextChannel.svelte`, `MessageItem.svelte`, `Composer.svelte`, `lib/markdown.ts`, `lib/markdown.test.ts`, `client/src-tauri/src/outbox.rs`

**Interfaces:**
- Consumes: Tasks 3, 4, 5
- Produces:
  - `renderMarkdown(src: string): string`, which is marked (GFM, no raw HTML) → DOMPurify allow-list (`p, br, strong, em, del, code, pre, blockquote, ul, ol, li, a[href]`). Links get `target="_blank" rel="noopener noreferrer"` and only `http(s):` / `mailto:` hrefs survive.
  - `Outbox` (Rust): `enqueue(channel, content, reply_to) -> nonce`, `flush(api, token)`. Pending items are retried when the gateway reaches `Connected`; after 3 failures → `failed`. It emits `pulse://outbox` `{nonce, status}`.
  - UI:
    - History loads 50 at a time on scroll-up.
    - Optimistic pending message (greyed) replaced by the `MessageCreated` with the same nonce.
    - Hover actions: reply / edit / delete (own only).
    - Reply quote above the message.
    - Typing sent at most once per 3 s; "X is typing…" from the store, fading at expiry.
    - System and deleted message rendering per the mockup.
- [ ] **Step 1: Failing tests:**
  - **vitest XSS corpus** in `markdown.test.ts`: `<script>alert(1)</script>`, `<img src=x onerror=alert(1)>`, `[x](javascript:alert(1))`, `<a href="data:text/html,...">`, `<svg onload=alert(1)>`, `**bold** <iframe>`. Assert no `<script`, `onerror`, `onload`, `javascript:`, `data:`, `<iframe`, `<svg` in the output, while `**bold**` → `<strong>bold</strong>` and `https://x.y` links keep `rel="noopener noreferrer"`.
  - **vitest** `typingThrottle(lastSentAt, now) -> boolean` (true at most once per 3000 ms).
  - **Rust** `outbox.rs`:
    - `outbox_flushes_after_reconnect`: enqueue while the test server is shut down, restart, flush → the message exists exactly once.
    - `outbox_marks_failed_after_three_attempts`: unroutable base URL.
    - `nonce_echoed_in_message_created`: via the gateway.

  Run → FAIL.
- [ ] **Step 2: Implement.** Run all → PASS.
- [ ] **Step 3: Commit** `feat: text channels with sanitised markdown, replies, typing, offline outbox`

### Task 13: Measure, CI, docs

**Files:** Modify `.github/workflows/ci.yml`, `spikes/voice/FINDINGS.md`, the spec's §12 footprint note, the README stub (`README.md`, new: what Pulse is + dev quickstart)

- [ ] **Step 1:** CI `ui` job runs `pnpm test`. The `rust` job installs clang 21, and the `#[ignore]` voice integration tests stay local-only. Run `actionlint` → clean.
- [ ] **Step 2: Footprint:** release build. Record PSS (`smaps_rollup`, process tree) for (a) idle logged in and (b) in Lounge with 1 peer, in FINDINGS.md and the spec §12 note.
- [ ] **Step 3:** `just check` green → commit `chore: CI for voice deps + vitest, footprint numbers, README`.

---

## Out of scope (next plans)

DM/group calls + ringing + missed-call lines, nicknames + Name Archive, roles UI, avatars, quick card / full profile,
badges, reactions, mentions + unread, mute/DND notification prefs, link handling beyond sanitising, screen share,
hosting, per-IP rate limits (deferred from plan 1), Windows leg of the voice spike.
