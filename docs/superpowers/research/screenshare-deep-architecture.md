# Screen share: challenging the architecture

**Date:** 2026-10-14. **Scope:** an adversarial review of the proposed screen-share architecture from
`screenshare-summary.md` ("video in WebView2 via `livekit-client` as a hidden second participant `<uid>#v`; voice and
stream audio in Rust"), with every serious alternative weighed, a decision matrix, a recommendation, the spike gates
that must confirm it, and the way back if it turns out wrong.

**Inputs:** the three reports beside this file (capture-encode, render, server-quality), the pulse-app source at
commit `448da41` (read-only), the LiveKit Rust SDK and Chromium sources, and a small prototype in
`/tmp/claude-1000/screenshare-arch/` (not in the repo).

**Labels:** **[cited]** a primary source (URL and date read; every URL below was read on 2026-10-14 unless noted).
**[code]** read in a source tree (path given). **[measured]** I ran it today. **[est.]** my engineering estimate.
**[speculation]** plausible but unchecked; the spike must settle it.

---

## 0. TL;DR

- **The proposal works, but its cost sits in the seams, not the media.** Two LiveKit participants per user means
  two signalling sockets and up to four PeerConnections, two reconnect state machines, a new token route, a voice
  membership check the server can't make reliably after a restart, roster and quality-event filtering, and a Linux
  viewing path that shares nothing with the Windows one. None of these is a blocker. Together they are most of the
  work and most of the bugs (§2).
- **The Rust SDK still has no Windows hardware codec,** as of `webrtc-sys` 0.3.49 / `livekit` 0.9.5 (2026-10-12). The
  Media Foundation PR #1282 has had no activity since 2026-07-29. So the hope that "the Rust SDK grew NVENC/D3D11"
  doesn't flip the decision (§1).
- **Two upstream changes do open a new option:**
  - **#1511, released in 0.9.5:** fixes the pre-encoded publish path.
  - **#1515, opened 2026-10-11 and not yet merged:** a **pass-through video decoder**. Rust receives the encoded H.264
    instead of decoded pictures. Rust can then forward ~1 MB/s of encoded video over a localhost WebSocket to the
    webview, and **WebCodecs decodes it in hardware inside WebView2**.
  - Result: one LiveKit connection per viewer, hardware decode, and the same WebSocket transport the Linux path needs
    anyway (§3.3).
- **My prototype** fed a 1080p60 H.264 stream over a localhost WebSocket into WebCodecs. Results:
  - Transport plus decode added **6 ms p50 / 11 ms p95**, and drawing on the next animation frame took that to
    **18 ms p50**, with 0 decode errors [measured, Linux headless Chrome 151, *software* decode].
  - It also found a trap: drawing every decoded frame straight from the decoder callback built up **0.5–3 s of lag**
    within 20 s. You must draw only the newest frame, once per animation frame (§4).
- **Recommendation: a split design.**
  - **Publish** from WebView2 with `getDisplayMedia` + `livekit-client`, as a *publish-only* participant `<uid>#s`
    that exists only while the user is live. Chromium gives hardware H.264, simulcast and mature rate control for free.
  - **View** in the existing Rust voice connection via the pass-through decoder, then a localhost WebSocket into a
    WebCodecs hardware decoder.
  - **Stream audio** from Rust (process loopback) on the user's own identity.
  - So there is **no per-viewer second participant**, and the publisher's second participant is short-lived and simple.
- **The fallback is the original proposal's JS viewer (`<uid>#v`), and the spike decides between them.** If #1515 is
  rejected upstream, can't be patched in cleanly, or fails the gates in §6, the viewer becomes the original JS
  `<uid>#v` design. The publisher half is the same in both, so nothing is thrown away.
- **Rejected:**
  - **B (all voice in the webview):** loses the voice pipeline and Linux voice.
  - **C1 as v1 (our own capture plus Media Foundation encode):** weeks of codec work. It stays as a later upgrade for a
    custom picker, HDR and exact game audio.
  - **D (a Chromium/CEF/OBS sidecar):** ships a second Chromium for nothing.
  - **E (WHIP/WHEP, Cloudflare, P2P):** a second media system, or uplinks that can't carry it.
  - **WebView2 TextureStream:** an experimental API since 2023, never stable, and Microsoft says not to ship it.
- **One latent bug applies to every option:**
  - The client keys receive tasks and mixer channels by *participant identity* (`voice/rx.rs`, `voice/mixer.rs`).
  - A second audio track from the same user (stream audio) would **replace and kill that user's mic stream**, and
    unsubscribing it would drop the mic too.
  - Key them by track SID, or by identity plus source, before any stream audio lands (§2.2).

---

## 1. Does the Rust SDK have hardware codecs now? (the decision-flipper check)

| Check (2026-10-14) | Result | Label |
|---|---|---|
| Latest releases | `webrtc-sys` 0.3.49, `libwebrtc` 0.3.52, `livekit` 0.9.5, all 2026-10-12. Their notes cover the pass-through fix, `PeerConnection::set_bitrate`, dynacast re-apply and `Room::close` finality. Nothing about Windows hardware. | [cited] `gh api repos/livekit/rust-sdks/releases` |
| 0.9.4 / 0.3.48 (2026-10-07) | VAAPI lifecycle fixes (Linux), and "screen share stays uncapped" by the start-bitrate ramp | [cited] same |
| PR #1282 "Windows hardware H264 encode/decode via MediaFoundation" | **Open**, last activity 2026-07-29, one comment (the CLA bot). Validated on Intel only. Even merged, decode produces CPU frames | [cited] https://github.com/livekit/rust-sdks/pull/1282 |
| NVENC work | #1194 (SetRate), #1297, #1191 (NVENC AV1) are merged, but NVENC is built only in the `linux` arm of `build.rs` | [cited] PR list; [code] `webrtc-sys-0.3.47/build.rs` L150–244 |
| PR #1511 (pre-encoded stays decodable when frames drop) | **Merged** 2026-10-12, shipped in 0.9.5 / 0.3.49 | [cited] https://github.com/livekit/rust-sdks/pull/1511 |
| **PR #1515 "Pass-through video decoder: receive encoded video without decoding it"** | **Open** since 2026-10-11, rebased onto `main` 2026-10-13. External author, reviewed only by a bot so far. Adds `VideoDecoderBackend::PassThrough`, `NativeEncodedVideoStream` (codec, frame type, size, RTP/NTP timestamps, `after_gap`, zero-copy payload, `request_keyframe()`) and `PeerConnectionFactory::with_video_decoder_backend`. Changes only `webrtc-sys` glue and `libwebrtc` Rust; **does not touch the `livekit` crate** | [cited] https://github.com/livekit/rust-sdks/pull/1515, https://github.com/livekit/rust-sdks/issues/1514 |
| `livekit-capture` crate (new, 0.1.3–0.1.5) | Capture sources: pattern, clock, encoded h26x, GStreamer (merged); V4L2/AVFoundation/Argus/RTSP (open). **No Windows screen or WGC source, no Windows encoder** | [cited] PR list #1348–#1370 |

**Verdict:** no flip. Windows publishing from Rust is still either software or our own encoder via `PreEncoded`.
Windows viewing in Rust is still software, **unless** we take the pass-through decoder and decode somewhere else
(WebCodecs). That is the new option C2 below.

**How the `livekit` crate picks its decoder** [code] `livekit-0.9.3/src/rtc_engine/lk_runtime.rs` L76–96: one
process-wide `PeerConnectionFactory`, created lazily by `LkRuntime::instance()` with
`PeerConnectionFactory::with_options(zero_playout_delay, true)`.
- Using #1515 needs a small patch (≈20–40 lines [est.]) so `LkRuntime` builds its factory with
  `with_video_decoder_backend(PassThrough)`, in the same way `enable_zero_playout_delay` already toggles a
  factory-wide option.
- Process-wide is what we want on Windows: Rust should never software-decode video.
- **Patchability:** `webrtc-sys` compiles its C++ glue with `cxx_build` against a *downloaded prebuilt* libwebrtc
  ([code] `build.rs` L65–113, `webrtc-sys-build` `download_url`). #1515 adds glue files only, so a git `[patch]` builds
  without rebuilding libwebrtc [est.; verify in the spike].

---

## 2. Option A (the proposal): attacking its weak points

The design under test:
- **Viewer:** `livekit-client` in WebView2, identity `<uid>#v`, hidden, subscribe-only.
- **Publisher:** `getDisplayMedia` as `<uid>#s`.
- **Voice and stream audio:** Rust, on identity `<uid>`.

### 2.1 Connections and lifecycle

1. **More PeerConnections than it looks.**
   - LiveKit clients historically use *up to two* PeerConnections each, one to publish and one to subscribe
     [cited] https://docs.livekit.io/reference/internals/client-protocol.
   - `livekit-client` ≥2.17 (2026-01-12) defaults to a single-PC mode with fallback
     [cited] https://github.com/livekit/client-sdk-js/releases/tag/v2.17.0. The Rust SDK 0.9.x has its own transport
     logic.
   - So a watching user can hold 3–4 PCs, each with its own ICE, DTLS handshake, TURN allocation (if relayed) and
     keep-alives, plus 2 signalling WebSockets.
   - The SFU cost is negligible for 6 people. The client and NAT cost is small. Time-to-watch is the real cost, see
     the next point.
2. **"Watch" is slow if the JS room connects on demand.**
   - Clicking Watch must first run a full JS connect: token fetch, signalling WS, ICE, DTLS. That is ≈0.3–1.5 s on SA
     links [est.], *before* the subscribe and keyframe (the server report's 250–600 ms first frame).
   - Pre-connecting while in voice avoids that, but doubles the always-on connections.
   - Contrast C2: Watch = `set_subscribed(true)` on a connection that already exists.
3. **Two reconnect state machines.** These have to be coordinated by hand:
   - The Rust room emits `Reconnecting/Reconnected/Disconnected` (`voice/mod.rs` L621–636).
   - The JS room has its own.
   - A network blip can leave voice reconnected and video stuck (or the reverse).
   - The UI needs a merged state.
   - The Rust `Disconnected` path must also tear down the JS room.
   - `Room::close` races were only fixed in Rust 0.9.5 ("a closed room kept reporting `Connected` … about half the
     time") [cited] livekit/v0.9.5 release notes.
4. **A lurking video participant after leaving voice.**
   - If the JS teardown fails (a webview reload, a crash, an exception), `<uid>#v` stays in the room watching, with no
     voice presence. The server can't remove it: it holds no RoomService client today
     ([code] `server/src/voice/routes.rs`: only token + webhook).
   - Token refresh makes this worse. LiveKit "proactively issues refreshed tokens to connected clients", so a TTL
     doesn't bound the session [cited] https://docs.livekit.io/frontends/authentication/tokens/.
5. **The webview dies independently.**
   - WebView2 renderer crashes and the Linux `WebKitWebProcess` reload (`lib.rs`) kill the JS room but not voice.
   - On reload, the new `#v` connection kicks the old one (`DUPLICATE_IDENTITY`). That is fine, but the stream state
     must be rebuilt from scratch.
   - This one actually favours keeping voice in Rust.

### 2.2 Identity, server and roster

6. **A new token route with a weak membership check.**
   - "Only issue `#v` while the user is in voice" relies on `VoiceState`. That state is in memory and **empty after a
     restart**, and LiveKit does not re-report connected participants
     ([code] `server/src/voice/state.rs` L1–3, `voice/mod.rs` L6–8).
   - After a server deploy, a user in voice would be *refused* a video token until they rejoin. The check must either
     be soft (channel access only, as the voice token does today) or rebuild state from LiveKit `ListParticipants`.
7. **The webhook ignores `#v`/`#s`, so the server can't show "LIVE".**
   - `"<uid>#s".parse::<UserId>()` fails, so it is acked and ignored ([code] `routes.rs` L80–90). That correctly avoids
     ghost voice sessions.
   - But it also means the server never learns who is streaming. Discord-style "LIVE" badges for people *not* in the
     channel need the webhook to handle `track_published`/`track_unpublished` for `#s` and broadcast a gateway event.
   - That is a server change in A **and** in the split design. Budget it.
8. **Grants should be tightened.** Today's token grants `can_publish: Some(true)` for everything (`routes.rs` L46–52).
   With screen share:
   - `<uid>` should get `can_publish_sources: [microphone, screen_share_audio]`.
   - `<uid>#s` should get `[screen_share]`.
   - `<uid>#v` should get `can_publish: false`, `hidden: true` [cited] tokens doc above.
   - Note: `livekit-api` 0.9.0 (2026-10-12) made these grants `Option<bool>`; the server already writes `Some(..)`
     [cited] livekit/v0.9.5 notes.
9. **Roster and event leakage in the Rust client.** The Rust room sees `#s` (and `#v`, if not hidden) as
   participants. These must be filtered by suffix:
   - `ConnectionQualityChanged` (`voice/mod.rs` L612–619 emits it for *any* identity);
   - `ParticipantDisconnected`;
   - speaking state;
   - per-user volume.
10. **The latent mic-killer bug (all options):**
    - `TrackSubscribed` keys the receive task by `participant.identity()` and calls `rx.replace(id, task)`, which
      **aborts the existing task for that identity** ([code] `voice/mod.rs` L566–600, `voice/rx.rs` L14–17).
    - `TrackUnsubscribed` calls `drop_for(id)` and `mixer.remove(id)`.
    - If stream audio is published on `<uid>` (as every design proposes), subscribing to it kills that user's mic
      audio, and stopping the stream silences them.
    - The receive stream is also created mono (`NativeAudioStream::new(.., 1)`), which would downmix stereo game audio.
    - Fix: key by track SID or `(identity, source)`, give stream audio its own mixer channel and volume, and use
      2 channels.

### 2.3 Media and sync

11. **A/V sync across participants is impossible for libwebrtc.**
    - Its lip-sync works only inside one PeerConnection, between tracks in the same sync group.
    - In A, video arrives in Chromium (`#s` → `#v`) and stream audio arrives in Rust (`<uid>`) on a different PC, from
      a different participant, through a different jitter buffer.
    - Pulse also plays audio outside libwebrtc's playout (`NativeAudioStream` into our mixer with a 200 ms cap,
      `PEER_BUFFER_MS`).
    - Sync becomes a constant offset we measure and tune: delay the audio in our mixer, or raise the video receiver's
      jitter-buffer target in JS [speculation on the size: tens to ~150 ms].
    - The split design has the same problem. Only "everything in JS" (B) gets sync for free.
12. **Picker and process ID.**
    - `getDisplayMedia` shows Chromium's picker, and the app is not told which window was picked (no HWND).
    - Stream audio in Rust wants process loopback *INCLUDE* on that window's PID (capture-encode report §2.1). We'd
      have to map the track label (likely the window title [speculation]) to an HWND via `EnumWindows`, then to a PID.
      That is heuristic.
    - Fallback: EXCLUDE mode ("everything except Pulse") for all shares.
    - Chromium itself now has **per-application audio for window capture on Windows**, enabled by default but **only on
      Windows 11** (`kApplicationAudioCaptureWin`, `IsWindowsProcessLoopbackCaptureSupported() = GetVersion() >= WIN11`)
      [code] chromium `media/base/media_switches.cc` main, read 2026-10-14. That audio lands in JS, though, and
      whether WebView2's Edge build carries it is unverified.
13. **Chromium screen share isn't zero-copy.**
    - `IsZeroCopyEnabled()` returns false for `VideoContentType::SCREENSHARE`, so captured frames reach the hardware
      encoder through mapped memory, not shared textures [code] chromium
      `third_party/blink/renderer/platform/peerconnection/rtc_video_encoder.cc` L657–660.
    - Hardware H.264 is still used: Media Foundation encode is on by default (`kMediaFoundationVideoEncodeAccelerator`).
      It is gated on the OpenH264 software fallback being present (`UseH264AcceleratedEncoderForWebRTC`) [code]
      `media_switches.cc`, `webrtc_util.cc` L137–142.
    - Expect a few percent of a core for the copies at 1080p60 [est.]. Measure the publisher's CPU while gaming.
14. **Screenshare temporal layers can force software encode.** VP8 screenshare with temporal layers falls back to
    software only when `kWebRtcScreenshareSwEncoding` is on (off by default) [code] `rtc_video_encoder.cc` L473–481,
    L748. We use H.264 without temporal SVC anyway.

### 2.4 Footprint, CSP and maintenance

15. **Bundle size.** `livekit-client` 2.22.4 (2026-10-09) bundles to **560 KB minified / 147 KB gzip** [measured]
    (esbuild 0.25.10, `import {Room}`). It is served locally, so the cost is parse time only. Lazy-load it with
    `import()` when the user goes live (or watches, in A).
16. **Two SDK upgrade tracks against one server.**
    - Every LiveKit server upgrade must be tested against both the Rust SDK and `livekit-client`, and their protocol
      versions differ.
    - The single-PC default in `livekit-client` is itself recent and had regressions, judging by downstream pins to
      2.16.1 [cited, secondary] https://newreleases.io/project/npm/livekit-client/release/2.17.0.
    - **Pin exact versions** (no `^`).
17. **CSP.**
    - `connect-src` must add the LiveKit `wss://` host and the `https://` host for `/rtc/validate`. Today it is only
      `ipc: http://ipc.localhost` ([code] `client/src-tauri/tauri.conf.json`).
    - `media-src` isn't needed for `srcObject`.
    - The split design instead needs `connect-src ws://127.0.0.1:*` for the local socket, plus the LiveKit hosts for the
      publisher only.
18. **Memory.**
    - The extra renderer-side PeerConnection plus hardware decode surfaces is ≈50–150 MB while watching [est.].
    - The Rust-side `#1277` (~40 MB retained per video publish until the room closes) does not apply to A or the split
      design, because video is never published from Rust.
19. **Linux is a second, unrelated viewer.** WebKitGTK has no usable WebRTC (render report §3.3), so A needs the
    Rust decode → WebSocket → WebGL2 path as well. That makes three video code paths in total:
    - JS publish;
    - JS view;
    - Rust view.

**Summary of A:** every piece is mainstream and low-risk. The integration surface is wide: items 3–9, 11, 16 and 19
are glue work that exists only because there are two participants per user.

---

## 3. The alternatives in depth

### 3.1 B: everything in WebView2 (move voice to JS too)

- **Gains:**
  - one SDK and one connection;
  - lip-sync for free;
  - Chromium's AEC can even use system loopback as its reference (`kSystemLoopbackAsAecReference`, default on where
    built) [code] `media_switches.cc` L1680–1696.
- **Losses:**
  - The whole `voice/` pipeline: 5,599 lines across 16 files [measured, `wc -l`]. That includes the cpal device
    handling with watchdog and reopen, the DeepFilterNet denoiser, the gate, the latency/backlog fix (`backlog.rs`,
    `SEND_SLACK_CHUNKS`), the netstats, and the per-peer mixer.
  - The FINDINGS rule against playing audio through the webview.
  - **Linux voice entirely,** since WebKitGTK has no WebRTC.
- **Global hotkeys** would still work (Rust → IPC → JS mute).
- **Verdict:** reject. It throws away the most-tested part of the app to save glue.

### 3.2 C1: everything in Rust, own capture and encode, native rendering

- **Publish:**
  - WGC capture (`windows-capture` 2.0.1) → `ID3D11VideoProcessor` BGRA→NV12 → Media Foundation H.264 MFT
    (or NVENC/AMF directly, or FFmpeg hardware encode) → `PreEncoded`.
  - Now viable: the #1511 freeze fix is released in 0.9.5.
  - Costs:
    - MF async-MFT quirks per vendor (AMD rate control, SPS/PPS, an IDR on every bitrate change);
    - single layer only (`supports_simulcast = false`);
    - our own rate-control loop from `take_rate_control_request`;
    - the #1277 leak per publish (reuse one track);
    - #1309 C++ exceptions aborting the process (capture-encode report §4).
  - Effort: **3–6 weeks** to make it good on NVIDIA, AMD and Intel [est.].
  - In return:
    - our own themed picker with thumbnails;
    - exact HWND → PID for game audio;
    - Win11 borderless capture;
    - HDR tone mapping;
    - cursor control.
- **View:** four routes. None is good on Windows today:
  1. **Rust software decode → WebSocket → WebGL2.** Works; costs 0.3–1 core at 1080p60 on a viewer who may be gaming
     (render report §2.3).
  2. **Native child surface (wgpu/D3D11) over the webview.** Airspace (no HTML over the video), position lag on
     scroll/resize, transparency bugs, and a non-starter on Wayland (render report §4.1). Still software decode unless
     #1282 lands (MF decode), and even then frames come back as CPU I420.
  3. **WebView2 SharedBuffer ring.** Stable API; saves copies but not the decode (render report §2.4).
  4. **WebView2 TextureStream** (`ICoreWebView2ExperimentalTextureStream`: `CreateTexture`, `PresentTexture`, a JS
     `getTextureStream` that returns a `MediaStream`). It would let Rust put D3D11 textures straight into a `<video>`.
     **Its status kills it:**
     - experimental since prerelease SDK 1.0.1724 (2023);
     - still experimental in 1.0.4255-prerelease (doc dated 2026-09-03);
     - "Applies to WebView2 Win32: N/A";
     - Microsoft: experimental APIs "aren't guaranteed to be forward-compatible, and might be removed", need a preview
       Edge channel for "full support", and are "not recommended for production apps".
     - Sources [cited]:
       https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2experimentaltexturestream,
       https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/versioning.
     - It also needs Rust hardware decode into a D3D11 texture first (MF/NVDEC), which we don't have.
     - Original proposal: https://github.com/MicrosoftEdge/WebView2Feedback/pull/2743 (2022–2023).
- **Verdict:**
  - C1 *viewing* is dominated by C2 (§3.3), which reaches hardware decode with no native rendering.
  - C1 *publishing* is the right **long-term upgrade** if `getDisplayMedia`'s limits bite (picker, PID, HDR,
    exclusive-fullscreen behaviour). It is not v1.

