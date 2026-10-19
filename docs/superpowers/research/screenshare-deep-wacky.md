# Screen share: wacky ideas from games, GPUs, cloud gaming and research

Date: 2026-10-14. Research only, nothing here is decided. It builds on `screenshare-summary.md` and the three
reports beside it. Baseline plan from those: `getDisplayMedia` in WebView2, hardware H.264, LiveKit SFU forwarding,
720p60 at 4.5 Mbps as the default game preset.

The brief was "use game rendering tricks if they work and are lighter, however wacky". So this report goes wide,
measures what it can on a Linux box, and then is honest about what survives.

**Labels.** [cited] = from a source in §8 (URL + date read). [measured] = I ran it, method in §2. [est.] = my
estimate from numbers I have. [speculation] = a guess, treat as such.

---

## 0. TL;DR

1. **The best "game trick" is upscaling on the viewer, and it's not wacky at all.** On hard game footage (GTA V,
   Minecraft, Rust), a 720p stream upscaled on the viewer looks as good as a native 1080p stream that uses
   **~10-15% more bits** (lanczos) or **~20-25% more bits** (FSR1) [measured]. On easy footage (a CS:GO clip, the
   Hearthstone card game) native 1080p wins and 720p never catches up [measured]. Bilinear upscaling (the cheap
   default) throws the gain away and then some: +6% to +65% bits [measured]. So what matters is *how* the viewer
   upscales, not only the resolution we send.
2. **AV1 is the biggest single bit-saver we measured.** NVENC AV1 vs NVENC H.264, same settings: +5 to +11 VMAF on
   hard games at 2-4.5 Mbps. Roughly **H.264 needs ~1.6x the bits** to match AV1 at 2 Mbps on GTA V [measured].
   That's bigger than any upscaling trick. With AV1, sending 720p instead of 1080p no longer helps [measured].
   The catch is hardware: AV1 encode needs RTX 40 / RX 7000 / Arc.
3. **Text is the opposite of games.** 1080p at 1 Mbps beats 720p at 2 Mbps with any upscaler by a mile (VMAF 96 vs
   62-80) [measured]. For text, never downscale. Spend frames, not pixels (the "Sharp 1080p30" preset is right).
4. **"Only send changed regions" (RDP-style dirty rects) is already done by the codec.** A static desktop frame
   costs ~80 bytes in H.264. Static screen with a blinking cursor = 39 kbps at 60 fps [measured]. WebRTC's
   zero-hertz screenshare mode also stops sending frames for static content [cited]. Nothing left to win there.
5. **Most of the cool streamer-side tricks are NOT reachable from WebView2.** Intra refresh, NVENC emphasis/ROI
   maps, ultra-low-latency presets, LTR, zero-copy capture into NVENC: all need the native Rust pre-encoded path
   ("Phase B" in the capture report). Chrome's own encoder exposes none of them [cited]. They're v2+ material.
6. **Viewer-side AI upscaling (RTX Video Super Resolution) may work for free.** Chromium switches it on for any
   `<video>` that gets a hardware overlay. It doesn't check whether the video is WebRTC [cited]. But it needs an
   RTX card, the driver toggle, and an overlay that WebView2 may or may not give us. Spike-worthy, zero code if it
   works. Drawing the video into a WebGL/WebGPU canvas (for FSR1) probably turns it off. So the two are either/or.
7. **New gotcha found on the way (not wacky, important):** Chromium **skips NVIDIA's hardware H.264 encoder** when
   the negotiated profile is *Constrained Baseline* (crbug 1088650) [cited]. Then a GeForce owner gets another
   encoder: an Intel iGPU if there is one, otherwise software. The spike must check the negotiated
   `profile-level-id` and `encoderImplementation` on a GeForce desktop without an iGPU.
8. **Not worth it in 2026:** neural codecs (DCVC-RT is ~40 fps at 1080p on a desktop GPU, with no browser path),
   generative compression, game-state streaming, and sending the HUD separately. Viewer-side frame interpolation
   ("capture 30, show 60") fits the friends' wishes, but there's no browser path, so it's a v3 spike at best.

### What the friends actually complained about (added after first draft)

Feedback from the group about Discord: the biggest pain was **the streamer's game FPS tanking while streaming**,
and the PC getting slower over ~30+ minutes. Viewers mostly saw **stutter**, not delay. So the ranking below puts
anything that cuts the **streamer's** GPU/CPU cost first, and treats "send less, rebuild on the viewer" as the
preferred trade. Viewer-side delay is cheaper than we assumed. Viewer stutter and streamer cost are what matter.

Through that lens:

- **Encode is not the expensive part on NVIDIA.** NVENC is a separate chip, and AV1 costs the same as H.264
  [measured, §2.10].
- **What hurts a gaming streamer** is everything around the encoder [est.]:
  - the capture copies (Chromium copies each frame GPU → CPU → GPU [cited]);
  - CPU colour conversion;
  - extra compositor work;
  - and worst of all, **accidentally ending up on a software encoder** (§1.1).

So the cheapest streamer wins are: **use the hardware encoder for sure, capture and encode fewer pixels, capture
fewer frames, and later do the whole thing on the GPU (zero-copy)**. Then let the viewer upscale.

### Ranked shortlist (weighted toward streamer cost)

| # | Idea | Streamer cost | Bits / viewer quality | Verdict |
|---|---|---|---|---|
| 1 | **Make sure the hardware encoder is used.** Fix H.264 profile negotiation (NVIDIA skip on Constrained Baseline) and verify `encoderImplementation` | Avoids 1-2 CPU cores of software H.264 at 1080p60 while gaming (capture report est.). Likely the #1 "FPS tanks" cause if it happens | n/a | **Spike, must-do** |
| 2 | **Send 720p (or 900p) for games, rebuild on the viewer** with lanczos/FSR1 (or RTX VSR) | 2.25x fewer pixels to scale, convert, copy and encode (720p vs 1080p) | Same look at 10-25% fewer bits on hard games [measured]. Worse on clean UI | **v1** |
| 3 | **Cap the capture fps** (30 for non-twitchy games, 5-15 for text), as a steady cadence | Halves capture copies, conversions and encodes (the WGC capture rate can be capped) | A steady 30 fps looks smoother than a ragged 60 [est.]. +6-10 VMAF per frame at the same bits [measured] | **v1** (FPS picker; pick the default per content) |
| 4 | **Zero-copy native pipeline** (WGC texture → GPU downscale/convert → NVENC/AMF/QSV) | Removes ~1 GB/s of GPU↔CPU copies at 1080p60 plus the CPU conversion [est.]. This is how Parsec/Sunshine stay light | Same | **v2** (Phase B). Do it sooner if the spike shows Chromium's capture path costs real game FPS |
| 5 | **RTX Video Super Resolution** on the viewer's `<video>` | 0 | Free AI upscale + deblock on RTX viewers | **Spike** (half a day) |
| 6 | **AV1** when the sharer has AV1 hardware | Same as H.264 on NVENC [measured] | ~35-40% fewer bits on hard games [measured] | **v1 option** (spike: does WebView2 use HW AV1?) |
| 7 | **Intra refresh** (+IDR only on joins) | ~0 | Removes 5x keyframe bursts = less viewer **stutter** [measured] | **v2** (native encoder only) |
| 8 | Viewer-side frame interpolation ("capture 30, show 60") | Halves streamer work | Smoother feel, +33 ms delay (acceptable per feedback). No ready browser path; DIY WebGPU optical flow | **Upgraded to spike-worthy for v3**: only if friends still miss 60 fps after #3 |
| 9 | ROI/emphasis map | ~0 | Moves quality to the centre | **v2 experiment** |
| 10 | L4S, neural codecs, game-state, separate HUD, dirty rects | n/a | n/a | **Not yet / not worth it** |

