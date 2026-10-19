# Discord Go Live: forensic teardown, failure atlas and competitor ideas

Date: 2026-10-14. All sources were read on 2026-10-14 unless noted. This report builds on
`screenshare-summary.md` and the three `screenshare-*.md` reports next to it, and does not repeat them.

Labels:

- **[cited]** a source says it.
- **[measured]** I counted or read it myself: a forum dataset, Discord's shipped client code, Vesktop's code.
- **[est.]** my estimate from evidence.
- **[speculation]** a plausible guess that is not verified.

---

## 0. TL;DR

1. **Discord's Go Live is architecturally what Pulse already plans.**
   - Each stream is a **separate RTC connection**, with its own SSRC namespace, its own MLS group and possibly its
     own server/region. The stream audio rides on that connection as Opus, flagged `SOUNDSHARE`.
   - The streamer only sends when someone watches.
   - It is one encoding at "quality 100". The viewer tells the SFU its rendered pixel count, and the SFU tells the
     sender what to produce.
   - Pulse's "hidden second participant `<uid>#v`" plus "Watch" opt-in is the same shape. [cited: userdoccers voice
     docs; measured: Discord web bundle]
2. **Discord still does not use simulcast for Go Live by default.** It has been A/B testing it since 2024
   (`2024-06_rtc_pacer__simulcast`, a low layer of 720p at 500k/750k/1000k, plus a worker pacer and BWE probing).
   - The cost of not doing it shows up in user posts. One example: "Stream tanks for all viewers if a mobile user
     joins".
   - With 6 friends and LiveKit simulcast, Pulse can beat this on day one. [measured]
3. **Discord's default Go Live bitrate is low.**
   - Web/desktop defaults are `desktopBitrate {min 500 kbps, max 3.5 Mbps, target 600 kbps}`, with capture and
     encode at 1280×720.
   - The 2026 AV1 experiment caps AV1 at 3.5 Mbps (variants 3.0/2.5).
   - "Discord quality drops to potato" is partly a policy choice, not physics. [measured]
4. **Discord is mid-migration on capture and encode.**
   - Experiments exist for:
     - Media Foundation GPU and CPU encode (`2025-12-wmf-gpu-encode`, `-intel`, `wmf-cpu-encode-intel`);
     - WGC in the picker;
     - WGC dirty regions;
     - DX12 video hook;
     - HDR capture (default `never`);
     - NVENC reconstructed frames;
     - "auto" quality.
   - A support article (Jan 2026) tells Windows users to install the Microsoft Store AVC/AV1 codec extensions,
     because "Discord relies on codecs provided by the operating system". [measured, cited]
5. **The top Go Live pain is "it doesn't start / it stops".**
   - In 47,720 r/discordapp posts (Oct 2025 to Oct 2026), 1,933 are about streaming (4.1%).
   - Of those, **~49% are start, stop or crash failures**, ~20% lag/low fps, ~14% audio missing or bad, ~10%
     CPU/GPU/encoder, ~9% blur, ~6% black screen or flicker, ~3.5% game FPS hit, ~2.3% echo, ~1.6% HDR, and ~1%
     game detection or anti-cheat.
   - Browser/web-client posts are the biggest platform cluster (166 mentions). [measured]
6. **The biggest surprise for our architecture: Chromium can now do per-app audio on Windows.**
   - Chrome 141 shipped `windowAudio: "window"` and `restrictOwnAudio`. Chromium's Windows audio code has an
     "application loopback" path (`PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE`).
   - Stream audio could then live in WebView2, lip-synced with the video by WebRTC itself.
   - The trap, learned the hard way by Vesktop (two years of echo bugs) and Stoat:
     - Chromium's "exclude own audio" (`loopbackWithoutChrome`) only excludes **Chromium's** audio process.
     - Pulse plays voice from **Rust**, so it would *not* be excluded.
   - [cited, measured]
7. **Steal list, top 5:**
   1. Simulcast with an always-on 720p low layer.
   2. Viewer-driven quality (rendered pixel counts).
   3. Game-capture-safe defaults: WGC, a borderless hint, never inject.
   4. App-only audio, with Pulse explicitly excluded, and processing **off**.
   5. A stats overlay like Parsec's (encode, decode, network ms, codec, encoder name).

---

## 1. How Discord Go Live actually works

### 1.1 Signalling and transport

| Aspect | What Discord does | Source |
|---|---|---|
| Stream is its own RTC session | "A stream uses a separate RTC connection from the parent voice channel… its own WebSocket, transport, state, and media packets". Start with Gateway `Create Stream`, then wait for `Stream Create` and `Stream Server Update` | [cited] userdoccers `voice-connections.mdx` §Streams, https://github.com/discord-userdoccers/discord-userdoccers |
| Stream region | `PATCH /streams/{key}/stream` takes `region`, so a stream can live on a different voice server/region from the call | [cited] userdoccers `resources/voice.mdx` |
| Stream audio | "send that audio on the stream RTC connection as Opus", marked with the `SOUNDSHARE` speaking flag ("context audio for video, no speaking indicator") | [cited] userdoccers; Discord `voice-connections.mdx` (discord/discord-api-docs) |
| Native client wire | Native clients skip SDP and ICE, use a ~1 KB custom handshake and the libwebrtc `webrtc::Call` API, with a custom C++ media engine on a modified libwebrtc | [cited] Discord blog 2018-09-10 https://discord.com/blog/how-discord-handles-two-and-half-million-concurrent-voice-users-using-webrtc |
| Transport crypto | `aead_aes256_gcm_rtpsize` preferred, `aead_xchacha20_poly1305_rtpsize` required; older modes discontinued 2024-11-18 | [cited] discord-api-docs `developers/topics/voice-connections.mdx` |
| E2EE | DAVE (MLS group + per-sender AES-128-GCM frame keys), mandatory for DMs, voice and **Go Live** since 2026-03-02. Codec-aware unencrypted ranges keep packetizer metadata clear. On web, each connection gets its own Web Worker, with a separate one for screen share | [cited] https://daveprotocol.com ; https://discord.com/blog/every-voice-and-video-call-on-discord-is-now-end-to-end-encrypted (2026-05-18); https://discord.com/blog/bringing-dave-to-all-discord-platforms (2025-09-02) |
| Stream E2EE group | The stream uses a stream-specific MLS group (its media-session id), not the voice channel's | [cited] userdoccers |
| Previews are *not* E2EE | Thumbnails go via `POST /streams/{key}/preview` (an image) and even `/preview/video`, served from the CDN. The support article says previews are not E2EE | [cited] userdoccers `resources/voice.mdx`; support 25968222946071 |
| RTP header extensions the SFU speaks | audio-level, abs-send-time, transport-cc, mid, **playout-delay**, **video-content-type** (screen vs camera), video-timing, color-space, rid, repaired-rid, video-orientation, toffset | [cited] README of https://github.com/Discord-RE/Discord-video-stream ; userdoccers |
| Payload types seen | Opus 120, H.264 101 (RTX 102), H.265 103/104, VP8 105/106, VP9 107/108, AV1 109/110 | [cited] Discord-video-stream `CodecPayloadType.ts` |

