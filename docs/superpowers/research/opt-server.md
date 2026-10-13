# Pulse: server, wire and infrastructure optimisation findings

Researched 2026-10-13. Read-only review of `server/src/**`, `server/migrations/*.sql`, `protocol/src/**`,
`deploy/*`, `deploy/server.Dockerfile`, `.github/workflows/release-server.yml`, the client's wire code
(`client/src-tauri/src/{gateway,api,outbox,commands}.rs`, `voice/mod.rs` join path) and
`docs/superpowers/specs/2026-10-09-deploy-design.md`. Query plans were checked with `EXPLAIN QUERY PLAN`
against the local dev `pulse.db`, opened read-only. Nothing was built and prod was not touched.

**Scale assumptions used for every estimate:** 10 users now, up to about 40 later. Two or three servers,
about 12 server channels, 15 to 40 DMs/groups. RTT 42-160 ms. One A1 OCPU, which is one Neoverse-N1
core with no SMT, so the guest sees 1 vCPU. LiveKit (Go, GOMAXPROCS=1), Caddy, the tokio runtime (1
worker), 8 sqlx SQLite worker threads and argon2 all share that one core.

**The main conclusion:** at this scale the server burns almost no CPU. User-visible latency is set by
**round trips** (TCP and TLS handshakes, sequential requests, slow start) and by **failure modes**
(deploys, restarts, Oracle reclaiming the VM). Most of the high-impact items below are about RTTs and
robustness, not CPU. The CPU and bytes items are real but small, so they are ranked Medium or Low.

---

## HIGH

### H1. `docker stop` SIGTERM is ignored: every deploy waits 10 s, then SIGKILLs, and clients never get the 1012 close
- **What and where:** `server/src/lib.rs:59` waits only on `tokio::signal::ctrl_c()` (SIGINT).
  `deploy/server.Dockerfile:27` uses an exec-form `ENTRYPOINT`, so `pulse-app-server` is **PID 1** in the
  container. `deploy/compose.yml:11-30` sets no `init: true` and no `stop_signal`.
- **Current cost:** Docker stops a container with SIGTERM. Inside a PID namespace, PID 1 **ignores**
  every signal it has no handler for, and tokio installs a SIGINT handler only. So on every
  `deploy.sh` → `docker compose up -d`, the old server keeps running until the 10 s grace period ends.
  Then it is SIGKILLed. `hub.close_all(1012)` (lib.rs:61) never runs, so clients see a dead TCP
  connection instead of "server restarting". They notice it through a socket error, or after up to 75 s
  of silence (`gateway.rs:18`) if the FIN or RST is lost. Each deploy therefore adds about 10 s of
  downtime, plus clients reconnecting late into a server that is already up.
- **Change (pick one):**
  - (a) In `deploy/compose.yml` under `pulse-server`, add `stop_signal: SIGINT` (zero code).
  - (b) In code, `select!` on `ctrl_c()` and `tokio::signal::unix::signal(SignalKind::terminate())`.
  - Also consider `stop_grace_period: 5s`.
- **Expected gain:** about 10 s less downtime per deploy, and clean 1012 closes so clients start
  reconnecting at once (backoff attempt 0 = 0.8-1.2 s).
- **Risk:** very low. (a) changes only which signal Docker sends.
- **Measure:** run `time docker compose stop pulse-server` before and after (expect about 10.x s, then
  under 1 s). Locally: `docker run` the image, then `docker stop` and time it. Prod: compare the
  duration of `just deploy` with Greg's OK.