The "PC gets slower over 30+ minutes" symptom smells like a leak or thermal throttling, not codec choice [est.]. For
example, the Rust SDK's per-publish retention (#1277, from the summary). The spike should add a **45-minute soak
test while gaming**, logging game FPS, GPU/CPU %, memory of every Pulse process, and GPU clocks/temperature.

---

## 1. The big constraint: what can we actually touch?

The baseline publishes and views through Chromium inside WebView2. That decides which tricks are even possible.

| Knob | From JS in WebView2? | From native Rust (pre-encoded path) |
|---|---|---|
| Resolution, fps, bitrate, simulcast layers, `scaleResolutionDownBy` | Yes | Yes |
| Codec choice (H.264 / VP9 / AV1) | Yes (via codec preferences) | Yes |
| `contentHint` (`motion` / `detail` / `text`), `degradationPreference` | Yes | Not exposed in the Rust SDK yet (capture report) |
| Per-frame QP | Only with WebCodecs `bitrateMode: "quantizer"` (Chrome 117+) [cited], not inside WebRTC | Yes |
| Per-block QP (ROI / emphasis map) | **No** | Yes: NVENC `qpDeltaMap` [cited] |
| Intra refresh, LTR, reference invalidation | **No**. Chrome's MF encoder sets GOP, rate control, low-latency mode, per-frame QP and temporal layers, and nothing else [cited] | Yes [cited] |
| Zero-copy capture into the encoder | **No**. libwebrtc's WGC capturer copies to a CPU staging texture unless a Windows 11 24H2+ texture path is on [cited] | Yes |
| Viewer shader post-processing | Yes: WebGL2 / WebGPU on the decoded `<video>` | Only via the Linux fallback path |
| Viewer driver AI upscaling (RTX VSR) | Maybe: needs `<video>` on a DirectComposition overlay [cited] | Via the RTX Video SDK (DX11/DX12/Vulkan/CUDA, RTX 20+) [cited] |

**So:** everything on the viewer side is reachable in v1. Most clever streamer-side encoder tricks need the native
pipeline that the capture report calls Phase B.

### 1.1 A real gotcha found on the way: NVIDIA + Constrained Baseline

[cited] Chromium's Media Foundation encoder has this in `ActivateAsyncEncoder`:

```
// Skip NVIDIA GPU due to https://crbug.com/1088650 for constrained
// baseline profile H.264 encoding, and go to the next instance according
// to merit value.
if (codec_ == VideoCodec::kH264 && is_constrained_h264 &&
    vendor == DriverVendor::kNvidia) { continue; }
```

`is_constrained_h264` is true when the negotiated SDP `profile-level-id` is Constrained Baseline (42e0xx), per
`rtc_video_encoder_factory.cc`. LiveKit's server registers 42001f (Baseline) and 4d001f (Main) with strict fmtp,
plus whatever is in its default codec list. Which profile wins depends on the offer order [cited].

What it means [est.]: if a GeForce sharer ends up on 42e01f, Chromium skips NVENC. On a laptop it may land on the
Intel iGPU's encoder (still hardware, but a cross-adapter copy). On a desktop with no iGPU it likely falls back to
software. The other reports already tell the spike to check `encoderImplementation`. Add: **log the negotiated
`profile-level-id`, and test on a GeForce desktop with no iGPU.** If it bites, prefer 42001f/4d001f/640c1f in the
publisher's codec preferences.

---

## 2. The lab: what I measured

**Rig.** Linux, i9 (32 threads), RTX 4070 Laptop GPU (driver 615.71), FFmpeg 9.0.2 with `h264_nvenc`, `av1_nvenc`,
`libvmaf`, `libplacebo`. The encoder is NVENC, the same silicon friends' GeForce cards have. But Chrome drives it
through Media Foundation with its own settings, so absolute numbers will differ in WebView2. The relative findings
should hold [est.].

**Clips.** Xiph's public-domain Twitch game recordings, 1080p60, first 6 s (360 frames) of each: CS:GO, GTA V,
Minecraft, Rust, Hearthstone [cited]. The sources are ~33-56 Mbps H.264, so the "reference" is already slightly
compressed. Fine for comparisons. Plus a synthetic 1080p "code editor" clip (FFmpeg `drawtext`): 3 s static, 2 s
fast scroll, 5 s static with a blinking cursor, and some coloured text.

**Encoder settings (WebRTC-like).** `-preset p4 -tune ll -rc cbr`, VBV ≈ 0.5 s, no B-frames, infinite GOP. Actual
bitrate averaged 3% over target (max 6%) [measured].

**Metric.** VMAF v0.6.1 at 1080p against the 60 fps source. I also report **VMAF NEG** ("no enhancement gain"),
which doesn't reward sharpening. That matters because FSR1's RCAS pass is a sharpener and can fool plain VMAF.
Every 2nd frame is scored.

**Upscalers.** `bilinear` (FFmpeg swscale, like a cheap GPU texture sample), `lanczos` (swscale), `fsr1` (AMD
FidelityFX FSR 1.0.2 EASU+RCAS, the mpv GLSL port run through libplacebo on the GPU; it upscales luma, chroma is
bilinear) [cited], `bilcas` (bilinear + AMD CAS sharpen).

Scripts and raw JSON are in `/tmp/claude-1000/screenshare-wacky/` (not in the repo; reproducible from §2.9).

### 2.1 Result A: send lower resolution, upscale on the viewer [measured]

VMAF (plain / NEG) at 1080p display, H.264 NVENC, 60 fps. A **bold** cell beats native 1080p at the same bitrate
on both metrics.

