# Pulse screen share on Windows: capture, encode and publish through LiveKit Rust

**Research date:** 2026-10-13. **Pinned versions:** `livekit` 0.9.3, `libwebrtc` 0.3.50, `webrtc-sys` 0.3.47, libwebrtc prebuilt tag `webrtc-89d790b`. **Server:** LiveKit 1.13.7.
**Scope:** research only. This feeds the v2 screen-share spec (design spec §14: "spike first; heavily optimised; fine-tune quality/fps").

Path shorthand used below:

- `REG` = `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f`
- `WINLIB` = the Windows prebuilt that is already on disk, at `~/Projects/pulse-app/target/release/build/scratch-816c66187b1c570d/out/livekit_webrtc/livekit/win-x64-release-webrtc-89d790b/win-x64-release`

---

## 0. TL;DR

1. **The SDK can already ship a basic screen share with no fork.**
   - `livekit::webrtc::desktop_capturer` wraps libwebrtc's own `DesktopCapturer`. It uses WGC on Windows, with DXGI as an option.
   - Frames go through a CPU BGRA→I420 step, then into `NativeVideoSource::new(res, true)`. They are published with `TrackSource::Screenshare`.
   - LiveKit's own `examples/screensharing` does exactly this.
   - On Windows, every encoder is **software**: VP8, VP9 (libvpx), H.264 (OpenH264) and AV1 (libaom). This is fine for text and desktops at 1080p 15–30 fps. It is not fine for games at 1080p60.
2. **There is no hardware encoder in webrtc-sys on Windows.**
   - The NVENC backend compiles only on Linux, because it needs `cuda.h` and dlopens a `.so`.
   - The `vaapi-windows` block in `build.rs` is commented out.
   - Upstream PR #1282 (Media Foundation H.264, opened 2026-07-26) is still open.
3. **The escape hatch is already in the pinned SDK: the pre-encoded pass-through.**
   - The API is `NativeVideoSource::new_encoded()`, `capture_encoded_frame(EncodedVideoFrame{H264|H265|VP8|VP9|AV1})`, `take_keyframe_request()` and `take_rate_control_request()`, published with `VideoEncoderBackend::PreEncoded`.
   - This lets Pulse own the whole GPU pipeline: **WGC D3D11 texture → GPU BGRA→NV12 (and HDR→SDR) → hardware H.264 (Media Foundation MFT, or NVENC/AMF directly) → LiveKit**. That is the same architecture as Discord, Parsec and Sunshine, and it needs no C++ fork.
   - The catch: pass-through has known frame-drop bugs (§4.4), and it gives a single layer only (no simulcast).
4. **Audio:**
   - WASAPI **process loopback** captures a game's audio, including its child processes. It is documented for Windows 10 build 20348 and later, and reported to work on older builds.
   - Use **EXCLUDE mode on Pulse's own PID** to get "whole system minus Pulse", which removes the voice echo.
   - The `wasapi` crate (0.26) supports both modes.
   - Feed it into a second `NativeAudioSource`, published as `TrackSource::ScreenshareAudio`.
5. **Recommended direction (verdict in §6):**
   - **Phase A:** SDK `DesktopCapturer` with software VP9 or H.264, for "share a window or desktop", in days.
   - **Phase B:** our own WGC capture plus a hardware H.264 pre-encoded pipeline, for games at 60 fps. The spike decides whether Phase A is worth shipping at all, or whether we go straight to B.

---

## 1. Capture APIs on Windows 10/11

### 1.1 Windows.Graphics.Capture (WGC)

- **Sources:** monitors (`HMONITOR`) and windows (`HWND`), created through `IGraphicsCaptureItemInterop::CreateForWindow/CreateForMonitor`, which needs 1903 / build 18362. There is also a system picker (`GraphicsCapturePicker`).
- **Minimum OS:** `GraphicsCaptureSession` arrived in 1803 (build 17134).
  - `IsCursorCaptureEnabled` arrived in 2004 (build 19041).
  - `IsBorderRequired` arrived in build 20348, so on clients it is effectively **Windows 11** (build 22000+).
  - `MinUpdateInterval` and `DirtyRegionMode` are newer (24H2/26100-era contract).
  - libwebrtc's comment says WGC needs 1809 (RS5) or later. ([MS docs: GraphicsCaptureSession](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession))
- **The yellow border:**
  - **Windows 10:** cannot be disabled.
  - **Windows 11:** call `GraphicsCaptureAccess::RequestAccessAsync(Borderless)`, then `session.IsBorderRequired(false)`. OBS does exactly this (`libobs-winrt/winrt-capture.cpp` L316–320, L382–386).
  - Sunshine just sets `IsBorderRequired(false)` when the property exists (`src/platform/windows/display_wgc.cpp` L150).
  - MS docs say a denied request still "succeeds" but the border stays. A packaged app needs the `graphicsCaptureWithoutBorder` capability. Unpackaged Win32 apps (like Pulse: a Tauri MSI/NSIS install) appear to work as OBS does, but **this needs verifying in the spike**. ([IsBorderRequired](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired))
- **Cursor:** `IsCursorCaptureEnabled` (2004+). It is toggled at runtime with no re-create.
- **Output:**
  - Frames are `ID3D11Texture2D` in a `Direct3D11CaptureFramePool`, so they are **GPU-resident and zero-copy capable**.
  - The format is chosen when the pool is created: `B8G8R8A8_UNORM`, or `R16G16B16A16_FLOAT` for HDR (OBS picks FP16 when the monitor is HDR, `get_pixel_format` L79–91).
  - Pool `Recreate` is needed on resize.
- **Frame pacing:**
  - Frames are event-driven (`FrameArrived`) when the compositor presents. A static desktop delivers ~0 fps, so we must re-send the last frame or let WebRTC handle 0 Hz.
  - It is capped at the display refresh rate. `MinUpdateInterval` (newer builds) throttles it.
  - Timestamps come from `SystemRelativeTime`.
