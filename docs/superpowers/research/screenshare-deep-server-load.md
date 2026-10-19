# Screen share deep dive: cutting server load and removing the server as a single point of failure

**Date:** 2026-10-14. **Scope:** every idea I could find, from sensible to strange, for making screen share cheaper on
the Oracle A1 box, or not dependent on it at all. Each idea is scored for a group of about 6 friends.
**Builds on:** `screenshare-server-quality.md` (its load numbers and config are assumed here and not repeated) and
`screenshare-summary.md` (video moves to `livekit-client` in WebView2; voice stays in Rust).

**Labels:**
- **[measured]**: I ran it on 2026-10-14, or read it directly from source code at a named tag.
- **[cited]**: from a primary source in the list at the end.
- **[est.]**: my own engineering estimate. Check it in the spike.
- **[speculation]**: plausible but unverified.

**A note on the measurements.** I ran them on this dev laptop (i9-13900HX) while other jobs were hammering the
machine (load average 3 to 100). LiveKit was pinned to one E-core, whose speed is closer to a Neoverse-N1 than a
P-core. I interleaved the runs A/B/A/B and give medians and ranges. Treat differences under about 10% as noise.
I did not touch production. The web-search budget for this session ran out partway through, so SA-specific NAT/CGNAT
facts are marked [speculation] and listed as things to measure.

---

## 0. TL;DR

1. **Server load is not the problem for 6 people. The decisions that matter are product decisions.** A whole
   Pulse-sized room (2 video, 2 audio, 6 viewers, about 2k packets/s out) costs about **4-5% of one E-core**
   [measured], which is about 10-12% of one A1 OCPU [est.]. Once the box is 2 OCPU, one person watching a 1080p60
   stream costs about **1-1.5% of the box** [est.]. Every "remove the server" idea saves only that much CPU.
2. **The big levers (ranked 1-3) are almost free and mostly on the client:**
   - opt-in "Watch" plus dynacast, so an unwatched stream costs zero (the sharer even stops encoding);
   - `adaptiveStream` in the **JS** SDK, which now lives in WebView2. It picks the layer from the tile size and pauses
     video when Pulse is minimised. The earlier advice to leave `adaptive_stream` off was about the Rust SDK only.
   - preview thumbnails instead of live video for people who aren't watching.

   Together these cut the expected packets a stream sends by roughly **50-85%** in a typical evening [est.].
3. **Server tuning results [measured]:**
   - **`rtc.batch_io` (sendmmsg)** saves **~5-30% CPU per packet**: about 9% on a quiet machine, and more when the CPU
     is contended. But it costs **~1.5% of a core even with zero users**, because a flusher goroutine ticks every
     millisecond. For Pulse's mostly-idle server that is roughly break-even. Leave it off for now.
   - **Binding the UDP mux to one IP** (`rtc.ips.includes`) is free hygiene. On a docker host LiveKit opened
     **11 sockets**, one per local IP including docker bridges.
   - **GOGC tuning does nothing.** GC is about 1% of the profile.
   - **A second UDP mux port** made no difference at this scale.
   - **Where the CPU goes:** about 55-60% is the `sendmsg` syscall path and about 6-8% is SRTP AES-GCM.
   - **Kernel GSO does not exist** anywhere in LiveKit's pion stack, and suits SFU fan-out poorly anyway.
4. **Single point of failure.** If the server dies, chat, voice and screen share all die together. Peer-to-peer
   screen share alone does not fix that, because signalling still needs the server. The biggest real availability
   wins are graceful deploys (already in `opt-server.md` H1/H2) and an **optional failover SFU on Cloudflare
   Realtime** (1 TB/month free, then $0.05/GB, anycast). A P2P "direct mode" for one viewer is possible in WebView2,
   but it adds an encoder per viewer on the sharer's GPU while they are gaming, so it isn't worth it for load.
