# Screen share: deep research summary (round 2)

Date: 2026-10-14. This replaces `screenshare-summary.md` (round 1) as the input for the screen-share spec. The
reports are beside this file:

- `screenshare-deep-discord-teardown.md`
- `screenshare-deep-webview2-capture.md`
- `screenshare-deep-codecs.md`
- `screenshare-deep-server-load.md`
- `screenshare-deep-network.md`
- `screenshare-deep-audio-sync.md`
- `screenshare-deep-ux.md`
- `screenshare-deep-architecture.md`
- `screenshare-deep-perf-reliability.md`
- `screenshare-deep-wacky.md`

`screenshare-deep-crosscheck.md` rules on their contradictions, re-checks 25 load-bearing claims against primary
sources, and holds the ordered spike checklist (S1-S17).

**Caveat:** nothing here was measured on Windows with WebView2 and a real GPU. Lab numbers come from Linux and
Chromium proxies. They are strong leads, not facts, until the spike repeats them.

## What we're designing for

- **Decided:**
  - v1 shares from Windows only. Linux can view but not share.
  - Mostly games, sometimes text or code.
  - Quality and fps pickers.
- **The group's real Discord pains, in priority order:**
  1. The streamer's game FPS tanks while streaming.
  2. Viewers see **stutter**: freezes, choppiness, sudden blur. Delay isn't the complaint.
  3. The streamer's PC gets slower after about 30 minutes until a reboot fixes it.
- **Reddit agrees on the shape of the problem.** Of the r/discordapp posts about streaming, about 49% are "won't
  start, stops or crashes", 20% lag, 14% audio and 10% CPU/GPU cost.

## The architecture (after the cross-check)