| Clip | Mbps | 1080p native | 900p+lanczos | 900p+FSR1 | 720p+bilinear | 720p+lanczos | 720p+FSR1 | 540p+FSR1 |
|---|---|---|---|---|---|---|---|---|
| CS:GO | 2.0 | 94.0 / 92.9 | 91.8 / 89.8 | 93.7 / 88.6 | 78.9 / 78.3 | 90.4 / 88.1 | 91.8 / 87.2 | 85.7 / 81.5 |
| CS:GO | 4.5 | 96.2 / 95.5 | 94.0 / 92.1 | 95.9 / 90.7 | 80.2 / 79.7 | 92.0 / 89.6 | 93.3 / 88.6 | 87.1 / 83.0 |
| GTA V | 2.0 | 57.1 / 55.5 | 57.7 / 56.2 | **60.4 / 58.2** | 55.0 / 54.0 | **61.6 / 59.8** | **64.5 / 62.0** | **63.3 / 61.0** |
| GTA V | 4.5 | 74.0 / 72.1 | **74.9 / 73.1** | **77.8 / 74.6** | 68.0 / 67.0 | **76.2 / 74.1** | **79.0 / 75.7** | **75.7 / 72.6** |
| GTA V | 7.0 | 82.1 / 80.3 | **82.6 / 80.6** | **85.5 / 81.9** | 74.1 / 73.1 | **83.0 / 80.8** | **85.8 / 82.0** | 81.8 / 78.4 |
| Minecraft | 2.0 | 48.5 / 47.2 | **51.4 / 49.9** | **53.7 / 51.9** | 49.3 / 48.4 | **55.0 / 53.4** | **57.5 / 55.4** | **58.1 / 55.9** |
| Minecraft | 4.5 | 70.0 / 68.0 | **73.0 / 70.9** | **75.6 / 72.6** | 66.9 / 65.7 | **74.3 / 72.1** | **77.0 / 73.8** | **74.8 / 71.8** |
| Minecraft | 7.0 | 82.3 / 80.2 | **83.4 / 81.1** | **86.0 / 82.5** | 75.1 / 74.0 | **83.6 / 81.2** | **86.3 / 82.7** | 82.7 / 79.2 |
| Rust | 2.0 | 72.0 / 70.3 | **74.7 / 72.7** | **76.8 / 73.8** | 68.5 / 67.6 | **76.6 / 74.3** | **79.0 / 75.8** | **77.1 / 73.9** |
| Rust | 4.5 | 85.3 / 83.3 | **86.3 / 84.1** | **88.6 / 85.0** | 77.3 / 76.3 | **86.5 / 83.9** | **89.0 / 85.1** | 84.7 / 80.8 |
| Hearthstone | 2.0 | 88.7 / 87.2 | 88.5 / 86.7 | 91.9 / 85.5 | 77.7 / 77.1 | 87.9 / 85.9 | 91.2 / 84.9 | 85.6 / 78.8 |
| Hearthstone | 4.5 | 94.4 / 93.2 | 92.9 / 91.3 | 96.1 / 88.8 | 80.7 / 80.1 | 91.4 / 89.5 | 94.5 / 87.4 | 87.8 / 80.5 |

(The full 4-bitrate × 4-resolution × 4-upscaler grid, including 3 Mbps and bilinear+CAS, is in the scratch folder.
Bilinear+CAS was always worse than lanczos, so I dropped it.)

**Equal-quality bitrate change vs native 1080p** (log-interpolated, averaged over the 1080p points each option can
reach; negative = fewer bits for the same VMAF / VMAF NEG) [measured]:

| Option | CS:GO | GTA V | Minecraft | Rust | Hearthstone |
|---|---|---|---|---|---|
| 720p + bilinear | never reaches | +32% / +25% | +10% / +6% | +65% / +49% | never reaches |
| 720p + lanczos | never | **-12% / -10%** | **-15% / -14%** | **-16% / -12%** | +15% / +27% |
| 720p + FSR1 | never | **-26% / -19%** | **-24% / -21%** | **-23% / -22%** | -10% / +104% |
| 900p + FSR1 | +20% / never | -18% / -12% | -18% / -15% | -21% / -15% | -34% / +38% |

What this says:

- **Hard, busy 3D games** (foliage, particles, camera motion) at 2-7 Mbps: the encoder is starved at 1080p. Sending
  720p and upscaling well is a real win: about 10-15% fewer bits with lanczos, ~20% with FSR1, and NEG agrees.
  Our default "720p60" preset is right for these, *as long as the viewer doesn't use bilinear*.
- **Clean or slow content** (this CS:GO clip, card games, UI-heavy games): native 1080p wins at every bitrate.
  Downscaling loses detail the encoder could have kept.
- **FSR1 vs lanczos:** on plain VMAF FSR1 always looks better. On NEG the gap shrinks to ~1-2 points on hard games
  and *reverses* on CS:GO and Hearthstone. In other words, part of FSR1's "win" is sharpening, and on clean UI it
  over-sharpens. Honest read: FSR1 ≈ lanczos + a bit of sharpening. Worth it for games, not for UI or text.
- **Bilinear is the trap.** If the viewer stretches 720p with a plain bilinear sample (a WebGL `LINEAR` texture or a
  cheap compositor path), 720p looks worse than 1080p on every clip.

What scaler WebView2 uses for an upscaled `<video>` today is unknown [est.]. If the video gets a DirectComposition
overlay, Chromium scales it with the D3D11 video processor while on AC power [cited]. That's a driver-quality
scaler, usually better than bilinear [est.]. If it's composited by Skia/GL instead, it's probably bilinear [est.].
Either way: **measure it in the spike, or take control by drawing into a canvas with our own shader.**

### 2.2 Result B: text, 1080p vs 720p upscaled [measured]

Synthetic code-editor clip, 30 fps, H.264 NVENC CBR:

| Stream | VMAF | PSNR-Y | SSIM |
|---|---|---|---|
| 1080p @ **1 Mbps** | **96.0** | 48.8 dB | 0.9998 |
| 1080p @ 2 Mbps | 97.3 | 55.1 dB | 1.0000 |
| 720p @ 2 Mbps + bilinear | 62.0 | 29.8 dB | 0.9971 |
| 720p @ 2 Mbps + lanczos | 76.9 | 31.2 dB | 0.9983 |
| 720p @ 2 Mbps + FSR1 | 79.6 | 31.4 dB | 0.9981 |

FSR1 makes 720p text visibly crisper than bilinear (I compared crops), but it can't put back strokes thinner than a
pixel. **Text: always native resolution.** Text is cheap anyway (next result).

### 2.3 Result C: what a static or scrolling desktop costs (the dirty-rect question) [measured]

Same clip, 1080p60, H.264 NVENC VBR capped at 4 Mbps, infinite GOP:

| Phase | Avg bytes/frame | Bitrate |
|---|---|---|
| First frame (IDR) | 35,305 | n/a |
| Static, right after the IDR (encoder still refining) | 693 | 333 kbps |
| Fast scrolling (400 px/s) | 2,581 (max 46 KB) | 1.24 Mbps |
| Static with a blinking cursor | **81** | **39 kbps** |

H.264 skip blocks already *are* dirty rects: unchanged 16×16 blocks cost almost nothing. On top of that, WebRTC's
zero-hertz screenshare mode stops sending frames when the content is static. It's default-on, and has a cheap WGC
change detector [cited]. An RDP-style "send changed rectangles" protocol would save at most ~40 kbps here. **Not
worth building.** The real text problems are resolution and chroma (next result), not bits.

### 2.4 Result D: chroma 4:2:0 vs 4:4:4 for coloured text [measured]

x264 at 1.5 Mbps, 1080p30. On a crop of red/orange text: V-plane PSNR 39.8 dB (4:2:0) vs 43.1 dB (4:4:4).
Visually the red text is a little softer and darker in 4:2:0, but readable. Small win, and WebRTC in browsers only
does 4:2:0 [est.]. **Not worth it.**

