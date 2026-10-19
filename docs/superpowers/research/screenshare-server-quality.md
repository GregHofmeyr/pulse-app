# Pulse screen share ("Go Live"): server, network and quality research

**Date:** 2026-10-13 · **Scope:** what Discord-style screen sharing costs and needs on the server, network and quality
side. This is the base for a later spec. **Workspace:** `~/Projects/pulse-app` (read-only).
**Target:** 720p–1440p at 30–60 fps, 1–2 streamers at a time, up to ~6 viewers, optional stream audio. ~6 friends
now, a few dozen later.
**Prod:** OCI A1 (Ampere Altra, Neoverse-N1), JNB, **1 OCPU / 6 GB**. Caddy + axum/SQLite + LiveKit **v1.13.7**, all
on host networking. UDP mux 7882, TCP 7881, TURN/UDP 3478. Clients use Rust SDK `livekit` 0.9.3.

Labels used below: **[measured]** means I ran it on 2026-10-13. **[cited]** is a primary source (list at the end).
**[est.]** is my own engineering estimate, so check it in the spike.

---

## 0. TL;DR

- **The SFU is not the problem; pps and one shared core are.** The target load (2 streams × 6 viewers + voice) is
  about **9–12k packets/s and 80–100 Mbps out**. On 1 OCPU that is an estimated **30–45% of the core** for LiveKit
  alone, with Caddy, axum, argon2 and backups on the same core and GOMAXPROCS=1. It works, but with thin margins.
  **Resize to 2 OCPU / 12 GB** (still Always Free) before shipping screen share.
- **The first thing to break is CPU scheduling, then the kernel UDP buffers.** It is not the NIC (1 Gbps per OCPU) and
  not egress (10 TB/month free). The default `rmem_max` of 208 KB is smaller than a single 1440p screen keyframe.
  LiveKit asks for 16 MiB and warns below 5 MB.
- **Egress per hour of streaming:** one 1080p60 stream at 6 Mbps to 6 viewers is **~16 GB/h**. A realistic month for
  our group is 0.1–1.5 TB, so **$0**. Beyond 10 TB the JNB rate is **$0.05/GB** (Middle East & Africa zone). That is
  6× the US/EU rate, so set a budget alert.
- **Use the "watch stream" opt-in.** Set `auto_subscribe: false` (or unsubscribe video on publish) and
  `dynacast: true`. Then nobody pays for a stream they aren't looking at, and the sharer sends nothing while nobody
  watches. That is exactly what Discord does.
- **Traps in the Rust SDK** (read from the 0.9.3 source):
  - `adaptive_stream: true` makes the SFU start every viewer on the **LOW** layer, and the Rust SDK has no
    widget-size tracking to raise it. **Leave it off.**
  - The default screen-share encoding tops out at **1080p30 / 3 Mbps**, and the default simulcast layer is
    **half-res at 3 fps**. Set encodings explicitly.
  - Pulse plays audio through `NativeAudioStream`, outside libwebrtc's lip-sync, so **A/V sync of stream audio is
    our job**.

---

## 1. SFU load

### 1.1 What forwarding costs

LiveKit does not transcode. For every packet it receives it decrypts SRTP once. For every packet it sends, it
rewrites the header, encrypts SRTP and makes a `sendmsg` call. **Cost scales with packets out per second, not
resolution.** LiveKit says the same: capacity "depends on the number of published tracks, subscribers and data sent
to each subscriber" [cited: LK benchmark].

**LiveKit's official benchmark** (GCP `c2-standard-16`, x86, 16 vCPU; page undated, read 2026-10-13):

| Scenario | Pubs/Subs | Out | pps out | CPU |
|---|---|---|---|---|
| Audio-only | 10 / 3000 | 23 MB/s | 959k | 80% |
| 720p simulcast meeting | 150 / 150 | 93 MB/s | 763k | 85% |
| 720p livestream | 1 / 3000 | 531 MB/s (~4.2 Gbps) | 561k | 92% |

That works out to roughly 38–60k pps per x86 vCPU. The livestream row averages ~950 B per video packet out.
Memory is not reported. There is no published ARM/A1 benchmark, and I found no GitHub issue with ARM per-stream
numbers.

### 1.2 My measurements (LiveKit v1.13.7 image, `lk load-test` v2.18, 2026-10-13)

**Setup:**
- A local, isolated `livekit-server` pinned to **one i9-13900HX P-core** (`--cpuset-cpus=2`). The load generator ran
  on other cores over loopback.
- The "high" preset is 720p at ~1.9 Mbps, ~217 pps per video track (≈1.09 kB/packet).
- CPU comes from LiveKit's `process_cpu_seconds_total` over a 30 s steady-state window.
- Prod and the dev LiveKit were not touched.

