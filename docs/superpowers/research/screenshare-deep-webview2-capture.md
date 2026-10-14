# Screen share deep dive: capture inside WebView2, and every way it can break

**Research date:** 2026-10-14. **Scope:** research only, for the screen-share spike. This goes deeper than
`screenshare-capture-encode.md` and `screenshare-summary.md` on one question: if Pulse captures and encodes game video
inside WebView2 (`getDisplayMedia` + `livekit-client`), what can go wrong on a friend's Windows PC?

**What I read (all on 2026-10-14):**

- Upstream Chromium `main` through the GitHub mirror (`github.com/chromium/chromium`, head `a70c0e3`, 2026-10-14).
  Chrome Stable on Windows is 155 ([chromiumdash](https://chromiumdash.appspot.com/fetch_releases?channel=Stable&platform=Windows&num=3)).
- WebRTC `main` at `webrtc.googlesource.com/src`.
- Microsoft WebView2 docs (SDK reference 1.0.4258.31) and the `MicrosoftEdge/WebView2Feedback` issue tracker.
- `wry` @ `6642074` (2026-10-12), `tauri` @ `94ffd77` (2026-10-14), `livekit-client` 2.22.4 @ `930942c` (2026-10-12),
  and `webview2-com-sys` 0.39.1, which is the version in Pulse's `Cargo.lock`.
- Pulse's own `client/src-tauri/src/lib.rs` and `tauri.conf.json`.

**Labels:** **[cited]** = read in source or docs (link given). **[measured]** = run here (there are none: this machine is
Linux and has no Windows). **[est.]** = my estimate from cited facts. **[speculation]** = plausible but unverified.

**One caveat applies to everything below:** WebView2 is built from Microsoft Edge, not from Chrome. Edge is Chromium
plus Microsoft's own patches.

- Upstream Chromium code is strong evidence for what WebView2 does, but it is not proof.
- A live example of the gap: Edge wraps the hardware video decoder in its own `EdgeD3D11VdaHybridVideoDecoderWrapper`,
  which Chrome does not have, and that wrapper currently breaks WebRTC hardware decode
  ([WebView2Feedback #5755](https://github.com/MicrosoftEdge/WebView2Feedback/issues/5755), opened 2026-10-11) **[cited]**.
- So every source-based claim here is also a spike test case.

---

## 0. TL;DR

1. **`getDisplayMedia` works in WebView2, but WebView2 shows its own picker and Pulse cannot choose the source.**
   - The host's only hook is `ScreenCaptureStarting` (`ICoreWebView2_27`, SDK 1.0.2903.40). It can allow, cancel or
     defer the picker, but it cannot pick a source or skip the picker.
   - `PermissionRequested` has no screen-capture kind.
   - The picker is drawn inside the webview's own rectangle. Since runtime 134 it is unusable if that rectangle is
     shorter than 600 px. **Pulse's minimum window height is 560 px.**
2. **The capture backend is not always WGC.**
   - Upstream Chromium uses **WGC for monitor capture only on Windows 11 24H2 and newer**. On Windows 10 22H2 and
     Windows 11 23H2, monitor capture uses **DXGI Desktop Duplication**.
   - Window capture uses WGC from Windows 10 2004 onwards. On Windows 10 it always draws the yellow border.
   - This corrects the summary's "Chromium captures with WGC".
3. **60 fps is allowed, but you have to ask for it, and it isn't guaranteed.**
   - `getDisplayMedia` defaults to 30 fps and is capped at 120.
   - Chromium polls the capturer on a timer, copies each frame to CPU memory, converts it to I420, and then uploads it
     again for the hardware encoder.
   - It also caps capture at 50% of one CPU core, and lowers the frame rate when a capture takes longer than that.
   - 1080p60 is likely fine. 1440p60 is borderline **[est.]**.
4. **Minimised or occluded Pulse is probably safe for capture and encode, but it must be measured.**
   - Capture runs on a browser-process thread, and encode runs on WebRTC threads and in the GPU process. Neither is
     tied to page visibility **[cited]**.
   - A renderer with an active media stream is never put into background priority **[cited]**.
   - Any live `MediaStreamTrack` or open `RTCDataChannel` opts the page out of intensive timer throttling **[cited]**.
   - The remaining risk is JavaScript timers aligned to 1 per second. Flags remove that.
5. **A hidden second webview is a bad idea.** The picker would be drawn inside an invisible webview. Run the hidden
   video participant as a **second `Room` object in the main page** instead.
6. **Hardware H.264 encode is on by default upstream**, through the Media Foundation encoder for NVENC, AMF and QSV.
   It has software fallbacks to know about:
   - Simulcast layers below 360p go to software on purpose.
   - Odd frame sizes go to software.
   - A missing OpenH264 disables H.264 hardware encode too.

   You can check it at runtime with `getStats().encoderImplementation`, which shows
   `"MediaFoundationVideoEncodeAccelerator (…)"`, but only while the page is capturing.
7. **Anti-cheat:** there are no primary sources from Riot, Epic, BattlEye, FACEIT or Activision.
   - WGC and DXGI are OS APIs. The capturing process is Microsoft-signed `msedgewebview2.exe`, the same binary that
     shares a screen from Edge.
   - That makes a ban very unlikely **[speculation]**. Black frames are the realistic failure.
   - Test Vanguard and one EAC title.

---

## 1. Does WebView2 support `getDisplayMedia`, and who controls the picker?

### 1.1 Support

- **Yes, `getDisplayMedia` is supported, and WebView2 draws its own picker.**
  - Bug reports describe the picker as a dialog drawn *inside the WebView2's bounds*. It is clipped when the WebView2
    control is smaller than the window ([#2184](https://github.com/MicrosoftEdge/WebView2Feedback/issues/2184),
    2022, still open) **[cited]**.
  - It is "unusably small if the parent window has a height less than 600px" from runtime 134 onwards
    ([#5173](https://github.com/MicrosoftEdge/WebView2Feedback/issues/5173), 2025, tracked internally, still open)
    **[cited]**.
- **Older bugs to re-check** (all still open):
  - Only the primary monitor was listed
    ([#1850](https://github.com/MicrosoftEdge/WebView2Feedback/issues/1850), 2021).
  - A freeze happened when switching between the app and the "sharing" notification window
    ([#2258](https://github.com/MicrosoftEdge/WebView2Feedback/issues/2258), 2022).
  - Both are **[cited]**, but I don't know whether they still reproduce.
- **A "you are sharing" notification window appears.** #2258 shows that WebView2, like Chrome, opens a small separate
  "sharing" window while capturing **[cited]**.
  - In upstream Chromium, the `getDisplayMedia` handler is built with `display_notification = true`
    ([display_media_access_handler.cc](https://github.com/chromium/chromium/blob/main/chrome/browser/media/webrtc/display_media_access_handler.cc))
    **[cited]**. I found no supported flag to suppress it.
  - Over a fullscreen game it may steal focus or flash **[speculation]**. This is a test case.
- **Pulse is a secure context, so the API is exposed.**
  - Tauri serves the UI from `http://tauri.localhost` (`use_https_scheme` defaults to false in
    `tauri-utils/src/config.rs`).
  - Chromium treats any `*.localhost` host as localhost (`IsLocalHostname` → `IsNormalizedLocalhostTLD` in
    [net/base/url_util.cc](https://github.com/chromium/chromium/blob/main/net/base/url_util.cc)) **[cited]**.
- **A user gesture is not required upstream.** `GetDisplayMediaRequiresUserActivation` has status `"experimental"`,
  which means off in release builds
  ([runtime_enabled_features.json5](https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/platform/runtime_enabled_features.json5);
  check in [media_devices.cc](https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/modules/mediastream/media_devices.cc))
  **[cited]**. Edge might differ.
- **Pulse's CSP blocks LiveKit today.** `tauri.conf.json` has `connect-src ipc: http://ipc.localhost`. `livekit-client`
  needs `wss://` and `https://` to the LiveKit host added to `connect-src` **[cited, Pulse repo]**.

### 1.2 What the host can control

| Mechanism | What it does | Can it choose the source or skip the picker? |
|---|---|---|
| `ICoreWebView2_27::add_ScreenCaptureStarting` (SDK 1.0.2903.40) | Fires when a page calls `getDisplayMedia`, *before* the picker. The arguments have `Cancel`, `Handled`, `OriginalSourceFrameInfo` and `GetDeferral()`. Cancelling shows no UI, and the page gets `NotAllowedError`. A deferral delays "the decision to show the Screen Capture UI". ([ICoreWebView2_27](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_27), [event args](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2screencapturestartingeventargs)) **[cited]** | **No.** Allow, cancel or defer only. |
| `PermissionRequested` / `COREWEBVIEW2_PERMISSION_KIND` | The kinds in `webview2-com-sys` 0.39.1 are AUTOPLAY, CAMERA, CLIPBOARD_READ, FILE_READ_WRITE, GEOLOCATION, LOCAL_FONTS, MICROPHONE, MIDI_SYSEX, MULTIPLE_AUTOMATIC_DOWNLOADS, NOTIFICATIONS, OTHER_SENSORS, UNKNOWN and WINDOW_MANAGEMENT (`src/bindings.rs`) **[cited]** | **No.** There is no screen-capture kind. |
| `--auto-select-desktop-capture-source=<substr>`, `--auto-select-window-capture-source-by-title=<substr>`, `--auto-select-screen-capture-source` | Chrome-layer test switches. The picker's list controller auto-picks the first source whose name contains the substring ([chrome_switches.h](https://github.com/chromium/chromium/blob/main/chrome/common/chrome_switches.h), [desktop_media_list_controller.cc](https://github.com/chromium/chromium/blob/main/chrome/browser/ui/views/desktop_capture/desktop_media_list_controller.cc)) **[cited]** | **Maybe, but impractical.** Process-wide and fixed when the browser starts. Changing them means a *new WebView2 environment with its own user-data folder*, which Tauri also warns about. Unknown whether Edge's picker reads them **[speculation]**. Spike experiment only. |
| `--use-fake-ui-for-media-stream` | Skips the prompt. For `getDisplayMedia` it returns a *screen* with a null id, or a window or tab only if a fake device is configured ([media_stream_manager.cc](https://github.com/chromium/chromium/blob/main/content/browser/renderer_host/media/media_stream_manager.cc) `DisplayMediaDevicesFromFakeDeviceConfig`) **[cited]**. It also auto-grants camera and microphone. Microsoft lists it as a dev-only flag ([WebView2 browser flags](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/webview-features-flags)) **[cited]** | **No useful choice.** It can't pick a game window. |
| Legacy `getUserMedia({video:{mandatory:{chromeMediaSource:'desktop', chromeMediaSourceId}}})` | In the Chrome layer, the id must come from `DesktopStreamsRegistry` (the extension `chooseDesktopMedia` API), or the call fails with `STREAM_NOT_FOUND_IN_REGISTRY`. With no id, it needs `--enable-usermedia-screen-capturing`, shows a yes/no box, and captures the *whole virtual desktop* ([desktop_capture_access_handler.cc](https://github.com/chromium/chromium/blob/main/chrome/browser/media/webrtc/desktop_capture_access_handler.cc)) **[cited]**. Electron's `desktopCapturer` ids work only because Electron replaces this handler. | **No.** |
| `CaptureController`, `displaySurface`, `selfBrowserSurface`, `monitorTypeSurfaces`, `surfaceSwitching`, `preferCurrentTab` | Hints to the picker. They can preselect the Window or Screen tab, and set focus behaviour after the pick (`CaptureController` is "stable") **[cited]** | **No.** They change the picker, not the choice. |
| **TextureStream** (`CoreWebView2TextureStream`, `msWebView2TextureStream` flag) | The host pushes D3D11 textures and the page gets a `MediaStream`. This would allow Rust WGC capture with our own picker, then WebView2 hardware encode. However: experimental since prerelease 1.0.1724 (2023), still only in prerelease SDKs (1.0.4255), NV12 only ([#3591](https://github.com/MicrosoftEdge/WebView2Feedback/issues/3591)). Experimental APIs need an Edge preview channel, "not the Evergreen WebView2 Runtime" ([versioning](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/versioning), [API](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/winrt/microsoft_web_webview2_core/corewebview2texturestream)) **[cited]** | **Yes, but it can't ship** to friends on stable runtimes. |
| **Shared buffer + breakout box** (`ICoreWebView2Environment12::CreateSharedBuffer`, `ICoreWebView2_17::PostSharedBufferToScript`, both stable and both in `webview2-com-sys` 0.39.1) | Rust owns the picker and the WGC capture (with thumbnails), and writes NV12 frames into shared memory. JS wraps them in `VideoFrame` → `MediaStreamTrackGenerator` → LiveKit, which still uses WebView2's hardware encoder. The `BreakoutBoxPreferCaptureTimestampInVideoFrames` flag on Microsoft's list suggests the breakout box exists in WebView2 **[cited]**. | **Yes.** It costs one CPU copy per frame (1080p NV12 is 3.1 MB, about 190 MB/s at 60 fps) plus a GPU upload **[est.]**. This is **Plan B** if the built-in picker or capture path fails the spike. |

### 1.3 Recommendation for the picker

- **v1:** use WebView2's built-in picker, gated by `ScreenCaptureStarting`.
  - Reach it in Rust with `win.with_webview(|wv| wv.controller().CoreWebView2()?.cast::<ICoreWebView2_27>())`.
  - Use it to:
    - cancel any `getDisplayMedia` that doesn't come from our "Go Live" flow;
    - log the call;
    - make sure the window is at least 600 px tall before the picker shows (a deferral gives time to resize).
- Pass `displaySurface: 'window'` so the Window tab opens first, and `selfBrowserSurface: 'exclude'`.
- **Fix the 600 px trap:** raise `min_inner_size` from 560 to at least 640, or grow the window inside the deferral.
- **Plan B** (if the picker is clipped, misses monitors or misbehaves): use our own Rust picker with the
  shared-buffer bridge. Do not depend on TextureStream until Microsoft makes it stable.

---

## 2. The capture backend: WGC or DXGI, frame rate, and the edge cases

### 2.1 Which backend Chromium uses

The source is `DesktopCaptureDevice::Create` in
[desktop_capture_device.cc](https://github.com/chromium/chromium/blob/main/content/browser/media/capture/desktop_capture_device.cc)
and `desktop_capture::CreateDesktopCaptureOptions` in
[desktop_capture.cc](https://github.com/chromium/chromium/blob/main/content/public/browser/desktop_capture.cc), with the
factory in WebRTC's
[desktop_capturer.cc](https://webrtc.googlesource.com/src/+/refs/heads/main/modules/desktop_capture/desktop_capturer.cc)
**[cited]**:

| Windows version | Monitor (screen) capture | Window capture | Yellow border |
|---|---|---|---|
| Windows 10 22H2 (19045) | **DXGI Desktop Duplication** (`allow_directx_capturer`), with GDI as fallback | **WGC**, with GDI fallback (`allow_wgc_capturer_fallback`) | Window capture: **always**. `IsBorderRequired` needs build 20348+, and Chromium only sets it when `IGraphicsCaptureSession3` exists. Monitor: none (DXGI). |
| Windows 11 21H2–23H2 | **DXGI** | **WGC** | None. Chromium sets `IsBorderRequired(false)` because `kWebRtcWgcRequireBorder` is off by default. |
| Windows 11 24H2+ (26100) | **WGC**, with "0 Hz" (no frames when nothing changes) | **WGC** | None |

- The 24H2 rule is `IsWgcEnabledForScreenCapture()`, which returns `GetVersion() >= WIN11_24H2`.
  - It was added in commit `e95d19499b` ("Enable WGC for screen capture of getDisplayMedia on Windows 24H2", May 2025,
    M138), written by a Microsoft engineer. The message says earlier WGC lacked 0 Hz support, and that "WGC will
    address the color space issue on the HDR monitor" **[cited]**.
  - The old `AllowWgcScreenCapturer` feature that could force WGC on older Windows was deleted in M151 (`f4a27845e6`,
    June 2026) **[cited]**. **No flag forces WGC monitor capture on Windows 10 or 23H2 any more.**
- WGC window capture needs UniversalApiContract 8 (`kRequiredApiContractVersion = 8` in
  [wgc_capturer_win.cc](https://webrtc.googlesource.com/src/+/refs/heads/main/modules/desktop_capture/win/wgc_capturer_win.cc)),
  which means Windows 10 2004 or later. Windows 10 22H2 qualifies **[cited]**.
- **Windows 11 border detail.**
  - Microsoft's docs say turning the border off needs `RequestAccessAsync(Borderless)` consent and the
    `graphicsCaptureWithoutBorder` capability in a package manifest. Otherwise "the value will be ignored"
    ([IsBorderRequired](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired))
    **[cited]**.
  - Chromium never calls `RequestAccessAsync`, yet Chrome on Windows 11 shows no border in practice. Unpackaged Win32
    callers appear to be exempt **[est.]**. Verify in WebView2.

### 2.2 Frame rate and frame pacing

- **Defaults and caps.**
  - `getDisplayMedia` defaults to 30 fps (`kDefaultScreenCastFrameRate = MediaStreamVideoSource::kDefaultFrameRate`)
    and is capped at `kMaxScreenCastFrameRate = 120.0`.
  - The default size cap is **2880×1800**
    ([media_stream_constraints_util_video_content.cc](https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/modules/mediastream/media_stream_constraints_util_video_content.cc))
    **[cited]**.
  - **Always ask for `frameRate: 60` and explicit width/height.**
- **Capture is polled on a timer, not driven by frame arrival.**
  - `AllocateAndStart` sets a 1 ms `ScopedHighResolutionTimer`.
  - `ScheduleNextCaptureFrame` arms a one-shot timer for
    `max(last_capture_duration × 100 / 50, requested_frame_duration)`.
  - `kDefaultMaximumCpuConsumptionPercentage = 50`: if one capture takes over **8.3 ms**, the frame rate falls below
    60 **[cited]**.
  - Polling at 60 Hz against a game presenting at 60–144 Hz means "latest available frame". Expect occasional
    duplicate or skipped frames, so motion looks less smooth than an OBS recording **[est.]**.
  - Capturing a 144 Hz game at 60 fps samples better than capturing a 60 Hz game at 60 fps **[est.]**.
- **WGC session settings Chromium uses**
  ([wgc_capture_session.cc](https://webrtc.googlesource.com/src/+/refs/heads/main/modules/desktop_capture/win/wgc_capture_session.cc))
  **[cited]**:
  - pixel format `B8G8R8A8UIntNormalized`;
  - a free-threaded frame pool;
  - no `MinUpdateInterval`, so WGC itself delivers at the display rate;
  - `IncludeSecondaryWindows` on 24H2 (default off).
- **Data path (CPU round trip).**
  - Today, WGC frames are copied to a staging texture and `Map`ped into CPU memory. DXGI frames are copied to CPU too.
  - `DesktopCaptureDevice` then converts BGRA to I420 on the CPU.
  - The encoder copies I420 into an NV12 shared image for Media Foundation. You can see the `CreateClientSharedImage`
    CPU-write usage in
    [rtc_video_encoder.cc](https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/platform/peerconnection/rtc_video_encoder.cc)
    **[cited]**.
  - The zero-copy options exist but are **off by default** **[cited]**:
    - `kWebRtcAllowWgcUsingTexture` (M151, Windows 11 24H2+ only, because it needs `DoesWgcSkipStaticFrames()`);
    - `kZeroCopyDesktopCapture` ([media_switches.cc](https://github.com/chromium/chromium/blob/main/media/base/media_switches.cc),
      [webrtc_features.cc](https://github.com/chromium/chromium/blob/main/media/webrtc/webrtc_features.cc)).
- **Budget estimate [est.].**
  - 1440p BGRA is 14.7 MB per frame, so 60 fps is about 0.9 GB/s of GPU→CPU readback plus about 2–4 ms of
    BGRA→I420 per frame.
  - On a modern 6–8-core CPU that is probably 4–8 ms per capture call: right at the 8.3 ms ceiling.
  - 1080p (8.3 MB per frame) has headroom. A 4K monitor at 60 fps will likely not hold 60.
  - So prefer **window capture of a 1080p or 1440p game window** over 4K monitor capture.

### 2.3 Edge cases

| Case | What happens | Label |
|---|---|---|
| **Cursor** | `prefer_cursor_embedded(true)` plus `DesktopAndCursorComposer`. The cursor is in the video (WGC draws it, or it is composited on the CPU for DXGI). The texture path forces embedding. FPS games that hide the OS cursor are fine. | [cited] `Create()` / `wgc_capture_session.cc` |
| **Minimised game window (window capture)** | `ShouldBeCapturable()` is `IsWindow && IsWindowVisible && !IsIconic`. While minimised, `GetFrame` skips `EnsureFrame` and returns the last frame, so **viewers see a frozen image**, not an error. | [cited] `window_capture_utils.cc`, `wgc_capture_session.cc` |
| **Borderless / "fullscreen optimisations" games** | Composed by DWM, so WGC window capture and monitor capture both work. | [est.] (matches OBS and Sunshine experience in the earlier report) |
| **True exclusive fullscreen** (old DX9/DX11 FSE, FSO disabled) | WGC window capture: likely black or frozen. DXGI monitor capture on 23H2 and older usually works, because DDA sees the scan-out. Win11 24H2 monitor capture via WGC: unknown. | [speculation], test |
| **Window capture falls back to GDI** | If WGC fails, Chromium falls back to a GDI window capturer, which gives black frames for D3D games and is slow. | [cited] `allow_wgc_capturer_fallback` |
| **HDR desktop** | WGC in Chromium asks for BGRA8, so Windows tone-maps to SDR (often dim or washed out, with clipped highlights). On 23H2 and older, monitor capture is DXGI `DuplicateOutput` (the old API, not `DuplicateOutput1`). That path accepts FP16 output; Microsoft's commit names a "color space issue on the HDR monitor" on that path. | [cited] commit `e95d19499b`, `dxgi_output_duplicator.cc`; looks [est.] |
| **Too many duplicators** | DXGI allows at most **four** concurrent duplications by default and returns `DXGI_ERROR_NOT_CURRENTLY_AVAILABLE` after that. OBS, NVIDIA overlay and remote-desktop tools all count, so a busy PC can fail monitor capture on Windows 10 or 23H2. | [cited] [DuplicateOutput1 docs](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_5/nf-dxgi1_5-idxgioutput5-duplicateoutput1) |
| **UAC / secure desktop** | DDA returns `E_ACCESSDENIED` ("only an application that runs at LOCAL_SYSTEM can access the secure desktop"). The stream freezes during a UAC prompt and should recover afterwards. | [cited] same doc; recovery [est.] |
| **Protected content** (Netflix in a browser, DRM video, windows marked `WDA_EXCLUDEFROMCAPTURE`) | Black. Some anti-cheat or anti-RTA clients hide their windows this way. One OBS forum report blames a poker client's anti-cheat ([thread](https://obsproject.com/forum/threads/windows-11-laptop-window-capture-issue-with-ggpoker.190122/post-705513), 2025–26, unconfirmed). | [speculation] |
| **Elevated (admin) game window** | WGC `CreateForWindow` from a normal-integrity process against a high-integrity window may fail. | [speculation], test |
| **Multi-monitor, mixed refresh rates** | DXGI duplicates per output, and capture is polled at the requested rate, so mixed refresh rates are fine. Picker bug #1850 (only the primary monitor listed) needs re-checking. | [cited]/[speculation] |
| **DPI scaling** | DDA and WGC return physical pixels. Mixed-DPI setups may confuse the picker thumbnails or window rectangles if the WebView2 browser process isn't per-monitor-v2 aware (Microsoft has the `embedded-browser-webview-dpi-awareness` flag). | [speculation] |
| **No display attached** | `IsWgcSupported` refuses screen capture with no active display. | [cited] |
| **GPU adapter change** (eGPU, driver reset) | Texture path: permanent error `kDesktopCaptureDeviceGpuAdapterChanged`. CPU path: survives. | [cited] |

---

## 3. The critical one: throttling when Pulse is minimised or behind a game

Taking each layer that could slow the stream down, from the screen to the network:

| Layer | Does it slow capture or encode when Pulse is hidden? | Evidence |
|---|---|---|
| **Capture thread** | No. `DesktopCaptureDevice` runs on its own `desktopCaptureThread` in the **browser process**, driven by its own timer. It also takes a "prevent display sleep" wake lock. | [cited] `desktop_capture_device.cc` |
| **Encode** | No. `RTCVideoEncoder` runs on WebRTC and media threads, and the Media Foundation encoder runs in the GPU process. Neither is a Blink main-thread task. | [cited] `rtc_video_encoder.cc`, `media_foundation_video_encode_accelerator_win.cc` |
| **Renderer process priority** | No, while capturing. `RenderProcessPriority::is_background()` is false when `has_media_stream` is true, and `OnMediaStreamAdded` bumps the count. | [cited] [child_process_launcher.cc](https://github.com/chromium/chromium/blob/main/content/browser/child_process_launcher.cc), [render_process_host_impl.cc](https://github.com/chromium/chromium/blob/main/content/browser/renderer_host/render_process_host_impl.cc) |
| **Intensive wake-up throttling** (timers once per minute after 5 min hidden) | No, while there is a live track or data channel. `MediaStreamTrackImpl` (any live track, local or remote) and `RTCDataChannel` register `kWebRTC` with `DisableAggressiveThrottling()`. LiveKit always opens data channels. **But** `RTCPeerConnection` alone does *not* opt out; it only disables the back/forward cache. | [cited] [media_stream_track_impl.cc](https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/modules/mediastream/media_stream_track_impl.cc), [rtc_data_channel.cc](https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/modules/peerconnection/rtc_data_channel.cc), [rtc_peer_connection.cc](https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/modules/peerconnection/rtc_peer_connection.cc) |
| **Ordinary background timer throttling** (timers aligned to 1/s when hidden) | **Yes, for JS timers.** That affects `livekit-client`'s signal pings, stats and quality loops, not the media itself. LiveKit routes its timers through `CriticalTimers` (`src/room/timers.ts`), which can be swapped for worker-based timers. | [cited] `page_scheduler_impl.cc`, livekit-client |
| **rAF / compositor** | Stops when hidden. Irrelevant for publishing. It matters only if we ever process frames on a canvas. | [est.] |
| **Edge "efficiency mode" (EcoQoS)** | Possible. One report says runtime 147 put Chromium-family processes into Efficiency Mode aggressively on a desktop PC ([#5573](https://github.com/MicrosoftEdge/WebView2Feedback/issues/5573), 2026-04). Edge's own policy may differ from upstream. | [cited] report; impact [speculation] |
| **WebView2's own "hidden" state** | Microsoft: "WebView2 as a child window does not get window messages when the top window is minimized", and apps *should* set `IsVisible = FALSE` on minimise ([ICoreWebView2Controller](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controller)). wry ignores `SIZE_MINIMIZED` and never hides the controller (`src/webview2/mod.rs` `parent_subclass_proc`). Yet Tauri maintainers report timers *are* throttled when minimised ([tauri#5250](https://github.com/tauri-apps/tauri/issues/5250)). So *something* in WebView2 detects it, possibly native occlusion tracking (`kCalculateNativeWinOcclusion`, on by default). | [cited]/[speculation] |
| **`TrySuspend`** | Only if the app calls it, and only when `IsVisible` is false ([ICoreWebView2_3](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_3)). **Pulse must never call it while sharing.** Future "close to tray" code must not call it either. | [cited] |
| **Track auto-mute on no frames** | Turned off upstream: `kMediaStreamTrackEmptyVideoFrameMonitor` is enabled, so `StartFrameMonitoring` returns early. If it were on, a static game screen with 0 Hz capture could fire `mute`, and `livekit-client` pauses upstream after 5 s of mute (`LocalTrack.debouncedTrackMuteHandler`). | [cited] [video_track_adapter.cc](https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/modules/mediastream/video_track_adapter.cc), livekit-client |
| **livekit-client's own background logic** | Desktop: none. `handleAppVisibilityChanged` returns early unless `isMobile()`. | [cited] `LocalTrack.ts`, `LocalVideoTrack.ts` |

**Verdict.**

- While Pulse is actively sharing, nothing I found in upstream Chromium slows capture or encode when the window is
  minimised or occluded **[est.]**.
- The risks left are:
  - Edge-specific behaviour (efficiency mode, or patches we can't see);
  - a hidden `Room` connected *before* any track is live, which, after 5 minutes hidden, could have its ping timers
    pushed to once a minute and drop **[est.]**.
- Mitigations:
  - connect the video `Room` only when going live or watching;
  - use the flags below;
  - optionally replace `CriticalTimers` with worker timers.

**How Electron handles it, for comparison.** Electron exposes `webPreferences.backgroundThrottling` ("Whether to throttle
animations and timers when the page becomes background", default `true`;
[docs](https://www.electronjs.org/docs/latest/api/structures/web-preferences)) **[cited]**. WebView2 has no such API.
Tauri's `background_throttling` is documented as unsupported on Windows (`tauri-utils/src/config.rs`) **[cited]**. Our
only levers are command-line flags. Microsoft lists `disable-background-timer-throttling` as a WebView2 flag, but says
flags are for development and "might be removed or altered at any time"
([flags page](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/webview-features-flags)) **[cited]**.

**What each flag does** **[cited]** for the mapping; effect in WebView2 **[est.]**:

| Flag | Effect | Source |
|---|---|---|
| `--disable-background-timer-throttling` | Turns off Blink's `TimerThrottlingForBackgroundTabs` | [runtime_features.cc](https://github.com/chromium/chromium/blob/main/content/child/runtime_features.cc) |
| `--intensive-wake-up-throttling-policy=0` | Force-disables intensive throttling | [blink switches.cc](https://github.com/chromium/chromium/blob/main/third_party/blink/common/switches.cc) |
| `--disable-renderer-backgrounding` | Keeps renderer process priority up | [content_switches.cc](https://github.com/chromium/chromium/blob/main/content/public/common/content_switches.cc) |
| `--disable-backgrounding-occluded-windows` | Turns OCCLUDED into VISIBLE in `WebContentsImpl::UpdateWebContentsVisibility` | [web_contents_impl.cc](https://github.com/chromium/chromium/blob/main/content/browser/web_contents/web_contents_impl.cc) |
| `--disable-features=CalculateNativeWinOcclusion` | Stops native occlusion tracking | [ui_base_features.cc](https://github.com/chromium/chromium/blob/main/ui/base/ui_base_features.cc) |

**A hidden or offscreen second webview: don't.**

- The picker is drawn inside the calling webview's bounds (#2184), so an invisible webview gets an invisible picker.
- A second webview also means a second renderer process, more memory, and its own visibility state.
- Instead, run **two `Room` objects in the main page** (`<uid>#v` for video). Both share one renderer, and the live
  screen track keeps it out of background priority.

---

## 4. Hardware encode inside WebView2

- **It is on by default upstream** **[cited]**:
  - `kWebRtcHWEncoding` is enabled
    ([content_features.cc](https://github.com/chromium/chromium/blob/main/content/public/common/content_features.cc)).
  - `kMediaFoundationVideoEncodeAccelerator` is enabled, and so is `kMediaFoundationUseSoftwareRateCtrl` (media_switches.cc).
  - The Media Foundation shared state lists hardware MFTs for **H.264, VP9, AV1**, plus **HEVC** when
    `kPlatformHEVCEncoderSupport` is on (enabled on Windows), with L1T2/L1T3 temporal layers where the vendor allows
    ([mf_video_encoder_shared_state.cc](https://github.com/chromium/chromium/blob/main/media/gpu/windows/mf_video_encoder_shared_state.cc)).
  - One code path covers NVENC, AMF and QSV, through whichever vendor MFT is installed.
  - D3D12 encode (`kD3D12VideoEncodeAccelerator`) is off by default.
- **The H.264 gate.** `UseH264AcceleratedEncoderForWebRTC()` returns `IsOpenH264SoftwareEncoderEnabledForWebRTC()` on
  non-Android ([webrtc_util.cc](https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/platform/peerconnection/webrtc_util.cc))
  **[cited]**. Hardware H.264 is offered *only if* the OpenH264 software fallback exists. Edge's build should include it
  **[est.]**. Verify that the SDP offers H.264.
- **Software fallbacks to design around** **[cited]** in `rtc_video_encoder.cc`:
  - `kForceSoftwareForRtcLowResolutions` (on for desktop): **any layer under 360p (270p for AV1) is encoded in
    software.** A 180p simulcast layer costs CPU (OpenH264). That is small, but keep low layers at 360p or above.
  - **Odd frame sizes:** "hardware H.264 encoder only supports even sized frames", so it falls back to software. The
    Media Foundation encoder reports an alignment of 2. Pick `scaleResolutionDownBy` or explicit sizes that give even
    dimensions.
  - `kWebRtcScreenshareSwEncoding` (screenshare with temporal layers forced to software) is **off** by default.
  - `kWebRTCHardwareVideoEncoderFrameDrop` is on: the hardware encoder may drop frames under pressure.
- **Content type.**
  - A `getDisplayMedia` track is a "screencast", so WebRTC keeps resolution and drops frames.
  - Setting `track.contentHint = 'motion'` makes `VideoRtpSender` set `is_screencast = false`; `'detail'` or `'text'`
    sets it to true ([pc/rtp_sender.cc](https://webrtc.googlesource.com/src/+/refs/heads/main/pc/rtp_sender.cc))
    **[cited]**.
  - **Use `'motion'` for game presets and `'detail'` for the "sharp text" preset.**
- **LiveKit defaults are wrong for games:** `screenShareEncoding` defaults to `ScreenSharePresets.h1080fps15` (2.5 Mbps,
  15 fps) in `src/room/defaults.ts` **[cited]**. Set the encoding, the frame rate and `videoCodec: 'h264'` explicitly.
- **AV1 and HEVC.**
  - AV1 hardware encode is enumerated through Media Foundation on GPUs that have it (RTX 40+, RX 7000+, Arc).
  - WebRTC H.265 is compiled in on Windows (`rtc_use_h265 = enable_hevc_parser_and_hw_decoder`,
    [webrtc.gni](https://webrtc.googlesource.com/src/+/refs/heads/main/webrtc.gni)), but has no software fallback.
  - Not needed for v1. H.264 has the widest hardware decode for viewers **[est.]**.
- **Checking it at runtime.**
  - `getStats()` outbound-rtp `encoderImplementation` should read
    `"MediaFoundationVideoEncodeAccelerator (<MFT name>)"`, and `powerEfficientEncoder` should be `true`
    (`media_foundation_video_encode_accelerator_win.cc` L604) **[cited]**.
  - **Gotcha:** Chromium exposes these hardware fields *only while the page is capturing* (`ExposeHardwareCapabilityStats` →
    `IsCapturing`, [rtc_stats_report.cc](https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/modules/peerconnection/rtc_stats_report.cc))
    **[cited]**.
    - The publisher sees them.
    - A pure *viewer* page won't see `decoderImplementation` or `powerEfficientDecoder`. In a debug build, open a
      throwaway `getUserMedia({audio:true})` to unlock them **[est.]**.
- **Viewer-side bug to carry into the spike.** On WebView2 152, received WebRTC video falls back to *software* decode
  when the colour-space RTP header extension is negotiated (it is, by default). The cause is the Edge-only hybrid
  decoder wrapper. The workaround is `--disable-features=msD3D11VdaHybridVideoDecoding`
  ([#5755](https://github.com/MicrosoftEdge/WebView2Feedback/issues/5755)) **[cited]**.
- **Encoder sessions.** GeForce cards allow a limited number of concurrent NVENC sessions; 8 per system in recent
  drivers, according to an NVIDIA SDK note quoted on NVIDIA's forum
  ([thread](https://forums.developer.nvidia.com/t/how-to-increase-number-of-nvenc-concurrent-sessions/169367))
  **[cited, secondary]**. ShadowPlay or Instant Replay, OBS and Pulse each take one. If the MFT fails to start, Chromium
  falls back to software **[est.]**.

---

## 5. Cost to the streamer's game FPS

- **Capture** **[est.]**:
  - WGC is a DWM-side copy of the composed surface and is cheap on the GPU. DDA is similar.
  - The real cost in Chromium is the **GPU→CPU readback plus the CPU BGRA→I420 conversion plus the re-upload**: one
    core at 30–50% at 1440p60. The 50% budget cap protects the game by lowering capture fps first.
  - On a CPU-bound game (high-refresh esports titles), expect a few percent fps loss.
- **Encode:** NVENC, AMF and QSV are fixed-function blocks separate from the 3D engine, so contention is small. The
  game's own recording (ShadowPlay) shares the same block **[est.]**.
- **Presentation mode [speculation]:** an active capture *might* force DWM to compose a game that was in
  independent-flip mode. That adds a frame of latency and some GPU work. Measure it with PresentMon's PresentMode column.
- **How to keep the cost down:**
  - capture the **window**, not a 4K monitor;
  - cap capture at 60 fps (not 120 or 144);
  - request the output size at capture (`width`/`height` constraints);
  - keep simulcast layers at 360p or above (to stay on hardware);
  - avoid a second monitor capture app;
  - on Windows 11 24H2, try the texture flag in §7. It removes the readback but is experimental.

---

## 6. Anti-cheat compatibility

- **No primary source** from Riot (Vanguard), Epic (EAC), BattlEye, FACEIT or Activision (Ricochet) says how they treat
  WGC or DDA capture. Searches turned up only OBS forum posts and third-party guides. Those say *game-hook* capture
  (DLL injection) is blocked or broken under Vanguard and FACEIT, and that Window or Display Capture is the usual
  workaround ([OBS forum, Valorant](https://obsproject.com/forum/threads/game-capture-not-working-on-valorant-black-screen.124987/),
  [OBS forum, FACEIT](https://obsproject.com/forum/threads/cs-go-game-capture-black-screen-only-when-using-faceit-anticheat.143921))
  **[cited, secondary]**.
- **Why WebView2 capture should be safe [speculation, reasonable].**
  - Nothing is injected into the game.
  - The capture runs inside Microsoft-signed `msedgewebview2.exe`, the same code path millions use to screen-share games
    from Edge or Chrome (Discord in a browser).
  - An anti-cheat that blocked it would break browser screen sharing everywhere.
- **The realistic failures:**
  - black or frozen frames from a game that uses `WDA_EXCLUDEFROMCAPTURE`, protected swap chains or true exclusive
    fullscreen;
  - an elevated game window that WGC can't open.
- **Spike:** Valorant (Vanguard) in borderless mode, with window capture and with monitor capture; one EAC title
  (Fortnite or Apex); CS2 with FACEIT if anyone in the group uses it.

---

## 7. Exact settings and flags for the spike

**WebView2 browser arguments.** Set them in Rust with `WebviewWindowBuilder::additional_browser_args`. Setting this
*replaces* wry's defaults, so keep them. Chromium reads only one `--disable-features` and one `--enable-features`, so
merge each into a single switch **[est.]**:

```text
--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,CalculateNativeWinOcclusion,msD3D11VdaHybridVideoDecoding
--disable-background-timer-throttling
--disable-renderer-backgrounding
--disable-backgrounding-occluded-windows
--intensive-wake-up-throttling-policy=0
--autoplay-policy=no-user-gesture-required
```

The last line keeps what wry adds when autoplay is on.

**Experimental flags (one per run):**

- `--enable-features=WebRtcAllowWgcUsingTexture` (24H2+ only: no readback);
- `--enable-features=ZeroCopyDesktopCapture`;
- `--auto-select-window-capture-source-by-title=<game title>` (does Edge's picker honour it?);
- `--enable-logging --v=1 --vmodule=desktop_capture_device=2,wgc*=2,dxgi*=2,media_foundation_video_encode*=2`. This
  shows which capturer and encoder were chosen; the log location in WebView2 is unverified **[est.]**.

**Rust host:**

- Register `ScreenCaptureStarting` (`ICoreWebView2_27`, present in `webview2-com-sys` 0.39.1). Allow only during our
  "Go Live" flow; use a deferral to ensure the window is at least 600 px tall.
- Raise `min_inner_size` height above 600.
- Never call `TrySuspend` or `SetIsVisible(false)` while sharing.
- Add the LiveKit `wss://` and `https://` origins to the CSP `connect-src`.

**JS (`livekit-client`):**

```js
const controller = new CaptureController();
controller.setFocusBehavior?.('focus-captured-surface');
const stream = await navigator.mediaDevices.getDisplayMedia({
  video: { frameRate: { ideal: 60, max: 60 }, width: { max: 1920 }, height: { max: 1080 },
           displaySurface: 'window' },
  audio: false, systemAudio: 'exclude', selfBrowserSurface: 'exclude',
  surfaceSwitching: 'include', monitorTypeSurfaces: 'include', controller,
});
const [track] = stream.getVideoTracks();
track.contentHint = 'motion';               // 'detail' for the sharp-text preset
await videoRoom.localParticipant.publishTrack(track, {
  source: Track.Source.ScreenShare, videoCodec: 'h264',
  screenShareEncoding: { maxBitrate: 4_500_000, maxFramerate: 60 },
  screenShareSimulcastLayers: [/* one layer, >= 360p, even dimensions */],
  degradationPreference: 'maintain-framerate',
});
```

---

## 8. Blocking risks for the spike

1. **Monitor capture on Windows 10 and 23H2 is DXGI, not WGC.** Different HDR behaviour, the 4-duplicator limit, and
   different exclusive-fullscreen behaviour. No flag changes this any more.
2. **The CPU round trip (readback, I420, upload)** may stop 1440p60 holding 60 fps, and costs game CPU. Fallback: a
   lower capture size, the texture flag (24H2), or Plan B.
3. **Picker UX:** clipped to the webview, the 600 px minimum (Pulse is at 560), possibly a missing second monitor, and a
   floating "sharing" bar over the game. There is no API to choose the source.
4. **Edge differs from Chromium:** efficiency mode, the hybrid decoder bug (#5755), and unknown patches. Everything above
   needs the Windows test.
5. **Minimised/occluded throttling** is probably fine, but this is the scenario that matters most for gamers. One bad
   result here would force the Rust capture path.
6. **Anti-cheat:** no primary sources. A black frame in Valorant would be a real limit for the group.

## 9. Test cases for the Windows spike

Run on at least one Windows 10 22H2 PC and one Windows 11 PC (ideally 24H2 *and* 23H2), on NVIDIA and AMD if possible.
Record the WebView2 runtime version (`navigator.userAgent`) for every run.

| # | Test | Pass condition / what to record |
|---|---|---|
| T1 | `getDisplayMedia` from Pulse at 1280×800, then at 900×560 | The picker is usable; note clipping. Confirm the 600 px bug. |
| T2 | Picker lists every monitor and the game window; the "sharing" bar appears | All monitors are listed; note where the bar appears and whether it steals focus from a fullscreen game |
| T3 | `ScreenCaptureStarting` fires; Cancel gives `NotAllowedError`; a deferral works | The events fire as documented |
| T4 | Capturer identity per OS (log, or a yellow border on Win10 window capture) | Matches the table in §2.1 |
| T5 | Window capture of a borderless game at 1080p and 1440p, requesting 60 fps | `framesPerSecond` sent ≥ 58 for 5 min; CPU % of the WebView2 browser and GPU processes |
| T6 | Monitor capture of a 4K or 1440p 144 Hz screen at 60 fps | Delivered fps; note where the 50% budget bites |
| T7 | **Minimise Pulse** for 10+ minutes while sharing | Viewer fps unchanged; no LiveKit reconnect; signal ping OK; repeat with and without the §7 flags |
| T8 | **Pulse behind a fullscreen game** (not minimised) for 10+ minutes | Same as T7; check `document.visibilityState` in a log |
| T9 | Video `Room` connected but idle (not publishing) while minimised for 10 minutes | Stays connected (tests the intensive-throttling gap) |
| T10 | Task Manager "Efficiency mode" leaf on `msedgewebview2` processes during T7 and T8 | No leaf on the renderer, GPU or browser process |
| T11 | `getStats` on the publisher | `encoderImplementation` contains `MediaFoundationVideoEncodeAccelerator`; `powerEfficientEncoder: true`; GPU "Video Encode" engine busy |
| T12 | Odd-size window (resize the game to 1281×721) | Stays on hardware (check `encoderImplementation`) |
| T13 | Simulcast with a 180p layer, then a 360p layer | 180p layer = OpenH264 (software) as predicted |
| T14 | Viewer decode with and without `msD3D11VdaHybridVideoDecoding` disabled | Hardware decode (use the getUserMedia trick to unlock the stats) |
| T15 | Game fps with and without sharing (PresentMon: fps, latency, PresentMode) | Loss ≤ 5% on a GPU-bound game; does PresentMode change from independent flip? |
| T16 | Minimise the captured game | Viewers see a frozen last frame; recovers on restore |
| T17 | Exclusive-fullscreen game (FSE, FSO disabled), window capture vs monitor capture | Record black, frozen or OK per OS |
| T18 | HDR on, SDR game and HDR game, monitor capture and window capture | Washed out? Dim? Note per OS and backend |
| T19 | UAC prompt during monitor capture | Freezes, then recovers |
| T20 | OBS + NVIDIA overlay + Pulse all capturing the monitor (Win10/23H2) | Does Pulse fail (DXGI limit)? |
| T21 | **Vanguard** (Valorant), window and monitor capture, 10 minutes | Picture OK, no kick or warning |
| T22 | **EAC** title (Fortnite or Apex), and FACEIT if used | Same |
| T23 | Elevated game window (a launcher run as admin) | Capture works or fails clearly |
| T24 | `--auto-select-window-capture-source-by-title` experiment in a fresh user-data folder | Does WebView2 honour it (Plan "own picker light")? |
| T25 | Win11 24H2: `WebRtcAllowWgcUsingTexture` on vs off | CPU drop, fps and stability |
| T26 | Voice (Rust) and video `Room` (JS) together for 30 minutes | No audio glitches; CPU is the sum of the parts |
| T27 | Plan B smoke test: Rust WGC → `CreateSharedBuffer` → `VideoFrame` → `MediaStreamTrackGenerator` → publish at 1080p60 | Hardware encode still used; CPU cost of the copy |