### 1.2 The "watching" model: who decides quality

- **The sender transmits only when watched.** The voice server sends the sender `Media Sink Wants` (op 15). A want
  of `0` pauses that layer. [cited] userdoccers §Simulcasting. "Video is not sent until another user watches the
  stream" [cited] https://github.com/giorgi-o/DiscordVideoProtocol.
- **Viewers request quality and say how big they render the stream.**
  - Discord's client `goLiveQualityManager` sends `{[ssrc]: 100}` when the viewer is watching and `{[ssrc]: 0}` when
    not.
  - It adds `pixelCounts[ssrc] = width × height × zoom²` of the on-screen video element, debounced 500 ms. [measured:
    Discord web bundle `web.843cc7edc28c426c.js`, cached by Vesktop on this machine, 2026-10-14]
- **Stream connections offer one screen layer at quality 100.**
  - "Camera video commonly offers two layers (100 and 50). Stream connections commonly offer one screen layer at
    quality 100." [cited] userdoccers.
  - The 2024 blog says the streamer "does not send more data than the slowest viewer's connection can support"
    [cited] https://discord.com/blog/how-it-all-goes-live-an-overview-of-discords-streaming-technology.
- **Simulcast for Go Live exists only as an experiment.**
  - `2024-06_rtc_pacer__simulcast` has `simulcastEnabled: false` by default.
  - Its treatments are "Golive Simulcast 720p@500k / 750k / 1000k", with a "worker-pacer", "worker-pacer-probe" and
    BWE trendline window 3750 ms. [measured: Discord web bundle]
- **Codec choice is negotiated by the server across everyone.** It picks "a send codec per-client that all other
  clients can decode", and sends op 14 Session Update when someone joins who can't decode. [cited] userdoccers.
  This is why one weak device downgrades everyone (see §2, "mobile joins → all viewers tank").

### 1.3 Capture

| Fact | Source |
|---|---|
| A fallback chain of capture methods. Some "require Discord to reach into the application through dll-injection" | [cited] Discord blog 2024-03-05 (URL above) |
| Win10 default: "advanced technology", a **signed DLL injected** into the game to "extract rendered frames as they are being sent to Windows for display" | [cited] support 9410427556375 "[Windows] Capturing Application Window" (updated 2026-10-10) |
| Win11 default: **WGC**, but "it does not work for full screen exclusive (FSE) games". FSE is detected with `SHQueryUserNotificationState`, which "may give false positive". Advice: use Borderless | [cited] same |
| Debug panel shows "the number of frames captured for each screen share method" | [cited] same |
| Experiments live in Oct 2026: `2026-05-screenshare-picker-wgc`, `2026-02-wgc-dirty-regions-all`, `2026-04-video-hook-dx12`, `2026-06-upscale-small-captured-frames` (min 130 px), `2026-09-single-cpu-copy`, `2025-11-global-frame-pool-lock` | [measured: Discord web bundle] |
| HDR: `2026-02-go-live-hdr` with `hdrCaptureMode` default **"never"** (variants "always", "permittedDevicesOnly") | [measured] |
| "To share your game it needs to be recognized by our built-in game detection system… if not recognized… you will not be able to stream the game" (the app-stream path) | [cited] support 360040816151 |
| Overlay: the 2024 revamp no longer injects. It works only in borderless/windowed, "will not capture the Game Overlay" in window capture, and keeps a "legacy overlay" toggle for true fullscreen | [cited] support 25289844838551 (2024-07-30) |
| Discord System Helper (2025): an elevated helper so keybinds work in elevated games | [cited] support 34853435033367 |

### 1.4 Encode