5. **Upgrade LiveKit to v1.13.10 (released today).**
   - It hardens embedded TURN: before #4992, a participant could relay through TURN to the host's own ports.
   - It removes per-packet allocations (#4951).
   - It also changes how the Prometheus port is configured (#4928).
6. **Wild ideas, verdict:**
   - Viewer relaying / cascading: no.
   - Swapping the SFU (mediasoup, Galene, str0m): no.
   - Media over QUIC via Cloudflare's free preview relay: a fun spike later, not for the product.
   - FEC instead of NACK: no.
   - AV1 SVC instead of simulcast: helps the sharer's uplink, not the server.

---

## 1. Ground truth: what a stream costs, and where the CPU goes

### 1.1 Absolute cost at Pulse scale [measured]

LiveKit v1.13.7 (official image) was pinned to one E-core, with `lk load-test` v2.18 on other cores, over loopback.
The "high" video preset is about 217 packets/s per track. Load average was about 4 (quiet).

| Run | Load | Packets/s out | LiveKit CPU (1 E-core) | Per 1k pps |
|---|---|---|---|---|
| Pulse-sized | 2 video + 2 audio publishers, 6 viewers | ~1.9-2.0k | **4.6-4.9%** | 2.3-2.5% |
| Pulse-sized, `batch_io` | same | ~1.9-2.0k | 3.8-4.3% | 2.0-2.1% |
| Heavy | 2 video + 2 audio publishers, 40 viewers | ~13k | 14.5-19.3% (median 15.9) | 1.08-1.47 (median 1.23) |
| Heavy, `batch_io` | same | ~13k | 14.1-15.4% (median 14.7) | 1.08-1.17 (median 1.12) |
| Idle server, 1 socket | nobody connected | 0 | **0.15%** | — |
| Idle server, 1 socket, `batch_io` | nobody connected | 0 | **1.5-1.7%** | — |
| Idle server, 11 sockets, `batch_io` | nobody connected | 0 | **3.0-3.1%** | — |

**What this means for the real target [est.]:**
- A 1080p60 stream at about 7 Mbps is about 700 packets/s per viewer.
- 2 streams × 5 viewers is about 7k packets/s of video, plus voice.
- Using the earlier working figure of 3-4% of an A1 OCPU per 1k packets/s, that is **~25-30% of one A1 OCPU**, or
  about 13-15% of the 2-OCPU box.
- Small loads cost more per packet (fixed per-room overheads).

### 1.2 CPU profile under load [measured]

A 30 s `pprof` CPU profile of the heavy run, taken from the built-in `debug_handler` port. That port is already in
v1.13.7 config, so it can be enabled on loopback in production for the spike.

| Where | Share of LiveKit CPU |
|---|---|
| `sendmsg` syscall path (`ice.(*udpMuxedConn).WriteToAddrPort` → `Syscall6`) | **~57-59%** |
| SRTP encrypt (`srtp.encryptRTP` → AES-GCM `Seal`) | ~6-8% |
| Forwarder logic (`GetTranslationParams`, RTP stats, pacer) | ~10% |
| UDP read path (`UDPMuxDefault.connWorker`, `ReadFromInet4`) | ~2% |
| GC (`gcBgMarkWorker`) and `mallocgc` | ~1-2% |

**Caveats:**
- On loopback, the receiving side's kernel work runs inside the sender's syscall, which inflates the syscall share.
  On the A1 the syscall path goes through virtio-net instead. That is probably cheaper per packet but still the
  biggest item [est.].
- **Conclusions:**
  - Fewer packets, or fewer syscalls per packet, is the only server-side lever that matters.
  - GC and allocation tuning is noise at this scale.
  - Crypto is small. Neoverse-N1 has AES instructions, and Go uses them on arm64 [est.].

---

## 2. Zero viewers = zero cost (rank 1)

### 2.1 What already happens with opt-in plus dynacast

- **Server side:**
  - With dynacast, once a layer has no subscribers for `video.dynacast_pause_delay` (default 5 s), the server tells
    the publisher which qualities are wanted [cited: v1.13.7 config defaults, from the earlier report].
  - With nobody watching, all qualities are off.
- **JS publisher side** [measured, from `client-sdk-js` main @ 6c20018]:
  - `setPublishingLayersForSender` sets `encoding.active = false` on every simulcast encoding that has no subscriber,
    then calls `sender.setParameters`. Chromium stops encoding inactive encodings.
  - So with zero watchers, **the sharer's GPU encoder, the sharer's uplink and the server all go to ~0.**
  - Capture (WGC) keeps running, which is cheap.
- **Defaults to override in JS:** `dynacast: false` and `adaptiveStream: false` [measured: `src/room/defaults.ts`].
  Set both true for the WebView2 video participant.

**What Pulse must do:**
- **Do not auto-subscribe screen-share video.** Viewers click Watch (already the plan).
- **Publish with dynacast on.**
- **Decide the zero-viewer UX.** The stream is "live", but nothing flows until someone clicks Watch. The first
  watcher pays one extra keyframe round trip plus about 5 s of nothing if they leave and come back within the pause
  delay. Simply accept the ~250-600 ms first-frame time from the earlier report.

**Payoff:** for 6 friends, a big share of "live" streams have 0-2 watchers. Discord's own design is "the streamer
transmits only when ≥1 viewer watches" [cited]. **Effort:** config only. **Risk:** none.

### 2.2 Thumbnail previews instead of live video (rank 3)

The idea is to show non-watchers a still image that refreshes every 10-30 s, Discord-style, so they can decide to
click Watch.

- **How:**
  - The sharer's WebView2 draws the capture `MediaStreamTrack` into a 320×180 canvas every 15 s and encodes a JPEG
    (~10-20 KB).
  - It POSTs the JPEG to axum, which keeps the latest one per stream in memory.
  - Viewers fetch it, or get it pushed over the existing WebSocket.
- **Cost [est.]:** 20 KB / 15 s ≈ 1.3 KB/s upload per stream. A 6-person fan-out is ~8 KB/s. Effectively zero.
- **Alternatives:**
  - A LiveKit data packet or participant attribute. Attributes have size limits and it's the wrong tool.
  - Subscribing to the **LOW** layer at very low fps for a hover peek. That costs ~50-100 packets/s per peeker, still
    cheap, but it wakes the low encoder.
- **Effort:** about half a day. **Risk:** privacy. The thumbnail exists on the server, so treat it like a chat image
  (permission-checked, memory only, cleared when the stream ends).

### 2.3 Subscriber-count events

The server's room webhook already reaches axum. For UI ("3 watching") use the client-side
`RemoteTrackPublication` subscription state, or `participant.attributes`. There's no need for a server poll.

---

## 3. Per-viewer smartness (rank 2)

Viewers are now `livekit-client` (JS) in WebView2, so the JS SDK's **adaptive stream** comes for free
[measured: `RemoteVideoTrack.ts`]:
- It watches each attached `<video>` element with `ResizeObserver` and `IntersectionObserver`, debounced 100 ms. It
  sends the element's pixel size (× device pixel ratio) to the SFU, which then picks the smallest simulcast layer
  that covers it.
- `pauseVideoInBackground` defaults to **true**. When `document.visibilityState` goes hidden (WebView2 minimised),
  the track is set `disabled` and the SFU stops forwarding.

**Payoff [est.]:**

| Viewer state | Layer | Packets/s (1080p60 stream, simulcast 1080p60 + 540p30) |
|---|---|---|
| Full screen / large window | HIGH | ~700 |
| Small tile (≤ 960×540 on screen) | LOW | ~100-140 |
| Pulse minimised | none | 0 |

A typical "two watching full screen, two in a tile, one minimised" evening sends about **1.7k instead of 3.5k
packets/s** per stream.

**Traps:**
- **Starts on LOW.** v1.13.7 OSS starts adaptive-stream subscribers on **LOW**. The PR #4595 "start at desired
  quality" switch exists in code, but `EnableStartAtDesiredQuality` is never set in OSS participant params
  [measured: grep at v1.13.7]. The JS client sends dimensions about 100 ms after attach, so the cost is one extra
  layer switch (a keyframe). It is barely visible, but measure it.
- **Focus is not visibility.** A friend playing the same game with Pulse on a second monitor is unfocused but
  visible. Never pause on `blur`. Only "hidden" (minimised, or covered by another window, depending on how WebView2
  reports occlusion [speculation]) should pause.
- **Pixel density.** `pixelDensity: 'screen'` asks for full-DPR pixels. On a 150%-scaled laptop that requests a
  bigger layer than needed. The default `1`-ish behaviour is fine for game streams.

**Effort:** a few lines of `RoomOptions`. **Risk:** low.

**Data-saver toggle:** for LTE viewers, call `setVideoQuality(LOW)` or cap dimensions. Again only a few lines.

---

## 4. Server/kernel tuning on the A1

### 4.1 What the LiveKit/pion stack actually does with UDP [measured from source]

| Mechanism | Present? | Where |
|---|---|---|
| `sendmmsg` (batch send) | **Yes, opt-in** via `rtc.batch_io` | `mediatransportutil/pkg/transport/createmux.go` wraps each mux socket in `pion/transport/v4/udp.NewBatchConn`, using ping-pong buffers and a flusher goroutine |
| `recvmmsg` (batch read) | No: one `ReadFrom` per packet in `ice.UDPMuxDefault` | pion/ice v4 (LiveKit fork `v4.4.0-warp.2`) |
| UDP GSO (`UDP_SEGMENT`) / GRO | **No.** Nothing in any pion or LiveKit dependency references `UDP_SEGMENT`/`UDP_GRO` | grep of the module cache for v1.13.7's `go.mod` |
| `SO_REUSEPORT` multi-socket | Not used. LiveKit instead opens one mux per port in `rtc.udp_port` (up to NumCPU) | `rtcconfig/webrtc_config.go` |
| Socket buffers | Requests 16 MiB read/write per mux socket; warns below 5 MB | `rtcconfig/config.go` |

### 4.2 Each kernel/runtime lever

| Lever | Verdict for Pulse | Evidence |
|---|---|---|
| `net.core.rmem_max`/`wmem_max` 16 MiB | **Do it** (already in the earlier report). Cheap insurance against keyframe-burst drops | [cited], earlier report |
| `rtc.batch_io` | **Off for now. Revisit** if streams ever run most of the evening or 20+ viewers appear | Saves ~9% per packet when quiet, ~10-30% when the CPU is contended. But the idle flusher costs ~1.5% of a core per socket group, all day. Break-even is roughly 5-7k packets/s sustained [measured] |
| `rtc.ips.includes` (one socket) | **Do it.** Fewer sockets, fewer pointless host ICE candidates (docker bridges), and makes `batch_io` cheaper if it's ever enabled | 11 sockets on a docker host vs 1 [measured]. Use the VNIC's private CIDR, which is site-specific and not written here |
| `rtc.udp_port` range ≥ vCPUs (config-sample advice) | **Skip.** Ingress for Pulse is 1-2k packets/s, one reader is plenty | 2 cores, 1 port vs 2 ports: no consistent difference (17.5/29.6% vs 19.8/27.3%, noisy) [measured]. Also needs firewall changes |
| GOMAXPROCS | **Leave default.** Go 1.25+ honours cgroup CPU limits and updates on change; LiveKit is built with Go 1.26 | [cited: Go 1.25 notes], `go.mod` [measured] |
| GOGC / GOMEMLIMIT | **Skip.** GC is ~1% of CPU; GOGC=400 changed nothing beyond noise | [measured] |
| Kernel UDP GSO | **Not available without patching pion.** It also batches only packets to the **same destination**. An SFU's fan-out goes to many destinations, so GSO helps only per-viewer bursts (a keyframe's 100-400 packets). `sendmmsg` (batch_io) is the right tool for fan-out | [measured: absent], [est.] on fit |
| `busy_poll`/`busy_read` | **No.** It trades idle CPU for latency, and Oracle's idle-reclaim rule looks at CPU but we're not short of latency | [est.] |
| IRQ affinity / NIC queues | **No.** 2 vCPUs, ~10-20k packets/s. Check `ethtool -i`/`-l` in the spike to learn whether the A1 VNIC is virtio-net and how many queues it has; Oracle's shape page doesn't say | [cited: OCI shapes page is silent] |
| MTU | **No change.** WebRTC keeps RTP at ~1.2 kB for internet paths; VCN jumbo frames don't apply to internet clients | [est.] |
| CPU governor/frequency | Not applicable on A1 (fixed 3.0 GHz) | [cited, earlier report] |

### 4.3 LiveKit config levers (beyond the earlier report's §7)

- **`debug_handler: { port: <loopback port> }`** exposes `/debug/pprof` on its own port without dev mode
  [measured: `pkg/config/config.go`, `config-sample.yaml`].
  - Use it in the spike to get a real **A1 CPU profile** of a 1080p60 stream.
  - The `--cpuprofile` flag wrote an empty file on `docker stop` in my test, so use the HTTP endpoint.
- **`enable_warp`** (experimental): saves the DTLS and SCTP round trips on connect, so joins get faster. It does
  nothing for load. Try it later for join latency.
- **`room.empty_timeout`, `departure_timeout`, `node_selector`:** no effect on media load (single node).
- **Unused features cost nothing.** Ingress, egress and SIP are not running unless configured. Embedded TURN costs
  nothing until someone relays.
- **`limit.bytes_per_sec` / `subscription_limit_video`:** guards, as in the earlier report.
- **Upgrade to v1.13.10** (2026-10-14) [measured: `git log v1.13.7..v1.13.10`]:
  - **#4992 (security):** embedded TURN in ≤1.13.9 let a participant relay to arbitrary peers, including (via RFC 6062
    TCP relays) the node's own ports. 1.13.10 refuses TCP relays and limits relays to the host to the media ports.
    Pulse runs embedded TURN/UDP, so this matters on its own.
  - **#4951, #4953:** stop allocating per ingested packet.
  - **#4928:** the deprecated top-level `prometheus_port` style is gone from the sample. Check the yaml uses the
    `prometheus: {port:}` form.
  - **Not yet in any tag** (master only): #4875 (no per-packet allocation in the RTP broadcast) and #4899 (a higher
    fan-out threshold when batch IO is on). Their benchmarks show 7-15% per-packet gains at large fan-out
    [cited: PR bodies], which is irrelevant at 5 viewers.

