# Screen share: cost to the streamer's PC, and reliability engineering

**Research date:** 2026-10-14. **Scope:** research only, feeding the screen-share spec. The design under study (from
`screenshare-summary.md`): video capture and encode run inside WebView2 (`getDisplayMedia` + `livekit-client` JS, as a
hidden second participant). Voice and stream audio stay in Rust.

**Labels used on every claim:**
- **[cited]**: a source says so. Each citation gives a URL and was read on 2026-10-14 unless another date is given.
- **[code]**: read directly in source code at the stated version or commit on 2026-10-14.
- **[measured]**: I measured it on the Linux test box (details in §1).
- **[est.]**: my engineering estimate.
- **[speculation]**: a plausible guess that needs testing.

---

## 0. TL;DR

1. **The biggest risk is that hardware encoding is off without anyone noticing.** It needs fixing in config on day one.
   - LiveKit's server answers a publisher's H.264 offer with **Constrained Baseline (`42e01f`) only** [code + measured]. It registers `42e01f` and High (`640032`) by default, and filters High out on the publisher's connection.
   - Chromium's Media Foundation encoder **skips NVIDIA GPUs for Constrained Baseline H.264** ("Skip NVIDIA GPU due to crbug.com/1088650") [code].
   - So, as things stand, an NVIDIA desktop with no usable iGPU will very likely encode **in software** (OpenH264). That is the path that costs CPU and game frames [est.].
   - **Fix (measured working):** add `profile-level-id=42001f` (and optionally `4d001f`) to `room.enabled_codecs` in `livekit.yaml`. With that change the same client negotiated `42001f` [measured]. The Windows spike must still confirm `encoderImplementation` on a real NVIDIA card.
2. **Chromium's capture path is not zero-copy.**
   - Each frame goes GPU → staging texture → CPU memcpy → CPU BGRA→I420 (libyuv) → CPU scale → shared memory → upload to the hardware encoder [code].
   - The zero-copy flags (`kWebRtcAllowWgcUsingTexture`, `kZeroCopyDesktopCapture`, `kMediaFoundationSharedImageEncode`) are all **disabled by default** [code].
   - This is the same round trip OBS removed in 2019 as its big NVENC performance fix [cited]. Expect a game-FPS cost nearer to "OBS 2018" than to "ShadowPlay": roughly 3-10% at 1080p30, more at 1080p60/1440p60 [est.].
   - **30 fps capture halves this work.** Capturing at lower resolution does not, because Chromium reads back and converts the full frame before scaling [code].
3. **Pulse's own UI can cost more than the encoder.**
   - A simulated busy Svelte screen (40 animated, blurred, glowing elements plus a 60 Hz rAF text meter) cost about **0.75 of a CPU core** (and added ~1 core to a publishing page) with software compositing [measured]. That is more than the capture and encode pipeline itself.
   - The Linux headless test exaggerates this (no GPU). The direction is still clear: **while sharing, freeze Pulse's animations, stop the levels meter, and hide the self-preview by default**.
4. **The friends' "PC gets slower until only a reboot fixes it" points below the app.**
   - The likely culprits are driver or kernel state (non-paged pool, dwm.exe, audiodg, GPU memory). A plain process leak is unlikely, because restarting the app would fix that.
   - §4 gives a counter for each cause, an automatic warning, and a mandatory 2-hour soak test.
5. **Reliability: many failures are silent.** All of these have to be detected by Pulse's own watchdogs, not by events:
   - **A minimised captured game:** frames stop, and the track stays `live` and unmuted [code].
   - **A hardware encoder error:** it falls back to software, and nothing fires but a stats change [code].
   - **A crashed streamer WebView:** the viewer saw a frozen frame for **~20 s** before LiveKit noticed [measured].
   - **`MediaStreamTrack.stop()`:** no `ended` event [measured].
6. **LiveKit's JS reconnect works well within limits** [measured]:
   - a server restart recovered in ~6 s, with automatic republish;
   - a 30 s outage recovered in ~2 s after the server came back;
   - an expired token was no problem;
   - **a 50 s outage gives up for good** (`Disconnected`), so Pulse needs its own outer retry.
7. **Stopping and restarting a share by unpublish/republish leaks ~20 MB per cycle** in the renderer [measured]. `replaceTrack`, or disconnecting the hidden participant when a share ends, avoids it (§4.5).
8. **Voice survives a WebView2 crash by design.** It lives in Rust. Rust should also tell viewers "stream interrupted" at once over the voice connection, rather than leaving them on a frozen frame for 20 s.

---

## 1. How I measured (and what the numbers are worth)

- **Box:**
  - Linux (CachyOS);
  - i9-13900HX, 32 threads;
  - Google Chrome 151.0.7922.71 headless (`--headless=new`, SwiftShader software GL, no GPU video);
  - local LiveKit server 1.13.7 (Docker, `--dev`);
  - `livekit-client` 2.22.4, driven by Playwright.
- **Harness:** `/tmp/claude-1000/screenshare-perf/` (throwaway, not in any repo):
  - `run.mjs`: perf matrix;
  - `fail.mjs`: fault injection;
  - `soak.mjs`: memory over time.