### 3.3 C2 (new): Rust owns the LiveKit connection, and WebView2 does the decoding (WebCodecs)

How it works:
- **View:**
  - The voice `Room` subscribes to a stream with the pass-through decoder backend (#1515).
  - `NativeEncodedVideoStream` yields H.264 access units in Annex B (that is what libwebrtc hands a decoder) with RTP
    timestamps.
  - Rust sends them over the localhost WebSocket (the one the Linux fallback needs anyway) with a 16-byte header.
  - JS feeds `VideoDecoder` (`optimizeForLatency: true`, `hardwareAcceleration: 'prefer-hardware'`). On Windows,
    Chromium's WebCodecs decodes through the platform decoder (D3D11) [speculation for WebView2 specifically; verify
    with `isConfigSupported` and GPU "Video Decode" load].
  - Draw the newest `VideoFrame` on each animation frame (OffscreenCanvas in a worker), or pipe frames into a `<video>`
    via `VideoTrackGenerator` / Chromium's `MediaStreamTrackGenerator` for native fullscreen and PiP. MDN marks
    `MediaStreamTrackGenerator` non-standard and points to `VideoTrackGenerator`
    [cited] https://developer.mozilla.org/en-US/docs/Web/API/MediaStreamTrackGenerator.
  - Decoder error → `reset()`, then IPC → Rust `request_keyframe()`.