| Run | Load | pps in | pps out | Out Mbps (subs' recv) | LiveKit CPU (1 P-core) | RSS | Loss |
|---|---|---|---|---|---|---|---|
| A | 2 video + 6 audio pubs, 6 subs (Pulse-like, 720p) | 853 | 3.0k | 11 | **6.2%** | 127 MB | 0 |
| B | 2 video, 60 subs, 3x3 layout (mid layer) | 160 | 7.4k | 58 | 10.1% | 264 MB | 0 |
| C | 2 video, 150 subs, 3x3 | 160 | 18.4k | 137 | 19.9% | 571 MB | 0 |
| D | 2 video, 150 subs, speaker (high layer) | 553 | 32.4k | 256 | **31.9%** | 520 MB | 0 |
| E | as D with 50 subs, cgroup quota 0.45 CPU | 551 | 10.9k | 92 | 14.6% | 240 MB | 0 |
| F | same with quota **0.20 CPU** | 553 | 9.9k | **74** (avg 1.8 → 1.5 Mbps) | 14.3% | 282 MB | 0 |

**What the runs show:**
- **~1.0–1.4% of a fast x86 core per 1k pps out.** Small loads cost more per packet (fixed overheads), large loads
  less.
- **Memory:** ~75 MB at idle, ~125 MB for a Pulse-sized room, ~3.5 MB per extra participant. On a 6 GB box,
  memory is irrelevant.
- **Run F:** when CPU is starved, LiveKit doesn't drop packets. Its congestion control sees the extra delay and
  moves viewers to lower layers, so the symptom is **blurrier or lower-fps streams**, not freezes. Without simulcast
  there is no lower layer to fall back to, and it shows up as loss and freezes.

**From x86 to A1 [est.]:**
- An A1 OCPU is one Neoverse-N1 core at 3.0 GHz with no SMT [cited: OCI shapes]. Its single-thread speed is roughly
  **2–2.5× lower** than a Raptor Lake P-core.
- Loopback also skips the virtio-net driver path that a real NIC adds.
- **Working figure: ~3–4% of one A1 OCPU per 1k pps out, and ~25–35k pps out at 100%.** This agrees with the
  earlier `docs/superpowers/research/opt-server.md` M6 estimate (25–40k). **Measure it in the spike.**

### 1.3 Packet rate and memory at 1080p60

- A 1080p60 stream at 6 Mbps with ~1.1–1.2 kB RTP payloads is **≈ 650–700 pps** steady state.
- Screen content is bursty:
  - A static desktop sends almost nothing.
  - A scroll or slide change sends a large delta frame.
  - A keyframe (on viewer join, layer switch or loss) is roughly 100–400 kB at 1080p and 200–600 kB at 1440p
    [est.]. That is 100–500 packets at once.
- Per viewer the SFU sends the same pps. Retransmissions (RTX/NACK) add 1–5% on Wi-Fi.
- **Per-track memory:** the SFU keeps a NACK ring buffer per video track (`packet_buffer_size_video`, default 500
  packets ≈ 0.75 MB). That is about **0.7 s of history at 1080p60** and **0.35 s at 1440p60 / 12 Mbps** (~1.4k pps).
  It is enough for RTTs under ~150 ms, which covers SA.

### 1.4 Can 1 OCPU carry 2 streams × 6 viewers plus voice?

**Packet budget for the target scenario [est.]:**
- **Video:** 2 streams × 6 viewers × ~700 pps (1080p60 6 Mbps) = 8.4k pps out. Ingress 1.4k pps.
- **Stream audio:** 2 × 6 × 50 pps = 0.6k pps.
- **Voice:** 8 in the call, DTX on, ~3 talking = 3 × 7 × 50 = ~1k pps.
- **RTX on Wi-Fi viewers:** +0.2–0.5k pps.
- **Total ≈ 10–11k pps out, 80–100 Mbps.** At 1440p60 / 12 Mbps: ~19k pps and ~150–170 Mbps.

| Resource | Need (1080p60 case) | 1 OCPU limit | Headroom | Breaks… |
|---|---|---|---|---|
| CPU (LiveKit) | ~30–45% of the core [est.] | 100%, shared with Caddy/axum/argon2/backup, GOMAXPROCS=1 | thin | **1st**, as jitter/quality drops during argon2 logins, `VACUUM INTO` or GC; at 1440p60 ×2 it is near saturation |
| Kernel UDP rx buffer | must absorb one keyframe burst + a 10–20 ms stall | **208 KB default** (`net.core.rmem_max`) | none | **2nd**, as drops → NACK/PLI storms → freezes. LiveKit asks for 16 MiB and warns below 5 MB [cited: mediatransportutil] |
| NIC bandwidth | 80–170 Mbps | 1 Gbps per OCPU [cited] | 6–12× | not a factor |
| NIC pps | 10–20k | not published by OCI | probably large | measure |
| Memory | ~150–300 MB LiveKit | 6 GB | huge | never |
| Egress quota | see §2 | 10 TB/month free | large | only at "few dozen" scale |

**Verdict:** 1 OCPU can carry it for 1080p, with weak isolation. Two 1440p60 streams to 6 viewers, plus a login
storm after a deploy, could exceed it.

**Would 2 OCPU help? Yes, clearly.**
- It doubles the CPU, and with GOMAXPROCS=2 LiveKit forwards on two threads.
- It isolates the SFU from argon2, SQLite and Caddy.
- It doubles NIC bandwidth to 2 Gbps.
- It stays inside Always Free: 1,500 OCPU-h and 9,000 GB-h a month is 2 OCPU / 12 GB.
- **Cost:** a stop, a shape edit and a start. Capacity may not be available, though PAYG reportedly helps.

This repeats H3 in `opt-server.md`; screen share turns it from "nice" into "should".

---

## 2. Bandwidth and network

### 2.1 Per-viewer bitrate for screen content

**Anchors:**
- LiveKit JS `ScreenSharePresets` [cited]: 720p15 1.5 Mbps · 720p30 2.0 · 1080p15 2.5 · **1080p30 5.0** · original
  7.0 Mbps @30.
- LiveKit Rust 0.9.3 `screenshare` presets [cited, source]: 720p15 1.0 · 1080p15 1.5 · **1080p30 3.0 Mbps**. The SDK
  scales by codec: VP9 ×0.85, AV1 ×0.7.
- YouTube live (H.264 min → recommended) [cited]: 720p60 3 → 8 · 1080p30 5 → 14 · **1080p60 6 → 17** · 1440p60
  8 → 34 Mbps. AV1/HEVC is ~25–30% lower. These targets are for buffered broadcast archive quality; real-time
  screen share runs nearer the minimums.
- **AV1 screen-content coding** cuts frame size by more than 25% on slides/text [cited: webrtcHacks 2023-12-19].
  libwebrtc enables it for screen-capture tracks with `contentHint = "detail"`.

**Recommended caps** (the encoder's max bitrate; Mbps; **[est.]**, tune in the spike):

| Res / fps | Text/code/UI cap (avg is much lower) | Gaming / video cap | VP9 | AV1 |
|---|---|---|---|---|
| 720p30 | 1.5 | 3 | ×0.85 | ×0.65–0.7 |
| 720p60 | 2 | 4.5 | ×0.85 | ×0.7 |
| 1080p30 | 3 | 5 | ×0.85 | ×0.7 (text ×0.55) |
| 1080p60 | 4 | **6–8** | ×0.85 | ×0.7 |
| 1440p30 | 4.5 | 8 | ×0.85 | ×0.7 |
| 1440p60 | 6 | **10–12** | ×0.85 | ×0.7 |

**Codec notes:**
- **H.264** is the only codec with universal hardware encode and decode: NVENC/AMF/QSV/VAAPI. The SDK ships NVENC
  (H.264/H.265/AV1) and VAAPI paths in `webrtc-sys`. It costs the streamer the least CPU at 1080p60+.
- **VP8** is software-only and expensive at 1080p60.
- **VP9** is better quality, but software encoding at 1440p60 is heavy.
- **AV1** is best for text. Hardware encode needs RTX 40+, RX 7000+ or Arc. Discord gates AV1 the same way and falls
  back to H.264 when any viewer can't decode it.
- **Average vs cap:** text streams average roughly **30–60%** of the cap (idle screens cost almost nothing).
  Games run at **85–100%**.

### 2.2 Server egress

**Egress ≈ Σ streams × viewers × average bitrate**, plus voice. Ingress is free. Per hour:
`GB/h = Mbps × 0.45`.

| Session | Out Mbps | GB/hour |
|---|---|---|
| 1 stream, 720p30 text, 4 viewers @ ~1 Mbps avg | 4 | 1.8 |
| 1 stream, 1080p60 game, 6 viewers @ 6 Mbps | 36 | **16.2** |
| 2 streams, 1080p60 game, 6 viewers each @ 6 Mbps | 72 | 32.4 |
| 2 streams, 1440p60 game, 6 viewers each @ 11 Mbps | 132 | 59.4 |
| Voice only, 8 people (for scale) | ~1–2 | ~0.5 |

### 2.3 Oracle network limits and pricing

- **A1.Flex: 1 Gbps per OCPU, max 40 Gbps** [cited: OCI Compute shapes]. 1 OCPU = 1 Gbps, 2 OCPU = 2 Gbps.
- **Outbound data transfer:** **first 10 TB/month free**, inbound free [cited: Oracle price-list API B93456,
  2026-10-13]. Over 10 TB, by zone:
  - North America/EU/UK: $0.0085/GB.
  - APAC/Japan/South America: $0.025/GB.
  - **Middle East & Africa (includes JNB): $0.05/GB.**

  A secondary blog claims Oracle dropped egress fees entirely in Feb 2026. Oracle's live price API still lists the
  tiers above, so treat that claim as false until shown otherwise.

**Monthly estimates [est.]:**

| Usage pattern | TB/month | Cost |
|---|---|---|
| Light: 2 evenings/wk × 2 h, 1 stream × 3 viewers @ 3 Mbps | ~0.1 | $0 |
| Typical: 25 evenings × 4 h, avg 1.3 streams × 4 viewers @ 5 Mbps | ~1.2 | $0 |
| Heavy (our group): 30 × 6 h, 2 streams × 6 viewers @ 8 Mbps | ~7.8 | $0 (close to the cap) |
| "Few dozen users", 3 parallel heavy sessions | ~20 | **~$500** (10 TB × $0.05/GB) |

Africa's egress price is the real cost cliff. **Mitigations:** opt-in viewing, dynacast, per-tier bitrate caps, an
OCI budget alert at $1, and a hard cap on concurrent streams per server.

### 2.4 Sharer's uplink

**Uplink = top layer + lower simulcast layers + stream audio + ~25% headroom** (for BWE probing, RTX and keyframe
bursts).

| Preset | Layers | Sustained up | Recommended free uplink |
|---|---|---|---|
| Smooth 720p60 | 4.5 + 0.8 (360p30) | ~5.4 Mbps | **≥ 8 Mbps** |
| Sharp 1080p30 | 3–4 + 0.8 (720p15) | ~4.9 Mbps | ≥ 8 Mbps |
| Gaming 1080p60 | 7 + 1.2 (540p30) | ~8.3 Mbps | **≥ 12 Mbps** |
| Source 1440p60 | 11 + 2 (720p30) | ~13 Mbps | **≥ 20 Mbps** |

- With dynacast, layers nobody watches are paused. With nobody watching at all, the uplink drops to ~0 after the
  5 s `dynacast_pause_delay`.
- SVC (VP9/AV1 L3T3) carries all layers in one stream at roughly **+10–30% over a single layer** instead of +25–40%
  for simulcast.

### 2.5 SA network realities

**Fibre:**
- Symmetric plans are now common: Vumatel, Frogfoot and Openserve all sell 50/50 and up. Openserve says it "scales
  up to 1 Gbps symmetrical".
- Older and cheap Openserve plans are asymmetric (20/10, etc.).
- Our 20–200 Mbps up is fine for every preset. Someone on 20/10 can only do Smooth 720p60.

**LTE/5G:** upload is typically 5–30 Mbps and swings a lot with cell load and time of day. LTE sharers should be
capped at 720p, and the uplink chosen from a measured BWE rather than a plan's advertised speed.

**Wi-Fi:** a viewer's last hop causes loss and jitter bursts. The fixes are NACK, simulcast fallback and an adequate
`packet_buffer_size_video`.

**Routing:**
- Oracle (AS31898) peers at **NAPAfrica Johannesburg and JINX at 100G** [cited: PeeringDB, 2026-10-13]. So do MTN,
  rain, Cybersmart and Telkom.
- Most SA eyeball traffic reaches JNB without leaving the country. Afrihost and Vodacom are not listed directly,
  so they go via their upstreams, but still in-country.
- RTT estimates [est.]: ~2–15 ms from Gauteng, ~20–30 ms from Cape Town or Durban.
- International paths (EU ~150–180 ms) only matter for travelling users.
- Data caps on LTE: a 1080p60 stream costs a viewer ~2.7 GB/h. Expose a "data saver" (720p cap) for viewers.

---

## 3. LiveKit features that matter

### 3.1 Simulcast vs SVC for screen share

| | Simulcast (H.264/VP8) | SVC L3T3(_KEY) (VP9/AV1) |
|---|---|---|
| Sharer CPU | N separate encodes (HW H.264 is cheap) | one encode, but mostly **software** at 1080p60+ (NVENC does no VP9 SVC; AV1 SVC on HW encoders is limited) |
| Uplink | +25–40% | +10–30% |
| SFU layer switch | needs a keyframe on the target layer (PLI) | temporal down/up and spatial down are instant; spatial **up** needs a keyframe with `_KEY` |
| Dynacast | pauses individual layers | can only pause the **whole** stream [cited: LK docs] |
| Rust SDK | `simulcast: true` (default) + `simulcast_layers` | set `scalability_mode: Some("L3T3_KEY")`. **Not automatic in Rust**, unlike JS |
| Viewer compat | universal | VP9 almost universal; AV1 decode needs a modern GPU/CPU |

Google Meet still uses VP8 simulcast for tab sharing [cited: webrtcHacks].

**Recommendation:** **H.264 simulcast, 2 layers**, as the default, because it is HW-encoded and universal. Add an
**AV1 option** for RTX 40/RX 7000 sharers when every watcher can decode it, the way Discord does. Revisit VP9 SVC
only if sharer CPU allows.

**Fix the Rust SDK's simulcast default.** For screen share it builds one extra layer at **half resolution, 3 fps**
(`screenshare::compute_default_simulcast_preset`). Under congestion a gaming viewer drops to 3 fps. Pass explicit
`simulcast_layers`:
- 720p30 under a game stream;
- 720p15 under a text stream.

### 3.2 Dynacast

- **What it does:** the SFU tells the publisher which layers have subscribers, and the client stops encoding the
  others. With 0 viewers, everything pauses.
- **Settings:** `RoomOptions.dynacast` is **false by default** in Rust 0.9.3; set it to true. The server default
  `video.dynacast_pause_delay` is 5 s.
- **Payoff:** it is the "streamer transmits only when at least one viewer is watching" behaviour from Discord's
  2024-03-05 post [cited]. Large saving on the sharer's uplink and CPU.

### 3.3 Adaptive stream (Rust SDK)

- In Rust, `adaptive_stream` is only a flag sent to the server. The SDK has **no video-element visibility or size
  tracking** (that is JS/Swift/Android behaviour).
- In server v1.13.7 (`pkg/rtc/subscribedtrack.go`), adaptive-stream subscribers **start at `VideoQuality_LOW`**
  until the client sends dimensions. **Leave `adaptive_stream: false`.**
- Do it by hand instead:
  - `RemoteTrackPublication::update_video_dimensions(w,h)` when the stream widget resizes;
  - `set_video_quality(High/Medium/Low)` for a viewer "quality" menu or data saver;
  - `set_enabled(false)` when the stream view is hidden (minimised, another tab). This pauses forwarding without
    unsubscribing, and is cheaper to resume than a re-subscribe.

### 3.4 Watch-stream opt-in

Pulse calls `Room::connect(..., RoomOptions::default())` (`client/src-tauri/src/voice/mod.rs:400`), which means
**`auto_subscribe: true`**. Every person in voice would receive every screen share. To get opt-in:
- set `auto_subscribe: false` and call `set_subscribed(true)` yourself for microphone tracks (and for screen-share
  tracks when someone clicks Watch); **or**
- keep auto-subscribe on and immediately `set_subscribed(false)` on `ScreenShare`/`ScreenShareAudio` publications in
  the `TrackPublished`/`TrackSubscribed` handler. This is simpler but costs a brief subscription and keyframe.

A server-side alternative: issue tokens whose `canSubscribe` is narrowed per track. That is more complex and not
needed.

### 3.5 Bandwidth estimation and congestion control

- **Server:** `rtc.congestion_control.enabled: true` is the default. The stream allocator moves each viewer between
  layers based on its downlink estimate. Screen share gets a **higher default priority** than camera video
  (`streamallocator/track.go`, `cPriorityDefaultScreenshare`).
- **`allow_pause`:** default false. Leave it false: we'd rather degrade than pause a stream.
- **`use_send_side_bwe`:** default false, TWCC-based. **A/B it in the spike;** it may react better to bursty
  high-bitrate video.
- **Publisher side:** libwebrtc GCC adapts the encoder. `degradation_preference` (Rust: `MaintainResolution` default
  for `Screenshare`) decides whether to drop fps or resolution.

### 3.6 Server config (livekit.yaml) for video

v1.13.7 defaults are verified from `pkg/config/config.go` and `pkg/sfu/receiver.go`.

| Key | Default | Recommendation |
|---|---|---|
| `rtc.packet_buffer_size_video` | 500 | 500 is enough for 1080p60. Use **1000** if 1440p60 ships (+~0.75 MB per track) |
| `rtc.pli_throttle` | low 500 ms, mid 1 s, high 1 s | maybe `high_quality: 500ms` (≤6 viewers, so keyframe spikes are cheap) to halve worst-case join black screen. Measure |
| `rtc.congestion_control` | enabled, allow_pause false | keep; A/B `use_send_side_bwe` |
| `rtc.allow_tcp_fallback` | true | keep |
| `room.max_participants` | 0 (unlimited) | set **25** (DoS/egress guard) |
| `room.enabled_codecs` | 10 codecs incl. VP8/H264/VP9/AV1/H265 | keep the video codecs. **Don't apply opt-server M5's opus+red-only trim** without adding `video/h264`, `video/vp8`, `video/av1` (and `video/vp9`) |
| `room.sync_streams` | false | `true` if relying on libwebrtc lip-sync (see 4.5) |
| `limit.subscription_limit_video` | 0 (unlimited) | 4 is a sane guard |
| `limit.num_tracks`, `limit.bytes_per_sec` | unset (no limit) | `bytes_per_sec: 50000000` (400 Mbps) as a guard against runaway egress |
| `video.dynacast_pause_delay` | 5 s | keep |
| `prometheus.port` | off | **6789 on loopback**, needed for every measurement below |

**Host side, not yaml:**
- `/etc/sysctl.d/99-livekit.conf`: `net.core.rmem_max=16777216`, `net.core.wmem_max=16777216`. That is what LiveKit
  requests (`mediatransportutil` `defaultUDPBufferSize`); the warning fires below 5,000,000. Same change as
  opt-server M6, but **more urgent with video**.
- Add `cpu_shares` priority for livekit if the box stays at 1 OCPU.

### 3.7 TURN and TCP fallback costs

- **Embedded TURN/UDP 3478:** relayed media goes client → TURN → SFU on the same host. That is roughly **2× packet
  handling (CPU) for that client**. Egress to the client is the same.
- **ICE/TCP 7881 and TURN/TLS** (deferred): head-of-line blocking. One lost segment stalls every later frame, so a
  stream over TCP on lossy Wi-Fi shows 200 ms – 1 s+ hitches. LiveKit says UDP is preferred for congestion and
  latency control [cited: LK deployment docs]. Expect the allocator to hold TCP viewers on a lower layer.
- **Plan:** log the selected candidate type per participant. If someone lands on TCP or relay, cap them at 720p.

---

## 4. Quality strategy

### 4.1 Presets

| Preset | Res / fps | Codec | Cap (avg) | Lower layer | Degradation | Content hint |
|---|---|---|---|---|---|---|
| **Smooth 720p60** | 1280×720 @60 | H.264 HW | 4.5 Mbps (3.5) | 640×360 @30, 0.8 Mbps | MaintainFramerate | motion |
| **Sharp 1080p30** (default for windows/desktops) | 1920×1080 @30 | H.264 HW (AV1 opt.) | 4 Mbps (1.5–2.5 on text) | 1280×720 @15, 1.0 Mbps | **MaintainResolution** | detail/text |
| **Gaming 1080p60** | 1920×1080 @60 | H.264 HW (AV1 opt.) | 7 Mbps (6–7) | 960×540 @30, 1.2 Mbps | **MaintainFramerate** | motion |
| **Source 1440p60** | 2560×1440 @60 | H.264/AV1 HW only | 11 Mbps (9–11) | 1280×720 @30, 2.0 Mbps | MaintainFramerate | motion |

**Rules:**
- Offer Source only when a hardware encoder exists and BWE shows ≥ 20 Mbps.
- Auto-pick Sharp for a code editor, browser or desktop, and Gaming when the captured window is a fullscreen game.
- Let the user override.
- Always set `TrackPublishOptions.video_encoding` explicitly. The Rust default computation caps screen share at the
  **1080p30 / 3 Mbps** preset whatever the capture size.

### 4.2 Content type and degradation

- **Text:** resolution matters, frame rate doesn't. Accept 5–15 fps under pressure.
- **Games:** frame rate matters. Drop to 720p before going under ~45 fps.
- Map these to `degradation_preference` plus the lower simulcast layer, so the SFU's fallback for a gamer is still
  30 fps.
- Desktop/window capture that ignores the content hint gets nothing from AV1 SCC. Check that libwebrtc enables
  screen-content tools for the desktop capturer track in the Rust SDK.

### 4.3 Keyframes and time-to-first-frame

- libwebrtc does **not** send periodic keyframes by default. It sends them on PLI/FIR.
- **On viewer join:** the SFU forwards nothing until it has a keyframe for that layer, then sends a PLI to the
  publisher (throttled per layer by `pli_throttle`).

**Time-to-first-frame [est.]:**

| Step | Time |
|---|---|
| Signalling and subscribe | 1–2 RTT, ~20–60 ms |
| PLI to publisher | ½ RTT |
| Encoder emits keyframe | next frame, 16–33 ms |
| Keyframe transmission | 150–400 kB at 6–11 Mbps ≈ **100–400 ms**, paced |
| Jitter buffer and decode | ~30–80 ms |
| **Total, typical** | **≈ 250–600 ms** |
| **Worst case** | **+1 s** if another viewer triggered a PLI within the last throttle window |

- A keyframe goes to *all* viewers of that layer, so each join gives everyone a brief bitrate spike. At ≤6 viewers
  this is fine.
- **Optional insurance:** a long periodic keyframe (10–20 s) helps recovery when PLIs are lost, but costs bandwidth
  on text streams. Only add it if the spike shows stuck streams.

### 4.4 Latency targets

**Budget for Pulse [est.]:**

| Component | Time |
|---|---|
| Capture | ≤ 1 frame (16 ms @60) |
| HW encode | 5–15 ms (SW 20–40 ms) |
| Pacer | 0–30 ms (bursts at keyframes) |
| Network sharer → JNB → viewer | 5–60 ms one-way in SA |
| SFU forwarding | < 5 ms |
| Jitter buffer | 20–80 ms (adaptive) |
| Decode | 5–15 ms |
| Render | ≤ 1 frame |
| **Total** | **~80–200 ms glass-to-glass** |

- **Target:** p50 ≤ 150 ms, p95 ≤ 250 ms inside SA. The "100–300 ms" figure for Discord is a community estimate,
  not a Discord number. Discord's own post says only that latency is kept "low enough that most users won't notice"
  and that Go Live is not synced with voice [cited].
- Don't set `room.playout_delay` for screen share.

### 4.5 Stream audio

**Encoding:**
- Opus **stereo, 96–128 kbps** (music/game). DTX off (avoids clipping quiet game audio).
- **RED off**: RED repeats the previous frame, which roughly doubles bitrate at 128k. Use Opus in-band FEC instead,
  if anything.
- Publish as `TrackSource::ScreenshareAudio`.

**Sync:**
- libwebrtc lip-syncs audio and video only when they share a MediaStream. That means the same `stream` in
  `TrackPublishOptions` on the publisher, `room.sync_streams: true` on the server, and **libwebrtc doing the
  playout**.
- **Pulse does its own playout.** It pulls PCM via `NativeAudioStream` (`voice/mod.rs:583`, currently mono,
  `num_channels = 1`) and renders video frames from `NativeVideoStream` itself. So **sync is the app's job**: align
  using frame timestamps and the known audio pipeline latency, or keep both paths low-latency and accept ≤ ±60 ms
  skew.
- Discord explicitly does not sync Go Live with voice.
- **Client spec items:** a stereo `NativeAudioStream` for stream audio, a separate mix bus and volume, and excluding
  stream audio from noise suppression.

---

## 5. How others do it

### Discord Go Live

Sources: Discord blog 2024-03-05 and 2018-09-10, Discord support articles updated 2026-10-13.

**Tiers:**

| Tier | Max quality |
|---|---|
| Free | **720p / 30 fps** |
| Nitro Basic | 1080p / 60 fps |
| Nitro | up to **4K / 60 fps** (iOS 1080p60) |

**Limits:** max **50 concurrent viewers** per stream. Up to 50 video/screen shares in a voice chat.

**Codecs:**
- VP8 and H.264, plus HEVC and AV1 encoders on specific platforms. AV1 needs an RTX 40 sharer.
- The codec is negotiated so that the streamer can encode it and **all** viewers can decode it, and is renegotiated
  when someone joins or leaves.
- Hardware encode and decode are preferred.

**Architecture:**
- An in-house C++ SFU ("RTC worker"); signalling in Elixir; Salsa20 transport encryption in native apps, not
  DTLS/SRTP.
- Already 850+ voice servers in 13 regions, including South Africa, in 2018.
- Streams always go through Discord's servers, which hides IPs.

**Model and adaptation:**
- Video is relayed **only to participants who are watching**, and the **streamer transmits only when ≥1 viewer
  watches**: the opt-in model.
- Quality follows bandwidth estimation, and the streamer "does not send more data than the slowest viewer's
  connection can support". That implies a **single encoding, not simulcast**: one slow viewer degrades everyone.
- With 6 friends and simulcast, Pulse can do better than that.
- Discord uses different BWE algorithms per use case, measures quality with VMAF, and runs post-stream surveys.

### Others (knobs, not engineering detail)

| Product | Model | Quality / latency knobs |
|---|---|---|
| **Slack huddles** | Amazon Chime SDK (per an AWS customer story) | up to **2 sharers at once**, 50 participants / 25 video. No published resolution or fps |
| **Zoom** | own SFU/MMR | screen share is tiny by default (~50–150 kbps for static content, low fps); "Optimize for video clip" mode for motion (~1.5 Mbps+). Detail over motion by default |
| **Parsec** (1:1 game streaming) | P2P / relay | bitrate cap default **10 Mbps**, ">50 Mbps has material latency impact"; H.265, 4:4:4 colour; encode latency ~5–8 ms; the target is ~LAN-grade latency, not rooms |
| **Moonlight / Sunshine** | P2P, one viewer | up to ~150 Mbps on LAN, H.264/HEVC/AV1; single-digit-ms encode latency; no multiparty |

**Takeaways:**
- Parsec and Moonlight show the ceiling for 1:1 (HEVC/AV1 at 20–50 Mbps, <30 ms).
- Zoom shows the floor for text (very low bitrate, low fps).
- Discord sits in between with opt-in viewing and per-tier caps.
- Pulse's 6-friend SFU can beat Discord's free tier easily. Matching Nitro 1080p60 is realistic; 4K isn't worth it.

---

## 6. Costs and limits summary for Pulse

| Item | Number |
|---|---|
| LiveKit CPU, 1 stream × 6 viewers @1080p60 + voice | ~15–25% of 1 A1 OCPU [est. from measured x86 ×2.5] |
| LiveKit CPU, 2 streams × 6 viewers @1080p60 + voice | **~30–45% of 1 OCPU** (15–25% of 2 OCPU) |
| LiveKit CPU, 2 × 6 @1440p60 | ~55–75% of 1 OCPU, not recommended on 1 OCPU |
| LiveKit memory | ~125 MB per Pulse-sized room, +3.5 MB per participant [measured x86] |
| Egress per stream-hour (1080p60 game, 6 viewers) | **~16 GB/h**; text stream ~3–7 GB/h |
| Monthly egress, our 6–10 friends | 0.1–1.5 TB typical, ≤8 TB heavy → **$0** |
| Monthly egress, a few dozen users, heavy | possibly >10 TB → $0.05/GB (≈ $51 per extra TB) |
| Compute | $0 at 1 or 2 OCPU (Always Free 1,500 OCPU-h / 9,000 GB-h) |
| Sharer uplink | 8 Mbps (720p60/1080p30) · 12 Mbps (1080p60) · 20 Mbps (1440p60) |
| Viewer downlink | = top layer, 3–11 Mbps; LTE data saver at 720p ≈ 1–1.5 GB/h |

### Failure modes to plan for

1. **One-core contention.** Argon2 logins after a deploy, `VACUUM INTO` backups, Caddy renewals and GC pauses delay
   forwarding → BWE drops → everyone's stream gets blurry. *Plan: 2 OCPU; or `cpu_shares`; schedule backups
   outside evening hours.*
2. **UDP buffer overflow on keyframe bursts** → loss → PLI storm → seconds of freeze. *Plan: sysctl 16 MiB before
   launch.*
3. **Rust SDK defaults:**
   - adaptive_stream starts viewers at LOW;
   - the 3 fps fallback layer;
   - the 1080p30/3 Mbps encoding cap;
   - auto-subscribe makes everyone pay for every stream.

   *Plan: explicit options (see §7).*
4. **A sharer on weak uplink** (LTE, 20/10 fibre, Wi-Fi). All viewers get a degraded top layer. *Plan: pre-flight
   uplink check (BWE after 5 s), auto-downgrade preset, show "your upload limits quality".*
5. **TCP/relay viewers** get hitches. *Plan: log candidate type, cap those viewers at 720p.*
6. **An AV1 viewer that can't decode.** *Plan: Discord-style. Fall back to H.264 or use a backup codec when a
   non-AV1 viewer subscribes.*
7. **Egress cliff at scale** ($0.05/GB in Africa). *Plan: OCI budget alert, `limit.bytes_per_sec`, a cap on
   concurrent streams, tier caps.*
8. **Hardware encoder session limits.** Consumer NVIDIA cards cap concurrent NVENC sessions. Simulcast with 2 layers
   = 2 sessions, plus OBS etc. *Plan: fall back to software for the low layer, or drop simulcast.*
9. **A/V drift of stream audio** with Pulse's own playout. *Plan: timestamp alignment, or accept small skew.*
10. **Oracle "out of capacity"** when resizing to 2 OCPU. *Plan: try off-peak. The xneelo fallback has unlimited
    traffic but 1 vCPU / 2 GB, which is worse for video.*

---

## 7. Recommended defaults (to carry into the spec)

**Server (`deploy/livekit.yaml`):**
```yaml
rtc:
  # existing keys unchanged …
  packet_buffer_size_video: 500      # 1000 if Source 1440p60 ships
  pli_throttle: { low_quality: 500ms, mid_quality: 1s, high_quality: 1s }   # try high 500ms in spike
  congestion_control: { enabled: true, allow_pause: false }                 # A/B use_send_side_bwe
room:
  max_participants: 25
  # keep default enabled_codecs (need video/h264, video/vp8, video/av1)
limit:
  subscription_limit_video: 4
  bytes_per_sec: 50000000            # ~400 Mbps guard
prometheus:
  port: 6789                          # loopback-only via firewall
```

**Host:**
- `net.core.rmem_max=16777216`, `net.core.wmem_max=16777216`.
- **Resize to 2 OCPU / 12 GB.**
- OCI budget alert at $1.

**Client, publisher:**
- `video_codec: H264`, `video_encoder: Auto` (prefer HW), explicit `video_encoding` per preset (§4.1).
- `simulcast: true` with explicit `simulcast_layers` (1 lower layer, ≥15–30 fps).
- `degradation_preference` per preset.
- `source: Screenshare`.
- Stream audio: `ScreenshareAudio`, Opus stereo 128 kbps, `dtx: false`, `red: false`, same `stream` id.

**Client, room:** `dynacast: true`, `adaptive_stream: false`, opt-in subscription for screen-share tracks.

**Client, viewer:**
- `set_subscribed(true)` on Watch;
- `set_enabled(false)` when the stream view is hidden;
- `update_video_dimensions` on resize;
- quality menu → `set_video_quality`.

**Product limits:**
- 2 concurrent streams per voice channel.
- Default preset Sharp 1080p30. Gaming 1080p60 is available to everyone. Source 1440p60 is HW and ≥20 Mbps only.

---

## 8. Measure in the spike

1. **A1 per-packet cost.** Rerun my `lk load-test` matrix against a scratch LiveKit on an A1 (or prod during a quiet
   window, with Greg's OK):
   - runs A/D/E: 2 video × {6, 50, 150} subs;
   - record `process_cpu_seconds_total`, `livekit_packet_total`, `mpstat`, `sar -n DEV 1`;
   - this replaces the 2–2.5× x86 → A1 factor with a real number.
2. **Real screen-share packets.** For a 1080p60 and a 1440p60 H.264 screen share (text vs game):
   - pps, bytes/packet, keyframe size, average vs cap bitrate (LiveKit Prometheus plus client `get_stats()`).
3. **UDP drops before and after the sysctl:**
   - `nstat -az | grep -i UdpRcvbufErrors`, `ss -uampn` (skmem), during joins (keyframe bursts) and a forced
     `VACUUM INTO`.
4. **1 vs 2 OCPU.** Same load. Compare p95 jitter, viewer layer downgrades (`video_quality_changed` events) and the
   frame-freeze count.
5. **Time-to-first-frame:** Watch click → first decoded frame, with `pli_throttle.high_quality` at 1 s vs 500 ms,
   and 2 viewers joining 300 ms apart.
6. **Glass-to-glass latency.** Film a millisecond clock on the sharer and the viewer screens, or embed a frame
   counter. Do Gauteng↔JNB and Cape Town↔JNB. Target p50 ≤150 ms.
7. **Sharer-side encoder:**
   - CPU/GPU at 1080p60 and 1440p60 for NVENC H.264, NVENC AV1, x264/OpenH264 software;
   - check whether the Rust SDK really selects NVENC/VAAPI with `VideoEncoderBackend::Auto`;
   - whether simulcast opens 2 NVENC sessions.
8. **Congestion.** Use `tc netem` on a viewer: 2% loss, 80 ms jitter, and a 5 Mbps cap. Check the layer fallback
   works and the gaming fallback keeps ≥30 fps. A/B `use_send_side_bwe`.
9. **Rust SDK traps:**
   - confirm `adaptive_stream: true` pins viewers to LOW;
   - confirm `auto_subscribe: false` with manual mic subscription works with the existing voice code (`rx.rs` stays
     attached).
10. **TCP/relay path.** Block UDP on one viewer, then measure hitch frequency and the layer chosen.
11. **Stream audio A/V skew** with Pulse's own playout. Use a clap test and measure the offset.
12. **Egress accounting.** Compare one evening's `sar` bytes against OCI's network metrics, then sanity-check the
    16 GB/h figure.

---

## Sources (accessed 2026-10-13 unless noted)

- LiveKit benchmark (undated page): https://docs.livekit.io/transport/self-hosting/benchmark/
- LiveKit deployment guide (host networking, UDP preferred, TURN/TLS): https://docs.livekit.io/transport/self-hosting/deployment/
- LiveKit codecs/SVC/dynacast: https://docs.livekit.io/transport/media/advanced/
- LiveKit KB, "Configuring the client SDK for optimal video quality" (≈2025): https://kb.livekit.io/articles/3859313029-configuring-the-client-sdk-for-optimal-video-quality
- LiveKit server v1.13.7 source, `pkg/config/config.go` (defaults: packet buffers, congestion control, dynacast 5 s, codecs), `pkg/sfu/receiver.go` (PLI throttle 500 ms/1 s/1 s), `pkg/rtc/subscribedtrack.go` (adaptive stream → LOW), `pkg/sfu/streamallocator/track.go` (screenshare priority): https://github.com/livekit/livekit/tree/v1.13.7
- LiveKit `config-sample.yaml` (num_tracks / bytes_per_sec / subscription_limit_video docs): https://github.com/livekit/livekit/blob/master/config-sample.yaml
- `livekit/mediatransportutil` `pkg/rtcconfig` (16 MiB UDP buffers requested, warn < 5 MB): https://github.com/livekit/mediatransportutil
- `livekit-cli` README (load-test sysctls rmem/wmem 25 MB): https://github.com/livekit/livekit-cli
- LiveKit JS `ScreenSharePresets`: https://github.com/livekit/client-sdk-js/blob/main/src/room/track/options.ts
- LiveKit Rust SDK 0.9.3 source (local cargo registry): `livekit/src/room/options.rs`, `room/mod.rs`, `publication/remote.rs`; `libwebrtc-0.3.50/src/rtp_sender.rs`; `webrtc-sys-0.3.47/src/nvidia`, `/vaapi`
- OCI Compute shapes (A1: 1 Gbps/OCPU, OCPU = 1 Altra core): https://docs.oracle.com/en-us/iaas/Content/Compute/References/computeshapes.htm
- Oracle price list API (B93456 Outbound — Middle East & Africa: 0–10 TB free, then $0.05/GB; B88327 NA/EU $0.0085; B93455 APAC/SA $0.025): https://apexapps.oracle.com/pls/apex/cetools/api/v1/products/?currencyCode=USD and https://www.oracle.com/cloud/networking/pricing/
- PeeringDB, Oracle AS31898 at NAPAfrica JHB and JINX (100G): https://www.peeringdb.com/api/netixlan?asn=31898
- Discord, "How It All Goes Live" (2024-03-05): https://discord.com/blog/how-it-all-goes-live-an-overview-of-discords-streaming-technology
- Discord, "How Discord handles 2.5M concurrent voice users using WebRTC" (2018-09-10): https://discord.com/blog/how-discord-handles-two-and-half-million-concurrent-voice-users-using-webrtc
- Discord support, Go Live and Screen Share (updated 2026-10-13; 720p30 free, 4K60 Nitro, 50 viewers): https://support.discord.com/hc/en-us/articles/360040816151
- Discord support, Nitro & Nitro Basic (1080p60): https://support.discord.com/hc/en-us/articles/115000435108
- YouTube live encoder settings (bitrates, 2 s keyframes): https://support.google.com/youtube/answer/2853702
- webrtcHacks, "The hidden AV1 gift in Google Meet" (2023-12-19): https://webrtchacks.com/the-hidden-av1-gift-in-google-meet/
- Parsec, max bitrate (default 10 Mbps, >50 Mbps latency impact): https://support.parsec.app/hc/en-us/articles/32381776828820
- Slack huddles help (2 sharers, 50/25 limits): https://slack.com/help/articles/4402059023635
- Zoom bandwidth (university mirrors of Zoom's table): https://alaska.edu/oit/services/video-conferencing/zoom/optimize.php
- SA fibre symmetric plans (indicative, affiliate sites): https://fastestfibre.co.za/insights/openserve-vs-vumatel-2026/
- Prior Pulse research: `docs/superpowers/research/opt-server.md` (H3 2-OCPU / reclamation, M5 livekit.yaml, M6 capacity + UDP buffers)
