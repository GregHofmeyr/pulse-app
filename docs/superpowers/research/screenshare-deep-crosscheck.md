# Screen share deep dive: adversarial cross-check

**Date:** 2026-10-14. **Scope:** a cross-check of the ten deep reports (`screenshare-deep-*.md`) against each other,
against the earlier round (`screenshare-summary.md`, `-capture-encode.md`, `-render.md`, `-server-quality.md`), and
against primary sources. It lists the contradictions with a ruling for each, re-verifies the most load-bearing claims,
names the gaps, and ends with one de-duplicated spike checklist.

**How evidence is ranked in the rulings:** measured on the target platform > read in source code > cited docs or
issues > an estimate. Almost every "measured" number in the deep reports comes from **Linux, headless Chrome 151,
software encode/decode**. None comes from WebView2 on Windows with a real GPU. So "measured" here usually means
"measured on a proxy", and a code reading of the exact Chromium/LiveKit path can outrank it.

**Short names used below:** teardown, capture (= webview2-capture), codecs, server-load, network, audio-sync, ux,
architecture, perf (= perf-reliability), wacky. Earlier-round reports keep their names.

**Primary sources I read myself** are listed in §2 with the URL. All were read on 2026-10-14. Downloaded copies are
in the session scratchpad, not in the repo.

---

## 0. Summary

- **Most contradictions are about two trade-offs the reports weigh differently:** streamer cost (pain 1) against
  viewer smoothness (pain 2). Examples: one video layer or two, `maintain-framerate` or `maintain-resolution`, 720p
  or 1080p.
- **Five claims in the deep reports are wrong** when checked against source:
  1. **perf's sample `livekit.yaml` would break voice and loss recovery.** It lists only opus, vp8, h264 and av1, so it
     drops `audio/red` and `video/rtx`. LiveKit registers RTX only when `video/rtx` is listed. That would also make the
     network report's `rtx-time` fix impossible.
  2. **codecs: "Munge the client offer to Main (4d001f)" does not work on its own.** LiveKit only registers
     `42001f`/`4d001f` when `enabled_codecs` lists that exact fmtp line. By default it registers only `42e01f` and High
     `640032`.
  3. **codecs: "a dynacast-paused low layer costs nothing until a weak viewer needs it" is wrong.** LiveKit's dynacast
     enables every layer up to the highest one anyone subscribes to. So the low layer encodes whenever anyone watches
     at full quality.
  4. **wacky: "send 720p and the streamer moves 2.25× fewer pixels" is wrong for the WebView2 path.** Chromium reads
     back and converts the **full captured frame** before it scales. Sending 720p saves encode work and upload, not
     readback or conversion.
  5. **perf: "we know the captured window from the picker via `ScreenCaptureStarting`" is wrong.** That event fires
     before the picker opens and carries no source information.
- **Smaller errors and corrections:**
  - server-load says `adaptiveStream` pauses video when Pulse is minimised. It can't: `document.visibilityState`
    never becomes hidden in Tauri (tauri #10592).
  - Chromium's per-app audio (`windowAudio: "window"`) is **Windows 11 only**. That hurts the teardown's "stream audio
    could live in WebView2" option for friends on Windows 10.
- **The biggest gaps:**
  - No Windows/WebView2/real-GPU number exists for anything.
  - Nobody has collected the friends' hardware and OS inventory, and many rulings depend on it.
  - Pain 3 ("PC slows down after 30 minutes") has candidate causes but no data from the friends' own machines.
  - No report solves how to find the shared window's HWND or PID when the WebView2 picker is used.

---

## 1. Contradictions and rulings

Each item gives what each side says, which evidence is stronger, the ruling, and the spike test that settles it.
Spike IDs (S-numbers) refer to the checklist in §4.

### C1. One video layer, or always a lower simulcast layer?

- **For a single layer:**
  - **codecs:** single layer by default; maybe an "on-demand" low layer that dynacast pauses until a weak viewer needs it.
  - **perf:** no simulcast for game streams, because each layer is another encoder session plus a CPU downscale.
  - **capture:** keep any layer at 360p or above.
- **For always having a lower layer:**
  - **network:** "always publish one lower simulcast layer at 30 fps". Measured in the lab: a viewer capped at
    3 Mbps got **no decodable frame for 45 s** without it, and a few seconds of chop then 360p with it.
  - **teardown:** steal idea #1 is "simulcast with a guaranteed low layer". One weak viewer dragging everyone down is
    Discord's #2 complaint.
  - **architecture:** one simulcast fallback layer. **ux:** assumes a 720p fallback plus a 1080p top layer.
  - **summary (earlier round):** one fallback layer at 15-30 fps.
- **Evidence:**
  - Source code confirms both halves of the problem:
    - A single-layer screen share is **unmanaged** by LiveKit's stream allocator (`IsManaged()` returns false for a
      screen share unless it has several layers). So a congested viewer is never moved down; they just lose packets.
    - Chromium encodes each simulcast layer in its own encoder (`supports_simulcast` is true only for VP9 with
      hardware k-SVC).
  - The network lab result is real but was measured with software OpenH264.
  - codecs' "on-demand low layer" is **wrong** (§2, V15): dynacast enables every quality up to the highest one
    subscribed (`Enabled: q <= quality`). So the low layer runs whenever anyone watches at full quality.
- **Ruling:**
  - Publish **one** low layer: about 640×360 (or 960×540) at 30 fps, roughly 0.6-1.2 Mbps, with even dimensions and
    360p or above so it stays on the hardware encoder.
  - **Accept that it encodes whenever anyone watches.** Pain 2 (stutter, including a 45 s blackout) outweighs one
    extra hardware session and a 360p CPU downscale.
  - Fall back to a single layer only if S5 shows the low layer costs the streamer a measurable amount of game FPS.
- **Spike (S4, S5):**
  - Throttle one viewer to 3 Mbps, with and without the low layer, and record time to a usable picture.
  - Record the streamer's game FPS and NVENC session count with 1 layer vs 2.
  - **Pass:** a capped viewer gets a picture within 5 s, and the low layer costs the streamer ≤1% of average game FPS.

### C2. Viewer: JS hidden participant (`#v`), or a Rust pass-through viewer (rust-sdks #1515)?