### 2.5 Result E: IDR keyframes vs intra refresh [measured]

CS:GO, 720p60, 4.5 Mbps, NVENC CBR. A forced keyframe every 2 s (like a viewer joining) vs intra refresh with a
2 s period:

| Variant | Frame at the forced refresh | The frames right after it | VMAF near the refresh |
|---|---|---|---|
| IDR, 0.5 s VBV | **45.7 KB (≈5x average)** | 586-695 B (starved) | 92.3 |
| Intra refresh, 0.5 s VBV | 9.4 KB (flat) | 9.4 KB (flat) | 92.8 |
| IDR, 1-frame VBV | 20.3 KB (squeezed) | 660 B, then recovering | **89.2** |
| Intra refresh, 1-frame VBV | 9.4 KB (flat) | 9.4 KB (flat) | 92.8 |

(With CBR filler, NVENC makes every frame the same size, so "flat" is literal.)

Intra refresh removes the keyframe burst entirely and doesn't lose quality. A 45 KB burst at 4.5 Mbps takes about
80 ms to send. That's a visible hitch, and on LTE it's the frame most likely to lose packets [est.].

**But, for us:**

- (a) Chrome's encoder doesn't expose intra refresh [cited], so it's native-path only.
- (b) **The SFU waits for a keyframe before it starts forwarding to a new viewer** (server-quality report). A pure
  intra-refresh stream has no keyframes, so new viewers would wait forever. The workable design is a hybrid: real
  IDR on PLI (joins), intra refresh to recover from loss.
- (c) WebRTC already sends keyframes only on PLI, not every 2 s. So the gain is limited to joins and loss recovery.

Verdict: **v2**, together with the native encoder.

### 2.6 Result F: ROI / "foveated" encoding [measured]

x264 with FFmpeg's `addroi`: centre 40%×50% of the frame, 720p, 3 Mbps, scored at 1080p:

| Clip | Variant | Full frame | Centre | Bottom edge (HUD band) |
|---|---|---|---|---|
| CS:GO | baseline | 89.8 | 83.4 | 89.9 |
| CS:GO | ROI, mild (qoffset −0.15) | 89.0 | **86.4** | 88.4 |
| CS:GO | ROI, strong (−0.5) | 83.9 | 87.7 | 80.3 |
| Rust | baseline | 74.2 | 60.8 | 74.9 |
| Rust | ROI, mild | 68.8 | **74.9** | 67.3 |
| Rust | ROI, strong | 37.6 | 79.7 | 33.7 |

ROI *moves* quality, it doesn't create it. Mild ROI buys +3 to +14 VMAF in the centre for −1 to −8 at the edges.
Strong ROI wrecks the edges, which is often exactly where the HUD and minimap live. Whether that *feels* better
depends on where viewers look. Gamers do mostly look at the crosshair, but spectators also read the HUD
[speculation]. NVENC has the exact mechanism (the emphasis map, `qpDeltaMap`, per macroblock, not combinable with
spatial/temporal AQ), and OBS ships an NVENC ROI implementation [cited]. Fun variant: put the ROI where the
**streamer's mouse cursor** is, which follows attention in strategy games and desktops [speculation].

Verdict: **v2 experiment** on the native path. Default off. If tried, use mild offsets and keep the HUD band
protected.

### 2.7 Result G: AV1 vs H.264 on the same NVENC [measured]

| Clip | Mbps | H.264 1080p | **AV1 1080p** | H.264 720p+lanczos | AV1 720p+lanczos |
|---|---|---|---|---|---|
| CS:GO | 2.0 | 94.0 | **95.6** | 90.4 | 91.3 |
| CS:GO | 4.5 | 96.2 | **97.0** | 92.0 | 92.7 |
| GTA V | 2.0 | 57.1 | **67.4** | 61.6 | 66.6 |
| GTA V | 4.5 | 74.0 | **79.5** | 76.2 | 78.8 |
| Minecraft | 2.0 | 48.5 | **59.9** | 55.0 | 60.3 |
| Minecraft | 4.5 | 70.0 | **76.3** | 74.3 | 76.8 |

AV1 at 2 Mbps on GTA V (67.4) matches H.264 1080p at about 3.2 Mbps [est., interpolated], which is ~35-40% fewer
bits. With AV1 the "send 720p" trick stops paying, because AV1 copes with 1080p detail much better.

Practical limits:

- AV1 hardware **encode** needs RTX 40 / RX 7000 / Intel Arc. Hardware decode covers more cards. dav1d software
  decode is fine on modern CPUs (capture report).
- Chromium's MF encoder handles AV1 [cited]. Whether WebView2 actually picks hardware AV1 for WebRTC on a friend's
  RTX 40 needs the spike.
- Chrome's software AV1 (libaom) is real-time capable for calls [cited], but not for a sharer at 1080p60 while
  gaming [est.].

### 2.8 Result H: send 30 fps, show 60 (frame interpolation) [measured, but the metric fails]

720p, 3 Mbps, scored against the 60 fps source:

| Clip | 60 fps encode | 30 fps encode, frames doubled | 30 fps + motion-compensated interpolation (FFmpeg `minterpolate`) |
|---|---|---|---|
| CS:GO | 91.2 | 92.2 | 92.0 |
| GTA V | 69.0 | 75.6 | 75.5 |
| Minecraft | 64.6 | 74.9 | 74.8 |

Per-frame quality goes up a lot at 30 fps (twice the bits per frame). But VMAF barely penalises repeated frames, so
it **can't measure smoothness** and can't show any value from interpolation. Classic block-based interpolation
added nothing measurable. What we know without a metric [est.]:

- Interpolating needs the *next* frame. At 30 fps that's +33 ms of latency, plus compute.
- Fast game motion (camera whips, HUD, particles) is the worst case for optical flow.

Verdict: offering 30 fps is already in the plan (FPS picker), and given the streamer-cost feedback it should be the
*default* for non-twitchy games. Interpolating back to 60 on the viewer has no ready browser path; see §3.1.5.

### 2.10 Result I: what the streamer pays per frame [measured, rough]

300 frames of 1080p BGRA (GTA V) from RAM. The GPU path uploads them, scales on the GPU (CUDA), and NVENC encodes.
The CPU path does a single-threaded BGRA→NV12 conversion (+ scale) in swscale, which is roughly what Chromium's
capture path does on the CPU [est.].

| Path | Throughput | Per-frame wall time | Share of a 60 fps budget (16.7 ms) |
|---|---|---|---|
| GPU: upload + scale + H.264 NVENC, 1080p | 387 fps | 2.6 ms | ~16% of the pipeline's real-time capacity |
| GPU: upload + scale + H.264 NVENC, 720p | 521 fps | 1.9 ms | ~12% |
| GPU: upload + scale + **AV1** NVENC, 1080p | 440 fps | 2.3 ms | ~14% |
| GPU: upload + scale + **AV1** NVENC, 720p | 542 fps | 1.8 ms | ~11% |
| CPU: BGRA→NV12, 1 thread, 1080p | 357 fps | 2.8 ms | **~17% of one CPU core at 60 fps** |
| CPU: BGRA→NV12 + downscale, 1 thread, 720p | 409 fps | 2.4 ms | ~15% of one core |