### H2. Voice presence is wiped by every server deploy/restart, while people are still in calls
- **What and where:** `server/src/voice/state.rs:1-3` (in-memory only; "rebuilding from LiveKit's
  ListParticipants at startup is a follow-up"). `server/src/voice/mod.rs:9-21` closes open sessions at
  startup. `server/src/lib.rs:43`.
- **Current cost:** every `just deploy` restarts `pulse-server`, but LiveKit, and the calls in it, keep
  running. After the restart `VoiceState` is empty, and LiveKit does not re-send `participant_joined`. So
  every client's Ready (`socket.rs:168-179`) says nobody is in voice until people leave and rejoin.
  `voice_sessions` stats also record zero-length sessions for everyone who was in a call. Deploys happen
  often during active development, so this is a visible correctness failure, not a rare edge case.
- **Change:** at startup, after `reconcile_on_startup`, call LiveKit's RoomService over loopback.
  `livekit-api` already depends on it; enable the `services-tokio` (or equivalent) feature for
  `RoomClient`, pointed at `http://127.0.0.1:7880`. Call `list_rooms`, then `list_participants` per room,
  and rebuild `VoiceState` (identity = user id, room = channel id, sid = participant sid). Instead of
  closing sessions at zero length, reopen `voice_sessions` rows for participants who are still present.
- **Expected gain:** voice presence survives deploys and stats stay right. One loopback RPC per room at
  startup costs a few ms.
- **Risk:** low to medium. Startup must not fail if LiveKit is down: log and continue empty, as today.
  A new crate feature means a new dependency surface.
- **Measure:** integration test with LiveKit dev: join, restart the server, then check that Ready.voice
  contains the participant. Prod: one deploy during a call, with Greg's OK.

### H3. Oracle Always Free idle reclamation, and an unused free second OCPU
- **What and where:** this is about the infrastructure, not one file. The deploy spec (§1, lines 9-11)
  planned for **2 OCPU / 12 GB**, but prod runs 1 OCPU / 6 GB.
- **Current cost:**
  - **(a) Reclamation.** Oracle may reclaim an *Always Free* instance as idle if, over 7 days, the 95th
    percentile CPU is under 20%, network use is under 20%, and (A1 only) memory use is under 20%. A
    10-person chat server on 6 GB uses roughly 0.3-0.6 GB of RAM and a few % of CPU, so it **meets all
    three conditions**.
  - **(b) Unused CPU.** The current Always Free A1 allowance is 1,500 OCPU-hours and 9,000 GB-hours per
    month, which is **2 OCPU / 12 GB**. Half of the free CPU is unused, on a box where LiveKit, Caddy,
    argon2 and the server all share one core.
- **Change:**
  - (a) Upgrade the tenancy to Pay-As-You-Go. Resources stay free within the Always Free limits, and
    secondary sources report that this exempts the instance from idle reclamation. Set a $1 budget
    alert.
  - (b) Resize the A1.Flex shape to 2 OCPU / 12 GB. This needs a stop, an edit of the shape, and a
    start; it can fail with "out of capacity", and PAYG reportedly makes capacity easier to get.
- **Expected gain:**
  - (a) removes a "the whole service disappears" failure mode.
  - (b) doubles CPU headroom and roughly doubles LiveKit's forwarding capacity (see M6). LiveKit and the
    server/Caddy no longer steal time from each other: an argon2 login or a backup `VACUUM INTO` no
    longer causes audio jitter.
- **Risk:** (a) a credit card is on file. If anything later leaves the free limits, it costs money. (b)
  costs a few minutes of downtime, and capacity may not be available.
- **Measure:** OCI console metrics for CPU p95, memory % and network over 7 days, to confirm the idle
  criteria. Before and after the resize, record `mpstat 1` and LiveKit's Prometheus `process_cpu_seconds`
  during a call.
- **Also verify:** that backups really run on prod. If `R2_*`/`AGE_RECIPIENT` are empty,
  `deploy/backup.sh:24-27` exits 0 and silently skips. Reclamation without backups means total data loss.

### H4. Every REST call after 90 s idle pays a fresh TCP+TLS handshake (+2 RTT = 84-320 ms), including sending a message and joining voice
- **What and where:**
  - The client's `reqwest::Client` (`client/src-tauri/src/api.rs:121-131`) uses the default
    `pool_idle_timeout` (90 s), so pooled connections are dropped after 90 s idle.
  - The `reqwest` features (`client/src-tauri/Cargo.toml:26`) are `json`, `rustls-tls` only: no
    `http2`, so REST is HTTP/1.1, and any two concurrent requests open two connections.
  - Caddy's server-side idle timeout is 5 min.
- **Current cost:** in a chat app most sends come after more than 90 s of quiet. Each such
  `POST /channels/{id}/messages`, `POST /voice/{id}/token` or `mark_read` costs
  TCP (1 RTT) + TLS 1.3 (1 RTT) + request (1 RTT), so 3 RTT = **126-480 ms instead of 42-160 ms**.
  Concurrent calls, such as `list_messages` and `mark_read` on a channel switch, can each open their own
  connection. The always-open gateway WebSocket is a separate connection and does not help.
- **Change, smallest first:**
  - (1) `.pool_idle_timeout(Duration::from_secs(280))` (just under Caddy's 5 min) and
    `.tcp_keepalive(Duration::from_secs(30))`. Also enable the reqwest `http2` feature, so ALPN picks h2
    with Caddy and one connection multiplexes everything. Add `.http2_keep_alive_interval(30s)` with
    `.http2_keep_alive_while_idle(true)`, which keeps the h2 connection warm through home NATs.
  - (2) Optional, larger: send messages over the gateway socket (a `ClientFrame::Send` with a nonce, an
    ack through `MessageCreated`). This removes HTTP headers (about 300-500 B, including the bearer
    token) and per-request auth (a SHA-256 plus a `sessions` lookup) from the hottest path.
- **Expected gain:** −2 RTT (84-320 ms) on the first action after each idle period, which is most
  sends and every voice join.
- **Risk:**
  - Low for (1). Some home or mobile NATs drop idle TCP after 2-5 min; that is what the h2 PING
    keepalive covers. Cost: about 100 bytes every 30 s per client.
  - (2) is a protocol change and affects the outbox's idempotency.
- **Measure:** client log timestamps around `api.send_message` after 2 min idle, before and after (with
  `RUST_LOG=reqwest=debug,hyper_util=debug`, look for "connecting to"). Prod: a temporary Caddy access
  log with `{http.request.tls.resumed}` and the proto, to count new connections (Greg's OK).

### H5. Startup and voice join are serial chains of round trips; cut the RTT count, not server CPU
This answers "what's the server's share of the ~1.8 s join": the server's compute share is about 1-5 ms.
The rest is RTTs.
- **Voice join** (`client/src-tauri/src/voice/mod.rs:380-394`, livekit 0.9.3, server v1.13.7). I checked
  the SDK source in `~/.cargo/registry/.../livekit-0.9.3`. `single_peer_connection` defaults to true,
  and the publisher offer goes in the join URL. The server has supported `/rtc/v1` since v1.9.1
  (2025-09-05). `enable_sctp_snap` is on, so the SCTP handshake costs 0 RTT. The sequential RTTs are:

  | Step | RTT | Notes |
  |---|---|---|
  | `POST /voice/{id}/token` | 1, or 3 if cold (H4) | Server work: 1-3 SELECTs plus one HMAC JWT, well under 1 ms |
  | TCP to `voice.<domain>` | 1 | Separate host, never pooled |
  | TLS 1.3 | 1 | |
  | WS upgrade, JoinRequest in URL → JoinResponse and SDP answer | 1 | |
  | ICE connectivity check | about 1 | |
  | DTLS 1.2 (pion) | 2 | |
  | publish: AddTrack → TrackPublished, then renegotiation offer/answer | about 2 | |
  | **Total** | **about 9-11** | **0.4-0.5 s at 42 ms; 1.4-1.8 s at 160 ms**, plus client device and PeerConnection setup |

  This matches the observed 1.8 s on a slow path.
- **Changes in my layer:**
  - (a) Fetch the voice token in parallel with audio-device start-up, or prefetch it on channel hover
    or focus. `TOKEN_TTL` is 10 min (`voice/routes.rs:20`), so a cached token is fine; the server is
    unchanged. Saves 1-3 RTT.
  - (b) Apply H4, so the token call never pays a handshake.
  - (c) Keep signalling through Caddy. It adds well under 1 ms on loopback, and LiveKit can't terminate
    TLS itself, so going "direct to 7880" would mean plain `ws://` and no gain. Serving it at
    `pulsechat.co.za/rtc*` instead of `voice.` would save only the first DNS lookup, because
    tungstenite opens its own TCP connection anyway.
  - (d) Make sure the ICE pair is direct UDP to 7882, not TCP 7881 or a TURN relay. See M5.
- **App startup** (`commands.rs:193`): `restore_session` awaits `GET /me` (cold: TCP+TLS+req = 3 RTT)
  and only then spawns the gateway (TCP+TLS+upgrade+Hello→Ready = 4 RTT, plus the Ready transfer, see
  M1). That is about 7-8 RTT = **0.3-1.3 s**. `Ready.me` already carries the user, and close 4001 already
  means "logged out". Start the gateway at once, in parallel with `/me` or instead of it, and treat 4001
  as invalid. Saves about 3 RTT (130-480 ms) on every app start.
- **Risk:** low. (a) needs care that a token prefetched for a channel you can no longer see is harmless
  (LiveKit would admit you to the room). Keep the TTL short, or prefetch only on click-down.
- **Measure:** add `Instant` timestamps per phase in `connect()` (token, `Room::connect`, publish) and
  in `restore_session`→Ready. Compare at 42 ms (home fibre) against 160 ms (mobile hotspot), before and
  after. Server side: temporary Caddy access-log `duration` for `/voice/*/token`.

---

## MEDIUM

### M1. Ready is sent whole on every (re)connect, uncompressed; above about 14 KB it costs extra slow-start RTTs
- **What and where:**
  - `server/src/gateway/socket.rs:133-193` (`build_ready`) and `:47-55` (JSON text frame).
  - `protocol/src/gateway.rs:46-64` (Ready).
  - The client clears its history markers on reconnect (`ui/.../TextChannel.svelte:48-66`), so every
    visited channel refetches 50 messages.
- **Current cost, estimated from field sizes:** a Message is about 250 B of JSON plus its content
  (three 26-char ULIDs, an RFC 3339 timestamp, long key names). For 10 users, 2 servers, 12 server
  channels and 15 DMs:

  | Part of Ready | Size |
  |---|---|
  | channels | about 3 KB |
  | members | about 2.2 KB |
  | people | about 1.4 KB |
  | read_states | about 3.2 KB |
  | latest | about 4.5 KB |
  | dm_members | about 1.5 KB |
  | **Total** | **about 16 KB now; about 50-70 KB at 40 users with 40 DMs** |

  Linux's initial congestion window is 10 segments (about 14 KB, a little more after the TLS
  handshake), so anything bigger arrives over **2-3 RTTs** on a new connection (+42-320 ms). JSON full of
  repeated ULIDs typically compresses 4-6x. No compression is possible today: tungstenite 0.27/0.29 has
  no permessage-deflate, and Caddy can't add it.
- **Change:**
  - (a) Send Ready as a **binary frame, zstd-compressed** (application-level: `zstd::encode_all`, level
    3), with the client advertising support in Hello (`compress: ["zstd"]`). Events stay JSON text; they
    are 300-600 B each, and per-message compression gains little without a shared context.
  - (b) Shorten Ready instead: drop `last_seen_at` for online users, and send `latest` as ids only
    when the client has them cached. This is optional.
  - (c) Add `?after=<id>` to `GET /channels/{id}/messages` so a reconnect fills only the gap, instead of
    discarding history.
  - Discord moved its gateway to zstd streaming compression plus delta "passive sessions" and cut
    gateway bandwidth by about 40% (Discord engineering blog, Sept 2024).
- **Expected gain:** −1 to −2 RTT per connect and reconnect once Ready passes about 14 KB, which is soon.
  About 4-5x fewer gateway bytes on connect. Fewer refetches after flaky Wi-Fi.
- **Risk:** low to medium. This is a protocol change; gate it on the Hello capability so old clients
  still get JSON. zstd at level 3 costs well under 1 ms of CPU for 60 KB.
- **Measure:** log `text.len()` of the Ready frame and the duration of `build_ready` (server, at
  `tracing::info`). On the client, measure the time from `connect_async` returning to Ready parsed. Prod:
  one log line per connect (Greg's OK to deploy the log line).

### M2. One message send = about 10 SQL statements in 3-5 separate autocommit transactions, each fsync'd (synchronous=FULL)
- **What and where:**
  - `server/src/messages/routes.rs:138-232`: INSERT message at 193, `mentions::store` (one autocommit
    per mention, `mentions.rs:88-97`), `DELETE FROM dm_hidden` at 204, read-point upsert at 209.
  - `server/src/db.rs:9-13` never sets `synchronous`. sqlx's default (and SQLite's) is **FULL**
    (confirmed in `sqlx-sqlite-0.8.6/src/options/mod.rs:187`).
- **Current cost:** in WAL with FULL, every commit fsyncs the WAL. The OCI boot volume is network block
  storage (iSCSI or paravirtualised), where fsync is about 1-5 ms. A send with 2 mentions makes 4-5
  commits; DELETE and upsert statements that change nothing don't write frames. That is about 4-15 ms
  of the send path, run serially before the HTTP response and before the fan-out. Same pattern for:
  - `remove_member` (`dms/routes.rs:188-240`, 5+ autocommits);
  - `join` (`servers/routes.rs:202-205`: `init_point` per channel on a bare connection);
  - `create_dm` user checks.
  It also isn't atomic: a crash between the INSERT and `mentions::store` leaves a message with lost
  mentions.
- **Change:**
  - (a) `.synchronous(SqliteSynchronous::Normal)` in `db.rs`. In WAL mode this is corruption-safe; only
    the last commits before an *OS crash or power loss* can be lost, not on a process crash.
  - (b) Wrap each handler's writes in **one** transaction started with `BEGIN IMMEDIATE`
    (`conn.begin_with("BEGIN IMMEDIATE")`, available in sqlx-sqlite 0.8.6). Insert all mentions with
    one multi-row `INSERT ... VALUES (?,?),(?,?)`.
  - (c) Run the `dm_hidden` DELETE only when `ch.kind.is_private()`. Today it takes the write lock on
    every server-channel send and full-scans `dm_hidden`, which has no `channel_id` index (see L2).
- **Expected gain:** send-path DB time drops from about 5-15 ms to about 0.5-1 ms. That is small next to
  one RTT, but it is pure server latency on every message, and it fixes atomicity.
- **Risk:** low. With NORMAL, a power loss can roll back the last second of messages. Litestream or
  the nightly backups bound this anyway.
- **Measure:** wrap the send handler in `Instant::now()` and log `db_ms`. Locally,
  `strace -c -e fsync,fdatasync` the server while sending 100 messages, before and after. Prod: run
  `fio --name=f --rw=write --bs=4k --size=4m --fdatasync=1` on the boot volume, read-only for data
  (it writes a scratch file in /tmp), with Greg's OK, to get the real fsync latency.

### M3. Deferred transactions that read and then write can fail instantly with SQLITE_BUSY (busy_timeout doesn't apply)
- **What and where:**
  - `servers/routes.rs:357-...` (`create_dm`: a SELECT on the existing DM, then INSERT in the same tx).
  - `create_channel` (`SELECT MAX(position)`, then INSERT).
  - `dms/add_members` (INSERT first, which is fine).
- **Current cost:** `s.db.begin()` issues a plain `BEGIN` (DEFERRED). If another connection commits a
  write between this transaction's first read and its first write, SQLite returns `SQLITE_BUSY`
  (BUSY_SNAPSHOT) immediately: the busy handler can't help, because the read snapshot is stale. The user
  gets a 500. This is rare at 10 users, but it is a real failure mode under bursts such as the reconnect
  herd, or `mark_read` storms after a deploy.
- **Change:** use `begin_with("BEGIN IMMEDIATE")` for every transaction that will write. Optionally
  split the pool: a **writer pool with `max_connections(1)`** plus a reader pool of 4. Writers then
  queue FIFO inside tokio, not in SQLite's sleep-and-retry busy handler (backoff steps of
  1, 2, 5, 10 … 100 ms). This is the standard single-writer pattern.
- **Expected gain:** removes a class of spurious 500s. Under write contention, waits are µs-level and
  FIFO instead of busy-handler sleeps of up to 100 ms.
- **Risk:** low. With a single-writer pool, a long write (`VACUUM INTO` runs in a separate CLI process,
  so not that one) would block other writers. Keep writes short.
- **Measure:** grep the logs for `database is locked` / code 5 / 517. Locally, a load test of 20
  concurrent `create_dm` and `send` calls (k6 or a small tokio script) checking the 500 count.

### M4. Ready and reconnect: N+1 queries (about 3 + 2×servers + 3×DMs + rooms)
- **What and where:**
  - `socket.rs:142-148` (2 queries per server), `:150-166` (1 query per DM for members), `:170`
    (`channel_for` per voice room).
  - `reads.rs:91-98` (`latest_for`: one `load` plus one mentions query **per DM**).
  - `mentions.rs:48-63` (one query per @name).
- **Current cost:** with 2 servers and 15 DMs that is about 55 statements; with 40 DMs about 130. sqlx's
  SQLite driver ships every statement to a per-connection worker thread and back. On a single vCPU those
  hops are context switches, so expect about 30-80 µs per trivial query, or 2-10 ms per Ready. After a
  deploy, every client reconnects within about 1-2 s, so about 40 × 10 ms of CPU is taken from LiveKit
  in a burst.
- **Change:** set-based queries:
  - all channels in one `SELECT ... FROM channels WHERE server_id IS NOT NULL`;
  - all server members in one join, grouped in Rust;
  - DM members in one `SELECT channel_id, user_id FROM channel_members WHERE channel_id IN (SELECT
    channel_id FROM channel_members WHERE user_id=?)`;
  - `latest_for` as one query with a window or `MAX(id)` join, plus one batched `mentions::for_messages`;
  - mention resolution as one `WHERE username IN (...)`.
- **Expected gain:** about 55-130 statements become about 10. Ready build time drops 3-5x. A small
  absolute gain now; it matters for the reconnect herd and at 40 users.
- **Risk:** low, but the privacy filtering must stay identical. `tests/privacy.rs` covers it.
- **Measure:** `tracing` span timing on `build_ready` (log `elapsed_ms` and the statement count), and a
  local bench with 40 users and 40 DMs seeded.

### M5. livekit.yaml for an audio-only, single-node, 1-OCPU box
- **What and where:** `deploy/livekit.yaml:4-20`. Defaults were checked against
  `livekit/livekit pkg/config/config.go` `DefaultConfig`.
- **Observations:**
  - **`rtc.udp_port: 7882` (UDP mux):** good. One port, no per-participant port allocation, and simple
    firewalling.
  - **TURN UDP 3478:** in `iceServersForParticipant` (`pkg/service/roommanager.go`), embedded TURN/UDP
    is also handed to clients as **the STUN server**, so they don't use Google STUN. Keep it.
    TURN/TLS (`tls_port`) defaults to 0, so no malformed `turns:` URL is emitted.
  - **ICE-lite:** the sample config says `use_ice_lite: true` "will speed up ice connection, but might
    cause connect issue if server running behind NAT". OCI is 1:1 NAT: the public IP is not on the NIC.
    Expected gain is at most about 1 RTT on the ICE phase. **Measure-first A/B only.**
  - **`room.enabled_codecs`:** the default lists 10 codecs (PCMU, PCMA, Opus, RED, VP8, H264, VP9, AV1,
    H265, RTX). Restricting them to `audio/opus` + `audio/red` shrinks SDPs and the JoinResponse, a few
    KB on the join path. Low gain, very low risk.
  - **`rtc.node_ip`:** with `use_external_ip: true`, LiveKit STUN-discovers its IP at **startup** only,
    not per join. Pinning `node_ip` (with `use_external_ip: false`) removes a boot-time dependency on
    external STUN. This is a failure-mode fix, not latency. It needs a reserved public IP.
  - **`room.departure_timeout` (20 s) and `empty_timeout` (300 s):** these don't affect join latency.
    Leave them.
  - **`audio.update_interval` (500 ms):** sets speaking-indicator lag. 250 ms is snappier at the cost of
    2x speaker-update signal frames. A UX choice, measure first.
  - **`prometheus.port: 6789`, bound to loopback:** enable it for measurement; nothing is exposed
    externally (firewall).
- **Expected gain:** small and certain for codecs; a possible RTT from ICE-lite; better observability.
- **Risk:** ICE-lite could break connectivity behind NAT, so test it on a staging LiveKit or a quiet
  evening. The other changes are low risk.
- **Measure:** LiveKit Prometheus (`livekit_participant_join_total`, packet/NACK counters,
  `process_cpu_seconds_total`). Client-side, the time from `Room::connect` start to `Connected`, and the
  selected candidate pair type from `room.get_stats()` / webrtc-internals equivalents: host/srflx UDP is
  good; TCP or relay is bad.

### M6. LiveKit capacity on 1 OCPU, plus the kernel UDP buffers it asks for
- **Capacity reasoning:** LiveKit's audio-only benchmark is 10 publishers to 3,000 subscribers on a
  `c2-standard-16` (16 vCPU): **959k packets/s out at 80% CPU**, about 60k pkt/s per x86 hyperthread.
  Assume an A1 core forwards a conservative **25-40k pkt/s** with headroom; that has to be measured.
  Opus at 20 ms is 50 pkt/s per active speaker, and outbound = 50 × speakers × (N−1).
  - **Everyone with an open mic, no DTX:** N=10 → 4.5k pkt/s; N=20 → 19k; N=25 → 30k. **About 20-25
    people is the ceiling on 1 OCPU.**
  - **With DTX** (the client has `dtx: true`, `voice/mod.rs:891`) and about 3 people speaking at once:
    N=40 → about 6k pkt/s. **1 OCPU carries a few dozen people comfortably.** H3's second OCPU roughly
    doubles everything.
  - **Bandwidth:** 64 kbps Opus plus RED is about 130-150 kbps per speaker per subscriber, so 3
    speakers × 39 listeners ≈ 17 Mbps out. Far below the A1's network limit and OCI's 10 TB/month free
    egress.
- **UDP buffers:** Ubuntu's default `net.core.rmem_max` is 212992. LiveKit logs a startup warning that
  suggests 5,000,000. Community deployment recipes use 7,500,000 for both `rmem_max` and `wmem_max`, set
  on the **host** (the sysctls are read when the socket opens). On a single vCPU, any scheduling stall
  (argon2, `VACUUM INTO`, Caddy cert renewal, the Docker healthcheck spawning bash) lets the socket
  queue fill. A ~200 KB buffer holds only about 100-250 packets in skb truesize terms: about 100 ms at
  1.5k pkt/s.
- **Change:** in `setup.sh`, write `/etc/sysctl.d/99-livekit.conf` with
  `net.core.rmem_max=7500000` and `net.core.wmem_max=7500000`, then `sysctl --system`, before LiveKit
  starts. Also lower `ARGON_SLOTS` (`auth/password.rs:26`) from 2 to 1 on a 1-vCPU host.
- **Expected gain:** fewer drops, and fewer NACK/PLC artefacts, under bursts and CPU stalls. It also
  removes the LiveKit startup warning.
- **Risk:** negligible. The memory cost is a cap, not an allocation.
- **Measure:** on prod, with Greg's OK: `docker compose logs livekit | grep -i buffer` (the warning), then
  `nstat -az UdpRcvbufErrors UdpInErrors` before and after a busy call. Zero RcvbufErrors is the goal.

### M7. Outbox retry after a proxy 502 can duplicate a message; the server ignores the nonce for dedupe
- **What and where:**
  - `client/src-tauri/src/api.rs:30-39` (`never_arrived` treats 502/503 as retryable and notes "a
    duplicate is possible").
  - `outbox.rs:97` retries.
  - The server's `send` (`messages/routes.rs:138`) never stores or checks `req.nonce`.
- **Current cost:** a reset during a deploy, or a Caddy 502 after the upstream processed the request,
  produces a double post. Deploys are when this happens, and H1 makes them longer and messier today.
- **Change:** add a nullable `nonce TEXT` column to `messages` with
  `UNIQUE INDEX (author_id, nonce) WHERE nonce IS NOT NULL`. In `send`, use
  `INSERT ... ON CONFLICT DO NOTHING`; on conflict, return the existing message. Idempotent retries can
  then retry on any 5xx or timeout.
- **Expected gain:** no duplicates, and the client can retry more aggressively, which improves
  perceived reliability.
- **Risk:** low: one migration and a partial index.
- **Measure:** a test that sends the same nonce twice and expects 1 row. On prod, count
  `messages GROUP BY author_id, content, created_at/1s HAVING count>1`, read-only against a backup copy.

---

## LOW

### L1. Fan-out clones and re-serialises the frame once per recipient connection
- **Where:** `gateway/hub.rs:182` (`frame.clone()`: a deep clone, including content of up to 4,000
  chars) and `gateway/socket.rs:48` (`serde_json::to_string` per connection).
- **Cost:** O(recipients) allocations and serialisations, about 2-10 µs each. That is about 0.3 ms per
  message at 40 recipients: negligible now, and the fix is trivial.
- **Change:** serialise once in `publish`/`sync_presence` into `axum::extract::ws::Utf8Bytes` (backed
  by `Bytes`, so a clone is a refcount). Make the queue `mpsc::Sender<Utf8Bytes>` and send
  `Ws::Text(bytes)` as-is.
- **Gain:** about 40x less fan-out work per event. Smaller per-connection queues, since 256 × a
  refcounted handle is cheaper than 256 owned frames.
- **Risk:** low.
- **Measure:** a criterion bench of `publish` to 40 fake connections.

### L2. Index and query nits (from `EXPLAIN QUERY PLAN` on the dev DB)
- **`DELETE FROM dm_hidden WHERE channel_id=?`** (`messages/routes.rs:204`) → **SCAN dm_hidden**. The
  PK is `(user_id, channel_id)` (`0002_dms.sql:5-10`). Add
  `CREATE INDEX dm_hidden_channel ON dm_hidden(channel_id)` and gate it on private channels (M2c).
- **`WHERE lower(username) = ?`** (`messages/mentions.rs:52,59`) → **SCAN users**. Usernames are
  already stored lowercase (`auth/routes.rs:146`), so use `username = ?` to hit the UNIQUE index.
- **`mentions_message`** (`0002_dms.sql:21`) duplicates the PK prefix of `(message_id, user_id)` (the
  plan uses `sqlite_autoindex_mentions_1`). It is a wasted write per mention, so drop it.
- **`messages`** is a rowid table with a TEXT PK, so every insert touches 3 b-trees (table, PK
  autoindex, `messages_channel`). `WITHOUT ROWID` would make it 2, but it is not worth a table rebuild
  now.
- **`sessions`** never deletes expired rows (`auth/session.rs`). Add a daily
  `DELETE FROM sessions WHERE expires_at < ?` (an index on `expires_at` is optional at this size).
- **Already good:**
  - message paging uses `messages_channel (channel_id, id)` as a range scan;
  - `states_for` unread and mention subqueries use `messages_channel` ranges;
  - `latest_for` uses the MAX optimisation on the covering index;
  - `voice_sessions_open` matches its UPDATE;
  - all tables are already `STRICT`.
- **Run `PRAGMA optimize`:** there is no `sqlite_stat1`. Run it on pool close (sqlx
  `optimize_on_close(true, None)`) or daily.

### L3. Remaining SQLite pragmas
- **`cache_size`:** the default is −2000 (2 MB per connection × 8). The DB is tiny, so leave it.
- **`temp_store=MEMORY`:** helps the few `ORDER BY` temp b-trees (members by `joined_at`, channels by
  kind/position). The gain is µs.
- **`mmap_size=256MB`:** helps read-heavy paths once the DB is above about 100 MB. Not yet.
- **`journal_size_limit=64MB`:** keeps the WAL from staying large after a bursty checkpoint.
- **Pool size:** 8 is fine for readers. See M3 for the writer split.
- **Statement cache:** 100 per connection by default. `format!`-built SQL is constant, so it caches
  fine; the `IN (...)` lists vary in arity but are rare.
- **Measure:** nothing user-visible; not worth a deploy alone. Fold these into the M2 change.

### L4. Heartbeat cadence and reconnect behaviour
- **Settings:** the client heartbeats every 30 s (`gateway.rs:14`) and declares the connection dead
  after 75 s (`:18`). The server times out after 60 s (`config.rs:34`). Each beat is about 20 B of JSON
  plus about 30 B of WS/TLS overhead, each way: negligible.
- **Behaviour:** half-open detection takes up to 75 s. Presence "offline" lags up to 60 s.
- **Option:** 20 s heartbeat, 45 s server timeout, 50 s client deadline.
- **After a 1012 close,** retry on a short fixed schedule (for example 0.5 s × 20) instead of
  exponential backoff (`backoff.rs`, 1 → 2 → 4 → 8 s). A 5 s restart can otherwise leave clients idle
  for an extra 3-7 s.
- **Gain:** faster recovery after deploys and network changes.
- **Risk:** a slightly bigger reconnect herd, which M4 makes cheap.

### L5. Caddy: compression, HTTP/2, HTTP/3, timeouts
- **Where:** `deploy/Caddyfile:5-13`.
  - **No `encode`.** Adding `encode zstd gzip` compresses the REST JSON (a message page is about
    15-20 KB for 50 messages; expect 4-5x smaller). It only helps once the client sends
    `Accept-Encoding`: enable the reqwest `gzip`/`zstd` features. WebSocket 101 responses are
    unaffected. BREACH risk is minimal: no secret is reflected next to attacker-controlled content, and
    the only token-bearing responses are login and register.
  - **HTTP/2** is already on in Caddy for TLS; the client needs the reqwest `http2` feature (H4).
  - **HTTP/3:** Caddy advertises `Alt-Svc: h3` by default, but `udp/443` is closed (`setup.sh:70`) and
    reqwest's h3 is experimental, so there is no gain. Leave it, or disable it to stop the useless
    Alt-Svc header.
  - **Upstream keep-alive** to `127.0.0.1:7890` is on by default. The extra loopback hop is well under
    1 ms.
  - **Access log:** off by default. Turn it on *temporarily* for measurements (duration, bytes, proto,
    TLS resumed).
- **Gain:** fewer bytes on history fetches, plus HTTP/2 multiplexing.
- **Risk:** low.

### L6. Docker image and build
- **Image:** `server.Dockerfile` uses `debian:bookworm-slim` (about 75 MB) plus the binary. It could
  move to `gcr.io/distroless/cc-debian12` (about 20 MB), but the healthcheck uses
  `bash /dev/tcp` (`compose.yml:26`), so it would need a `pulse-app-server healthcheck` subcommand.
  The gain is about 50 MB less to pull per deploy (faster deploys).
- **Release profile:** there is no `[profile.release]`. Adding `lto = "thin"`, `codegen-units = 1` and
  `strip = true` gives about 10-20% smaller and slightly faster binaries.
- **Build caching:** `COPY . .` comes before `cargo build` (`server.Dockerfile:4,12`), so any source
  change rebuilds every dependency, and the GHA cache can't help. Use `cargo-chef` or BuildKit cache
  mounts (`--mount=type=cache,target=/usr/local/cargo/registry` and `target`) for much faster CI
  images. That means faster deploys, not runtime gains.
- **Allocator:** glibc malloc is fine at this scale. Don't bother with jemalloc or mimalloc.

### L7. Container limits, logs, CPU governor
- **Logs:** `compose.yml:4-8` already rotates the json-file logs at 10 MB × 3 for all three services.
  Good. The `local` driver would compress them; optional.
- **No memory limits.** With 6 GB and no swap, a LiveKit leak could push the OOM killer onto
  pulse-server or dockerd. Add `mem_limit: 2g` for livekit, `512m` for pulse-server and `256m` for
  caddy as blast-radius caps, not tuning.
- **No CPU priority.** On 1 vCPU, consider `cpu_shares` (livekit 2048, the others 1024) so audio
  forwarding wins contention.
- **CPU governor:** A1 guests normally expose no cpufreq driver (Ampere Altra runs at a fixed
  3.0 GHz), so there is nothing to tune. Verify with
  `ls /sys/devices/system/cpu/cpu0/cpufreq/ 2>/dev/null || echo none`.

### L8. Smaller wire and protocol nits
- Typing and voice events go to every connection, including the sender's own sessions. Recipients
  could skip `user_id == me`; a few bytes.
- The sender receives its message twice: in the HTTP response body and in `MessageCreated` over WS.
  A few hundred bytes per send. If H4 (2) moves sends onto the gateway, this disappears.
- Each event goes through `audience_for`, which runs `load_channel` (1 query) plus, for private
  channels, a `channel_members` query (`gateway/audience.rs:37-48`). `Typing` also runs `channel_for`
  first (`socket.rs:257`), so 2-4 queries per typing tick. An in-memory channel→kind/members cache,
  invalidated on the few membership-changing routes, removes most per-event queries. Low priority.
- Field names: renaming JSON keys (`channel_id` → `c` and so on) would save about 25% of uncompressed
  bytes but hurts debuggability and TS bindings. **Not recommended**; M1's compression gets more for
  free.

---

## Measure first (before changing anything)

**Locally (no prod access needed):**
1. Ready size and build time: log `text.len()` and `elapsed` for `build_ready` with a seeded DB (10
   users / 15 DMs, then 40 / 40). This decides M1 and M4.
2. fsync count per send: `strace -f -c -e trace=fsync,fdatasync` on the server while scripting 100
   sends. This decides M2.
3. A contention test: 20 concurrent `create_dm` and send loops, counting 500s or "database is locked".
   This decides M3.
4. A voice join phase breakdown: timestamps in `voice/mod.rs::connect` (token / `Room::connect` /
   publish) against local LiveKit with `tc qdisc add dev lo root netem delay 40ms` and then 80ms
   (≈ RTT 80/160). Count RTTs per phase. This confirms H5's RTT table.
5. Cold vs warm REST: send after 2 min idle, with and without the H4 client settings.
6. SIGTERM: `docker run` the image, `docker stop`, and time it (expect about 10 s today). This
   confirms H1.

**On prod, only with Greg's OK (all read-only or temporary):**
1. `docker compose logs livekit | grep -iE "buffer|rmem|ice lite|external ip"` for the UDP-buffer
   warning and the discovered IP (M6, M5).
2. `nstat -az UdpRcvbufErrors UdpInErrors UdpSndbufErrors` before and after an evening call (M6).
3. `cat /proc/sys/net/core/rmem_max /proc/sys/net/core/wmem_max`, plus
   `ls /sys/devices/system/cpu/cpu0/cpufreq` (M6, L7).
4. OCI console: 7-day CPU p95, memory %, and network %, against the reclamation thresholds. Also check
   the tenancy type, Always Free or PAYG (H3).
5. Check that backups actually run: `systemctl list-timers pulse-backup.timer`,
   `journalctl -u pulse-backup --since -2d`, and `rclone lsf r2:<bucket>/daily | tail` (H3).
6. Fsync latency of the boot volume: `fio --name=fsync --filename=/tmp/fio.test --rw=write --bs=4k
   --size=8m --fdatasync=1`, then delete the file (M2). This writes 8 MB to /tmp; it is harmless, but it
   is a write.
7. `docker stats --no-stream` at idle and during a call (memory and CPU per container: H3, L7).
8. Time `just deploy` end to end, and note how long clients show "reconnecting" (H1, L4).
9. Temporarily enable LiveKit `prometheus.port: 6789` and a Caddy access log for one evening: join
   counts, packets and NACKs, CPU seconds; and REST `duration`, `proto` and new connections (H4, M5,
   M6).
10. During a test call, read the selected ICE candidate pair type from the client logs
    (`RUST_LOG=livekit=debug`). It should be UDP to :7882, not TCP :7881 or a relay (M5).

---

## Sources (accessed 2026-10-13)
- LiveKit config sample (`use_ice_lite`, `node_ip`, `udp_port`, `audio.update_interval`, TURN ports,
  prometheus): https://github.com/livekit/livekit/blob/master/config-sample.yaml
- LiveKit server defaults (`DefaultConfig`: TURN TLSPort 0, `EmptyTimeout` 300, `DepartureTimeout`
  20, enabled codecs): https://github.com/livekit/livekit/blob/master/pkg/config/config.go
- LiveKit ICE servers for participants (embedded TURN/UDP is used as STUN):
  https://github.com/livekit/livekit/blob/master/pkg/service/roommanager.go
- LiveKit `/rtc/v1` handler (`pkg/service/rtcservice.go:101-104`); single peer connection added in
  server **v1.9.1 (2025-09-05)**, fixes in v1.9.2 (2025-10-17): https://github.com/livekit/livekit/releases
- LiveKit Rust SDK 0.9.3 source (local registry): `RoomOptions.single_peer_connection` defaults to true
  (`room/mod.rs:475`), publisher offer sent with the JoinRequest (`rtc_engine/rtc_session.rs:529-560,
  723-729`), `enable_sctp_snap` (`:527`), v1→v0 fallback only on 404 (`livekit-signaling-0.1.4/src/lib.rs:555-614`).
- LiveKit Rust SDK changelog (single PC #888 2026-02-16; publisher offer with join #996 2026-06-03):
  https://releases.sh/livekit/livekit-rust-sdks/changelog
- LiveKit benchmark, audio-only (c2-standard-16, 10 pub / 3,000 sub, 80% CPU, 959k pkt/s out):
  https://docs.livekit.io/transport/self-hosting/benchmark/
- LiveKit ports and firewall (UDP mux, port ranges, TURN/TLS on 443):
  https://docs.livekit.io/transport/self-hosting/ports-firewall/
- LiveKit client protocol (two-PC model, publisher PC created on first publish):
  https://docs.livekit.io/reference/internals/client-protocol
- UDP buffer recommendation for LiveKit (warning suggests 5,000,000; recipe sets 7,500,000 on the host),
  lasuite-meet recipe commit: https://git.autonomic.zone/recipe-maintainers/lasuite-meet/commit/802a96e8496630bbe08c86b77f022500414c3999
- Oracle Always Free resources: idle reclamation criteria (7-day p95 CPU < 20%, network < 20%,
  memory < 20% for A1), A1 allowance of 1,500 OCPU-h / 9,000 GB-h = 2 OCPU / 12 GB:
  https://docs.oracle.com/en-us/iaas/Content/FreeTier/freetier_topic-Always_Free_Resources.htm
- PAYG upgrade avoiding idle reclamation (secondary, 2023):
  https://blog.51sec.org/2023/02/oracle-cloud-cleaning-up-idle-compute.html
- SQLite pragma `synchronous` (NORMAL is corruption-safe in WAL): https://www.sqlite.org/pragma.html#pragma_synchronous
- SQLite transactions (DEFERRED vs IMMEDIATE, BUSY on upgrade): https://www.sqlite.org/lang_transaction.html
- "Optimizing SQLite for servers" (Kerkour, via Simon Willison, 2024-03-31): WAL, `busy_timeout` 5000,
  `synchronous=NORMAL`, BEGIN IMMEDIATE so that `busy_timeout` applies:
  https://simonwillison.net/2024/Mar/31/optimizing-sqlite-for-servers/
- sqlx-sqlite 0.8.6 defaults (`synchronous` FULL, statement cache 100, `busy_timeout` 5 s,
  `begin_with`): local registry `sqlx-sqlite-0.8.6/src/options/mod.rs:111-212`,
  `src/connection/mod.rs:257`.
- Discord, "How Discord reduced websocket traffic by 40%" (Sept 2024: zstd plus Passive Sessions v2):
  https://discord.com/blog/how-discord-reduced-websocket-traffic-by-40-percent
- tungstenite 0.27/0.29 and axum 0.8 ws: no permessage-deflate feature (local registry `Cargo.toml`
  checked).
- reqwest 0.12 `pool_idle_timeout` default 90 s; `http2`/`gzip`/`zstd` are opt-in features: https://docs.rs/reqwest/0.12/reqwest/struct.ClientBuilder.html