| Part | Decision | Why |
|---|---|---|
| Publish video | `livekit-client` in WebView2, `getDisplayMedia`, hardware H.264 | The Rust SDK still has no hardware codecs on Windows (0.9.5). Chromium gives Media Foundation hardware encode, simulcast and rate control. |
| JS participant | **One visible participant per user, `<uid>#s`.** It publishes when the user goes live and subscribes when they watch. It connects on first use and leaves when the user leaves voice. | A hidden participant can't usefully publish. Rust and the webhook filter the `#s` suffix. |
| View video | `livekit-client` `<video>` with hardware decode (JS viewer) | Chromium paces frames for us, and pacing is exactly where stutter comes from. The Rust pass-through viewer (rust-sdks #1515, not yet merged) is a later option. |
| Voice | Unchanged, in Rust | |
| Stream audio | Rust process loopback. A window share captures the game's process tree; a screen share captures everything except Pulse. It plays through our mixer, so the echo canceller hears it. | Chromium's per-app audio is Windows 11 only, and its "exclude own audio" doesn't exclude `pulse-app.exe` (our voices). |
| Previews | A 480×270 WebP every 15 s over HTTP, plus a gateway event | About 10 kbps per person looking, against 200-2,000 kbps for any live layer |

**Prerequisite bug (confirmed in code):** voice receive (`voice/mod.rs` → `rx.rs`, `mixer.rs`) is keyed by
participant identity. A second audio track from the same person would replace their mic, and ending the stream
would silence them. Key it by track before any stream audio lands.

## Findings that change the design

### Pain 1: streamer's game FPS

- **The silent NVIDIA software fallback.** Found independently by three agents and verified in Chromium source
  (`media_foundation_video_encode_accelerator_win.cc`, crbug 1088650).
  - Chromium skips NVIDIA's hardware encoder for H.264 Constrained Baseline, and LiveKit negotiates exactly that
    (`42e01f`) by default.
  - On a GeForce PC the "hardware" stream can run on OpenH264 in software instead. That takes 2-7 cores at 1080p60,
    with the worst quality.
  - **Fix on the server:** keep LiveKit's full default `enabled_codecs` list and add strict H.264 lines for `4d001f`
    and `42001f`. Never ship a trimmed list: it drops voice RED and video RTX.
  - The client can then prefer Main. Spike S1.
- **Pulse's own animated UI costs about a core while streaming** (measured). Freeze animations and turn the self-preview
  off while live.
- **Capture isn't zero-copy in Chromium.**
  - Every frame is read back from the GPU and converted on the CPU, at full captured size even when we send 720p.
  - Expect about 3-10% game FPS loss at 1080p30, and more at 60 fps.
  - A 30 fps cap halves it.
  - If S2 shows more than 5%, a native zero-copy capture path moves up the v2 list.
- **Hardware only for games.** A software fallback must warn the user. It must never be silent.
- **Upscaling on the viewer.** Busy 3D games can go out at 720p/900p and be upscaled on the viewer with Lanczos or
  FSR1, never bilinear. Measured to look like 1080p at 10-20% fewer bits. Text is never downscaled.

### Pain 2: viewer stutter

- **`rtx-time=300` in the viewer's SDP answer.**
  - Without it, one unrepaired lost packet at 1% loss froze the picture for about 3 s in 7 of 8 lab runs.
  - With it, there were no freezes of 2.5 s or longer.
- **Never fill the streamer's upload.** Cap video at about 70% of a measured upload and pick the preset from that.
  Voice shares the queue, and LiveKit gives published audio no congestion control.
- **Always publish one low layer** (360p+, 30 fps).
  - A single-layer screen share is exempt from LiveKit's stream allocator. A slow viewer just freezes; the lab saw 45 s
    with no picture.
  - Spike S5 measures what the extra layer costs the streamer.
  - With dynacast, the low layer still encodes whenever anyone watches.
- **Games:** `contentHint: 'motion'` plus `degradationPreference: 'maintain-framerate'`. LiveKit's screen-share
  default (`maintain-resolution`) drops frames, which is the choppiness complaint.
- **Settings must go in `screenShareEncoding`.** livekit-client ignores `videoEncoding` for screen share. Otherwise
  the default is 1080p15 at 2.5 Mbps.
- **Recovery:**
  - Pulse needs its own reconnect loop. LiveKit gives up for good after about 50 s down, and after a 25 s outage the
    picture took 3-61 s to return.
  - A crashed streamer took viewers 21 s to notice. Rust should tell them within about 1 s.
- **Health badge.** `qualityLimitationReason` and LiveKit's connection quality said "fine" during real trouble, so we
  use our own rules ("which link is at fault", network report §7).

### Pain 3: the slowdown that needs a reboot

- **The evidence points below the app.**
  - **Suspects:** kernel memory pool, dwm, the Windows audio service, GPU memory spilling over, heat soak.
  - Quitting Discord wouldn't clear any of these, which fits "only a reboot fixes it".
- **What we build:** a `share:` log line every 5 s with a counter for each cause, and an automatic warning.
- **Before writing any code:** run the `typeperf` triage during a real Discord stream on a friend's PC.
- **A leak we can avoid:** about 20 MB per unpublish/republish cycle (not confirmed upstream). Use `replaceTrack`, or
  keep one long-lived publication.

### Other traps

- **Picker.**
  - WebView2 draws its own picker inside our window, and Pulse can't choose the source.
  - It is unusable below 600 px tall. Pulse's minimum is 560 px, so raise the minimum or grow the window while
    picking.
  - Nothing tells us which window was picked, so a window share may have to fall back to EXCLUDE audio (spike S8).
- **Minimise.**
  - Minimising doesn't set `visibilityState` hidden in Tauri (#10592), so LiveKit's background pause never fires.
    Pulse pauses watched video itself after 5 s minimised.
  - Capture and encode on the streamer should keep running. Ship the anti-throttling flags only if S6 fails.
- **Presets.**
  - The game default is 720p60 at a 5-6 Mbps cap.
  - 1080p60 at about 8 Mbps.
  - Text is 1080p at a low fps (5-15), never 720p.
  - AV1 is opt-in behind a flag for RTX 40+, RX 7000+ and Arc. It saves 15-25% at our bitrates.
- **Stereo game audio** needs a ~5-line Rust SDK patch (rust-sdks #1018). Otherwise it's mono.
- **A/V sync.** Nothing syncs audio and video across the two participants. Audio is estimated 0-80 ms late, inside
  the noticeable limit. Measure it in S9; build the sync controller only if needed.
- **Viewer decode.** WebView2 152 can fall back to software decode (WebView2 #5755). A flag works around it (S7).
- **Server.**
  - Plan on 25-45% of one OCPU for 2 streams × 6 viewers at 1080p60.
  - Resize to 2 OCPU and raise UDP buffers to 16 MiB before launch.
  - Ideas that take the server out of the media path (P2P, mesh, another SFU) save almost nothing at our size. P2P
    would also make the streamer's GPU encode once per viewer.

## Ops to do (needs the owner's OK; prod)

- **Upgrade LiveKit v1.13.7 → v1.13.10, released 2026-10-14.** It fixes embedded TURN relaying to arbitrary
  destinations, including this host's other ports. Prod has embedded TURN enabled. Exposure is low because a valid
  login is needed.
- **Server resources:** 2 OCPU and 16 MiB UDP buffers (above).

## Friend inventory (first pass, 2026-10-15)

- **Windows:** everyone is on Windows 11. That makes Chromium's `windowAudio` (Windows 11 only) usable for window
  shares, so the stream-audio ruling should be re-weighed. A screen share still needs Rust loopback to exclude Pulse.
- **GPUs, all NVIDIA:** RTX 2080 Ti (Turing), 4070 Ti Super (Ada), RTX 5050 (Blackwell), RTX 3050 8 GB (Ampere), and
  probably a 3060 (Ampere).
  - The constrained-baseline NVIDIA skip hits every streamer, so the server `enabled_codecs` fix is mandatory.
  - **AV1:** only the Ada and Blackwell cards encode it. The 2080 Ti can't hardware-decode it either. AV1 stays off
    while any viewer lacks AV1 decode, which in practice means H.264 for v1.
  - The 2080 Ti is the oldest encoder and gets its own S1/S2 run. Turing NVENC is still good for H.264.
  - iGPU: the owner's and their partner's PCs have none. The others are unknown, and there may be an AMD GPU and an AMD CPU in the group.
- **Upload:** roughly 50 Mbps and up. The 70% cap leaves 35 Mbps or more, far above every preset, so upload isn't the
  limit. 1080p60 at about 8 Mbps is affordable for everyone. The limits are viewers' downloads and the route to JHB.
- **Games:** competitive FPS (Overwatch, Apex Legends, CS2), story games (Final Fantasy, Expedition 33, Persona) and
  co-op party games. The competitive titles make game-FPS cost (S2) the deciding test. Anti-cheat: Easy Anti-Cheat
  (Apex), VAC (CS2), Overwatch's own. Spike S2 runs inside at least one of them.
- **Audio software:** SteelSeries Sonar / SteelSeries GG (two PCs) and Turtle Beach software. Others not yet known.
- **The weakest PC so far:** RTX 3050 8 GB (Ampere) with a Ryzen 5 1600 (Zen 1, 6 cores / 12 threads, 2017, no
  iGPU). It's the worst case for the CPU side of Chromium's capture (GPU readback plus CPU conversion every frame) and
  for a software-encode fallback, which would be crippling here. It's the most valuable third machine for S2 and S3
  if its owner is willing.

## Before the spike (no code)

1. **Friend inventory** (still to fill in: iGPU, the other friends' audio software, exact upload speeds):
   - Windows version and build;
   - GPU, and whether the PC also has an iGPU;
   - HDR monitor;
   - upload speed;
   - main games and their anti-cheat;
   - audio software (Sonar, Voicemeeter and so on).
2. **`typeperf` triage** on one friend's PC during a long Discord stream (pain 3).

## The spike

Run the 17 tests in `screenshare-deep-crosscheck.md` §4 with pass bars, on one NVIDIA desktop without an iGPU, one
Intel laptop, AMD if available, and Windows 10 22H2 plus Windows 11. The top three:

- **S1:** hardware H.264 really in use.
- **S2:** game-FPS cost with PresentMon (≤5% at 720p60).
- **S3:** the 2-hour soak.

## Rejected (with reasons in the reports)

- Moving voice to JS.
- Our own Rust capture and encode in v1 (3-6 weeks; it's the v2 path if S1 or S2 fail).
- WebView2 TextureStream (experimental).
- A sidecar process.
- WHIP, Cloudflare or P2P as the main path.
- Neural codecs.
- Game-state streaming.
- Changed-region streaming.
- Viewer frame interpolation (no browser path).
- Kernel and Go tuning of the SFU.