- **architecture:** recommends a "split" design.
  - Publish from WebView2.
  - View in the Rust voice connection with the unmerged pass-through decoder (#1515), then send encoded H.264 over a
    localhost WebSocket to WebCodecs.
  - Reasons: one connection per user, instant "Watch", and a viewer path shared with Linux.
- **Every other report** (summary, render, capture, codecs, ux, network, perf) assumes the `livekit-client` viewer
  inside WebView2.
- **Evidence:**
  - #1515 is open and has no human review (verified by the owner; on 2026-10-14 it shows 2 comments and 4 review
    comments).
  - architecture's own prototype found that the naive WebCodecs render loop builds up **0.5-3 s of lag**.
  - The C2 path also gives up Chromium's own WebRTC render pacing, which hands each frame to the compositor at its
    scheduled time (ux §6.2, cited Chromium source). Pulse's pain 2 is stutter, and frame pacing is exactly what we
    would then own.
  - The JS viewer is the mainstream path that Chromium tests.
- **Ruling:**
  - **v1 uses the JS viewer.**
  - Keep C2 as a later option (Linux unification, fewer connections), and revisit if #1515 merges upstream.
  - Hide "Watch" latency by connecting the JS participant on first Watch, then keeping it connected for the rest of
    the voice session.
- **Spike:**
  - S12 measures time-to-first-frame for a cold JS connect.
  - C2 gates G1-G4 (architecture §6) only run if time is left.
  - **C2 replaces the JS viewer only if** it is equal or better on `freezeCount` and inter-frame-delay deviation over
    10 minutes, not merely on connection count.

### C3. Stream audio: Rust process loopback, or WebView2 `getDisplayMedia` audio (`windowAudio` / `restrictOwnAudio`)?

- **teardown:** Chrome 141's `windowAudio: "window"` gives per-app audio in WebView2 with lip sync "for free"; Rust
  process loopback is option B.
- **audio-sync and architecture:** capture in Rust.
  - Chromium's `restrictOwnAudio` excludes only Chromium's own process tree, not `pulse-app.exe`, which plays every
    friend's voice.
  - Lip sync would not be free anyway, because LiveKit's sync group includes the publisher's participant ID.
- **Evidence (source, verified in §2):**
  - Chrome 141 did ship both options (V12).
  - **But** Chromium's application audio capture is gated on `IsWindowsProcessLoopbackCaptureSupported()`, which is
    `GetVersion() >= WIN11` (V13). So `windowAudio` can't work for friends on Windows 10 22H2.
  - Lip sync only happens when the audio and video share a participant **and** the viewer plays both in JS (V8). That
    breaks the echo reference, because WebView2's audio output isn't in our AEC reference.
  - Vesktop's `loopbackWithoutChrome` was lowered to build 19045 (teardown), but that is Electron's code, not the
    Chromium gate.
