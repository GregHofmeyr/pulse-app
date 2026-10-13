# Screen-share viewing in Pulse: how to get decoded video onto the screen

**Date:** 2026-10-13 · **Scope:** the viewer side of screen share (1080p–1440p, up to 60 fps) in the Tauri 2 client.
**Versions in repo:** tauri 2.12.1, wry 0.57.0, webview2-com 0.39.1, livekit 0.9.3, libwebrtc 0.3.50, webrtc-sys 0.3.47,
webkit2gtk-4.1 2.52.6 on the dev box. Read-only research: nothing in the repo was changed or built.

---

## 0. TL;DR

- **Primary (Windows):** run WebRTC for screen-share video inside WebView2 with `livekit-client` JS, as a
  second LiveKit participant (`<uid>#v`) that is subscribe-only. Chromium decodes in hardware (D3D11) and composites the frame
  in the GPU with no CPU copies. Native `<video>` gives fullscreen, resizing, scrolling and HTML overlays for free.
  Voice stays in Rust and does not change.
- **Fallback, and the Linux path:** Rust subscribes and decodes via `NativeVideoStream` (queue = 1, latest frame
  only). It sends the I420 planes over a **local WebSocket** (127.0.0.1, random port, secret token) to a **WebGL2 YUV
  shader** in the webview. My measurements (Chromium 151, Linux) show plenty of headroom: WebSocket ≈1.3 GB/s, and the upload plus
  draw costs 2.7 ms per 1080p frame.