- **Publish (C2-pure):**
  - `getDisplayMedia` → `MediaStreamTrackProcessor` → WebCodecs `VideoEncoder` (H.264, hardware, `latencyMode:
    'realtime'`, Annex B) → WebSocket → Rust `capture_encoded_frame` (`PreEncoded`) on the user's own identity.
  - Rate requests from `take_rate_control_request` go to JS as `encoder.configure({bitrate})`.
  - Chromium **flushes the encoder before every reconfigure** (`ProcessReconfigure` → `Flush` → `ChangeOptions`)
    [code] chromium `third_party/blink/renderer/modules/webcodecs/video_encoder.cc` L993–1007, L1493–1580. Whether MF
    then emits an IDR is unverified.
  - Single layer, no simulcast.
- **Why it's attractive:**
  - one participant and one connection per user;
  - no token, identity, roster or lifecycle glue;
  - Watch is instant (subscribe on an existing PC);
  - Windows and Linux share the Rust subscribe code and the WebSocket transport (Linux just uses the default decoder
    and sends I420 to WebGL2);
  - ~1 MB/s over the socket instead of 187 MB/s.
- **Why it's risky:**
  - It depends on an **unmerged** PR plus our own `livekit` runtime patch.
  - We own the parts Chromium's WebRTC would otherwise do: keyframe recovery, the render loop, frame pacing and stats
    (§4 shows the render loop *will* bite if done naively).
  - A pure-C2 publisher gives up simulcast and Chromium's coupling of rate control to the encoder.
  - WebKitGTK WebCodecs runs on GStreamer, and the earlier report saw renderer aborts, so Linux keeps the I420 path.