---

## 5. Taking the server out of the media path

### 5.1 P2P direct mode (1-2 viewers)

WebView2 is Chromium, so a plain `RTCPeerConnection` from the sharer to each viewer works. Our axum WebSocket
would carry the signalling.

**What it saves:**
- About 700 packets/s and ~3 GB/h of egress per 1080p60 viewer. That is **~2-3% of one A1 OCPU** [est.] and $0
  (egress is free under 10 TB). **Negligible.**
- Latency: a Gauteng↔Gauteng direct path skips the trip to the Johannesburg DC. The JNB RTT inside SA is already
  2-15 ms [earlier report], so it saves **~5-15 ms** [est.]. Users won't notice.

**What it costs:**
- **One encoder per viewer on the sharer.** Each `RTCPeerConnection` sender runs its own encoder [est., libwebrtc
  architecture]. Three P2P viewers means 3× NVENC sessions plus 3× the GPU video-engine load, **while the sharer is
  playing a game**. GeForce cards allow 12 concurrent NVENC sessions [cited], so the session cap isn't the issue;
  GPU contention and the sharer's uplink are.
- **The sharer's uplink is multiplied:** 3 × 7 Mbps = 21 Mbps at 1080p60.
- **NAT traversal:**
  - About **80%** of WebRTC connections work without TURN (bloggeek, 2025-11-10) [cited].
  - With the full set of tricks Tailscale reaches **>90%** direct. If **both** ends are behind endpoint-dependent
    ("hard") NAT, direct connection is practically impossible (0.01% after 20 s of port guessing) [cited].
  - SA mobile (LTE/5G) and some cheap fibre packages are widely believed to sit behind CGNAT [speculation, not
    verified: search budget exhausted].
  - IPv6 won't rescue it: **~4% of SA users are IPv6-capable** (APNIC, data dated 2026-10-11) [cited/measured].