Notes:

- These times include the host→GPU upload, so they overstate NVENC itself. On a real game machine the extra cost
  is **memory/PCIe traffic and contention with the game**, not NVENC compute [est.].
- AV1 on Ada NVENC costs about the same as H.264 here, so AV1's bit savings come free for the streamer.
- Going from 1080p to 720p trims the pipeline ~25-30% here [measured]. The payoff is bigger wherever the CPU
  touches pixels (copies scale with pixel count: 2.25x fewer at 720p) [est.].
- Halving fps halves all of it.

Laptop RTX 4070 + i9, Linux. The friends' Windows machines and Chromium's exact path will differ. This is only for
ranking ideas. The spike must measure game-FPS loss on Windows directly.

### 2.9 Reproducing

```
# encode (WebRTC-like NVENC):   ffmpeg -t 6 -i SRC -vf fps=60,scale=1280:720:flags=bicubic \
#   -c:v h264_nvenc -preset p4 -tune ll -rc cbr -b:v 4500k -maxrate 4500k -bufsize 2250k -bf 0 -g 100000 out.mp4
# FSR1 upscale + score:  ffmpeg -i out.mp4 -t 6 -i SRC -lavfi "[0:v]libplacebo=w=1920:h=1080:upscaler=bilinear:\
#   custom_shader_path=FSR.glsl,format=yuv420p[d];[1:v]fps=60,format=yuv420p[r];[d][r]libvmaf=model=\
#   'version=vmaf_v0.6.1\:name=vmaf|version=vmaf_v0.6.1neg\:name=neg':n_subsample=2" -f null -
```

**Caveats.** 6 s clips, 5 games, one encoder on Linux, and VMAF is a model trained on TV-style content, not games
or text. Treat differences under ~2 VMAF as noise. Nobody watched these with human eyes in a blind test.

---

## 3. The idea catalogue

For each idea: what it saves, how it fits, effort, maturity, risk, verdict.

### 3.1 Viewer-side reconstruction

#### 3.1.1 FSR1 (EASU + RCAS) or lanczos as a WebGL2/WebGPU shader on the viewer

- **Saves:** 10-25% bits on hard games at the same look, or the same bits look better (§2.1). No streamer or server
  cost.
- **Fits:** the viewer draws the `<video>` into a canvas each frame:
  - WebGL2: `texImage2D(video)`.
  - WebGPU: `importExternalTexture`, which can sample the decoded frame "potentially in a 0-copy way" [cited].
  - The FSR1 source is MIT-licensed HLSL/GLSL. A WGSL port exists (Three.js "Shade") [cited], and the mpv GLSL port
    is ~400 lines [cited].
  - Note: AMD says FSR1 input should be anti-aliased, perceptual-space, **noise-free**, and applied before
    grain/UI [cited]. Compressed video isn't noise-free, and RCAS sharpens blocking too. That fits the NEG result:
    use RCAS gently (or lanczos with no sharpening) and tune by eye.
- **Cost:** EASU is about a dozen taps per output pixel on luma. Well under 1 ms at 1080p on any discrete GPU
  [est.]. Not measured in a browser. On a weak iGPU at 1440p, check.
- **Effort:** 1-3 days, including a fallback to the plain `<video>`, fullscreen and resize handling.
- **Risk:**
  - We lose the free `<video>` overlay path (and with it RTX VSR, §3.1.2).
  - Adds a requestVideoFrameCallback/render loop with its own frame pacing.
  - Over-sharpens UI and text. Turn it off for "Sharp" text shares and when the stream is already ≥ the display
    size.
- **Verdict: v1.** Upscale only when the stream is smaller than the tile. Lanczos for text/UI shares, FSR1 (gentle
  RCAS) for game shares.

#### 3.1.2 RTX Video Super Resolution (NVIDIA) and friends

- **What it is:** a driver-level neural upscaler and artifact remover for browser video, on RTX 30/40 (later RTX 20
  through the SDK). The user enables it in the NVIDIA control panel / app [cited]. It "reduces or eliminates
  artifacts caused by compressing video", like blockiness and ringing [cited]. That's exactly our failure mode at
  low bitrates.
- **Does it work in WebView2 on a WebRTC `<video>`?** Not proven, but likely enough to test:
  - Chromium enables it in `SwapChainPresenter` via `VideoProcessorSetStreamExtension` with NVIDIA's GUID whenever
    a video goes through the video-processor overlay path.
  - It is off on battery power.
  - The code has **no check** for WebRTC/MediaStream, frame rate, or source [cited].
  - Intel's equivalent is behind `kIntelVpSuperResolution`, **disabled by default** [cited].
  - User reports show it "Inactive" in some Chromium browsers (Vivaldi) and active in Edge [cited]. So whether
    WebView2's composition path produces an overlay is the open question.
- **Other evidence:**
  - Moonlight (a game-streaming client) built a VSR PR (NVIDIA/Intel via D3D11, MetalFX on Mac). It reported
    **<1 ms** added on Windows dGPUs, with trouble in HDR and windowed scaling. The PR was closed in favour of a
    newer one, still open [cited].
  - NVIDIA's Maxine VFX SDK lists 720p→1080p video super resolution at 1.4 ms on an RTX 4090 and 13 ms on an
    RTX 2060 [cited]. The browser version uses a lighter path [est.].
- **Edge's own VSR** (Microsoft, for sub-720p video, RTX 20+/RX 5700+) was a 2023 Canary experiment. Its 2026
  status is unknown to me [cited, dated]. I couldn't re-check: my web search budget ran out.
- **AMD:** no driver-level browser video upscaler that I could find. AMD's frame-gen (AFMF) is for games, not video
  [cited].
- **Saves:** quality for free on RTX viewers. Combined with sending 720p it could be a big win [speculation].
- **Effort:** zero code if it works. Half a day in the spike: play a 720p WebRTC stream in WebView2 fullscreen at
  1080p/1440p, with NVIDIA's VSR on, and look at the driver's "VSR active" indicator.
- **Verdict: spike.**
  - If it works, keep a "Use GPU AI upscaling (RTX)" mode that draws the plain `<video>` (no canvas), and use the
    FSR1 canvas for everyone else.
  - **Native alternative:** the RTX Video SDK (DX11/DX12/Vulkan/CUDA, RTX 20+) [cited]. That's only for a native
    renderer, so v3 at best.

#### 3.1.3 Temporal upscalers: FSR2/3/4, DLSS, XeSS

They need the game's motion vectors, depth and jitter. A captured video has none of those [cited: FSR1 doc
contrasts; general knowledge]. Wacky idea: use the H.264 decoder's motion vectors as "motion vectors". WebCodecs
and `<video>` don't expose them, and they are per-block and noisy [est.]. **Not worth it.**

#### 3.1.4 ML upscalers in WebGPU (Anime4K-style)