| Fact | Source |
|---|---|
| VP8 and H.264, with HEVC and AV1 "on specific platforms"; HW preferred | [cited] Discord blog 2024-03-05 |
| AV1 since Jan/Feb 2023 for RTX 40 senders ("4K60 at 8 Mbps" in NVIDIA's framing); falls back when a viewer can't decode | [cited] https://www.nvidia.com/en-us/geforce/news/nvidia-studio-discord-av1/ (2023-02-24) |
| Settings flags: `h265Enabled: true`, `openH264Enabled: true` (OpenH264 = Cisco software codec toggle) | [measured: bundle defaults] |
| Moving to OS codecs: "Discord relies on codecs provided by the operating system… An AVC encoder is especially important for camera and Go-Live". Users are told to install the MS Store "AVC Encoder Video Extension" and the "AV1 Video Extension"; N/KN editions need the Media Feature Pack | [cited] support 37976724130711 and 37976093740695 (created 2026-01-27) |
| WMF encode experiments: `2025-12-wmf-gpu-encode`, `2026-01-wmf-gpu-encode-intel`, `2026-06-wmf-cpu-encode-intel`, `2026-07-nvenc-reconstructed-frames`, `2026-06-av1-encode-linux` | [measured] |
| Default web/desktop Go Live budget: `desktopBitrate {min 5e5, max 3.5e6, target 6e5}`; `getDefaultGoliveQuality()` capture/encode 1280×720, pixelCount 921600; camera budget 150 kbps to 2.5 Mbps | [measured] |
| AV1 rate rule: AV1 gets a configured bitrate (default **3.5 Mbps**; experiment variants 3.0/2.5) when ≤720p and ≤30 fps, or for source resolution under 10 fps. 1080p30 has its own experiment-driven `maxBitrate`. `2026-06-server-ladder` (9 Mbps default, disabled; variant 3.5 Mbps enabled) | [measured] |
| Native connection defaults: `keyframeInterval=0`, `minimumJitterBufferLevel=0`, `postponeDecodeLevel=100` | [measured] |
| Client-side quality measurement: PSNR and **VMAF v0.6.1 every 5000 frames** (`imageQualityWebrtcPsnrDb:5000,imageQualityVmaf_v061:5000,hwdec`) | [measured] |
| AMD fixes (2024): stop forcing keyframes on every fps change (it was "up to once a second"), relax per-frame bit caps, leave headroom so the WebRTC frame dropper doesn't halve fps. Sessions below 50 fps went from 39% to 18% | [cited] https://discord.com/blog/from-blocky-to-brilliant-improving-video-quality-on-discord-go-live-on-amd-gpus (2024-05-01) |
| Driver regressions they had to chase: NVIDIA 545.84 washed-out colour in Go Live (fixed in 546.31); AMD encoder init hangs causing `RTC_CONNECTING` (workaround: disable OpenH264) | [cited] support 18621944634391, 4978019693463 |

### 1.5 Audio (stream audio, voice, Krisp)

| Fact | Source |
|---|---|
| "We capture audio from the shared process and all its children" | [cited] blog 2024-03-05 |
| Audio sharing works on Windows/macOS desktop, Chrome and mobile, **not Linux** | [cited] support 360040816151 |
| Clips doc: stream via **Application**, not Screen, "to ensure game audio is also captured" | [cited] support 16861982215703 |
| An "Experimental audio capture" toggle disappeared around Oct 2025, after which users reported missing game voice lines or no sound | [measured: r/discordapp post 1oiyvmu, 2025-10-29] |
| Error 1001 "Sound Sharing Issues" means a "system-level permission request… Grant Access" | [cited] support 30952914470807 |
| Voice defaults: AEC on, NS off, AGC on, Krisp noise cancellation on, attenuation (ducking) 0, `attenuateWhileSpeakingOthers: true`, sidechain compression on (strength 50). Experiment `2026-06-systemwide-echo-cancellation-for-people-who-refuse-to-wear-headphones` switches the echo reference from "mix" to "auto" | [measured] |
| `2026-09-stream-zero-vad-leading`: a stream-audio VAD tweak | [measured; purpose: speculation] |
| Krisp runs on-device and is free in Discord; it auto-disables when CPU is high; it works in Go Live calls | [cited] support 360040843952 |
| `2026-02-process-boost-on-voice`: the Discord process runs at **HIGH priority** by default while in voice; variants test NORMAL plus thread-priority configs | [measured] |

### 1.6 Tiers and limits (Oct 2026)

| | Free | Nitro Basic | Nitro |
|---|---|---|---|
| Go Live (support caps table) | 720p30 | 1080p60 | Source @60 (support also says "up to 4K/60") |
| Server boost L1 / L2 | 720p60 / 1080p60 for everyone in that server | | |
| Viewers per stream | 50 (+ broadcaster) | | |

- [cited] support 33694251638295 (caps table), 360040816151.
- Users reported in Dec 2025 that 1080p was removed from "Nitro Classic" (r/discordapp post 1pjl0oe) [measured].
- A Go Live auto-quality preset is rolling out (`2025-10-go-live-auto-quality`: allow/default/migrate) [measured].

### 1.7 Server side and latency

- **SFU history:**
  - The SFU was an in-house C++ "RTC worker" in 2018 [cited].
  - By 2026 voice/video runs on **Cloudflare's edge** for >80% of traffic, in 300+ cities, on 25,000+ SFU instances.
  - A **Rust SFU** (single-threaded Tokio per worker) was trialled in Europe in late 2025. Its recv loop starved the
    flush timer, sending ping above 500 ms until a 9 ms forced-flush budget was added.
  - [cited] https://discord.com/blog/how-we-moved-discord-voice-to-the-edge (2026-06-09);
    https://discord.com/blog/behind-the-scenes-of-the-3-25-26-voice-outage (2026-04-29)
- **Placement lesson:**
  - All participants connect to one SFU host per call. Placing a mixed-region call in a small PoP made remote users'
    ping 2.7×.
  - A transit problem (Telia to Orange) caused >1 s latency and +30% freeze ratio until it was reverted. [cited, same]
- **Latency:**
  - Discord publishes **no** glass-to-glass figure. It says it adds some latency to cut drops, and drops frames once
    latency is too high. [cited, blog 2024-03-05]
  - Native defaults of jitter buffer min 0 and keyframe interval 0 suggest a low-latency target. [measured]
  - Typical Discord Go Live latency is **~150-400 ms** [est.; no primary measurement found].

---

## 2. The pain: why Go Live gets left in the dust

### 2.1 Datasets (measured)

| Dataset | Size | Stream-related | Method |
|---|---|---|---|
| r/discordapp posts, 2025-10-01 to 2026-10-13 (via the Arctic Shift archive API) | 47,720 | **1,933 (4.1%)** | Keyword classifier on title + first 800 chars; one post can hit several categories. Precision eyeballed on ~15 titles per category: roughly 70-85% on-topic [est.] |
| Discord's feedback forum (Zendesk community; newest ~9.8k posts Jul 2023 to May 2026, plus top 4k by votes) | 13,472 | 409 (3.0%) | Same classifier. Mostly feature requests, so it is used for "wishes", not bug frequency |

### 2.2 Failure atlas (r/discordapp, share of the 1,933 stream posts)

| # | Failure | Share | Typical root cause (and how sure we are) |
|---|---|---|---|
| 1 | **Won't start / stops / crashes / viewer can't load** | 48.9% (sub-hits: stops midway 158, start fails 111, viewer can't watch or infinite load 25, PC freeze/BSOD 12) | Several causes: <ul><li>RTC connect/endpoint failures ("Awaiting Endpoint", the 2026-03-25 outage) [cited]</li><li>"preview paused to save resources" when the game gets focus (post 1ve5vkg) [measured]</li><li>AMD encoder init hang [cited]</li><li>error codes 2001, 2007, 2011-2015 = start failure, reconnecting, streamer/viewer timeouts [cited support 30952914470807]</li><li>PC-wide freezes on Ryzen + RTX 30 fixed by PCIe 3.0 / BIOS (support 9360429789463) [cited]</li></ul> |
| 2 | **Lag / stutter / low fps for viewers** | 19.5% | <ul><li>Sender uplink and BWE.</li><li>The single-encoding design: the slowest viewer sets quality (post 1pnapb9, "Stream tanks for all viewers if a mobile user joins") [measured].</li><li>The WebRTC frame dropper feedback loop on AMD [cited].</li><li>Codec renegotiation [cited].</li></ul> |
| 3 | **Stream audio missing or wrong** | 14.1% | <ul><li>Screen-vs-app capture semantics ("stream via Application to get audio") [cited].</li><li>Process-tree capture misses audio played by other processes, e.g. in-game voice chat in BF6/Marathon (post 1rmdnv3) [measured; root cause: speculation].</li><li>The removed "experimental audio capture" path [measured].</li><li>Output-device routing.</li><li>Friend "can only hear stream when I talk" (post 1nurcq7) suggests VAD/gating applied to share audio [speculation].</li></ul> |
| 4 | **CPU/GPU/encoder trouble** | 10.3% | <ul><li>Software fallback (OpenH264).</li><li>Driver regressions [cited].</li><li>AV1 only on NVIDIA ("no AV1 encoding except Nvidia going into 2026", post 1prqrod) [measured].</li><li>"Discord using 100% GPU" in specific games.</li></ul> |
| 5 | **Blurry / blocky / "potato"** | 9.1% | <ul><li>Low default budget (600 kbps target, 3.5 Mbps max on the default path) [measured].</li><li>Tier caps.</li><li>A sudden server-wide drop ("past week… quality took a huge nosedive", post 1wz1evm, 2026-10-06) is consistent with an experiment or ladder change [speculation].</li></ul> |
| 6 | **Multi-monitor / fullscreen / ultrawide** | 7.2% | <ul><li>FSE games: WGC fails, hook needed, false FSE detection [cited].</li><li>"This is what screenshare looks like… fullscreen; windowed works" (1rjb946, 138 comments).</li><li>Win11 + AMD 7000 + ultrawide broken, traced by users to AV1 encode/decode; workaround: switch to 16:9 or have a non-AV1 viewer join (1qpg5ld, Jan 2026) [measured].</li></ul> |
| 7 | **Black screen / flicker / stale-frame flicker** | 5.6% | <ul><li>DRM content (Netflix/Prime → HDCP black) is expected.</li><li>FSE capture.</li><li>Hybrid GPU (wrong adapter).</li><li>"Flickering a frame from earlier in the stream rapidly" (1vv5n1h) and AMD flicker (1s7mxn5) suggest decoder/reference corruption [speculation].</li></ul> |
| 8 | **Game FPS drop / input lag while streaming** | 3.5% (+ cursor lag 1.9%) | <ul><li>Capture plus encode contention; DWM/WGC cursor lag.</li><li>"Streaming >10 min → severe system lag" (1wdmmhx, RX 9070 XT).</li><li>"60 fps streams tank whole PC, 30 fps fine" (1qoadsl).</li><li>Reports of a game FPS cap at 60 while Go Live is on (old forum thread) [cited].</li></ul> |
| 9 | **Hearing yourself / voice chat in the stream (echo)** | 2.3% | <ul><li>Full-screen share captures system audio, including Discord's own voice playback ("How do I stream with audio WITHOUT streaming discord audio?", 1rb8n8k) [measured].</li><li>Viewers' own speakers.</li></ul> |
| 10 | **HDR washed-out / brightness pumping** | 1.6% | <ul><li>HDR capture off by default (`hdrCaptureMode:"never"`) [measured].</li><li>Tone mapping.</li><li>"brightness constantly steps up and down" (1wnn4jz, 2026-09).</li><li>The forum's top HDR fix was a hidden "HDR Screen Capture" experiment [cited forum 360044838292].</li></ul> |
| 11 | **Game not detected / anti-cheat** | 1.0% | <ul><li>Game detection gates the app-stream path [cited].</li><li>Hook capture blocked or flagged by some anti-cheats (prior report).</li><li>Rare in posts, because WGC/display capture works [est.].</li></ul> |
| 12 | **Paywall / limits** | 3.6% | <ul><li>720p30 free.</li><li>The Nitro Classic change.</li><li>Boost confusion.</li></ul> |

Platform mentions in stream posts:

- browser/web client 166;
- Windows 11: 56;
- NVIDIA 53, AMD 50, Intel 12;
- macOS 50;
- DRM services 34;
- Linux 26;
- Windows 10: 19;
- HDR 15;
- AV1 9.

The web-client cluster matters for Pulse, because we will be "Discord-web-in-Chromium" too (see §4). [measured]

### 2.3 Wishes that never went away (feedback forum, votes)

| Wish | Votes | Note |
|---|---|---|
| Video and screen share at the same time | 2,537 | |
| Show who is live, in the channel list | 2,514 + 1,677 | |
| Linux screen-share sound | 1,891 | Linux audio |
| Mute or volume a stream without muting the person | 245 | |
| Screen-share audio sliders | 245 | |
| Watch without joining the voice channel | 83 + 72 | |
| Viewer-side "lower quality for watching" | 124 | |
| Show the cursor | 161 + 130 | |
| Remote pointer / annotate | 179 + 45 | |
| Multiple monitors at once | 51 + 83 | |
| Pop-outs per stream on different monitors | 63 | |
| Stop pausing my own preview when the game has focus | 129 | |
| HDR | 70 + 17 | |

[measured]

### 2.4 Competitor failure echoes (same disease, other apps)

- **Stoat (ex-Revolt; LiveKit JS in Electron, the closest architectural twin):**
  - share audio echo: others hear their own voices (for-desktop #247; for-web #1517);
  - "underwater" share audio that mutes when the listener speaks (for-web #1366);
  - publish-triggered signalling ping-timeout leading to call drop, fixed by upgrading to `livekit-client` 2.22.3
    (#1679);
  - multi-viewer flicker from visibility-based auto-subscribe (#1537);
  - virtual VR display adapters breaking capture (#1490);
  - WGC yellow-border on Win10 stalling Explorer over long window shares (for-desktop #263).
  - [cited] https://github.com/stoatchat/for-web/issues , https://github.com/stoatchat/for-desktop/issues
- **TeamSpeak 6 (P2P mesh WebRTC):**
  - "Without GPU encoding and server sharing… useless; 40% CPU" (topic 55041);
  - AV1 default makes streams lag for others (62485);
  - stream audio played on the default *communication* device (62466);
  - stuck "connecting" behind NAT (55152, 12.8k views);
  - League of Legends' client/game window swap breaks app capture (64755);
  - a multi-app audio picker in alpha, Sept 2026 (65236).
  - [cited] https://community.teamspeak.com
- **Vesktop:** see §4.

---

## 3. Competitors: what they do that Discord doesn't

| Product | Model | Ideas worth stealing | Source |
|---|---|---|---|
| **Parsec** | P2P 1:1 / few, DDA capture, vendor HW encoders | <ul><li>**Never touches system memory** (capture → shader NV12 → encoder).</li><li>V-sync with drop-accumulated-frames.</li><li>**Doesn't send frames when nothing changes**, unless "Constant FPS" (which improves still-image clarity).</li><li>Opt-in **4:4:4**, H.265, 10-bit.</li><li>Overlay shows **encode / decode / network ms**, decoder type, codec, chroma, plus warnings: "host is using software encoding", "you are having network issues", "your hardware can't keep up".</li><li>Console line `FPS, L(encode), B(bitrate/max), N(slow-rtx/fast-rtx/congestion)`.</li><li>Known killers: NVIDIA driver encode-latency regressions, G-Sync, HAGS, power saving, Wi-Fi.</li></ul> | https://parsec.app/blog/description-of-parsec-technology-b2738dcc3842 ; support.parsec.app articles 32381352822804 (2025-11-03), 32381603663636 (2025-03-17), 32381785123860 (2026-07-07) |
| **Sunshine / Moonlight** | P2P 1:1, custom RTP + FEC | <ul><li>**Infinite GOP + reference-frame invalidation (RFI)** instead of keyframes on loss.</li><li>Optional **intra-refresh** (period 300) + recovery-point SEI.</li><li>NVENC `ULTRA_LOW_LATENCY` tuning, zero reorder delay, no lookahead.</li><li>**Single-frame VBV** (`vbv = bitrate/fps`), 2-pass quarter-res.</li><li>**FEC 20%**.</li><li>`nvenc_latency_over_power`: force high P-state, because "Adaptive P-State… doesn't work well with low latency streaming".</li><li>HAGS realtime priority can freeze NVENC near full VRAM.</li><li>Min-fps target with duplicate frames for static content.</li><li>Client "frame pacing" that delays early frames to cut micro-stutter.</li></ul> | https://github.com/LizardByte/Sunshine `docs/configuration.md`, `src/nvenc/nvenc_base.cpp` ; https://github.com/moonlight-stream/moonlight-qt `pacer.cpp`, `SettingsView.qml` |
| **Chrome Remote Desktop** | WebRTC | <ul><li>VP9/AV1 tuned `CONTENT_SCREEN`.</li><li>**Cyclic-refresh "top-off"**: static screens converge to high quality.</li><li>**Lossless colour (I444) option**.</li><li>**Active maps from dirty regions**: only encode what changed.</li><li>Min bitrate per megapixel chosen so a static screen converges in <10 frames.</li></ul> | chromium `remoting/codec/webrtc_video_encoder_vpx.cc`, `webrtc_video_encoder_av1.cc` |
| **Chrome WebRTC (what we get for free)** | | <ul><li>"Zero-hertz" screenshare: static content repeats at 1 Hz until QP converges.</li><li>Low-latency renderer when playout-delay min=0, max>0 (max ≤500 ms).</li><li>MF HW encoder with **software rate control for desktop H.264**, temporal layers L1T2/L1T3 on Intel/NVIDIA, `kWebRTCHardwareVideoEncoderFrameDrop`.</li><li>HEVC HW encode enabled on Windows.</li></ul> | webrtc `video/frame_cadence_adapter.cc`; commit 111e981 (2020-10-26); chromium `media/gpu/windows/media_foundation_video_encode_accelerator_win.cc`, `media/base/media_switches.cc` |
| **Google Meet / Chrome AV1** | SFU | AV1 brings "significant screen sharing efficiency improvements over VP9" and video at ≤30 kbps. A WebCodecs-level "force screen content tools" API was **dropped** in favour of `contentHint` | https://blog.chromium.org/2021/03/chrome-90-beta-av1-encoder-for-webrtc.html ; chromestatus 6307770441138176 |
| **Jitsi** | SFU | <ul><li>Screenshare **5 fps by default**, with `contentHint='detail'`; `'motion'` when fps >5.</li><li>For low-fps shares it disables lower layers and keeps only the HD layer ("ensures the highest resolution stream is available always"); L1T3 for capped screenshare.</li><li>Screenshare codec AV1 configurable.</li></ul> | https://github.com/jitsi/jitsi-meet `config.js`; lib-jitsi-meet `TPCUtils.ts`, `ScreenObtainer.ts` |
| **Element Call** | LiveKit JS + E2EE | <ul><li>Screen share 1080p30 at 5 Mbps by default, optional simulcast layers.</li><li>`backupCodec: vp8`.</li><li>RED disabled because of E2EE.</li><li>adaptiveStream + dynacast.</li></ul> | https://github.com/element-hq/element-call `sdk/src/config.ts`, `livekitOptions.ts` |
| **Microsoft Teams** | SFU, RDP or video-based screen sharing | Screen sharing is adaptive **1.875-7.5 fps** at minimum bandwidth, 7.5-30 fps recommended, 15-30 best. HD under 1.5 Mbps. Detail over motion | https://learn.microsoft.com/en-us/microsoftteams/prepare-network (2026-05-26) |
| **OBS** | WHIP/WebRTC out | WHIP with H.264/HEVC/AV1, and **WebRTC simulcast since 32.1.0 (2026-03-11)**. LiveKit Ingress accepts WHIP (separate service; can pass through without transcoding) | https://github.com/obsproject/obs-studio/releases ; https://docs.livekit.io/home/ingress/overview/ |
| **TeamSpeak 6** | P2P mesh | <ul><li>A per-app multi-app **audio picker** (alpha).</li><li>Right-click → codec info.</li><li>A lesson: mesh + AV1 + software encode = laggy.</li></ul> | community.teamspeak.com topics above |
| **Steam Remote Play Together, NVIDIA, Guilded, Zoom** | | Not verified in depth (web-search budget ran out; the Steam help pages need JS). Zoom/Steam knobs are in the earlier server-quality report. "NVIDIA ShareStream" could not be confirmed as a current product [speculation that it refers to GameStream, end-of-life 2023] | n/a |

---

## 4. Vesktop: Discord's web Go Live inside Chromium (our closest living analogue)

Local install (read only): Vesktop 1.6.7 (pacman, installed 2026-10-06), **Electron 43.2.0 / Chromium 150.0.7871.129**.
I recovered the sources from the `app.asar` source maps. I did not launch the app or read any account data. I only
pulled Discord's public JS bundles out of the HTTP cache. [measured]

| What | Detail | Lesson for Pulse |
|---|---|---|
| Custom picker | `session.setDisplayMediaRequestHandler` + `desktopCapturer.getSources` (176 px thumbnails, 1920 px on demand) and its own React picker (resolution 480-2160, fps 15/30/60, content-hint, audio sources) | WebView2 offers the same hook (its `ScreenCaptureStarting` event, or our own picker) [est.] |
| Overriding Discord's caps | Patches Discord's `getDefaultGoliveQuality()` to `bitrateMin 500k, bitrateMax 8M, bitrateTarget 600k` plus the chosen width/height/fps | Discord's own defaults are the bottleneck; with our own client we set them directly |
| Constraints | On Linux it forces `frameRate {min, ideal}`, `width/height {min 640×480, ideal, max}`, **`resizeMode:"none"`**, and sets `track.contentHint` | Stops Chromium's adapter downscaling; pin the capture size and let the encoder adapt |
| Share-audio processing | Linux venmic audio track with `echoCancellation:false, noiseSuppression:false, autoGainControl:false, 48 kHz, 2 ch` | Never run voice DSP on stream audio (Stoat #1366 is what happens otherwise) |
| Windows audio and echo | <ul><li>`audio:"loopback"` (whole system) caused the "call audio is captured… echoed back" bug (#789, 2024-08).</li><li>`loopbackWithMute` was tried, then **reverted**: it "mutes the system audio for the user".</li><li>Fixed 2026-08-18 with undocumented **`loopbackWithoutChrome`** on build ≥20348, lowered to **≥19045 (Win10 22H2)** on 2026-09-04 (#1294, #1309).</li><li>#334 "streaming with audio does not work" (2024-01) is slated for v1.6.8 (2026-09).</li></ul> | It works for Vesktop only because Discord-web's voice plays through Chromium's audio process. **Pulse voice plays from Rust, so a Chromium exclusion would not remove it.** Use app-only capture (`windowAudio:"window"` → application loopback of the game's process tree) or our Rust process loopback excluding Pulse's process tree |
| Window-only audio | #569 (closed): "using electron API we can stream either all sound from Windows system or sound from one tab" | Electron lags Chrome here; Chrome 141 has `windowAudio`. Spike whether the WebView2 picker exposes it |
| venmic (Linux) | PipeWire include/exclude by node properties, "only speakers", "ignore input media", "ignore virtual", a workaround for mic-shared-instead | This is the Linux analogue of process loopback; a good model for our include/exclude UI |
| Chromium flags | <ul><li>`AcceleratedVideoEncoder/Decoder` (+ Linux GL zero-copy) when "video HW accel" is on.</li><li>`disable-renderer-backgrounding`, `disable-background-timer-throttling`, `disable-backgrounding-occluded-windows`.</li><li>Windows: disable `CalculateNativeWinOcclusion`.</li><li>`autoplay-policy=no-user-gesture-required`.</li><li>Disable `HardwareMediaKeyHandling`, `MediaSessionService`.</li></ul> | **Occlusion and backgrounding throttling must be off** in WebView2 or a minimised Pulse will starve capture/encode. Equivalents go via `AdditionalBrowserArguments` [est.] |
| Issue themes | <ul><li>"Screenshare has poor performance" (#191, 105 comments): fps collapses when the Discord window isn't focused.</li><li>Very low fps (#629, #525).</li><li>Chromium DMA-buf stutter fixed in Chromium 126.</li><li>Adding an audio track dropped video bitrate to ~28 kbps once a viewer joined (#528).</li><li>Frametime worse while sharing (#1062): "screen capture is handled entirely by chromium… we have 0 control".</li><li>Fullscreen apps unshareable (#372).</li><li>Wayland corrupt/infinite loading (#1207).</li></ul> | The Chromium-path failure modes we inherit: background throttling, encoder fallback, and audio-track/bitrate-allocation bugs. Plan getStats-based self-checks |

Sources: https://github.com/Vencord/Vesktop (`src/main/screenShare.ts`, `src/main/main.ts`,
`src/renderer/patches/screenShareFixes.ts`, `src/renderer/components/ScreenSharePicker.tsx`, commits `156ba6ab`,
`c7d830c5`, `cb9c55fe`, `84d6f177`, and issues above); https://github.com/Vencord/venmic.

---

## 5. Ranked: ideas to steal

| Rank | Idea | Why | Where it lands in Pulse |
|---|---|---|---|
| 1 | **Simulcast with a guaranteed low layer** (e.g. top preset + 720p/30 at ~1 Mbps) | Discord's #2 complaint is one weak viewer dragging everyone down; Discord itself is A/B testing exactly this | WebView2 `livekit-client` publish with `screenShareSimulcastLayers`; watch NVENC session count (2 layers = 2 sessions) |
| 2 | **Viewer-driven quality**: report rendered size and pick the layer | Discord's `pixelCounts` × zoom² design | LiveKit `adaptiveStream` on the `<video>` element (sizes known in WebView2) + dynacast so unwatched layers pause |
| 3 | **Watch opt-in, sender idle until watched** | Discord's model; Stoat added the same after flicker bugs | Pulse-controlled subscribe (no visibility-based auto-unsubscribe for screen tiles, per Stoat #1537); dynacast pauses publishing |
| 4 | **App-only stream audio with voice DSP off; Pulse always excluded** | Echo, "underwater" audio and missing audio are ~16% of posts | Option A: `getDisplayMedia({windowAudio:"window"})` in WebView2 (A/V sync for free), with `echoCancellation/noiseSuppression/autoGainControl:false`. Option B: Rust process loopback, `EXCLUDE_TARGET_PROCESS_TREE` on Pulse's PID for screen shares. Never use plain system loopback |
| 5 | **Capture defaults that never inject** | Hooks cost anti-cheat trust; Discord is still on hooks for Win10/FSE | WGC via Chromium; detect FSE (Discord uses `SHQueryUserNotificationState`) and tell the user "switch to Borderless" before going live |
| 6 | **Stats overlay like Parsec's** | Users can't diagnose Discord; Parsec's three numbers are self-explaining | `getStats()`: `encoderImplementation`, `powerEfficientEncoder`, qualityLimitationReason, fps sent/received, jitterBufferDelay, freezes; Rust adds RTT/loss. Warnings: "software encoding", "your upload limits quality", "viewer can't keep up" |
| 7 | **Low-latency render path** | Chrome's low-latency renderer triggers on playout-delay min=0 | LiveKit `room.playout_delay {enabled, min:0, max:≤500}` (room-wide) and/or receiver `jitterBufferTarget`; measure vs. smoothness in the spike |
| 8 | **Never force keyframes on rate changes; prefer recovery over IDR** | Discord's AMD fix; Sunshine's RFI and intra-refresh | Chromium handles PLI; keep LiveKit PLI throttling sane, raise UDP buffers (earlier report), avoid resolution flips (`degradationPreference: maintain-resolution` for text, `balanced` for games) |
| 9 | **Static-content convergence** (zero-hertz / Constant FPS / top-off) | Text/code sharing looks blurry when bitrate is spent on motion | `contentHint:"detail"` + a "sharp text" preset (1080p, 5-15 fps) like Jitsi/Teams; Chrome's zero-hertz mode kicks in for screen content |
| 10 | **Keep the app awake** | Vesktop's #1 perf bug: focus/occlusion throttling | WebView2 args: disable occlusion, backgrounding and timer throttling; keep the hidden video connection in a never-throttled host |
| 11 | **Auto quality + per-viewer quality picker** | Discord is rolling out "auto"; forum wish "lower quality for watching" | Default "Auto" (BWE-driven layer); viewer menu "Auto / Source / Low" maps to LiveKit `setVideoQuality` |
| 12 | **Thumbnail previews outside the media path** | Discord uploads a still image to the CDN; no need to subscribe to video | Sharer posts a JPEG to our server every ~30-60 s (canvas from the local track) [est.]. We're E2EE-agnostic; with E2EE note previews leak as Discord's do |
| 13 | **Per-stream volume, mute the stream not the person, cursor toggle, multi-popout** | Top long-standing wishes | Rust mixer gives stream audio its own gain; `cursor:"always"` constraint; WebView2 multi-window |

## 6. Ranked: failure modes we must design out

| Rank | Failure mode | Design-out in our architecture |
|---|---|---|
| 1 | **Start/stop/crash, no explanation** (~49%) | <ul><li>Pre-flight before Go Live: WGC available? HW encoder present (`RTCRtpSender.getCapabilities` + a test encode)? Uplink estimate?</li><li>Map every failure to a human message plus an error code like Discord's 2001-2015.</li><li>Pin `livekit-client` ≥2.22.3 (publish-path reconnect fix, Stoat #1679).</li><li>The hidden video connection must not take voice down with it.</li></ul> |
| 2 | **One weak viewer ruins everyone** | Simulcast + per-viewer layer selection; never renegotiate the *publisher's* codec because a viewer joined. Use LiveKit `backupCodec` (H.264 primary or backup when AV1 is used) |
| 3 | **Echo of friends' voices in the stream** | App-only audio by default. For full-display shares, exclude Pulse's process tree (Rust) and verify the WebView2 child processes are inside that tree [est., spike]. Never Electron-style `loopback` |
| 4 | **Voice DSP on stream audio** ("underwater", ducking, gating) | Stream audio bypasses AEC/NS/AGC/VAD/attenuation in our Rust mixer; disable constraints on any display-audio track; music-grade Opus (stereo, 96-128 kbps, no DTX) |
| 5 | **Missing game audio** (separate audio processes, in-game VC) | Show a live level meter for "what viewers hear" before going live; allow adding extra processes (TeamSpeak-style multi-app picker) |
| 6 | **FSE black/frozen capture** | Detect FSE and prompt for Borderless; monitor capture fps; if frames stall >2 s, show "game is in exclusive fullscreen" |
| 7 | **Background/occlusion throttling kills fps** | Chromium switches (see Vesktop) + a test with Pulse minimised behind a fullscreen game |
| 8 | **Encoder trouble** (SW fallback, AMD/AV1 regressions, driver bugs) | Show `encoderImplementation` in stats; default to H.264 HW; AV1 opt-in only after a capability probe; per-vendor kill switches in server config |
| 9 | **Game FPS hit / cursor lag** | Cap capture fps to the preset; avoid >60 fps capture; note Discord's "60 fps tanks PC" reports; spike-measure the game frame time delta with and without sharing |
| 10 | **HDR washed out** | Detect an HDR display; rely on Chromium/WGC SDR conversion; warn "HDR is on; colours may differ"; test with Windows HDR on |
| 11 | **Multi-monitor, ultrawide, virtual displays** | Picker lists real monitors; ignore virtual/VR adapters (Stoat #1490); allow non-16:9 encode sizes; test ultrawide |
| 12 | **Paywall-feel / opaque quality** | No tiers; show the actual resolution, fps and bitrate to the sharer and viewers |
| 13 | **Long-share Explorer stalls** (Win10 WGC yellow border) | Prefer monitor capture on Win10 for long sessions, or test border-free paths; flag it in the spike |

## 7. Open questions for the spike

1. Does WebView2's `getDisplayMedia` honour `windowAudio:"window"` and `restrictOwnAudio` (Chrome 141+)? Does its
   picker show "share window audio"?
2. Are WebView2's `msedgewebview2.exe` processes inside Pulse's process tree for `EXCLUDE_TARGET_PROCESS_TREE`?
3. Does LiveKit 1.13.7 forward playout-delay and trigger Chrome's low-latency renderer, and at what smoothness cost?
4. NVENC session count with 2-layer simulcast + OBS/ShadowPlay running.
5. Real glass-to-glass latency vs. Discord on the same machines (measure both; Discord publishes nothing).

## Sources (all read 2026-10-14)

- Discord blog:
  - https://discord.com/blog/how-it-all-goes-live-an-overview-of-discords-streaming-technology
  - https://discord.com/blog/from-blocky-to-brilliant-improving-video-quality-on-discord-go-live-on-amd-gpus
  - https://discord.com/blog/how-discord-handles-two-and-half-million-concurrent-voice-users-using-webrtc
  - https://discord.com/blog/how-we-moved-discord-voice-to-the-edge
  - https://discord.com/blog/behind-the-scenes-of-the-3-25-26-voice-outage
  - https://discord.com/blog/every-voice-and-video-call-on-discord-is-now-end-to-end-encrypted
  - https://discord.com/blog/bringing-dave-to-all-discord-platforms
  - https://discord.com/blog/discord-update-march-24-2026-changelog
- Discord support (via the Zendesk JSON API), article IDs:
  - 360040816151, 9410427556375, 9360429789463, 18621944634391, 37976724130711, 37976093740695, 4978019693463;
  - 30952914470807, 360045138471, 33030151293079, 360045784891, 360040843952, 25968222946071, 33694251638295;
  - 16861982215703, 25289844838551, 34853435033367, 360032665831.
- Protocol:
  - https://daveprotocol.com
  - https://github.com/discord/discord-api-docs (`developers/topics/voice-connections.mdx`)
  - https://github.com/discord-userdoccers/discord-userdoccers (`pages/topics/voice-connections.mdx`,
    `pages/resources/voice.mdx`)
  - https://github.com/Discord-RE/Discord-video-stream
  - https://github.com/giorgi-o/DiscordVideoProtocol
- Discord client code: web bundle `web.843cc7edc28c426c.js` from Vesktop's local HTTP cache (public JS, no account
  data).
- Vendors:
  - https://www.nvidia.com/en-us/geforce/news/nvidia-studio-discord-av1/
  - https://parsec.app/blog/description-of-parsec-technology-b2738dcc3842
  - support.parsec.app (Zendesk API) 32381352822804, 32381603663636, 32381785123860
- Open source:
  - https://github.com/LizardByte/Sunshine
  - https://github.com/moonlight-stream/moonlight-qt
  - https://github.com/jitsi/jitsi-meet
  - https://github.com/jitsi/lib-jitsi-meet
  - https://github.com/element-hq/element-call
  - https://github.com/livekit/client-sdk-js (v2.22.4)
  - https://github.com/livekit/livekit (`config-sample.yaml`)
  - https://github.com/obsproject/obs-studio/releases
  - https://github.com/Vencord/Vesktop
  - https://github.com/Vencord/venmic
  - https://github.com/stoatchat/for-web
  - https://github.com/stoatchat/for-desktop
- Chromium / WebRTC:
  - `media/audio/win/audio_low_latency_input_win.cc`
  - `media/gpu/windows/media_foundation_video_encode_accelerator_win.cc`
  - `media/base/media_switches.cc`
  - `remoting/codec/webrtc_video_encoder_{vpx,av1}.cc`
  - https://webrtc.googlesource.com/src/+/111e9814665c2aadc19ec3b2d31f36883c1508b6
  - https://developer.chrome.com/release-notes/141
  - chromestatus API features 5072779506089984, 5128140732760064, 6307770441138176
  - https://blog.chromium.org/2021/03/chrome-90-beta-av1-encoder-for-webrtc.html
- Others:
  - https://learn.microsoft.com/en-us/microsoftteams/prepare-network
  - https://docs.livekit.io/home/ingress/overview/
  - https://community.teamspeak.com (topics 55041, 62485, 65163, 64755, 65236, 62466, 55152, 65521)
- Datasets:
  - r/discordapp posts via https://arctic-shift.photon-reddit.com API (2025-10-01 to 2026-10-13, 47,720 posts);
    post IDs are cited inline.
  - Discord feedback forum via the `support.discord.com/api/v2/community/posts` API.
- Limitation: the web-search budget ran out mid-research. Steam Remote Play, Guilded, Zoom and "NVIDIA ShareStream"
  were not verified from primary sources.