- **Rejected for 60 fps:** Tauri Channels / IPC / custom protocol (≈50 MB/s on WebView2, while 1080p60 needs 187 MB/s, and
  every frame goes through the UI thread), MJPEG, and SharedArrayBuffer (it can't cross into Rust).
  A native child surface (wgpu/D3D11) over a placeholder div works on Windows, but you lose HTML overlays and get
  resize/scroll lag, and on Wayland it is a non-starter.
- **Big finding:** **webrtc-sys 0.3.47 has no hardware video decode *or encode* on Windows.** NVDEC/NVENC and VAAPI
  are compiled only on Linux (`build.rs`). Any Rust-side path on Windows decodes in software (FFmpeg H.264,
  libvpx, dav1d), and *publishing* from Rust on Windows means software encoding while the publisher is gaming.
  This favours the webview for publishing as well (§3.4).
- **Trap in today's code:** `voice/mod.rs:400` connects with `RoomOptions::default()`, so `auto_subscribe = true`.
  Once anyone publishes video in a voice room, every Rust client subscribes and libwebrtc **decodes the video in
  software even though nothing reads it**. Screen-share v2 has to turn off video subscription on the Rust connection.

---

## 1. Ground truth from the code

### 1.1 Pulse today
- `client/src-tauri/src/lib.rs` creates a single `WebviewWindowBuilder` "main" window: frameless, opaque background
  `#141519`, and a per-profile `data_directory`. Linux reloads the webview if `WebKitWebProcess` dies.
- CSP (`tauri.conf.json`): `connect-src ipc: http://ipc.localhost`. Any WebSocket or `wss://` LiveKit path needs a
  CSP change. `media-src` isn't set, so it falls back to `default-src 'self'`; `<video>.srcObject` with a
  MediaStream isn't a URL fetch, so it's fine, but a blob URL would need `blob:`.
- Server `server/src/voice/routes.rs`: the token has identity = `UserId`, grants `can_publish/can_subscribe`, and TTL 10 min.
  The webhook **ignores identities that don't parse as `UserId`** ("Anything else isn't ours: ack and ignore"), so a
  companion identity like `<uid>#v` would not create ghost voice-state or stats rows. That is already in our favour.
- `voice/mod.rs` subscribes to audio per participant identity. A companion participant would appear in
  `remote_participants()` and must be filtered out of the UI roster.

### 1.2 What the LiveKit Rust SDK gives a viewer
- `libwebrtc::native::video_stream::NativeVideoStream::new(track, queue_size_frames)`. `Some(1)` selects
  `LatestVideoFrameQueue` (keeps only the newest frame, which is what a renderer wants), `Some(n)` uses a bounded rtrb ring,
  and `None` is unbounded (a latency trap).
- Frames are `VideoFrame { rotation, timestamp_us, frame_metadata, buffer: Box<dyn VideoBuffer> }`. Buffer types are
  `Native | I420 | I420A | I422 | I444 | I010 | NV12`. `to_i420()` and `to_argb(VideoFormatType, dst, stride, w, h)` are
  libyuv conversions on the CPU.
- `NativeBuffer` exposes **only** a CVPixelBuffer (Apple) or a DMA-BUF fd (Linux, aimed at the Jetson encoder input).
  **No D3D11 texture handle exists on Windows.** The "GPU handle → readback" worry is moot on Windows because frames
  are always CPU I420.
- **Decoders** (`webrtc-sys/src/video_decoder_factory.cpp`): VP8 (libvpx), VP9 (libvpx), H.264 (FFmpeg,
  only if it probes OK), and AV1 (dav1d, `RTC_DAV1D_IN_INTERNAL_DECODER_FACTORY`). The prebuilt `win-x64-release` `args.gn`
  in our `target/` shows `rtc_use_h264 = true` and `ffmpeg_branding = "Chrome"`, so software H.264 is present.
  The NVIDIA factory (`USE_NVIDIA_VIDEO_CODEC`) is built **only in the `"linux"` arm of `build.rs`**, only if `cuda.h`
  exists at build time, and even then `nvidia/h264_decoder_impl.cpp` copies NV12 to I420 into CPU memory
  (`libyuv::NV12ToI420`). The Windows arm has only commented-out VAAPI lines. **Result: Windows = software decode.**
- Encoders (same story): NVENC/VAAPI only on Linux. Windows publishing from Rust = OpenH264/libvpx/libaom software
  encode. There is a passthrough "PreEncoded" backend (`EncodedVideoFrame`) for publishing pre-encoded access units,
  which is a lever for later (§3.4).
- There is **no receive-side encoded-frame tap** in the Rust API (no frame transformer exposed), so "Rust forwards H.264 NALs
  and WebCodecs decodes them in hardware" requires forking webrtc-sys (§2.6).

### 1.3 Tauri internals (why IPC is slow for frames)
`tauri-2.12.1/src/ipc/channel.rs`:
- A payload of 1 KiB or more (raw) or 8 KiB or more (JSON) is **not** pushed. Rust does `webview.eval(...)` of a JS snippet that calls
  `invoke('plugin:__TAURI_CHANNEL__|fetch', {headers:{'Tauri-Channel-Id': id}})`. JS then makes a second request
  to fetch the bytes over the `ipc://` custom protocol. That is **two webview round trips per frame**, and `eval` runs on the
  UI thread.
- On WebView2 the custom protocol is served by wry's `WebResourceRequested` handler. The response body is copied into
  `SHCreateMemStream` (`wry/src/webview2/mod.rs:1200`), then WebView2 marshals it from the browser process to the renderer.
  This is not streaming: the whole body is buffered. A maintainer said WebView2 can't do streaming custom-protocol
  responses ([tauri#5690](https://github.com/tauri-apps/tauri/discussions/5690)).
- A source comment says "8192 byte JSON payload runs roughly 2x faster through eval than through fetch on WebView2
  v135".

---

## 2. Option A: frames decoded in Rust, then drawn by the webview

Bytes per second the transport must carry:

| Format | 1080p frame | 1080p60 | 1440p frame | 1440p60 |
|---|---|---|---|---|
| I420 / NV12 | 3.11 MB | **187 MB/s** | 5.53 MB | **332 MB/s** |
| RGBA | 8.29 MB | 498 MB/s | 14.7 MB | 885 MB/s |

Keep it YUV all the way to a GPU shader. Converting to RGBA in Rust multiplies the bytes by 2.67 and costs a libyuv pass.

### 2.1 Tauri IPC / events / Channels: **reject**
- WebView2 numbers from others:
  - "~5 ms on macOS but ~200 ms on Windows" for 10 MB of binary IPC. A maintainer (FabianLars) relayed this on 2024-12-09, adding that
    "Windows 100% used to be better" ([tauri disc. #11915](https://github.com/orgs/tauri-apps/discussions/11915)). That is
    **≈50 MB/s**.
  - 3 MB took about 200 ms (2025-05-09). The thread also reports 40 ms typical with 700 ms spikes for 9 MB, and says events and WebSocket were "slower than IPC" in
    that user's setup ([tauri#13405](https://github.com/tauri-apps/tauri/issues/13405), 2025-05 → 2025-10).
  - Tauri `invoke` measured 11.1 ms for 256 KiB and 40.5 ms for 1 MiB on `windows-latest` ([kiri PR #40](https://github.com/ChloeVPin/kiri/pull/40),
    2026-09-23, one author and unverified).
- At ≈40–60 ms per 3 MB frame you get **8–16 fps at best**, plus UI-thread stalls that make chat and the voice UI stutter.
  It's fine for a 1–2 fps thumbnail, not for a stream.

### 2.2 Custom URI scheme (raw frames or MJPEG): **reject**
- It uses the same `WebResourceRequested` and `SHCreateMemStream` path as the IPC above, so the throughput ceiling is the same, and WebView2 does not
  stream custom-scheme responses, which `multipart/x-mixed-replace` MJPEG needs.
- MJPEG via a real localhost HTTP server would work in an `<img>`, but it means a CPU JPEG encode per frame in Rust
  (libjpeg-turbo ≈ several ms per 1080p frame per core, so 60 fps is a large fraction of a core), a JPEG decode in the renderer,
  and chroma and ringing artefacts on text, which is the main content of a screen share. That's worse than option 2.3 on every axis.

### 2.3 Local WebSocket (raw I420) + WebGL2: **fallback, and the Linux path**
- This uses Chromium's own network stack (network service → renderer over a mojo data pipe), not wry's UI-thread hook.
- **Measured here** (headless Chrome 151 on Linux, i9-13900HX, a minimal Node WS server pushing random I420 frames, 2 frames in flight,
  scripts in `scratchpad/bench/`):

  | Transport | Res | Frames/s sustained | MB/s | per-frame RTT p50 / p95 |
  |---|---|---|---|---|
  | WebSocket binary | 1080p I420 | **415** | **1290** | 4.1 / 9.6 ms |
  | WebSocket binary | 1440p I420 | **281** | **1551** | 6.3 / 11.6 ms |
  | HTTP fetch, sequential | 1080p | 302 | 939 | 3.2 ms |
  | HTTP fetch, sequential | 1440p | 178 | 983 | 5.6 ms |

  That is 5–7× headroom over what 60 fps needs. WebView2 is the same Chromium network stack, so expect the same order of
  magnitude, **but this has to be confirmed on Windows** (spike step D2.4).
- Rust side: `tokio-tungstenite` is already a dependency. Bind to `127.0.0.1:0`, pass the port and a random secret to the
  UI via IPC, check the secret and Origin on connect (any local process or browser tab can open a localhost WebSocket). Add
  `connect-src ws://127.0.0.1:*` to the CSP.
- Frame protocol: a 16-byte header (w, h, strides, `timestamp_us`), then Y, U and V. Send only when a tile is visible.
  **Downscale in Rust with libyuv `I420Buffer::scale`** to the tile's pixel size (a 480p thumbnail is 0.46 MB).
  Drop frames if the socket has more than one frame unacked (send "latest" only, never queue).
- Draw (**measured**, Chrome 151 with ANGLE→GL on the Intel UHD iGPU of the dev box; this is the *weaker* GPU, so treat it as
  an upper bound):

  | Draw path | 1080p p50 / p95 | 1440p p50 / p95 |
  |---|---|---|
  | WebGL2: 3× `texSubImage2D` (R8 planes) + YUV→RGB shader + forced sync (`readPixels` 1 px) | **2.7 / 4.5 ms** | 6.3 / 12.2 ms |
  | WebCodecs `new VideoFrame(buf,{format:'I420'})` + 2D `drawImage` (synced) | 4.4–5.5 / ~20 ms | 6.0–6.5 / ~10 ms |

  The sync forces a full pipeline stall, so real pipelined cost is lower. WebGL2 with our own shader is the
  cheapest and most predictable. The WebCodecs `VideoFrame` path is simpler to write (Chromium does the conversion), has worse
  tail latency, and can also feed a `<video>` through `MediaStreamTrackGenerator` if we want native fullscreen
  behaviour. Do the upload in a Worker with an `OffscreenCanvas` so main-thread jank (chat rendering) can't drop frames.
- Copies per frame: libwebrtc buffer → WS send buffer (1) → kernel loopback (2) → network service → renderer
  (3) → ArrayBuffer → GPU transfer buffer (4) → GPU. At 3 MB each that is about 1–2 ms of memcpy per frame (≈6–12% of one
  core at 60 fps) plus WS framing. That's acceptable.
- **Windows CPU caveat:** decode is software (§1.2). Chromium WebCodecs software decode on this CPU (my run, 1080p, 8 Mbps
  synthetic text and motion, multithreaded throughput) did H.264 **991 fps**, VP9 **421 fps** and AV1 **729 fps**. So 1080p60 is a
  fraction of a modern CPU, but libwebrtc's threading differs and the viewer may be gaming. Estimate 0.3–1 core for
  1080p60 decode plus the transport, versus about 0 for hardware decode. 1440p60 VP9 in software is the worst case.

### 2.4 WebView2 SharedBuffer (`ICoreWebView2SharedBuffer`): **Windows-only optimisation of 2.3**
- API: `ICoreWebView2Environment12::CreateSharedBuffer(size)` and
  `ICoreWebView2_17::PostSharedBufferToScript(buf, access, additionalDataAsJson)`. JS gets
  `chrome.webview.addEventListener('sharedbufferreceived', e => e.getBuffer())`, an `ArrayBuffer` backed by shared memory,
  and must call `chrome.webview.releaseBuffer(buf)`. Stable since WebView2 1.0.1661.34 (2023). Writing to a read-only
  buffer **crashes the renderer** ([MS docs](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_17)).
  Both interfaces are present in `webview2-com-sys 0.39.1`. Tauri exposes `PlatformWebview::controller()` in
  `with_webview`, which gets you to `ICoreWebView2` → `_17`, and `Environment()` → `Environment12`.
- Design: allocate **3 ring slots once** (one 1440p I420 frame each), post each slot to script **once** at
  startup, then per frame `memcpy` into a free slot and `PostWebMessageAsJson({slot, w, h, ts})` (a tiny message). JS uploads from
  the slot's ArrayBuffer and acks. That removes copies 2–3 and the network stack.
- Evidence it is "weirdly slow": a maintainer's second-hand remark (tauri #11915, 2024-12), and kiri's *per-reply* SharedBuffer
  at 22 ms per MiB (it creates a buffer per message, which is the expensive part). Allocate-once has no public benchmark,
  so measure it in the spike before committing. Unsafe COM, Windows-only, and a second code path to maintain.
  **Only worth it if the D2.4 measurement shows WebSocket costing meaningful CPU on Windows.**

### 2.5 SharedArrayBuffer: **not applicable**
SAB shares memory between JS agents (workers) inside the renderer. It cannot be mapped from the Rust process, and it requires
cross-origin isolation (COOP/COEP headers on our custom-protocol pages). Its only use would be passing frames from a WS-receiving worker to a
rendering worker, and an OffscreenCanvas worker that does both already covers that.

### 2.6 (Future) Rust forwards *encoded* frames → WebCodecs `VideoDecoder` in hardware
The most bandwidth-efficient hybrid: ~1–2 MB/s over IPC (even Tauri Channels could carry that), hardware decode in Chromium,
one LiveKit connection. But the Rust SDK has no receive-side encoded-frame hook. You'd fork webrtc-sys to add a
"passthrough" `VideoDecoder` in `VideoDecoderFactory` that hands NAL units to Rust and returns a dummy frame. You would also own
keyframe requests and error concealment. WebKitGTK's WebCodecs goes through GStreamer, which is the renderer-abort risk class from
FINDINGS. **Park it.**

---

## 3. Option C: WebRTC inside WebView2, for video only. **Primary**

### 3.1 Why
- WebView2 is Chromium (Edge 1xx). Its WebRTC receive path uses the GPU decoder (`D3D11VideoDecoder`, which has
  per-codec accelerators for H.264, VP9 and AV1 ([Chromium src](https://chromium.googlesource.com/chromium/src/+/main/media/gpu/windows/d3d11_video_decoder.cc))).
  The decoded texture goes **straight to the compositor**: no CPU YUV, no copies, no IPC. This is the
  same path Discord's *browser* client and every web meeting app use for 1080p60.
- `<video>` gives fullscreen (`requestFullscreen`), CSS sizing, scrolling, overlays (names, "LIVE",
  controls, context menus), picture-in-picture, and `requestVideoFrameCallback` for stats, all for free.
- **Codec gotcha:** VP8 has no hardware decode on Windows Chromium, and `livekit-client` defaults to VP8. The publisher must use
  **H.264** (universal hardware decode, and hardware encode on Windows via Media Foundation) or AV1/VP9 where GPUs support it.
  LiveKit lets the publisher choose (`videoCodec`, plus `backupCodec` for older clients).
  Verify in the spike with `getStats()` → `inbound-rtp.decoderImplementation` and `powerEfficientDecoder`.

### 3.2 Two connections per user: token and identity design
- LiveKit identity is unique per room: **a second connection with the same identity kicks the first**
  (`DUPLICATE_IDENTITY`) ([LiveKit docs](https://docs.livekit.io/intro/basics/connect/), accessed 2026-10-13). So the
  webview needs its own identity: **`<uid>#v`** (or `#s` when publishing).
- New server route (or a `?role=video` flag on `/voice/{id}/token`) mints:
  - viewer: `identity=<uid>#v`, `hidden: true`, `can_subscribe: true`, `can_publish: false`,
    `can_publish_data: false`. Hidden participants don't appear in others' rosters.
  - publisher (if §3.4 is adopted): not hidden, `can_publish_sources: ["screen_share","screen_share_audio"]`, with
    participant attribute `owner=<uid>` so clients show it as the user's stream.
  - The same membership checks as the voice token. Only issue it while the user is in that voice channel (check
    `s.voice` state) so a client can't lurk on video without being in voice.
- Webhook: `"<uid>#v".parse::<UserId>()` fails, so it's ignored, which is correct. Optionally parse the suffix later for "who is
  watching" UI.
- Rust voice connection: set `RoomOptions { auto_subscribe: false, .. }` and subscribe explicitly to **audio**
  publications only (`TrackPublished`/`set_subscribed(true)` for `TrackKind::Audio`). Otherwise Rust decodes every video
  for nothing (§0). Filter `#`-suffixed identities out of rosters and speaking indicators.
- Webview connection: `autoSubscribe: false` and subscribe only to `Track.Source.ScreenShare` that the user chose to watch
  (Discord-style "click to watch" saves everyone's bandwidth). Use `adaptiveStream: true` so the SFU sends a layer that
  matches the element size (a thumbnail gets low simulcast layers), and `dynacast`.
- **Stream audio (`screen_share_audio`) should be subscribed in *Rust*, not JS**, so it goes through our mixer (per-user
  volume) and into APM's reverse stream (echo cancellation for speaker users). Cost: A/V sync between two connections
  isn't guaranteed (separate jitter buffers, typically tens of ms apart). That's acceptable for game streams; measure it in the spike.
- Cost per viewer: a second PeerConnection (ICE, DTLS, TURN allocation), a second signalling WS, about 20–40 MB more renderer
  memory while watching (estimate), and `livekit-client` (~150–200 KB min+gz) in the UI bundle. Connect only while
  watching or sharing, not for the whole voice session.
- CSP: add `connect-src wss://<livekit host>` (plus the TURN/https host for `/rtc/validate`). WebView2 media autoplay of a
  muted `<video>` works. The video track has no audio in this design anyway.

### 3.3 Linux (WebKitGTK)
- WebKitGTK 2.52 has a `enable-webrtc` setting (GstWebRTC backend), but FINDINGS showed GStreamer elements missing
  (`gst-plugins-good` is **not installed** here; `gst-plugins-bad` 1.28.7 is) and abort the renderer. **Don't
  depend on it.** At most, an opt-in experiment on Greg's box.
- So on Linux the viewer uses **§2.3 (Rust decode → local WS → WebGL2)**. Linux has the extra option of NVDEC if
  webrtc-sys is built with CUDA headers present (RTX 4070 here), though it still copies to I420. Linux is the dev box, so 1080p30 is
  an acceptable bar. The same JS `<ScreenView>` component takes either a `MediaStreamTrack` or a WS frame source.

### 3.4 Publishing from the webview too (`getDisplayMedia` in WebView2)
Not this report's main topic, but it decides the architecture:
- **Rust publishing on Windows = software encode** (§1.2): OpenH264/libvpx at 1080p60 while the publisher's game uses
  the CPU. WebView2 uses Chromium's encoder, which has **Media Foundation hardware H.264** on Windows (NVENC/AMF/QSV through MF).
- `getDisplayMedia` works in WebView2. It shows Chromium's own picker (screens, windows), captures with Windows Graphics Capture
  and supports `frameRate: 60`. The host can veto or defer with `ScreenCaptureStarting` (`ICoreWebView2_27`, stable since
  1.0.2903.40) ([MS docs](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_27)).
  wry's `PermissionRequested` handler maps mic/camera/geolocation. Check that display capture isn't auto-denied.
- Caveats:
  - **Audio:** Chromium on Windows offers *system* loopback only when sharing a whole screen. It includes Pulse's own voice
    output (played by `pulse-app.exe` through cpal, not by the webview), so **friends would hear themselves**. Per-app audio
    (Discord-style) needs Windows process-loopback capture (`AUDIOCLIENT_PROCESS_LOOPBACK`, Win10 2004+) in Rust,
    published as `screen_share_audio` from the Rust connection. Recommendation: video from the webview, audio from Rust.
  - Exclusive-fullscreen games can capture black through WGC. Borderless windowed works. This is the same limitation Rust's
    `DesktopCapturer` (WGC/DXGI) has.
  - Chromium's picker UI is not themeable. A custom picker (Rust lists windows) would need `preferCurrentTab`-style
    tricks that WebView2 doesn't expose. Live with the Chromium picker.
- **Discord itself:** an Electron (Chromium) shell, with voice/video capture, encode and decode in a native N-API module
  (`discord_voice.node`, their libwebrtc fork with hardware codecs). Community reverse-engineering notes the Linux client
  bypasses `getDisplayMedia` entirely ([edisionnano repo](https://github.com/edisionnano/Screenshare-with-audio-on-Discord-with-Linux)).
  I found **no public source** on how Discord's native decoded frames reach the Electron renderer, so don't treat any claim
  about it as fact. The relevant point: Discord can afford a native media engine with hardware codecs because it maintains a
  libwebrtc fork. Pulse can't, so lean on Chromium's.

---

## 4. Option B: native rendering beside or over the webview

### 4.1 Child native surface (wgpu/D3D11 swapchain) over a placeholder div
- Mechanics: create a `WS_CHILD` HWND (or a Tauri child with the `unstable` multiwebview feature) as a sibling above WebView2's
  controller HWND. Upload I420 to 3 textures and convert in a shader. One CPU→GPU copy, the most efficient *Rust-decode* render.
- Problems:
  - **Airspace:** nothing HTML can draw over the video. Tooltips, context menus, toasts, dropdowns and the "stop
    watching" overlay all get clipped. You'd re-implement overlays natively or hide the surface while menus are open.
  - **Position sync:** a `ResizeObserver`/scroll listener in JS sends IPC to move the HWND, so the HWND lags 1–3 frames behind
    scroll and resize, and the tear is visible during window drags. Placeholder clipping inside scroll containers has to be computed by hand.
  - **Transparency:** WebView2-over-wgpu transparency only became possible after a `WS_CLIPCHILDREN` fix
    ([wry#1331](https://github.com/tauri-apps/wry/issues/1331), 2024-08 → fixed 2025-01-01). Flicker was reported with
    wgpu + transparent Tauri v2 windows ([tauri#9220](https://github.com/tauri-apps/tauri/issues/9220)), and WebView2 transparency regressions
    in v2 ([tauri#8308](https://github.com/tauri-apps/tauri/issues/8308)). Community attempts at wgpu-over-webview, including
    a Steam-overlay decoy swapchain, were fiddly and platform-specific, and Linux was declared infeasible
    ([tauri disc. #11944](https://github.com/orgs/tauri-apps/discussions/11944), 2024-12 → 2026-09).
  - **Linux/Wayland:** tao uses GTK. You can't position a separate toplevel on Wayland, and embedding a wgpu surface in a GTK
    widget needs subsurface or GLArea work. Hyprland makes this worse.
  - **Fullscreen:** the easy part. Make the native window itself fullscreen.
- Examples: [wry `examples/wgpu.rs`](https://github.com/tauri-apps/wry/blob/dev/examples/wgpu.rs),
  [clearlysid/tauri-wgpu-cam](https://github.com/clearlysid/tauri-wgpu-cam) (webview UI over a wgpu camera texture, same window),
  [kenichi-preview](https://awesome.ecosyste.ms/projects/github.com%2Fsimplearyan%2Fkenichi-preview) (Tauri v2 + FFmpeg + wgpu
  video player, 1 star). None are production video-call apps.
- **Verdict:** it doesn't fix the real Windows bottleneck (software decode) and adds the airspace and sync problems. Reject for
  in-app tiles. A **pop-out "theatre" window** that is *only* a native wgpu window (no webview, no overlays besides what we draw) is a
  plausible later feature, but not the base.

### 4.2 How Zed does it (gpui + LiveKit Rust)
`crates/livekit_client/src/remote_video_track_view.rs` + `livekit_client/playback.rs` (main, read 2026-10-13):
- macOS: zero-copy. It retains the `CVPixelBuffer` from `buffer.as_native()` (VideoToolbox hardware decode) and draws it with `gpui::surface`.
  Otherwise it converts I420 to NV12 into a `CVPixelBufferPool`.
- **Windows/Linux:** `buffer.to_argb(VideoFormatType::ARGB, …)` into a **freshly `alloc`'d buffer per frame**, then
  `RgbaImage` → `Arc<RenderImage>` → `gpui::img`, and the previous frame's texture is freed with `window.drop_image`. That means software
  decode + CPU RGB conversion + RGBA upload (8.3 MB per 1080p frame).
- Lesson: even a fully native GPU UI on Windows pays software decode and a CPU colour conversion when it uses the LiveKit Rust SDK.
  Zed avoids airspace only because gpui *is* the whole window. Our §2.3 path (YUV upload + shader) is lighter than
  Zed's on Windows.

---

## 5. Decode costs summary

| Where | Windows | Linux (dev) |
|---|---|---|
| libwebrtc in Rust (livekit 0.9.3) | **Software only**: FFmpeg H.264, libvpx VP8/VP9, dav1d AV1 → CPU I420 | Software, or NVDEC H.264/H.265 if built with `cuda.h` (`LK_DISABLE_NVDEC` to opt out), still copied to I420 |
| Chromium WebRTC (WebView2) | **Hardware** D3D11 for H.264/VP9/AV1 where the GPU supports it (all recent NVIDIA/AMD/Intel do H.264+VP9; AV1 on RTX 30+/RX 6000+/Arc), texture stays on GPU. VP8 = software | n/a (WebKitGTK) |
| GPU→CPU readback | not applicable (Rust never gets GPU handles on Windows) | the NVDEC path does it internally (NV12→I420 to host memory) |

Software decode throughput measured here (Chrome 151 WebCodecs, i9-13900HX, multithreaded, 1080p 8 Mbps synthetic text and motion):
H.264 991 fps, AV1 729 fps, VP9 421 fps. At 60 fps that's roughly 6–15% of the decoder's throughput budget. Real CPU% in
libwebrtc (fewer threads) is likely 0.3–1 core at 1080p60 and more at 1440p, versus ≈0 with hardware. Hardware decode in WebCodecs
was unavailable in headless Linux Chrome (no VAAPI), so no hardware number here.

---

## 6. Recommendation matrix

Scores: ++ best · + good · 0 ok · − poor · −− unacceptable

| Option | Quality | Latency (added on top of WebRTC) | CPU/GPU (viewer, 1080p60) | Complexity | Linux story | Risk |
|---|---|---|---|---|---|---|
| **C. livekit-client in WebView2 (video only)** | ++ native `<video>`, hardware decode | ++ ~0 | ++ ≈ 0 CPU, GPU video engine | + JS SDK, 2nd token/identity, roster filtering | −− no WebRTC in WebKitGTK → needs A3 | + mainstream path. Risks: 2 connections, A/V sync if audio comes from Rust |
| **A3. Rust decode → local WS → WebGL2** | + lossless I420, our shader | + ≈5–15 ms (WS ~4 ms + upload 3 ms + rAF) | − software decode 0.3–1 core + ~10% for copies | 0 WS server, frame protocol, worker renderer | ++ same code on Linux (and Windows) | + measured headroom; Windows CPU while gaming is the risk |
| A4. Rust decode → SharedBuffer ring → WebGL2 | + | + slightly better than A3 | − same decode, fewer copies | − unsafe COM, Windows-only | −− Windows-only | 0 unproven perf ("weirdly slow" reports) |
| A1. Tauri Channel / IPC | + | −− 40–200 ms per frame | −− UI thread blocked | ++ trivial | 0 | −− 8–16 fps ceiling |
| A2. MJPEG / custom scheme | − text artefacts | − | −− JPEG enc+dec | 0 | 0 | − |
| A5. SharedArrayBuffer | n/a | n/a | n/a | n/a | n/a | not a Rust↔JS transport |
| B1. Native child surface over div | ++ | ++ | − software decode, 1 upload | −− airspace, position sync, transparency bugs | −− Wayland | −− |
| B2. Pop-out native wgpu window | ++ | ++ | − software decode | − | − (Wayland toplevel ok, but GTK/wgpu glue) | 0. Later "theatre mode" only |
| A6. Rust forwards encoded frames → WebCodecs hardware | ++ | ++ | ++ | −− webrtc-sys fork | − (GStreamer WebCodecs) | −− |

**Primary: C** (WebView2 WebRTC for screen-share video, subscribe-only `#v` identity; ideally the publisher also
uses `getDisplayMedia` in WebView2 for hardware H.264 encode, with stream *audio* in Rust).
**Fallback: A3** (Rust decode → localhost WebSocket → WebGL2 in an OffscreenCanvas worker). It is the Linux path from day
one and the escape hatch if C hits a wall on Windows. The JS `<ScreenView>` component is built to accept either a `MediaStreamTrack` or a
frame-socket source.

---

## 7. The spike (≈2 days, throwaway code under `spikes/screenshare/`)

### Day 1: prove C on Windows (WebView2)
1. **Tokens:** no server change yet. Mint with `lk token create --identity <uid>#v --room <room> --join --hidden
   --valid-for 1h` (viewer) and `<uid>#s` with publish rights (publisher), against `just dev-livekit`, or the deployed
   LiveKit with the Windows boxes on the real network.
2. **Viewer page** in a throwaway Tauri app (copy `create_main_window` settings: frameless, data dir) with
   `livekit-client`: `new Room({adaptiveStream:true, dynacast:true})`, `connect(url, token, {autoSubscribe:false})`.
   On `TrackPublished` with `source === ScreenShare`, call `pub.setSubscribed(true)` and attach to a `<video>`. Test fullscreen
   (`requestFullscreen`) and a resizable tile in a scroll container.
3. **Publisher A (synthetic):** `lk room join --identity bot#s --publish <1080p60 h264 file>` (loop), so the viewer
   is measured without a game in play.
   **Publisher B (real):** same page with `getDisplayMedia({video:{frameRate:60, width:1920, height:1080}})` →
   `localParticipant.publishTrack(track, {source: ScreenShare, videoCodec:'h264', screenShareEncoding:
   {maxBitrate: 6_000_000, maxFramerate: 60}})` while running an actual game (borderless). Try `videoCodec:'av1'`
   on an RTX 40 box.
4. **Measure** (viewer and publisher, 5 minutes each):
   - `pc.getStats()` every 1 s: `inbound-rtp.decoderImplementation` (expect a hardware/D3D11 label, **not** `libvpx`/`FFmpeg`),
     `powerEfficientDecoder`, `framesPerSecond`, `framesDropped`, `freezeCount`, `totalDecodeTime/framesDecoded`,
     `jitterBufferDelay/jitterBufferEmittedCount`. Publisher: `outbound-rtp.encoderImplementation` (expect MediaFoundation),
     `qualityLimitationReason`, `framesPerSecond`.
   - Task Manager → Details: CPU% of all `msedgewebview2.exe` processes plus `pulse-app.exe`. Performance tab: GPU "Video Decode"
     and "Video Encode" engine %. Also the publisher's in-game FPS with and without sharing.
   - Glass-to-glass: share a window showing a ms clock (e.g. `performance.now()` page), and phone-photograph the
     publisher and viewer screens together, 10 samples. Compare with Discord on the same pair if possible.
   - The `ScreenCaptureStarting` event fires (log it), the permission isn't auto-denied, and the picker shows windows and screens.
   - **Pass:** ≥58 fps at 1080p60 sustained, hardware decoder confirmed, viewer total CPU <5% (6-core), glass-to-glass ≤200 ms
     over the internet / ≤120 ms LAN, no effect on Rust voice (`voice://event` stats unchanged).
5. **Dual-connection checks:** the Rust voice client and the webview `#v` in the same room. Confirm the voice roster doesn't list `#v`
   (needs a filter) and the webhook ignores it (server log). Kill and reconnect each side independently. Token
   expiry at 10 min while watching (livekit-client reconnect with an expired token: does it need a refresh hook?).

### Day 2: prove A3 (Linux + Windows comparison), and close open questions
1. Rust: subscribe to the screen-share track, `NativeVideoStream::new(track.rtc_track(), Some(1))`, take
   `buffer.to_i420()` (or `as_i420()`), optionally `scale()` to the tile size, and send over a `tokio-tungstenite` server on
   `127.0.0.1:0` with a secret. Header + Y/U/V, at most one frame in flight.
2. JS: Worker + `OffscreenCanvas` + WebGL2 R8 planes + YUV shader (the shader in `scratchpad/bench/page.html` works).
   Ack each frame. Draw in `requestAnimationFrame`.
3. Measure on Linux (WebKitGTK!) and Windows (WebView2): delivered fps, **frame age** at draw (`Date.now()` vs
   `timestamp_us` plus a clock offset, or a Rust-stamped send time), `pulse-app` CPU (decode + copy separately via `tracing`
   spans), renderer CPU, and dropped frames, at 1080p60 and 1440p60, for VP9 and H.264 publishers.
   WebKitGTK WS throughput is **unmeasured**, so this is the important new number.
4. 30 min each, for the record:
   - (a) the same frames over a `tauri::ipc::Channel<InvokeResponseBody::Raw>` on WebView2, to confirm the ≈50 MB/s ceiling.
   - (b) a 3-slot SharedBuffer ring on WebView2 (`with_webview` → `controller().CoreWebView2()` → `cast::<ICoreWebView2_17>`),
     to decide whether A4 is ever worth it.
5. Verify the `auto_subscribe` trap: with a video publisher in the room, measure `pulse-app` CPU before and after setting
   `auto_subscribe:false` plus audio-only subscription.

**Output of the spike:** a FINDINGS.md with the tables above filled with Windows numbers, plus a go/no-go on C as primary.

---

## 8. Sources (accessed 2026-10-13 unless noted)
- Code: `client/src-tauri/src/lib.rs`, `tauri.conf.json`, `src/voice/mod.rs:400,566`, `server/src/voice/routes.rs`;
  `~/.cargo/registry/src/*/{libwebrtc-0.3.50/src/native/video_stream.rs, libwebrtc-0.3.50/src/video_frame.rs,
  webrtc-sys-0.3.47/{build.rs, src/video_decoder_factory.cpp, src/video_encoder_factory.cpp, src/nvidia/h264_decoder_impl.cpp},
  tauri-2.12.1/src/ipc/channel.rs, wry-0.57.0/src/webview2/mod.rs, webview2-com-sys-0.39.1/src/bindings.rs}`;
  prebuilt `target/debug/build/scratch-*/out/livekit_webrtc/livekit/win-x64-release-webrtc-89d790b/.../args.gn`.
- Benchmarks: my own, `scratchpad/bench/{server.mjs,page.html}`, headless Chrome 151 on Linux, i9-13900HX, Intel UHD via ANGLE-GL.
  **Proxy for WebView2, not WebView2.**
- [tauri discussion #11915](https://github.com/orgs/tauri-apps/discussions/11915): Windows IPC 10 MB ≈200 ms (2024-12-09).
- [tauri issue #13405](https://github.com/tauri-apps/tauri/issues/13405): large binary payloads (2025-05-09 → 2025-10-09).
- [tauri discussion #5690](https://github.com/tauri-apps/tauri/discussions/5690): IPC improvements, no WebView2 streaming responses.
- [kiri PR #40](https://github.com/ChloeVPin/kiri/pull/40): Tauri invoke vs SharedBuffer on windows-latest (2026-09-23).
- [WebView2 ICoreWebView2_17 / SharedBuffer](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_17) (stable 1.0.1661.34).
- [WebView2 ICoreWebView2_27 / ScreenCaptureStarting](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_27) (stable 1.0.2903.40; doc dated 2026-09-21).
- [PSPDFKit: Sharing buffers from UWP to WebView2](https://pspdfkit.com/blog/2023/sharing-buffers-from-uwp-to-webview2/) (2023).
- [wry #1331](https://github.com/tauri-apps/wry/issues/1331) (2024-08 → 2025-01-01), [tauri #9220](https://github.com/tauri-apps/tauri/issues/9220),
  [tauri #8308](https://github.com/tauri-apps/tauri/issues/8308), [tauri discussion #11944](https://github.com/orgs/tauri-apps/discussions/11944) (2024-12 → 2026-09).
- [wry examples/wgpu.rs](https://github.com/tauri-apps/wry/blob/dev/examples/wgpu.rs), [clearlysid/tauri-wgpu-cam](https://github.com/clearlysid/tauri-wgpu-cam).
- Zed: [remote_video_track_view.rs](https://github.com/zed-industries/zed/blob/main/crates/livekit_client/src/remote_video_track_view.rs),
  [playback.rs](https://github.com/zed-industries/zed/blob/main/crates/livekit_client/src/livekit_client/playback.rs) (main, 2026-10-13).
- [LiveKit: connecting / identity uniqueness](https://docs.livekit.io/intro/basics/connect/), [access tokens (hidden grant)](https://docs.livekit.io/guides/access-tokens).
- Chromium [d3d11_video_decoder.cc](https://chromium.googlesource.com/chromium/src/+/main/media/gpu/windows/d3d11_video_decoder.cc).
- Discord: [edisionnano/Screenshare-with-audio-on-Discord-with-Linux](https://github.com/edisionnano/Screenshare-with-audio-on-Discord-with-Linux)
  (community; the native module handles media). No authoritative source found for Discord's decoded-frame render path.