Anime4K-WebGPU runs CNN upscalers on video in the browser. Its authors say heavier pipelines struggle at 1080p on
some machines [cited]. It's tuned for line-art anime, not games [est.]. **Not worth it** over FSR1/VSR. Fun demo
material.

#### 3.1.5 Frame interpolation ("send 30, show 60")

- **Options:**
  - NVIDIA Optical Flow SDK FRUC: hardware optical-flow engine, Turing+, D3D/CUDA, native only [cited].
  - AMD AFMF: driver frame generation for DX11/12 games. It works on the game's swapchain, not on video in a
    browser [cited].
  - WebGPU optical flow: you'd write it yourself.
- **Costs:** +1 frame of latency (+33 ms at 30 fps), artifacts on fast game motion and HUDs [est.]. No metric gain
  measurable with VMAF (§2.8).
- **Saves:** in theory up to ~half the bits for "60 fps feel" [speculation]. In practice, 30 fps at the same
  bitrate already gives much better per-frame quality (§2.8).
- **Verdict (revised after the streamer-cost feedback): spike-worthy for v3, not v1/v2.**
  - It is the purest form of "send less, rebuild on the viewer", and the friends tolerate delay better than
    stutter.
  - But there is no off-the-shelf browser path. RTX VSR doesn't interpolate, and AFMF only works on games.
  - A DIY WebGPU optical-flow + warp shader is weeks of work, with visible artifacts on HUDs and camera whips
    [est.].
  - Do the cheap streamer savings (#1-#3 in the shortlist) first. Revisit only if the friends miss 60 fps.

### 3.2 Streamer-side GPU work

#### 3.2.1 Zero-copy capture → encode (WGC texture → NVENC/AMF/QSV)

- **Saves:** streamer CPU and memory bandwidth.
  - Chromium's path: WGC → CPU staging copy (`D3D11_USAGE_STAGING` + `Map` + `memcpy`) → shared memory → back to
    the GPU for MF encode [cited]. At 1080p60 BGRA that's ~0.5 GB/s each way [est.].
  - A zero-copy texture path exists for Windows 11 24H2+ behind an option [cited].
  - Parsec and Sunshine do zero-copy GPU pipelines [cited].
- **Fits:** native Rust pre-encoded path only (capture report Phase B). NVENC accepts registered D3D11 textures
  directly (`NvEncRegisterResource`) [cited].
- **Verdict: v2, possibly pulled forward.** It's the Phase B pipeline anyway, and the friends' #1 complaint (game
  FPS tanking) is exactly what it fixes. Measure the game-FPS loss of Chromium's path in the spike. If a sharer
  loses more than ~5% game FPS at 720p60 with hardware encode confirmed, prioritise Phase B over other v2 work
  [est. threshold].

#### 3.2.2 Downscale on the GPU before encode

With `getDisplayMedia` + `applyConstraints({width,height})` or `scaleResolutionDownBy`, Chromium scales somewhere
in its pipeline. Where (CPU libyuv vs GPU video processor) I couldn't confirm [est.]. In the native path, do it in
the same compute shader as BGRA→NV12 [est.: trivial]. **v2 detail.**

#### 3.2.3 ROI / emphasis maps / "VRS for encoding"

See §2.6. Variable Rate Shading's idea ("spend less where nobody looks") maps directly to per-block QP. **v2
experiment**, native only.

#### 3.2.4 Intra refresh instead of keyframes

See §2.5. **v2**, native only, hybrid with IDR-on-PLI.

#### 3.2.5 NVENC low-latency settings

NVIDIA's guidance for game streaming: "ultra-low latency" tuning with CBR on bandwidth-limited links, infinite GOP,
no B-frames, plus LTR and reference-frame invalidation for loss recovery [cited]. Sunshine adds a "single-frame
VBV" default and quarter-resolution two-pass [cited].

- Chrome sets `CODECAPI_AVLowLatencyMode` and CBR for us [cited]. So **in v1 there's nothing to do.**
- In the native path, use ULL + CBR + small VBV + intra refresh.
- My test (§2.5): a 1-frame VBV squeezes keyframes and costs ~3 VMAF right after them. It's fine with intra
  refresh.

**v2.**

#### 3.2.6 Split-frame / slice encoding for sub-frame latency

NVENC split-frame encoding is HEVC/AV1 only, for ≥2112-line frames, and "degrades quality" [cited]. Slices with
sub-frame readback can shave a few ms. At our latency budget (80-200 ms glass to glass, mostly network and jitter
buffer) that's noise [est.]. **Not worth it.**

### 3.3 Content-aware tricks

- **Dirty rects / changed-region streaming:** already free (§2.3). **Not worth it.**
- **Switch presets by content:** this is the one that matters:
  - Games: 720p (or 900p) + 60 fps + good viewer upscale.
  - Text/desktop: native res + 5-30 fps, no upscale shader, `contentHint: "text"/"detail"`.

  Auto-detect via the captured window (fullscreen game vs editor/browser) as the server-quality report suggests.
  Cheap extra signal: zero-hertz behaviour (static frames → text-like) [est.]. **v1.**
- **Codec by content:** AV1 has screen-content tools (palette, intra block copy). They only matter for text, and
  only with a content hint the encoder honours [cited in capture report]. AV1 already wins on games (§2.7).
  **v1 option where AV1 hardware exists, else v2.**
- **Lossless text tiles / 4:4:4:** tiny gain (§2.4), no browser support. **Not worth it.**
- **Sending the HUD separately:** captured frames have the HUD baked in. There's no API to split it, short of
  injecting into the game (anti-cheat risk). **Not worth it.**

### 3.4 Cloud-gaming techniques

| Technique | What it is | Fit for Pulse | Verdict |
|---|---|---|---|
| Hardware-only, zero-copy pipelines (Parsec: "<10 ms encode+decode", 4-8 ms total at 240 fps on LAN) [cited] | Everything on GPU, no wrappers | Phase B | v2 |
| Frame pacing on the viewer | Present frames on a steady cadence, not as they arrive | Chromium's WebRTC renderer already smooths with its jitter buffer [est.]. A canvas render loop (§3.1.1) must use `requestVideoFrameCallback` and not add its own queue | v1 (as a "don't break it" rule) |
| L4S + SCReAMv2 | ECN-based low-queue congestion control. Comcast + GeForce NOW claim up to 78% lower working latency [cited] | Chromium's SCReAMv2 is experimental, Canary-only, with partial L4S [cited]. Our ISPs and Oracle's network don't mark L4S [est.] | Not yet. Revisit in 2027 |
| LTR + reference invalidation | Recover from loss without a keyframe [cited] | Native only. The SFU forwards NACK/PLI. Would need RPSI-style feedback through LiveKit [est.] | Not worth it |
| Slice/sub-frame encode | See §3.2.6 | n/a | Not worth it |
| Lower fps under congestion, keep resolution (text) / the reverse (games) | `degradationPreference` | Already in the plan | v1 |

### 3.5 Neural and "generative" codecs