- **Effort:**
  - Viewer: ≈1.5–2 weeks [est.].
  - Publisher (C2-pure): another ≈1–2 weeks [est.].
  - Ongoing: carrying a `[patch]` until #1515 merges, then rebasing onto each release.

### 3.4 Split (recommended): A's publisher + C2's viewer

- **Publish** exactly as in A: `livekit-client` in WebView2 as `<uid>#s`, only while live, with `getDisplayMedia`,
  H.264, hardware encode and one simulcast fallback layer.
- **View** as in C2: the Rust voice connection subscribes to `#s`'s screen-share track with the pass-through decoder.
  - It picks the layer with `RemoteTrackPublication::set_video_quality` [code] `livekit-0.9.3/src/room/publication/
    remote.rs` L271.
  - It sends frames over the WebSocket to WebCodecs.
- **Stream audio** from Rust process loopback on `<uid>` (after the keying fix in §2.2).
- **What this keeps from A:** the publisher gets Chromium's mature pieces: picker, WGC capture, hardware encoder,
  simulcast and rate control.
- **What it removes from A:**
  - the per-viewer second participant;
  - the "Watch" connect delay;
  - the `#v` token, membership check and lurking-viewer problem;
  - the second Linux viewing stack.
- **What remains of the two-participant glue** is a *short-lived, publish-only* `#s`, and it is simple:
  - its lifecycle is "Rust voice up and the user pressed Go Live";
  - tear-down follows Rust `Disconnected`;
  - a token route with `can_publish_sources: [screen_share]`;
  - roster filtering of one suffix;
  - the webhook `track_published` handling for "LIVE" (needed in any design).