- **Cost:** low. DWM already composites, so WGC is a copy of the composed surface. OBS reports WGC usually performs better than DXGI in games (anecdotal, OBS forum).
- **Multi-GPU:** WGC works across GPUs with no user action. That is its key advantage over DXGI on Optimus laptops. ([OBS forum, WGC vs DXGI](https://obsproject.com/forum/threads/windows-graphics-capture-vs-dxgi-desktop-duplication.149320))
- **Limits:**
  - Exclusive-fullscreen (true FSE, "fullscreen optimisations" disabled, old DX9) can capture black or stall. Most modern games run as borderless/independent-flip, which WGC captures.
  - Protected content (DRM video) is black.
  - WGC cannot run from a Windows service (Sunshine notes this), which does not matter for Pulse.
- **Rust crates:**
  - [`windows-capture`](https://github.com/NiiightmareXD/windows-capture) 2.0.1 (2026-08-08, MIT, ~2M downloads).
    - Exposes `Frame::as_raw_texture() -> &ID3D11Texture2D` (zero-copy) and CPU `buffer()`.
    - Settings: `ColorFormat::{Rgba16F, Rgba8, Bgra8}`, `CursorCaptureSettings`, `DrawBorderSettings::WithoutBorder`, `SecondaryWindowSettings`, `MinimumUpdateIntervalSettings`, `DirtyRegionSettings`, plus DXGI duplication (`dxgi_duplication_api.rs`).
    - Its `encoder.rs` uses WinRT `MediaTranscoder` (file or stream oriented, not real-time RTC friendly). Ignore it; use only the capture side.
  - Raw `windows` crate (0.62 already in the tree): ~300 lines to do it ourselves.
  - `scap` (Zed's fork) is cross-platform and abstracts WGC.
- **libwebrtc's WGC** (what the SDK uses):
  - `WgcCapturerWin` maps the texture to a CPU frame.
  - `DesktopCaptureOptions` has `wgc_require_border` (default **false**, so no border on Win11), `allow_wgc_zero_hertz`, `allow_wgc_using_texture` (texture-backed `DesktopFrame` with `FrameTexture*`) and `d3d_device_luid`. See `WINLIB/include/modules/desktop_capture/desktop_capture_options.h` L186–253.
  - LiveKit only sets `allow_wgc_screen/window_capturer`, `allow_directx_capturer`, `prefer_cursor_embedded` and `enumerate_current_process_windows(false)` (`REG/webrtc-sys-0.3.47/src/desktop_capturer.cpp` L31–48). The texture, zero-Hz, border, LUID and secondary-window flags are **not exposed**.

### 1.2 DXGI Desktop Duplication (DDA)

- **Monitors only**, Windows 8+.
  - `IDXGIOutput1::DuplicateOutput` gives BGRA8.
  - `IDXGIOutput5::DuplicateOutput1` takes a format list: FP16 `R16G16B16A16_FLOAT` for HDR desktops.
  - libwebrtc's DXGI code accepts only BGRA8 or FP16.
- **Strengths:**
  - It gives dirty and move rects (great for static text and low bitrate).
  - It works for the secure desktop and UAC when running as SYSTEM.
  - Sunshine's default on Windows is `ddx`, and its WGC is "beta". Parsec used DDA ("direct framebuffer grab... never let the raw frame touch system memory").
- **Exclusive fullscreen:**
  - DDA can capture true FSE.
  - A mode change, FSE transition or UAC prompt causes `DXGI_ERROR_ACCESS_LOST`, so you must re-create.
  - Many games also cause frequent `ACCESS_LOST` on alt-tab.
- **HDR:** FP16 scRGB linear. We need our own tone map (see 1.5).
- **Multi-GPU laptops:**
  - **Hard failure.** DDA must run on the adapter that drives the output. On a hybrid (Optimus/Advanced Optimus/AMD switchable) laptop, an app on the dGPU duplicating the iGPU-driven panel gets `DXGI_ERROR_UNSUPPORTED` by design. ([MS KB](https://learn.microsoft.com/en-us/troubleshoot/windows-client/shell-experience/error-when-dda-capable-app-is-against-gpu), [freemancw 2020](https://www.freemancw.com/2020/11/desktop-duplication-on-hybrid-graphics-systems/))
  - Linking CUDA or NVENC can push the process onto the dGPU and break DDA.
  - The workaround is a capture helper on the iGPU plus a cross-adapter copy. Sunshine does this with a helper process.
- **Cost:** lowest-level path, cheap. Anecdotes say DDA can hurt game fps more than WGC on some setups.
- **Crates:** `windows-capture`'s DXGI module, `rusty-duplication`, `dxgcap`.

### 1.3 Game hooks (OBS "Game Capture", old Discord, Parsec-era tools) and anti-cheat

- **How hooks work:** inject a DLL (`graphics-hook64.dll`), hook `Present`/`SwapChain` in D3D9/10/11/12, OpenGL and Vulkan, and copy the back buffer to a shared texture.
  - This gives the best pacing (it captures exactly the game's frames, not composited ones) and works with FSE.
- **Anti-cheat:** EAC, BattlEye, Vanguard and FACEIT treat injection as cheating.
  - OBS ships a signed "anti-cheat compatibility hook" that some anti-cheats whitelist. Vanguard (Valorant) blocks game capture, and users fall back to Window/Display capture.
  - Discord's March 2024 post says Go Live uses "the best capture method the OS offers", and some methods "require Discord to reach into the application through dll-injection". Discord's hook is whitelisted by the major anti-cheats through business relationships.
  - **Pulse can never get whitelisted.** ([Discord, 2024-03-05](https://discord.com/blog/how-it-all-goes-live-an-overview-of-discords-streaming-technology); OBS forum threads)
- **Verdict:** **do not hook.** WGC window/monitor capture is the anti-cheat-safe path. It is an OS API, not injection; none of the anti-cheats we know of block WGC itself. **Unverified for Vanguard: test Valorant in the spike.** Black frames from WGC in a protected game would be the failure mode, not a ban.

### 1.4 Comparison table

| | WGC (own impl / windows-capture) | WGC via SDK `DesktopCapturer` | DXGI DDA | Game hook |
|---|---|---|---|---|
| Windows | 1903+ (cursor toggle 2004+) | 1809+ | 8+ | any |
| Monitor / window | both / both | both / both | monitor only | game only |
| Yellow border | Win10: always; Win11: removable | not drawn on Win11 (`wgc_require_border=false`) | none | none |
| Cursor | toggle | `set_include_cursor` | separate shape, we composite | game only |
| Exclusive fullscreen | often black/stall (rare in modern games) | same | yes, with ACCESS_LOST churn | yes |
| Hybrid GPU laptop | works | works | **fails** unless on the panel's GPU | n/a |
| HDR | FP16 pool | not exposed (BGRA8 path) | FP16 via DuplicateOutput1 | game's format |
| Zero-copy GPU texture | **yes** | no (CPU BGRA `data()`) | yes | yes |
| Frame pacing | event on present (≤ refresh) | we poll `capture_frame()` | `AcquireNextFrame` timeout | exact game frames |
| Anti-cheat | safe (OS API) | safe | safe | **blocked / ban risk** |
| Effort | medium (COM/WinRT) | ~0 (example exists) | medium-high | not viable |

### 1.5 HDR → SDR

- With an HDR desktop, a BGRA8 WGC/DDA capture gets Windows' own SDR conversion: usually clipped and washed out.
- A proper path is: capture FP16 scRGB (linear, 1.0 = 80 nits) → divide by the SDR white level (from `DISPLAYCONFIG_SDR_WHITE_LEVEL`, typically 200–300 nits) → tone-map highlights (Reinhard/BT.2390-ish) → sRGB OETF → BT.709 → NV12. This is one compute or pixel shader pass on the GPU.
- OBS uses SDR white level with a default of 300 nits plus optional tone-map filters. Sunshine has `fp16_sdr_gamma_encoded` quirks for FP16-on-SDR displays.
- **Cost:** negligible on GPU. On the CPU path it is expensive (8 bytes/pixel).
- **Recommendation:** v2 = SDR only. Phase B adds an FP16 → tone-map shader if any friend runs Windows HDR. Ask them first.

---

## 2. Audio capture for a share

### 2.1 Process loopback (per-application)

- **Activation:** `ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, AUDIOCLIENT_ACTIVATION_PARAMS{ ActivationType: PROCESS_LOOPBACK, ProcessLoopbackParams{ TargetProcessId, ProcessLoopbackMode } })`.
- **Modes:** `INCLUDE_TARGET_PROCESS_TREE` or `EXCLUDE_TARGET_PROCESS_TREE`.
- **Minimum OS:** MS docs say Windows 10 build 20348. GStreamer's wasapi2 notes it "seems to be supported by old versions too" (19041/2004, 21H2, 22H2, which are the Win10 builds friends actually run). **Spike: test on the oldest Win10 a friend uses.** ([MS docs](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_process_loopback_params))
- **Quirks** (from the `wasapi` crate docs, `src/api.rs`):
  - `GetMixFormat`, `IsFormatSupported`, `GetDevicePeriod`, `GetCurrentPadding` and `GetBufferSize` don't work.
  - You must request a format explicitly (e.g. 48 kHz f32 stereo with autoconvert) in event-driven shared mode.
  - Process loopback captures the process's render streams independent of the endpoint (useful with SteelSeries Sonar, which renders games to its virtual "Game" device). **Spike: verify with Sonar.**
- **Rust:**
  - [`wasapi`](https://github.com/HEnquist/wasapi-rs) 0.26.0 (2026-10-11): `AudioClient::new_application_loopback_client(pid, include_tree)`.
  - **Gotcha:** in that crate, `include_tree=false` means **EXCLUDE** mode, not "include just this process".
  - `cpal` has no process loopback.
- **How Discord does it:** captures "from the shared process and all its children" ([Discord 2024](https://discord.com/blog/how-it-all-goes-live-an-overview-of-discords-streaming-technology)). The process tree matters for launchers and multi-process games.

### 2.2 System loopback, without echo

- **Classic endpoint loopback** (`AUDCLNT_STREAMFLAGS_LOOPBACK` on the default render device) captures everything on that endpoint, **including Pulse's playback of friends' voices**. That causes an echo for viewers, who hear themselves.
- **Fix:** process loopback in **EXCLUDE mode with Pulse's own PID**, which gives the whole system minus Pulse. The WebView2 helper processes are children of Pulse, so they are excluded too (Pulse plays no audio there anyway: FINDINGS "never play audio through the webview").
- With Sonar, endpoint loopback of the default device misses the Game/Media virtual devices. Process loopback EXCLUDE avoids the endpoint question. **Spike: verify.**

### 2.3 Feeding LiveKit

- **Track setup:** a second `NativeAudioSource::new(AudioSourceOptions{echo_cancellation:false, noise_suppression:false, auto_gain_control:false}, 48000, 2, queue_ms)` (`REG/libwebrtc-0.3.50/src/audio_source.rs` L181). Publish it as `TrackSource::ScreenshareAudio` (`REG/livekit-0.9.3/src/room/track/mod.rs` L67) with `audio_encoding: MUSIC_HIGH_QUALITY_STEREO` (128 kbps, `options.rs` L483), `dtx:false`, `red:false`.
- **Stereo:** the SDK munges `stereo=1` into Opus fmtp (`rtc_engine/peer_transport.rs` L339–382). **Spike: confirm that the receiver actually gets 2 channels.**
- **Must not go through our voice APM/denoiser.** Separate pipeline: WASAPI event thread → ring buffer → 10 ms frames → `capture_frame`.
- **Receivers:** a separate `NativeAudioStream` mixed into our existing output mixer with its own per-stream volume slider (like Discord's stream volume). The leak rule from FINDINGS still applies: drop the stream on unsubscribe.
- **A/V sync:** separate tracks are synced only loosely by RTP/RTCP and libwebrtc's A/V sync, which needs the same `stream` id. Set `TrackPublishOptions.stream` identically for the video and audio tracks. **Spike: measure lip-sync drift.**

---

## 3. Encoding

### 3.1 What the prebuilt libwebrtc in webrtc-sys 0.3.47 has on Windows

Evidence:

- `WINLIB/args.gn`: `rtc_use_h264=true ffmpeg_branding="Chrome" enable_libaom=true rtc_libvpx_build_vp9=true is_clang=true`.
- `WINLIB/webrtc.ninja` defines: `WEBRTC_USE_H264`, `RTC_USE_LIBAOM_AV1_ENCODER`, `RTC_DAV1D_IN_INTERNAL_DECODER_FACTORY`, `RTC_ENABLE_VP9`, `RTC_ENABLE_H265`, `RTC_ENABLE_WIN_WGC`.
- Same flags in `REG/webrtc-sys-0.3.47/libwebrtc/build_windows.cmd`.

| Codec | Windows encoder | Windows decoder | HW? |
|---|---|---|---|
| VP8 | libvpx | libvpx | no |
| VP9 | libvpx (profile 0) | libvpx | no |
| H.264 | **OpenH264** (Constrained Baseline) | FFmpeg (sw) | no |
| AV1 | libaom (realtime) | dav1d | no |
| H.265 | **none** (`RTC_ENABLE_H265` is set, but there is no sw encoder; only the pass-through could send it) | none on Windows | no |

How the hardware backends are wired:

- **`REG/webrtc-sys-0.3.47/build.rs`:**
  - NVENC (`src/nvidia/*`, `NvEncoderCuda`, `-DUSE_NVIDIA_VIDEO_CODEC`) and VAAPI are added **only in the `"linux"` arm**. They need `cuda.h`/libva at build time and dlopen `.so` files.
  - The Windows arm has VAAPI-on-Windows **commented out** (`//.file("src/vaapi/vaapi_display_win32.cpp")`, ~L150–170).
  - No `amf`, `mediafoundation` or `qsv` sources exist in the crate (grep clean).
- **`src/video_encoder_factory.cpp`:**
  - Backends: `PreEncoded` (always), VideoToolbox (Apple), Android, Jetson, NVENC and VAAPI (each `#if`).
  - On Windows that leaves software plus `PreEncoded`.
  - `LIVEKIT_PREFERRED_HW_ENCODER=nvenc` just logs "not compiled in".
- **Upstream PR [#1282](https://github.com/livekit/rust-sdks/pull/1282)** "Add Windows hardware H264 encode/decode via MediaFoundation" (danryu, opened 2026-07-26, **open**):
  - Async MFT, CBR, low-latency, Constrained Baseline only, system-memory NV12 input (no texture input).
  - Validated on Intel only. AMD and NVIDIA are untested.
  - If merged, `VideoEncoderBackend::Hardware` would work on Windows with no code from us, but still with a CPU frame path.

### 3.2 Codec trade-offs for screen content

| | VP8 | VP9 | H.264 | AV1 |
|---|---|---|---|---|
| Text sharpness at given bitrate | ok | **good** (screen-content mode under `is_screencast`) | ok (sw Baseline, mediocre) / good (HW High) | **best** (screen-content tools, palette/IntraBC) |
| Motion (games) sw CPU | moderate | high | moderate (OpenH264) | very high |
| HW encode on friends' GPUs | none | Intel only (rare) | **all**: NVENC (GTX 600+), AMF (GCN+), QSV | RTX 40+/RX 7000+/Arc only |
| HW decode for receivers | n/a (we decode in sw in libwebrtc anyway) | | | |
| SVC | temporal | **L1T3/L3T3 SVC** (`scalability_mode`) | simulcast only | SVC |
| Pass-through support | yes | yes | **yes** (main target) | yes (OBU handling in `passthrough_video_encoder.cpp`) |

Receivers decode in libwebrtc **in software for every codec on Windows** (`src/video_decoder_factory.cpp`: VP8/VP9 libvpx, H.264 via FFmpeg, AV1 via dav1d; NVIDIA decode is Linux-only). A 1080p60 H.264 sw decode is ~5–15% of one modern core (FFmpeg). Fine. AV1 1080p60 via dav1d is OK on 6+ cores.

### 3.3 Screen-content tuning knobs available today

- **`NativeVideoSource::new(res, is_screencast=true)`** (`REG/libwebrtc-0.3.50/src/video_source.rs` L86). libwebrtc treats the source as screencast: content type `kScreen`, VP8/VP9 screenshare mode, different min bitrate, and no quality scaling by resolution.
  - Note: `new_encoded()` sets `is_screencast=false` (`native/video_source.rs` L110). That is irrelevant for pass-through.
- **`content_hint`** (`detail`/`text`/`motion`):
  - Bound in `webrtc-sys/src/video_track.rs` L24/L109–110 (`set_content_hint`), **but not exposed** by `libwebrtc` 0.3.50's Rust `RtcVideoTrack`.
  - Using it needs a one-line patch, or the `webrtc-sys` ffi directly, since it's a public crate. For a "game" share we'd want `motion` (keep fps, lower resolution). `is_screencast=true` gives the "detail" behaviour.
- **`TrackPublishOptions.degradation_preference`** (`REG/livekit-0.9.3/src/room/options.rs` L153):
  - The default for `Screenshare` is `MaintainResolution` (drop fps), which suits text.
  - For a game, set `MaintainFramerate`.
  - ⚠️ The doc comments on `libwebrtc::rtp_parameters::DegradationPreference` (L31–34) are **swapped**. The value mapping (L76–77: `MaintainFramerate=1`) matches WebRTC's `MAINTAIN_FRAMERATE`, which degrades resolution. livekit's own comment in `options.rs` is correct.
- **`video_encoding: Some(VideoEncoding{max_bitrate, max_framerate})`**:
  - The built-in screenshare presets are conservative: 1080p30 = 3 Mbps, 1080p15 = 1.5 Mbps (`options.rs` `screenshare::PRESETS`).
  - A game at 1080p60 needs ~6–8 Mbps for H.264 HW, ~4–6 Mbps for AV1.
- **`simulcast`:** the screenshare default adds one low layer at half resolution and **3 fps**. For 6 friends, turn simulcast **off** to save encoder CPU, or use VP9 `scalability_mode: "L1T3"` so the SFU can drop fps per viewer.

### 3.4 CPU cost: 1080p60 software vs hardware (estimates; the spike must measure)

| Path | Encode CPU (8-core desktop) | Notes |
|---|---|---|
| CPU BGRA→I420 (libyuv `argb_to_i420`) 1080p60 | ~3–6% of 1 core | Plus a GPU→CPU readback from WGC/DDA staging (~0.5–1 GB/s of memcpy) |
| OpenH264 1080p60, game motion | ~1–2 cores, often **can't hold 60** | WebRTC's CPU-overuse detector then degrades. Quality is Baseline-grade |
| libvpx VP8 1080p60 realtime | ~1–2 cores | |
| libvpx VP9 1080p60 realtime (speed 7–9) | ~2–3 cores | Best sw quality for text at 1080p ≤30 fps |
| libaom AV1 realtime 1080p60 | 3+ cores | Not viable for games in sw |
| NVENC/AMF/QSV H.264 via MFT or SDK | **<1% CPU**, ~1–3 ms GPU per frame | Games already saturate the GPU. NVENC is a separate ASIC; AMF/QSV share some GPU. This is what ShadowPlay users expect |

Sources: Parsec's claim of "<10 ms encode+decode on recent GPUs, hardware only". [Discord's AMD post](https://discord.com/blog/from-blocky-to-brilliant-improving-video-quality-on-discord-go-live-on-amd-gpus) (2024-05-01) shows that HW rate control, not CPU, is the hard part. No authoritative per-codec CPU numbers for libwebrtc's sw encoders at 1080p60 game content exist; **measure in the spike**.

---

## 4. LiveKit Rust SDK video publish path

### 4.1 Sources and frame types (`REG/libwebrtc-0.3.50/src/video_frame.rs`, `video_source.rs`)

- **`NativeVideoSource::new(VideoResolution, is_screencast)`.** `capture_frame(&VideoFrame<T: VideoBuffer>)` returns false if the adapter dropped it.
  - A raw-keepalive tokio task injects black frames until the first capture.
- **Buffer types:** `I420`, `I420A`, `I422`, `I444`, `I010`, `NV12`, `Native`.
  - **NV12 is accepted.** If we do BGRA→NV12 on the GPU and read back NV12, that is 1.5 B/px instead of 4 B/px.
  - `NativeBuffer` constructors exist only for **macOS `CVPixelBuffer`** and **Linux DMA-BUF** (`from_dmabuf`, for the Jetson zero-copy).
  - **There is no D3D11 texture `NativeBuffer` on Windows.** Passing a D3D11 texture without a CPU copy into a libwebrtc *encoder* is impossible today. It would also be pointless, because no Windows HW encoder exists in the factory to consume it.
- **`NativeVideoSource::new_encoded(res)`** plus the pass-through (§4.4) is the real zero-copy route. We encode on the GPU and hand LiveKit compressed bytes.

### 4.2 Publish options (`REG/livekit-0.9.3/src/room/options.rs`)

| Field | Type / default | Use for screen share |
|---|---|---|
| `source` | `TrackSource::Screenshare` | Drives defaults (degradation and presets) and shows the track as a screen share to other clients |
| `video_codec` | `VP8` default; VP9/H264/AV1/H265 | VP9 for the sw path; H264 for HW pass-through |
| `video_encoding` | `Option<VideoEncoding{max_bitrate, max_framerate}>` | Set explicitly (presets top out at 3 Mbps / 30 fps) |
| `simulcast`, `simulcast_layers` | `true`, default screenshare low layer = ½ res @3 fps | Off for small groups, or custom |
| `scalability_mode` | `Option<String>` e.g. `"L1T3"` | VP9/AV1 SVC, single encoding |
| `degradation_preference` | `Option<…>`; default `MaintainResolution` for Screenshare | `MaintainFramerate` for games |
| `video_encoder` | `VideoEncoderBackend::{Auto, Software, Hardware, Nvenc, Vaapi, VideoToolbox, PreEncoded}` (`libwebrtc-0.3.50/src/rtp_sender.rs` L25) | `PreEncoded` for our GPU pipeline |
| `stream` | `String` | Same value for video and screenshare audio (A/V sync) |
| `frame_metadata_features` | packet trailer (user timestamp, frame id) | Optional glass-to-glass latency measurement in the spike |

### 4.3 The SDK's own desktop capturer: yes, it exists

- `livekit::webrtc::desktop_capturer` (`REG/libwebrtc-0.3.50/src/desktop_capturer.rs`; cfg macOS/Windows/Linux, `lib.rs` L50).
  - API: `DesktopCapturer::new(DesktopCapturerOptions::new(Screen|Window))`, `set_include_cursor`, `get_source_list() -> Vec<CaptureSource{id,title,display_id}>`, `start_capture(Some(src), |Result<DesktopFrame,_>| …)`, then **we call `capture_frame()` on our own timer**.
  - `DesktopFrame` exposes `width/height/stride/left/top/data()`, and only CPU BGRA.
- C++: `REG/webrtc-sys-0.3.47/src/desktop_capturer.cpp`. Merged in [PR #725](https://github.com/livekit/rust-sdks/pull/725) (opened 2025-10-05 by the Hopp author; Zed's Be-ing tested Linux).
- Example: [`examples/screensharing/src/lib.rs`](https://github.com/livekit/rust-sdks/tree/main/examples/screensharing). It does capture thread → `yuv_helper::argb_to_i420` → `NativeVideoSource::new(res, true)` → publish `Screenshare` + `VideoCodec::VP9`.
- **Windows gaps:**
  - No HDR, no texture, no WGC-specific flags, and no `allow_wgc_capturer_fallback`.
  - Window resize means a new resolution. The example re-allocates the I420 buffer each time. LiveKit adapts to resolution changes in-stream.
- **Known issue [#1277](https://github.com/livekit/rust-sdks/issues/1277)** (open, 2026-07-23): on **Windows, each video `publish_track` retains ~40 MB until the Room closes**, and unpublish doesn't free it. Repeated start/stop share leaks. Mitigations: keep one published track and swap its source, or mute/unmute instead of unpublishing. **Confirm in the spike.**
- **Known issue [#1309](https://github.com/livekit/rust-sdks/issues/1309)** (open): on Windows, C++ exceptions in webrtc-sys abort the process instead of returning `Err`. Any capture-path failure in C++ is a crash risk, which is an argument for doing capture in Rust.

### 4.4 Pre-encoded pass-through (the hardware route)

- **API** (`REG/libwebrtc-0.3.50/src/video_source.rs` L92–180):
  - `new_encoded(res)`.
  - `capture_encoded_frame(&EncodedVideoFrame{codec, payload:&[u8], timestamp_us, frame_type: Key|Delta, resolution, frame_metadata})`.
  - `take_keyframe_request()` (PLI/FIR → force IDR).
  - `take_rate_control_request() -> Option<EncodedRateControl{target_bitrate_bps, framerate_fps}>` (GCC bandwidth estimate → reconfigure the HW encoder).
  - Publish with `video_encoder: VideoEncoderBackend::PreEncoded`.
- **C++:** `REG/webrtc-sys-0.3.47/src/passthrough_video_encoder.cpp`.
  - `supports_simulcast = false`, `scaling_settings = kOff`, `preferred_pixel_formats = {kNative}`.
  - It caches AV1 sequence headers and fills VP9 codec-specific info.
  - Matched by codec name only, so H.264 profile parameters don't constrain it (`video_encoder_factory.cpp` `FormatSupportedByBackendFactory`). An NVENC **High** profile stream will go out under whatever H.264 fmtp was negotiated. libwebrtc's FFmpeg decoder handles High; **verify with the web client if anyone views from a browser.**
- **Bugs that matter, and they are precisely our use case:**
  - **[PR #1459](https://github.com/livekit/rust-sdks/pull/1459)** (open, 2026-09-23). The pass-through lacks `has_trusted_rate_controller`, so WebRTC's frame dropper discards **delta** access units whenever the stream overshoots. That breaks the reference chain and causes freezes and PLI storms.
    - The measurement there is **Windows, 1080p60 H.264 from the Media Foundation hardware encoder (AMD), LiveKit server 1.13.6**: before the fix, min receiver fps 8, 24 freezes and 64 PLIs in 30 s; after it, 60 fps with 0 freezes.
    - A second user hit it with MF/NVIDIA in gameplay.
    - In the pinned 0.3.47, `GetEncoderInfo()` (L329–337) indeed lacks the flag.
  - **[PR #1511](https://github.com/livekit/rust-sdks/pull/1511)** (merged 2026-10-12). It sets the trusted-rate flag **and** adds a chain guard: after any dropped access unit, it drops deltas and requests a keyframe instead of sending corrupt video.
    - It likely lands in `webrtc-sys` ≥0.3.49 / `livekit` ≥0.9.5 (both published 2026-10-12). **Check the release before the spike.** If it's missing, it's a one-line `[patch]`.
  - **Single layer only.** No simulcast with pass-through, so the SFU can't give a weak viewer a lower layer. With ≤6 friends this is acceptable: the sender's GCC adapts to the slowest path the SFU reports, and LiveKit forwards a single stream.
- **The upstream encoder must:**
  1. Honour rate requests (Discord's lesson: average over a window, don't hard-cap per frame).
  2. Produce IDR only on request, plus a long periodic cadence (Discord: about 1/minute).
  3. Use no B-frames and Annex-B output with SPS/PPS on IDRs.
  4. Reconfigure bitrate **without** emitting a keyframe (the AMD/Discord trap).

### 4.5 What exists today vs fork vs C++

| Need | Today (0.9.3, no fork) | Small patch / `[patch]` | Real fork or C++ |
|---|---|---|---|
| Monitor/window capture (WGC/DXGI), cursor | ✅ `desktop_capturer` | | |
| No yellow border on Win11 | ✅ (libwebrtc default `wgc_require_border=false`) | | |
| WGC zero-Hz, include-secondary-windows, LUID, texture frames | ❌ | expose options in `desktop_capturer.cpp`/`.rs` | |
| Software VP8/VP9/H.264/AV1 encode | ✅ | | |
| Screencast tuning | ✅ `is_screencast`, `degradation_preference`, encodings, SVC | `content_hint` exposure (1 fn) | |
| HW encode on Windows inside libwebrtc | ❌ | PR #1282 (MF H.264) as a git patch | NVENC/AMF texture-input encoder inside webrtc-sys (C++) |
| HW encode outside libwebrtc (our pipeline) | ✅ `PreEncoded` | PR #1511/#1459 fix if not released | |
| D3D11 texture `NativeBuffer` | ❌ | | C++ (and pointless without an in-libwebrtc HW encoder) |
| Process-loopback audio | n/a (our code, `wasapi` crate) | | |
| Screenshare audio track | ✅ `NativeAudioSource` + `ScreenshareAudio` | | |
| Rebuilding libwebrtc itself | not needed | | only for e.g. FFmpeg HW or a newer WGC; avoid (Be-ing: "3 days of yak shaving") |

---

## 5. What others have built

| Project | Capture | Encode | Transport | Notes / source |
|---|---|---|---|---|
| **Discord Go Live** | Best OS method, fallback chain, **DLL injection** for some games (whitelisted). Audio from "the shared process and all its children" | Own capture+encode, HW preferred. "VP8 and H264, with HEVC and AV1 on specific platforms" | WebRTC to their SFU | [2024-03-05 overview](https://discord.com/blog/how-it-all-goes-live-an-overview-of-discords-streaming-technology). [2024-05-01 AMD quality](https://discord.com/blog/from-blocky-to-brilliant-improving-video-quality-on-discord-go-live-on-amd-gpus): per-frame bit caps, keyframe on every rate change, and the WebRTC frame dropper halving fps. Fixes moved sessions below 50 fps from 39% to 18% |
| **Zed** | Windows: `scap` fork (WGC), [PR #34223](https://github.com/zed-industries/zed/pull/34223) merged 2025-07-10. Linux: libwebrtc DesktopCapturer via LiveKit Rust. A full switch to libwebrtc capture ([#42670](https://github.com/zed-industries/zed/pull/42670)) was closed | libwebrtc sw | LiveKit Rust SDK | Proves LiveKit Rust screen share works in a shipping app (sharing text/code, not games) |
| **Hopp** (pair programming) | libwebrtc DesktopCapturer (author of SDK PR #725), own libwebrtc fork | libwebrtc | LiveKit Rust | [PR #725](https://github.com/livekit/rust-sdks/pull/725) |
| **Sunshine** (Moonlight host) | `ddx` (DXGI DDA) default; `wgc` beta, incompatible with its service mode; sets `IsBorderRequired(false)` | NVENC/AMF/QSV direct from D3D11 textures; HEVC/AV1 with HDR 10-bit | Custom RTP | [docs/configuration.md](https://github.com/LizardByte/Sunshine/blob/master/docs/configuration.md), `src/platform/windows/display_wgc.cpp` |
| **OBS** | Game hook + WGC + DDA; FP16 for HDR monitors; Borderless request on Win11 | NVENC/AMF/QSV via texture input (zero-copy) | n/a | `libobs-winrt/winrt-capture.cpp` |
| **Parsec** (~2016 post) | Desktop Duplication ("direct framebuffer grab"); never touches system memory | Vendor SDKs directly (NVIDIA/AMD/Intel), HW only; <10 ms enc+dec | Own UDP (BUD) | [description of Parsec technology](https://parsec.app/blog/description-of-parsec-technology-b2738dcc3842) |
| **RustDesk** | `libs/scrap` DXGI (+ GDI fallback) | `hwcodec` crate: NVENC/AMF/QSV, "VRAM" path from D3D11 textures; dropped CUDA-context codecs over freezes | Own | [rustdesk/libs/scrap](https://github.com/rustdesk/rustdesk/tree/master/libs/scrap), hwcodec README mirror |
| **windows-capture** crate | WGC + DDA, texture access | WinRT `MediaTranscoder` (file-style, not RTC) | n/a | [GitHub](https://github.com/NiiightmareXD/windows-capture), 2.0.1 2026-08-08 |
| **LiveKit PR #1282 / #1459 authors** | (Windows apps already publishing MF HW H.264 through the pass-through into LiveKit 1.13.x) | MF async MFT | LiveKit Rust | Shows that our exact Phase-B architecture is being used by others right now |

---

## 6. Recommendations (each with its risk)

**R1. Capture with WGC, in Rust (`windows-capture` crate or our own `windows`-crate code), not with the SDK's DesktopCapturer, for the production path.**

- **Why:**
  - Zero-copy textures.
  - Border off on Win11.
  - Cursor toggle.
  - FP16 for HDR.
  - Works on hybrid laptops.
  - No C++ exceptions aborting the process (#1309).
  - Anti-cheat safe.
- **Risks:**
  - The Win10 yellow border can't be removed. That is cosmetic; tell users.
  - True-FSE games may give black frames. Mitigation: tell users "borderless windowed".
  - Vanguard/Valorant behaviour is unverified.
- **Fallback:** DXGI DDA for monitors when WGC fails. Not on hybrid laptops.

**R2. Encode H.264 on the GPU ourselves and publish via `PreEncoded`.**

- **Primary:** a Media Foundation H.264 encoder MFT with D3D11 input (`IMFDXGIDeviceManager`, `MFCreateDXGISurfaceBuffer`). One code path covers NVENC, AMF and QSV, with no vendor SDKs and no DLLs to ship. GPU work: `ID3D11VideoProcessor` for BGRA→NV12 and scaling, plus a shader for HDR tone mapping.
- **Optional later:** direct NVENC (NVIDIA Video Codec SDK, texture input) for better rate control and lower latency on NVIDIA. Most of the group is likely NVIDIA (ShadowPlay users).
- **Risks:**
  - MF vendor-MFT quirks: AMD rate control (Discord's post), SPS/PPS not inline, async event pump, bitrate change forcing an IDR.
  - The pass-through frame-dropper bug (#1459/#1511): must be on a release with the fix, or patched.
  - Single layer, no simulcast.
  - H.264 High vs negotiated Constrained Baseline fmtp: fine for libwebrtc receivers, verify for browsers.
- **Alternative if MF is painful:** FFmpeg (`h264_nvenc`/`h264_amf`/`h264_qsv` with d3d11va hwframes) via `ffmpeg-next`. Cost: ~80–100 MB of LGPL DLLs to ship, against the spec's footprint budget.

**R3. Phase A (optional, quick win): ship "share a window/desktop for text" with the SDK `DesktopCapturer` + `is_screencast=true` + VP9 + `MaintainResolution`, 1080p ≤15–30 fps, ~2–3 Mbps, simulcast off.**

- **Risks:**
  - Software CPU cost on weaker PCs.
  - Not suitable for games.
  - The ~40 MB-per-publish leak (#1277), so reuse one track.
  - Throwaway once Phase B lands, except the publish/UI/subscription plumbing, which carries over.

**R4. Audio: process loopback via the `wasapi` crate.**

- **Modes:**
  - "Window share" means INCLUDE the window's PID tree (`GetWindowThreadProcessId`).
  - "Screen share" means EXCLUDE Pulse's PID (system minus us, no echo).
- Publish as `ScreenshareAudio`, stereo 48 kHz, 96–128 kbps, no APM/denoise/DTX, same `stream` id as the video.
- **Risks:**
  - Documented minimum build 20348, while friends on Win10 22H2 (19045) rely on the "works on older builds" reports. Needs a test. Fallback: endpoint loopback, which has the echo problem; mitigate by subtracting our own output (hard) or by telling users to wear headphones.
  - Launchers whose game isn't a child process (e.g. Steam → game is a Steam child, but EA/Riot clients spawn differently). The picker should map window → PID, not launcher → PID.
  - Sonar routing (verify).

**R5. Rate and quality defaults for games (1080p60).**

- Settings: H.264 HW CBR-ish with a ~0.5–1 s VBV window, 6 Mbps start, `max_bitrate` 8 Mbps, `max_framerate` 60, `MaintainFramerate`, IDR on PLI plus every 60 s, no B-frames, low-latency mode.
- For text: VP9 sw or H.264 HW at 1080p30, 3–4 Mbps, `MaintainResolution`.
- **Risk:** SA uplinks. Friends on asymmetric fibre or LTE may only have 5–10 Mbps up, and voice must not starve. The 1-OCPU Oracle box forwarding 6 Mbps × 5 viewers is ~30 Mbps egress per share: fine for CPU (an SFU forwards, it doesn't transcode), but **watch the free-tier egress allowance** (10 TB/month free on OCI) and the box's per-OCPU network cap.

**R6. Avoid game hooks completely, and avoid rebuilding libwebrtc.**

- **Risk of ignoring this:** anti-cheat bans for friends; days of build-system yak shaving.

---

## 7. Unknowns a 1–2 day spike must answer

Run on Windows: one NVIDIA desktop, one AMD (ideally a friend's), and one Optimus laptop if available. LiveKit 1.13.7 on the Oracle box.

1. **SDK baseline (day 1 AM).** Build `examples/screensharing` against our pinned livekit. Measure CPU % and achieved fps at 1080p30/60 for VP8, VP9, H.264 and AV1 software, with desktop text and with a game. Use the `lk` CLI or a second Pulse as viewer.
   - Is VP9 sw at 1080p30 under ~1 core? → decides whether Phase A is shippable.
2. **The #1277 leak:** publish/unpublish 5× and read process memory. Does mute or source-swap avoid it?
3. **WGC via `windows-capture`:**
   - Is the border removable on Win11 from an **unpackaged** exe (RequestAccessAsync(Borderless) + `IsBorderRequired(false)`)? Does a consent prompt appear?
   - Window capture of a borderless game: are delivered fps at 60/144 Hz stable?
   - Cursor toggle.
   - Behaviour with a true-exclusive-fullscreen title.
4. **Anti-cheat smoke test:** WGC window/monitor capture while Valorant (Vanguard) and one EAC title (e.g. Fortnite/Apex) run. Do we get black frames, a kick, or does it work?
5. **MF H.264 MFT on NVIDIA and AMD:**
   - Does it enumerate as hardware/async?
   - Does it accept D3D11 NV12 textures (no readback)?
   - Can bitrate change at runtime **without** an IDR?
   - Is SPS/PPS inline on IDR?
   - Latency per frame.
   - If MF is bad on AMD: try AMF via FFmpeg as a comparison.
6. **Pass-through end to end:**
   - `new_encoded` + `capture_encoded_frame` at 1080p60 over the real Oracle server.
   - Does our pinned version have #1511? If not, `[patch]` it and compare freezes/PLIs (reproduce #1459's table).
   - Keyframe-request latency.
   - Rate-request cadence.
7. **H.264 profile negotiation:** what fmtp does LiveKit 1.13.7 negotiate for a Rust publisher? Does a High-profile pass-through stream decode in a Pulse receiver and in a browser (meet.livekit.io)?
8. **Process loopback:**
   - Works on the oldest Win10 build in the group (likely 19045)?
   - INCLUDE for a game PID tree.
   - EXCLUDE for Pulse's PID: confirm no echo of friends' voices.
   - Behaviour with SteelSeries Sonar (Game device) installed.
   - Stereo arrives as 2 channels at the receiver.
   - A/V sync drift with a shared `stream` id.
9. **HDR:** does any friend run Windows HDR? If yes: FP16 capture plus a tone-map shader prototype, compared against BGRA8 capture.
10. **Receiver side** (out of this doc's scope, but it blocks the feature): software decode of 1080p60 H.264/VP9 and the cost of getting frames into the UI.
    - WebView2 canvas via IPC is expensive. A native child window/wgpu surface is likely needed.
    - Measure the decode plus present CPU.
11. **Bandwidth:** each friend's real uplink (`speedtest` to Johannesburg). Does voice survive a concurrent 6 Mbps share? Does the LiveKit server need `congestion_control` tuning?

---

## 8. Sources

**Local (pinned dependencies):**

- `REG/webrtc-sys-0.3.47/build.rs`: platform arms. NVENC/VAAPI/Jetson are Linux-only; the Windows VAAPI block is commented out; `desktop_capturer.cpp` is built on desktop.
- `REG/webrtc-sys-0.3.47/src/video_encoder_factory.cpp`: backends `PreEncoded`/VideoToolbox/Android/Jetson/NVENC/VAAPI, the `LIVEKIT_PREFERRED_HW_ENCODER` env, and the OpenH264/libaom/libvpx template factory.
- `REG/webrtc-sys-0.3.47/src/video_decoder_factory.cpp`: Windows sw decoders (VP8/VP9/FFmpeg H.264/dav1d).
- `REG/webrtc-sys-0.3.47/src/passthrough_video_encoder.cpp` L185–337: pass-through; `supports_simulcast=false`; no trusted rate controller.
- `REG/webrtc-sys-0.3.47/src/desktop_capturer.{rs,cpp}`, `include/livekit/desktop_capturer.h`: DesktopCapturer bridge and the WGC/DXGI options set.
- `REG/webrtc-sys-0.3.47/src/video_track.rs` L24, L109–110: `content_hint` bound in ffi.
- `REG/webrtc-sys-0.3.47/libwebrtc/build_windows.cmd`: GN args.
- `REG/webrtc-sys-build-0.3.19/src/lib.rs` L31: `WEBRTC_TAG = "webrtc-89d790b"`.
- `WINLIB/args.gn`, `WINLIB/webrtc.ninja` defines, `WINLIB/include/modules/desktop_capture/desktop_capture_options.h` L186–253.
- `REG/libwebrtc-0.3.50/src/desktop_capturer.rs`, `src/lib.rs` L50.
- `REG/libwebrtc-0.3.50/src/video_source.rs` L86–190, `src/native/video_source.rs` L100–235: `new`/`new_encoded`/`capture_encoded_frame`/keyframe and rate requests.
- `REG/libwebrtc-0.3.50/src/video_frame.rs` L36–135 and L565–650: buffer types, `EncodedVideoFrame`, `NativeBuffer` (CVPixelBuffer/DMA-BUF only).
- `REG/libwebrtc-0.3.50/src/rtp_parameters.rs` L24–90: `DegradationPreference` (swapped doc comments).
- `REG/libwebrtc-0.3.50/src/rtp_sender.rs` L25: `VideoEncoderBackend`.
- `REG/libwebrtc-0.3.50/src/audio_source.rs` L22, L181: `AudioSourceOptions`, `NativeAudioSource::new`.
- `REG/livekit-0.9.3/src/room/options.rs`: `TrackPublishOptions` L117–154, degradation defaults, `compute_video_encodings`, screenshare presets, audio presets.
- `REG/livekit-0.9.3/src/room/track/mod.rs` L67: `ScreenshareAudio`.
- `REG/livekit-0.9.3/src/rtc_engine/peer_transport.rs` L339–382: stereo munging.

**LiveKit upstream:**

- [examples/screensharing](https://github.com/livekit/rust-sdks/tree/main/examples/screensharing)
- [PR #725 Expose desktop capturer](https://github.com/livekit/rust-sdks/pull/725) (2025-10-05, merged)
- [PR #1282 MF hardware H.264 on Windows](https://github.com/livekit/rust-sdks/pull/1282) (2026-07-26, open)
- [PR #1418 pre-encoded ingest FFI](https://github.com/livekit/rust-sdks/pull/1418) (2026-09-09, merged)
- [PR #1459 passthrough trusted rate controller](https://github.com/livekit/rust-sdks/pull/1459) (2026-09-23, open; Windows MF/AMD measurements)
- [PR #1511 keep pre-encoded decodable when frames drop](https://github.com/livekit/rust-sdks/pull/1511) (merged 2026-10-12)
- [Issue #1277 Windows video publish retains ~40 MB](https://github.com/livekit/rust-sdks/issues/1277) (2026-07-23, open)
- [Issue #1309 Windows C++ exceptions abort](https://github.com/livekit/rust-sdks/issues/1309) (2026-08-04, open)
- crates.io: webrtc-sys 0.3.49 / livekit 0.9.5 / libwebrtc 0.3.52, all 2026-10-12.

**Microsoft:**

- [GraphicsCaptureSession](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession) (1803; IsCursorCaptureEnabled 2004)
- [IsBorderRequired](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired) (20348; consent semantics)
- [GraphicsCaptureAccess.RequestAccessAsync](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscaptureaccess.requestaccessasync?view=winrt-20348)
- [AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_process_loopback_params) (min build 20348)
- [PROCESS_LOOPBACK_MODE](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ne-audioclientactivationparams-process_loopback_mode)
- [DDA on hybrid GPU fails by design](https://learn.microsoft.com/en-us/troubleshoot/windows-client/shell-experience/error-when-dda-capable-app-is-against-gpu)

**Others:**

- Discord: [How It All Goes Live](https://discord.com/blog/how-it-all-goes-live-an-overview-of-discords-streaming-technology) (2024-03-05, Josh Stratton); [From Blocky to Brilliant (AMD)](https://discord.com/blog/from-blocky-to-brilliant-improving-video-quality-on-discord-go-live-on-amd-gpus) (2024-05-01, Alex Converse).
- [Parsec: description of Parsec technology](https://parsec.app/blog/description-of-parsec-technology-b2738dcc3842) (undated, ~2016).
- Sunshine: [configuration.md capture table](https://github.com/LizardByte/Sunshine/blob/master/docs/configuration.md); `src/platform/windows/display_wgc.cpp` L150–156.
- OBS: `libobs-winrt/winrt-capture.cpp` (Borderless request L316–320, FP16 L79–91); [WGC vs DXGI forum](https://obsproject.com/forum/threads/windows-graphics-capture-vs-dxgi-desktop-duplication.149320).
- Zed: [PR #34223 Windows screen sharing](https://github.com/zed-industries/zed/pull/34223) (merged 2025-07-10); [PR #42670 libwebrtc DesktopCapturer](https://github.com/zed-industries/zed/pull/42670) (closed).
- Rust crates: [windows-capture](https://github.com/NiiightmareXD/windows-capture) 2.0.1; [wasapi-rs](https://github.com/HEnquist/wasapi-rs) 0.26.0 `src/api.rs` L770–821; [RustDesk scrap](https://github.com/rustdesk/rustdesk/tree/master/libs/scrap).
- [freemancw: Desktop Duplication on Hybrid Graphics](https://www.freemancw.com/2020/11/desktop-duplication-on-hybrid-graphics-systems/) (2020-11).
- Anti-cheat vs hooks: OBS forum threads and third-party guides only. **No primary source from Riot or Epic was found; treat this as unverified.**
