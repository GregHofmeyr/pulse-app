# Screen share deep dive: codecs, encoder settings and layering

**Date:** 2026-10-14. **Scope:** how to get the most picture per bit and per watt for Pulse screen share. Video is
captured and encoded in WebView2 (`getDisplayMedia` + `livekit-client` JS) and viewed in a `<video>` with hardware
decode. The server is LiveKit 1.13.7. Mostly games, sometimes text and code, for about 6 friends in South Africa.
This builds on `screenshare-summary.md`, `screenshare-capture-encode.md`, `screenshare-render.md` and
`screenshare-server-quality.md` next to this file. It does not repeat them.

**Labels:**
- **[measured]** I ran it on 2026-10-14 (setup in §2.1).
- **[cited]** A source says so (URL and the date I read it, which is 2026-10-14 unless noted).
- **[source]** I read the code myself, at the pinned version given.
- **[est.]** My own engineering estimate.
- **[speculation]** An idea that nobody has tested.

**Pinned code read for this report:**
- `livekit-client` **v2.22.4** (commit 21c3b3d, 2026-10-09).
- `livekit` server **v1.13.7** (8d11efd), plus the `livekit/protocol` version its `go.mod` pins.
- Chromium `main` and WebRTC `main` as of 2026-10-14 (WebRTC head commit 421d35b).

Paths written as `CR …` are Chromium source (`chromium.googlesource.com/chromium/src/+/main/…`). Paths written as
`WR …` are WebRTC source (`webrtc.googlesource.com/src/+/main/…`).

---

## 0. TL;DR

1. **H.264 stays the default, but check that it really is hardware.**
   - Chromium's Media Foundation encoder **skips NVIDIA for H.264 Constrained Baseline**
     (`profile-level-id=42e0xx`) because of crbug 1088650 [source: `CR media/gpu/windows/media_foundation_video_encode_accelerator_win.cc`
     L1456-1463].
   - LiveKit 1.13.7 puts Constrained Baseline first in its H.264 preference list [source: `protocol/codecs/codecs.go`
     L177-186; `livekit/pkg/rtc/transport.go` L3287-3322].
   - So on an NVIDIA-only PC, a "hardware H.264" share may silently run on **OpenH264 in software**.
   - The fix is to negotiate H.264 **Main (4d001f)** or High. This is the first thing the spike must check, using
     `getStats().encoderImplementation`.
2. **For games, AV1 and HEVC buy about 20-25% bitrate over H.264 at the same look.** This is [measured] with NVENC on
   four game clips; the gap is biggest at low bitrates (e.g. CS:GO 1080p60 at 4.5 Mbps: H.264 VMAF 60, AV1 72).
   - **But Chromium on Windows does not use hardware AV1 encode for WebRTC by default**: `kWebRtcAV1HWEncode` is
     off on Windows only [cited/source, §1.2].
   - **Edge/WebView2 does not offer H.265 in WebRTC at all** [cited, §1.2].
   - Because Pulse owns the WebView2 command line (`additional_browser_args` in Tauri/wry), we *can* switch hardware
     AV1 on for RTX 40/50, RX 7000+ and Arc sharers. Treat that as an opt-in experiment, not the default.
3. **For text and code, software AV1 with screen-content tools is in a different league** [measured]:
   - At 1080p30 it gives VMAF 95.5 with a worst frame of 87 at about 0.44 Mbps. Hardware H.264 at 1 Mbps gives
     VMAF 95.0 with a worst frame of 67.
   - It costs about 1 core of a fast desktop CPU at 1080p30.
   - **LiveKit's client turns those tools off.** For VP9/AV1 screen share it forces `contentHint='motion'` and L1T3
     [source: `client/src/room/participant/LocalParticipant.ts` L1155-1172]. With the tools off, AV1 text drops to
     VMAF 89.6 at 1 Mbps.
   - Sending text needs that override undone (§5.4).
4. **Never downscale text.** 720p text upscaled to 1080p tops out at **VMAF ~74 / PSNR 31.6 dB whatever the
   bitrate** [measured], against 95-97 at native 1080p. For text, keep resolution and drop the frame rate
   (5-15 fps is fine).
5. **Is 720p60 at 4.5 Mbps H.264 "fine" for games? It depends on the game.**
   - At **4.5 Mbps, 1080p60 beats 720p60** on 3 of 4 clips (GTA V 95.7 vs 92.1; Minecraft 95.1 vs 88.9; Dota 2
     71.1 vs 69.6). The exception is CS:GO, a very detailed fast shooter (60.1 vs 67.8) [measured].
   - So "1080p, and let the encoder drop resolution when it struggles" (`contentHint='motion'` →
     maintain-framerate → Chromium's quality scaler) is a better default than a hard 720p.
   - Discord's free tier is **720p30**; its client's default caps were reported as **2.5 Mbps (app) / 4 Mbps
     (desktop)** [cited, third party, 2023].
6. **Layering:**
   - **No simulcast or SVC for the 6-friend default.** Chromium can't simulcast in one hardware session, so every
     extra layer is another NVENC/AMF/QSV session [cited, §4].
   - LiveKit treats a **single-layer screen share as exempt** from congestion control [source], so a viewer on a bad
     link just gets loss and freezes, not a lower layer.
   - Better: one layer at 1080p plus **one 720p30 simulcast layer only while a viewer on a weak link is watching**
     (dynacast pauses it otherwise). Alternatively, accept that the sharer's encoder follows the sharer's uplink only.
7. **Keyframes cost quality, not just bits** [measured].
   - With NVENC low-latency CBR, a forced mid-stream IDR is only ~2× a normal frame (24 KB at 6 Mbps 1080p60), but
     VMAF drops from ~80 to ~51 and needs ~0.4 s to recover. **Every viewer join makes the picture "pulse" for
     everyone watching.**
   - The *first* keyframe is the big one: 100-250 KB at 1080p60 4.5-12 Mbps.
   - Intra refresh fixes the dip (no big frame, steady quality) but is **not reachable from Chromium**, and LiveKit's
     SFU waits for a real keyframe before it forwards to a new viewer anyway.
8. **The group's real pains (the streamer's game FPS dropping, viewers stuttering) point the same way** (§0.5):
   - Hardware-only encoding. Software encoders take 2-7 cores at 1080p60 [measured].
   - A single layer.
   - **`contentHint='motion'` + `degradationPreference: 'maintain-framerate'` for games.** LiveKit's screen-share
     default, `maintain-resolution`, turns congestion into dropped frames, which is the "choppy" complaint.
9. **Viewer-side upscaling matters more than people expect** [measured].
   - Upscaling a 720p stream to 1080p with bilinear instead of bicubic costs 5-7 VMAF points.
   - NVIDIA RTX Video Super Resolution should apply to WebRTC `<video>` in Chromium whenever the video goes through
     a DirectComposition overlay swap chain [source]. Nobody documents it for WebView2: verify in the spike.
   - A WebGL upscaler (FSR1-style) is a portable fallback [speculation].

---

## 0.5 The group's two real pains: streamer FPS drops and viewer stutter

The group's first-hand experience with Discord is that the two things that hurt were the **streamer's game FPS
dropping** while streaming, and **viewers seeing stutter** (freezes, choppiness, blurry drops), not delay. This
section ranks every choice in this report by those two costs.

### What costs the streamer's game performance (worst first)