- **DCVC-RT** (Microsoft, CVPR 2025): the most practical learned codec.
  - **~40 fps encode / 34 fps decode at 1080p on an RTX 2080 Ti**, 125/113 fps on an A100.
  - Claims −21% bitrate vs H.266/VTM.
  - Its review notes it has no VMAF/subjective results [cited].
- For us it would need a custom CUDA/PyTorch pipeline on both ends, a custom RTP payload, and every viewer's GPU
  busy decoding while they game. There's no browser path: WebNN is in origin trial (Chrome 145-147ish), not shipped
  [cited]. Decoding 6 friends × 60 fps in WebGPU shaders is a research project [est.].
- **NVIDIA Maxine:** its codec-like features target talking heads (face reenactment), not games. Its VSR is the
  same idea as RTX VSR (§3.1.2) [cited for VFX perf; scope is general knowledge].
- **Generative compression** ("send a prompt + sketch, hallucinate the frame"): fun, but it invents details. That's
  a deal-breaker when friends are watching your actual game [speculation].
- **Quantified verdict: not worth it in 2026.** Best case ~20-30% bits over AV1 at a ~10x compute cost, no hardware
  support, no browser support [est.]. AV1 hardware gets ~35% over H.264 today for free (§2.7).

### 3.6 Game-state streaming instead of pixels

Some games already do this. CS2/Dota 2 broadcast (GOTV/SourceTV-style) and replay files ship game state, and the
viewer's own game client renders it [general knowledge, not cited]. Bandwidth is tiny. But:

- the viewer needs the game installed and the same version;
- only games with spectator/replay support work;
- it doesn't work for single-player, emulators or anything else.

**Not worth it** as a Pulse feature. At most, a "join their GOTV" deep link someday [speculation].

### 3.7 Anything else

- **AV1 film-grain synthesis:** denoise, send grain parameters, the decoder (dav1d) regenerates grain. Up to ~50%
  savings on heavy-grain film [cited]. Games mostly have no grain, or post-process grain the player can switch off.
  Real-time encoders don't do this by default [cited]. **Not worth it.**
- **Deband/deblock shader on the viewer:** a cheap WebGL debanding pass on dark gradients (sky, fog), where
  low-bitrate H.264 bands. It would sit in the same canvas pass as FSR1 [est.]. **v2 nicety.**
- **HDR → SDR tonemap on GPU:** already covered by the capture report (needed for HDR desktops). Not a bit-saver.
- **Spend bits where people watch, across the group:** dynacast already pauses layers nobody watches.
  Per-viewer layer choice by tile size is already in LiveKit's adaptive stream (server-quality report). Nothing
  new to build.
- **Cursor sent separately:** Chrome can send the cursor in the video. Drawing it as a separate overlay (Parsec
  style) keeps it sharp at low res. Not exposed by `getDisplayMedia` beyond `cursor: "always"|"never"` [est.].
  **Not worth it.**

---

## 4. Verdict table

| Idea | Saves | Fit | Effort | Maturity | Risk | Verdict |
|---|---|---|---|---|---|---|
| Viewer upscale shader (lanczos / FSR1) | 10-25% bits on hard games [measured] | WebView2 JS (WebGL2/WebGPU) | 1-3 d | FSR1 shipped in games since 2021 | Loses overlay/VSR; over-sharpens UI | **v1** |
| Content-aware presets (games downscale, text never) | Avoids the bad quadrant [measured] | Config + detection | 1 d | n/a | Misdetection | **v1** |
| AV1 where HW encode exists | ~35-40% bits at low rates [measured] | JS codec prefs | 0.5 d + spike | Chromium MF AV1 exists [cited] | HW coverage among friends; HW path unproven in WebView2 | **v1 option / spike** |
| H.264 profile fix for NVIDIA | Avoids software encode | JS codec prefs / SDP | 0.5 d | n/a | Breaking negotiation | **Spike, must-do** |
| RTX VSR via `<video>` overlay | Free quality on RTX viewers | No code | 0.5 d test | Shipped since 2023 in Chrome | Overlay may not happen in WebView2; RTX only; AC power only | **Spike** |
| Intra refresh (+IDR on PLI) | Removes 5x bursts [measured] | Native only | in Phase B | NVENC since forever [cited] | SFU keyframe gating | **v2** |
| Zero-copy WGC→NVENC | Streamer CPU/memcpy | Native only | Phase B | Parsec/Sunshine do it | Driver variety (AMD/Intel) | **v2** |
| ROI / emphasis map | Moves quality to centre [measured] | Native only | 2-3 d | OBS ships it [cited] | Hurts HUD | **v2 experiment** |
| Deband shader | Looks better at low bits | Same canvas pass | 0.5 d | Common in mpv | Low | **v2** |
| L4S / SCReAMv2 | Latency under congestion | Browser flag | n/a | Canary only | n/a | **Not yet** |
| Frame interpolation (capture 30, show 60) | Halves streamer work; +1 frame of delay | DIY WebGPU (no ready path) | weeks | Games-only tech today | Artifacts on HUD/fast motion | **Spike-worthy, v3** |
| Neural codec (DCVC-RT etc.) | ~20-30% over AV1 at 10x compute [est.] | No browser path | months | Research | Everything | **Not worth it** |
| Game-state streaming | ~100% (for one game) | Per-game | n/a | Per-game | n/a | **Not worth it** |
| Dirty-rect protocol | ≤40 kbps [measured] | Custom | weeks | RDP | n/a | **Not worth it** |
| 4:4:4 text | +3 dB chroma on red text [measured] | Not in WebRTC | n/a | n/a | n/a | **Not worth it** |

---

## 5. What to add to the spike

1. **Profile check:** on a GeForce desktop *without* an iGPU, publish H.264 and log the negotiated
   `profile-level-id` and `getStats().encoderImplementation`. If it isn't hardware, set codec preferences so
   Baseline (42001f) / Main / High is chosen over Constrained Baseline.
2. **Viewer scaler check:** show a 720p stream in a 1440p window, screenshot it, and compare with a WebGL bilinear
   and lanczos render of the same frame. That tells us whether WebView2's default scaling is bilinear-grade or
   video-processor-grade.
3. **RTX VSR check:** same setup with VSR on in the NVIDIA control panel/app. Is VSR active (driver indicator /
   GPU tensor load) for a WebRTC `<video>` in WebView2? On AC power.
4. **FSR1 canvas prototype:** WebGPU `importExternalTexture` → EASU → RCAS (sharpness ~0.2-0.5 stops). Measure GPU
   ms and frame pacing (`requestVideoFrameCallback` jitter) at 1080p and 1440p output.
5. **AV1 check:** on an RTX 40 sharer, does WebView2 publish AV1 with a hardware `encoderImplementation`? Measure
   CPU while gaming.
