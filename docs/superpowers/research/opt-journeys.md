# Pulse: time-to-X journeys, end to end (client + server)

Research only, nothing changed. Repo state: HEAD `ac145d0` plus the working tree. The Windows logs come from build `32f0e87`. RTT on the Windows test path is about 160 ms, measured by WebRTC ("rtt 157-161ms" in the path lines). Greg's own line is about 42 ms. "RTT" below means the round trip from client to server.

## 0. Ground truth from the logs

The Windows clock offset was worked out from the leave event. The client logged "voice: leaving" at 17:49:12.250, and LiveKit logged "participant closing / CLIENT_REQUEST_LEAVE" at 17:49:43.205. Allowing about 80 ms one way gives **server = client + ~30.88 s**. All times below are in client time. The ±50 ms error comes from the one-way guess.

**Voice join #1** (SJA6W4, first join in the process, warm REST pool):

| t (ms) | event | source |
|---|---|---|
| 0 | `join_voice` / "voice: joining" | client 17:41:41.197 |
| 208 | "audio devices opened" (voice token REST, then device open) | client |
| 283–588 | DeepFilterNet init (encoder, ERB, DF decoder) on its own thread, **off the critical path** | client |
| ~1035 | LiveKit "starting RTC session" (the WSS upgrade has reached the server) | lk 17:42:13.112 |
| ~1573 | ICE pair (prflx, then srflx) | lk 13.650 |
| 1639 | "voice: room connected" | client |
| ~1709 | "participant active", LiveKit `connectTime: 673.9ms` | lk 13.786 |
| 1826 | "mic published", `elapsed_ms=1826` | client |

**Voice join #2** (R2ZSZA, same process, about 3.5 min after the previous REST call):