| Choice | Cost to the streamer's game | Evidence | Do this instead |
|---|---|---|---|
| **Any software encode of game video** (OpenH264 fallback, VP8, VP9, libaom AV1) | **2-7 CPU cores at 1080p60** on an i9 [measured, §2.4]. On a 6-8 core gaming CPU it eats directly into game frame time, and the encoder still can't hold 60 fps | §2.4 | Hardware H.264 only for games. Treat "software encoder in use" as an error state and tell the user |
| **Silent fallback to OpenH264 on NVIDIA** because Constrained Baseline was negotiated | Same as above, without anyone noticing | [source, §3.2] | Munge the offer to Main/High, and check `encoderImplementation` on every share start |
| **Simulcast layers** | One more hardware session each. Plus, because Chromium feeds screen-share frames to the hardware encoder **through CPU memory (I420 in shared memory, not zero-copy)** when content type is screenshare [source: `CR …/rtc_video_encoder.cc` `IsZeroCopyEnabled` L657-660, L2923-2930], each layer adds a CPU downscale and upload | [source] | Single layer by default. If a weak-viewer layer is added, keep it on-demand (dynacast-paused), at most one |
| **Capturing above what you send** (1440p/4K capture → 1080p send, or 144 Hz capture → 60 fps) | Extra capture copies and a CPU downscale per frame | [est.] | Ask `getDisplayMedia` for the target size and `frameRate: {max: 60}`, so the capturer does the work once |
| **60 fps vs 30 fps** | Roughly 2× the capture, convert and copy work. The hardware encode itself is cheap | [est.] | 60 fps for games (it is the point), 15 fps for text |
| **Hardware AV1 vs hardware H.264** | About the same on the GPU (separate fixed-function block). NVENC is its own engine; AMD VCN and Intel QSV share memory bandwidth with the game | [cited: Tom's Hardware encode speeds; est.] | Fine to offer as opt-in on RTX 40+/RX 7000+/Arc |
| **AV1 software for text** | ~1 core at 1080p30, ~0.5 at 15 fps | [measured] | Only in "text/code" mode, never while a game is the shared window |

**Not measurable here and needs the Windows spike:**
- GPU 3D/compute load from WGC capture and the BGRA→I420 conversion while a game saturates the GPU. Chromium's
  capture path is not zero-copy for screen share.
- Whether `contentHint='motion'` (content type "realtime") switches Chromium to the zero-copy NV12 GPU path. It only
  can if the capturer delivers GPU frames.
- In-game fps with and without the share, 1% lows included.

### What causes viewer stutter, and the settings that make it degrade smoothly instead

| Cause | What viewers see | Fix |
|---|---|---|
| Encoder can't keep up (software fallback, or a CPU-starved capture) | Frame rate collapses, uneven pacing | Hardware only (above) |
| **`degradationPreference: 'maintain-resolution'` on a game.** This is LiveKit's default for screen share [source: `publishUtils.ts` L493-502], and libwebrtc's default for screencast tracks | Under load or low bandwidth, the frame rate drops (5-15 fps). That is exactly the "choppy" complaint | Games: **`contentHint='motion'` + `degradationPreference: 'maintain-framerate'`**. Under pressure the resolution drops (1080p → 720p → 540p) and fps holds. Text: the opposite |
| Encoder overshoot → WebRTC frame dropper | Skipped frames, like stutter. Hardware encoders run with drop threshold 30 (`kWebRTCHardwareVideoEncoderFrameDrop`, on by default) [source: `rtc_video_encoder.cc` L304-312; `media_switches.cc` L1088]. Discord saw this halve fps on AMD [cited, 2024-05-01] | Keep a ~1 s rate buffer, no hard per-frame caps. On AMD watch `framesEncoded` vs `framesSent` in the spike |
| Packet loss on a viewer's link with a single-layer stream (allocator-exempt) | Freeze until NACK/PLI recovers, then a keyframe "pulse" | Fix loss at the server (16 MiB UDP buffers, 2 OCPU). Optional on-demand low layer for the one weak viewer |
| Keyframe on every join or PLI | ~0.3-0.5 s of blur for all viewers [measured, §6.1] | Join-to-watch only; keep `pli_throttle` ≥500 ms |
| Bitrate starting low (BWE ramp) | First seconds blurry | LiveKit already sets `x-google-start-bitrate` to 90% of the screen-share cap [source]; check H.264 (issue #1816) |
| The streamer's own uplink saturating (game traffic, other uploads) | Everyone stutters together | Cap at ~70% of measured uplink; prefer 720p60 at 5-6 Mbps on uplinks below 12 Mbps |

**Net effect on the recommendations:**
- Hardware H.264 (Main/High), single layer, `motion` + `maintain-framerate` for games.
- No software encoders anywhere near a game.
- No simulcast unless a specific weak viewer needs it.
- These choices are reflected in §3, §4, §5 and §7.

---

## 1. What Chromium in WebView2 actually does on Windows

### 1.1 Decode (viewer side)

| Codec | Hardware decode in Chromium on Windows | Notes |
|---|---|---|
| H.264 | Yes (D3D11, all GPUs since ~2012) | Edge's default H.264 path is the MF "DXVA VDA" decoder; D3D11VideoDecoder is the alternative [cited: [MS Learn, Edge video playback issues](https://learn.microsoft.com/en-us/troubleshoot/microsoft-edge/development/video-playback-issues), page dated 2026-03-31] |
| VP8 | **No** (software libvpx) | There is no VP8 entry in `CR media/gpu/windows/supported_profile_helpers.cc` [source] |
| VP9 profile 0/2 | Yes where the GPU has it | k-SVC hardware decode is off by default (`kD3D11Vp9kSVCHWDecoding`) [source] |
| AV1 | Yes on RTX 30+, RX 6000+ (not 6500/6400), Intel Xe (11th gen+) and Arc | **AV1 with spatial layers always falls back to software** ("No hardware decoder supports AV1 SVC stream") [source: `CR …/rtc_video_decoder_adapter.cc`]. L1T3 (temporal only) is fine |
| HEVC | Chrome yes; **Edge goes through the Windows MF path and needs the HEVC Video Extension** [cited: MS Learn above] | WebRTC has **no software fallback** for HEVC [source] |

The 1080p60 software decode cost was not measured here. The earlier render report has Chromium WebCodecs software
throughput on this i9: H.264 991 fps, VP9 421 fps, AV1 729 fps at 1080p.

### 1.2 Encode (sharer side)

Chromium's hardware encoder on Windows is `MediaFoundationVideoEncodeAccelerator`. On top of it sits the WebRTC
encoder factory in Blink.

| Codec | Hardware encode used by WebRTC on Windows today? | Gate | Software fallback |
|---|---|---|---|
| **H.264** | **Yes**, through MF (NVENC/AMF/QSV MFTs) | Only offered if Chromium's OpenH264 software encoder is also built in (`UseH264AcceleratedEncoderForWebRTC()` returns `IsOpenH264SoftwareEncoderEnabledForWebRTC()`) [source: `CR …/peerconnection/webrtc_util.cc`]. **NVIDIA is skipped for Constrained Baseline** [source, L1456-1463] | OpenH264 (Constrained Baseline, CAVLC) |
| VP8 | No hardware on Windows | none | libvpx |
| VP9 | MF enumerates it (Intel Ice Lake+, AMD RDNA2+); **NVIDIA has no VP9 encoder at all** [cited: [NVIDIA support matrix](https://developer.nvidia.com/video-encode-and-decode-gpu-support-matrix-new), undated page] | Intel drivers before Ice Lake are blocked (bug-list 412) [source: `CR gpu/config/gpu_driver_bug_list.json`] | libvpx |
| **AV1** | **No by default.** `kWebRtcAV1HWEncode` is `FEATURE_DISABLED_BY_DEFAULT` under `IS_WIN` and enabled on every other platform [source: `CR media/webrtc/webrtc_features.cc`; `rtc_video_encoder_factory.cc`] | `--enable-features=WebRtcAV1HWEncode` | **libaom** (realtime, speed 10; speed 11 for screen content) |
| **H.265** | Chromium: yes since M136 (`kWebRtcAllowH265Send`) [cited: chromestatus 5153479456456704]. **Edge (and so WebView2): not advertised.** An MS moderator says Edge has not enabled the H.265 send/receive features, and command-line flags did not help the asker (posted 2026-05-04, follow-up 2026-09-09) [cited: [MS Q&A 5880331](https://learn.microsoft.com/en-ca/answers/questions/5880331/)] | — | none |

Other encoder facts that matter for quality:

- **Rate control.**
  - For H.264 in CBR, Chromium by default runs its **own software rate controller** and drives the hardware MFT in
    "Quality" mode with a per-frame QP (`kMediaFoundationUseSWBRCForH264Desktop`, `…Camera`) [source: MF encoder
    `SetSWRateControl` L1640-1735].
  - **On AMD this is turned off** (`disable_h264_accelerator_sw_brc`, crbug 417752242), so AMD uses the driver's own
    CBR [source].
  - That matters because AMD's H.264 rate control is the weakest. Discord had to work around it [cited: [Discord,
    "From Blocky to Brilliant"](https://discord.com/blog/from-blocky-to-brilliant-improving-video-quality-on-discord-go-live-on-amd-gpus), 2024-05-01].
  - Tom's Hardware's 2023 tests found AMD H.264 still last behind NVIDIA (Pascal onward) and Intel Arc, with RDNA2/3
    improving mainly at low bitrates [cited: [Tom's Hardware](https://www.tomshardware.com/news/amd-intel-nvidia-video-encoding-performance-quality-tested), 2023-03-10].
- **Screen-content tuning on the hardware path:**
  - Only **Intel** gets `CODECAPI_AVScenarioInfo = eAVScenarioInfo_DisplayRemoting` for display content. The code
    comment says this turns on AV1 screen-content tools (palette, intra block copy) and long-term references. A TODO
    says other vendors aren't enabled yet [source: MF encoder L1864-1877].
  - **NVIDIA and AMD hardware encoders get no screen-content tuning from Chromium.**
- **GOP.** The MF encoder's default GOP is 3000 frames (`kDefaultGOPLength`) [source, L75]. In practice WebRTC
  keyframes come from PLI/FIR (§6).
- **Temporal layers on hardware.**
  - H.264 can do up to L1T3 if the driver reports it, but the software rate controller handles at most 2.
  - VP9 and AV1 are limited to 1 layer unless flags are on.
  - HEVC on NVIDIA is limited to 1 [source: `CR media/gpu/windows/mf_video_encoder_util.cc` `GetMaxTemporalLayerVendorLimit`].
- **Simulcast.** "No VEA currently supports simulcast" [source: `CR …/peerconnection/rtc_video_encoder.cc`]. libwebrtc
  then builds one encoder per layer, so **each simulcast layer is a separate hardware session** [inferred].
- **D3D12 encoder.** There is a newer one (`kD3D12VideoEncodeAccelerator`), but it is **off by default**. It is
  also blocked on AMD and on Windows older than 11 24H2 [source: `CR media/base/media_switches.cc`, bug list
  461/470].

### 1.3 Consumer GPU matrix (2016 onward)

[cited: [NVIDIA support matrix](https://developer.nvidia.com/video-encode-and-decode-gpu-support-matrix-new) (undated);
[Wikipedia, Video Core Next](https://en.wikipedia.org/wiki/Video_Core_Next); [Intel media-driver features](https://raw.githubusercontent.com/intel/media-driver/master/docs/media_features.md).
The AMD rows rely on the Wikipedia prose because its tables were garbled.]

| GPU family | HW encode | HW decode | What Pulse gets in WebView2 WebRTC today |
|---|---|---|---|
| NVIDIA Pascal (GTX 10) | H.264, HEVC | H.264, HEVC, VP9 (most) | H.264 HW **only if not Constrained Baseline**. Note: GT 1030 has no NVENC |
| NVIDIA Turing (GTX 16 / RTX 20) | H.264, HEVC | + VP9 | same |
| NVIDIA Ampere (RTX 30) | H.264, HEVC | + **AV1** | same; can *view* AV1 in hardware |
| NVIDIA Ada (RTX 40) | H.264, HEVC, **AV1** | all | H.264 HW; AV1 HW only with the flag |
| NVIDIA Blackwell (RTX 50) | H.264, HEVC, AV1 (+4:2:2) | all | same as Ada |
| AMD Polaris/Vega (RX 400/500, Vega) | H.264, HEVC (VCE) | H.264, HEVC; VP9 uncertain | H.264 HW, weakest quality |
| AMD RDNA1 (RX 5000) | H.264, HEVC | + VP9 | same |
| AMD RDNA2 (RX 6000) | H.264, HEVC (**no AV1 encode**; RX 6500 XT/6400 reportedly no encode at all) | + AV1 | H.264 HW |
| AMD RDNA3/RDNA4 (RX 7000/9000) | H.264, HEVC, **AV1** | all | H.264 HW; AV1 HW only with the flag |
| Intel Gen9.5 (UHD 620/630) | H.264, HEVC | H.264, HEVC, VP9 | H.264 HW (VP9 encode blocked before Ice Lake) |
| Intel Xe-LP (11th-14th gen iGPU, UHD 770) | H.264, HEVC, VP9 | + AV1 | H.264 HW, and VP9 HW |
| Intel Arc A / Meteor Lake | H.264, HEVC, VP9, **AV1** | all | H.264 HW; AV1 HW only with the flag (and Intel *does* get Chromium's SCC scenario hint) |
| Intel Lunar Lake / Battlemage | H.264, HEVC, AV1 (no VP9 encode) | all | same |

**NVENC session limit on GeForce:** NVIDIA's support matrix says 12 per card. The NVENC application note for SDK 13.0
says 8 per system [cited: [NVENC app note](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-application-note/index.html)].
Either way, OBS, ShadowPlay/Instant Replay, Discord and Pulse simulcast layers all count against it.

---

## 2. Quality per bit: measurements

### 2.1 Setup

**Hardware:** this Linux dev box, an i9-13900HX with an RTX 4070 Laptop (Ada NVENC) and the CPU's Intel UHD iGPU
(Xe-LP QSV). FFmpeg n9.0.2 with libvmaf.

**Encoders, configured the way real-time WebRTC runs them:**
- CBR with a 1-second rate buffer. WebRTC's libvpx settings use `rc_buf_sz = 1000` ms [source: `WR …/libvpx_vp9_encoder.cc`
  L678-690].
- No B-frames and an infinite GOP.
- Per encoder:
  - NVENC `p4 -tune ll`, H.264 High (or Baseline for "nv264cb").
  - QSV low-power, `veryfast`.
  - libvpx VP8 `cpu-used 6`, VP9 `cpu-used 7`, `aq-mode 3`, with screen-content mode for text.
  - libaom realtime `cpu-used 10`. For text: `cpu-used 11 tune-content=screen enable-palette=1`, which is exactly
    what WebRTC sets for screen sharing [source: `WR …/libaom_av1_encoder.cc` L392-422, L476-490].
  - OpenH264 built from source (commit 1bca623) with WebRTC's parameters. It was built without assembly, so its
    CPU numbers are not meaningful.

**Clips:**
- Four 4-second 1080p60 slices from the public-domain Xiph "twitch" set, frames 1800-2039
  [cited: [media.xiph.org/video/derf/twitch](https://media.xiph.org/video/derf/twitch/)]:
  - **CS:GO**: fast shooter, fine textures; the hardest.
  - **Dota 2**: busy top-down view with lots of UI.
  - **Minecraft**: blocky, camera motion.
  - **GTA V**: plane at sunset, smooth gradients; the easiest.
- One synthetic **code-editor** clip, 1080p30, 8 s. A VS Code-like layout with 11pt monospace and syntax colours:
  2 s static, 2 s smooth scroll at 360 px/s, 2 s static, then 2 s of page jumps every 0.5 s.

**Metric:**
- VMAF (v0.6.1 1080p model), PSNR-Y and SSIM, always **at 1080p display size**. A 720p encode is upscaled with
  bicubic before scoring, which models a viewer watching fullscreen on a 1080p monitor.
- Frame rates are compared at their own rate. VMAF does not punish 30 fps against 60 fps, so only compare like
  with like.

**How to read VMAF:** ≥95 is about indistinguishable; 90 is very good; 80 is good; 70 is "fair", with visible
artefacts on detail; <60 is poor [cited: Tom's Hardware, above].

**Caveats:**
- Linux NVENC/QSV through FFmpeg is not the same as Chromium's MF path on Windows. Chromium drives the MFT with its
  own per-frame QP controller, so absolute numbers will differ.
- Comparisons *between codecs on the same hardware* should hold.
- The 4 s clips mean these are rate-control-limited results, not steady-state ones.
- NVENC CBR pads up to the target, so for NVENC "actual bitrate" equals the target.

Scripts and raw CSV: `/tmp/claude-1000/screenshare-codec/` (`run.py`, `oh.py`, `analyze.py`, `results.csv`). Not committed.

### 2.2 Games: VMAF by codec and bitrate [measured]

**1080p60** (target Mbps across the top):

| Clip | Encoder | 2.5 | 4.5 | 6 | 8 | 12 |
|---|---|---|---|---|---|---|
| CS:GO | NVENC H.264 | 37.7 | 60.1 | 70.7 | 79.6 | 89.5 |
| | NVENC HEVC | 55.9 | 72.1 | 78.6 | 84.4 | 91.4 |
| | NVENC AV1 | 56.1 | 72.1 | 79.1 | 85.3 | 92.3 |
| | Intel QSV H.264 | 33.0 | 53.8 | 64.3 | 74.2 | 84.5 |
| Dota 2 | NVENC H.264 | 53.5 | 71.1 | 77.7 | 83.2 | 88.6 |
| | NVENC AV1 | 62.2 | 75.8 | 81.2 | 85.2 | 89.4 |
| | Intel QSV H.264 | 43.9 | 63.7 | 71.9 | 78.6 | 85.5 |
| Minecraft | NVENC H.264 | 79.7 | 95.1 | 98.0 | 99.1 | 99.5 |
| | NVENC AV1 | 94.0 | 98.5 | 99.2 | 99.4 | 99.6 |
| | Intel QSV H.264 | 66.8 | 91.2 | 96.0 | 98.3 | 99.3 |
| GTA V | NVENC H.264 | 93.1 | 95.7 | 96.7 | 97.1 | 97.3 |
| | NVENC AV1 | 95.3 | 96.3 | 96.6 | 96.8 | 97.2 |

**720p60, viewed at 1080p:**

| Clip | Encoder | 1.5 | 2.5 | 3.5 | 4.5 | 6 |
|---|---|---|---|---|---|---|
| CS:GO | NVENC H.264 | 32.3 | 48.9 | 60.3 | **67.8** | 76.0 |
| | NVENC AV1 | 42.1 | 56.0 | 66.4 | 72.6 | 79.4 |
| Dota 2 | NVENC H.264 | 40.5 | 55.8 | 64.5 | **69.6** | 74.8 |
| | NVENC AV1 | 46.3 | 59.4 | 66.9 | 71.3 | 76.5 |
| Minecraft | NVENC H.264 | 66.3 | 81.4 | 86.3 | **88.9** | 90.8 |
| GTA V | NVENC H.264 | 87.7 | 90.9 | 91.5 | **92.1** | 92.4 |

**Bitrate needed for VMAF 80, interpolated, 1080p60:**

| Clip | NVENC H.264 | Intel QSV H.264 | NVENC HEVC | NVENC AV1 |
|---|---|---|---|---|
| CS:GO | 8.2 Mbps | 9.8 | 6.5 | 6.6 |
| Dota 2 | 7.3 | 9.0 | 7.2 | 6.1 |

**What this says:**
- **AV1 ≈ HEVC > H.264.** The gap is 10-18 VMAF points at 2.5 Mbps and shrinks to 1-3 points at 12 Mbps. In
  bitrate terms, AV1/HEVC need **~20-25% fewer bits** for the same score on hard content. That matches Tom's
  Hardware, where Ada AV1 was about 2 points above HEVC [cited]. It falls short of NVIDIA's own "up to 40%"
  claim [cited: [NVIDIA, Discord AV1](https://www.nvidia.com/en-us/geforce/news/nvidia-studio-discord-av1/),
  2023-02-24].
- **Intel iGPU H.264 is 5-10 points behind NVENC** at the same bitrate. On the iGPU, HEVC closes most of that gap.
- **720p vs 1080p at the same bitrate depends on content.** At 4.5 Mbps H.264, 1080p wins on GTA V, Minecraft and
  Dota 2. 720p wins only on CS:GO (67.8 vs 60.1).
  - 720p has a **ceiling** when upscaled: about 92 on GTA V and about 91 on Minecraft, however many bits you spend.
  - 1080p has a **floor**: it collapses on CS:GO below ~6 Mbps.
  - This is exactly the trade-off Chromium's quality scaler makes for camera-like tracks: start at full resolution
    and drop it when the QP gets too high. It argues for `contentHint='motion'` with a 1080p source rather than a
    fixed 720p (§5).
- **The worst frame matters for games too.** At 1080p60 6 Mbps, CS:GO's worst frame was 61 (H.264) or 67 (AV1);
  Dota 2's was 70 or 69.

### 2.3 Text and code: VMAF / worst frame / PSNR-Y by codec [measured]

1080p30 code-editor clip. Columns are target Mbps; cells are "mean / worst frame". The actual average bitrate is
often below the target, because static text is cheap.

| Encoder | 0.3 | 0.6 | 1.0 | 2.0 | 4.0 | Encoder CPU (cores, i9-13900HX) |
|---|---|---|---|---|---|---|
| NVENC H.264 High | 81.7 / 11 | 90.8 / 27 | 95.0 / 67 | 97.0 / 84 | 97.6 / 92 | ~0.1 |
| NVENC H.264 Baseline | 78.8 / 24 | 88.6 / 37 | 94.4 / 67 | 96.8 / 84 | 97.4 / 91 | ~0.1 |
| Intel QSV H.264 | 90.2* / 37 | 92.6* / 58 | 92.9 / 58 | 96.8 / 84 | 97.7 / 96 | ~0.15 |
| NVENC HEVC | 82.5 / 10 | 90.9 / 23 | 94.8 / 56 | 96.7 / 78 | 97.5 / 88 | ~0.1 |
| NVENC AV1 | 85.6 / 0 | 94.2 / 17 | 96.2 / 55 | 97.3 / 77 | 97.6 / 89 | ~0.1 |
| **OpenH264 (WebRTC's software H.264), screen mode** | — | — | 91.2 / 83 (0.44 Mbps actual) | 91.9 / 86 (0.41 actual) | — | n/a (no-asm build) |
| libvpx VP8, screen mode | 75.1* / 38 | 76.0* / 38 | 86.9 / 40 | 95.3 / 69 | 97.0 / 86 | 1.0-3.0 |
| libvpx VP9, screen mode | 75.3 / 14 | 89.1 / 63 | 93.8 / 78 | 96.5 / 88 | 97.2 / 93 | 0.7-0.95 |
| **libaom AV1, screen mode (what WebRTC does for screen share)** | **95.5 / 87** (0.44 actual) | **97.1 / 92** | **97.3 / 94** | 97.6 / 96 | 97.8 / 97 | **1.0-1.3** |
| libaom AV1, *no* screen tools (what LiveKit's forced `motion` hint gives) | 65.4 / 1 | 84.4 / 43 | 89.6 / 53 | 94.8 / 79 | 96.8 / 94 | 0.9-1.1 |

\* The encoder overshot its target, to 0.75-1.0 Mbps.

**Same clip at 720p30, upscaled to 1080p:**

| Bitrate (Mbps) | NVENC H.264 | NVENC AV1 | libaom AV1, screen mode |
|---|---|---|---|
| 0.6 | 71.7 | 73.1 | 73.9 |
| 1.0 | 73.4 | 74.0 | 74.3 |
| 2.0 | 74.1 | 74.5 | 74.5 |

PSNR-Y is stuck at ~31.6 dB in every case. **Resolution, not bitrate, is the limit.**

**Lower frame rate, 1080p:**

| Encoder | 1080p15 at 0.6 Mbps | 1080p5 at 1 Mbps |
|---|---|---|
| NVENC H.264 | 92.0 / 65 | 96.0 / 92 |
| libaom AV1 screen | 95.9 / 92 | — |

At 5 fps H.264's worst frame climbs from 67 to 92 at the same 1 Mbps.

**What this says:**
- **AV1 with screen-content tools is roughly 2-3× as bit-efficient as anything else on text**, and it has by far the
  best worst-case frame. That matches the ">25%" on slides that webrtcHacks measured with `contentHint` [cited: [webrtcHacks](https://webrtchacks.com/the-hidden-av1-gift-in-google-meet/),
  2023-12-19], Chrome's "45% better compression than VP9" for AV1 speed 10 [cited: [Chrome blog](https://developer.chrome.com/blog/av1),
  updated 2023-05-02], and Jitsi's "AV1 is the best codec for screen sharing on all platforms" [cited: [Jitsi](https://jitsi.org/blog/av1-and-more-how-does-jitsi-meet-pick-video-codecs/),
  2024-12-17].
- **The tools, not the codec, do the work.** AV1 without them is *worse* than hardware H.264.
- **Hardware encoders are fine for text from ~2 Mbps** at 1080p30 (VMAF ≥96, worst frame ≥77). Below 1 Mbps, scroll
  and page-change frames break up.
- **OpenH264's screen mode is quality-capped.** Its defaults for screen content are min QP 26 / max QP 35 [cited:
  `h264enc -h`]. It undershoots (0.4 Mbps against a 2 Mbps target) and never gets sharper than ~91 / 44 dB. That
  is the "blurry text" floor if the H.264 path falls back to software.
- **Text needs native resolution.** Every 720p row is about 74.

### 2.4 Software encoder CPU at 1080p60 game content [measured]

These are the paths Chromium falls back to when hardware isn't used. "Cores" means CPU-seconds per second of video on
an i9-13900HX (24 cores) through FFmpeg's threading. libwebrtc uses fewer threads, so on a real sharer these
encoders mostly **can't hold 60 fps at all**, and WebRTC's CPU-overuse detector then cuts resolution or frame rate.

| 1080p60 | VMAF at 2.5 / 4.5 / 8 Mbps, CS:GO | Same, Dota 2 | CPU cores (CS:GO / Dota 2) | Rate control |
|---|---|---|---|---|
| NVENC H.264 High (hardware) | 37.7 / 60.1 / 79.6 | 53.5 / 71.1 / 83.2 | ~0.3 (FFmpeg overhead; the encode is on the GPU) | on target |
| NVENC H.264 **Baseline** (hardware) | 29.3 / 50.5 / 71.8 | 47.7 / 66.3 / 80.2 | ~0.3 | on target |
| OpenH264 (WebRTC's software H.264) | 45.5 / 45.8 / 57.0 | 47.0 / 52.4 / 67.5 | n/a (no-asm build) | **overshoots: 5.7 Mbps when asked for 2.5** |
| x264 veryfast zerolatency (reference only) | 23.9 / 40.4 / 64.7 | 30.5 / 54.8 / 73.3 | 2.7-3.2 / 1.6-2.2 | on target |
| libvpx VP8 (cpu-used 6) | 54.7 / 54.9 / 66.3 | 54.6 / 55.6 / 72.2 | 3.4-3.9 / 2.3-2.8 | **overshoots: 5.5 Mbps when asked for 2.5** |
| libvpx VP9 (cpu-used 7) | 40.2 / 59.5 / 76.0 | 56.7 / 70.8 / 81.8 | 4.2-5.4 / 2.9-4.2 | on target |
| libaom AV1 (speed 10) | 45.1 / 62.2 / 77.5 | 61.5 / 74.7 / 83.4 | **5.1-6.7 / 3.5-3.8** | on target |

At 720p60 the software encoders still take 1.1-4.5 cores.

**What this says:**
- **Software encoding of a 60 fps game costs several CPU cores**, the same cores the game wants. A fast-motion
  game is the worst case.
- **H.264 Baseline costs 5-10 VMAF points against High** on the same hardware encoder. OpenH264 software Baseline is
  worse again, and blows through its bitrate. In WebRTC, overshoot becomes frame drops (stutter).
- **No software path is acceptable for games.** Software AV1/VP9 are only reasonable for text at 5-15 fps.

---

## 3. Codec strategy

### 3.1 Recommendation

1. **Default codec: H.264, hardware, Main or High profile.**
   - It is the only codec with hardware encode on every friend's GPU from 2016 onward, and hardware decode
     everywhere.
   - **Make sure Constrained Baseline is not what gets negotiated** (§3.2).
   - Verify on every machine with `RTCOutboundRtpStreamStats.encoderImplementation` (expect a MediaFoundation
     name, not "OpenH264") and `powerEfficientEncoder: true`.
2. **Text/code mode: AV1 (libaom software), with screen-content tools kept on.**
   - Use 1080p (or native up to 1440p) at 5-15 fps, cap 1-1.5 Mbps.
   - About 1 core at 30 fps, ~0.5 core at 15 fps [measured, fast desktop CPU; laptops maybe 2×].
   - Viewers need AV1 decode: hardware on RTX 30+, RX 6000+, Intel 11th gen+; dav1d software otherwise, which is
     cheap at 15 fps.
   - **Fallback:** H.264 hardware at 1080p 15 fps, 2 Mbps.
3. **Game mode, opt-in "efficient" toggle: AV1 hardware** on RTX 40/50, RX 7000/9000 and Arc sharers.
   - Launch WebView2 with `--enable-features=WebRtcAV1HWEncode` (§3.3).
   - It is worth about 1.5-2 Mbps at 1080p60 for the same look, or noticeably cleaner fast motion at the same
     bitrate.
   - Gate it on every *current* viewer having AV1 decode (`RTCRtpReceiver.getCapabilities('video')` plus
     `navigator.mediaCapabilities.decodingInfo({type:'webrtc', …})` reporting `powerEfficient`), the way Discord does.
   - If a weak viewer joins, fall back to H.264 (§3.4).
4. **Skip VP8 and VP9.**
   - VP8 is software-only on both ends in Chromium on Windows: the worst picture per watt.
   - VP9 has no NVIDIA encoder, no gain over AV1, and its hardware SVC paths are flag-gated.
   - LiveKit's default `videoCodec` is `'vp8'` [source: `client/src/room/defaults.ts` L10-23], so **set the codec
     explicitly**.
5. **Skip HEVC for now.** Edge/WebView2 doesn't offer it in WebRTC [cited], and LiveKit's backup codec can only be
   VP8 or H.264.

### 3.2 The NVIDIA Constrained Baseline trap (most important practical finding)

**The chain:**
- The MF encoder skips NVIDIA MFTs when `is_constrained_h264` is set [source, L1456-1463].
- Chromium's WebRTC factory advertises Baseline, Main and High, plus a Constrained Baseline copy of each Baseline
  [source: `rtc_video_encoder_factory.cc` L108-135, L286-306].
- The LiveKit server orders its publisher-side H.264 preferences with **42e01f (Constrained Baseline) first**, then
  High (640032), then 42001f and 4d001f. It applies them with `SetCodecPreferences` on the answer
  [source: `protocol/codecs/codecs.go` L177-186; `livekit/pkg/rtc/mediaengine.go` L61-85; `livekit/pkg/rtc/participant_sdp.go`
  L193-246; `transport.go` L3287-3322].

**Likely result [inferred]:** a Chromium publisher negotiates 42e01f. On an NVIDIA desktop the MF encoder then
skips NVENC and falls back to:
- the Intel iGPU's MFT, if one is active (common on laptops), or
- **OpenH264 in software**. That is CPU-heavy at 1080p60 while gaming, and Baseline-only quality.

At the default `cpu-used`-equivalent speed, OpenH264 runs in the tens of fps at 1080p on a fast core [est.].

**Fixes, in order of preference:**
1. **Spike first:** read `encoderImplementation` on an NVIDIA machine. If it says MediaFoundation, the problem
   doesn't exist (for example, if 42001f or Main was actually picked).
2. **Munge the publisher's offer** in the WebView before it is sent (wrap `RTCPeerConnection.prototype.createOffer`
   / `setLocalDescription` in our page). Drop 42e01f and 42001f from the H.264 payloads, or move `4d001f` (Main) to
   the front. Main is in LiveKit's registered list. The server forwards the publisher's profile unchanged, and
   Chromium decoders handle Main/High [source: `transport.go` L345-352, where the server filters High from the
   *subscriber* offer only because browsers mis-signal it].
3. Ask upstream (LiveKit) for a publish option that orders H.264 profiles.

### 3.3 Turning Chromium features on in WebView2

- **The lever:** wry passes WebView2's `AdditionalBrowserArguments`. Tauri exposes it as
  `WebviewWindowBuilder::additional_browser_args` [source: `tauri-2.12.1/src/webview/webview_window.rs` L1080;
  `wry-0.57.0/src/webview2/mod.rs` L300-333].
- **Gotcha:** setting it **replaces** wry's default `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`
  [source: `wry-0.57.0/src/lib.rs` L1788], so keep those.
- **Gotcha:** combine everything into a single `--enable-features=` and a single `--disable-features=`.
- **Gotcha:** all WebView2 instances that share a user-data folder must use identical arguments.
- **Candidate flags:**
  - `WebRtcAV1HWEncode`: hardware AV1 send.
  - `MediaFoundationVP9L1T3Support` / `MediaFoundationAV1L1T3Support`: hardware temporal layers. Not needed if we go
    single-layer.
  - `--force-fieldtrials=…`: libwebrtc field trials, e.g. `WebRTC-Video-BalancedDegradation/Enabled/`.
- **Risk:** flags are unsupported in Edge and can disappear in any WebView2 Evergreen update. Wrap every feature
  in a runtime check (`encoderImplementation`, codec capabilities) with a fallback.

### 3.4 Per-viewer negotiation and `backupCodec`

**How LiveKit does it** [source: client `LocalParticipant.ts` L1186-1206, L2064-2084; server `subscribedtrack.go`
L456-480, `mediatrack.go` L110-123, L511-566]:
- `backupCodec` can be `'vp8'` or `'h264'` and defaults to VP8. The backup is **encoded only on demand**: when a
  subscriber that can't decode the primary subscribes, the server asks the publisher to start it.
- It forces dynacast on.
- For screen share the backup is published without simulcast.
- **Regression is one-way and room-wide.** With the default policy (`PREFER_REGRESSION`) the primary is suspended
  for everyone once the backup starts, and it never comes back for that publication.

**Recommendation:**
- AV1 primary + `backupCodec: {codec: 'h264'}` + `backupCodecPolicy: REGRESSION` is fine *if* we accept that one
  weak viewer downgrades the stream for everyone until the share restarts. That is the same as Discord: "renegotiated
  when someone joins or leaves", "all viewers can decode" [cited in server-quality report].
- With 6 friends, simpler is better: **choose the codec before publishing** from the known viewer capabilities. The
  app knows who is in the voice channel, and Pulse can share each client's decode capabilities through its own
  server. Restart the publication (a 0.3-0.6 s blip) if an incompatible viewer clicks Watch.
- Keep `backupCodec: {codec: 'h264'}` as the safety net, never VP8 (software decode on every Windows viewer).
- Note that during regression the backup is encoded **in addition**: two hardware sessions, or AV1 software plus H.264
  hardware.

---

## 4. Layering: simulcast vs SVC

### 4.1 What the stack supports

| | Simulcast (separate encodes) | SVC L1T3 (temporal only) | SVC L3T3_KEY (spatial + temporal) |
|---|---|---|---|
| Codecs in LiveKit 1.13.7 | VP8, H.264, H.265; VP9/AV1 as "rid simulcast" since client 2.22.3 with server >1.13.6 | VP9, AV1 (AV1 needs the dependency-descriptor extension, which LiveKit negotiates) [source: `forwarder.go` L382-455] | VP9, AV1 |
| Chromium hardware on Windows | One hardware session **per layer** | H.264 yes (up to T3, driver-dependent); AV1/VP9 flag-gated | No hardware encoder; libvpx/libaom software |
| LiveKit client for screen share | Default: one extra layer at ½ resolution and the same fps, bitrate max(150k, ¼) [source: `publishUtils.ts` L56-75, L213-216] | **Forced** for VP9/AV1 screen share, together with `contentHint='motion'` [source] | Not used for screen share |
| Server stream allocator | Can move a viewer between layers | **A single-spatial-layer screen share is "exempt"**: always given its optimal allocation, never downgraded or paused [source: `streamallocator/track.go` L112-114; `streamallocator.go` L1174-1205] [inferred: so temporal layers are not dropped for congestion] | Can switch layers |
| Dynacast | Pauses individual layers | Whole stream on/off only | Whole stream only |
| Viewer hardware decode | Yes | Yes | **AV1 spatial SVC → software decode** [source] |

### 4.2 What layering buys and costs here

- **Costs on the sharer:**
  - Each simulcast layer is another encoder session and another 15-30% of uplink.
  - For hardware that's cheap in watts but uses NVENC sessions.
  - For software AV1/VP9 SVC it is the whole encode cost again.
- **What it buys viewers on bad links:**
  - Only simulcast gets a viewer a lower layer. The allocator moves a congested viewer down a simulcast layer.
  - With a single layer, or L1T3 (exempt), a viewer whose downlink can't take the stream just sees loss → NACK/PLI →
    freezes.
  - **The sharer's encoder never hears about it.** In an SFU the publisher's congestion control measures the uplink
    to the server, not each viewer's downlink. Discord instead holds every viewer to "the slowest viewer's
    connection" [cited in server-quality report].
- **Temporal layers (L1T3):** even when they do work, they only let the SFU drop to 30 or 15 fps. That costs the
  encoder ~5-10% efficiency [est.], and the gain is small for 6 people on fibre.

### 4.3 Recommendation for 6 friends

1. **Default: a single layer.** H.264 hardware, `simulcast: false`. No temporal layers.
2. **Weak-viewer layer, on demand [est.]:**
   - Publish with `simulcast: true` and **exactly one** low layer (`screenShareSimulcastLayers: [720p30 @ ~2.5 Mbps]`
     for a 1080p60 main layer).
   - Rely on dynacast: the server pauses the low layer while no viewer needs it, after the 5 s
     `dynacast_pause_delay`, and the client sets `encoding.active=false` [source: `LocalVideoTrack.ts` L503-551].
   - So it costs nothing until someone on LTE or bad Wi-Fi watches.
   - Two NVENC sessions are well inside the limit. **But while the low layer is active, the streamer pays a CPU
     downscale and copy per frame** (screen-share input is not zero-copy, §0.5). So keep it truly on-demand.
   - **Spike check:** that an inactive encoding really frees its MF session (or at least costs nothing), and that the
     layer switch needs only one PLI.
3. **Don't use SVC for screen share** in this stack:
   - Hardware support on Windows is flag-gated.
   - LiveKit forces `motion` (which loses text quality).
   - The server treats the single-spatial-layer stream as exempt anyway.
4. Leave `adaptiveStream` off for the fullscreen viewer, as the earlier reports say. Note that `setVideoQuality(HIGH)`
   is still capped by adaptive-stream element dimensions when adaptive stream is on [source: `RemoteTrackPublication.ts`
   L320-357].

---

## 5. Encoder knobs reachable from JS

### 5.1 The knobs

| Knob (JS) | What it does in Chromium/libwebrtc | Games | Text/code |
|---|---|---|---|
| `track.contentHint` | `''`: follows the source; `getDisplayMedia` tracks are screencast [source: `CR …/user_media_processor.cc`, `WR pc/rtp_sender.cc` L1650-1665]. `'motion'`: `is_screencast=false`, so camera mode: quality scaler on, denoise allowed, default degradation `MAINTAIN_FRAMERATE`, AV1/VP9/VP8/OpenH264 lose their screen-content modes. `'detail'`/`'text'`: screencast mode: no automatic resize, default `MAINTAIN_RESOLUTION`, AV1 `tune-content=screen` + palette at speed 11, VP9 `tune-content=screen`, VP8 `screen-content-mode=2` with min QP 12 and `static_thresh=100`, OpenH264 `SCREEN_CONTENT_REAL_TIME`, and a 5 fps "steady state" frame-rate limiter when nothing changes [source: `WR media/engine/webrtc_video_engine.cc` L962-1052, L2030-2066; the libaom/libvpx encoders] | **`'motion'`** | **`'detail'`** (`'text'` behaves the same in libwebrtc) |
| `degradationPreference` (RTCRtpSendParameters) | What the CPU/quality adapter gives up first. Unset: from the hint (above). `balanced` on a screencast is treated as `maintain-resolution` [source: `WR video/video_stream_encoder.cc` L640-651]. LiveKit sets `'maintain-resolution'` for screen share unless told otherwise [source: `publishUtils.ts` L493-502] | `'maintain-framerate'` (lets resolution drop under load) | `'maintain-resolution'` |
| `encodings[].maxBitrate` | The encoder's cap. Without it, libwebrtc's default for ≥960×540 is 2.5 Mbps (1.2 Mbps floor for screenshare) [source: `WR video/config/encoder_stream_factory.cc` L99-115] | Per preset (§7) | 1-2 Mbps |
| `encodings[].maxFramerate` | A frame-rate cap. It also sets the rate controller's per-frame budget | 60 | 15 (5 for "slides") |
| `encodings[].scaleResolutionDownBy` / `scaleResolutionDownTo` | A fixed downscale before encode | Use for a 720p preset from a 1080p/1440p capture (cheaper than asking `getDisplayMedia` for 720p, which some sources ignore) | Never |
| `encodings[].scalabilityMode` | `'L1T1'`…`'L1T3'`, `'L3T3_KEY'`. See §4 | `'L1T1'` | `'L1T1'` |
| `encodings[].priority` / `networkPriority` | `bitrate_priority` splits the *sender's* available bitrate between its tracks; DSCP marking on the wire (mostly bleached on home internet) | `'high'` for the share, so voice in Rust is unaffected (separate connection) | same |
| `encodings[].minBitrate` (non-standard, via `setParameters`) / SDP `x-google-min-bitrate` | A floor for the *allocator*. The encoder will not be given less. It does not stop congestion control from dropping below it on the wire, so this causes frame drops instead | Leave unset | ~300 kbps can keep text from going blurry; spike |
| SDP `x-google-start-bitrate` | Where bandwidth estimation starts (default 300 kbps) [source: `WR api/transport/bitrate_settings.h` L44]. **LiveKit already munges it to 90% of the screen-share max** (no 1 Mbps cap for screen share) [source: `client/src/room/PCTransport.ts` L21-90]. Open issue client-sdk-js#1816 says H.264 was excluded from the start-bitrate fix: check `getStats` in the first 5 s | already handled | already handled |
| SDP `x-google-flag:conference` | "Legacy conference screenshare": VP8 two temporal layers with TL0 capped at 200 kbps and TL1 at 1 Mbps [source: `WR video/config/simulcast.cc` L44-60]. Designed for slides at low bitrate | **Never** | Not useful above 1 Mbps; skip |
| Keyframe interval | Not settable from JS. WebRTC sends keyframes on PLI/FIR; MF GOP is 3000 frames | — | — |
| VBR vs CBR | Not settable. WebRTC runs CBR-ish with a ~1 s buffer; the H.264 MF path uses Chromium's own QP controller (except on AMD) | — | — |

### 5.2 Latency effects

- **`maintain-resolution` on a game** means that under load the frame rate drops, which feels like stutter. Under
  `maintain-framerate` the resolution drops, which feels like blur.
- **Screencast mode** also allows the VP8/VP9 frame-rate limiters to hold frames. That adds no latency, but lowers
  fps.
- **Pacing.** None of these knobs add encoder latency on the hardware path. Bigger `maxBitrate` means bigger
  keyframes, so longer pacing bursts (§6).

### 5.3 Auto-switching the hint

Pulse knows what is shared: the track label and `getSettings().displaySurface` in JS, or the window's process from
the Rust side.

- **Rule [est.]:**
  - fullscreen or borderless game window → `'motion'` + `maintain-framerate` + 60 fps;
  - IDE, browser or desktop → `'detail'` + `maintain-resolution` + 15 fps.
- `contentHint` can be changed **live**. Chromium forwards the change to the sender, which reconfigures the encoder
  (`OnContentHintChanged` → `VideoRtpSender::SetSend`) [source: `CR …/media_stream_video_webrtc_sink.cc` L289-293;
  `WR pc/rtp_sender.cc` L1643-1665]. Expect a keyframe on the switch.
- **Motion detection** as a fallback [speculation]: sample `RTCOutboundRtpStreamStats` every 2 s. A high
  `totalEncodedBytesTarget` with high `qpSum`/`framesEncoded` means motion; near-zero bytes and dropped frames mean a
  static screen. Or read 1 frame/s through `MediaStreamTrackProcessor`, downscaled.

### 5.4 Undoing LiveKit's forced `motion` for AV1 text mode

**The cause.** For any SVC codec without "SVC simulcast", the client sets `scalabilityMode='L1T3'` and
`contentHint='motion'` on screen share. The code comment says Chrome caps L1T3 screen content at 5 fps and has L3T3
bugs [source: `LocalParticipant.ts` L1155-1172].

The 5 fps behaviour is real. libvpx VP9's screenshare path has a steady-state limiter at 5 fps when the frame's
update rectangle is empty [source: `WR …/libvpx_vp9_encoder.cc` L94-101, L1040-1080]. But it only fires on
unchanged content, which is fine for text.

**Options [speculation, test in the spike]:**
1. Publish AV1 with `simulcast: true, scalabilityMode: 'L1T1'`. This is the "rid simulcast" path (client ≥2.22.3,
   server >1.13.6), which skips the forced hint branch. Pass `screenShareSimulcastLayers: []` or one low layer.
2. After `publishTrack`, set `track.mediaStreamTrack.contentHint = 'detail'` again. Chromium reconfigures the
   encoder live.

**Confirm** that `getStats` shows the expected fps and that the bitrate drops on static text.

---

## 6. Keyframes and recovery

### 6.1 Keyframe size vs bitrate [measured, NVENC low-latency CBR, 1 s buffer]

| | 1080p60 at 4.5 Mbps | at 6 | at 8 | at 12 | 720p60 at 4.5 |
|---|---|---|---|---|---|
| First keyframe, CS:GO (H.264 / AV1) | 99 / 82 KB | 128 / 117 | 183 / 158 | 254 / 215 | 112 / 102 |
| First keyframe, Dota 2 (H.264) | 78 | 99 | 137 | 187 | 88 |
| Average frame (H.264) | 9 KB | 12 | 16 | 25 | 9 |
| **Forced IDR mid-stream, CS:GO (H.264, 6 Mbps)** | | **24 KB** (2× average), **VMAF 80 → 51**, back to ~68 after 20 frames | | | |

For the code clip, **page-change delta frames** reached 75-220 KB at 1-4 Mbps (H.264) and 180-215 KB for libaom.
**On screen content, the burst problem is text "slide changes", not keyframes.**

**What it means:**
- **A first keyframe is ~10× an average frame,** or roughly 0.15 s of the stream's bitrate. libwebrtc's pacer
  sends it at ~2.5× the target rate [est.], so it takes ~50-100 ms and 70-180 packets. This is fine with the 16 MiB UDP
  buffers already recommended.
- **Mid-stream keyframes are squeezed by the rate controller** (NVENC ll/CBR), so they don't burst much. Instead
  they **blur the whole picture for ~0.3-0.5 s.** libvpx/libaom let keyframes overshoot more: VP8 screenshare
  explicitly sets `MAX_INTRA_BITRATE_PCT` [source: `WR …/libvpx_vp8_encoder.cc` L1129-1137].
- **Every PLI hits all viewers of that layer.** One friend joining or recovering from loss makes the picture pulse
  for everyone [measured dip; inferred for the SFU].

### 6.2 PLI/FIR behaviour in LiveKit 1.13.7 [source]

- A new subscriber's DownTrack runs a `keyFrameRequester`. Until it has locked onto a layer, it sends a PLI upstream
  every clamp(2×RTT, 200 ms, 1 s) (`downtrack.go` L944-982). Subscriber PLIs are relayed upstream (L1987-2016).
- **The throttle is per publisher layer**, not per subscriber, so N subscribers do not cause N keyframes:
  - `pli_throttle`: low 500 ms, mid 1 s, high 1 s (`receiver_base.go` L53-64, L729-741).
  - For a single-layer stream the only buffer is layer 0, which is throttled as "low" (500 ms) [inferred].
- **No keyframe cache.** The SFU can't give a joiner the last keyframe without asking the publisher.
- **No FEC/RED for video.** Recovery is NACK + RTX + PLI only (`protocol/codecs/codecs.go` L177-186).

### 6.3 Intra refresh, long-term references, alternatives

- **Intra refresh (gradual decoder refresh)** [measured]:
  - NVENC `-intra-refresh` over 60 frames gave no frame bigger than 23 KB and no quality dip (VMAF 70.1 overall vs
    70.9 for the IDR run).
  - **Not reachable:** Chromium's MF encoder has no intra-refresh code path [source: grep of the MF encoder], and
    WebRTC has no API for it.
  - **Not compatible with LiveKit as is:** the forwarder waits for a keyframe to lock a new subscriber, so a stream
    with only GDR frames would never start for a joiner [inferred from `downtrack.go`].
  - Only a native pre-encoded pipeline (capture-encode report, Phase B) could do it, together with a periodic IDR.
- **Long-term references (LTR):**
  - Chromium sets them only on Intel, via the DisplayRemoting scenario [source].
  - libwebrtc's own encoders don't expose them.
  - The Loss Notification RTCP extension (`goog-lntf`) exists in libwebrtc but LiveKit doesn't negotiate it [not
    verified].
  - Nothing to do here.
- **What we *can* do:**
  1. Keep `pli_throttle` at defaults or raise "low" to 1 s, so a flapping viewer can't keyframe everyone twice a second.
  2. Join late, never early. Auto-subscribing would make every voice-channel member trigger a keyframe.
  3. With the on-demand low simulcast layer, a weak viewer's PLIs hit only the low layer.
  4. Fix loss at the source (UDP buffers, 2 OCPU), since every lost packet past the NACK window becomes a PLI.

---

## 7. Preset table

Targets are the encoder **cap** (`maxBitrate`). Games run at 85-100% of the cap; text at 20-60%. Quality estimates
come from §2. "Hard" means CS:GO-like (fine detail plus fast motion); "typical" means Dota 2 / Minecraft-like.
1440p numbers are [est.], scaled from 1080p by pixel count^0.75 (≈1.54×). I had no 1440p source.

| Preset | Codec | Games cap | Expected look, games | Text/code cap | Expected look, text |
|---|---|---|---|---|---|
| **720p30** | H.264 HW | 3 Mbps | Typical 85-90, hard ~70 [measured at 2.5-4] | **Don't use for text** | VMAF ~74 ceiling whatever the bitrate |
| | AV1 | 2.5 | +3-6 points | — | — |
| **720p60** | H.264 HW | **5-6 Mbps** | Typical 89-91 (720p ceiling), Dota ~72, hard 70-76 | — | — |
| | AV1 | 4-4.5 | Same look | — | — |
| **1080p30** | H.264 HW | 5 | Typical 85-98, hard ~80 | **1.5-2 Mbps at 15 fps** | ≥95, worst frame ≥80 |
| | AV1 | 4 | | **0.6-1 Mbps at 15 fps (libaom, screen tools on)** | ~96-97, worst frame ≥92 |
| **1080p60** | H.264 HW | **8 Mbps** | Typical 83-99, hard ~80 | (use 15 fps) | |
| | AV1/HEVC | 6-6.5 | Same look | | |
| **1440p60** | H.264 HW | 12-14 [est.] | | 2-3 Mbps at 15 fps | |
| | AV1 | 9-10 [est.] | | 1-1.5 Mbps (AV1 software at 1440p is ~1.8× the CPU: only on strong CPUs) | |

**Is 720p60 at 4.5 Mbps H.264 fine?**
- It's *okay*, not great: Minecraft 89, GTA V 92, Dota 2 70, CS:GO 68 [measured].
- At that bitrate **1080p60 looks better on three of four games**, while 720p protects the hardest content.
- Chromium can make this decision itself:
  1. Capture at 1080p.
  2. Set `contentHint='motion'` and `maxBitrate` 4.5-6 Mbps.
  3. The quality scaler drops to 720p or 540p when the QP stays high. Check it is active on the MF path in the
     spike: `qualityLimitationReason='bandwidth'` with `frameWidth` < capture width.
- If we want a fixed default for the group, **720p60 at 6 Mbps H.264**, or **1080p60 at 8 Mbps** for friends with
  ≥12 Mbps up, are the honest numbers.

**What Discord uses:**
- Free tier: 720p30. Nitro Basic: 1080p60. Nitro: up to 4K60.
- VP8/H.264, plus HEVC/AV1 on some platforms; AV1 needs an RTX 40 sharer [cited in server-quality report].
- Reported default bitrate caps: desktop min 500 / target 600 / max **4000 kbps**; application max **2500 kbps**
  [cited: [YABDP4Nitro issue #155](https://github.com/riolubruh/YABDP4Nitro/issues/155), comment of 2023-11-01,
  third party, unofficial].
- So Discord's free tier is roughly "720p30 at ≤2.5-4 Mbps". By our numbers that's VMAF ~65 (hard) to ~92 (easy).
  **Pulse at 720p60/6 or 1080p60/8 will look clearly better than Discord free** and similar to Nitro.

---

## 8. Wild ideas

1. **Viewer-side super resolution: send 720p, show ~1080p.**
   - **NVIDIA RTX VSR:**
     - Works in Chrome and Edge `<video>` on all RTX GPUs (RTX 20 support since VSR 1.5, Oct 2023).
     - Input 360p-1440p.
     - Only when the video is actually upscaled, visible and the most recently focused one.
     - Off on battery, and off while a game uses NIS/DSR/DLDSR.
     - On Edge, Microsoft's "Enhance videos" must be off.
     - [cited: [NVIDIA RTX Video FAQ](https://nvidia.custhelp.com/app/answers/detail/a_id/5448), updated 2025-10-09]
   - **How Chromium triggers it:** it enables the NVIDIA driver extension on **any video overlay swap chain**
     (`ToggleNvidiaVpSuperResolution` in `CR ui/gl/swap_chain_presenter.cc`). It doesn't check whether the frames
     came from MSE or WebRTC [source]. So a WebRTC `<video>` *should* qualify when promoted to a DirectComposition
     overlay [inferred].
   - **Unknowns:**
     - WebView2 hosting: does it get overlays and the swap-chain path?
     - Whether HTML overlays on top of the video block promotion.
     - No documentation or report for WebView2 exists [not found].
   - **Spike:** a 720p stream fullscreen in Pulse. Toggle RTX Video in the NVIDIA app; the NVIDIA overlay/stat shows
     whether it is active.
   - **Intel's equivalent** (`kIntelVpSuperResolution`) is off by default in Chromium [source]. We could enable it
     with the flag mechanism in §3.3.
   - **AMD:**
     - "Video Upscale" in Adrenalin 24.1.1+ is for RX 7000. In browsers it reportedly needs the
       `Media Foundation for Clear` path [cited: AMD Adrenalin 24.1.1 notes via [overclock3d](https://overclock3d.net/news/gpu-displays/amd-brings-fluid-motion-frames-to-its-radeon-gpu-drivers-with-version-24-1-1/)].
     - That is the MediaFoundationRenderer path, unlikely to apply to WebRTC MediaStreams [inferred].
   - **Edge's own ML VSR:** only for video under 720p, RTX 20+/RX 5700+, and through a DX12 path [cited: [Edge blog](https://blogs.windows.com/msedgedev/2023/03/08/video-super-resolution-in-microsoft-edge/),
     2023-03-08]. Not documented for WebRTC or WebView2.
   - **Why it matters [measured]:** the plain choice of upscaler is worth 5-7 VMAF points (720p60 at 4.5 Mbps,
     bilinear → bicubic: CS:GO 62.6 → 67.8, Minecraft 82.1 → 88.9; Lanczos about the same as bicubic). An ML
     upscaler should be worth more.
2. **Our own upscaler in WebGL** [speculation]:
   - Draw the `<video>` into a WebGL2 canvas on `requestVideoFrameCallback` and run FSR1 (EASU + RCAS, open source,
     two passes, < 1 ms at 1080p on any GPU) or a Lanczos-2 shader.
   - Portable to AMD, Intel and Linux viewers.
   - **Cost:** it defeats overlay promotion, so the composition costs some power.
   - A "Sharpen stream" toggle on the viewer side.
3. **Text at 5 fps, near-lossless** [measured]:
   - At 1080p5, H.264 hardware at 1 Mbps reaches a 96 average and a 92 worst frame. libaom screen mode gets there at
     ~0.3 Mbps.
   - A "Slides/code" preset at 5 fps is nearly free and sharper than anything at 30 fps.
   - **Hint:** `maxFramerate: 5` alone doesn't stop libwebrtc's encoders sending a frame when the screen changes. The
     cap holds, and frames arrive within 200 ms of a change, which is fine for code.
4. **Auto-detect content** (§5.3). Switch `contentHint`, `maxFramerate` and the codec per share. Combine it with
   "game window → 1080p60 motion", "anything else → native resolution, detail, 15 fps".
5. **Region-of-interest encoding** [speculation]:
   - Not reachable through WebRTC: there is no ROI API in `RTCRtpSender`, and no ROI path in Chromium's MF
     encoder.
   - WebCodecs `VideoEncoder` has `bitrateMode:'quantizer'` with a per-frame QP for AV1/VP9/AVC (Chrome 117+
     reportedly) [cited: [MDN encode()](https://developer.mozilla.org/en-US/docs/Web/API/VideoEncoder/encode)]. That
     is per-frame, not per-region.
   - True ROI (e.g. a sharper HUD/minimap) needs the native pipeline: NVENC/AMF expose QP delta maps. **Park it.**
6. **Frame interpolation on the viewer** [speculation]:
   - Send 30 fps and show 60.
   - AMD's AFMF is for games, not browser video [cited: overclock3d above].
   - No vendor offers it for browser video.
   - A WebGL optical-flow interpolator adds a frame of latency and artefacts on HUDs. **Not worth it.** Spend the
     bits on 60 fps instead.
7. **4:4:4 / lossless text tiles** [speculation]: Parsec and Sunshine use 4:4:4 for desktop text. WebRTC in Chromium
   is 4:2:0 only, so coloured syntax highlighting and ClearType fringes lose chroma. AV1 palette mode partly
   compensates. Not reachable otherwise.
8. **HDR games** stay SDR (capture-encode report). AV1/HEVC 10-bit are not used by WebRTC on Windows
   (`kPlatformHEVCHbdEncoderSupport` off) [source].

---

## 9. Add to the spike (beyond the existing list)

1. On an **NVIDIA-only** desktop, an **AMD** desktop and an **Intel laptop**, for H.264 screen share, record:
   - the negotiated `profile-level-id` (from the local SDP);
   - `encoderImplementation`, `powerEfficientEncoder`, `qualityLimitationReason`;
   - CPU %;
   - and repeat after the offer munge that puts Main (4d001f) first.
2. `--enable-features=WebRtcAV1HWEncode` via `additional_browser_args` on an RTX 40 or RX 7000:
   - does `encoderImplementation` change to MediaFoundation?
   - quality vs H.264 at 6 Mbps on a real game;
   - does an RTX 30 viewer decode it in hardware?
3. AV1 software text mode: CPU on a mid-range laptop at 1080p15/1440p15 with screen tools (after undoing the forced
   `motion` hint, §5.4). Check the viewer's dav1d CPU without hardware AV1.
4. Does Chromium's quality scaler run on the MF H.264 path with `contentHint='motion'`? Feed a hard game at 4.5 Mbps
   and watch `frameWidth`.
5. The weak-viewer simulcast layer:
   - throttle one viewer to 3 Mbps with `tc`/clumsy;
   - confirm the allocator moves it to the low layer;
   - confirm the low layer pauses when that viewer leaves;
   - count NVENC sessions with `nvidia-smi encodersessions`.
6. RTX VSR in WebView2: a 720p stream in a 1440p window, NVIDIA app → RTX Video status. Also check whether HTML
   overlays on the `<video>` disable it.
7. **Streamer game impact:** in-game fps and 1% lows with no share, then with a 1080p60 hardware H.264 share, then
   with share + low simulcast layer. Also GPU 3D/copy-engine load in Task Manager, and CPU of the WebView2 GPU and
   renderer processes. Repeat with `contentHint='motion'` vs default to see if the zero-copy path engages.
8. Join-pulse: with 3 viewers, have a 4th join. Record the VMAF-visible dip on the others (screen recording) and the
   PLI count in `getStats` (`pliCount` on outbound-rtp).

---

## 10. Sources

**Local measurements:** scripts and raw data in `/tmp/claude-1000/screenshare-codec/` (not committed). Test clips:
Xiph twitch set (public domain), frames 1800-2039 of CSGO, DOTA2, GTAV and MINECRAFT. The code clip was rendered
locally from a Python standard-library file.

**Source code read (2026-10-14):**
- Chromium main:
  - `media/gpu/windows/media_foundation_video_encode_accelerator_win.cc` (L75 GOP; L128-190 rate controller config; L1448-1470 NVIDIA CBP skip; L1640-1790 SW BRC and rate modes; L1856-1877 low latency and the Intel DisplayRemoting scenario)
  - `media/gpu/windows/mf_video_encoder_util.cc`, `mf_video_encoder_shared_state.cc`
  - `media/base/media_switches.cc`, `media/webrtc/webrtc_features.cc`
  - `third_party/blink/renderer/platform/peerconnection/{rtc_video_encoder_factory.cc, rtc_video_encoder.cc, webrtc_util.cc, rtc_video_decoder_factory.cc, rtc_video_decoder_adapter.cc}`
  - `third_party/blink/renderer/modules/peerconnection/media_stream_video_webrtc_sink.cc` (L49-62 hint mapping)
  - `media/gpu/windows/supported_profile_helpers.cc`, `gpu/config/gpu_driver_bug_list.json`
  - `ui/gl/swap_chain_presenter.cc`, `ui/gl/gl_switches.cc`
- WebRTC main (head 421d35b):
  - `pc/rtp_sender.cc` L1643-1665
  - `media/engine/webrtc_video_engine.cc` L962-1052, L2030-2066, L2395-2510
  - `video/video_stream_encoder.cc` L640-651
  - `video/config/encoder_stream_factory.cc` L99-115
  - `video/config/simulcast.cc` L44-60
  - `video/video_send_stream_impl.cc` L86-172
  - `modules/video_coding/codecs/av1/libaom_av1_encoder.cc` L380-432, L476-520
  - `modules/video_coding/codecs/vp9/libvpx_vp9_encoder.cc` L94-101, L678-690, L965-967, L1040-1080
  - `modules/video_coding/codecs/vp8/libvpx_vp8_encoder.cc` L82-95, L625-650, L907-921, L1129-1137
  - `modules/video_coding/codecs/h264/h264_encoder_impl.cc` L685-745
  - `media/base/media_constants.cc` L42-44, `api/transport/bitrate_settings.h` L44
- livekit-client v2.22.4:
  - `src/room/defaults.ts`, `src/room/track/options.ts` L534-544
  - `src/room/participant/publishUtils.ts` L56-220, L493-502
  - `src/room/participant/LocalParticipant.ts` L727-773, L1141-1206, L2064-2084
  - `src/room/track/LocalVideoTrack.ts` L503-551
  - `src/room/track/RemoteTrackPublication.ts` L304-360
  - `src/room/PCTransport.ts` L21-170
- livekit server v1.13.7:
  - `pkg/rtc/mediaengine.go` L61-100, `pkg/rtc/participant_sdp.go` L175-246, `pkg/rtc/transport.go` L345-352, L3287-3322
  - `pkg/sfu/downtrack.go` L944-982, L1987-2016
  - `pkg/sfu/receiver_base.go` L53-64, L620-625, L729-741
  - `pkg/sfu/forwarder.go` L382-455
  - `pkg/sfu/streamallocator/{track.go L86-114, streamallocator.go L43-46, L1174-1205}`
  - `pkg/rtc/{subscribedtrack.go L456-480, mediatrack.go L110-185, L511-566}`
- `livekit/protocol` (pinned by server go.mod): `codecs/codecs.go` L97-186, L225-260; `codecs/mime/mimetype.go` L323-333.
- Tauri 2.12.1 `src/webview/webview_window.rs` L1080; wry 0.57.0 `src/lib.rs` L1751-1792, `src/webview2/mod.rs` L300-333.
- OpenH264 (cisco/openh264 @1bca623, 2026-10-12): `h264enc -h` defaults.

**Web (read 2026-10-14):**
- Chromestatus API, HEVC in WebRTC (5153479456456704), HEVC HW decode (5186511939567616): https://chromestatus.com/api/v0/features?q=HEVC
- MS Q&A, Edge H.265 WebRTC (2026-05-04, 2026-09-09): https://learn.microsoft.com/en-ca/answers/questions/5880331/
- MS Learn, Edge video playback issues (2026-03-31): https://learn.microsoft.com/en-us/troubleshoot/microsoft-edge/development/video-playback-issues
- NVIDIA Video Encode and Decode GPU Support Matrix (undated): https://developer.nvidia.com/video-encode-and-decode-gpu-support-matrix-new
- NVENC Application Note, SDK 13.0: https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-application-note/index.html
- Wikipedia, Video Core Next: https://en.wikipedia.org/wiki/Video_Core_Next ; Unified Video Decoder: https://en.wikipedia.org/wiki/Unified_Video_Decoder
- Intel media-driver features: https://raw.githubusercontent.com/intel/media-driver/master/docs/media_features.md
- Tom's Hardware, "Video Encoding Tested: AMD GPUs Still Lag Behind Nvidia, Intel" (2023-03-10): https://www.tomshardware.com/news/amd-intel-nvidia-video-encoding-performance-quality-tested
- Discord, "From Blocky to Brilliant" (2024-05-01): https://discord.com/blog/from-blocky-to-brilliant-improving-video-quality-on-discord-go-live-on-amd-gpus
- NVIDIA, Discord AV1 streaming (2023-02-24): https://www.nvidia.com/en-us/geforce/news/nvidia-studio-discord-av1/
- YABDP4Nitro issue #155 (Discord bitrate defaults, 2023-11-01, unofficial): https://github.com/riolubruh/YABDP4Nitro/issues/155
- webrtcHacks, "The hidden AV1 gift in Google Meet" (2023-12-19): https://webrtchacks.com/the-hidden-av1-gift-in-google-meet/
- Chrome for Developers, "Improved video calling with faster AV1 encoding" (updated 2023-05-02): https://developer.chrome.com/blog/av1
- Jitsi, "AV1 and more … how does Jitsi Meet pick video codecs?" (2024-12-17): https://jitsi.org/blog/av1-and-more-how-does-jitsi-meet-pick-video-codecs/
- NVIDIA RTX Video FAQ (updated 2025-10-09): https://nvidia.custhelp.com/app/answers/detail/a_id/5448
- Microsoft Edge blog, Video super resolution (2023-03-08): https://blogs.windows.com/msedgedev/2023/03/08/video-super-resolution-in-microsoft-edge/
- AMD Adrenalin 24.1.1 coverage: https://overclock3d.net/news/gpu-displays/amd-brings-fluid-motion-frames-to-its-radeon-gpu-drivers-with-version-24-1-1/
- MDN, VideoEncoder.encode(): https://developer.mozilla.org/en-US/docs/Web/API/VideoEncoder/encode
- LiveKit docs, advanced tracks: https://docs.livekit.io/home/client/tracks/advanced/
- GitHub issues: client-sdk-js #1783, #1816, #1822, #2087; livekit #4594, #4695 (states as of 2026-10-14)
- Xiph test media, twitch set: https://media.xiph.org/video/derf/twitch/