- **CPU:** CPU % is per process type, from `/proc/<pid>/stat` over 30 s, as a percentage of **one core**. Memory is PSS (proportional set size, so shared pages aren't double counted).
- **Repeats:** each configuration was run twice.
- **Sources:**
  - "canvas" = a synthetic high-entropy, game-like picture drawn at the stated size and fps;
  - "gdm" = Chromium's fake `getDisplayMedia` device (simple content).
- **Caveats (important):**
  - Encoding here is **OpenH264 software** (Linux Chrome has no hardware H.264 encoder without VA-API). Read the encode numbers as **"what a streamer pays if the hardware encoder silently falls back"**, not as the Windows hardware path.
  - Compositing is software (SwiftShader), which inflates UI and preview costs compared with a real GPU.
  - A first matrix run was **contaminated**: a sibling job running ffmpeg/VMAF drove the load average to about 76. I discarded it and re-ran when the load was below 5. Only the clean re-run is reported.

---

## 2. Part 1: the cost to the streamer's game and PC

### 2.1 What Chromium actually does per frame (Windows)

**Choosing the capturer** [code: `content/browser/media/capture/desktop_capture_device.cc`, chromium/src `main`]:
- **Window capture:** always WGC (Windows.Graphics.Capture).
- **Screen capture:** WGC **only on Windows 11 24H2+** (`IsWgcEnabledForScreenCapture`). Older systems use DXGI Desktop Duplication, with a fallback.
- **Zero-hertz:** on for WGC, so a static picture produces no new frames.

**Readback** [code: `webrtc/src/modules/desktop_capture/win/wgc_capture_session.cc`]:
- WGC copies the frame into a `D3D11_USAGE_STAGING` texture with CPU read access, then `Map()`s it and memcpys row by row into a CPU `DesktopFrame`.
- The DXGI path also uses a CPU-readable staging texture.
- The texture-sharing path needs Windows 11 24H2 **and** `kWebRtcAllowWgcUsingTexture`, which is `FEATURE_DISABLED_BY_DEFAULT` [code: `media/webrtc/webrtc_features.cc` L27].

**Colour conversion and scaling** [code: `desktop_capture_device.cc`]:
- `libyuv::ARGBToI420` runs on the CPU over the **full-size** frame. Then `libyuv::I420Scale(kFilterBox)` scales to the size you asked for.
- The frame then goes to the renderer and on to the encoder as a CPU buffer.
- The GPU-texture route (`kZeroCopyDesktopCapture`, `kMediaFoundationSharedImageEncode`) is disabled by default [code: `media/base/media_switches.cc`].

**Built-in CPU limiter** [code]:
- The capture period is `max(last_capture_duration × 100/50, requested_frame_duration)`, with `kDefaultMaximumCpuConsumptionPercentage = 50`.
- If one capture takes more than half the frame interval, Chromium quietly lowers the capture fps. That is a source of **viewer stutter** when the streamer's PC is busy [est.].

**Data moved** [est.]:
- 1080p at 4 bytes/pixel is 8.3 MB per frame. At 30 fps that is ~250 MB/s of GPU→CPU readback plus CPU conversion; at 60 fps it is ~500 MB/s. 1440p60 is ~885 MB/s.
- The readback competes with the game for PCIe and memory bandwidth. A WGC copy also uses some of the GPU's 3D/copy engine time.

**The encoder** [code: `media/gpu/windows/media_foundation_video_encode_accelerator_win.cc`]:
- `kMediaFoundationVideoEncodeAccelerator` is **enabled** by default.
- The encoder MFTs (Media Foundation transforms) are enumerated per adapter (`MFTEnum2`, `MFT_ENUM_ADAPTER_LUID`) and tried in merit order.
- If the MFT is on a different adapter from the GPU process, `dxgi_resource_mapping_required_` adds a cross-adapter copy.

**Comparison** [cited]:
- OBS 2019: "NVENC performance improvements": frames "no longer go to system RAM before reaching NVENC … directly from VRAM", which "should noticeably reduce resource usage" — https://obsproject.com/forum/threads/nvenc-performance-improvements-release-candidate.98950/
- Discord does its own capture and encode in a native module that "integrates with operating systems and video drivers" — https://discord.com/blog/from-blocky-to-brilliant-improving-video-quality-on-discord-go-live-on-amd-gpus (2024-05-01). So Pulse's path is heavier per frame than Discord's [est.].

### 2.2 The H.264 profile trap (hardware encode silently not used on NVIDIA)

**Chromium skips NVIDIA for Constrained Baseline** [code]:

```cpp
// media_foundation_video_encode_accelerator_win.cc, ActivateAsyncEncoder()
// Skip NVIDIA GPU due to https://crbug.com/1088650 for constrained
// baseline profile H.264 encoding, and go to the next instance according to merit value.
if (codec_ == VideoCodec::kH264 && is_constrained_h264 && vendor == DriverVendor::kNvidia) continue;
```

The crbug is not public.

**What LiveKit negotiates:**
- LiveKit's default codec set registers H.264 `42e01f` (packetization mode 0 and 1) and High `640032` [code: `livekit/protocol` `codecs/codecs.go` L193-195].
- The SFU sets `filterOutH264HighProfile: !IsOfferer` [code: `livekit/livekit` `pkg/rtc/transport.go` L1042]. The publisher's connection is the one where the server answers, so High is filtered out there.
- `42001f`/`4d001f` are registered only if `enabled_codecs` lists them with that exact fmtp line [code: `pkg/rtc/mediaengine.go` L66-94].
- **[measured]** Chrome offers `42001f` first (`RTCRtpSender.getCapabilities` order: 42001f, 42e01f, 4d001f). With default server settings the publish still negotiated `profile-level-id=42e01f`.
- **Consequence [est.]:**
  - **NVIDIA desktop with no iGPU:** Chromium skips NVENC and falls to OpenH264 software. This is the worst case for a CPU-bound game.
  - **Intel CPU with iGPU enabled:** it falls to Quick Sync, which is fine.
  - **AMD:** not affected by this skip.
- **Fix [measured]:** a second local server with the `livekit.yaml` below made the same publisher negotiate `profile-level-id=42001f`. Baseline (not constrained) is not skipped for NVIDIA.

  ```yaml
  room:
    enabled_codecs:
      - mime: audio/opus
      - mime: video/vp8
      - mime: video/h264
        fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f"
      - mime: video/h264
        fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=4d001f"
      - mime: video/h264        # keeps the defaults (42e01f / High) for other clients
      - mime: video/av1
  ```

  - Viewers decoded `42001f` fine (Chromium's decoder handles Baseline).
  - **The spike must confirm on Windows + NVIDIA:** `outbound-rtp.encoderImplementation` should name Media Foundation (not "OpenH264"), and `powerEfficientEncoder` should be `true`.
  - Also check whether WebView2 on a hardware-capable machine offers High (`640c1f`). If it does, consider allowing High for the publisher [speculation].
  - Mobile/Safari viewers would also need Baseline decode. They have it [est.].

### 2.3 Encoder sessions, simulcast and VRAM

**Simulcast means one encoder per layer:**
- Chromium's `RTCVideoEncoder` refuses simulcast except VP9 (`ERR_SIMULCAST_PARAMETERS_NOT_SUPPORTED`). libwebrtc then builds one encoder per layer [code: `third_party/blink/renderer/platform/peerconnection/rtc_video_encoder.cc`].
- Layers **below 360p are forced to software** (`kForceSoftwareForRtcLowResolutions`, on by default on desktop) [code].
- livekit-client's default screen-share simulcast adds one layer at half resolution [code: `publishUtils.ts`]. 1080p → 540p is a second hardware session; 720p → 360p is right at the limit.
- **[measured, software path]** simulcast of game-like 1080p60 content in software cost ~0.65 of a core, and the full-resolution layer collapsed to ~2 fps (§2.6). The real cost on Windows is a second NVENC session and a second capture-scale [est.].

**NVIDIA consumer session limit:**
- **12** concurrent NVENC sessions for every GeForce in NVIDIA's current support matrix (RTX 5090, 4070, 3060, 2060, GTX 1660, 1630) — https://developer.nvidia.com/video-encode-and-decode-gpu-support-matrix-new [cited, page undated].
- The SDK 13.0 application note still says 8 — https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-application-note/index.html [cited].
- vMix reports that 12 arrived with GeForce driver 591.44 (December 2025) — https://blog.vmix.com/?p=4331 [cited].
- History: 3 sessions until about March 2023, then 5 — https://www.techspot.com/community/topics/nvidia-geforce-users-can-now-encode-more-simultaneous-streams.279727/ [cited].

**Does Discord + OBS + Pulse hit the limit?**
- Discord Go Live (1-2 sessions) + OBS stream and record (2) + Pulse with 2 layers (2) ≈ 6 [est.].
- That is **under the limit on any driver from 2023 onward**. It could only bite on drivers from before March 2023 (limit 3).
- AMD has no documented session limit — https://jellyfin.org/docs/general/post-install/transcoding/hardware-acceleration/amd [cited, secondary]. Intel: none known [speculation].

**When sessions run out:**
- MFT activation fails, `RTCVideoEncoder` returns `WEBRTC_VIDEO_CODEC_FALLBACK_SOFTWARE`, and libwebrtc switches to software [code; that session exhaustion surfaces this way: est.].
- It is **silent** except for `encoderImplementation` changing.
- Also unverified: whether WebView2 ships OpenH264 at all. If not, a hardware failure might leave **no** H.264 encoder [speculation; check in the spike with `--disable-features=WebRtcHWEncoding`].

**Recommendation:** no simulcast for game streams (6 viewers), or keep the low layer at 360p or above. Revisit only if a viewer on a bad link needs it.

**VRAM per session:**
- No NVIDIA figure exists. One report puts an idle session configured for 2880×1800 at about 130 MB — https://forums.developer.nvidia.com/t/reducing-nvenc-memory-usage/165325 [cited].
- **Estimate:** 50-150 MB per 1080p/1440p session, plus Chromium GPU-process textures. That matters only for games already at the VRAM limit (see §4.2 E) [est.].

### 2.4 Game FPS loss: what published data exists

No rigorous benchmark (1% lows, several games) of Discord, Chromium or WGC streaming was found. The data is anecdotal:

| Setup | Baseline → while streaming | Source |
|---|---|---|
| GTX 980, The Division (pre-2019 OBS) | 101 → OBS NVENC 74 (−27%), x264 veryfast 73, **ShadowPlay 98 (−3%)** | https://obsproject.com/forum/threads/obs-poor-nvenc-performance.47201 [cited, anecdotal] |
| GTX 1070 + i7-6700K, Guild Wars 2 | 92 → NVENC 82 (−11%), x264 71 (−23%) | https://obsproject.com/forum/threads/low-fps-using-nvenc.98601 [cited, anecdotal] |
| AC Odyssey 1080p Ultra, OBS 60 fps | NVENC: 0% skipped frames; x264 "faster": 45.6% skipped | https://www.igorslab.de/en/nvidias-nvenc-vs-cpu-encoding-the-turing-video-encoder-for-twitch-streaming-co-comparison-analysis-with-netflix-vmaf/4/ (2020-07-22) [cited] |
| ShadowPlay Instant Replay | −1-2 fps at 1080p-1440p, up to ~10% at 4K | https://linustechtips.com/topic/812083-performance-impact-of-nvidia-shadowplay-instant-replay-testing/ [cited, anecdotal] |
| Window capture (OBS), Witcher 3 | −20 fps reported | https://obsproject.com/forum/threads/huge-in-game-fps-hit-when-window-capture-is-on.53135 [cited, anecdotal] |

**Reading [est.]:**
- The ShadowPlay vs OBS-2018 gap (−3% vs −27%) is mostly the **readback through system RAM**. Pulse/Chromium has that readback today.
- So with a working hardware encoder, expect **roughly 3-10% average FPS loss at 1080p30**, worse at 60 fps and on 1% lows when the game holds the GPU at 99%.
- With a **software fallback**, expect x264-like losses in CPU-bound games.
- The spike must measure this with PresentMon/CapFrameX: off vs 720p60 vs 1080p30 vs 1080p60.

**GPU-bound games** [cited]: OBS's long-standing advice is to cap the game so the GPU sits at or below about 90%, leaving room for capture/encode — https://obsproject.com/forum/threads/low-fps-in-obs-frames-missed-due-to-rendering-lag-gpu-problems-look-here.79146/post-372633 [cited, anecdotal threshold]. Pulse can suggest this: "your game is using 99% GPU; capping it at your monitor's refresh rate makes the stream smoother".

### 2.5 GPU scheduling, priority, Game Mode

- **OBS raises its own GPU priority.** `libobs-d3d11/d3d11-subsystem.cpp` calls `D3DKMTSetProcessSchedulingPriorityClass(..., hags_enabled ? HIGH : REALTIME)` and `SetGPUThreadPriority`, and needs admin. OBS's 24.0.3 post warns it "may result in a small reduction in performance of any games" — https://github.com/obsproject/obs-studio/blob/master/libobs-d3d11/d3d11-subsystem.cpp ; https://obsproject.com/forum/threads/obs-studio-24-0-3-gpu-priority-fix-testing.111669/ [cited].
- **Chromium does not do this** [est.]. Pulse can't set it for the WebView2 GPU process without admin, and **shouldn't**: normal priority protects the game, at the cost of stream frames when the GPU is saturated [est.].
- **HAGS** (hardware-accelerated GPU scheduling):
  - OBS still logs a warning when it is on — "Hardware-Accelerated GPU Scheduling enabled on adapter!" [code].
  - OBS forum staff: "HAGS is currently known to cause performance and capture issues … recommend disabling it" (Windows 10 2004 era) — https://obsproject.com/forum/threads/frames-missed-skipped-due-to-rendering-encoding-lag-help.151942/post-582401 [cited].
  - User results are mixed — https://obsproject.com/forum/threads/safe-to-re-enable-hags-hardware-accelerated-gpu-scheduling.162780/latest [cited].
  - **Log HAGS on/off in bug reports.** Don't advise either way until we have our own data [est.].
- **Game Mode** gives the foreground game more CPU and GPU priority (press coverage: https://www.pcworld.com/article/411833/how-windows-10s-game-mode-will-make-your-pc-games-run-better.html [cited, secondary]). It protects the game; Pulse's capture may drop frames instead [est.].
- **"Optimizations for windowed games"** (Windows 11) moves DX10/11 windowed games to flip model — https://support.microsoft.com/en-us/topic/3f006843-2c7e-4ed0-9a5e-f9389e535952 [cited]. Whether WGC capture pushes a game back to composed flip (more latency, less FPS) is unknown [speculation].
- **WGC pacing quirk on 24H2:** with `MinUpdateInterval` at 0, one developer got about 50 fps from a 144 fps source, and exactly 1 ms fixed it — https://github.com/robmikh/Win32CaptureSample/issues/82 [cited; the maintainer could not reproduce]. Pulse cannot set this inside Chromium. If the spike sees capture fps stuck near 50, this is a suspect [est.].

### 2.6 Measured: per-process CPU and memory of the pipeline (Linux, software encode)

Clean re-run (load average ≤ 6 on 32 threads). The CPU column is the sum over all of the publisher browser's processes, in % of one core, averaged over two 30 s runs. "Capture" is the browser process (fake capture, convert and scale). "Renderer" is WebRTC: encode, packetize, and the page itself. Memory is the PSS of the whole publisher browser. [measured]

| Configuration | Publisher CPU (total) | Capture (browser proc.) | Renderer | GPU proc. | PSS | Encode ms/frame | Sent |
|---|---|---|---|---|---|---|---|
| Idle page, connected, nothing published | 1.9% | 0.1% | 1.0% | 0% | ~220 MB | – | – |
| **Fake screen 720p60, H.264 software (OpenH264)** | **23%** | 4.2% | 17.6% | 0% | ~235 MB | 2.0 | 1280×720@60 |
| **Fake screen 1080p60, H.264 software** | **39%** | 6.8% | 30.5% | 0% | ~250 MB | 4.2 | 1920×1080@60 |
| Game-like canvas 720p60 drawn but **not** published (cost of the test source itself) | 19% | – | 18% | 0% | ~215 MB | – | – |
| Game-like canvas 720p60, published, **nobody watching** (dynacast) | 18% (≈ source only) | – | 17% | 0% | ~227 MB | – | layer `active:false`: **no encoding** |
| Game-like canvas 1080p60 software H.264, simulcast (2 layers) | 66% | – | 63% | 0% | ~290 MB | 8.6-37 | top layer collapsed to **~2 fps** |
| Canvas 720p60 + **self-preview `<video>`** (not published) | 35% (+16 vs no preview) | – | 25% | 9.5% | ~228 MB | – | – |
| Canvas 720p60 + **1 Hz 32×18 black/frozen sampler** (not published) | 17% (no change) | – | 16% | 0% | +~140 MB* | – | 2.7 ms per sample |
| **Busy UI only** (40 blurred, pulsing, glowing pills + 60 Hz rAF text meter), nothing published | **75%** | – | 48% | 27% | ~320 MB | – | – |
| Canvas 720p60 published **+ busy UI** | **133%** (vs 32% without UI) | – | 97% | 34% | ~277 MB | 1.9 | – |

\* The sampler's PSS rise is mostly the hidden `<video>` element and canvas backing in a software-rendered headless browser. Re-check on Windows; a `MediaStreamTrackProcessor` in a worker would avoid the `<video>` element [est.].

**What the numbers say:**

- **Software encode costs real CPU** [measured]:
  - A fake 1080p60 screen costs ~0.4 of a core, and 1080p60 game-like (noisy) content is far worse. With 2-layer simulcast, the full-resolution layer **fell to ~2 fps at 8-37 ms per frame**.
  - The capture-and-convert step alone (browser process) costs 4-7% of a core at 720p-1080p60, even before any encoding. On Windows that step also includes the GPU→CPU readback [code].
  - Viewer decode on Linux (software) costs ~10% of a core at 1080p60.
- **Dynacast saves everything when nobody watches** [measured]. With no subscriber, livekit-client marks the layer inactive and the encoder does no work: the publisher cost equals the source cost.
  - So "Go Live" with nobody watching is nearly free. Pulse should still keep the capture running, so the first viewer gets a picture instantly.
- **Pulse's own UI can dominate.**
  - A heavily animated UI cost **~0.75 core** on its own, and **added ~1 core** to a publishing page.
  - Software compositing exaggerates this. On a real GPU the CPU cost is far smaller, but the GPU cost then competes with the game.
  - **Freezing animations while sharing is the cheapest big win we control** [measured direction, est. size on Windows].
- **A live self-preview costs as much as a second encode** here (+16% of a core) [measured]. Off by default.
- **Black/frozen sampling is free** at 1 Hz × 32×18 [measured].
- **Game-like content hit the bandwidth limit at 4.5 Mbps**: random-noise frames are unrealistically hard to encode. With `maintain-framerate`, WebRTC held 60 fps and cut resolution to 320×180-480×270 [measured]. That shows the degradation preference doing its job: resolution goes before smoothness. Real games compress far better than noise [est.].
- **Memory:** a publishing browser (all processes) was ~230-250 MB PSS on Linux. That is close to the idle browser, so WebRTC publishing adds roughly 10-30 MB [measured]. WebView2 on Windows will differ, but expect the same order [est.].

### 2.7 Laptops (hybrid iGPU + dGPU)

- **Which GPU:**
  - WebView2's GPU process runs on Chromium's chosen adapter. Windows' per-app GPU preference defaults to "Let Windows decide", usually the iGPU for non-games — https://pureinfotech.com/set-default-gpu-apps-windows-11/ [cited, secondary].
  - The preference is per executable (`msedgewebview2.exe`, shared by every WebView2 app) [speculation].
  - One report says WebView2 ignores the host app's high-performance request — https://www.construct.net/en/forum/construct-3/general-discussion-7/wv2-discrete-gpu-184007 [cited, anecdotal].
- **Likely outcome [speculation]:**
  - The GPU process sits on the iGPU and the encoder MFT is picked by merit. Combined with the Constrained-Baseline NVIDIA skip, **Intel QSV is the likely encoder** on Intel laptops.
  - That is actually **good**: it uses the iGPU's media engine, leaves the dGPU to the game, and helps battery.
  - The cost is a cross-adapter copy if the game is on the dGPU and is captured by WGC on the iGPU.
  - If we fix the profile (§2.2), an NVIDIA laptop might start choosing NVENC. Measure both on a laptop in the spike.
- **WGC works across GPUs.** DXGI duplication fails on hybrid laptops (see `screenshare-capture-encode.md` §1.2). Before 24H2, Chromium uses DXGI for **screen** capture, so **prefer window capture on laptops** [est.].
- **Battery and thermals:** no measured NVENC-vs-QSV battery data found [gap]. Expect heat-soak throttling after 10-30 min on thin laptops (§4.2 G). On battery, default to 720p30 [est.].

### 2.8 Ways to cut the cost, ranked

| # | Lever | Saves | Evidence |
|---|---|---|---|
| 1 | **Fix the H.264 profile** so NVIDIA uses NVENC (§2.2) | Avoids software encode: ~0.4 core for simple 1080p60 content in our test, far more for game-like content | [code + measured] |
| 2 | **30 fps instead of 60** for "sharp text" and on laptops | Halves readback, conversion and encode work | [code: capture timer = frameRate] [est. for size of saving] |
| 3 | **Freeze Pulse's UI while sharing:** pause CSS animations, stop the `Levels` meter (opt-client M1), no rAF loops, `prefers-reduced-motion`-style class on `<body>` | Simulated busy UI ≈ 0.75 core alone, +1 core on a publishing page (software compositing) | [measured] |
| 4 | **Self-preview off by default** (or a 1 fps snapshot thumbnail) | A 640 px live preview cost +16% of a core (software compositing) | [measured] |
| 5 | **No simulcast** for 6 friends | One encoder session instead of two; one scale pass | [code] |
| 6 | **`contentHint = 'motion'` and `degradationPreference: 'maintain-framerate'`** for games, `'detail'` for text | The right thing drops under load (resolution, not frames). Also fixes stutter (§4.3) | [code: livekit-client forces `maintain-resolution` for ScreenShare by default; WebRTC maps `motion`→MAINTAIN_FRAMERATE] |
| 7 | **Ask for the size you send** (`width/height/frameRate` constraints) | Saves encode work and the IPC copy, **not** the full-size readback/convert | [code] |
| 8 | **Window capture of the game, not the whole screen**, on Windows before 24H2 | Avoids DXGI (worse on hybrid GPUs) | [code] [cited] |
| 9 | **Suggest a game frame cap** when GPU is at 99% | Leaves GPU room for capture | [cited, OBS advice] |
| 10 | **Don't `TrySuspend` or set `MemoryUsageTargetLevel=Low` on the WebView while it's sharing** | TrySuspend fails while capturing anyway; Low can swap the encode loop's memory out | [cited: icorewebview2_3 / _19 docs] |

**Does hiding Pulse's window throttle the stream?** No, mostly not:
- The capture timer runs in the browser process with a wake lock.
- A live `MediaStreamTrack` opts the page out of aggressive throttling.
- `media_stream_count_ > 0` keeps the renderer's priority up [code: `media_stream_track_impl.cc`, `render_process_host_impl.cc`].
- **But JS timers in a hidden page are still throttled to once per second** — https://developer.chrome.com/blog/timer-throttling-in-chrome-88 [cited]. So keep **no JS in the frame path**, and run the 5 s stats poll knowing it may drift [est.].
- Chromium's native occlusion tracking stops rendering an occluded WebView, which is good: the UI costs nothing while the game covers it — https://chromium.googlesource.com/chromium/src/+/da170c2267b2201954efe1c8fa5d2d89ab09f7bf/docs/windows_native_window_occlusion_tracking.md [cited].

---

## 3. Part 2: reliability (detect, recover, observe)

### 3.1 Measured fault injection (Linux, Chrome 151, livekit-client 2.22.4, LiveKit 1.13.7)

The setup was a publisher (canvas 720p30, H.264) and a viewer, with the viewer's decoded fps sampled every second. "Viewer frozen" means 0 decoded frames per second.

| Fault injected | Events seen (seconds after the fault) | Viewer experience | Notes |
|---|---|---|---|
| `track.stop()` on the capture track | **none**: no `ended`, nothing unpublished | Frozen last frame, **indefinitely**, with no event on either side | Per spec, `stop()` doesn't fire `ended`. Our own stop path must unpublish explicitly |
| `ended` fired on the capture track | publisher `LocalTrackUnpublished` at +0.0; viewer `TrackUnsubscribed` at +0.2 | Stream disappears in 0.2 s | livekit-client auto-unpublishes ScreenShare on `ended` [code: `LocalParticipant.handleTrackEnded`] |
| Second client joins with the **same identity** | old client `Disconnected` reason **2** (`DUPLICATE_IDENTITY`) at +0.5 | Stream gone at +0.7 s | Must be treated as "signed in elsewhere": don't auto-reconnect, or two instances will fight |
| Browser `offline` for 15 s (WebSocket blocked, UDP still up) | `SignalReconnecting` at +13 s; `Reconnected` 2.8 s after it came back | **No interruption** (30 fps throughout) | Media rides on UDP. Signal loss alone is harmless |
| Server **frozen 8 s** (`docker pause`) | **No events at all** | Frozen 8 s, then ~24 fps for ~40 s while bandwidth estimation re-ramped | Short outages are silent. Only stats show them |
| Server **frozen 30 s** | `SignalReconnecting` at +13.6 s, `Reconnecting` at +17.7 s, `Reconnected` 2 s after the server returned | Frozen 30 s; full 30 fps 4 s after return | Within livekit-client's ~50 s retry budget |
| Server **restart** | `SignalReconnecting` at +1 s; full reconnect; publisher **republished automatically**; viewer re-subscribed at +8.4 s | Frozen ~6 s | Full reconnect = everyone leaves and rejoins (`ParticipantDisconnected` + `Connected` events), so UI must not flash "X left" |
| Server restart **after the token expired** (20 s TTL, restart at 30 s) | Identical to the line above | Frozen ~6 s | Confirms reconnects use the server-pushed refreshed token [cited: docs.livekit.io/frontends/authentication/tokens] |
| Server **down 60 s** | `Reconnecting` at +18.7 s, **`Disconnected` at +49.7 s (viewer) / +51.3 s (publisher)**; nothing after the server returned | Stream gone; **never recovers by itself** | `DefaultReconnectPolicy` gives up after 10 tries (0, 0.3, 1.2, 2.7, 4.8, 7 s ×5, plus jitter) [code]. Pulse needs an outer retry loop |
| **Publisher renderer crash** (`chrome://crash`) | Viewer: **nothing for 20.8 s**, then `ParticipantDisconnected` (reason 14 `CONNECTION_TIMEOUT`) | Frozen last frame for ~21 s | The server waits out its participant timeout. Rust voice can tell viewers at once (§3.3) |

**Logs:** `/tmp/claude-1000/screenshare-perf/fail/*.json` (harness `fail.mjs`).

### 3.2 Failure modes, end to end

Key for the "Detect" column: `ms-fps` = `media-source.framesPerSecond`; `out` = `outbound-rtp`; `in` = `inbound-rtp`.

| Failure | Detect (cheapest reliable signal) | Auto-recover | What the user sees |
|---|---|---|---|
| **Captured window closed** | Track `ended`. WGC `GraphicsCaptureItem.Closed` → permanent error → `ended` [code: `wgc_capture_session.cc`, `desktop_capture_device.cc`]. livekit-client auto-unpublishes | None possible: a new `getDisplayMedia` needs a user gesture | Streamer: toast "Your stream ended because the window closed", with a **one-click "Share again"** button. Viewers: "Stream ended" card, not a frozen frame |
| **Captured window minimised** | No `mute`, no `ended` [code: `video_track_adapter.cc` `kMediaStreamTrackEmptyVideoFrameMonitor` disables the mute timer; WGC skips frames when `IsIconic`]. A Chrome WebRTC lead: Chrome "doesn't produce frames from a window capture if the window's minimized" — https://lists.w3.org/Archives/Public/public-webrtc-logs/2025Jan/0093.html [cited]. Detect with Rust `IsIconic(hwnd)` on the captured window (we know it from the picker via `ScreenCaptureStarting`) [est.], or `ms-fps` = 0 for > 2 s | Resumes when restored | Streamer: banner "Your game is minimised; viewers see a paused picture". Viewers: "Paused" overlay after 2 s of no frames (§3.4) |
| **Static picture** (text, paused game) | WGC zero-hertz: **no frames is normal** [code] | n/a | Nothing. Don't alarm on 0 fps alone; pair it with "window not minimised" and a frozen-hash check |
| **Exclusive fullscreen game** | Black frames (luma ≈ 0 with no variance) or no frames | User switches the game to borderless | Streamer warning within ~3 s of going live: "We're capturing a black picture. Set the game to Borderless/Windowed" — Discord gives the same advice ([cited] https://support.discord.com/hc/en-us/articles/9410427556375 via search snippet) |
| **Protected/anti-cheat black** | Same black detector | None | Same warning, worded "this game blocks capture" if the black persists in borderless |
| **Lock screen / UAC prompt** | Frozen or black frames. DXGI temporary errors loop; `UNSUPPORTED_SESSION` ends the track [code: `screen_capturer_win_directx.cc`]. Rust can watch `WTS_SESSION_LOCK` | Resumes on unlock (temporary), or re-share | Viewers: "Paused" overlay |
| **Monitor unplugged / display change** | Screen source invalid → permanent error → `ended` [code; user-visible result est.]. Rust: `WM_DISPLAYCHANGE` | Re-share | Same as "window closed" |
| **Window resized** | `ms` width/height change; WGC re-creates its frame pool [code] | Automatic | Nothing |
| **GPU device lost / TDR / driver update** | WebView2 `ProcessFailed` with `GPU_PROCESS_EXITED` ("most common … auto-recoverable", content "might flash") [cited: https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-related-events]. The encoder errors and falls back to software [code: `RTCVideoEncoder::Impl::NotifyErrorStatus` → `WEBRTC_VIDEO_CODEC_FALLBACK_SOFTWARE`]. Capture may end (texture path) or continue [code; TDR behaviour speculation] | After a GPU-process exit, **re-publish the track** (unpublish, then publish the same `MediaStreamTrack`) to get a fresh encoder and retry hardware [speculation; test with `dxcap -forcetdr`] | Streamer: "Graphics driver reset; stream restarted" |
| **Encoder silently falls back to software** | `out.encoderImplementation` changes (e.g. to "OpenH264") or `powerEfficientEncoder` becomes `false`. Only visible while the page is capturing (spec "exposing hardware is allowed" rule; Chromium `ExposeHardwareCapabilityStats()`) [cited/code]. **[measured]** a canvas-source publisher (not capturing) got no `encoderImplementation`; a `getDisplayMedia` publisher got `"OpenH264"`. A viewer page that isn't capturing never sees `decoderImplementation` [measured] | Re-publish once to retry hardware; if still software, drop to 720p30 automatically | Streamer: "Hardware encoder unavailable; using a lighter quality to protect your game" |
| **CPU-limited encode** | `out.qualityLimitationReason == 'cpu'`; livekit-client emits `TrackEvent.CpuConstrained` / `ParticipantEvent.LocalTrackCpuConstrained` (polled every 2 s) [code: v2.22.4] | Step down a preset (fps first for text, resolution first for games) | Streamer: small "Your PC is struggling; lowered to 720p" notice |
| **Bandwidth-limited** | `qualityLimitationReason == 'bandwidth'`; `candidate-pair.availableOutgoingBitrate` below the preset | WebRTC adapts; Pulse suggests a lower preset after 30 s | "Your upload limits quality" |
| **Network drop, short (< 15 s)** | No room events [measured]. Viewer `in` framesDecoded stalls; `freezeCount` rises | Automatic (ICE) | Viewer "Reconnecting…" overlay after 2 s of no frames |
| **Network drop, long (15-50 s)** | `RoomEvent.SignalReconnecting` → `Reconnecting` → `Reconnected` [measured] | Automatic (resume or full reconnect) | "Reconnecting…" overlay |
| **Outage > ~50 s / give-up** | `RoomEvent.Disconnected` with no reason [measured] | **Pulse's own retry:** back off 5 s → 60 s, ask Rust for a fresh token (new `connect()` needs one [cited]), re-`connect()`, re-publish the **same** `MediaStreamTrack` if it is still `live` (no new picker) [est.] | "Stream lost connection. Retrying…" |
| **Server restart / deploy** | `Reconnecting`, then a full reconnect [measured] | Automatic, ~6 s | Brief "Reconnecting…". Suppress join/leave sounds and roster flicker for `#v`/`#s` identities |
| **Token expiry** | Not a problem for reconnects [measured]; only a brand-new `connect()` needs a new token | Rust mints a new token on demand | Nothing |
| **Second instance** | `tauri-plugin-single-instance` stops a second copy on the same PC (named mutex + `WM_COPYDATA`; it "must be the first plugin registered") — https://v2.tauri.app/plugin/single-instance/ [cited]. Another device with the same user: `Disconnected(DUPLICATE_IDENTITY=2)` [measured] | **Do not** auto-reconnect on reason 2 | "Your stream was taken over by another device" |
| **Streamer's WebView2 renderer crash** | Rust `ProcessFailed` (`RENDER_PROCESS_EXITED`). Viewers otherwise see nothing for ~21 s [measured] | Rust sends "stream interrupted" to the room over the **voice** connection at once; reload the WebView; offer "Share again" (capture permission is lost) | Viewers: "Stream interrupted" within ~1 s instead of a 21 s frozen frame |
| **Viewer's WebView2 renderer crash** | `ProcessFailed` | Reload, then auto-rejoin and re-watch the same stream (a new `connect()` needs a fresh token) | Brief blank, then the stream comes back |
| **WebView2 browser process exits** | `ProcessFailed` `BROWSER_PROCESS_EXITED`; all controls must be recreated [cited] | Recreate the WebView (Tauri: restart the window); voice continues | "Pulse's window restarted; you're still in voice" |
| **WebView2 runtime auto-updated** | `NewBrowserVersionAvailable`; running WebViews keep the old version [cited: https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2environment] | **Never restart while sharing.** Apply at the next idle | Nothing |
| **Sleep/resume** | The PC transport comes back `NEW`, which forces a full reconnect [code: `RTCEngine.ts` comment "signaling state could change to closed due to hardware sleep"]. Rust: `WM_POWERBROADCAST` / `PBT_APMRESUMEAUTOMATIC` | End the share on suspend (the game is paused anyway); voice reconnects as today | "Stream ended when your PC went to sleep" |

### 3.3 WebView2 process failures, and keeping voice alive

- **The events.** `ICoreWebView2::add_ProcessFailed`; `ICoreWebView2ProcessFailedEventArgs2` adds `Reason`, `ExitCode`, `ProcessDescription` and `FrameInfosForFailedProcess` [cited: https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/webview2-idl].
  - Kinds: `BROWSER_PROCESS_EXITED`, `RENDER_PROCESS_EXITED`, `RENDER_PROCESS_UNRESPONSIVE`, `FRAME_RENDER_PROCESS_EXITED`, `UTILITY_PROCESS_EXITED`, `SANDBOX_HELPER_PROCESS_EXITED`, `GPU_PROCESS_EXITED`, `PPAPI_*`, `UNKNOWN_PROCESS_EXITED`.
  - Reasons: `UNEXPECTED`, `UNRESPONSIVE`, `TERMINATED`, `CRASHED`, `LAUNCH_FAILED`, `OUT_OF_MEMORY`, `NORMAL_EXIT`, `ABNORMAL_EXIT`, `INTEGRITY_FAILURE` [cited].
- **Microsoft's guidance** [cited: process-related-events page]:
  - Renderer exited: `Reload()`, or close and recreate.
  - Unresponsive: fires repeatedly ("every 15 seconds"); the app picks the threshold.
  - GPU/utility exited: auto-recovered, so log only.
  - Browser exited: recreate everything.
- **Tauri/wry do not expose `ProcessFailed` yet.** wry PR #1862 (`with_process_failed_handler`) is open and unmerged; wry 0.57.0 was released 2026-09-08 — https://github.com/tauri-apps/wry/pull/1862 [cited].
  - **Today:** `webview.with_webview(|pw| pw.controller())` gives the `ICoreWebView2Controller` [code: tauri `crates/tauri/src/webview/mod.rs`]. Then `controller.CoreWebView2()?.add_ProcessFailed(...)` with `webview2_com` handler types [est.].
  - Do the recovery on the event loop, not inside the COM callback [cited: wry PR notes].
- **Voice must survive.** It already does structurally: voice is in the Rust process, and a renderer crash doesn't touch it. Two rules keep it that way:
  1. **Nothing voice-critical may depend on the WebView** (mute/deafen hotkeys, PTT, device switching must work from Rust). Today's UI→Rust commands are fine. Just make sure no voice task `await`s a WebView reply.
  2. **A browser-process exit takes the hidden video participant down.** Rust should immediately update a voice-participant **attribute** (e.g. `stream=interrupted`), which viewers' Rust clients see within ~100 ms. That replaces the 21 s server timeout [measured] with a ~1 s UX [est.]. Do the same for a clean stop (`stream=off`), so viewers never rely on SFU timeouts.
- **Never call `TrySuspend` on a sharing WebView.** It needs `IsVisible=false` and defers to Sleeping-Tabs rules, which exempt capturing pages [cited: icorewebview2_3; Edge sleeping-tabs FAQ].
  - `MemoryUsageTargetLevel=Low` (opt-client H4) must be **skipped while sharing or watching**. "Scripts … continue to run", but memory may be swapped out [cited: icorewebview2_19].

### 3.4 Observability: what to log every 5 s

Use the same style as the existing `voice: path` line (one `tracing::info!` per 5 s window, `-` for unknown). The JS side gathers `getStats()` and posts a compact object to Rust over IPC, so it lands in the same log and bug reports [est.].

**Streamer** (`share: send`):

```text
share: send src 1920x1080@58 enc 1920x1080@57 impl MediaFoundation hw yes ql none(cpu 0s bw 2s)
  qp 24 kbps 6980/7000 aob 11.2M rtt 38ms loss 0.1% nack 3 pli 0 kf 1 huge 0
  enc_ms 4.1 send_delay_ms 6 codec H264/42001f layers 1 black no frozen no
```

| Field | Where it comes from |
|---|---|
| `src` | `media-source` width, height, `framesPerSecond` |
| `enc` | `outbound-rtp` frameWidth, frameHeight, framesPerSecond |
| `impl`, `hw` | `encoderImplementation`, `powerEfficientEncoder` (**log on every change too, as its own line**) |
| `ql` | `qualityLimitationReason`, plus the deltas of `qualityLimitationDurations` this window |
| `qp` | `qpSum` / `framesEncoded` delta |
| `kbps` | bytes delta, against `targetBitrate` |
| `aob` | `candidate-pair.availableOutgoingBitrate` |
| `rtt`, `loss` | `remote-inbound-rtp` `roundTripTime`, `fractionLost` |
| `nack`, `pli`, `kf`, `huge` | deltas of `nackCount`, `pliCount`, `keyFramesEncoded`, `hugeFramesSent` |
| `enc_ms` | `totalEncodeTime` / `framesEncoded` delta: the leak/heat trend signal |
| `send_delay_ms` | `totalPacketSendDelay` / `packetsSent` delta: pacer queue |
| `codec` | `codec.sdpFmtpLine`: proves the profile fix |
| `black` / `frozen` | the sampler below |

**Viewer** (`share: recv`):

```text
share: recv 1920x1080@59 dec_ms 1.9 jb 62ms freeze 0 (0ms) pause 0 drop 0 ifd_sd 4.2ms kf 0 pli 0 loss 0.0% layer f
```

| Field | Where it comes from |
|---|---|
| resolution and fps | `inbound-rtp` |
| `dec_ms` | `totalDecodeTime` / `framesDecoded` delta |
| `jb` | `jitterBufferDelay` / `jitterBufferEmittedCount` delta |
| `freeze` | `freezeCount` and `totalFreezesDuration` deltas |
| `pause` | `pauseCount` delta |
| `drop` | `framesDropped` delta |
| `ifd_sd` | **the stutter metric**: standard deviation of inter-frame delay, from `totalInterFrameDelay` and `totalSquaredInterFrameDelay` deltas |
| `kf` / `pli` / `loss` | inbound counterparts of the streamer's fields |

**Host health** (`share: host`, from Rust, every 5 s while sharing or watching; also once a minute in voice):

```text
share: host priv 412MB wv_priv 980MB handles 1840 gdi 310 commit 18.2/31.9G npool 312MB dwm 145MB audiodg 22MB/540h
  gpu ded 6.1/8.0G shr 0.4G venc 18% 3d 97% temp 78C clk 2610MHz thr none cpu_perf 104%
```

| Field | Where it comes from |
|---|---|
| `priv`, `wv_priv`, `handles`, `gdi` | Pulse's and the WebView2 children's `PrivateUsage`, handle count and GDI count |
| `commit`, `npool` | `GetPerformanceInfo`: commit total/limit and kernel non-paged pool |
| `dwm`, `audiodg` | PDH `\Process(dwm)\Private Bytes`, and `audiodg` private bytes plus handles |
| `gpu ded`, `shr` | GPU adapter dedicated and shared usage, against the budget |
| `venc`, `3d` | GPU engine utilisation for `VideoEncode` and `3D` |
| `temp`, `clk`, `thr` | NVML temperature, clock and throttle reasons (NVIDIA only; `-` elsewhere) |
| `cpu_perf` | `% Processor Performance` |

The APIs and their admin needs are in §4.4.

**Event lines** (one-off, not 5 s): `share: start` (preset, source kind, window title hash, WGC vs DXGI guess, OS build, HAGS on/off, GPU vendor/driver), `share: impl` changes, `share: ended reason=…`, `share: process_failed kind=… reason=… exit=…`, `share: reconnect …`, `share: warn …`.

**Black and frozen detection** (streamer side, plus viewer side):

- **[measured] cost:** sampling the track once per second into a **32×18** `OffscreenCanvas` (`drawImage` + `getImageData`, Rec.709 luma mean, plus per-pixel diff from the last sample) took **~2.7 ms per sample**, with no measurable change in CPU on the renderer main thread. That is negligible.
- **Prior art** [code]:
  - Chromium's own test helper `isVideoBlack` uses Rec.709 luma with a threshold of 20 per pixel — https://chromium.googlesource.com/chromium/src/+/main/content/test/data/media/webrtc_test_utilities.js
  - WebRTC's `BlankDetectorDesktopCapturerWrapper` samples every 105th pixel — https://webrtc.googlesource.com/src/+/main/modules/desktop_capture/blank_detector_desktop_capturer_wrapper.cc
- **Black rule [est.]:**
  - Mean luma < 16/255 **and** luma spread < 4 for **3 consecutive samples** means "black".
  - Pair it with "frames are arriving" (`ms-fps` > 0) to tell *black* (exclusive fullscreen or protection) from *no frames* (minimised or static).
- **Frozen rule [est.]:** identical 32×18 hash for > 10 s while `IsIconic`=false and the game is the foreground window is a "frozen capture" warning.
  - Static desktops are normal. Only warn when sharing a **game window**.
- **Viewer stall overlay:** if `framesDecoded` doesn't advance for 2 s, show "Paused/Reconnecting…" over the last frame. That is the same idea as Jitsi's `TrackStreamingStatus`, which tracks the last `framesDecoded` change — https://github.com/jitsi/lib-jitsi-meet/blob/master/modules/connectivity/TrackStreamingStatus.ts [code]. **Never show a silent frozen frame.**
- **Hidden-page caveat:** if the Pulse window is hidden, the 1 s sampler's timer is throttled to at most one run per second [cited]. That is fine at a 1 s cadence. Don't go faster.

**Self-test before going live** (about 3 s, behind the "Go Live" button; the streamer sees "Checking…"):

1. `getDisplayMedia` succeeds. Check the frames: if **black** → borderless warning; if **no frames for 2 s** → "window minimised?".
2. Publish to the room with **no viewers subscribed**.
   - **[measured]** with dynacast and nobody watching, livekit-client marks the layer inactive (`active: false`). The encoder then does no work, so the test needs a self-subscribe or a forced layer on.
   - Better: publish, and let the streamer's own **Rust** side confirm the SFU sees the track (`TrackPublished`).
3. After ~2 s of encoding, read `encoderImplementation`/`powerEfficientEncoder`.
   - If software: auto-pick 720p30 and tell the user why.
   - Also log `codec.sdpFmtpLine` (expect `42001f`).
4. Read `availableOutgoingBitrate` after ~3-5 s. If it is below the preset, offer the lower preset (`screenshare-server-quality.md` §6 point 4).
5. Show the result in one line: "Hardware encoder ✓ · Upload 18 Mbps ✓ · Capture 60 fps ✓".

**Automatic warnings** (a toast for the streamer, a line in the log):

| Warning | Trigger | Suggested toast |
|---|---|---|
| Software encode | `hw` = no for > 10 s | "Hardware encoder off; using 720p30" |
| CPU-limited | `ql=cpu` for > 20 s of the last 30 s | "Your PC is struggling; lowered quality" |
| GPU saturated | `3d` > 97% for 60 s **and** capture fps < 80% of target | "Your game uses all the GPU. Cap its FPS for a smoother stream" |
| Encode-time creep | `enc_ms` 1.5× its 5-minute baseline, with the same resolution | "Stream is slowing; check temperatures" (log, plus toast if it persists) |
| VRAM | dedicated > 95% of budget, or shared usage +500 MB above its baseline | "Graphics memory is full; your game may stutter. Lower the stream quality" |
| Memory leak | Pulse or WebView private bytes growing > 50 MB per 10 min for 30 min (linear fit) | "Pulse memory growing" (log + bug-report prompt) |
| System pool | non-paged pool > 1 GB or +300 MB since stream start | "Windows kernel memory is climbing (a driver leak). See help" |
| dwm or audiodg growth | dwm private bytes +500 MB since start; audiodg handles > 20 k | The help link explains a driver leak |
| Thermal | NVML throttle reason SwThermal/HwThermal/HwSlowdown set, or `cpu_perf` < 70% under load | "Your GPU/CPU is throttling from heat" |

---

## 4. The friends' Discord pain, and how Pulse avoids it

This is first-hand evidence from the friend group, and their #1 complaint about Discord:
- **(a)** the streamer's game FPS tanks while streaming, whether they share the game or another window;
- **(b)** a progressive slowdown: about 30 minutes are fine, then the whole PC (Discord, the browser) gets slower and slower until **only a reboot** helps;
- **(c)** viewers mainly complain about **stutter**, not delay.

### 4.1 (a) The game's FPS tanks

Likely causes, in rough order for a Chromium-style pipeline [est. unless marked]:

1. **Software encoding without anyone noticing.** For Pulse, that is the H.264 profile trap (§2.2) [code + measured].
   - Discord has its own hardware toggles, and users report fixes from toggling "OpenH264"/hardware encoding [cited, anecdotal; see Discord support threads in §6].
   - **Pulse fix:** the server codec config, the self-test, a permanent `impl` log line, and a "software" warning.
2. **GPU-to-CPU readback and CPU colour conversion** of every frame at full size (§2.1) [code]. OBS's 2019 fix of exactly this was the big NVENC gain [cited].
   - **Pulse mitigation:** 30 fps for anything that isn't a fast game; 720p60 as the game default; no 1440p60 default.
   - Later, an opt-in experiment with `--enable-features=WebRtcAllowWgcUsingTexture,ZeroCopyDesktopCapture,MediaFoundationSharedImageEncode` through `AdditionalBrowserArguments`. Microsoft advises against shipping flags, and these are experimental [cited: https://learn.microsoft.com/microsoft-edge/webview2/concepts/webview-features-flags]. **Spike item, not a default.**
3. **GPU saturation.** The game at 99% leaves no room for the capture copy and the compositor [cited, OBS advice]. **Pulse:** detect `3d` ≥ 97% and suggest a frame cap.
4. **Sharing a different window still costs.** WGC copies the composed window, and DWM has to keep compositing it even when it is behind the game [est.].
   - A full-screen game in independent flip may be knocked into composed mode while *its own* window is captured [speculation].
   - **Pulse:** measure "share game window" vs "share browser while gaming" separately in the spike.
5. **The app's own UI** (animations, meters, preview) [measured: §2.6]. **Pulse:** freeze the UI while sharing; preview off.
6. **Discord-specific things** Pulse doesn't have:
   - The **injected capture hook** (Pulse never hooks).
   - Discord's earlier bug forcing NVIDIA cards into the CUDA P2 memory clock (−200 MHz memory, small FPS loss) — https://www.hothardware.com/news/geforce-memory-clocks-gimped-with-discord-open [cited].
   - **Lesson [est.]:** any CUDA/NVENC context we create ourselves can change the GPU's P-state. That is one more reason to leave encoding to Media Foundation inside WebView2 and never open NVENC/CUDA from Rust.

### 4.2 (b) "Fine for 30 minutes, then the PC crawls until a reboot"

**What it is probably not.** "Only a reboot fixes it", together with "even the browser is slow", points **away from a plain process leak** (quitting the app would fix that) and **towards system-level state**: kernel pool, dwm.exe, audiodg, or GPU driver memory [est.].

**What the evidence shows.** No bug report matched this exact story [cited: search found only an unresolved 2019 Discord thread, https://support.discord.com/hc/en-us/community/posts/360055945392-Crashing-when-streaming-games]. The candidates, with how to tell them apart:

| # | Cause | Signature (what climbs, how fast) | Fixed by app restart? | Fixed without reboot? | Evidence | How Pulse's design avoids it | Live counter + auto-warning |
|---|---|---|---|---|---|---|---|
| A | **GPU-driver leak inside dwm.exe** | dwm.exe private bytes and GPU memory climb steadily while capture runs; whole desktop sluggish | No | Yes: `Win+Ctrl+Shift+B` (driver reset) or dwm restart | Intel DCH drivers after 27.20.100.8587 leaked in dwm, "starts around 30MB and grows over time", fixed in 30.0.101.1191 — https://www.intel.com/content/www/us/en/support/articles/000058381/graphics.html [cited]; newer unexplained Win11 reports — https://learn.microsoft.com/en-us/answers/questions/2359194/dwm-exe-memory-leak [cited] | We can't. But WGC through Chromium uses the standard OS path, and there is no hook [est.] | `dwm` private bytes in `share: host`; warn at +500 MB since start |
| B | **Non-paged pool leak in a kernel driver** (network filter, anti-cheat, VPN, GPU) | `\Memory\Pool Nonpaged Bytes` rises steadily; no process owns it; **only a reboot frees it** | No | **No: reboot only** | Killer Networking suite leaks reported past 1.6 GB — https://community.intel.com/t5/Wireless/Technical-Bug-Report/m-p/1746123 [cited]; "only the Windows reboot helps to clean up the non-paged pool" — https://woshub.com/huge-memory-usage-non-paged-pool-windows/ [cited]. A per-packet leak would scale with stream bitrate [speculation] | Nothing in-app. We can **detect and explain** it, and PoolMon finds the driver tag | `npool`; warn if > 1 GB or +300 MB since stream start. This is **the best match for the friends' "only a reboot" story** |
| C | **audiodg.exe handle/memory leak** from a third-party audio effect (Razer 7.1, FXSound, Nahimic, ASUS) | audiodg handles climb toward millions; audio glitches, then the system slows | No | Yes: restart the Windows Audio service | Razer's 7.1 APO leaked audiodg up to 16.7 M handles / 12.4 GB — https://insider.razer.com/razer-synapse-4-55/bug-7-1-surround-rzsurroundapo-dll-leaks-handles-in-audiodg-exe-until-windows-limit-audio-dies-after-10h-92071 [cited] | Our process-loopback capture keeps the audio graph busy too [est.] | `audiodg` handles; warn > 20 k |
| D | **NVENC / GPU driver session or VRAM leak** | Adapter dedicated memory creeps up with no process accounting for it; later "OpenEncodeSessionEx failed: out of memory" | Maybe | Driver reset, usually | OBS NVENC "out of memory (10)" after several starts, fixed by reboot — https://obsproject.com/forum/threads/nvenc-failed-to-start.117094/ [cited]; NVIDIA 565.xx VRAM-fills-over-time in one app — https://support.keyshot.com/en/knowledge-base/memory-leak-issue-with-nvidia-drivers-565.09-566.03-and-566.14-in-keyshot [cited] | One encoder session (no simulcast); encode in Chromium's MF path; no repeated encoder churn [est.] | `gpu ded` adapter total vs the sum of processes; `venc` |
| E | **VRAM exhaustion spilling to shared memory** | Dedicated ≈ 100%, shared climbs; game and desktop stutter as allocations are paged — Microsoft: over-budget processes "will likely experience stuttering, as they are intermittently frozen and paged-out" — https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_4/nf-dxgi1_4-idxgiadapter3-queryvideomemoryinfo [cited]. Builds as the game streams in assets | Yes (stop stream / game) | Yes | [cited] above; GPU shared-memory cap = half of RAM — https://devblogs.microsoft.com/directx/gpus-in-the-task-manager/ [cited] | Low preset on ≤ 6 GB cards; no simulcast; preview off; free WebView surfaces (UI frozen) [est.] | `gpu ded/shr`; warn at > 95% of budget or shared +500 MB |
| F | **App process leak** (Discord, Electron, Chromium capturer, WGC frame pool, encoder) | App/WebView private bytes, handles or GDI grow linearly with stream time | **Yes** (fully quit, including the tray) | Yes | Thin: old WebRTC fix for a window capturer leaking GDI handles — https://webrtc.googlesource.com/src/webrtc/+/08099e0d57605dd3ae750623429a1e5124fc6672 [cited]; no current Chromium/WGC leak report found | **LiveKit rust-sdks #1277** (each Rust *video* publish keeps ~40 MB until the Room closes; audio publishes don't leak — https://github.com/livekit/rust-sdks/issues/1277, opened 2026-07-23 [cited]) does **not** apply: video publishes from JS, and Rust publishes only stream *audio*. **[measured]** see §4.5 | `priv`, `wv_priv`, `handles`, `gdi`; warn on linear growth |
| G | **Thermal heat-soak** (laptops and small cases) | Temperatures plateau, clocks fall, NVML throttle bits set; encode time per frame creeps up; memory flat. 5-30 min onset | No | Yes, after cooling (a reboot also cools, which may explain the "reboot fixed it" belief) | Laptop GPU clocks dropping after ~30 min of gaming — https://yomotherboard.com/?p=503682 [cited, anecdotal]; NVML clock-event reasons — https://docs.nvidia.com/deploy/nvml-api/group__nvmlClocksEventReasons.html [cited] | Lighter defaults on laptops (720p30 on battery); hardware encode keeps the CPU cool | `temp`, `clk`, `thr`, `cpu_perf`; warn on thermal throttle bits |
| H | **CPU-side queue buildup** (frames captured faster than encoded/sent) | Latency grows, then memory; `send_delay_ms` and capture→encode gap rise | Yes | Yes | WebRTC drops frames rather than queueing: "the encoder drops frames when congestion is high" (Discord blog, above) [cited]. Chromium's capture timer self-throttles at 50% CPU [code] | No JS in the frame path; libwebrtc owns the queues [est.] | `send_delay_ms`, `src` fps vs `enc` fps |
| I | **DWM / MPO (multiplane overlay) interaction** | Flicker, black frames or stutter in capture and video playback; not progressive | n/a | Registry workaround (`OverlayTestMode=5`; reportedly ineffective on 24H2) | https://bugzilla.mozilla.org/show_bug.cgi?id=1693130 [cited]; mixed user results [cited, Reddit via §6] | Nothing to do in-app | Not countable. Note the driver version in `share: start` |

**A triage script for the friends today, before Pulse has screen share** [est.]:
1. During a long Discord stream, run `typeperf` (or a tiny Pulse-side logger) every 5 s on:
   - `\Memory\Pool Nonpaged Bytes`;
   - `\Process(dwm)\Private Bytes`;
   - `\Process(audiodg)\Handle Count`;
   - `\Process(Discord*)\Private Bytes`;
   - `\GPU Adapter Memory(*)\Dedicated Usage` and `Shared Usage`.
2. When it gets slow, try these in order and note which one brings the PC back:
   1. fully quit Discord (and its tray icon) → F;
   2. `Win+Ctrl+Shift+B` → A or D;
   3. restart the "Windows Audio" service → C.

   If none of them work and non-paged pool has climbed, it is B: run PoolMon as admin to name the driver.

### 4.3 (c) Viewer stutter (not delay)

**Causes in this area, and what Pulse does:**

- **The streamer's capture cadence is uneven:**
  - Chromium's 50%-CPU capture limiter (§2.1) [code];
  - the WGC 24H2 pacing quirk [cited];
  - Game Mode starving the capture [est.].
  - **Detect:** the streamer's `src` fps below target, or `ms-fps` jitter.
  - **Fix:** the streamer-side fixes above, plus a 60→30 fps auto-step when the source can't hold 60.
- **Resolution-first degradation turns load into dropped frames.** livekit-client defaults ScreenShare to `maintain-resolution` [code]. Under CPU or bandwidth pressure that **drops frames**, which looks like stutter.
  - **Fix:** `maintain-framerate` + `contentHint='motion'` for game presets [code; Chrome default behaviour cited: https://lists.w3.org/Archives/Public/public-webrtc-logs/2024Nov/0035.html].
- **Loss → PLI (picture-loss indication) → keyframe → freeze** (`freezeCount`, `pliCount`). Kernel UDP buffers and PLI throttle are covered in `screenshare-server-quality.md` §6-7.
- **Too little receive buffer.**
  - For a "watching a game" experience, 100-150 ms of extra viewer delay is invisible but absorbs jitter. Set `RTCRtpReceiver.jitterBufferTarget` (or `playoutDelayHint`) to about 100 ms on the viewer for game streams.
  - Chrome applies it to video as render-time clamping — https://groups.google.com/a/chromium.org/g/blink-dev/c/4W4orKqA3Rs/m/6k0LdB9wCwAJ [cited]. The setting itself is [est.]; tune it in the spike using `ifd_sd`.
- **60 fps content on 144 Hz monitors** has uneven frame gaps by construction — https://obsproject.com/forum/threads/video-still-stuttering.114472/ [cited]. Not fixable by us; VRR would help [est.].
- **Simulcast layer switching** (resolution pops). Not an issue without simulcast [est.].
- **Viewer-side busy UI.** The same animation freeze applies to viewers while a stream is open (their compositor shares the GPU with their game, if they play while watching) [est.].
- **[measured]** In the clean test runs, the viewer's `freezeCount` stayed at 0 on an idle box. Under the contaminated (load ≈ 76) run, freezes appeared at 20+ per 30 s, and resolution collapsed to 320×180 because the bandwidth estimate and CPU starved. So **a starved streamer PC shows up as viewer stutter**, and the streamer's `ql`/`src` fields are where to look first.

### 4.4 Counters Pulse can sample every 5 s from Rust (cheap; mostly no admin)

| Counter | API | Admin? | Source |
|---|---|---|---|
| Pulse and WebView2 children: private bytes | `GetProcessMemoryInfo` → `PROCESS_MEMORY_COUNTERS_EX.PrivateUsage` | No (own children; find PIDs via `ICoreWebView2Environment8::GetProcessInfos` [cited, WebView2 docs]) | MS docs [est. for child access] |
| Handles / GDI / USER | `GetProcessHandleCount`, `GetGuiResources(GR_GDIOBJECTS/GR_USEROBJECTS)` | No; same session | https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getguiresources [cited] |
| Commit, non-paged pool, system handles | `GetPerformanceInfo` (`CommitTotal`, `CommitLimit`, `KernelNonpaged`; pages × `PageSize`) | No | https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-performance_information [cited] |
| dwm, audiodg memory and handles | PDH `\Process(dwm)\Private Bytes`, `\Process(audiodg)\Handle Count` | No (PDH) [est.] | |
| GPU memory (adapter, per process) | PDH `\GPU Adapter Memory(*)\Dedicated Usage`, `Shared Usage`; `\GPU Process Memory(pid_*)\…`; or `IDXGIAdapter3::QueryVideoMemoryInfo` (budget for our own process) | No | https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_4/nf-dxgi1_4-idxgiadapter3-queryvideomemoryinfo [cited]; PDH names [cited, secondary] |
| GPU engines | PDH `\GPU Engine(*engtype_VideoEncode)\Utilization Percentage`, `(*engtype_3D)` | No | [est.; instance format from `typeperf -q "GPU Engine"`] |
| NVIDIA temperature, clocks, throttle reasons, encoder use and sessions | NVML `nvmlDeviceGetTemperature`, `GetClockInfo`, `GetCurrentClocksEventReasons` (bits SwPowerCap 0x4, HwSlowdown 0x8, SwThermal 0x20, HwThermal 0x40), `GetEncoderUtilization`, `GetEncoderSessions` (`nvml.dll` ships with the driver) | No [est.] | https://docs.nvidia.com/deploy/nvml-api/group__nvmlClocksEventReasons.html [cited] |
| AMD/Intel temperature | ADLX (AMD); no generic Windows API. WMI thermal zones are unreliable and need admin | — | [est.] |
| CPU throttling | PDH `\Processor Information(_Total)\% Processor Performance` (> 100 = boost; falling under load = throttle) | No | [est.]; `CallNtPowerInformation` `CurrentMhz` is imprecise — https://community.osr.com/discussion/265659/ [cited] |

**Cost [est.]:** PDH queries for about 15 counters every 5 s is well under 1 ms of CPU. Open the query once and keep it. NVML is optional (load the DLL dynamically; skip it if absent).

### 4.5 Measured: memory over time in the browser pipeline (Linux)

Setup: a publisher (fake screen, 720p30, H.264 software) and a viewer, with PSS sampled every 20 s per process type. Harness: `soak.mjs`. [measured]

| Run | Duration / cycles | Publisher renderer PSS (start → end) | Per cycle | JS heap | Viewer renderer PSS |
|---|---|---|---|---|---|
| Steady stream, no changes | 30 min | 71 → 92 MB (one transient peak of 114 MB) | – | 6 → 8 MB (flat) | 62 → 86 MB |
| **Stop + start share by `unpublishTrack` + `publishTrack`** (each with a new `getDisplayMedia`) | 30 cycles in 15 min | **70 → 685 MB** | **+20.5 MB per cycle** | flat (6-9 MB) | flat (~80 MB) |
| Stop + start by **`LocalVideoTrack.replaceTrack(newTrack)`** (same publication) | 24 cycles in 8 min | 72 → 93 MB | ~0.9 MB (≈ the steady drift) | flat | 58 → 75 MB |
| Stop + start by **disconnecting the room**, reconnecting, then publishing | 24 cycles in 8 min | 77 → 126 MB | ~2 MB | flat | 61 → 104 MB |

**What this shows:**

- **Republishing within one connection leaks about 20 MB of native memory per share start** [measured]. The JS heap stays flat, so the memory is outside JS. That points at libwebrtc keeping each old sender/encoder until the peer connection closes [speculation]. It is the browser-side twin of LiveKit rust-sdks #1277 (~40 MB per Rust video publish) [cited].
  - A friend who starts and stops a share 20 times in an evening would add ~400 MB to the WebView renderer.
  - **Design rule:**
    - when switching windows mid-share, use `replaceTrack`;
    - when a share ends, **disconnect the hidden `#s` participant** (closing the peer connection frees it);
    - never unpublish and republish inside one connection.
  - Re-check on Windows/WebView2 in the spike, and file upstream (livekit client-sdk-js / crbug) if it reproduces.
- **A steady stream drifts about +0.7 MB/min** in the renderer over 30 min, flattening towards the end [measured]. That is within normal allocator warm-up and well under the soak criterion (< 50 MB/hour after the first 10 min) in the second half. It needs the full 2-hour soak on Windows to call it flat.
- **Handles/fds were flat** in every run (366-370 open fds per publisher browser) [measured].
- **Viewer frame rate stayed at 27-30 fps throughout.** No freezes in any soak.

### 4.6 Mandatory soak test (before ship, and on every Chromium/WebView2 or livekit-client bump)

**Two hours on real hardware**, at the 1080p60 preset, sharing a running game with 2+ viewers.

**Record:**
- the `share: host` / `send` / `recv` lines;
- PresentMon game frame times for the first 5 min and the last 5 min.

**Pass criteria [est.]:**
- Pulse + WebView private bytes grow < 50 MB/hour after the first 10 min.
- Handles and GDI are flat (± 5%).
- Non-paged pool, dwm and audiodg are flat.
- GPU dedicated usage is flat.
- `enc_ms` in the last 10 min is within 20% of the first 10 min.
- Viewer `freezeCount` per hour does not increase between the first and second hour.
- Game 1% low FPS in the last 5 min is within 5% of the first 5 min.

**Variants:**
- 20 start/stop cycles in one session (catches #1277-style per-publish retention);
- one laptop on battery (thermal);
- one run with Discord also open and streaming (session limits, contention).

---

## 5. Automated test harness (catch regressions)

### 5.1 What exists to build on

- **Chromium fake capture** [cited: peter.sh switch list "as of October 14, 2026", https://peter.sh/experiments/chromium-command-line-switches/; code: `media_stream_manager.cc`]:
  - `--use-fake-device-for-media-stream[=fps=60,display-media-type=monitor]` makes **`getDisplayMedia` return a fake surface**. **[measured]** It works headless and gives 1280×720 or 1920×1080 at the fps you ask for.
  - Add `--use-fake-ui-for-media-stream` (no picker) and `--auto-select-desktop-capture-source="Entire screen"` (or `--auto-select-screen-capture-source`).
  - **Force software encode** for comparison: `--disable-features=WebRtcHWEncoding,WebRtcHWDecoding`. The old `--disable-webrtc-hw-encoding` switch is gone [cited].
  - **Wire into WebView2:** `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` env var, appended to the app's own arguments [cited: https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/webview2-idl]. Tauri already passes a `--disable-features=…` list, so put all features into **one** list in the test build's `additionalBrowserArgs` (whether two lists merge is unknown [speculation]).
- **`lk` CLI** (v2.19.0, 2026-10-08):
  - `lk room join --publish file.h264|.ivf --fps 60`, or `--publish h264://127.0.0.1:16400` (TCP, works on Windows), or `h264:///tmp/x.sock` (Unix socket, not on Windows) [cited: https://github.com/livekit/livekit-cli README, `cmd/lk/room.go`].
  - **It cannot mark the track as screen share.** `handlePublish` never sets `Source` [code: `cmd/lk/join.go`]. So for a true `SCREEN_SHARE` publisher, use a ~50-line Go program with `server-sdk-go` (`TrackPublicationOptions.Source = livekit.TrackSource_SCREEN_SHARE`) [cited + est.].
  - Or (simpler, and closer to the real thing) use headless Chromium running **our own JS bundle** with fake capture, as I did.
  - `lk load-test --video-publishers 1 --subscribers 5 --video-codec h264 --no-simulcast --duration 1m` for SFU load; no loss injection [cited].
- **`room.simulateScenario()`** in livekit-client: `signal-reconnect`, `node-failure`, `migration`, `resume-reconnect`, `full-reconnect`, `force-tcp`, `force-tls`, `subscriber-bandwidth` [code: client-sdk-js `src/room/types.ts`]. Useful in JS unit/integration tests without Docker tricks.
- **Quality and latency:** jitsi-meet-torture stamps a **QR code on every frame** of a y4m fed as the fake camera, then decodes it on the receiver for PSNR, drops and latency — https://github.com/jitsi/jitsi-meet-torture [cited]. Reusable: QR (frame number + send timestamp) → viewer decodes → freezes, drops and glass-to-glass latency. Whether `--use-file-for-fake-video-capture` also feeds the *display* fake device is unknown [speculation]; a canvas source with a drawn QR works regardless.
- **Network impairment:** `tc qdisc add dev <veth> root netem delay 80ms 20ms loss 2% rate 10mbit` on Linux runners (use a Docker network or netns so signalling isn't impaired) — https://man7.org/linux/man-pages/man8/tc-netem.8.html [cited]. toxiproxy is TCP-only (signalling/TURN-TLS) [cited]. clumsy on Windows has no CLI [cited].

### 5.2 Proposed harness (three tiers)

**Tier 1: Linux, every PR (about 3 min)** [est.]. This is what `/tmp/claude-1000/screenshare-perf/` already does:
1. `livekit-server --dev` in Docker, with the **production codec config** (§2.2), so the profile regression is caught.
2. Publisher: headless Chrome, **the real Pulse share module**, fake capture or a QR canvas at 720p60.
3. Viewer: headless Chrome, the real viewer component.
4. Assert:
   - `codec.sdpFmtpLine` contains `42001f`;
   - viewer fps ≥ 55 for 20 s;
   - `freezeCount` == 0;
   - QR sequence has no gaps > 2;
   - `ql` != cpu.
5. Fault tests: `docker pause 8s/30s`, `docker restart`, renderer crash, duplicate identity, `stop()`.
   - Assert the **app's** behaviour: overlay shown within 2.5 s; republish after restart; outer retry after a 60 s outage.
   - These are the §3.1 scenarios turned into assertions.
6. A netem variant: 2% loss / 80 ms → freezes < N.

**Tier 2: Windows hosted runner, nightly (no GPU)** [cited facts, est. design]:
- **What the runner has:**
  - `windows-2025` has Edge 154 + msedgedriver, and WebView2 is present but **not pinned** (Evergreen from the base image) — https://github.com/actions/runner-images/blob/main/images/windows/Windows2025-Readme.md and https://github.com/actions/runner-images/issues/9538 [cited].
  - Runner sessions are interactive (WinSta0) — https://github.com/actions/runner-images/issues/14049 [cited].
  - **No GPU**, so Chromium renders with WARP/SwiftShader and **no hardware MFT exists** [est.].
  - `Server-Media-Foundation` is not in the image's feature list — https://github.com/actions/runner-images/blob/main/images/windows/toolsets/toolset-2025.json [cited]. Check with `Get-WindowsFeature Server-Media-Foundation`; install it in the job if needed.
- **Drive the app with Playwright over CDP, not msedgedriver.**
  - `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222 --use-fake-ui-for-media-stream --use-fake-device-for-media-stream"`, then `chromium.connectOverCDP('http://localhost:9222')` — https://playwright.dev/docs/webview2 [cited].
  - The msedgedriver attach to WebView2 is reported **broken on windows-2025 from runtime 133 on** — https://github.com/actions/runner-images/issues/14738 [cited]. Tauri's own WebDriver docs now push `@wdio/tauri-service` — https://v2.tauri.app/develop/tests/webdriver/ [cited].
- **What it can test:**
  - the real Tauri app boots;
  - the share UI flow (picker auto-accepted);
  - publish and view against a server (run `livekit-server.exe` on the runner, or use Tier 1's server through a tunnel; local is simpler);
  - IPC of the stats lines into the Rust log;
  - `ProcessFailed` handling: kill the `msedgewebview2.exe` renderer by PID, assert the reload and that **voice stays connected** (Rust `lk`-style assertions on the voice participant);
  - single-instance;
  - the software encode path (`encoderImplementation` = OpenH264 expected, which also proves the software fallback exists in WebView2's build).
- **What it can't test:** WGC on real games, NVENC/AMF/QSV, game FPS impact, the profile skip on NVIDIA.

**Tier 3: real GPU, before each release** [cited prices, est. design]:
- **Option A:** a self-hosted Windows box (the owner's or a friend's, with an NVIDIA GPU).
- **Option B:** GitHub `windows_4_core_gpu` (Tesla T4, $0.102/min ≈ $12 for the 2 h soak) — https://docs.github.com/en/billing/reference/actions-runner-pricing ; https://docs.github.com/en/actions/reference/runners/larger-runners [cited]. A T4 is a datacenter card: no session cap and different drivers. Good for "hardware encode is used", not for consumer-session or game-FPS questions [est.].
- **Runs:**
  - Tier 1 assertions plus `impl` = Media Foundation, `hw` = true;
  - the **2-hour soak** (§4.6) on the self-hosted box with a looping game demo or benchmark (e.g. a built-in game benchmark loop) and PresentMon.

---

## 6. Additions to the spike plan (Day 1, Windows)

1. **Profile check first:**
   - Default server config vs `42001f` config on an NVIDIA desktop. Log `encoderImplementation`, `powerEfficientEncoder` and `sdpFmtpLine`. Use `nvidia-smi dmon -s u` / Task Manager "Video Encode" to see NVENC in use.
   - Repeat on an Intel laptop (QSV expected) and on AMD if a friend has one.
2. **Game FPS:** PresentMon/CapFrameX, 3 runs × 60 s each. Off / 720p60 / 1080p30 / 1080p60, sharing the game window vs sharing a browser while gaming. Repeat with Pulse's UI animations frozen vs live, and preview on vs off.
3. **Process overhead on Windows:** Task Manager → Details for each `msedgewebview2.exe` (CPU, private bytes, GPU engine). Compare with §2.6.
4. **Silent-failure checks:**
   - minimise the shared game (confirm no `mute`, and that `IsIconic` detection works);
   - exclusive fullscreen (black detector);
   - Win+L;
   - UAC prompt;
   - `dxcap -forcetdr` or a driver reset (does hardware come back after re-publish?);
   - kill the WebView2 renderer (voice survives; viewers get "interrupted" via the Rust attribute).
5. **Does WebView2 ship OpenH264?** `--disable-features=WebRtcHWEncoding`, then check `encoderImplementation`.
6. **Zero-copy flags experiment:** `--enable-features=WebRtcAllowWgcUsingTexture,ZeroCopyDesktopCapture,MediaFoundationSharedImageEncode` on Windows 11 24H2. Measure game FPS and the browser process's CPU. Only adopt if the gain is large and it is stable over the 2 h soak.
7. **Start the 2-hour soak** at the end of Day 1, overnight, on the NVIDIA desktop.

---

## 7. Sources

All read 2026-10-14 unless noted. Source-code references are to `main`/HEAD on that date unless a version is given.

**Chromium / WebRTC source (code):**
- `content/browser/media/capture/desktop_capture_device.cc`: WGC/DXGI choice, 0 Hz, libyuv convert/scale, CPU limiter, GPU-adapter-changed error — https://chromium.googlesource.com/chromium/src/+/main/content/browser/media/capture/desktop_capture_device.cc
- `media/gpu/windows/media_foundation_video_encode_accelerator_win.cc`: NVIDIA Constrained-Baseline skip, per-adapter MFT enum — https://chromium.googlesource.com/chromium/src/+/main/media/gpu/windows/media_foundation_video_encode_accelerator_win.cc
- `media/webrtc/webrtc_features.cc`, `media/base/media_switches.cc`: feature defaults — https://chromium.googlesource.com/chromium/src/+/main/media/webrtc/webrtc_features.cc ; https://chromium.googlesource.com/chromium/src/+/main/media/base/media_switches.cc
- `third_party/blink/renderer/platform/peerconnection/rtc_video_encoder.cc`: simulcast refusal, software fallback — https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/platform/peerconnection/rtc_video_encoder.cc
- `third_party/blink/renderer/modules/mediastream/video_track_adapter.cc`: mute timer disabled — https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/modules/mediastream/video_track_adapter.cc
- `third_party/blink/renderer/modules/peerconnection/rtc_stats_report.cc`: `ExposeHardwareCapabilityStats` — https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/modules/peerconnection/rtc_stats_report.cc
- `third_party/blink/renderer/modules/mediastream/media_stream_track_impl.cc`: throttling opt-out — https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/modules/mediastream/media_stream_track_impl.cc
- `content/test/data/media/webrtc_test_utilities.js`: `isVideoBlack` — https://chromium.googlesource.com/chromium/src/+/main/content/test/data/media/webrtc_test_utilities.js
- WebRTC `modules/desktop_capture/win/wgc_capture_session.cc`, `screen_capturer_win_directx.cc`, `blank_detector_desktop_capturer_wrapper.cc` — https://webrtc.googlesource.com/src/+/main/modules/desktop_capture/
- Native window occlusion tracking doc — https://chromium.googlesource.com/chromium/src/+/da170c2267b2201954efe1c8fa5d2d89ab09f7bf/docs/windows_native_window_occlusion_tracking.md
- `gpu/config/gpu_driver_bug_list.json` (entry 337, old NVIDIA MFT block) — https://chromium.googlesource.com/chromium/src/+/main/gpu/config/gpu_driver_bug_list.json

**LiveKit:**
- `livekit/protocol` `codecs/codecs.go` — https://github.com/livekit/protocol/blob/main/codecs/codecs.go
- `livekit/livekit` `pkg/rtc/transport.go` (L1042), `pkg/rtc/mediaengine.go` — https://github.com/livekit/livekit/tree/master/pkg/rtc
- `livekit-client` 2.22.4 (npm, read in `node_modules/livekit-client/dist/livekit-client.esm.mjs`): `DefaultReconnectPolicy`, `handleTrackEnded`, `CpuConstrained`, online/offline handling, `DisconnectReason`
- client-sdk-js `src/room/types.ts` (`simulateScenario`), `publishUtils.ts` — https://github.com/livekit/client-sdk-js
- Tokens and refresh — https://docs.livekit.io/frontends/authentication/tokens/ ; connection and `DUPLICATE_IDENTITY` — https://docs.livekit.io/home/client/connect/
- rust-sdks #1277 — https://github.com/livekit/rust-sdks/issues/1277
- livekit-cli README and `cmd/lk/room.go`, `join.go`, `perf.go` — https://github.com/livekit/livekit-cli

**Microsoft:**
- WebView2 process-related events — https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-related-events
- WebView2 IDL (ProcessFailed kinds/reasons, `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`) — https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/webview2-idl
- `ICoreWebView2_3` (`TrySuspend`), `_19` (memory target), `_27` (`ScreenCaptureStarting`), `ICoreWebView2Environment` (`NewBrowserVersionAvailable`) — https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/
- WebView2 browser flags guidance — https://learn.microsoft.com/microsoft-edge/webview2/concepts/webview-features-flags
- `IDXGIAdapter3::QueryVideoMemoryInfo` — https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_4/nf-dxgi1_4-idxgiadapter3-queryvideomemoryinfo
- `GetGuiResources` — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getguiresources ; `PERFORMANCE_INFORMATION` — https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-performance_information
- GPUs in Task Manager — https://devblogs.microsoft.com/directx/gpus-in-the-task-manager/
- Windowed-game optimizations — https://support.microsoft.com/en-us/topic/3f006843-2c7e-4ed0-9a5e-f9389e535952
- `D3DKMTSetProcessSchedulingPriorityClass` — https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/d3dkmthk/nf-d3dkmthk-d3dkmtsetprocessschedulingpriorityclass

**W3C / Chrome:**
- webrtc-stats — https://w3c.github.io/webrtc-stats/ (CRD 2025-09-25: https://www.w3.org/TR/webrtc-stats/)
- mediacapture-screen-share — https://w3c.github.io/mediacapture-screen-share/ ; mediacapture-main (capturing state) — https://w3c.github.io/mediacapture-main/ ; mediacapture-extensions (track stats) — https://w3c.github.io/mediacapture-extensions/
- Minimised window produces no frames — https://lists.w3.org/Archives/Public/public-webrtc-logs/2025Jan/0093.html
- Timer throttling — https://developer.chrome.com/blog/timer-throttling-in-chrome-88
- Chromium switches — https://peter.sh/experiments/chromium-command-line-switches/

**NVIDIA / OBS / vendors / forums:**
- NVENC support matrix — https://developer.nvidia.com/video-encode-and-decode-gpu-support-matrix-new ; NVENC app note 13.0 — https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-application-note/index.html ; NVML clock reasons — https://docs.nvidia.com/deploy/nvml-api/group__nvmlClocksEventReasons.html
- vMix on the session limit 12 — https://blog.vmix.com/?p=4331 ; TechSpot on 3→5 — https://www.techspot.com/community/topics/nvidia-geforce-users-can-now-encode-more-simultaneous-streams.279727/
- NVENC session memory — https://forums.developer.nvidia.com/t/reducing-nvenc-memory-usage/165325
- OBS: NVENC improvements — https://obsproject.com/forum/threads/nvenc-performance-improvements-release-candidate.98950/ ; GPU priority — https://obsproject.com/forum/threads/obs-studio-24-0-3-gpu-priority-fix-testing.111669/ ; `d3d11-subsystem.cpp` — https://github.com/obsproject/obs-studio/blob/master/libobs-d3d11/d3d11-subsystem.cpp ; HAGS — https://obsproject.com/forum/threads/frames-missed-skipped-due-to-rendering-encoding-lag-help.151942/post-582401 and https://obsproject.com/forum/threads/safe-to-re-enable-hags-hardware-accelerated-gpu-scheduling.162780/latest ; FPS anecdotes as in §2.4 ; NVENC out of memory — https://obsproject.com/forum/threads/nvenc-failed-to-start.117094/ ; GPU headroom — https://obsproject.com/forum/threads/low-fps-in-obs-frames-missed-due-to-rendering-lag-gpu-problems-look-here.79146/post-372633
- IgorsLab NVENC vs CPU — https://www.igorslab.de/en/nvidias-nvenc-vs-cpu-encoding-the-turing-video-encoder-for-twitch-streaming-co-comparison-analysis-with-netflix-vmaf/4/ ; PCWorld ShadowPlay — https://www.pcworld.com/article/402765/nvidia-shadowplay-review.html ; LTT ShadowPlay — https://linustechtips.com/topic/812083-performance-impact-of-nvidia-shadowplay-instant-replay-testing/
- Discord engineering — https://discord.com/blog/how-it-all-goes-live-an-overview-of-discords-streaming-technology (2024-03-05) ; https://discord.com/blog/from-blocky-to-brilliant-improving-video-quality-on-discord-go-live-on-amd-gpus (2024-05-01) ; Go Live FPS cap thread — https://support.discord.com/hc/pl/community/posts/360050558792-In-game-FPS-cap-with-Go-Live ; crash/leak thread — https://support.discord.com/hc/en-us/community/posts/360055945392-Crashing-when-streaming-games ; P2 clock — https://www.hothardware.com/news/geforce-memory-clocks-gimped-with-discord-open
- Intel dwm leak — https://www.intel.com/content/www/us/en/support/articles/000058381/graphics.html ; MS Q&A dwm — https://learn.microsoft.com/en-us/answers/questions/2359194/dwm-exe-memory-leak
- Non-paged pool — https://community.intel.com/t5/Wireless/Technical-Bug-Report/m-p/1746123 ; https://woshub.com/huge-memory-usage-non-paged-pool-windows/
- audiodg leak — https://insider.razer.com/razer-synapse-4-55/bug-7-1-surround-rzsurroundapo-dll-leaks-handles-in-audiodg-exe-until-windows-limit-audio-dies-after-10h-92071
- NVIDIA 565 VRAM leak — https://support.keyshot.com/en/knowledge-base/memory-leak-issue-with-nvidia-drivers-565.09-566.03-and-566.14-in-keyshot
- Laptop throttling — https://yomotherboard.com/?p=503682
- WGC pacing on 24H2 — https://github.com/robmikh/Win32CaptureSample/issues/82
- MPO — https://bugzilla.mozilla.org/show_bug.cgi?id=1693130
- jitterBufferTarget for video — https://groups.google.com/a/chromium.org/g/blink-dev/c/4W4orKqA3Rs/m/6k0LdB9wCwAJ ; degradation default — https://lists.w3.org/Archives/Public/public-webrtc-logs/2024Nov/0035.html
- Hybrid-GPU WebView2 — https://www.construct.net/en/forum/construct-3/general-discussion-7/wv2-discrete-gpu-184007 ; GPU preference — https://pureinfotech.com/set-default-gpu-apps-windows-11/
- Jitsi `TrackStreamingStatus` — https://github.com/jitsi/lib-jitsi-meet/blob/master/modules/connectivity/TrackStreamingStatus.ts ; jitsi-meet-torture — https://github.com/jitsi/jitsi-meet-torture
- Tauri single-instance — https://v2.tauri.app/plugin/single-instance/ ; WebDriver — https://v2.tauri.app/develop/tests/webdriver/ ; wry PR #1862 — https://github.com/tauri-apps/wry/pull/1862
- GitHub runners — https://github.com/actions/runner-images (Readme, issues 9538, 14049, 14738, toolset-2025.json) ; pricing — https://docs.github.com/en/billing/reference/actions-runner-pricing ; Playwright WebView2 — https://playwright.dev/docs/webview2
- netem — https://man7.org/linux/man-pages/man8/tc-netem.8.html ; toxiproxy — https://github.com/Shopify/toxiproxy ; clumsy — https://jagt.github.io/clumsy/