- **TURN fallback needs a relay.**
  - LiveKit's embedded TURN credentials are an HMAC of the API secret, participant id and expiry
    [measured: `pkg/service/turn.go`], so axum *could* mint them.
  - But **v1.13.10 limits relays to the host's media ports**, so embedded TURN can't relay P2P any more. We'd need
    coturn, or Cloudflare TURN (which shares the 1 TB free tier).
  - A relayed P2P stream costs the server **as much as or more than** an SFU forward: TURN handles each packet in and
    out.
- **Complexity:** a second media path, renegotiation when a 3rd viewer joins (P2P → SFU switch), and a glitch at
  the switch.

**Verdict for 6 friends: no, not for load.** It's only worth reconsidering as part of the outage fallback (§6).

### 5.2 Mesh for ≤3 people

Same as P2P, but everyone sends to everyone. Screen share is one-to-many, so mesh is the same as §5.1.
**Not worth it.**

### 5.3 Viewer relaying / cascading (a viewer forwards to another)

- **In the browser:** a received track can be re-sent, but only by **decoding and re-encoding** it on the relaying
  viewer. That adds GPU load, a frame or two of latency and a generation of quality loss.
- **Forwarding without re-encoding** would need Encoded Transform hacks. The relaying sender still runs an encoder,
  and keyframe requests aren't wired across hops [speculation]. **Fragile.**