6. **Streamer soak test (top priority, from the friends' feedback):** 45 minutes of a real game while sharing at
   720p60 and at 720p30. Log game FPS (PresentMon), CPU %, GPU 3D/copy/video-encode engine %, memory of every
   Pulse/WebView2 process, and GPU clocks/temperature. Compare against the game without sharing.
7. **Capture fps cap:** check that `getDisplayMedia({video:{frameRate:30}})` really captures less often (not just
   encodes less often). Use WGC's update-interval setting in the native path [est.: Windows 11 24H2+].


---

## 6. Honest bottom line

Weighted by what the friends actually suffer from (the streamer's game FPS, and stutter), the order is: use the
hardware encoder for sure, then capture and encode fewer pixels and frames and upscale on the viewer, then move the
whole capture→encode path onto the GPU (Phase B).

The "game rendering mechanics" that genuinely help are the boring, proven ones:

- a good upscaler on the viewer (FSR1 is literally a game upscaler, and it does help on game footage);
- a better codec (AV1);
- matching resolution vs fps to the content.

Everything flashier (frame generation, neural codecs, game-state, foveation) either can't run inside WebView2,
costs latency, or saves less than switching to AV1 does. The fun native-encoder tricks (intra refresh, ROI,
zero-copy) are real but belong with the Phase B native pipeline, not v1.

---

## 7. Notes on method and gaps

- Web search hit its session budget partway through. Some claims (Edge VSR status in 2026, the exact scaler WebView2
  uses for `<video>`) are therefore not re-verified. They're marked [est.] or dated.
- All quality numbers are VMAF on 6-second clips. A blind side-by-side with the friends would be the real test.
- FSR1 here is the mpv port (luma-only EASU+RCAS, default sharpness). A WebGPU port's output will differ a little.

## 8. Sources (all read 2026-10-14)

- Chromium `ui/gl/swap_chain_presenter.cc` (NVIDIA/Intel VSR toggles, battery gate, VP scaling):
  https://chromium.googlesource.com/chromium/src/+/main/ui/gl/swap_chain_presenter.cc
- Chromium `ui/gl/gl_features.cc` (`kIntelVpSuperResolution` disabled by default):
  https://chromium.googlesource.com/chromium/src/+/main/ui/gl/gl_features.cc
- Chromium `ui/gl/dc_layer_tree.h` (VSR/scaling disable workarounds):
  https://chromium.googlesource.com/chromium/src/+/main/ui/gl/dc_layer_tree.h
- Chromium MF encoder (codecs, low-latency, rate control, per-frame QP, NVIDIA constrained-baseline skip):
  https://chromium.googlesource.com/chromium/src/+/main/media/gpu/windows/media_foundation_video_encode_accelerator_win.cc
- Chromium `rtc_video_encoder_factory.cc` (`IsConstrainedH264`):
  https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/platform/peerconnection/rtc_video_encoder_factory.cc
- LiveKit server `pkg/rtc/mediaengine.go` (H.264 fmtp registration):
  https://github.com/livekit/livekit/blob/master/pkg/rtc/mediaengine.go
- libwebrtc WGC capture session (staging copy, zero-hertz damage):
  https://webrtc.googlesource.com/src/+/refs/heads/main/modules/desktop_capture/win/wgc_capture_session.cc
- WebRTC zero-hertz default-on commit: https://webrtc.googlesource.com/src/+/a57229bf36eec3b69f7fae20bb428ec7ff8d3509%5E%21
  and WGC static-content detection: https://webrtc.googlesource.com/src/+/6c453b7d59145bec2e7d3060ea373a9abcde3ef1
- NVIDIA RTX Video Super Resolution blog: https://blogs.nvidia.com/blog/rtx-video-super-resolution
- NVIDIA RTX Video SDK: https://developer.nvidia.com/rtx-video-sdk/getting-started
- NVIDIA Maxine VFX performance reference: https://docs.nvidia.com/maxine/vfx/latest/WindowsVFXSDK/PerformanceReference.html
- Moonlight-qt VSR PR #1180: https://github.com/moonlight-stream/moonlight-qt/pull/1180
- Vivaldi forum, VSR "Inactive": https://forum.vivaldi.net/post/706339
- Edge VSR (2023): https://www.techspot.com/news/97832-microsoft-now-has-web-video-upscaler.html
- AMD FSR1 docs: https://gpuopen.com/manuals/fidelityfx_sdk/techniques/super-resolution-spatial/
- FSR1 mpv port (used in tests): https://gist.github.com/agyild/82219c545228d70c5604f865ce0b0ce5
- WGSL FSR port (Shade): https://discourse.threejs.org/t/shade-webgpu-graphics/66969/175
- Anime4K-WebGPU: https://npmjs.com/package/anime4k-webgpu
- Chrome WebGPU external textures: https://developer.chrome.com/blog/new-in-webgpu-116
- NVIDIA Optical Flow SDK / FRUC: https://developer.nvidia.com/optical-flow-sdk ,
  https://docs.nvidia.com/video-technologies/optical-flow-sdk/nvfruc-programming-guide/index.html
- AMD AFMF (games): https://www.pcworld.com/article/2216804/amd-fluid-motion-frames-now-works-in-most-directx-games.html ;
  video-playback gap: https://forums.guru3d.com/threads/discover-amd-fluid-motion-frames-for-video.449942/page-4
- NVENC programming guide 13.0 (emphasis map, intra refresh, LTR, split-frame, tuning):
  https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-video-encoder-api-prog-guide/index.html
- OBS NVENC ROI commit: https://git.tjdev.de/mirror/obs-studio/commit/5e9b2d632252a157f79fb2fa70c10b3ffdae4517
- Parsec technology: https://parsec.app/blog/description-of-parsec-technology-b2738dcc3842 ;
  240 fps latency: https://parsec.app/blog/parsec-game-streaming-total-latency-at-240-frames-per-second-c0818cc0daa5
- Sunshine NVENC config: https://docs.lizardbyte.dev/projects/sunshine/latest/structnvenc_1_1nvenc__config.html
- WebCodecs per-frame quantizer: https://developer.mozilla.org/en-US/docs/Web/API/VideoEncoder/encode ;
  https://zenn.dev/tetter/articles/webcodecs-qp-encoding
- SCReAMv2 in libwebrtc: https://webrtc.googlesource.com/src/+/HEAD/modules/congestion_controller/scream/g3doc/implementation_diff.md ;
  IETF 125 slides: https://datatracker.ietf.org/meeting/125/materials/slides-125-ccwg-scream-v2-01
- Comcast L4S: https://tomshardware.com/tech-industry/comcasts-l4s-low-latency-tech-promises-up-to-a-78-percent-latency-reduction-testing-begins-in-select-cities
- DCVC-RT: https://arxiv.org/html/2502.20762v2 ; review:
  https://streaminglearningcenter.com/articles/evaluating-dcvc-rt-a-real-time-neural-video-codec-that-delivers-on-speed-and-compression.html
- WebNN origin trial: https://phoronix.com/news/chrome-146-beta ; https://groups.google.com/a/chromium.org/g/blink-dev/c/5CWKSChYo98
- AV1 film grain (Norkin & Birkbeck, DCC 2018): https://norkin.org/pdf/DCC_2018_AV1_film_grain.pdf
- Chrome AV1 (libaom real-time): https://developer.chrome.com/blog/av1
- Test clips (public domain): https://media.xiph.org/video/derf/twitch/