- **Ruling:**
  - **Rust process loopback.**
    - INCLUDE mode (the game's process tree) for window shares.
    - EXCLUDE mode (everything except Pulse) for screen shares.
  - Publish from the Rust `<uid>` participant, after fixing the mixer keying (a stream-audio track would otherwise
    kill the same user's mic task) and either patching stereo or accepting mono (V7).
  - `windowAudio` stays a curiosity: Windows 11 only, and its audio lands in JS.
- **Spike (S9):**
  - Process loopback on the oldest Windows 10 a friend runs.
  - Check whether WebView2 shows a "share window audio" option on Win11 and on Win10.
  - Measure A/V skew with the flash-and-beep test.
  - **Pass:** no friend voices audible in the stream; skew between 0 and +125 ms (audio late), per ITU-R BT.1359.

### C4. Does a minimised or covered Pulse throttle capture and encode?

- **capture and perf:** mostly no.
  - Capture runs on a browser-process thread with its own timer.
  - The encoder runs on WebRTC threads and in the GPU process.
  - A renderer with a live media stream isn't backgrounded.
  - Only JS timers are throttled.
- **teardown (Vesktop):** yes. Vesktop #191 ("fps collapses when the window isn't focused", 105 comments) led
  Vesktop to disable occlusion, backgrounding and timer throttling with flags.
- **ux:** Tauri never tells WebView2 the window is hidden, so `visibilityState` stays `"visible"` (tauri #10592,
  verified by the owner).
- **A Tauri maintainer** in tauri #5250: timers *are* throttled on Windows when the window is minimised, even though
  the webview is not hidden.
- **perf:** occlusion tracking is *good*, because the UI costs nothing while the game covers it.
- **capture:** recommends disabling occlusion tracking.
- **Evidence:** source code beats anecdote for the capture and encode threads. But the anecdote comes from a
  Chromium-based app, and Edge adds its own behaviour (efficiency mode, WebView2Feedback #5573). The mechanism that
  throttles timers in Tauri while `visibilityState` stays visible is unexplained.
- **Ruling:**
  - Expect capture and encode to keep running.
  - **Do not ship throttling flags by default.**
  - Keep JS out of the frame path, so the only throttling risk is the 1 Hz timer clamp.
  - Run the stats sampler in a worker if it drifts.
  - Add the flags only if S6 fails. Keep occlusion tracking on unless S6 proves it hurts, because it saves the
    streamer GPU work while the game covers Pulse.
- **Spike (S6):**
  - Minimised for 10 minutes, covered by a borderless game for 10 minutes, and idle-connected for 10 minutes.
  - Repeat with and without the flags. Log `visibilityState` and the Efficiency-mode leaf.
  - **Pass:** the viewer's fps never drops below 95% of the stream's fps; no LiveKit reconnect.

### C5. How much does AV1 save: 20-25% or 35-40%?

- **codecs:** ~20-25% fewer bits than H.264 for the same VMAF (CS:GO and Dota 2, VMAF-80 points at 6-8 Mbps).
- **wacky:** ~35-40% fewer bits (GTA V at 2 Mbps), "H.264 needs ~1.6× the bits".
- **NVIDIA marketing:** "up to 40%".
- **Evidence:** both measured on the same NVENC silicon, but at different operating points and on different clip
  segments. codecs used frames 1800-2039; wacky used the first 6 s. The gap narrows as bitrate rises: codecs shows
  10-18 VMAF points at 2.5 Mbps and 1-3 points at 12 Mbps.
- **Ruling:**
  - Both are right at their own bitrate.
  - Plan on **about 15-25%** at Pulse's 1080p60 game targets (6-8 Mbps). Use **35-40%** only for starved links
    (2-3 Mbps: LTE, 720p).
  - The v1 decision doesn't change either way: hardware AV1 encode is off by default in Windows Chromium (V6). It
    stays an opt-in flag experiment.
- **Spike (S14, optional):** RTX 40 sharer with `--enable-features=WebRtcAV1HWEncode`; compare H.264 and AV1 at 6
  and 3 Mbps on the same game, by eye and with `qpSum`.

### C6. Server CPU: ~30-45% or ~10-12% of one A1 OCPU?

- **summary / server-quality:** 2 streams × 6 viewers at 1080p60 is ~10-11k packets/s, so ~30-45% of one OCPU.
- **server-load:** a "Pulse-sized room" costs 4-5% of an E-core, which it equates to ~10-12% of an A1 OCPU. But that
  room was ~2k packets/s (720p-class load-test tracks at ~217 pps each).
  - For the real 1080p60 target (7k packets/s) the same report says **~25-30% of one A1 OCPU**.
- **Evidence:** both are x86 measurements scaled by an assumed 2-2.5× to Neoverse-N1. The headline numbers describe
  different loads.
- **Ruling:**
  - There is no real conflict. The 10-12% figure is a 2k-packets/s room.
  - Plan on **25-45% of one OCPU** for 2×6 at 1080p60.
  - Resizing to 2 OCPU and raising UDP buffers to 16 MiB stay "do before launch".
- **Spike (S11):** `lk load-test` plus one real 1080p60 share against a scratch A1 instance, with the `debug_handler`
  pprof endpoint.
  - **Pass:** under 50% of one OCPU at 2×6 1080p60, and zero `UdpRcvbufErrors` during joins.

### C7. How to fix the H.264 profile: client offer munging, or server `enabled_codecs`?

- **codecs:** munge the publisher's offer in JS to drop `42e01f`/`42001f` or put Main `4d001f` first. It claims "Main
  is in LiveKit's registered list".
- **perf:** add `42001f` (and optionally `4d001f`) fmtp lines to `room.enabled_codecs`. Measured on Linux: the
  publisher then negotiated `42001f`.
- **wacky:** "set codec preferences" on the client.
- **Evidence (source, §2 V14):**
  - LiveKit registers `42001f`/`4d001f` **only with strict fmtp matching**, so only if `enabled_codecs` lists that
    exact line. The defaults are `42e01f` and High `640032`. Client munging alone can't negotiate a profile the
    server didn't register.
  - Chromium skips NVIDIA only for `kProfileConstrainedBaseline`. `42001f` parses as plain Baseline, so the skip
    doesn't fire.
  - **But perf's sample yaml lists only opus, vp8, h264 and av1.** It drops `audio/red` (voice RED), `video/rtx`
    (LiveKit registers RTX only when `video/rtx` is listed), VP9, H.265 and PCMU/PCMA.
- **Ruling:**
  - **Fix it on the server.** Copy LiveKit's full default `enabled_codecs` list, then add the two strict H.264 lines
    (`4d001f` and `42001f`, packetization-mode=1).
  - Then optionally prefer Main on the client with `RTCRtpTransceiver.setCodecPreferences`. Main allows CABAC; codecs
    measured Baseline at 5-10 VMAF below High on NVENC.
  - Never ship the trimmed list.
- **Spike (S1):**
  - On an NVIDIA desktop with no iGPU: default config vs fixed config.
  - Record `codec.sdpFmtpLine`, `encoderImplementation` and `powerEfficientEncoder`.
  - Confirm the Rust voice clients still negotiate Opus+RED, and that RTX (`apt=` lines) is still present.
  - **Pass:** `MediaFoundationVideoEncodeAccelerator`, `powerEfficientEncoder: true`, not `42e01f`, voice RED
    intact.

### C8. Identity naming: `#v`, `#s`, or both?

- **render / summary:** viewer `<uid>#v` (hidden, subscribe-only), publisher `<uid>#s` (visible).
- **architecture:** `#s` publisher plus a Rust viewer.
- **capture:** two `Room` objects in the main page.
- **perf:** "`#v`/`#s`".
- **Evidence (source, §2 V16):** LiveKit never broadcasts a hidden participant's info to others. So a hidden
  participant can't usefully publish, and it is also anonymous in data packets. Two `Room` objects in one page also
  mean two signalling sockets and two sets of peer connections.
- **Ruling:**
  - Use **one visible JS participant per user**, with one suffix (say `<uid>#s`, "screen"). It publishes when the
    user goes live and subscribes when the user watches.
  - It connects on first use and leaves when the user leaves voice.
  - Rust filters one suffix everywhere (roster, speaking state, connection-quality events, volume).
  - The webhook can parse the suffix for "LIVE" badges and "who is watching".
  - Drop `hidden`. Being visible costs nothing among 6 friends, and the streamer's health data message (network §7.2)
    then carries a sender identity.
- **Spike (S12):** one JS participant publishing and subscribing at once; the Rust roster filter; the webhook
  ignoring it for voice state.

### C9. Preview thumbnails

- **ux:** 480×270 WebP (q 0.6) every 15 s, from `ImageCapture.grabFrame()`, `POST` to axum, then a gateway event
  carrying a revision number. Viewers pull it only while it's on screen. Measured median 13 KB for games.
- **server-load:** 320×180 JPEG every 15 s to axum.
- **teardown:** a JPEG every 30-60 s, like Discord's CDN preview.
- **Alternatives:** a LOW-layer "peek" (server-load).
- **Evidence:** ux measured sizes and encode time (headless Chrome).
  - Dynacast semantics (§2 V15) make a LOW-layer peek worse than it looks: it wakes the encoder and needs a keyframe
    request (PLI) per peek.
  - ux's start-dialog live preview conflicts with perf's "self-preview costs as much as a second encode, off by
    default".
- **Ruling:**
  - Use **ux's design.** Add `blob:` to `img-src` or use `data:` URLs.
  - Include a "Hide preview" option, and send the first snapshot after 3 s.
  - Show the live self-preview **only in the start dialog**. While live, the streamer's own card shows the 15 s
    snapshot.
- **Spike (S13):** snapshot cost during a real 1080p60 game share. **Pass:** no measurable game-FPS change; p90
  upload ≤32 KB.

### C10. `adaptiveStream`: on or off?

- **On:** ux (with `pixelDensity: 'screen'`) and server-load ("pauses when minimised").
- **Off:** network (set layers by hand), codecs (off for the full-screen viewer), and server-quality (the earlier
  round, but that was about the Rust SDK).
- **Evidence:**
  - `livekit-client` defaults `adaptiveStream` and `dynacast` to **false** (V3).
  - A LiveKit 1.13.7 server starts adaptive-stream subscribers on **LOW** until dimensions arrive (V19). That costs a
    layer switch and a keyframe on HIGH, which every viewer then sees as a short blur.
  - The background pause can't fire in Tauri (#10592).
- **Ruling:**
  - **Off for v1.** With two layers and 6 viewers, the size-based choice adds little.
  - Do it by hand instead:
    - `setVideoQuality(HIGH)` on Watch;
    - a "Data saver" option that picks LOW;
    - Pulse's own minimise controller, which calls `setEnabled(false)` after 5 s minimised.
  - Turn `dynacast` **on**.
- **Spike (S12):** minimise controller (inbound bitrate near 0 within 6 s of minimise); and the first-watch blur on
  other viewers with adaptive stream on vs off.

### C11. Degradation for games: `maintain-framerate` + `motion`, or keep `maintain-resolution`?

- **For `maintain-framerate` with `contentHint='motion'`:** codecs, perf and capture. `maintain-resolution` turns
  pressure into dropped frames, which is the "choppy" complaint.
- **network:** keep `maintain-resolution`; `balanced` was bad (on OpenH264 it started at 480×270 / 12 fps and never
  recovered).
- **perf (measured):** `maintain-framerate` held 60 fps on noise content but cut the picture to 320×180-480×270.
- **Evidence:** network never tested `maintain-framerate`, only `balanced`, and none of the three ran on a hardware
  encoder. Pain 2 is stutter, which argues for keeping fps. perf shows the failure mode is a resolution collapse.
- **Ruling:**
  - **`maintain-framerate` + `motion` for game presets; `maintain-resolution` + `detail` for text.**
  - Guard against collapse with the pre-flight upload cap (≤70% of measured upload, network §8.1), so bandwidth
    pressure is rare.
- **Spike (S4):** NVENC/MF on a real game, with a 4 Mbps cap and with 2% loss, for each preference. Record
  `frameWidth`, fps and freezes. **Pass:** the game preset holds ≥50 fps and stays ≥540p for ≥90% of the time.

### C12. Default game preset: 720p60 at 4.5 Mbps, or 1080p?

- **summary / ux / perf:** Smooth 720p60, 4.5 Mbps.
- **codecs:** at 4.5 Mbps, 1080p60 beat 720p60 on 3 of 4 clips (CS:GO was the exception). It suggests 720p60 at
  6 Mbps or 1080p60 at 8 Mbps.
- **wacky:** the opposite on the same clip names. 720p plus lanczos/FSR1 beats 1080p on GTA V, Minecraft and Rust;
  native 1080p wins on CS:GO and Hearthstone.
- **Evidence:**
  - Both measured, but different 4-6 s segments of the Xiph clips, different VBV sizes (1 s vs 0.5 s), and bicubic vs
    lanczos/FSR1 upscaling. Absolute scores differ wildly (GTA V 1080p at 4.5 Mbps: VMAF 95.7 in codecs, 74.0 in
    wacky). The only safe conclusion is "it depends on the content".
  - wacky's "720p cuts the streamer's copies 2.25×" is wrong for Chromium (V20).
- **Ruling:**
  - Default **720p60 with a 5-6 Mbps cap** (codecs: "4.5 is okay, not great"), capped at 70% of measured upload.
  - Offer 1080p60 at 8 Mbps as "Best".
  - Text: native resolution, 5-15 fps.
  - The streamer-cost argument for 720p is weaker than the reports say: in the WebView2 path it saves encode work and
    upload, not capture readback.
- **Spike (S2, S15):**
  - In S2, measure game FPS at 720p60 vs 1080p60 from the same 1440p window, to confirm readback is the same.
  - In S15, check which scaler WebView2 uses to upscale a 720p `<video>`.

### C13. Smaller contradictions

| # | Topic | Sides | Ruling |
|---|---|---|---|
| a | `backupCodec` | network: off. codecs: keep `h264` as a safety net, never VP8. `livekit-client` default: `backupCodec: true`, which means VP8 (V3) | With H.264 primary, set `backupCodec: false`. Use `{codec:'h264'}` only if AV1 is ever primary |
| b | Low-latency rendering (`playout_delay` min 0) | ux and teardown: try it. server-quality: don't set `playout_delay` | Low priority. "Render immediately" trades smoothness for latency, the opposite of what the friends want |
| c | `jitterBufferTarget` | perf: default ~100 ms for games. network: optional "smooth mode", small effect. ux: only a hint. audio-sync: use it as the sync actuator | Not a default. Keep it for the sync controller and an opt-in smooth mode |
| d | Discord's default bitrate | codecs: 2.5 / 4 Mbps (a third-party source from 2023). teardown: target 600 kbps, max 3.5 Mbps (Discord's 2026 bundle) | teardown's reading is newer and closer to primary |
| e | NVENC session limit | 8 (SDK note) vs 12 (support matrix) | Doesn't matter: Pulse uses 1-2 sessions |
| f | Window capture backend | perf: "always WGC". capture: WGC, with GDI fallback | capture is more precise; GDI fallback means black frames for D3D games |
| g | Disconnect-on-stop as the leak fix (perf) | perf measured ~2 MB per cycle with disconnect; `livekit-client` #2008 (open) says `Room` objects stay in the heap after `disconnect()` | Reuse one `Room` and one publication with `replaceTrack`; disconnect only on leaving voice. Soak test S3 |
| h | Earlier round: "same `stream` id gives A/V sync" | audio-sync and architecture: wrong | Wrong; confirmed in source (V8) |
| i | Earlier round: "Chromium captures with WGC" | capture and perf: monitor capture uses DXGI before Windows 11 24H2 | Corrected (V5) |
| j | Earlier round (capture-encode): "the SDK munges `stereo=1`, so check the receiver gets 2 channels" | audio-sync: publishing can't be stereo without `TfStereo` | Receive side is fine; publish side is mono until patched (V7) |

---

## 2. Verification of load-bearing claims

All read on 2026-10-14. The owner had already verified: the NVIDIA Constrained-Baseline skip (crbug 1088650), the
`livekit-client` screen-share defaults and presets, rust-sdks #1515's status, tauri #10592, the LiveKit v1.13.10 TURN
fix, Pulse's 560 px minimum height and `webview2-com-sys` 0.39.1, and the identity keying of voice receive tasks. I
did not redo those.

| # | Claim (report) | Verdict | Evidence |
|---|---|---|---|
| V1 | One unrepaired loss freezes the picture for ~3 s; `rtx-time=300` cuts that to ~0.9 s (network) | **CONFIRMED** | libwebrtc `video/video_receive_stream2.h`: `kMaxWaitForFrame = 3 s`, `kMaxWaitForKeyFrame = 200 ms`. `.cc` `DetermineMaxWaitForFrame`: if 3 × history < 3 s, wait 3 × history. `media/engine/webrtc_video_engine.cc`: NACK history defaults to 1000 ms (so 3 × 1000 is not < 3 s, and the wait stays 3 s); `rtx-time` from the negotiated codec replaces it, capped at 1000. With 300 ms the wait is 900 ms. Side effect: the NACK history also drops to 300 ms. https://webrtc.googlesource.com/src/+/refs/heads/main/video/video_receive_stream2.cc , https://webrtc.googlesource.com/src/+/refs/heads/main/media/engine/webrtc_video_engine.cc |
| V2 | `livekit-client` ignores `videoEncoding` for screen share; the default is `h1080fps15` (network, codecs, capture) | **CONFIRMED** | v2.22.4 `publishUtils.ts` `computeVideoEncodings`: `if (isScreenShare) videoEncoding = options?.screenShareEncoding`. `defaults.ts`: `screenShareEncoding: ScreenSharePresets.h1080fps15.encoding`. Also: `simulcast: true` by default, and the screen-share low layer is half resolution at the same fps, bitrate max(150 kbps, cap/4). https://github.com/livekit/client-sdk-js/blob/v2.22.4/src/room/participant/publishUtils.ts |
| V3 | JS SDK `dynacast` and `adaptiveStream` default to off (server-load, codecs) | **CONFIRMED** | `defaults.ts`: `adaptiveStream: false`, `dynacast: false`. Also `backupCodec: true`, `videoCodec: 'vp8'`, `autoSubscribe: true`, `singlePeerConnection: true`. https://github.com/livekit/client-sdk-js/blob/v2.22.4/src/room/defaults.ts |
| V4 | The WebView2 picker is unusable under 600 px tall since runtime 134 (capture) | **CONFIRMED** | WebView2Feedback #5173 (opened 2025-03-19, open): "unusably small if the parent window has a height less than 600px", Win10 and 11, also reproduces in Edge. https://github.com/MicrosoftEdge/WebView2Feedback/issues/5173 |
| V5 | Chromium uses WGC for monitor capture only on Windows 11 24H2+ (capture, perf) | **CONFIRMED** | `content/browser/media/capture/desktop_capture_device.cc`: `IsWgcEnabledForScreenCapture()` returns `GetVersion() >= WIN11_24H2`; window capture always enables WGC. https://github.com/chromium/chromium/blob/main/content/browser/media/capture/desktop_capture_device.cc |
| V6 | `kWebRtcAV1HWEncode` is off on Windows (codecs) | **CONFIRMED** | `media/webrtc/webrtc_features.cc`: `FEATURE_DISABLED_BY_DEFAULT` under `IS_WIN`, enabled elsewhere. `kWebRtcAllowWgcUsingTexture` is also disabled by default. https://github.com/chromium/chromium/blob/main/media/webrtc/webrtc_features.cc |
| V7 | The Rust SDK never publishes stereo (rust-sdks #1018) (audio-sync) | **CONFIRMED** | #1018 is open (2026-04-15, no comments). rust-sdks `main` `local_participant.rs` only pushes `TfPreconnectBuffer` to `audio_features`. LiveKit v1.13.7 `participant_sdp.go` removes `;stereo=1` unless the track has `TF_STEREO`. https://github.com/livekit/rust-sdks/issues/1018 , https://github.com/livekit/livekit/blob/v1.13.7/pkg/rtc/participant_sdp.go |
| V8 | LiveKit's sync group includes the publisher ID, so two participants can't be lip-synced (audio-sync) | **CONFIRMED** | v1.13.7 `subscribedtrack.go` L126: `PackSyncStreamID(params.MediaTrack.PublisherID(), …)`; `utils.go`: `participantID + separator + stream`. It also needs `room.sync_streams`. https://github.com/livekit/livekit/blob/v1.13.7/pkg/rtc/utils.go |
| V9 | WebView2 152 falls back to software decode for WebRTC when the colour-space extension is negotiated (#5755) (capture) | **CONFIRMED (one report)** | WebView2Feedback #5755, opened 2026-10-11, open, no Microsoft response yet. It reports `libdav1d (fallback from: ExternalDecoder (D3D11VideoDecoder))` and blames the Edge-only hybrid wrapper on re-`Initialize()`; workaround `--disable-features=msD3D11VdaHybridVideoDecoding`. Which runtimes are affected beyond M152 is unknown. https://github.com/MicrosoftEdge/WebView2Feedback/issues/5755 |
| V10 | `livekit-client` gives up reconnecting after ~50 s (perf) | **CONFIRMED** | `DefaultReconnectPolicy.ts`: 10 delays (0, 0.3, 1.2, 2.7, 4.8 s, then 7 s × 5) plus up to 1 s of jitter from the third try: ~44-52 s, then `null` (stop). https://github.com/livekit/client-sdk-js/blob/v2.22.4/src/room/DefaultReconnectPolicy.ts |
| V11 | LiveKit forces `contentHint='motion'` and L1T3 for VP9/AV1 screen share (codecs) | **CONFIRMED, with a scope note** | `LocalParticipant.ts` L1155-1172: only when `isSVCCodec && !isSVCSimulcast`. The rid-simulcast path for VP9/AV1 (2.22.3, "Support simulcast for svc codecs") skips it. Code comment: "Chrome does not allow more than 5 fps with L1T3 … has encoding bugs with L3T3". https://github.com/livekit/client-sdk-js/blob/v2.22.4/src/room/participant/LocalParticipant.ts |
| V12 | Chrome 141 shipped `windowAudio` and `restrictOwnAudio` (teardown, audio-sync) | **CONFIRMED** | Chrome 141 release notes list both. `windowAudio` is described as a hint ("exclude", "system", "window"). https://developer.chrome.com/release-notes/141 |
| V13 | Chromium per-app audio for window capture is Windows 11 only (architecture) | **CONFIRMED** | `media/base/media_switches.cc`: `IsApplicationLoopbackCaptureSupported()` = `kApplicationAudioCaptureWin` (enabled) && `IsWindowsProcessLoopbackCaptureSupported()`, which is `GetVersion() >= WIN11`. This weakens the teardown's option A for Windows 10 friends. https://github.com/chromium/chromium/blob/main/media/base/media_switches.cc |
| V14 | H.264 profile: LiveKit answers `42e01f` by default; registering `42001f` avoids the NVIDIA skip (perf); "Main is in LiveKit's registered list" (codecs); High is "filtered out on the publisher's connection" (perf) | **Partly WRONG** | `livekit/protocol` (pinned `a4f4b5c0c23f`) `codecs.go`: default video list has `42e01f` (packetization-mode 0 and 1) and High `640032` only. v1.13.7 `mediaengine.go`: `42001f`/`4d001f` are registered only with **strict** fmtp, so only if listed in `enabled_codecs`. **codecs is wrong:** Main is not registered by default, so a client munge alone fails. **perf is wrong on one point:** `newPeerConnection` filters High when the server is the *offerer*, and `configureSenderCodecs` filters it on the server's *send* side; publish-side receive codecs are not High-filtered. Chromium `rtc_video_encoder_factory.cc` `IsConstrainedH264()` is true only for `kProfileConstrainedBaseline`, so `42001f` (Baseline) avoids the skip. perf's fix direction is **CONFIRMED**. https://github.com/livekit/livekit/blob/v1.13.7/pkg/rtc/mediaengine.go , https://github.com/livekit/livekit/blob/v1.13.7/pkg/rtc/transport.go , https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/platform/peerconnection/rtc_video_encoder_factory.cc |
| V14b | perf's sample `enabled_codecs` yaml is safe | **WRONG** | v1.13.7 `config.go` defaults: PCMU, PCMA, opus, **red**, vp8, h264, vp9, av1, h265, **rtx**. `mediaengine.go`: `rtxEnabled := IsCodecEnabled(codecs, VideoRTXCodecParameters…)`. perf's list (opus, vp8, h264 ×3, av1) disables Opus RED for voice and video RTX, and so also network's `rtx-time` fix. **Correction:** start from the full default list and add the two strict H.264 entries. https://github.com/livekit/livekit/blob/v1.13.7/pkg/config/config.go |
| V15 | Dynacast pauses a low layer until a weak viewer needs it (codecs §4.3) | **WRONG** | v1.13.7 `pkg/rtc/dynacast/dynacastmanagervideo.go` L250-253: for each quality LOW..HIGH, `Enabled: q <= quality`, where `quality` is the highest subscribed. Any viewer on HIGH keeps LOW encoding. Only "nobody watching" turns everything off (perf measured that). https://github.com/livekit/livekit/blob/v1.13.7/pkg/rtc/dynacast/dynacastmanagervideo.go |
| V16 | A single-layer screen share is exempt from LiveKit's stream allocator (codecs) | **CONFIRMED** | `streamallocator/track.go`: `IsManaged()` = `source != SCREEN_SHARE \|\| isMultiLayered`. https://github.com/livekit/livekit/blob/v1.13.7/pkg/sfu/streamallocator/track.go |
| V16b | (Supports C8.) Hidden participants are invisible to others | **CONFIRMED** | v1.13.7 `room.go`: hidden participants are left out of participant broadcasts; their updates go "only to the hidden participant itself". `participant.go`: their data packets carry an empty identity. https://github.com/livekit/livekit/blob/v1.13.7/pkg/rtc/room.go |
| V17 | Unpublish/republish leaks ~20 MB per cycle in the renderer (perf) | **UNVERIFIABLE** | A Linux headless measurement only; no upstream issue matches. Related and open: `livekit-client` #2008, "Room instances remain in the heap after disconnect()" (2026-07-13). Re-measure on WebView2 (S3). https://github.com/livekit/client-sdk-js/issues/2008 |
| V18 | Pin `livekit-client` ≥ 2.22.3 because Stoat fixed publish-triggered call drops (teardown) | **CONFIRMED, with nuance** | Stoat for-web #1679 is a **PR**, merged 2026-09-30, bumping 2.20.2 → ~2.22.3. The actual fix is client-sdk-js #2030 ("recover broken publish paths and stuck reconnects"), merged 2026-08-12, so earlier versions may have it. Pin to an exact version anyway. https://github.com/stoatchat/for-web/pull/1679 , https://github.com/livekit/client-sdk-js/pull/2030 |
| V19 | Adaptive-stream subscribers start at LOW on LiveKit OSS (server-load) | **CONFIRMED** | v1.13.7 `subscribedtrack.go` L212-224: `else if AdaptiveStream → Quality LOW` unless `EnableStartAtDesiredQuality`. https://github.com/livekit/livekit/blob/v1.13.7/pkg/rtc/subscribedtrack.go |
| V20 | Chromium converts and scales the full captured frame on the CPU, and screen share isn't zero-copy (perf); "720p cuts copies 2.25×" (wacky) | **perf CONFIRMED; wacky WRONG for the WebView2 path** | `desktop_capture_device.cc` L974 `ARGBToI420` at `frame->size()`, then L1038 `I420Scale` to the output size. `rtc_video_encoder.cc` `IsZeroCopyEnabled()` returns false for `VideoContentType::SCREENSHARE`. Lower fps does cut the work (the capture timer follows the frame rate); a lower *send* size does not cut readback or conversion. https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/platform/peerconnection/rtc_video_encoder.cc |
| V21 | `ScreenCaptureStarting` tells the host which window was picked (perf, failure table) | **WRONG** | Its event args expose only `Cancel`, `Handled`, `OriginalSourceFrameInfo` (the requesting frame) and `GetDeferral()`, and it fires before the picker UI. capture §1.2 says the same. https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2screencapturestartingeventargs |
| V22 | `pauseVideoInBackground` pauses video when Pulse is minimised (server-load §3) | **WRONG in Pulse** | It is the default in `RemoteVideoTrack.updateVisibility`, but it keys off page visibility, which never becomes hidden in Tauri on Windows (tauri #10592, verified by the owner). ux states this correctly. |
| V23 | Timers throttle on Windows when a Tauri window is minimised (capture) | **CONFIRMED (maintainer statement)** | tauri #5250: "On Windows, we don't hide the webview when the window is minimized … and yet the timers are being throttled." https://github.com/tauri-apps/tauri/issues/5250 |

**Not re-checked because the owner verified them:** the MF NVIDIA Constrained-Baseline skip; the screen-share
`degradationPreference` default and `ScreenSharePresets`; #1515's status; tauri #10592; the LiveKit 1.13.10 TURN fix;
the 560 px minimum height; `webview2-com-sys` 0.39.1; identity-keyed voice receive.

---

## 3. Gaps: important questions no report answered

1. **No Windows numbers at all.** Every perf, codec, network and latency figure comes from Linux, mostly headless
   Chrome with software codecs or FFmpeg NVENC. The architecture rests on WebView2 + Media Foundation + WGC
   behaviour that nobody has run.
2. **The friends' inventory, which several rulings depend on.** For each friend:
   - Windows build (10 22H2 / 11 23H2 / 11 24H2+): this decides DXGI vs WGC for monitor capture, whether
     `windowAudio` is possible, and the yellow border;
   - GPU vendor and generation, and whether the iGPU is enabled: the NVIDIA skip, AV1, and AMD's rate control (codecs:
     Chromium turns software rate control off on AMD);
   - HDR on or off;
   - monitor refresh rate;
   - wired or Wi-Fi, and a measured uplink;
   - games and their anti-cheats;
   - audio software (Sonar, Nahimic, Razer 7.1: perf's audiodg leak candidate).
3. **Pain 3 has no data from the friends' machines.** perf lists nine candidate causes and a `typeperf` triage. But
   nobody has run it during a real Discord stream, so we don't know whether Pulse will inherit the problem (for
   example a non-paged-pool driver leak) or avoid it. **Run the triage on a friend's PC before the spike.** It's free,
   and it says whether the soak test is checking the right counters.
4. **Mapping the shared window to an HWND/PID with the WebView2 picker.**
   - Process-loopback INCLUDE mode, the `IsIconic` "game minimised" detector and content auto-detection all need the
     HWND.
   - Neither the picker nor `ScreenCaptureStarting` provides it (V21).
   - Candidates: match the track label against `EnumWindows` titles, or `CaptureController` / `getSettings()`
     fields. Untested. Fallback: EXCLUDE mode and no minimise detector.
5. **Does WGC capture change the game's present mode?** perf, capture and wacky all speculate that capture may push
   an independent-flip game into composed flip (+1 frame latency, less FPS). It could be a large part of pain 1.
   Nobody measured it (PresentMon's `PresentMode` column).
6. **Viewers who are also gaming.** Every report costs the *streamer*. A viewer playing a game while watching shares
   the GPU's decode engine and compositor with their game. Unmeasured.
7. **Does WebView2 ship OpenH264?** Chromium offers hardware H.264 only if the OpenH264 fallback is built in
   (`UseH264AcceleratedEncoderForWebRTC()`). If Edge's build lacks it, H.264 might not be offered at all. perf flags
   it; nobody resolved it.
8. **How "Go Live" starts from inside a game.** Discord has a "Stream <game>" button and an overlay. Pulse needs an
   alt-tab to Pulse plus the picker every time (`getDisplayMedia` always shows it). ux designs the dialog but not
   this flow, or how the WebView2 "you are sharing" bar behaves over a full-screen game.
9. **Edge-specific divergence.** #5755 shows Edge patches can break WebRTC decode. Nobody listed which WebView2
   runtime versions the friends get, or planned a check that re-runs on each Evergreen update (perf's tier-2 CI is the
   closest).
10. **Changing `enabled_codecs` affects voice too.** The list is room-wide. Nobody checked how the Rust voice client
    negotiates after the change (V14b shows one way it can go wrong).
11. **A thumbnail-on-the-server privacy rule.** ux and server-load keep snapshots in memory. Nobody specified
    retention after the stream ends or a crash, or who may fetch them (server members not in the channel?).

---

## 4. Consolidated spike checklist

Windows only unless noted. Use one NVIDIA desktop **with the iGPU disabled or absent**, one Intel laptop, and an AMD
PC if a friend has one. Cover one Windows 10 22H2 machine and one Windows 11 machine (ideally 23H2 *and* 24H2). Record
the WebView2 runtime version (`navigator.userAgent`) and the GPU driver version for every run.

**Step 0 (before the spike, no code):** collect the friend inventory (gap 2), and run perf's `typeperf` triage on one
friend's PC during a long Discord stream (gap 3).

The order is by how much each test could change the design.

| # | Test | Pass bar | If it fails |
|---|---|---|---|
| **S1** | **H.264 hardware encode.** On the NVIDIA desktop, publish `getDisplayMedia` H.264 with (a) the default server config and (b) the full default `enabled_codecs` plus strict `4d001f` and `42001f` lines. Log `codec.sdpFmtpLine`, `encoderImplementation` and `powerEfficientEncoder`, and watch Task Manager "Video Encode". Repeat on the Intel laptop and on AMD. Also confirm Rust voice still has Opus RED and that `apt=` RTX lines exist. Check (gap 7) with `--disable-features=WebRtcHWEncoding` that an OpenH264 fallback exists | MF encoder named; `powerEfficientEncoder: true`; fmtp is not `42e01f`; voice unchanged | No hardware H.264 in WebView2 on NVIDIA means the native capture/encode path (Phase B) moves into v1 |
| **S2** | **Streamer game-FPS cost.** PresentMon/CapFrameX, 3 × 60 s per configuration: off / 720p60 / 1080p60 (from the same 1440p window) / 720p30. Share the game window, then share a browser while gaming. Pulse UI animations frozen vs live. Record `PresentMode` (gap 5) | ≤5% average FPS and ≤5% worse 1% lows at 720p60 on a GPU-bound game, with hardware encode confirmed | Larger loss promotes the zero-copy native pipeline over all other v2 work (wacky §3.2.1) |
| **S3** | **2-hour soak (pain 3).** 1080p60 game share, 2+ viewers. perf's `share: host` counters (non-paged pool, dwm, audiodg handles, GPU dedicated/shared, private bytes, `enc_ms`). Plus 20 start/stop cycles with `replaceTrack` and with disconnect/reconnect | perf §4.6 criteria: private bytes < 50 MB/h after 10 min; pool/dwm/audiodg/GPU memory flat; `enc_ms` within 20%; game 1% lows within 5% between the first and last 5 min | Find the counter that climbs before choosing a fix; for a leak per cycle, keep one long-lived publication |
| **S4** | **Viewer stutter under impairment** (Linux-router `tc` or clumsy, network §9.2 profiles). With and without (a) the low simulcast layer, (b) `rtx-time=300` in the viewer's answer, (c) `maintain-framerate`+`motion` vs `maintain-resolution`, on NVENC/MF | No freeze ≥2.5 s at 1-5% loss; a 3 Mbps-capped viewer gets a picture within 5 s; the game preset holds ≥50 fps and ≥540p for ≥90% of the time | Pick the settings that pass; if none does, revisit layering (C1) |
| **S5** | **Cost of the low layer** to the streamer: NVENC sessions (`nvidia-smi`), streamer CPU and game FPS, 1 layer vs 2 | ≤1% game FPS, ≤5% of one core | Single layer by default, low layer opt-in |
| **S6** | **Minimised and covered Pulse.** Pulse minimised for 10 min while sharing; covered by a borderless game for 10 min; idle-connected for 10 min. With and without the throttling flags (capture §7). Log `visibilityState` and the Efficiency-mode leaf | Viewer fps ≥ 95% of the stream's fps; no reconnect | Ship the flags (merged into one `--disable-features` with wry's defaults) |
| **S7** | **Viewer hardware decode.** Use a `getUserMedia` call to unlock the stats. `decoderImplementation` with and without `--disable-features=msD3D11VdaHybridVideoDecoding`; viewer CPU at 1080p60 | D3D11 decoder, `powerEfficientDecoder: true`, viewer CPU < 5% of 6 cores | Ship the #5755 workaround flag; track the issue |
| **S8** | **Picker.** Window heights of 560 and 640 px; are all monitors listed; is the game window listed; where the "sharing" bar sits over a full-screen game; `ScreenCaptureStarting` cancel and deferral; does the track label carry the window title (gap 4) | Usable at the new minimum height; no focus steal; label matches a window | Raise the minimum height; for gap 4, fall back to EXCLUDE audio |
| **S9** | **Stream audio.** Process loopback INCLUDE and EXCLUDE on Win10 22H2 and Win11; no friend voices in the stream; game restart with a new PID; WebView2 `windowAudio` on Win11 vs Win10; stereo with the SDK patch (left-only tone); A/V skew with the flash-and-beep test on a clean link and with 2% loss | No echo; skew within 0 to +125 ms (audio late); L≠R with the patch | Skew outside the bar: build the sync controller (audio-sync §4.6). No stereo: ship mono |
| **S10** | **Anti-cheat.** Valorant (Vanguard), one EAC title and FACEIT if used: window and monitor capture, 10 min each. Plus an exclusive-fullscreen title | Picture OK, no kick or warning | Document the affected games; show the borderless hint |
| **S11** | **Server (scratch A1, not prod).** `lk load-test` plus one real 1080p60 share, 2×6. `debug_handler` pprof, `process_cpu_seconds_total`, `UdpRcvbufErrors` during joins, before and after the 16 MiB sysctl; 1 vs 2 OCPU. Run on v1.13.10 | < 50% of one OCPU; 0 UDP receive-buffer drops with 16 MiB | Resize to 2 OCPU before launch (already planned) |
| **S12** | **Lifecycle and the JS participant.** One visible JS participant publishing and subscribing (C8); Rust roster and webhook filtering; cold time-to-first-frame on Watch; kill the WebView2 renderer while live (voice survives; viewers told within ~1 s via a Rust attribute); a 60 s outage (Pulse's outer retry); duplicate identity; minimise controller (`setEnabled(false)`); first-watch blur with adaptive stream on vs off | Watch-to-frame ≤ 1.5 s cold; no orphaned participant; voice untouched | Pre-connect the JS participant while in voice |
| **S13** | **Preview snapshots** during a real 1080p60 game share (`grabFrame` → `OffscreenCanvas` → WebP, in a worker) | No measurable game-FPS change; p90 ≤ 32 KB | Lower the cadence or size |
| **S14** | *(optional)* **AV1 hardware** on an RTX 40: `--enable-features=WebRtcAV1HWEncode`; `encoderImplementation`; quality vs H.264 at 6 and 3 Mbps; does an RTX 30 viewer decode it in hardware | Hardware encode; visibly better at 3 Mbps | AV1 stays off |
| **S15** | *(optional)* **Viewer upscaling.** A 720p stream full-screen on 1440p: which scaler WebView2 uses, whether RTX VSR activates in `msedgewebview2.exe`, and whether HTML controls over the `<video>` block the overlay | Driver-grade scaler or VSR active | WebGL lanczos/FSR1 canvas (wacky §3.1.1) |
| **S16** | *(optional)* **C2 pass-through viewer**, architecture gates G1-G4, only if S1-S12 leave time | Equal or better `freezeCount` and inter-frame-delay deviation than the JS viewer over 10 min | Stay on the JS viewer |
| **S17** | *(Linux, dev only)* Rust decode → WebSocket → WebGL2 in WebKitGTK at 1080p30 | Smooth 30 fps | Accept a lower bar |

---

## Sources (all read 2026-10-14)

**Source code:**
- livekit-client v2.22.4: `src/room/defaults.ts`, `src/room/participant/publishUtils.ts`,
  `src/room/participant/LocalParticipant.ts`, `src/room/DefaultReconnectPolicy.ts`,
  `src/room/track/RemoteVideoTrack.ts`: https://github.com/livekit/client-sdk-js/tree/v2.22.4
- LiveKit server v1.13.7: `pkg/rtc/{transport.go, mediaengine.go, subscribedtrack.go, utils.go, participant.go,
  participant_sdp.go, room.go}`, `pkg/rtc/dynacast/dynacastmanagervideo.go`, `pkg/sfu/streamallocator/track.go`,
  `pkg/config/config.go`, `go.mod`: https://github.com/livekit/livekit/tree/v1.13.7
- livekit/protocol `codecs/codecs.go` at the version v1.13.7 pins:
  https://github.com/livekit/protocol/blob/a4f4b5c0c23f/codecs/codecs.go
- rust-sdks `main` `livekit/src/room/participant/local_participant.rs`: https://github.com/livekit/rust-sdks
- Chromium `main` (GitHub mirror):
  - `content/browser/media/capture/desktop_capture_device.cc`
  - `content/public/browser/desktop_capture.cc`
  - `media/webrtc/webrtc_features.cc`
  - `media/base/media_switches.cc`
  - `third_party/blink/renderer/platform/peerconnection/{rtc_video_encoder_factory.cc, rtc_video_encoder.cc}`

  https://github.com/chromium/chromium
- libwebrtc `main`: `video/video_receive_stream2.{h,cc}`, `media/engine/webrtc_video_engine.cc`:
  https://webrtc.googlesource.com/src/

**Issues, PRs and docs:**
- WebView2Feedback #5173, #5755, #2184: https://github.com/MicrosoftEdge/WebView2Feedback/issues
- `ICoreWebView2ScreenCaptureStartingEventArgs`:
  https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2screencapturestartingeventargs
- Chrome 141 release notes: https://developer.chrome.com/release-notes/141
- rust-sdks #1018 and #1515: https://github.com/livekit/rust-sdks
- client-sdk-js #2008, #2030, and the v2.22.2-v2.22.4 release notes: https://github.com/livekit/client-sdk-js
- Stoat for-web PR #1679: https://github.com/stoatchat/for-web/pull/1679
- tauri #5250: https://github.com/tauri-apps/tauri/issues/5250
- LiveKit releases (v1.13.8-v1.13.10): https://github.com/livekit/livekit/releases

**Reports cross-checked:** the ten `screenshare-deep-*.md` files and `screenshare-{summary, capture-encode, render,
server-quality}.md` in this folder.