- **A Rust-side relay** (str0m inside a viewer's Tauri app forwarding raw RTP) is doable. It makes a friend's home
  upload part of everyone's stream, which is a reliability downgrade.
- **Wild variant, "sharer-hosted mini-SFU":** the sharer's own Pulse runs a str0m forwarder. WebView2 encodes once to
  it over loopback, and it fans out to viewers. This fixes §5.1's N-encoder problem (one encode), but the uplink is
  still multiplied. It's a lot of new Rust for a ~3% server saving.

**Verdict: no** for all three.

### 5.4 Managed SFU: Cloudflare Realtime (SFU + TURN) [cited]

**Facts (docs updated 2026-09-22 / 2026-10-06; blog 2024-04-04):**
- **Pricing:** egress only, **$0.05/GB after 1,000 GB/month free**. The SFU and TURN share the free tier. Ingress is
  free.
- **Network:** anycast WebRTC on Cloudflare's network ("all 310+ cities", 2024 blog), with per-track cascading trees
  between data centres. Johannesburg/Cape Town/Durban PoPs serving Realtime specifically: not confirmed [speculation].
- **Codecs:** H.264, H.265, VP8, VP9, AV1, Opus.
- **Simulcast:** the subscriber **chooses** the layer (`preferredRid`). Fallback (`priorityOrdering`,
  `ridNotAvailable`) defaults to `none`, so there is **no LiveKit-style automatic per-viewer bandwidth allocation**.
  We'd write it from `getStats`.
- **Limits:** no fixed track limits; 50 API requests/s per session; a track with no packets for 30 s is
  garbage-collected.
- **API:** HTTP calls with an app secret, so axum would broker sessions. It is **not** LiveKit-protocol compatible:
  the WebView2 video code would need a second, plain-WebRTC implementation (or Cloudflare's RealtimeKit SDK).

**Cost for Pulse [est.]:**

| Month | Oracle (now) | All screen share on Cloudflare |
|---|---|---|
| Light (~0.1 TB) | $0 | $0 |
| Typical (~1.2 TB) | $0 | ~$10 |
| Heavy (~7.8 TB) | $0 | **~$340** |

**Verdict:** as the *primary* video path it **costs more** than Oracle's 10 TB free and saves a few % CPU. As a
**failover SFU** (used only when our LiveKit is unreachable) it is the best "no single point of failure" option for
media. See §6.

### 5.5 Media over QUIC (MoQ) via Cloudflare's relay (wild)

- **Facts (2025-08-22) [cited]:**
  - A MoQ relay on every Cloudflare server.
  - **Free tech preview "at any scale"**, with breaking changes possible. At GA, $0.05/GB egress.
  - Browsers need **WebTransport**. WebView2 is Chromium, so WebTransport and WebCodecs should be there [est.].
- **What it would mean:** sharer encodes with WebCodecs (hardware H.264), publishes to the relay; viewers decode
  with WebCodecs and render to a canvas.
- **Downsides:** no WebRTC jitter buffer, BWE, NACK or simulcast logic, so we'd build all of it. Sub-second, but
  likely above WebRTC's 100-200 ms.
- **Verdict:** a fun weekend spike, **not a product path** for 2026.

### 5.6 Alternative self-hosted SFUs

| SFU | Notes | Worth switching? |
|---|---|---|
| mediasoup (C++ worker, Node control) | One worker = one core; "typically over ~500 consumers" per worker [cited] | No. Pulse has ≤10 consumers per stream. We'd lose LiveKit SDKs, dynacast and the stream allocator |
| Galene (Go, MIT) | "very moderate server resources" [cited], no numbers | No |
| str0m (Rust, sans-IO) | Used as an SFU by its maintainers; the chat example "not for production" [cited] | No as a replacement. Only interesting for §5.3's sharer-hosted relay |
| Janus, ion-sfu | ion-sfu is effectively unmaintained [speculation]; Janus is C and plugin-based | No |

LiveKit's own benchmark is ~38-60k packets/s per x86 vCPU (earlier report). Even a 2× more efficient SFU saves
~1-2% of the box at Pulse scale. **The switching cost is weeks; the payoff is nothing.**

---

## 6. Removing the server as a single point of failure

**What actually fails today when the box is down:** login, chat, voice and screen share, all together. Screen share
is the least of it. Ranked by value for effort:

1. **Graceful deploys and presence survival** (`opt-server.md` H1/H2): most "outages" are our own restarts. *Cheap,
   high value.*
2. **Upgrade LiveKit and keep it patched** (the TURN hardening above). *Cheap.*
3. **Idle-reclaim insurance:** Oracle reclaims Always Free VMs if, over 7 days, p95 CPU, network **and** memory
   (A1) are all under 20% [cited]. Zero-viewer streams reduce load further, so keep whatever keep-alive
   `opt-server.md` H3 decided. Note that the free allowance is **2 OCPU / 12 GB total** (1,500 OCPU-h, 9,000 GB-h)
   [cited, re-checked 2026-10-14]. A second free VM for redundancy means splitting into 2 × 1 OCPU, which undoes the
   resize. **Not recommended.**
4. **Optional failover for media (larger project):**
   - **Signalling fallback:** a tiny Cloudflare Worker + Durable Object that knows room membership and brokers
     Cloudflare Realtime sessions [speculation: free-tier limits not checked here].
   - **Media fallback:** Cloudflare Realtime for both voice and video during an outage. That needs voice in WebView2
     too, which conflicts with "voice stays in Rust", so only video could fail over cheaply.
   - **Auth while the main server is down:** a short-lived signed token cached on the client.

   *Effort:* about 1-2 weeks. *Payoff:* survives Oracle outages. *Verdict:* park it until outages actually happen.
5. **P2P as outage fallback:** needs the signalling fallback above anyway. Once that exists, P2P video for 1-2
   viewers is a cheaper fallback than Cloudflare egress, but it carries all of §5.1's NAT and GPU problems.

---

## 7. Other creative ideas, briefly

- **Static content costs nothing.** Chromium's WGC capture only delivers frames on change. A static desktop or
  paused game sends almost nothing [est.; the earlier report saw the same]. With `contentHint = "detail"`, text
  streams also drop frame rate under pressure instead of resolution.
- **AV1 SVC instead of simulcast:**
  - One encode, roughly +10-30% uplink instead of +25-40%.
  - For the server, SVC means fewer **ingress** packets, but ingress is ~5% of Pulse's packet budget.
  - Dynacast can only pause the whole SVC stream.
  - Hardware AV1 SVC is limited.
  - **Sharer-side benefit only; not a server lever.**
- **FEC instead of NACK:**
  - ULPFEC/FlexFEC adds 10-50% bitrate **all the time** to save retransmits that are ~1-5% of packets on Wi-Fi.
  - Worse for both server load and egress.
  - **No.** Keep NACK/RTX (and v1.13.10's #5000, which sends NACKs when processing RTX).
- **Playout-delay extension:** no load effect. Don't use it for screen share (earlier report).
- **Per-viewer bitrate caps:** already implied by layer choice plus `setVideoQuality`. A server-wide guard is
  `limit.bytes_per_sec`.
- **Same-house viewers (two friends on one LAN):** a P2P path would find the LAN route and save their ISP twice.
  Rare for this group. **No.**
- **Keyframe discipline:** `pli_throttle` already stops a keyframe storm when several viewers join. That is the
  cheapest burst control. Keep it, maybe tune `high_quality` (earlier report).
- **Stream cap per channel:** "max 2 concurrent streams per voice channel" (earlier report). This is the hard upper
  bound on server load, and it's free.

---

## 8. Ranking (for a 6-person group)

| # | Idea | Server load saved | Other payoff | Effort | Risk | Verdict |
|---|---|---|---|---|---|---|
| 1 | Opt-in Watch + dynacast (zero viewers = zero cost) | **Large** (unwatched streams cost 0) | Sharer GPU and uplink idle | Config | None | **Do** |
| 2 | JS `adaptiveStream` (tile-size layers, pause when hidden) + data-saver | **Large** (~50% per stream [est.]) | Viewer data and CPU | Tiny | Low (pause on hidden only, not blur) | **Do** |
| 3 | Preview thumbnails for non-watchers | Medium (lets people *not* watch) | Discord-like UX | ½ day | Privacy, handled like images | **Do** |
| 4 | Upgrade LiveKit to v1.13.10 | Small | **TURN security fix** | Small | Low | **Do** |
| 5 | `rtc.ips.includes` (one mux socket) | Tiny | Cleaner ICE candidates | Config | Low (get the CIDR right) | **Do** |
| 6 | 2 OCPU + 16 MiB UDP buffers (earlier report) | Headroom, isolation | — | Small | Capacity availability | **Do** (already planned) |
| 7 | `debug_handler` pprof + A1 profile in spike | — | Real numbers | Small | Keep it on loopback | **Do in spike** |
| 8 | Stream cap per channel + `limit.*` guards | Bounds the worst case | Egress safety | Config | None | **Do** |
| 9 | `rtc.batch_io` | 5-30% per packet under load | — | Config | **+1.5% idle CPU, 0-2 ms added latency** | **Not now**; revisit if load grows |
| 10 | Cloudflare Realtime as failover SFU | — | Removes a single point of failure for video | 1-2 weeks | New code path, $ if heavy | **Park** |
| 11 | P2P direct mode | ~2-3% of an OCPU per viewer | 5-15 ms latency | 1-2 weeks | NAT, sharer GPU, uplink | **No** (maybe as outage fallback later) |
| 12 | MoQ over the Cloudflare relay | All | Free preview | Large | Preview, rebuild BWE | **Spike for fun only** |
| 13 | Alternative SFU | ~1-2% | — | Weeks | High | **No** |
| 14 | Kernel GSO / busy-poll / IRQ / GOGC / multi-port | ~0 | — | Patching pion (GSO) | — | **No** |
| 15 | Viewer relaying / cascading / mesh | ~2-3% | — | Large | Reliability | **No** |
| 16 | FEC instead of NACK | Negative | — | Small | More egress | **No** |

---

## 9. Add to the spike

1. **A1 profile:** enable `debug_handler` on loopback on a scratch A1 (not prod), run `lk load-test` 2 video ×
   {6, 40} viewers, and take a 30 s profile. Compare the syscall share with this report's loopback profile.
2. **Real per-packet cost on the A1:** `process_cpu_seconds_total` with and without `batch_io`, plus **idle** CPU
   with `batch_io`. This decides item 9 with real numbers.
3. **Zero-viewer check:** publish from WebView2 with dynacast, nobody watching, then confirm:
   - `getStats().outbound-rtp` bytes stop;
   - `encoderImplementation` goes quiet;
   - LiveKit ingress for that track goes to 0 after ~5 s.
4. **Adaptive stream:** with a JS viewer, check:
   - the layer chosen for a 640×360 tile vs full screen;
   - pause on minimise;
   - **no** pause when unfocused but visible;
   - the LOW → HIGH ramp time on first watch.
5. **NAT census (cheap, informs §5 forever):** have every client log its ICE candidate types and run a two-server
   STUN mapping test (same mapped port to both means endpoint-independent; different means hard NAT). Then we know
   our friends' real NAT/CGNAT situation per ISP, instead of guessing.
6. **`ethtool -i/-l` on the A1:** driver (virtio?) and queue count. This is for the record, not to tune.

---

## Appendix A. Experiment details (2026-10-14) [measured]

**Setup:**
- `livekit/livekit-server:v1.13` (image label v1.13.7) and `livekit/livekit-cli:v2.18`, both with host networking.
- The server was pinned with `--cpuset-cpus` to one E-core (or two for the multi-port test); the load generator ran
  on 12 other E-cores. Both containers had `--cpu-shares=10240` because the host was busy.
- `lk load-test --video-publishers 2 --audio-publishers 2 --subscribers {6|40} --video-codec h264 --layout speaker`,
  85 s. Measured over a 30 s window after 35 s of warm-up.
- CPU comes from `process_cpu_seconds_total`; packets from `livekit_packet_total{direction="outgoing"}`.
- Config: dev keys, a single UDP mux port, loopback candidate.
  - "one" = `rtc.ips.includes` restricted to loopback, so 1 socket.
  - "b128" = `batch_io: {batch_size: 128, max_flush_interval: 2ms}`.
  - "b5" = the same with 5 ms.

**Busy-host round (load average 50-100, 11 sockets, 3 interleaved reps), CPU % per 1k packets/s:**
- base 2.29/2.32/2.36;
- b128 1.98/2.06/2.26 (**−3% to −14%**);
- GOGC=400 2.23/2.41/2.31 (no change).

**Busy→quiet transition (1 socket, 2 reps):**
- one 2.04/1.22;
- b128one 1.42/0.99 (**−19 to −30%** within each pair);
- b5one 1.51/1.24.

**Quiet round (load average ~4, 1 socket, 3 reps):**
- one 1.23/1.47/1.08;
- b128one 1.12/1.17/1.08 (median **−9%**).

**Pulse-sized (6 subs):** one 4.9%/4.6% CPU; b128one 3.8%/4.3% CPU (at ~2k packets/s).

**Idle (60 s, no participants):**
- 11 sockets: base 0.12-0.17%, b128 3.0-3.1%.
- 1 socket: base 0.15-0.17%, b128 1.7%, b5 1.3-1.5%.

**Two cores:** 1 port 17.5/29.6% vs 2 ports 19.8/27.3% CPU at ~13k packets/s. No consistent difference.

**Profiles:** base 15.5% vs b128 13.3% of a core over the same 30 s (−15%). The send-syscall path was 59% vs 52-54%
of samples.

**Caveat:** absolute numbers swing ±30% with host load. Only the within-pair deltas and the idle numbers are solid.

---

## Sources (read 2026-10-14 unless noted)

- LiveKit server v1.13.7 source (tag v1.13.7): `pkg/config/config.go` (EnableWarp, DebugHandler, RTC keys),
  `pkg/rtc/subscribedtrack.go` (adaptive start at LOW unless EnableStartAtDesiredQuality), `pkg/rtc/participant.go`,
  `pkg/service/turn.go` (TURN auth and permission handler), `config-sample.yaml` (batch_io, udp_port range advice),
  `cmd/server/main.go` (cpuprofile flag), `go.mod` (Go 1.26, pion forks): https://github.com/livekit/livekit/tree/v1.13.7
- LiveKit v1.13.7..v1.13.10 commit log, incl. #4992 TURN hardening, #4951/#4953, #4928, #5000:
  https://github.com/livekit/livekit/compare/v1.13.7...v1.13.10 ; releases: https://github.com/livekit/livekit/releases
- LiveKit PR #4595 (start at desired quality, merged 2026-06-19): https://github.com/livekit/livekit/pull/4595
- LiveKit PR #4875 (broadcast without per-packet allocations, merged 2026-09-16): https://github.com/livekit/livekit/pull/4875
- LiveKit PR #4899 (fan-out threshold with batch IO, merged 2026-09-23, includes µs/packet benchmarks): https://github.com/livekit/livekit/pull/4899
- LiveKit PR #3765 (CPU profiling flag, 2025-06-26): https://github.com/livekit/livekit/pull/3765
- `livekit/mediatransportutil` @ v0.0.0-20260821083140-f234b534b095: `pkg/rtcconfig/config.go` (BatchIOConfig, 16 MiB
  buffers), `pkg/rtcconfig/webrtc_config.go`, `pkg/transport/createmux.go`: https://github.com/livekit/mediatransportutil
- `pion/transport` v4.1.0 `udp/batchconn.go` (sendmmsg ping-pong batcher): https://github.com/pion/transport/blob/master/udp/batchconn.go ;
  open PR #412 "Wake all batch writers after flush" (2026-09-28): https://github.com/pion/transport/pull/412
- LiveKit JS SDK, main @ 6c20018 (2026-10-14): `src/room/track/LocalVideoTrack.ts` (dynacast layer pausing),
  `src/room/track/RemoteVideoTrack.ts` (adaptive stream, pauseVideoInBackground), `src/room/defaults.ts`:
  https://github.com/livekit/client-sdk-js
- Go 1.25 release notes (container-aware GOMAXPROCS): https://go.dev/doc/go1.25
- Cloudflare Realtime SFU pricing (updated 2026-09-22): https://developers.cloudflare.com/realtime/sfu/pricing/
- Cloudflare Realtime SFU simulcast (updated 2026-09-22): https://developers.cloudflare.com/realtime/sfu/simulcast/
- Cloudflare Realtime SFU limits (updated 2026-10-06): https://developers.cloudflare.com/realtime/sfu/limits/
- Cloudflare blog, "Cloudflare Calls: millions of cascading trees all the way down" (2024-04-04): https://blog.cloudflare.com/cloudflare-calls-anycast-webrtc/
- Cloudflare blog, MoQ relay tech preview (2025-08-22): https://blog.cloudflare.com/moq/
- Tailscale, "How NAT traversal works" (2020-08-21): https://tailscale.com/blog/how-nat-traversal-works
- BlogGeek.me, "WebRTC TURN" (2025-11-10; ~80% of connections resolve without TURN): https://bloggeek.me/webrtc-turn/
- APNIC IPv6 measurement, South Africa (data 2026-10-11, ~4% capable): https://stats.labs.apnic.net/ipv6/ZA and
  https://data1.labs.apnic.net/v6stats/v6economy/ZA.json
- NVIDIA Video Encode and Decode GPU Support Matrix (GeForce: 12 concurrent NVENC sessions): https://developer.nvidia.com/video-encode-and-decode-gpu-support-matrix-new
- mediasoup scalability (undated): https://mediasoup.org/documentation/v3/scalability/
- Galene README: https://github.com/jech/galene
- str0m README: https://github.com/algesten/str0m
- OCI Compute shapes (A1: 1 Gbps/OCPU; networking type not stated): https://docs.oracle.com/en-us/iaas/Content/Compute/References/computeshapes.htm
- OCI Always Free resources (A1 1,500 OCPU-h / 9,000 GB-h = 2 OCPU / 12 GB; idle reclaim rule; 10 TB egress): https://docs.oracle.com/en-us/iaas/Content/FreeTier/freetier_topic-Always_Free_Resources.htm
- Discord, "How It All Goes Live" (2024-03-05): https://discord.com/blog/how-it-all-goes-live-an-overview-of-discords-streaming-technology
- Earlier Pulse research: `screenshare-server-quality.md`, `screenshare-summary.md`, `opt-server.md` (same folder).