| t (ms) | event |
|---|---|
| 0 | joining (17:45:16.553) |
| **515** | audio devices opened (token, then devices: **+307 ms against join #1**) |
| ~935 | LiveKit "starting RTC session" (48.368): **420 ms** of signal setup, against **~827 ms** in join #1 |
| 1534 | room connected (~600 ms after the session started) |
| 1723 | mic published, `elapsed_ms=1723` |

What the two joins show:
- **Signal setup**, from the start of `Room::connect` to the server seeing the join, takes ~420 ms when warm. That is about TCP (1 RTT) + TLS 1.3 (1 RTT) + the upgrade request (0.5 RTT) at 160 ms. The first join in a process takes **~400 ms more**. The likely causes are WebRTC runtime and PeerConnectionFactory creation, the first initial-offer creation, a cold DNS lookup of `voice.<domain>`, and the rustls native-root load. Those causes are inferred, not yet measured: see "measure first".
- **Transport setup**, from session start to the PC being connected, takes ~600–670 ms, or ~4 RTT. That is the JoinResponse plus answer (0.5 RTT), the ICE check (1 RTT) and DTLS 1.2 (2 RTT). SCTP is already skipped through SNAP, which `rtc_session.rs:527` turns on, if the server accepts it.
- **Publish** takes ~190 ms, which is 1 RTT (AddTrack, then TrackPublished).
- **Token plus devices** varies between 208 and 515 ms. The extra ~300 ms in join #2 matches **a cold HTTPS connection** (2 extra RTT): reqwest's default `pool_idle_timeout` is 90 s, and that call came 3.5 min after the previous REST call.
- The total of ~1.7–1.8 s is about **11 RTT**. Greg's local Linux join (`elapsed_ms=598`, 12 Oct 11:10) has the same shape on a shorter path.

**App launch** (R2ZSZA, second launch at 17:42:00):
`Pulse starting` 00.419 → `gateway Connecting` 01.141 (**722 ms**: Tauri setup, WebView2, JS boot, `restore_session` and its `/me` call) → `gateway Connected` 01.987 (**846 ms**: WSS connect plus Hello → Ready, ≈ 5 RTT). Ready arrives **~1.57 s** after the process logs its start. The process exec and pre-setup time before the first log line are not logged.

**Leave** takes 163 ms, from "voice: leaving" to Disconnected(ClientInitiated). That is ~1 RTT.

---

## 1. App launch to a usable window

### Timeline (Windows, RTT 160)

| # | step | file:line | est. wait | critical? |
|---|---|---|---|---|
| 1 | exec, Tauri builder, single-instance plugin | `lib.rs:46-58` | ~50–150 ms (not logged) | yes |
| 2 | setup: crash `detect` (reads `running`, `crash.json`, log tail), `mark_running`, `logging::init` | `lib.rs:67-75` | <10 ms | yes |
| 3 | `create_main_window`: WebView2 environment plus window build | `lib.rs:23-31,78` | ~150–400 ms cold on Windows (not logged) | yes |
| 4 | Core, outbox, mictest, hotkeys (`hotkeys::install`), VoiceManager | `lib.rs:79-113` | <20 ms | yes, before setup returns |
| 5 | webview loads `index.html`, the bundle and 4 font weights, then mounts the Svelte app | `main.ts`, `App.svelte` | ~100–250 ms | yes |
| 6 | `startListening()` + `startVoiceListening()`: 6 `listen` IPC calls | `App.svelte:23`, `store.svelte.ts:53-76` | ~5–20 ms | yes |
| 7 | `restore_session`: keyring load, **`GET /me`** on a brand-new reqwest client (DNS + TCP + TLS + request ≈ 3–3.5 RTT) | `commands.rs:185-204` | **~500–560 ms** | **yes, the boot screen is a blank div until this returns** (`App.svelte:49-50`) |
| 8 | `activate`: spawn the gateway, send the pending crash report | `commands.rs:50-82` | ~0 | |
| 9 | Shell renders with **empty state** (no servers or channels yet) | `App.svelte:53` | | first "real" paint |
| 10 | gateway `connect_async`: a new TCP and TLS connection plus the upgrade (2.5 RTT), then Hello → `build_ready` → Ready (1 RTT + server time) | `gateway.rs:169-192`, server `socket.rs:133-200` | **~650–850 ms** (846 observed) | **yes, for lists** |
| 11 | `pulse://ready` → `applyReady`: server and channel lists appear | `store.svelte.ts:55` | ~ms | |
| 12 | the user clicks a channel (no channel opens by itself; `activeChannelId` starts as `null`, `Shell.svelte:28`), then `list_messages`: 1 RTT if the `/me` connection is still pooled, 3 RTT cold | `TextChannel.svelte:51-68` | 160–480 ms after the click | yes, for messages |

**Serial chains that could overlap.** `/me` (step 7) waits for the webview to boot (3–6). The gateway (step 10) waits for `/me`. Both run their own DNS, TCP and TLS handshakes in series. Nothing starts on the network until the JS asks for it.

**What blocks first paint.** The webview (3, 5) blocks the window. The `/me` round trips (7) block the shell. The gateway Ready (10) blocks the lists.

### Proposed cuts

**L1. Start the gateway from Rust in `setup` with the stored token. Drop `/me` from the boot path.** (High)
- Where: `commands.rs:185-204` (`restore_session`) and `lib.rs:91`.
- Current cost: ~500–560 ms of serial `/me` round trips, plus the webview boot time (~300–600 ms) during which nothing uses the network.
- Change:
  - In `setup`, load `Store` and, if a token exists, call `activate` straight away. The gateway connect then overlaps WebView2 creation and the JS boot.
  - `restore_session` returns the **cached `User`**, saved next to the token at login, or waits for the first Ready. Ready already carries `me` (`protocol gateway.rs:48`).
  - A bad token comes back as close code 4001 → `LoggedOut` (`gateway.rs:189`), which already resets to the login screen (`App.svelte:40-46`).
  - Ready can arrive before the JS has subscribed (`App.svelte:22` "listen before restoring"), so Rust must keep the latest Ready and return it from `restore_session`.
- Expected gain: Ready lands **~0.6–1.0 s sooner**, at about webview boot + 4 RTT instead of webview boot + 3.5 RTT + 4 RTT. The shell paints about 500 ms sooner.
- Risk: low to medium. A revoked token flashes the shell before the 4001 sends the user to login. A network error used to surface on boot through `while_restoring`; it now shows as "Connecting / Reconnecting" in the shell.
- Measure: log `setup start`, `window built`, a JS `mounted` marker (one `invoke` with a timestamp), `restore_session` start and end, and gateway `tcp/tls/upgrade/ready` times.

**L2. Cache the last Ready snapshot on disk and paint from it.** (Medium)
- Where: `commands.rs:121-126` (forward) and `state.ts` (`applyReady`).
- Current cost: the lists stay empty for ~1.5 s after launch.
- Change: write the Ready JSON to the profile dir on each Ready. In the JS, apply it at boot, marked stale, then replace it when the live Ready arrives.
- Expected gain: lists paint about **1 s sooner**, at roughly webview boot time.
- Risk: stale unread counts or presence for about 1 s. It is a privacy file on disk, though in the same per-profile dir as the logs. Old snapshots must be discarded across protocol versions.
- Measure: time from `mounted` to the first non-empty server list.

**L3. Open the last-viewed channel and prefetch its first page in parallel with the gateway.** (Medium)
- Where: `Shell.svelte:28`, `TextChannel.svelte:51-68`.
- Current cost: messages need a click plus 1–3 RTT.
- Change: remember the last `activeServerId` and `activeChannelId` per profile in localStorage. On boot, issue `list_messages` for that channel at the same time as the gateway connect. A Rust-side prefetch in `setup` would overlap even more.
- Expected gain: messages are visible at about Ready time, with no extra 160–480 ms after the click.
- Risk: low.
- Measure: time from `mounted` to the first message rendered.

**L4. Keep REST warm: one long-lived HTTP connection, HTTP/2, keepalive.** (High: this also drives the voice and message journeys)
- Where: `api.rs:121-126`. `Cargo.toml` sets `reqwest` `default-features = false`, and `Cargo.lock` contains no `h2`, so REST is **HTTP/1.1 only**.
- Current cost:
  - reqwest drops an idle pooled connection after **90 s** (its default), so every REST call after a 90 s lull pays **+2 RTT (~320 ms)**. Join #2 shows exactly this: +307 ms.
  - Under HTTP/1.1, concurrent calls such as the boot `list_*` calls or the crash-report upload each open **another** TCP and TLS connection.
- Change:
  - Enable reqwest `http2` (Caddy negotiates h2 through ALPN).
  - Set `.pool_idle_timeout(None)` or 10 min, `.tcp_keepalive(30 s)`, `.http2_keep_alive_interval(25 s)` and `.http2_keep_alive_while_idle(true)`.
  - Keep in mind that Caddy's default `idle_timeout` is 5 min, so either set the keepalive interval under 5 min or raise the timeout in the Caddyfile.
  - Optionally fire a cheap request right after the token loads to open the connection early.
- Expected gain: saves **~320 ms** on the first REST call after any lull longer than 90 s. In a chat app that means most message sends and most voice joins. Startup calls also share one connection.
- Risk: low. Watch that h2 keepalive pings don't prevent Windows idle power states (one ping every 25 s is negligible).
- Measure:
  - Log the latency of `voice_token` and `send_message`.
  - Compare calls after 30 s idle with calls after 120 s idle, before and after the change.
  - Turn on reqwest `connection_verbose(true)` to count new connections.

**L5. Small items.** (Low)
- The `pulse://ready` payload is not compressed. If Ready grows past ~14 KB (the TCP initial window), it costs an extra RTT. Measure the Ready size. If it is large, enable `permessage-deflate` or trim `people`, `latest` and `members`.
- `build_ready` makes N+1 queries (`socket.rs:139-148`). That is irrelevant at the current scale. Measure the server time inside `build_ready`.

---

## 2. Clicking a voice channel to hearing people

### Timeline (current code, Windows, RTT 160)

Everything below runs in series inside `VoiceManager::connect` (`voice/mod.rs:372-421`). The task brief asked whether devices already open in parallel with the connect. **They do not.** The order is token → devices → connect → publish.

| # | step | file:line | est. / observed | critical? |
|---|---|---|---|---|
| 1 | UI `invoke('join_voice')` → command → `op` lock → `leave_locked` (no-op when not in voice) | `voice.svelte.ts:74`, `commands.rs:349-366`, `mod.rs:338-346` | ~1–5 ms | yes |
| 2 | **`api.voice_token`** POST. Server: `channel_for` + `is_server_member` + JWT sign, ~1–3 ms | `mod.rs:380`, server `routes.rs:28-59` | 1 RTT warm (160), **3 RTT cold (~480)** | **yes** |
| 3 | `processor::spawn` (thread). Strong noise suppression starts loading DeepFilterNet on another thread, ~300 ms of CPU, and RNNoise fills in meanwhile | `mod.rs:386`, `processor.rs:207,289` | ~0 on the path | no |
| 4 | **`start_audio`**: `open_half(input)` **then** `open_half(output)`, each on a parked thread with a 5 s guard. Each does `find_device`, `supported_*_configs` and `build_*_stream` | `mod.rs:388`, `devices.rs:570-600,972-982` | ~50–300 ms (Sonar exposes 8 ch at 96 kHz) | **yes** |
| 5 | **`Room::connect(RoomOptions::default())`** | `mod.rs:392-398` | 1.0–1.43 s | **yes** |
| 5a | `LkRuntime::instance()`, which is held weakly (see below). First join: PC factory, 3 WebRTC threads, synthetic ADM | `livekit-0.9.3/src/rtc_engine/lk_runtime.rs:74-95` | tens of ms to ~200 ms (est.) | yes |
| 5b | early publisher PC: 3 data channels, 3+3 recv sections, `create_initial_offer` (single-PC mode is on by default) | `rtc_session.rs:531-561`, `room/mod.rs:475` | ~10–50 ms | yes |
| 5c | signal WSS to `voice.<domain>` through Caddy: DNS + TCP + TLS 1.3 + upgrade to `/rtc/v1?join_request=…`, with the offer carried in the URL | `livekit-signaling lib.rs:556-616`, `livekit-net native/proxy.rs:218-223` | ~420 ms warm, ~830 ms first in process | yes |
| 5d | JoinResponse plus answer → trickle → ICE check → DTLS → PC connected (`wait_pc_connection`) | `rtc_engine/mod.rs:503-505`, `rtc_session.rs:2379-2450` | **~600 ms** (LiveKit `connectTime` 674 ms) | yes |
| 6 | **`publish_track`**: AddTrack → TrackPublished (1 RTT), then create_sender and renegotiation | `mod.rs:405-412`, `local_participant.rs:384-450` | ~190 ms | **yes, for "joined"** (but not for *hearing* others) |
| 7 | tasks spawned; `emit_state(Connected)` | `mod.rs:362` | ~0 | |
| 8 | **hearing others**: with auto-subscribe, the server adds downtracks and renegotiates on the single PC (offer → answer, ~1 RTT), RTP starts, then `TrackSubscribed` → `NativeAudioStream` → mixer → playout | `mod.rs:538-555` | ~1 RTT + jitter buffer (**not logged**) | yes, for "hear people" |
| 9 | others see you: LiveKit `participant_joined` webhook → `VoiceJoined` fan-out (~5 ms) → 0.5 RTT to each peer. Then `VoiceState` flags after the publish | server `routes.rs:96-116`, `commands.rs:372` | ~at "participant active" + 0.5 RTT | |

The click-to-hearing total is about **1.8 s + ~1 RTT + playout**. Of the ~11 RTT, the irreducible floor for a fresh LiveKit session through TLS is about 2.5 (signal) + 0.5 + 1 (ICE) + 2 (DTLS 1.2) ≈ 6 RTT ≈ 1.0 s. Everything else is serialisation or cold connections.

### What LiveKit Rust SDK 0.9.3 exposes, compared with the JS SDK

Read from `~/.cargo/registry/src/*/livekit-0.9.3`, `livekit-signaling-0.1.4`, `livekit-net-0.1.3` and `libwebrtc-0.3.50`.

- **`RoomOptions`** (`room/mod.rs:442-480`):
  - `auto_subscribe` (true), `adaptive_stream` and `dynacast` (false; both are video-only, so irrelevant for audio).
  - `rtc_config`, `join_retries` (3), `single_peer_connection` (**true**), `connect_timeout` (5 s per signal attempt), `sdk_options`.
- **`RtcConfiguration`** (`libwebrtc peer_connection_factory.rs:54-73`) has only `ice_servers`, `continual_gathering_policy` (GatherContinually), `ice_transport_type` and `enable_sctp_snap`. There is **no `iceCandidatePoolSize`** and no bundle or rtcp-mux knobs.
- **Already optimised by the SDK:**
  - The v1 single-PC path sends **the publisher offer inside the JoinRequest**, which saves a negotiation RTT (`rtc_session.rs:531-567,726-730`).
  - WARP is on: SPED, the "WebRTC-IceHandshakeDtls" field trial that piggybacks DTLS on STUN (`lk_runtime.rs:83-90`, `peer_connection_factory.cpp:169-174`), and SNAP, SCTP-in-SDP (`rtc_session.rs:527`). Both degrade silently if the server does not support them. Whether LiveKit server v1.13.7 (pion) accepts SPED is unknown, so measure it. The WARP and SPED drafts say piggybacking removes one DTLS RTT, and DTLS 1.3 removes another.
- **Not in the Rust SDK:**
  - **There is no `prepareConnection`.** In the JS and Flutter SDKs it pre-resolves DNS, warms TLS, and on LiveKit Cloud picks the edge region.
  - `RegionUrlProvider` only matters for LiveKit Cloud URLs. A self-hosted URL returns an empty list (`lib.rs:362-384`).
  - `JoinRequest.add_track_requests` exists in the protocol (`livekit-protocol livekit.rs:4565`), but the SDK never fills it, so the mic cannot be published as part of the join.
  - `TrackPublishOptions.preconnect_buffer` exists but serves the agents use case.
- **`LkRuntime` is held weakly** (`lk_runtime.rs:28-35,74-95`). When the last Room drops at leave, the PeerConnectionFactory and its threads are destroyed and **rebuilt on the next join**. `LkRuntime::instance()` is `pub`, through `livekit::rtc_engine::lk_runtime`.
- **The signal WebSocket uses `tokio_tungstenite::connect_async`** (`livekit-net native/proxy.rs:218-223`). That leaves Nagle on and loads the native root store for every connection.
- **MoveParticipant** (`livekit-api services/room.rs:319`) is **LiveKit Cloud-only**. On self-hosted servers a channel switch cannot reuse a connection. LiveKit community thread, 2025, citing livekit#4203.

### Proposed cuts, ranked

**V1. Parallelise the connect: token first, then `Room::connect` and the device open at the same time.** (High)
- Where: `voice/mod.rs:380-398`.
- Current cost: devices are serial and take ~50–300 ms. The second join's token plus devices took 515 ms.
- Change:
  - After `voice_token`, `tokio::join!` (or `select!` with an abort on failure) `start_audio(...)` and `Room::connect(...)`. Neither needs the other.
  - `NativeAudioSource` and the track do not need devices.
  - The mic task's `drop_stale` (`mod.rs:455`) already discards audio captured before the join.
  - Also open the input and output halves in parallel (`devices.rs:973-974`).
  - Error path: if the devices fail after the room connected, close the room as the publish-failure path does (`mod.rs:418-420`). If the room fails, drop the `AudioIo`.
- Expected gain: **−50 to −300 ms** on every join.
- Risk: low. A device failure now costs a wasted LiveKit connect, and others briefly see you join and leave. Mitigate by publishing only after both succeed, which is already the order.
- Measure: log `token_ms`, `devices_ms`, `signal_ms`, `pc_ms` and `publish_ms` as separate fields in "voice: joined".

**V2. Get the voice token off the critical path: prefetch on hover, cache per channel, or include it in Ready.** (High)
- Where: `mod.rs:380`, `api.rs:452-465`, server `routes.rs:20,28-59`.
- Current cost: 1 RTT (160 ms), or 3 RTT (~480 ms) when the REST connection is cold.
- Change, either of two options:
  - (a) A `prefetch_voice_token(channel_id)` command fired on `mouseenter`/`focus` of a voice channel row. The UI→invoke hover-to-click gap is usually 150–400 ms, which covers 1 RTT. Cache the token for its 10 min TTL (`TOKEN_TTL`), keyed by channel, and refresh it at about 8 min.
  - (b) Have the server mint tokens for every voice channel the user can join on Ready and on channel-create events. Short TTL, re-issue on demand.
  - Either way, `connect` uses a cached token when one is valid and falls back to the REST call.
- Expected gain: **−160 ms warm, −480 ms cold**.
- Risk: low. A token is a capability to join one room for 10 minutes and stays in process memory. Revoking server membership would not invalidate an already-issued token for up to TTL. The server already rechecks access at token mint, so keep the TTL short (2–5 min) if (b) is used.
- Measure: `token_ms` from V1, plus how often a cached token is used.

**V3. Keep the HTTPS connection warm** (L4). This removes the +307 ms that join #2 paid. (High; same change as L4.)

**V4. Pre-warm what can be pre-warmed for LiveKit.** (Medium)
- Where: `lib.rs:108-113` (VoiceManager creation) and `mod.rs:394`.
- Current cost: the first join in a process paid ~400 ms more signal setup than join #2 (827 against 420 ms). Part of this probably recurs after every leave, because the runtime is weak.
- Change, after login on a blocking thread:
  - (a) Call `livekit::rtc_engine::lk_runtime::LkRuntime::instance()` once and **hold the `Arc` in `VoiceManager` for the app's lifetime**. The factory, threads and synthetic ADM are then never rebuilt.
  - (b) Resolve `voice.<domain>` with `tokio::net::lookup_host` (it is a different hostname from the REST and gateway host, so it needs its own lookup). Do this at startup and every few minutes so it stays in the OS cache.
  - (c) Call `rustls_native_certs::load_native_certs()` once at startup if the OS caches the store. Its cost is unproven.
  - (d) Optionally build a throwaway PeerConnection and offer once, to page in libwebrtc code.
- Expected gain: probably **−100 to −400 ms on the first join**, plus a smaller amount on every later join. Unproven, so measure it.
- Risk: low. Holding the runtime keeps a few idle threads and a little memory.
- Measure: `signal_ms` for first and later joins, with and without the held runtime.

**V5. Report "connected" when the room is connected, and publish in the background.** (Medium: perceived latency)
- Where: `mod.rs:405-422,359-363`.
- Current cost: the UI waits ~190 ms for TrackPublished. Hearing others does not depend on it.
- Change: emit Connected (and play the join sound) right after `Room::connect`. Run `publish_track` as a session task. If it fails, emit a mic error and retry, rather than failing the join.
- Expected gain: the UI and the join sound arrive **~190 ms sooner**. Others hear you at the same time as now.
- Risk: medium. "Connected" no longer guarantees the mic is live, so a failed publish needs a visible state, for example a mic-error icon.
- Measure: the time from Connected to TrackPublished, and how often publish fails.

**V6. Server `rtc.use_ice_lite: true` in `deploy/livekit.yaml`.** (Medium: config-only, A/B it)
- Current cost: ICE plus DTLS take ~600 ms. The full-ICE server also runs its own checks and nomination; the log shows the server switching from the prflx pair to the srflx pair 538 ms in.
- Change: add `use_ice_lite: true` under `rtc`. LiveKit's `config-sample.yaml` says: "server will use a lite ice agent, that will speed up ice connection, but might cause connect issue if server running behind NAT".
- Expected gain: likely tens of ms to ~1 RTT. Unquantified.
- Risk: **medium**. Oracle Cloud uses a 1:1 NAT (the VNIC holds a private IP, and `use_external_ip` advertises the public one). With 1:1 static NAT ICE lite usually works, but test it from a Windows client on a real ISP and on a CGNAT or mobile hotspot before keeping it.
- Measure: LiveKit `connectTime` in "participant active" (674 ms now), and `pc_ms` on the client.

**V7. Confirm WARP (SPED + SNAP) is actually negotiated.** (Medium: measure, then possibly upgrade the server)
- Current cost: if SPED or SNAP is declined, setup costs DTLS 2 RTT + SCTP 2 RTT. The `connectTime` of 674 ms ≈ 4 RTT suggests SNAP works but SPED probably does not.
- Change: none until it is measured. Check whether a newer LiveKit server supports DTLS-in-STUN. If it does, upgrading saves about 1 RTT.
- Expected gain: up to ~160 ms (1 RTT) if SPED can be enabled.
- Risk: a server upgrade.
- Measure: client libwebrtc logs at verbose, or LiveKit debug logs, during one join.

**V8. Log the moment people are actually heard.** (Measure first; no gain by itself)
- Where: `mod.rs:538-555`.
- Change: log `TrackSubscribed` with `since_join_ms`, and the first `NativeAudioStream` frame per peer.
- Why: the step that matters to users (the subscriber renegotiation, then the first RTP, then the jitter buffer) is invisible today.

**V9. Speculative connect on hover, with a `hidden` participant grant.** (Low: experimental, not recommended yet)
- Change: connect on hover as hidden, so others don't see you, then become visible on click.
- Why not yet: the SDK cannot change `hidden` on a live connection; that needs a server `UpdateParticipant` call with new permissions, and support is uncertain. Hover-only joins would also churn webhooks and rooms, and LiveKit's `departure_timeout` is 20 s.
- Expected gain: the whole ~1.4 s connect, if it works.
- Risk: high. Consider it only after V1 to V4.

**V10. DeepFilterNet reload per session.** (Low)
- Where: `processor.rs:207-252`.
- Current cost: ~300 ms of CPU per join and per device reopen (the R2ZSZA log shows about 20 reloads in 2 min of settings changes). This is off the critical path, but during that time Strong noise suppression is not active yet (RNNoise stands in), and it competes for CPU during the connect.
- Change: keep a loaded `DfTract` warm in a process-wide pool and reset its state per session instead of rebuilding it.
- Expected gain: Strong suppression from the first word, and less CPU contention while connecting.
- Risk: low to medium (state reset correctness).

**What does not help.** `adaptive_stream` and `dynacast` are video-only. `auto_subscribe` is already true. `iceCandidatePoolSize` is not exposed (and `GatherContinually` already starts gathering when the early offer is created). A different region is not possible with one Johannesburg box.

---

## 3. Switching voice channels and leaving

### Timeline (switch A → B)

| # | step | file:line | est. |
|---|---|---|---|
| 1 | `join(B)`: cancel any pending join, `op` lock | `mod.rs:338-343` | ~0 |
| 2 | **`leave_locked`**: `alive=false`, **`room.close()` awaited** (5 s cap), rx clear, `AudioIo` dropped (WASAPI threads joined), state=Disconnected | `mod.rs:344,723-740` | **~160 ms** (1 RTT, observed 163 ms), plus the device close, ~10–150 ms |
| 3 | the full join of B from section 2: token, **reopen devices**, **reload DFN**, connect, publish | | ~1.7–1.8 s |

The UI goes Connected(A) → Disconnected → Connecting(B) → Connected(B). For about 2 s you are in no room at all.

### Proposed cuts

**S1. Don't wait for the old room to close: detach it.** (High for switches)
- Where: `mod.rs:723-740` (leave_locked from `join`).
- Current cost: ~160 ms, up to 5 s on a bad network.
- Change:
  - On a switch, set `alive=false`, mute the outgoing frames (stop feeding `source`), take the `Session` out, and `tokio::spawn` the `room.close()` with the 5 s timeout. Then go straight to joining B.
  - Plain Leave can stay as it is, or also detach.
- Expected gain: **−160 ms** per switch, and up to 5 s on lossy links.
- Risk: low to medium. For about 1 RTT, others may see you in both rooms (the webhook `participant_left` arrives a bit later). The audio is already muted. The app-exit leave (`lib.rs:162-173`) must still await.

**S2. Keep the audio pipeline (AudioIo, processor, DFN) across a switch; reset only the mixer.** (Medium)
- Where: `mod.rs:382-390` (created per session) and `mod.rs:735-736` (dropped on leave).
- Current cost: device close plus reopen (~60–450 ms; 208–515 ms including the token in the logs) and the DFN reload (~300 ms of CPU). V1 already takes the device open off the critical path, so the gain here is mostly smoothness: no WASAPI churn and Strong suppression stays on.
- Change: move `Shared`, the processor and `AudioIo` into a `Pipeline` owned by `VoiceManager`, and hand it from session to session. Close it only on an explicit Leave, or after N seconds without a session.
- Expected gain: −50 to −300 ms if V1 is not done; otherwise less CPU and fewer device glitches.
- Risk: medium. The reopen, follow and watchdog logic is session-scoped today (`Reopener`), so it would need a refactor.

**S3. Prefetched token for the destination channel** (V2). Hover happens just before a switch click too. (High; same change as V2.)

**S4. Keeping one connection while switching rooms is not possible self-hosted.** `MoveParticipant` is Cloud-only. Discord-style "one voice connection, many channels" would need all voice channels as one LiveKit room with selective subscriptions (`auto_subscribe=false` plus manual `set_subscribed`), which conflicts with the per-channel webhooks and token grants. Not recommended. (Note only)

**Leave by itself:** 163 ms, awaited, with the UI showing Disconnected afterwards (`mod.rs:738`). Emit Disconnected (and play the leave sound) **before** awaiting `close`. That makes Leave feel instant, at no risk. (Low effort, Medium UX)

---

## 4. Sending a message until others see it

### Timeline

| # | step | file:line | est. |
|---|---|---|---|
| 1 | Composer: optimistic pending bubble locally, then `invoke('send_message')` | `Composer.svelte`, `commands.rs:297-310` | ~1–5 ms (the sender sees their own message immediately) |
| 2 | `outbox.enqueue`, then `flush` (one flush at a time, in order) | `outbox.rs:57-130` | ~0; a burst of N messages is serialised at ~1 RTT each |
| 3 | REST `POST /channels/{id}/messages` | `api.rs:404-418` | **0.5 RTT one way if warm; +2 RTT (TCP+TLS) if idle for more than 90 s** (L4) |
| 4 | Caddy → axum. Server `send`: rate limit; `channel_for`; member check; mentions resolve; reply lookup; **INSERT; mentions store; DELETE dm_hidden; reads::advance**. That is 3–4 separate autocommit writes, each a WAL commit with `synchronous=FULL` (sqlx default; `db.rs:9-13` sets only WAL, FKs and busy timeout), so one fsync each on an Oracle block volume | server `messages/routes.rs:138-209` | ~3–20 ms (est.; not logged) |
| 5 | `hub.publish(MessageCreated)`: `audience_for` (1 query, plus a members query for DMs), then a `try_send` to each connection queue. This happens **before** the HTTP response | `routes.rs:210-218`, `hub.rs:122-132` | ~1 ms |
| 6 | the socket task writes the frame; Caddy proxies the WS (Go sets TCP_NODELAY) | `socket.rs:228-231` | ~0 |
| 7 | the receiver gets it 0.5 RTT later; Tauri emits `pulse://event`; `applyEvent` renders | `commands.rs:127-128`, `store.svelte.ts:56-59` | ~1–5 ms |
| 8 | the sender's REST response arrives in parallel with step 7 → outbox `sent` → bubble confirmed | `outbox.rs:99` | |

**Round trips.** The sender pays 1 RTT for confirmation. The receiver sees the message **0.5 RTT(sender) + server + 0.5 RTT(receiver)** after the send, which is optimal for a star topology. A cold REST connection adds **2 RTT (~320 ms) on the sender side**. That is the only large removable cost.

### Proposed cuts

**M1. Keep REST warm** (L4). This removes the 320 ms penalty on the first message after a lull. (High)

**M2. Send messages over the gateway WebSocket, which is already warm and ordered, with REST as the fallback.** (Medium)
- Where: `gateway.rs:213-219`, the server `ClientFrame`, and `outbox.rs`.
- Current cost: HTTP framing plus the cold-connection risk. Even with L4, h2 adds header overhead (negligible).
- Change: add a `ClientFrame::SendMessage { nonce, … }` and acknowledge it with an `Ack { nonce, message }` frame, or rely on the echoed `MessageCreated` with the nonce. The outbox falls back to REST when the gateway is not connected.
- Expected gain: guaranteed warm path. Saves 0–320 ms depending on L4.
- Risk: medium. It is a protocol change (version bump; old clients keep using REST). The server path needs the same validation and rate limits. Don't do it if L4 alone proves enough.

**M3. One transaction per send on the server, plus `synchronous=NORMAL` in WAL mode.** (Low)
- Where: `messages/routes.rs:193-209`, `db.rs:9-13`.
- Current cost: 3–4 fsyncs per message (est. 1–5 ms each on block storage).
- Change: wrap INSERT, mentions, dm_hidden and reads in `db.begin()` … `commit()`. Add `.synchronous(SqliteSynchronous::Normal)`, which is safe from corruption in WAL mode; only the last commits can be lost on power loss.
- Expected gain: a few ms to ~15 ms of server time per message.
- Risk: low.
- Measure: a `tracing` span on `send`, and the p50/p99 of the handler.

**M4. Bursts:** the outbox sends in series, at 1 RTT per message, to keep order. That is fine as it is. h2 or the gateway path would allow pipelining but needs order guarantees on the server. (Note only)

---

## 5. Reconnect after a network blip

### Gateway timeline (current)

| # | step | file:line | est. |
|---|---|---|---|
| 1a | **detection, hard error** (RST or WS close): immediate | `gateway.rs:205-212` | ~0 |
| 1b | **detection, silent blip** (Wi-Fi drop or NAT rebind with no RST): heartbeat every 30 s. The client only notices when no frame arrives for **75 s** (`DEAD_AFTER`), or when a write fails after the OS gives up retransmitting (Windows ~20–60 s) | `gateway.rs:14-18,197-203` | **up to 75 s** |
| 1c | the UI `online`/`focus` "kick" only acts if the state is already `reconnecting`. A half-open socket still reads `connected`, so the kick does nothing | `store.svelte.ts:78-80` | |
| 2 | Retry: `Reconnecting`, then `backoff_delay(attempt=0)` = **0.8–1.2 s**, even after a healthy session | `gateway.rs:121-128`, `backoff.rs:8-11` | ~1 s |
| 3 | `connect_async` (DNS cached; TCP + TLS + upgrade ≈ 2.5 RTT), then Hello → **full Ready** (1 RTT + build) | `gateway.rs:169-192` | ~650–850 ms |
| 4 | UI `applyReady` clears the history markers, then refetches the open channel's page (`TextChannel.svelte:48-68`) | | +1 RTT (or 3 cold) |
| 5 | the outbox flushes on Connected | `commands.rs:136-145` | |

Total after the network returns: about **1.8–2.3 s** for a hard error, and **up to ~75 s** for a silent one.

### Voice (LiveKit's own)

- The signal ping timeout comes from the JoinResponse. ICE disconnect after ~5 s triggers a resume: a signal reconnect with `reconnect=1`, then an ICE restart.
- The SDK backs off with full jitter, starting at 300 ms and growing ×2 up to 7 s per attempt, for **10 attempts**, up to ~45 s (`reconnect_strategy.rs`).
- If all attempts fail, `Disconnected` arrives and Pulse **tears the session down for good** (`mod.rs:583-591`). The user has to click the channel again.
- Voice runs independently of the gateway (separate connections).

### Proposed cuts

**R1. Detect dead sockets fast.** (High)
- Where: `gateway.rs:14-18,194-222`, `store.svelte.ts:78-82`.
- Current cost: up to 75 s offline without the UI saying so.
- Change:
  - (a) After each heartbeat, expect a `HeartbeatAck` within ~max(3×RTT, 5 s). If it doesn't come, reconnect.
  - (b) Lower `HEARTBEAT` to 15 s. Server load is trivial at this scale; keep the server's `heartbeat_timeout` above it.
  - (c) Add a `probe_now` command, sent on `online`, `focus` and resume from sleep regardless of state. It sends a heartbeat and requires the ack within 3 s.
- Expected gain: silent-blip detection drops from ≤75 s to **≤20 s**, and to ~3 s when Windows signals that the network came back.
- Risk: low. False positives on a very slow link are mitigated by tying the deadline to RTT.
- Measure: disable Wi-Fi for 5, 30 and 60 s and log the times for `Reconnecting` and `Connected`.

**R2. Retry immediately the first time after a healthy session.** (Medium)
- Where: `gateway.rs:121-128`.
- Current cost: 0.8–1.2 s.
- Change: when `was_ready` is true, use a 0–250 ms jittered delay for attempt 0, then the existing schedule.
- Expected gain: −0.8 to −1 s per blip.
- Risk: low. In a stampede after a server restart, all clients come back within 250 ms; the server was restarting anyway and close code 1012 already exists.

**R3. Auto-rejoin voice after LiveKit gives up.** (Medium)
- Where: `mod.rs:583-591`.
- Current cost: you leave the voice channel for good after ~20–45 s of outage (expected time with full jitter), and must click again.
- Change: on `Disconnected` with a reason other than ClientInitiated, DuplicateIdentity, ParticipantRemoved or RoomDeleted, schedule a `join(same channel)` with Pulse's backoff while the gateway is up. Show "Reconnecting voice…".
- Expected gain: recovery without the user doing anything, as Discord does (its engineering blog on voice describes reconnecting quickly after network changes, Wi-Fi blips and IP changes).
- Risk: medium. Avoid rejoin loops by capping attempts and respecting the user's Leave.

**R4. Gateway resume with an event cursor instead of a full Ready.** (Low)
- Current cost: Ready costs the same RTTs as a resume. The extra cost is payload size, the N+1 build and the message-page refetch.
- Change: a per-user ring buffer of recent events plus `Hello { resume_from: seq }`. Fall back to Ready.
- Expected gain: small at this scale.
- Risk: medium complexity. Prefer R1 and R2.

**R5. Turn off Nagle on Pulse's own gateway socket.** (Low)
- Where: `gateway.rs:170`.
- Change: `connect_async_with_config(url, None, /*disable_nagle*/ true)`.
- Why: back-to-back small frames (Typing, VoiceState, Heartbeat) can otherwise wait up to 1 RTT for an ACK.
- The LiveKit signal socket has the same issue inside `livekit-net` and would need an SDK patch. Note it for upstream.

---

## 6. What others did (sources)

- **LiveKit `config-sample.yaml`** (master; fetched 2026-10-13): `use_ice_lite` "will speed up ice connection, but might cause connect issue if server running behind NAT". It also covers `tcp_port`, `allow_tcp_fallback` (default true) and `departure_timeout` (20 s). https://github.com/livekit/livekit/blob/master/config-sample.yaml
- **LiveKit `prepareConnection`** (JS and Flutter SDKs): "call as soon as the page is loaded… DNS resolution, TLS, and on Cloud the best edge". The Rust SDK 0.9.3 has no equivalent. Flutter API reference: https://docs.livekit.io/reference/client-sdk-flutter/livekit_client/Room/prepareConnection.html. JS SDK commit adding it with region pinning: https://app.semanticdiff.com/gh/livekit/client-sdk-js/commit/264a0cac4366396e6469c54462b4361f8c188211
- **LiveKit region pinning** is Cloud-only and irrelevant to one self-hosted box: https://docs.livekit.io/deploy/admin/regions/region-pinning
- **MoveParticipant is Cloud-only.** LiveKit community thread "Support for MoveParticipant in self-hosted LiveKit" (2025), citing livekit#4203: https://community.livekit.io/t/support-for-moveparticipant-in-self-hosted-livekit/245
- **WARP/SPED, IETF drafts (2026):**
  - draft-hancke-webrtc-sped-00 (March 2026): DTLS inside STUN, so ICE and DTLS run in parallel. https://datatracker.ietf.org/doc/html/draft-hancke-webrtc-sped-00
  - draft-uberti-tsvwg-warp-00 (July 2026): about 6 RTT for a classic setup; piggybacking saves 1 DTLS RTT, and DTLS 1.3 saves another. https://www.ietf.org/archive/id/draft-uberti-tsvwg-warp-00.html
  - libwebrtc DTLS-STUN piggyback doc: https://webrtc.googlesource.com/src/+/refs/heads/main/g3doc/dtls_stun_piggyback.md
  - The Rust SDK 0.9.3 turns this on (`lk_runtime.rs:83-90`).
- **Discord:**
  - "We care about your voice" engineering special: fast detection of network changes, Wi-Fi blips and IP changes, with voice reconnection that responds as fast. https://discord.com/blog/we-care-about-your-voice-engineering-special
  - Voice server assignment by reported health and load: https://discord.com/blog/how-discord-handles-two-and-half-million-concurrent-voice-users-using-webrtc
  - Voice moved to the edge, to be close to users: https://discord.com/blog/how-we-moved-discord-voice-to-the-edge
  - None of these publishes join-time numbers.
- **No LiveKit rust-sdks issue about slow `Room::connect`** turned up. The release notes mention compressing the initial offer in the JoinRequest URL "to improve connection performance on slow networks" (releases.sh/livekit/livekit-rust-sdks); the 0.9.3 source already gzips it (`signaling lib.rs:975-992`).
- **Defaults read from source, not docs:**
  - reqwest 0.12: `pool_idle_timeout` 90 s; h2 needs the `http2` feature.
  - Caddy: `idle_timeout` 5 min.
  - sqlx sqlite: `synchronous=FULL`.
  - Check these against the pinned versions before relying on them.

---

## 7. Ranked proposals (summary)

| Rank | ID | Change | file:line | Gain (est.) | Risk |
|---|---|---|---|---|---|
| **High** | L4/V3/M1 | Warm REST: h2, long pool idle, keepalive | `api.rs:121-126`, `Cargo.toml` reqwest features | −320 ms on the first REST call after 90 s idle (voice token, send) | low |
| **High** | V1 | `Room::connect` ∥ device open (∥ in/out halves) | `voice/mod.rs:380-398`, `devices.rs:973-974` | −50 to −300 ms per join | low |
| **High** | V2/S3 | Voice token prefetch on hover, or cache per channel | `mod.rs:380`, server `voice/routes.rs:20,28-59` | −160 to −480 ms per join | low |
| **High** | L1 | Gateway from `setup` with the stored token; no `/me` on boot | `commands.rs:185-204`, `lib.rs:91` | Ready −0.6 to −1.0 s, shell −0.5 s | low/med |
| **High** | R1 | Heartbeat-ack deadline, 15 s beats, probe on online/focus | `gateway.rs:14-18,194-222`, `store.svelte.ts:78-82` | silent-blip detection ≤75 s → ≤20 s (~3 s on network-up) | low |
| **High** | S1 | Detach the old room's `close()` on a switch | `mod.rs:344,723-740` | −160 ms per switch (up to −5 s on bad links) | low/med |
| Medium | V4 | Hold `LkRuntime`; pre-resolve `voice.<domain>` | `lib.rs:108-113`, `mod.rs:394` | first join −100 to −400 ms (to verify) | low |
| Medium | V5 | Connected after room connect, publish in background | `mod.rs:405-422` | UI −190 ms | med |
| Medium | V6 | `rtc.use_ice_lite: true` (A/B) | `deploy/livekit.yaml:4-12` | tens of ms to 1 RTT | med (1:1 NAT) |
| Medium | R2 | Immediate first gateway retry after a healthy session | `gateway.rs:121-128` | −0.8 to −1 s per blip | low |
| Medium | R3 | Auto-rejoin voice after LiveKit `Disconnected` | `mod.rs:583-591` | no manual rejoin | med |
| Medium | L2 | Paint from a cached Ready | `commands.rs:121-126`, `state.ts` | lists −1 s | low/med |
| Medium | L3 | Restore and prefetch the last channel | `Shell.svelte:28`, `TextChannel.svelte:51-68` | messages visible at Ready | low |
| Medium | S2 | Keep the audio pipeline across switches | `mod.rs:382-390,735-736` | smoother switches, Strong suppression stays on | med |
| Medium | M2 | Messages over the gateway WS | `gateway.rs:213-219`, `outbox.rs` | warm path always | med |
| Medium | V7 | Verify WARP negotiation; upgrade the server if SPED is missing | — | up to −1 RTT | — |
| Low | M3 | One transaction plus `synchronous=NORMAL` | `messages/routes.rs:193-209`, `db.rs:9-13` | −few ms server time | low |
| Low | V10 | Keep DFN loaded across sessions | `processor.rs:207-252` | Strong suppression from the first word, less CPU during connect | low/med |
| Low | R5 | `disable_nagle` on the gateway WS | `gateway.rs:170` | ≤1 RTT on frame bursts | low |
| Low | L5/R4 | Ready size and compression, gateway resume | `socket.rs:133-200` | small | med |
| Low | V9 | Hidden speculative connect on hover | — | ~1.4 s | high |
| Low | — | Emit Disconnected before awaiting `close` on Leave | `mod.rs:729-738` | Leave feels instant | none |

With **V1 + V2 + V3 + V4**, a warm join drops from ~1.75 s to about **signal (~420) + PC (~600) + publish (190) ≈ 1.2 s**, and with V5 the user sees "connected" at about **1.0 s**. On the 160 ms path the floor without protocol changes is about 6 RTT (≈1.0 s) to a connected PC.

---

## 8. Measure first

1. **Per-phase voice join timing.** In "voice: joined", log `token_ms`, `devices_ms` (split input and output), `room_connect_ms`, `publish_ms`, and after the join `first_subscribed_ms` and `first_remote_audio_ms` per peer (`mod.rs:380-422,538-555`). Without this the remaining numbers are inferences from timestamp gaps.
2. **Inside `Room::connect`.** Turn on `RUST_LOG=livekit=debug,livekit_signaling=debug,livekit_net=debug` for one Windows join. Look at "connecting to", the JoinResponse, the PC state changes, and whether SPED/SNAP were negotiated. This confirms the 827 against 420 ms first-join gap and whether it repeats after leave+rejoin (the weak `LkRuntime`).
3. **REST cold against warm.** Time `voice_token` and `send_message` after 30 s and after 120 s idle. Count new connections with `reqwest::ClientBuilder::connection_verbose(true)`.
4. **Boot phases.** Timestamps for setup start, window built, JS mounted (via an invoke), `restore_session` begin and end, gateway TCP/TLS/upgrade/Ready, Ready byte size, and the first list paint. Do one cold boot (after a reboot) and one warm boot on Windows.
5. **Server-side timings.** Add a tracing span to `send` (messages), `token` (voice) and `build_ready`. Note LiveKit `connectTime` per join (already logged: 674 ms) before and after `use_ice_lite`.
6. **Blip tests.** Wi-Fi off for 5, 30 and 60 s, and laptop sleep and resume. Record time to `Reconnecting` and `Connected` for the gateway, LiveKit `Reconnecting` and `Reconnected` for voice, and whether voice survives past 45 s.
7. **Device open on Windows.** Time `find_device`, `supported_*_configs` and `build_*_stream` separately for Sonar's virtual endpoints (8 ch at 96 kHz output) against the Arctis directly.
8. **Switch timing.** A → B: the close duration, the gap with no audio, and the full B join, using item 1's fields.
