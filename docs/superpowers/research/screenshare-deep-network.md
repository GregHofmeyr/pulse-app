# Screen share on bad networks: keeping the stream smooth, and telling people the truth

**Date:** 2026-10-14 · **Scope:** how a game screen share behaves on South African home networks. Video is sent from
WebView2 with `livekit-client`; voice is still sent from Rust. The report covers what makes viewers see stutter,
what to change, what to show users, and how to test it. It builds on `screenshare-summary.md` and the three
screen-share reports beside it.

**Labels:**
- **[cited]**: from a source listed in §11 (all read 2026-10-14 unless a date is given).
- **[measured]**: I ran it on 2026-10-14 in the local lab (§9).
- **[est.]**: my own engineering estimate.
- **[speculation]**: a likely explanation I could not confirm.

**How far to trust the lab:**
- **Set-up.** One Linux PC runs LiveKit v1.13.7 and headless Chromium 151 (Chrome for Testing) with `livekit-client`
  2.22.4. Everything runs inside unprivileged network namespaces shaped with `tc netem`.
- **Video source.** A synthetic 1280×720, 60 fps canvas, encoded by **software OpenH264** at 4.5 Mbps. It is not a
  real game on NVENC; hardware encoders have different rate control and keyframe sizes.
- **Voice stand-in.** A second `livekit-client` connection (Opus + RED) in the same browser, not the Rust SDK. On
  LiveKit neither one has congestion control on audio (§3.3), so it is a fair stand-in for queueing effects.
- **Rendering.** Headless Chromium presents fewer frames than it decodes, so "rendered fps" is only a relative
  signal. Freezes and gaps are measured with `requestVideoFrameCallback` and `freezeCount`.
- **Repeats.** Most scenarios ran once. The loss, recovery and outage scenarios ran 3–8 times, because results
  varied between runs; the counts are given.
- **Contamination.** For one stretch, another job loaded the PC to a load average of 70+ on 32 threads. I threw
  those runs away and kept them only as the "CPU starvation" example in §1.2.
- **Bottom line.** Treat the numbers as directions and orders of magnitude, then confirm them on Windows in the
  spike.

---

## 0. TL;DR

1. **The friends complain about stutter, not delay, so this report is about stutter.** Six causes produce it (§1).
   The fixes that remove the most stutter, in order:
   1. **Never fill the streamer's upload.** Cap video at ~70% of a measured upload, and pick the preset
      automatically.
   2. **Signal `rtx-time=300` in the viewer's SDP answer.** One unrepaired lost packet freezes the picture for
      **~3.1 s**, because libwebrtc waits 3 s before asking for a keyframe [cited]. That happened at 1% loss in 7
      of 8 runs [measured]. With the munge it took **~0.9–1.1 s**, and there were **zero freezes ≥ 2.5 s** in
      either test run [measured].
   3. **Always publish a 30 fps lower simulcast layer.** Without it, a viewer whose link was too slow got **no
      picture at all for 45 s** [measured].
2. **SA upload reality** [cited]:
   - Entry fibre is now mostly **25/25 or 30/30**. The worst common tiers are 20/10 and 40/10.
   - Mobile upload medians are **~11–17 Mbps**, and 5G is no better than LTE. Evenings dip.
   - Ookla's own data puts **SA latency at ~0.5 s once the upload is saturated** (mobile ~1 s), about 2× Europe.
3. **Voice gets no congestion control on LiveKit, and video's queue is voice's delay.**
   - LiveKit doesn't negotiate transport-wide feedback for published audio, so Opus is never in a bandwidth
     estimate [cited]. Voice can't be starved of bandwidth, but it sits in the same router queue.
   - With video at 90% of a 5 Mbps upload, voice RTT went 21 → 55 ms. Capped at 60%, it was 31 ms [measured].
   - A sudden capacity drop cost voice **1 s at 10% concealment** [measured].
   - DSCP marking won't help: Windows needs admin rights or a QoS policy, and ISPs ignore it [cited]. **Headroom
     is the only lever.**
4. **There is no FEC, so loss recovery is NACK plus keyframes** [cited]. NACK repairs 1–5% random loss at 20 ms RTT
   with ~50–230 kbps of retransmits [measured]. At 10% loss, Chromium's loss-based estimator collapses the stream
   to ~1 Mbps and ~4–15 fps [measured].
5. **Recovery is slow, and it varies.**
   - **After the upload capacity comes back,** the stream took **5–8 s in 4 runs and ~30–45 s in 4 runs** to reach
     full rate [measured].
   - **After a 25 s media-only outage** (signalling still up), viewers got a usable picture back after **3–61 s**:
     3, 5, 25, 26, 42, 58 and 61 s across 7 runs [measured].
   - **A full reconnect recovers fast.** After a 25 s outage that included signalling, the picture was back
     **4 s** after the network returned, at full 60 fps after 10 s [measured].
   - **Don't trust the quality badge.** LiveKit's connection quality said "excellent" while the picture was frozen
     [measured].
6. **Transport fallback works but is slow to trigger.** With UDP blocked mid-stream, `livekit-client` took **~16 s**
   to notice and move to ICE/TCP, and never moved back [measured]. TCP itself was fine at 20 ms RTT with 1–3% loss
   [measured, with a caveat in §6.2].
7. **Two stats people rely on lie about bandwidth problems.**
   - `qualityLimitationReason` stayed `none` throughout a hard 4 Mbps cap and at 10% loss [measured].
   - The streamer's LiveKit `ConnectionQuality` stayed "excellent" at 10% uplink loss [measured].
   - §7 gives rules that work: target vs cap, sent vs captured fps, pacer delay, and freezes.
8. **The streamer should send a 2 s health summary to viewers** as a data message. Then every viewer's bug report
   holds both sides, and the UI can say whose link is at fault (§7).

---

## 1. Where viewer stutter comes from, and how to design it out

The friends' Discord experience matches the lab: what viewers notice is **frames not arriving on time** (a freeze,
choppy motion, a sudden blur). Glass-to-glass delay of 100–300 ms is rarely noticed. So the design rule is: **trade
delay for smoothness whenever we can.**

### 1.1 The six causes

| # | Cause | What the viewer sees | Where it happens |
|---|---|---|---|
| A | **Sender upload congestion.** Video plus everything else exceeds the upload, a queue builds, and Chromium's GCC cuts the bitrate | choppy (fps drops), then blur; slow to get better afterwards | streamer's router/modem |
| B | **A lost packet that isn't repaired** (Wi-Fi, LTE, a shallow router buffer, bursts) | **a ~3 s freeze**, then the picture snaps back on a keyframe | any hop |
| C | **Encoder or capture overload** at the source (GPU busy with the game, software encode, encoder session limits) | low or uneven fps, **with no network symptoms** | streamer's PC |
| D | **Forced keyframes and SFU layer switches** (viewer joins, layer up/down, keyframe requests) | a short hitch; blur steps when a viewer is moved down | SFU ↔ streamer |
| E | **Viewer downlink congestion or loss** | that viewer only: choppy, freezes, or stuck on the blurry layer | viewer's link |
| F | **Viewer decode/render**: no hardware decode, a hidden or occluded window, frame pacing against the display | judder while the network stats look clean | viewer's PC |

### 1.2 What each looked like in the lab [measured]