- **Simulcast layer switches still reach the pass-through decoder.** The SFU switches layers on a keyframe, and the
  resolution can change mid-stream. #1515 says "a resolution change keeps the decoder" and takes sizes from the SPS.
  WebCodecs must accept an in-band SPS change on a keyframe [speculation; spike gate G4].

### 3.5 D: a dedicated helper process

- **D1, a CEF/Electron sidecar just for streaming:** it re-ships the Chromium that WebView2 already *is* (100+ MB
  [est.]). It still needs a second participant and inter-process glue. Nothing gained. Reject.
- **D2, an OBS-style native media sidecar** (Rust: WGC + MF encode + its own LiveKit connection):
  - This is C1's publisher in another process.
  - The upside is crash isolation from #1309 (C++ exceptions abort) and reclaiming #1277's leak by killing the process.
  - The downside is a second participant again, process management, and a second installer component.
  - Worth considering *only* if C1's publisher is ever built and proves crashy. Park it.

### 3.6 E: video over a path other than LiveKit

- **LiveKit Ingress (WHIP):**
  - a separate service plus **Redis** ("The Ingress service uses Redis messaging queues to communicate with your
    LiveKit server");
  - guidance of 4 CPUs / 4 GB per instance;
  - transcoding can be bypassed for WHIP ("consumes minimal resources");
  - [cited] https://docs.livekit.io/transport/self-hosting/ingress/, https://docs.livekit.io/home/ingress/overview/.
  - It only solves *publishing*. Viewers still need WebRTC.
  - Too heavy for the 1–2 OCPU A1 box. Reject.
- **Cloudflare Realtime SFU:**
  - first 1,000 GB/month free, then $0.05/GB egress, ingress free
    [cited] https://developers.cloudflare.com/realtime/sfu/pricing/;
  - it adds a vendor account, a second signalling model and a second media stack in the client;
  - our Oracle egress is already free up to 10 TB (server report §2.3).
  - Only interesting if the A1 box can't carry the packet rate even after the 2-OCPU resize. Reject for now.
- **P2P mesh:**
  - a streamer uploads one copy per viewer (5 viewers × 4.5–7 Mbps = 22–35 Mbps up);
  - the server report puts typical SA uplinks at 5–10 Mbps;
  - it only works for one viewer.
  - Reject, except as a possible "1:1 DM screen share" optimisation later.

---

## 4. Prototype: the C2 viewer transport (localhost WebSocket → WebCodecs)

**Setup** [measured] (`/tmp/claude-1000/screenshare-arch/wc/`):
- **Stream:** ffmpeg/x264 makes a 20 s 1080p60 H.264 High stream (testsrc2 plus a frame counter), 7 Mbps CBR-ish,
  no B-frames, Annex B, AUD + repeated headers. That is 1,200 access units, the largest 38 KB (synthetic content, so
  real keyframes will be larger).
- **Sender:** Node `ws` pushes one access unit every 16.7 ms with a send timestamp.
- **Receiver:** the page decodes with WebCodecs (no `description`, so Annex B, `optimizeForLatency`).
- **Browser:** headless Chrome 151 on Linux (i9-13900HX), **software decode** (no VAAPI in headless).

| Render strategy | Drawn / decoded | WS receive lag p50 / p95 | Send → decoded (or drawn) p50 / p95 / max |
|---|---|---|---|
| No draw (decode only), `--disable-gpu` | 1200 / 1200 | 1 / 2 ms | **6 / 9 / 19 ms** |
| No draw, default GPU flags | 1200 / 1200 | 1 / 2 ms | 6 / 11 / 26 ms |
| `drawImage` inside the decoder callback, `--disable-gpu` | 1042 decoded | 9 / 40 ms | **1356 / 3133 / 3180 ms** (backlog grows) |
| `drawImage` inside the callback, default flags | 1192 decoded | 7 / 36 ms | 550 / 1556 / 1727 ms |
| **Keep newest frame, draw once per `requestAnimationFrame`**, `--disable-gpu` | 1185 drawn, 15 superseded | 3 / 7 ms | **18 / 21 / 26 ms** |
| Same, default flags (headless GPU emulation) | 916 drawn, 284 superseded | 1 / 54 ms | 25 / 885 / 1021 ms |

**Takeaways:**
- The transport and decode are not the problem: ~6 ms, and the WebSocket carries ~1 MB/s with sub-2 ms lag.
- **Rendering policy is.** Drawing every frame from the decoder callback on the main thread backs up the decoder
  output queue, and latency grows without bound. The rule is the same as for the audio backlog fix: newest frame only,
  close superseded frames, and never queue.
- Headless GPU emulation is a poor proxy, so the default-flags rows are noise. The real numbers must come from WebView2
  with hardware decode on Windows (gate G2).

---

## 5. Decision matrix

Scores: ++ best · + good · 0 ok · − poor · −− unacceptable. Effort is in focused person-days for v1 [est.].

| | A: proposal (JS view `#v` + JS publish `#s`) | **Split: JS publish `#s` + Rust pass-through view** | C2-pure (Rust connection, WebCodecs encode and decode) | B: all JS | C1: all Rust (MF encode, Rust view) | D: sidecar | E: other path |
|---|---|---|---|---|---|---|---|
| Viewer CPU (Windows 1080p60) | ++ hardware | ++ hardware (WebCodecs) [verify G2] | ++ | ++ | − software decode, 0.3–1 core | depends | depends |
| Publisher CPU while gaming | + hardware MF, mapped copies | + same | + hardware via WebCodecs | + | ++ zero-copy textures | ++/+ | + |
| Quality controls | ++ simulcast, adaptive stream | + simulcast; Rust picks the layer | − single layer | ++ | − single layer | − | 0 |
| Time to first frame on Watch | − full connect first (or always-on 2nd PC) | ++ subscribe on existing PC | ++ | ++ | ++ | − | − |
| Connections per watching user | −− 2 participants, 3–4 PCs | + 1 participant (+1 short-lived only while *publishing*) | ++ 1 | ++ 1 | ++ 1 | − 2 | − 2 stacks |
| Server changes | − token route, membership check, webhook LIVE | 0 `#s` token + webhook LIVE | + webhook LIVE only | + | + | − | −− new service |
| Identity/roster glue | −− filter `#v` + `#s` everywhere | 0 filter `#s` | ++ none | ++ | ++ | − | − |
| A/V sync of stream audio | − our offset (cross-PC) | − our offset | − our offset (same participant) | ++ free | − ours | − | − |
| Linux dev story | − separate Rust I420 path | + same Rust subscribe + same WS; I420 payload | + same | −− no WebRTC, no voice | + | − | − |
| Upstream dependency risk | ++ none unmerged | − #1515 unmerged (+ small `livekit` patch) | − #1515 + rate-control glue | + | − #1282 stalled; PreEncoded OK | − | − |
| Code we own (media) | + little | 0 decoder loop + render loop + keyframe recovery | − also encoder loop + rate control | + | −− capture, encode, rate control, render | −− | −− |
| Picker / game-audio PID | − Chromium picker, no HWND | − same | − same | − | ++ own picker, exact PID | ++ | 0 |
| Effort v1 (days) [est.] | 12–18 | 14–20 | 18–28 | 25–40 (redo voice) | 25–40 | 30+ | 20+ |
| Failure blast radius | webview crash = video only | webview crash = video render only; LiveKit session survives | same | webview crash = voice too | Rust C++ abort = whole app (#1309) | isolated | isolated |
| **Overall** | **Good fallback** | **Recommended** | Later, if simulcast unneeded | Reject | Later publisher upgrade only | Park | Reject |

---

## 6. Recommendation and spike gates

**Build the split design:**
- **Publish:** WebView2 `getDisplayMedia` + `livekit-client` (pinned exactly, lazy-loaded) as a short-lived `<uid>#s`.
- **View:** Rust pass-through receive → localhost WebSocket → WebCodecs hardware decode, drawn newest-frame-only.
- **Stream audio:** Rust process loopback on `<uid>`.

**Keep the original JS `#v` viewer as the pre-planned fallback.**

**Do these first, whatever the outcome:**
1. Fix the receive keying (§2.2 item 10).
2. Set `auto_subscribe: false` with explicit audio subscription (render report §0).
3. Tighten grants with `can_publish_sources` (§2.2 item 8).
4. Teach the webhook `track_published` / `track_unpublished` for "LIVE" badges (§2.2 item 7).

**Spike gates** (Windows unless noted; a fail on a "must" gate moves the viewer to A's `#v`):

| Gate | Must prove | Pass bar | If it fails |
|---|---|---|---|
| **G1** Patch builds | `webrtc-sys`/`libwebrtc` from #1515 via `[patch]` plus a ≈30-line `LkRuntime` factory patch build on Windows MSVC and Linux against the prebuilt libwebrtc, with no libwebrtc rebuild. The voice tests still pass | Builds; `cargo test` green; voice join unchanged | Viewer → A (`#v`) |
| **G2** Hardware decode in WebView2 | `VideoDecoder.isConfigSupported({codec:'avc1.64002a', hardwareAcceleration:'prefer-hardware'})` is true in WebView2. Task Manager shows GPU "Video Decode" load and near-zero renderer CPU at 1080p60 | Hardware confirmed; viewer total CPU < 5% of 6 cores | Viewer → A (Chromium's WebRTC path is known to decode in hardware) |
| **G3** End-to-end latency and smoothness | Real `#s` publisher (game, 1080p60, 7 Mbps) → SFU → Rust pass-through → WS → WebCodecs → OffscreenCanvas in a worker. 10 min each on LAN and over the internet | ≥58 fps; glass-to-glass within +20 ms of A's `#v` on the same pair; no backlog growth; voice stats unchanged | Fix the render loop; else A |
| **G4** Loss and layer switches | Inject 2–5% loss (clumsy on Windows) and force simulcast layer switches (`set_video_quality`). Check: keyframe recovery via `request_keyframe()`, WebCodecs accepting an in-band SPS change, no permanent freeze | Recovers < 1 s; no decoder wedge | Rust-side fix; else A |
| **G5** Publisher (`#s`) | `outbound-rtp.encoderImplementation` shows MediaFoundation / hardware. Game FPS drop < 5%. `ScreenCaptureStarting` fires. Measure what the track label and `getSettings()` reveal (window title?) for PID mapping | Hardware encode; acceptable game impact | C1 publisher on the roadmap; v1 ships EXCLUDE-mode audio |
| **G6** Stream audio | Process loopback EXCLUDE (Pulse PID) on Win10 22H2 and Win11; stereo arrives; mic unaffected after the keying fix; A/V offset measured (clap test) and corrected with a constant | Offset after correction within ±45 ms | Ship with a manual offset slider |
| **G7** Lifecycle | Kill and restart the webview while live and while watching; drop the network 10 s; leave voice while live. Verify `#s` always leaves; the Rust viewer rebuilds | No orphaned `#s`; recovery without user action | Add server-side RoomService removal |
| **G8** Linux | Same Rust subscribe with the default decoder → I420 → WS → WebGL2 in WebKitGTK at 1080p30 | Smooth at 30 fps | Dev-only; accept a lower bar |

**What A's `#v` needs from the spike anyway** (so the fallback is ready): repeat G3 with the JS viewer on the same
publisher and record the time to first frame for "Watch" with and without pre-connect.

---

## 7. Migration paths if the choice is wrong

- **Split → A (viewer moves to JS):**
  - The publisher, stream audio, webhook, grants and keying fixes carry over unchanged.
  - Add the `#v` token route (with a *soft* membership check), JS room lifecycle tied to Rust `Disconnected`, and
    roster filtering of `#v`.
  - Keep the Rust WebSocket path for Linux.
  - Design rule to make this cheap: the UI's `<ScreenView>` takes a *source* interface (a `MediaStreamTrack` or a
    frame socket), never LiveKit objects directly.
  - Cost ≈4–6 days [est.].
- **Split → C2-pure (drop `#s`):**
  - Swap the JS publisher for WebCodecs `VideoEncoder` → WS → Rust `PreEncoded`.
  - Only worth it if simulcast proves unnecessary for 6 friends, or if the second participant causes trouble.
  - The viewer is unchanged. ≈1–2 weeks [est.].
- **Split → C1 publisher (own capture and encode):**
  - Triggered by picker/PID needs, HDR friends, exclusive-fullscreen games, or Chromium encoder problems (G5).
  - Replace `#s` with WGC + MF/NVENC → `PreEncoded` on `<uid>`.
  - Viewers don't change at all, because they already consume encoded H.264 from Rust.
  - This is the strongest structural argument for the split viewer: every future publisher upgrade lands behind the
    same receive path.
- **If #1515 is rejected upstream:**
  - The patch is glue-only, so carrying it is possible, but every SDK upgrade then needs a rebase.
  - If the rebase ever costs more than a day, fall back to A's viewer as above.
  - Watch the PR, and reconsider if LiveKit ships an official encoded-receive API instead.

---

## 8. Sources

**Code read (2026-10-14):**
- pulse-app `448da41`:
  - `server/src/voice/{routes.rs, state.rs, mod.rs}`;
  - `client/src-tauri/src/voice/{mod.rs, rx.rs, mixer.rs}`;
  - `client/src-tauri/tauri.conf.json`;
  - `client/ui/package.json`;
  - `Cargo.lock` (livekit 0.9.3, libwebrtc 0.3.50, webrtc-sys 0.3.47, wry 0.57.0, webview2-com 0.39.1).
- Cargo registry:
  - `livekit-0.9.3/src/rtc_engine/lk_runtime.rs`;
  - `livekit-0.9.3/src/room/publication/remote.rs`;
  - `webrtc-sys-0.3.47/build.rs`;
  - `webrtc-sys-build-0.3.19/src/lib.rs`;
  - `wry-0.57.0/src/webview2/mod.rs` (PermissionRequested L514–560; no display-capture kind).
- Chromium `main` via gitiles:
  - `media/base/media_switches.cc`;
  - `third_party/blink/renderer/platform/peerconnection/{rtc_video_encoder.cc, rtc_video_encoder_factory.cc,
    webrtc_util.cc}`;
  - `third_party/blink/renderer/modules/webcodecs/video_encoder.cc`.

**LiveKit:**
- Releases: https://github.com/livekit/rust-sdks/releases (webrtc-sys 0.3.48/0.3.49, libwebrtc 0.3.51/0.3.52,
  livekit 0.9.4/0.9.5, livekit-capture 0.1.3–0.1.5).
- PRs and issues:
  - https://github.com/livekit/rust-sdks/pull/1282
  - https://github.com/livekit/rust-sdks/pull/1511
  - https://github.com/livekit/rust-sdks/pull/1515
  - https://github.com/livekit/rust-sdks/issues/1514
- Docs:
  - https://docs.livekit.io/frontends/authentication/tokens/
  - https://docs.livekit.io/reference/internals/client-protocol
  - https://docs.livekit.io/transport/self-hosting/ingress/
  - https://docs.livekit.io/home/ingress/overview/
- JS SDK releases: https://github.com/livekit/client-sdk-js/releases/tag/v2.17.0 (single-PC default, 2026-01-12),
  https://github.com/livekit/client-sdk-js/releases/tag/v2.22.4 (2026-10-09).
- Secondary: https://newreleases.io/project/npm/livekit-client/release/2.17.0.

**Microsoft:**
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2experimentaltexturestream
  (1.0.4255-prerelease, doc 2026-09-03)
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/webview2experimental-idl
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/versioning
- https://github.com/MicrosoftEdge/WebView2Feedback/pull/2743

**Web platform:**
- https://developer.mozilla.org/en-US/docs/Web/API/MediaStreamTrackGenerator
- `windowAudio` TAG review: https://lists.w3.org/Archives/Public/public-webapps-github/2025Jun/0114.html. Shipping
  status is unverified.

**Others:** https://developers.cloudflare.com/realtime/sfu/pricing/

**Measured:**
- `/tmp/claude-1000/screenshare-arch/bundle/`: livekit-client bundle size.
- `/tmp/claude-1000/screenshare-arch/wc/{server.mjs, page.html, run.sh}`: the WebSocket → WebCodecs latency
  prototype.