| Cause | Test | Result |
|---|---|---|
| A | upload 20 → 4 Mbps, deep (250 ms) router buffer | GCC cut the target within ~1 s (4.5 → 2.9 Mbps). Video ran 35–45 fps at 720p while capped, with no freezes. **Voice: 1 s at 10% concealment, an RTT spike to 180 ms, and a jitter buffer that grew 50 → 170 ms and took 20+ s to drain** |
| A, shallow buffer | 5 Mbps upload, 40 ms buffer (cheap router) | GCC can't sit near capacity. Burst losses gave **7–8 freezes in 95 s, each ~3.1 s** (2 runs). The same link with a deep buffer: **0 freezes**, but +34 ms of voice RTT |
| B | random loss on the streamer's upload, 20 ms RTT, 8 runs | **≥ 2.5 s freezes, per 27 s of loss:** 7 of 8 runs at 1%, 0 of 8 at 3%, 5 in 8 runs at 5%, 15 in 8 runs at 10%. Roughly **one ~3 s freeze per minute at 1–5% loss**. The 3% result looks like run-to-run luck, not a real dip |
| C | CPU starvation (another job on the PC) | fps swung 1–30, **47 freezes in 90 s with zero packet loss**, and the voice jitter buffer reached 250–470 ms. `qualityLimitationReason` **did** say `cpu` here |
| D | a new viewer joining every 20 s, clean link | one keyframe request and one keyframe per join. The existing viewer's longest frame gap was 50 ms: **invisible** |
| E | viewer link capped at 3 Mbps under a 4.5 Mbps stream | **With simulcast:** 2–8 s of choppy 36 fps, then a switch to 640×360@30. **Without simulcast:** **no decodable frame for 45 s** (26 keyframe requests, 13.6k packets lost), because the SFU keeps pushing the only layer (`allow_pause: false`) |
| E | viewer downlink loss | without simulcast: 4 freezes in 30 s at 3%, 11 at 8%. With simulcast (on the 360p layer by then): 0–2 at 3%, 5–10 at 8% |
| E, recovery | viewer link back to normal after the cap | **The viewer stayed on 640×360 for the rest of the run, 25–110 s.** Its receive estimate climbed only ~100 kbps/s, and the 720p layer needs ~5.4 Mbps |
| F | – | not testable in headless Chromium (§10) |

### 1.3 The fixes, ranked by how much stutter they remove

The ranking is my estimate [est.]. It weighs how often each cause should hit this group (home fibre, some LTE,
evening gaming) against how bad it is when it does. Evidence is given for each.

1. **Never fill the streamer's upload (cause A, and much of B).**
   - Cap `maxBitrate` at ~**70% of a measured upload**: the pre-flight test in §8.1, then the running stats.
   - Pick the preset from that number.
   - *Evidence:* at 90% of a 5 Mbps link, voice gained +34 ms [measured]; at 60%, +10 ms [measured]. Near capacity
     on a shallow-buffer router, the stream froze for 3 s every ~10–15 s [measured]. SA lines bloat to ~0.5 s when
     the upload is saturated [cited].
2. **Shorten the freeze after an unrepaired loss (cause B) by signalling `rtx-time`.**
   - libwebrtc's receiver waits `kMaxWaitForFrame` = **3 s** for a decodable frame before requesting a keyframe. If
     the negotiated RTX codec carries `rtx-time`, it waits only 3 × rtx-time [cited: `video_receive_stream2`].
     LiveKit doesn't send `rtx-time`.
   - Adding `;rtx-time=300` to the `apt=` fmtp lines of the **viewer's local answer** (a small
     `setLocalDescription` wrapper):
     - **long freezes:** 2 runs × 5 loss levels, **zero freezes ≥ 2.5 s**, against 27 in 8 control runs; the
       remaining unrepaired-loss freezes were 0.9–1.1 s [measured];
     - **shallow-buffer router:** the worst gap fell from ~3.1 s to 1.1 s, and freezes from 7–8 to 4 [measured].
   - Munging only the remote (server) description did nothing [measured]. Chromium takes the value from the
     receiver's own negotiated codec.
   - **Cost:** the receiver also uses rtx-time as its NACK history. 300 ms comfortably covers SA RTTs of 20–160 ms.
   - **Clean fix:** an upstream LiveKit option to send rtx-time. Until then, the wrapper is ~10 lines.
3. **Always publish one lower simulcast layer at 30 fps (cause E).**
   - It turns "no picture for 45 s" into "a few seconds of choppiness, then 360p" [measured].
   - **Don't use the defaults:** the Rust SDK's default is a 3 fps fallback, and the JS default is half resolution
     at maxBitrate/4.
   - Keep `allow_pause: false`.
4. **Hardware encode, and detect encoder overload (cause C).**
   - H.264 through the GPU.
   - Treat `qualityLimitationReason == "cpu"`, or sent fps < 0.8 × captured fps, as overload. Then step the preset
     down **before** viewers notice.
   - This cause has no network symptom, so it needs its own detector.
   - *Evidence:* CPU starvation gave the worst stutter of any lab test, with zero packet loss [measured].
5. **Reconnect rather than wait when the stream is stuck (cause A, recovery).**
   - After long media outages, Chromium's sender sometimes sits at 0.1–0.3 Mbps and 1–2 fps for up to a minute
     [measured, 4 of 7 runs over 25 s].
   - A full reconnect brought the picture back 4 s after the network did [measured].
   - **Rule:** if the link is demonstrably alive (RTCP reports arriving, RTT normal) but `targetBitrate` stays
     < 400 kbps for ~8 s, disconnect and reconnect the hidden video participant.
   - My one lab attempt at this came back with video paused, probably my own bug (reusing the track across rooms
     with dynacast). Prove it in the spike.
6. **Make the SFU downgrade viewers faster: `use_send_side_bwe: true` (cause E).**
   - **Downgrade time:** 3 Mbps cap → 360p switch in **~2 s, vs ~8 s** with the default REMB estimate [measured,
     1 run each].
   - **Collateral damage:** the viewer's voice took 18% concealment for 1 s, instead of 39% for several seconds.
   - **What it didn't fix:** it didn't speed up the upgrade back to 720p.
   - Worth enabling, then watching for side effects.
7. **Keep the default `maintain-resolution`, and don't switch to `balanced` without testing it on hardware
   (causes A and C).**
   - With OpenH264, `balanced` started at **480×270 at 12 fps** and never got back above 960×540 in 3 minutes, even
     with 9 Mbps free [measured].
   - It did avoid most 3 s freezes at 1–5% loss: lower resolution means fewer packets per frame.
   - Retest it on NVENC/Media Foundation.
8. **Tell the streamer which link is bad (all causes).** Plugging in a cable, pausing a download or picking a lower
   preset fixes most of cause A at home. People only do that when the app says so plainly (§7.3) [est.].

**Tested, and no measurable benefit** [measured]:

| Fix | Result |
|---|---|
| **The bitrate "nudge"** (re-applying `maxBitrate` every 4 s to trigger Chromium's probe) | recovery after a capacity drop was 6, 27 and 43 s with it, against 5–32 s without. One jitter run looked better with it, but that is too little evidence |
| **A viewer "stall watchdog"** (flip `setEnabled(false/true)` after 600 ms without a frame) | it didn't produce a keyframe sooner. The SFU didn't re-request one |
| **LiveKit PLI throttle at 300 ms** instead of 500 ms/1 s | the 3 s freezes are a receiver timer, not a PLI delay |
| **Republishing the track** after a stall | 41–61 s recovery, the same as doing nothing |
| **A viewer `jitterBufferTarget` of 150 ms** | the worst gap fell from 84 to 50 ms under bursty jitter, which was already smooth. A small effect at a 120 ms delay cost. Keep it as an optional "smooth mode", not a default |

**Not available or not useful:**
- **FEC:** not negotiated by LiveKit, and Chromium can't use ULPFEC with H.264 plus NACK [cited].
- **DSCP:** §3.4.
- **Periodic keyframes:** §8.3.

---

## 2. South African network reality

The upload numbers disagree a lot: roughly **17 to 53 Mbps median fixed upload**. That is a matter of method:
- **Ookla** tests are started by users and lean towards wired PCs.
- **Opensignal** measures phones in the background, so home broadband means Wi-Fi.
- **MyBroadband** reports means.

For a gamer streaming from a wired PC, Ookla is the closest proxy. For a friend on laptop Wi-Fi, Opensignal is.
All [cited] figures below are from §11.2.

### 2.1 Fibre packages (2025–2026) [cited]

- **Symmetric 25/25 or 30/30 is now the common entry tier** on Openserve (30/30), MetroFibre (30/30, and 56 of its
  63 tracked packages are symmetric), Octotel (25/25) and Vuma Core (25/25).
- **Asymmetric "value" tiers survive:**
  - Vuma Reach: 20/10, **40/10** and 100/50.
  - Openserve Web Connect prepaid: 20/10 and 40/20.
  - **40/10 is the worst common tier for a streamer.**
- **Higher asymmetric tiers are usually 2:1:** 50/25, 100/50, 1000/500. Frogfoot's entry tier is 60/30.
- **Retail ISPs** (Afrihost, Webafrica, Cool Ideas, RSAWEB, Axxess, Vox) resell the FNO tiers. The upload comes from
  the FNO product; the ISP affects peering, CGNAT and latency.

### 2.2 What people measure [cited]

| Source | Fixed upload | Mobile upload |
|---|---|---|
| Ookla Global Index, May 2026 | **40 Mbps median** (7 ms) | – |
| Ookla open data, Q2 2026 tiles (a sub-agent's own query) | tile median 53, **p25 21**; 14% of tests in tiles averaging < 10 | tile median 15.6, **p25 7.6**; 33% of tests < 10 |
| MyBroadband Q1 2026 | 28 average | 19 average |
| Opensignal fixed, Q1 2026 (a Wi-Fi panel) | best ISP 17.5; Vodacom 11.1; rain 7.8; MTN 7.7 | – |
| MyBroadband mobile, Q2 2026 | – | MTN 32.5, Cell C 32.5, Vodacom 17.3, Telkom 12.1, rain 11.3 |
| Ookla mobile, H1 2025 | – | MTN 13.65; **Vodacom 5G 11.86** |

- **5G upload is barely better than LTE.** Non-standalone 5G sends its upload over LTE [cited + est.].
- **SA mobile speeds bottom out around 19:00–21:00,** exactly when friends game [cited]. Plan for a 30–50% evening
  drop on LTE/5G upload [est.].

### 2.3 Bufferbloat: the hidden SA problem [cited]

From Ookla open data, Q2 2026:

| | Idle latency | Latency while the upload is saturated |
|---|---|---|
| SA fixed | 12 ms | **517 ms** median (p75 784) |
| SA mobile | 25 ms | **~1,020 ms** (p75 1,700) |
| London / Amsterdam fixed (for scale) | 8 ms | 258 / 183 ms |

If anything fills the upload (the stream, a Steam download, a cloud backup), everything behind it in the queue,
voice included, arrives 0.5–1 s late [est.]. GCC backs off when it sees delay growing, but other apps and other
people in the house don't. The lab showed the same split:
- **Deep queue:** smooth video, delayed voice.
- **Shallow queue:** drops and 3-second freezes.

### 2.4 CGNAT [cited]

- **Mobile:** rain LTE/5G uses CGNAT, and I assume all SA mobile operators do.
- **Fibre:** Afrihost and Webafrica put some DHCP fibre customers behind CGNAT. Opting out is on request or paid.
- **UDP:** no report was found of any SA operator blocking or throttling UDP.

**What it means for Pulse** [est.]:
- CGNAT doesn't matter for an SFU, since every connection is outbound.
- ICE consent checks keep the UDP mappings alive.
- TURN/TLS on 443 matters only on strict networks (work, campus, hotel).

### 2.5 Peering to Oracle Johannesburg [cited]

- **Oracle (AS31898)** is at **NAPAfrica Johannesburg (2×100G) and JINX (100G)**.
- **Also at NAPAfrica JHB:** Vodacom, MTN, Telkom/Openserve, rain, Cell C, Vox, MetroFibre, Cool Ideas, RSAWEB,
  Cybersmart, Frogfoot and Vumatel.
- **No IX ports listed:** Afrihost, Webafrica, Axxess and MWEB. They reach Oracle through an upstream: still
  in-country, but possibly one hop more.
- **Cape Town → JHB:** ~18–30 ms on good fibre [cited, low-quality sources; est.].
- **Pulse's own voice RTTs** of 20–160 ms suggest the high end is Wi-Fi, LTE or bufferbloat, not routing [est.].

### 2.6 Loadshedding [cited]

- **Status:** no loadshedding since **16 May 2025** (504 days at 2 Oct 2026).
- **What remains:** local "load reduction" for ~2.8% of Eskom customers. Eskom's outlook flags a risk again in
  2029–2030.
- **When it does happen:**
  - Home routers and ONTs need a UPS; 600–1000 VA lasts ~2–4 h.
  - Tower batteries last 6–12 h and recharge slowly.

**Design for** short outages and network switches (Wi-Fi → phone hotspot), not daily blackouts. Those are exactly the
reconnect cases in §6 [est.].

### 2.7 What upload can a streamer really sustain? [est.]

These figures are 70–80% of the measured upload, then derated for evenings, Wi-Fi and bufferbloat.

| Connection | Typical upload | Safe video bitrate | Fits which Pulse preset |
|---|---|---|---|
| Fibre 20/10 or 40/10 | 10 | **6–7 Mbps wired**, 4–5 on Wi-Fi | 720p60 (4.5) or 1080p30 (4) |
| Fibre 25/25–30/30 | 25–30 | 15–20 | 1080p60 (7), Source 1440p60 (11) |
| Fibre 50/25 and up | 25+ | 18–35 | anything; cap at ~12 anyway (encoder, SFU egress) |
| LTE, including fixed-LTE | 8–17 median, p25 ~7.6, evening dips | **3–5** | 720p30 (~3) only; 720p60 risky |
| 5G home | ~11–12 median | 5–8, variable | 720p60 / 1080p30 |

**What this means for the summary's presets:**
- The **720p60 default at 4.5 Mbps** fits every fibre tier when wired.
- The summary's **"~8 Mbps upload for 720p60" rule** matches the 70% rule.
- LTE/5G streamers need an **auto-selected 720p30 at ~2.5–3 Mbps**.
- "Source 1440p60" needs ≥ 25/25 symmetric fibre.

---

## 3. Congestion control: what reacts, and how fast

### 3.1 Chromium's sender (GoogCC) [cited]

**Delay detector**
- A trendline over 20 packet groups, with an adaptive threshold that starts at 12.5 ms.
- It declares overuse after a 10 ms sustained trend.

**Rate control**
- **On overuse:** rate = **0.85 × measured throughput**.
- **When increasing:** multiplicative **+8%/s** while there is no link-capacity estimate. Once there is one, the
  increase is **additive**: about one packet per 2×(RTT + 100 ms), i.e. tens of kbps per second.

**Loss-based estimator v2**
- On by default since late 2022.

**Probing**
- At start: 3× and 6× the start rate.
- "ALR" probes every 5 s at 2× when the encoder uses < ~65% of the estimate.
- A probe when the max allocated bitrate changes.

**Pacer**
- Paces at 1.1× the target once feedback flows.
- Drains so no packet waits more than 2 s.
- Priority order: audio > audio retransmit > video retransmit > video > padding. That order applies **only within
  one PeerConnection**.

**SCReAM v2 / L4S**
- In the tree behind field trials. They need RFC 8888 feedback, which LiveKit doesn't negotiate, so **default
  GoogCC is what we get**.

**In the lab** [measured]
- **Capacity drop:** the target fell within the first 1 s sample, then settled at 75–85% of the new capacity.
- **Capacity restored:** recovery to full rate took **5–8 s or ~30–45 s**, a bimodal split over 8 runs. My
  [speculation] is that it depends on whether a probe succeeds early or the controller is stuck in its slow
  additive mode.

### 3.2 The SFU side [cited]

**Publisher feedback**
- LiveKit sends the publisher **transport-wide CC feedback**, so Chromium's GoogCC drives the encoder.
- It sends no REMB to publishers.

**Viewers**
- By default (`use_send_side_bwe: false`), LiveKit uses each viewer's **REMB** receive estimate. The stream
  allocator picks a layer per viewer.
- It downgrades at once on congestion. Upgrades need probes at 120% overage to succeed.
- Screen share gets higher allocation priority than camera video.
- **`allow_pause: false` means the SFU never pauses a viewer's video.** Without simulcast it keeps forcing the only
  layer into a pipe that can't carry it: the 45 s no-picture result.

**Lab results** [measured]
- **Downgrade:** 3 Mbps cap → 360p took ~8 s with REMB and ~2 s with `use_send_side_bwe: true`.
- **Upgrade back:** no upgrade to 720p within 25–110 s with either setting. The receive estimate climbed ~100 kbps/s
  from ~1.4 Mbps and was knocked back by later loss.
- *Practical consequence:* a viewer who had one bad minute stays blurry for a long time. The UI should say so, and
  offer "try higher quality", which calls `setVideoQuality(HIGH)` [est.].

**Connection quality (`ConnectionQualityChanged`)**
- A 0–100 score every 5 s, from loss, delay, bitrate and missing layers, computed on what the **server** sees after
  its own NACK repair.
- In the lab it said "excellent" at 10% uplink loss, and while a viewer was frozen at 1 fps after an outage
  [measured].
- It is useful for "lost", but **not a stutter indicator.**

### 3.3 Two PeerConnections on one uplink: does video starve voice?

**The key fact** [cited]
- LiveKit's publisher config gives **audio only NACK, no transport-cc**.
- libwebrtc only allocates and paces audio when a congestion-control feedback type was negotiated.
- So **the Rust voice connection has no working bandwidth estimate.** Opus sends at its fixed `max_bitrate` (Pulse:
  64 kbps plus RED, ~130 kbps on the wire) whatever happens.
- The video connection sees voice only as cross-traffic.

**So:**
- **Bandwidth:** voice can't be starved. It's small and never backs off.
- **Delay and loss:** voice shares the router queue, so it inherits video's queueing delay and drops.

[measured], voice stand-in on its own PeerConnection, same upload queue:

| Upload | Video | Voice RTT | Voice concealment | Voice jitter buffer |
|---|---|---|---|---|
| unconstrained | 4.5 Mbps | 21 ms | 0% | ~46 ms |
| 5 Mbps, deep buffer (300 ms) | 4.5 Mbps (90% of the link) | **55 ms** | 0% | 91 ms |
| 5 Mbps, deep buffer | **capped at 3 Mbps** (60%) | **31 ms** | 0% | 54 ms |
| 5 Mbps, shallow buffer (40 ms) | GCC settles at ~3.1–3.6 Mbps | 31–32 ms | 0% | 62–72 ms |
| 20 → 4 Mbps step, deep buffer | GCC 4.5 → 2.9 within ~1 s | **180 ms spike** | **10% for 1 s** | 50 → 170 ms, 20+ s to drain |
| viewer's own downlink capped (REMB) | the SFU took ~8 s to drop the viewer to 360p | – | **39% for several s** (the viewer's incoming voice) | up to ~3 s (a NetEq reaction to the burst) [measured; mechanism unclear] |

**Conclusions:**
- Keep video at **≤ 70% of the upload**.
- Expect a short voice glitch whenever the upload suddenly shrinks (someone else starts an upload). Nothing on our
  side prevents that [est.].
- **Two-PC coordination (§8.5)** feeds voice RTT into the video cap. It is the only way to put voice "inside" the
  congestion loop.

### 3.4 Can we prioritise voice? Mostly no

**DSCP marking** [cited]
- libwebrtc marks from `networkPriority`, and DSCP is on by default since 2022.
  - audio `high` → EF (46);
  - video `high` → AF41, `medium` → AF42.
- `livekit-client` sets **audio** to `high`.
- In the lab, Chromium on Linux marked the JS voice stand-in EF and the screen share DSCP 0 [measured: nft
  counters].

**Windows** [cited]
- Chromium marks via qWAVE. Its explicit `QOSSetOutgoingDSCPValue` call needs admin rights ("may fail").
- The Rust SDK's libwebrtc socket layer doesn't support DSCP on Windows, and the SDK has no priority option. So
  **voice from Rust leaves unmarked.**
- Policy-based QoS (`New-NetQosPolicy`, plus the "Do not use NLA" registry value outside a domain) can mark by
  executable. But it is a per-PC admin set-up, not something to ask friends to do [est.].

**On the path** [cited]
- ISPs bleach or ignore DSCP (RFC 8837).
- The only place it helps is **home Wi-Fi uplink**, via WMM (EF → voice queue; RFC 8325).
- A wired PC's bottleneck is the modem's upstream FIFO [est.].

**What works instead:**
1. headroom (the 70% cap);
2. dynacast (no viewers means no upload);
3. few forced keyframes;
4. SQM (CAKE/fq_codel) on the router, for the rare friend with an OpenWrt router: a help page, not a feature.

### 3.5 Recommended settings (congestion)

| Where | Setting | Value | Why |
|---|---|---|---|
| JS publish | **`screenShareEncoding`** (not `videoEncoding`) | `{ maxBitrate: min(preset, 0.7 × upload − 0.15 Mbps), maxFramerate, priority: 'high' }` | For a screen-share source, `livekit-client` ignores `videoEncoding` and falls back to **1080p15 at 2.5 Mbps** [measured: my first runs were stuck at exactly that] |
| JS publish | `degradationPreference` | keep the default `maintain-resolution` until `balanced` is tested on hardware | §1.3 #7 |
| JS publish | `simulcast` + `screenShareSimulcastLayers` | one layer at half resolution, **30 fps**, ~1–1.5 Mbps | §1.3 #3 |
| JS publish | `backupCodec` | off, unless AV1 ships | the extra encode costs CPU (cause C) |
| JS room | `dynacast: true`, `adaptiveStream: false` (set layers by hand) | – | server report §3 |
| JS viewer | `rtx-time=300` in the local answer | see §4.3 | §1.3 #2 |
| Server | `rtc.congestion_control.use_send_side_bwe` | `true` (watch in the spike) | §1.3 #6 |
| Server | `allow_pause` | `false` (default) | with a 30 fps low layer, degrade rather than pause |
| Server | `pli_throttle` | default | 300 ms changed nothing [measured] |
| Rust voice | nothing | – | protect it with headroom |

---

## 4. Loss recovery

### 4.1 What exists [cited]

**NACK and retransmission (RTX)**
- The sender keeps ~1 s of history; the receiver retries up to 100 times.
- Retransmits jump ahead of new video in the pacer.
- LiveKit repairs the streamer's losses (it NACKs the streamer) and answers viewers' NACKs from its own buffer.

**FEC: none**
- LiveKit registers no `ulpfec`, `flexfec` or video `red`.
- Chromium only uses ULPFEC with NACK on codecs with a picture ID (VP8/VP9), so H.264 would get none anyway.
- FlexFEC is behind field trials.

**RED for voice:** yes (Opus RED, on by default in the Rust SDK).

**Keyframes**
- No periodic keyframes (the default interval is 3,000 frames).
- Recovery relies on keyframe requests (PLI).
- LiveKit throttles PLIs per layer: 500 ms for the low layer, 1 s for mid/high.
- Chromium ignores requests within 300 ms of the last keyframe.

**The 3 s rule**
- `video_receive_stream2.h`: `kMaxWaitForFrame` = 3 s; `kMaxWaitForKeyFrame` = 200 ms.
- If the receive codec has `rtx-time`, both waits scale from it (3× rtx-time for frames).
- When the timer fires, the receiver sends a keyframe request.
- [speculation] Why some losses are never repaired at only 1% loss: a retransmit that is itself lost, or one that
  the SFU can't supply in time. The lab can't see inside the SFU. Either way, the receiver then sits out the full
  wait.

### 4.2 Measured loss behaviour [measured]

Streamer upload, random loss, 20 ms RTT, 4.5 Mbps 720p60. "Long" means freezes ≥ 2.5 s per 27 s of loss, summed
over runs:

| Loss | Retransmits | Viewer fps | Long freezes, 8 control runs | Long freezes, 2 runs with `rtx-time=300` |
|---|---|---|---|---|
| 1% | ~35–50 kbps | 60 | **7** | **0** |
| 3% | ~115–150 kbps | 60 | 0 | 0 |
| 5% | ~210–250 kbps | 57–60 | **5** | **0** |
| 10% | ~100–130 kbps (at a collapsed rate) | 4–15 | **15** | **0** (shorter freezes remained: 3–8 per 27 s) |

- **Bursty loss is worse than random.** A shallow (40 ms) router buffer gave 7–8 freezes of ~3.1 s in 95 s at only a
  few percent average loss. With `rtx-time=300`: 4 freezes, the worst 1.1 s.
- **Viewer downlink loss without simulcast:** 4 freezes in 30 s at 3%, 11 at 8%.

### 4.3 Settings to change

1. **`rtx-time=300` on the viewer's answer** [measured win].
   - Wrap `RTCPeerConnection.prototype.setLocalDescription` in the webview: append `;rtx-time=300` to every
     `a=fmtp:<pt> apt=<pt>` line.
   - Ask LiveKit upstream for a server option that does the same for every client.
   - **Risk:** a viewer with RTT > ~150 ms gets fewer retransmit chances before giving up. Use 300–500 ms if anyone
     is far away [est.].
2. **Keep the PLI throttle at its defaults** [measured: 300 ms changed nothing].
3. **Raise the server's UDP buffers** (already in the server report). A dropped burst at the SFU looks like cause B
   to every viewer at once.
4. **An optional viewer "smooth mode"**, `jitterBufferTarget` of 100–150 ms, for a viewer on bursty Wi-Fi/LTE. The
   effect was small in the lab.

---

## 5. Recovery: the "stuck choppy" problem

### 5.1 What happens [measured]

**After the upload capacity comes back**
- The stream reached full rate in 5–8 s (4 runs) or ~30–45 s (4 runs).
- Meanwhile video ran at 35–48 fps at 720p (with `maintain-resolution`).

**After a media outage while signalling stays up**
- **3 s and 10 s outages:** recovered within a couple of seconds.
- **25 s outages, viewer usable picture back after** 3, 5, 25, 26, 42, 58 and 61 s (7 runs).
- **In the slow cases:**
  - the sender sat at **0.1–0.3 Mbps and 1–2 fps**, with **300–1,400 ms of pacer delay**;
  - then it jumped back to 4.5 Mbps at once, with no event.
- **LiveKit's state during the outage:**
  - its resume fired, and reported "Reconnected" *during* the outage;
  - connection quality said "excellent" while the picture was still frozen.

**After an outage that also cut signalling**
- **10 s:** no reconnect event. The picture came back at once, but held 3.6 Mbps and 45 fps for 40+ s.
- **25 s:** a full reconnect. The picture was back **4 s** after the network returned, at 60 fps after 10 s.

**After heavy *reordering* jitter** (netem ±40–75 ms)
- The stream collapsed to 0.2–0.5 Mbps and didn't recover within 30 s of clean network (2 runs).
- In one run with the nudge, it held 2–3.6 Mbps and recovered at once.
- Bursty **in-order** jitter (netem `slot`, up to 150 ms) didn't hurt at all: 4.5 Mbps, 60 fps, 0 freezes
  (3 runs).

### 5.2 Fixes

1. **A Pulse-side stall detector that does a full reconnect** of the hidden video participant [est.; supported by
   the 25 s blackout result].
   - Trigger: "RTCP reports arriving and RTT normal, but `targetBitrate` < 400 kbps for 8 s".
   - Reconnect: `room.disconnect()`, then reconnect and republish.
   - Use a fresh `getDisplayMedia` track, or set `stopLocalTrackOnUnpublish: false` and check the track is still
     live.
   - **Test it in the spike.** My lab version came back with video paused.
2. **Honest UI:** "Recovering your stream…" for the streamer, and "Streamer's connection is recovering" for viewers.
3. **Not worth it:** the nudge or a plain republish [measured].

---

## 6. Transport fallbacks and reconnects

### 6.1 What the stack does [cited]

**Ports**
- LiveKit offers UDP 7882, then ICE/TCP 7881.
- TURN/UDP 3478 is used by Pulse as STUN.
- TURN/TLS (443/5349) isn't configured.

**`livekit-client` 2.22**
- **Resume:** re-signal plus an ICE restart.
- **Full reconnect:** if the resume fails, or the server says so.
- **Retry delays:** 0, 0.3, 1.2, 2.7 and 4.8 s, then 7 s × 5, with jitter.
- It listens for the browser's `online`/`offline` events.
- It forces a full reconnect if quality stays "lost" for 10 s while publishing.

**Rust SDK 0.9.x**
- 10 attempts, 300 ms base, ×2 backoff, max 7 s.
- One failed resume means a full reconnect. 15 s ICE timeout.

**The two connections reconnect independently.** After a network switch, voice and video may come back at different
times [est.]. Show each separately.

### 6.2 Measured [measured]

**UDP blocked from the start**
- Both connections came up on **ICE/TCP** with no visible delay.
- With 1% and 3% loss at 20 ms RTT: no freezes.
- With a 6 Mbps cap: behaved like UDP.
- *Caveat:* lab TCP runs over loopback with a 64 KB MTU. On a real 1,500-byte path with 100+ ms RTT, head-of-line
  blocking will be worse [est.; cited practitioner report of higher jitter over TCP].

**UDP blocked mid-stream**
- Detected after **~16 s** (`SignalReconnecting`), then resumed on TCP within 2 s. Viewers saw a **15.9 s freeze**.
- When UDP came back, **the client stayed on TCP**.

**Full reconnect after a 25 s blackout**
- **The voice stand-in's audio was never republished:** `livekit-client` logged "error trying to re-publish tracks
  after reconnection" (a TypeError). The video track came back fine.
- Most likely my lab created the audio track with `userProvidedTrack = false`, so the SDK tried to re-acquire it.
- **Lesson for Pulse:** create the screen-share track as user-provided (or via `publishTrack(mediaStreamTrack)`).
  After every `Reconnected`, check that the publication exists, and republish it ourselves if not.

### 6.3 Jitter

**Reordering jitter** (netem's normal-distribution jitter) collapses GoogCC: 2.9 → 0.5 → 0.2 Mbps at ±15/40/75 ms
[measured]. Real Wi-Fi/LTE jitter is bursty but **in order**. Modelled with netem `slot` (bursts of 5–30, 10–80 and
20–150 ms), the stream stayed at **full rate, 60 fps, 0 freezes** [measured, 3 runs]. Voice RTT rose with the slot
size, to 45–105 ms.

So **in-order jitter isn't a stutter source by itself; reordering is.** Reordering is rare on the internet, but
some LTE links and bonded or "load-balancing" routers produce it [est.].

### 6.4 Recommendations [est.]

1. **Keep `allow_tcp_fallback`.** Add TURN/TLS 443 only if a friend ends up on a UDP-blocking network.
2. **Don't wait 16 s.**
   - If the viewer gets no new frames, or the streamer gets no RTCP, for **3 s**: show "Reconnecting…" at once.
   - If nothing improves in ~8 s: reconnect the hidden video participant ourselves.
3. **Get back to UDP.** After a TCP fallback, reconnect at a quiet moment (dynacast says nobody is watching) to
   try UDP again.
4. **On a Windows network-change event:** kick voice and video together.
5. **Log the selected candidate type** at connect and on every change. Show "TCP fallback" in the stream health
   panel.

---

## 7. Telemetry and stream health UX

### 7.1 Stats to collect (every 2 s while streaming or watching)

**Streamer (screen-share sender):**

| Field | Use |
|---|---|
| `media-source.framesPerSecond` | what capture delivers. Low here means a capture/game problem (C) |
| `outbound-rtp.framesPerSecond`, `framesEncoded`, `framesSent` | sent fps. **Sent < captured** means encoder or rate-control drops (C or A) |
| `outbound-rtp.targetBitrate` vs our cap | **< 80% of the cap means upload-limited (A).** The reliable bandwidth signal |
| `candidate-pair.availableOutgoingBitrate` | GCC's estimate for this connection. It excludes the Rust voice traffic |
| `qualityLimitationReason`, `qualityLimitationDurations` | `cpu` is reliable for C. **`bandwidth` often doesn't show for A** (`none` in every lab test with `maintain-resolution`) |
| `totalPacketSendDelay` Δ / packets Δ | pacer queue in ms. > 50 ms means the sender is backlogged (A, or "stuck") |
| `remote-inbound-rtp.roundTripTime`, `fractionLost`, `roundTripTimeMeasurements` | upload RTT growth (bufferbloat), loss (B), and **link liveness** (the count keeps increasing while RTCP arrives) |
| `nackCount`, `retransmittedBytesSent`, `pliCount`, `keyFramesEncoded` | loss-repair load; keyframes forced by the SFU (D) |
| `encoderImplementation`, `powerEfficientEncoder` | hardware or software (only exposed while capturing, which a screen share is) |
| local candidate `protocol` / `candidateType` | UDP, TCP or relay |

**Viewer (screen-share receiver):**

| Field | Use |
|---|---|
| `framesReceived`, `framesDecoded`, `framesDropped`, `framesPerSecond` | **decoded ≈ received but render gaps** means F |
| `freezeCount`, `totalFreezesDuration`, `pauseCount` | the headline stutter numbers. A freeze is an inter-frame gap ≥ max(3× average, average + 150 ms) |
| `totalInterFrameDelay`, `totalSquaredInterFrameDelay` | frame-time variance (judder) |
| `jitterBufferDelay` / `jitterBufferEmittedCount`, `jitterBufferTargetDelay` | buffer depth |
| `packetsLost`, `nackCount`, `pliCount` | downlink loss (E) and keyframe requests (a burst of PLIs plus one long gap is the 3 s rule) |
| `frameWidth`/`frameHeight` changes | layer switches (D/E) |
| `candidate-pair.availableIncomingBitrate` | the viewer's receive estimate. Explains "stuck at 360p" |
| `decoderImplementation`, `totalDecodeTime` / `framesDecoded` | hardware decode and its cost (F) |
| `requestVideoFrameCallback` count and max gap | what actually reached the screen (F). Measure it in our `<video>` |

**Voice (Rust, already logged every 5 s):** RTT, jitter-buffer target, concealed %. Line these up with video by
wall-clock time.

### 7.2 "Stutter blame": which link is at fault

For each 2 s window:

1. **Streamer PC (C):** captured fps < 0.8 × chosen, **or** `qualityLimitationReason == cpu`, **or** encode time per
   frame > 0.8 / fps.
2. **Streamer upload (A):** `targetBitrate` < 0.8 × cap, **or** pacer delay > 50 ms, **or** upload RTT > baseline
   + 80 ms.
3. **Streamer loss (B):** upload `fractionLost` > 2%, **or** keyframe requests arriving while the streamer's own loss
   > 0.
4. **Viewer link (E):** viewer `packetsLost` rising while the streamer's summary is clean, **or** a layer drop while
   the streamer is at full rate.
5. **Viewer PC (F):** decoded fps ≈ sent fps but presented fps low or render gaps > 100 ms, **or** no hardware
   decoder.
6. **Server (rare):** **every** viewer loses packets in the same window while the streamer is clean. Check LiveKit
   Prometheus and `UdpRcvbufErrors`.

Rules 4–6 need both sides, so **the streamer broadcasts a ~100-byte health summary every 2 s** as a LiveKit data
message on the hidden video participant:

```json
{"v":1, "capFps":60, "sentFps":58, "tgt":4300, "cap":4500, "aob":6100,
 "rtt":24, "loss":0.4, "pacerMs":3, "kf":0, "proto":"udp", "enc":"hw"}
```

### 7.3 What to show people

**Streamer badge** (next to "LIVE"). The worst state wins, and a state shows only after holding for ≥ 4 s:

| State | Trigger | Text |
|---|---|---|
| 🟢 Good | none | "Streaming 720p60" |
| 🟡 Upload-limited | rule 2 | "Your upload is limiting quality (sending 2.9 of 4.5 Mbps). Close downloads or pick a lower preset." |
| 🟡 PC-limited | rule 1 | "Your PC can't keep up: sending 41 of 60 fps. Try 30 fps or 720p." |
| 🟠 Losing packets | rule 3 | "Your connection is dropping packets. On Wi-Fi? Try a cable." |
| 🔴 Recovering / reconnecting | no RTCP for 3 s, or the stuck rule in §5.2 | "Reconnecting your stream…" |
| ⚪ Nobody watching | dynacast: 0 viewers | "Nobody's watching: sending nothing" |

**Viewer overlay**
- A corner message that stays hidden unless something is wrong:
  - "Streamer's upload is limited"
  - "Streamer is reconnecting"
  - "Your connection is dropping packets"
  - "Low quality: your connection (try higher)"
  - "Your PC is dropping frames"
- Plus a "Stats for nerds" toggle with the raw numbers.

**Rules:**
- Never blame without evidence.
- Never use LiveKit's connection quality as the "good" signal.
- Use the same words in the bug report.

### 7.4 What goes into bug reports

Reports carry a **256 KB log tail**. 300 B every 2 s would be ~540 KB/h, which is too much. Instead:

**Every 10 s while streaming or watching:** one line per side (~250 B, i.e. ~90 KB/h) with:
- min/median/max over the window: fps, target, RTT, jitter-buffer depth;
- counts: freezes, keyframes, PLIs, NACKs;
- the blame flags.

**Immediately, on every event:**
- a state change, or a freeze > 500 ms;
- a layer, candidate or preset change;
- a reconnect;
- a change in `qualityLimitationReason`.

**Once per session:**
- encoder/decoder implementation, codec/fmtp, simulcast layers, preset, cap;
- the pre-flight result;
- Windows build, WebView2 version, GPU.

**In memory:** a ring buffer of the last 120 s of full 2 s samples, dumped into the report when it is filed. "It
stuttered just now" then always has the detail.

---

## 8. Ideas worth building

### 8.1 Pre-flight upload test → automatic preset [est.]

Before the first "Go Live", and whenever the network changes:

1. **Measure goodput.** POST ~4–8 MB of random bytes to our own server (Caddy → axum, body discarded) for ~4 s.
2. **Measure latency under load at the same time.** Ping over the existing gateway WebSocket during the upload. The
   Waveform-style grade compares idle with loaded latency: A+ < 5 ms … F > 400 ms [cited].
3. **Set the cap.** `cap = min(preset, 0.7 × goodput − 0.15 Mbps)`.
   - If loaded latency is > 200 ms, use 0.5× and say why: "your router queues a lot; we'll stream lighter".
4. **Cache it per network.** Re-run if the first minute's live stats disagree.

`livekit-client` ships a `ConnectionCheck` with a canvas publish test, capped at 2 Mbps. It's a template, not the
tool [cited: library source].

### 8.2 Viewer data-saver and "try higher quality"

- **LTE viewers** (Windows network-cost API, or the user's choice) default to the low layer.
- **A viewer stuck on the low layer** gets a one-click "try higher quality" (`setVideoQuality(HIGH)`), because
  LiveKit's own upgrade is slow (§3.2).

### 8.3 Scheduled keyframes? No

Periodic keyframes would shorten some freezes. But each one is a bitrate spike on the streamer's upload, sent to
every viewer, all the time [est.]. The `rtx-time` fix gets most of the benefit and only costs a keyframe when one is
really needed.

### 8.4 A built-in network simulator for Windows testing

**The idea**
- CDP's `Network.emulateNetworkConditionsByRule` documents that global conditions also apply to **p2p connections**,
  with WebRTC packet-loss and queue-length fields [cited: CDP types in playwright-core 1.64].
- WebView2 exposes CDP (`CallDevToolsProtocolMethodAsync`), so a dev build could have a "simulate bad network" menu
  with no admin rights and no driver.

**What I found**
- The deprecated `emulateNetworkConditions` did **not** visibly throttle the WebRTC upload in my one test, which ran
  under heavy CPU contention [measured, low confidence].
- The `ByRule` variant is wired into the lab runner but not re-tested.
- Verify it on Windows before relying on it.

### 8.5 Two-PC coordination

- The Rust voice connection already measures RTT and jitter. Send them to the webview every 2 s.
- If voice RTT rises > 60 ms above baseline while streaming, lower the video cap by 15%. Raise it again after 30 s
  of calm.
- This puts voice into video's congestion loop by hand, since LiveKit won't negotiate TWCC for audio [est.].

---

## 9. The lab

### 9.1 Layout

Everything is in `/tmp/claude-1000/screenshare-net/` on the dev PC, not in the repo.

**Server**
- `livekit-server` v1.13.7, copied from the official image.
- It runs in an **unprivileged user + network namespace** (`unshare -Urn`), on a dummy interface.

**Streamer and voice stand-in**
- They share that namespace.
- Their media to the SFU crosses `lo`. There a `prio` qdisc steers packets "to SFU" and "from SFU" into two `netem`
  qdiscs, so both clients share one shaped upload queue.

**Viewer**
- It runs in a nested namespace behind a veth pair, so its downlink is shaped on its own.

**Driving each role**
- Each role is a headless Chromium page (`www/page.html`).
- A Playwright runner samples `getStats` every second, plus `requestVideoFrameCallback` on the viewer.

**Scripts**
- `scenario.sh` replays a timed schedule of `net.sh` commands.
- `analyze.py` prints per-segment medians and a per-second timeline.
- `longfreeze.py` counts long freezes.

**Shaping knobs**
- `rate` + `buf` (a FIFO bottleneck of that many ms), `delay`, `jitter`, `loss`, `slot` (bursty in-order), `udpblock`
  and `blackout` (signalling too).
- Fix toggles are URL parameters: `rtxt` (rtx-time munge), `jbt`, `nudge`, `wd`, `repub`, `rejoin`, `deg`, `sim`.
- Server variants use `LKCFG=`.

To run one scenario:

```sh
unshare -Urn ./ns-init.sh ./scenario.sh s2 sched/s2.txt && python3 analyze.py s2
```

### 9.2 Scenario matrix (reuse it for the spike)

| Id | What | Schedule |
|---|---|---|
| s0 | baseline | 10 ms each way |
| s1 / s1b | streamer upload 20 → 4 → 20 Mbps | 250 ms (bloated) / 50 ms buffer |
| s2 | streamer upload, random loss | 0 → 1 → 3 → 5 → 10 → 0% |
| s3 / s3b | streamer jitter | netem reordering jitter / `slot` bursts (in order) |
| s4sim / s4nosim / s4ss | viewer downlink 3 Mbps cap, then 3% and 8% loss | with / without simulcast / send-side BWE |
| s5 / s5cap / s5s | 5 Mbps upload: video 4.5 vs 3 Mbps; deep vs shallow buffer | voice vs video |
| s6 / s6m | UDP blocked from the start / mid-stream | TCP fallback |
| s7 / s7long / s7b | media outages of 3/10/25 s; signalling-included blackouts of 10/25 s | reconnect and recovery |
| s8 | a new viewer every 20 s | join keyframes |
| suffixes | the same with a fix applied | `n` nudge, `rtx`/`rtxL` rtx-time (remote/local), `jbt`, `wd`, `r` repeat, `pli`, `ss`, `bal`, `j` rejoin |

**SA profiles for the Windows spike** (a Linux router, §10):

| Profile | Up | Down | RTT | Loss | Notes |
|---|---|---|---|---|---|
| Good fibre (Gauteng) | 25 | 25 | 10 ms | 0 | baseline |
| Value fibre | 10 | 40 | 15 ms | 0 | the 40/10 case |
| Cape Town fibre on Wi-Fi | 30 | 30 | 30 ms | 0.5% bursty | `slot 5ms 30ms` |
| Evening LTE | 5 | 20 | 50 ms | 1% | `slot 10ms 80ms`, 500 ms buffer (bloat) |
| Bad LTE | 2.5 | 8 | 80 ms | 3% | stress |
| Shared upload | 10 | 40 | 15 ms | 0 | plus a background TCP upload (`iperf3`) as "a family member's backup" |

### 9.3 Where the results are

- Stutter causes: §1.2.
- Voice vs video: §3.3.
- Loss: §4.2.
- Recovery: §5.1.
- Transport: §6.2.
- Fix A/B results: §1.3.

---

## 10. Testing on Windows (for the spike)

**No `tc` on Windows. The options, best first:**
1. **A Linux box as the router** [est.].
   - Run a hotspot or USB-Ethernet bridge from the dev laptop, and shape its interfaces with `tc` (netem plus CAKE)
     using the same `net.sh` profiles.
   - The Windows PC then sees a real SA-like link, with voice (Rust) and video (WebView2) sharing it as at home.
2. **clumsy** (WinDivert) [cited]: lag, drop, throttle, out-of-order, tamper.
   - Good for loss and outages.
   - Bandwidth limiting needs a fork and is experimental. Needs admin.
3. **CDP throttling inside WebView2** (§8.4): only once verified. It only affects the webview, not Rust voice.

**Things only real Windows can show:**
- **Viewer-side stutter (F).** Log `requestVideoFrameCallback` presented frames and gaps next to `framesDecoded`.
- **WebView2 throttling.**
  - Intensive timer throttling doesn't apply to pages with live WebRTC [cited], but ordinary background throttling
    does.
  - Don't drive the stats sampler from `setInterval` in a hidden view.
  - Check that a hidden video webview (`IsVisible = false`) doesn't pause capture or encode.
- **Re-run in the spike, with NVENC/Media Foundation:**
  - the `rtx-time` win;
  - `balanced` vs `maintain-resolution`;
  - send-side BWE;
  - the full-reconnect stall fix.

---

## 11. Sources

### 11.1 Code and specs

**libwebrtc** at commit `421d35bd` (2026-10-14):
https://webrtc.googlesource.com/src/+/421d35bd3de31251f1c36ba22aaee2ce1157e2c8/
- **Congestion control:** `modules/congestion_controller/goog_cc/` (trendline_estimator.cc, delay_based_bwe.cc,
  loss_based_bwe_v2.cc, probe_controller.cc, alr_detector.h, link_capacity_estimator.cc,
  goog_cc_network_control.cc) and `modules/remote_bitrate_estimator/aimd_rate_control.cc`.
- **Pacer:** `modules/pacing/pacing_controller.{h,cc}`, `prioritized_packet_queue.cc`.
- **Audio/video send and DSCP:** `audio/audio_send_stream.cc`, `media/engine/webrtc_voice_engine.cc`,
  `media/engine/webrtc_video_engine.cc` (rtx-time from the negotiated codec → receiver NACK history; DSCP mapping),
  `media/base/media_config.h`.
- **FEC decisions:** `call/rtp_video_sender.cc`, `modules/video_coding/fec_controller_default.cc`,
  `modules/video_coding/media_opt_util.cc`.
- **NACK:** `modules/video_coding/nack_requester.cc`.
- **Receive side:**
  - `video/video_receive_stream2.{h,cc}`: `kMaxWaitForFrame` 3 s, `kMaxWaitForKeyFrame` 200 ms,
    `DetermineMaxWaitForFrame` (3 × rtx-time), `OnDecodableFrameTimeout`;
  - `video/encoder_rtcp_feedback.cc`;
  - `video/video_quality_observer2.cc` (freeze definition).
- **Other:** `api/video_codecs/video_encoder.cc`, `rtc_base/physical_socket_server.cc`,
  `api/transport/goog_cc_factory.cc`.
- **Commits:**
  - Loss-based BWE v2 on by default: https://webrtc.googlesource.com/src/+/e04726281c3110718f5df9d4e8780229a23234e6
  - DSCP on by default: https://webrtc.googlesource.com/src/+/35c5cc8a6f5f256d28629205e397c46a2122dc50

**Chromium (main):** https://chromium.googlesource.com/chromium/src/+/main/
- `net/socket/udp_socket_win.cc` (qWAVE, `QOSSetOutgoingDSCPValue`)
- `services/network/p2p/socket_udp.cc`
- `third_party/blink/renderer/modules/peerconnection/rtc_rtp_sender.cc`

**LiveKit server v1.13.7:** https://github.com/livekit/livekit/tree/v1.13.7
- `pkg/rtc/config.go`: publisher audio is NACK only; video gets TWCC/REMB/FIR/NACK/PLI; subscribers get REMB unless
  send-side BWE is on.
- `pkg/rtc/mediaengine.go`: no video FEC or RED.
- `pkg/config/config.go`: congestion control, PLI throttle and TCP fallback defaults; `use_send_side_bwe`.
- `pkg/sfu/receiver_base.go`: PLI throttle.
- `pkg/sfu/downtrack.go`, `pkg/sfu/streamallocator/streamallocator.go`, `pkg/sfu/bwe/remotebwe/`,
  `pkg/sfu/connectionquality/`.

**livekit-client v2.22.4:** https://github.com/livekit/client-sdk-js/tree/v2.22.4 (also read from the npm package)
- `src/room/track/options.ts`: ScreenSharePresets; `VideoEncoding.priority`.
- `src/room/defaults.ts`: screen-share default h1080fps15; `singlePeerConnection: true`; retry delays.
- `src/room/participant/publishUtils.ts`.
- `LocalParticipant.ts`: audio priority high; re-publish after reconnect.
- `src/room/RTCEngine.ts`: resume/restart; 10 s lost-quality timeout.
- `src/room/DefaultReconnectPolicy.ts`.
- `src/connectionHelper/checks/connectionProtocol.ts`.

**LiveKit Rust SDKs (main, 0.9.5):** https://github.com/livekit/rust-sdks
- `livekit/src/room/options.rs`, `livekit/src/rtc_engine/reconnect_strategy.rs`, `libwebrtc/src/rtp_parameters.rs`,
  `webrtc-sys/CHANGELOG.md`.
- Pulse's `Cargo.lock` pins `livekit` 0.9.3 / `webrtc-sys` 0.3.47, which is libwebrtc m150 per the changelog.

**Other references:**
- LiveKit ports and firewall: https://docs.livekit.io/home/self-hosting/ports-firewall/
- draft-ietf-rmcat-gcc-02: https://datatracker.ietf.org/doc/html/draft-ietf-rmcat-gcc-02
- W3C webrtc-stats: https://w3c.github.io/webrtc-stats/
- W3C webrtc-priority: https://www.w3.org/TR/webrtc-priority
- RFC 8837: https://www.rfc-editor.org/rfc/rfc8837.html
- RFC 8325: https://www.rfc-editor.org/info/rfc8325
- MDN, `jitterBufferTarget` (Chrome 124+): https://developer.mozilla.org/en-US/docs/Web/API/RTCRtpReceiver/jitterBufferTarget
- MDN browser-compat-data (stats support): https://raw.githubusercontent.com/mdn/browser-compat-data/main/api/RTCStatsReport.json
- Chrome timer throttling and the WebRTC exemption: https://developer.chrome.com/blog/timer-throttling-in-chrome-88/
- CDP `Network.emulateNetworkConditionsByRule`: https://chromedevtools.github.io/devtools-protocol/tot/Network/ (read
  from `playwright-core` 1.64 `types/protocol.d.ts`)
- **Microsoft:**
  - `QOSSetFlow`: https://learn.microsoft.com/en-us/windows/win32/api/qos2/nf-qos2-qossetflow
  - `QOS_TRAFFIC_TYPE`: https://learn.microsoft.com/en-us/windows/win32/api/qos2/ne-qos2-qos_traffic_type
  - `IP_TOS`: https://learn.microsoft.com/en-us/windows/win32/winsock/ipproto-ip-socket-options
  - Policy-based QoS: https://technet.microsoft.com/library/dd759093(v=ws.11)
- Cisco, Windows DSCP tagging ("Do not use NLA"): https://www.cisco.com/c/en/us/support/docs/quality-of-service-qos/qos-configuration-monitoring/221868-enable-dscp-qos-tagging-on-windows-machi.html
- clumsy: https://github.com/jagt/clumsy
- Bufferbloat grade bands: https://speedtesthq.com/guides/diagnostics/test-for-bufferbloat
- TURN over TCP (testRTC/Cyara): https://cyara.com/blog/happens-webrtc-shifts-turn-tcp/

### 11.2 South African network data

**Speed and latency measurements**
- Ookla Global Index via FastestFibre (updated 2026-08-26): https://fastestfibre.co.za/benchmarks/
- ICASA State of the ICT Sector 2026, Ookla chart (pub. 2026-05-04): https://theoutlier.co.za/charts/chart/5ed73ca3-980b-4d7c-8638-832800250734
- Ookla open data, Q2 2026 (fixed; mobile at the same path with `type=mobile`): https://ookla-open-data.s3.amazonaws.com/parquet/performance/type=fixed/year=2026/quarter=2/2026-04-01_performance_fixed_tiles.parquet
- MyBroadband Q1 2026 (pub. 2026-04-08): https://mybroadband.co.za/news/broadband/637770-mobile-speeds-higher-than-fixed-broadband-in-south-africa.html
- Opensignal Q1 2026 via Vox (pub. 2026-04-23): https://www.vox.co.za/content-hub/article/vox-fibre-wins-opensignal-awards/
- MyBroadband mobile Q2 2026 via BusinessTech (pub. 2026-07-16): https://businesstech.co.za/news/telecommunications/866475/this-is-the-best-mobile-network-in-south-africa/
- Ookla mobile H1 2025 via Memeburn (pub. 2025-11-05): https://memeburn.com/2025/11/new-ookla-insights-show-mtn-holding-mobile-speed-lead-as-vodacom-claims-5g-advantage/
- Ookla H2 2024 via TechCentral (pub. 2025-02-10): https://techcentral.co.za/mtn-king-of-speed-vodacom-5g-is-faster/258975/
- MyBroadband ISP latency Q1 2025 (pub. 2025-04-29): https://mybroadband.co.za/news/fibre/592472-best-south-african-fibre-isps-for-online-gamers-in-2025.html
- Opensignal, evening dips: https://insights.opensignal.com/2023/05/25/south-african-networks-see-the-worst-4g-signal-strength-but-fastest-4g-download-speeds-at-night

**Packages**
- Openserve: https://fastestfibre.co.za/openserve/ and https://fastestfibre.co.za/rsaweb-openserve-pre-paid/
- Vuma: https://fastestfibre.co.za/insights/vuma-core-vs-vuma-reach/ and https://fastestfibre.co.za/vuma-reach-fibre/
- Frogfoot: https://fastestfibre.co.za/frogfoot-fibre-deals/
- MetroFibre: https://fastestfibre.co.za/network/metrofibre/
- Octotel: https://fastestfibre.co.za/webafrica-octotel-fibre/
- Telkom: https://www.telkom.co.za/about_us/download/FAQ_Telkom_Core_and_Telkom_Endless_Fibre.pdf
- 5G home caps: https://fastestfibre.co.za/insights/best-5g-home-internet-south-africa/

**CGNAT (MyBroadband forum)**
- https://mybroadband.co.za/forum/threads/cgnat-woes.1322591/
- https://mybroadband.co.za/forum/threads/pay-to-exit-cgnat.1177834/
- https://mybroadband.co.za/forum/threads/double-nat-or-cgnat.1104275/

**Bufferbloat (SQM guide, pub. 2026-08-29)**
- https://evezone.evetech.co.za/deep-dives/bufferbloat-south-africa-sqm-fix

**Loadshedding**
- https://furtherafrica.com/2026/10/06/eskom-load-shedding-ends-for-504-straight-days/
- https://www.sanews.gov.za/south-africa/load-shedding-free-winter-cards-sa
- https://itweb.co.za/article/mtn-sa-deploys-20-000-batteries-to-beat-load-shedding-woes/DZQ58vV8epaMzXy2

**Peering**
- PeeringDB: https://www.peeringdb.com/net/1905 and https://www.peeringdb.com/api/netixlan
- Teraco: https://www.teraco.co.za/news/napafrica-and-teraco-acx-growing-africas-digital-community-as-napafrica-hits-6tbps/
  and https://www.teraco.co.za/news/teraco-announces-access-to-oracle-cloud-via-fastconnect/

**ISP routers (pub. 2026-08-22)**
- https://www.fibretiger.co.za/article/isp-free-fibre-routers-compared
