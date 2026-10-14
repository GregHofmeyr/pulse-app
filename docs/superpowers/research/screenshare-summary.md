# Screen share: research summary

> **Superseded by `screenshare-deep-summary.md` (2026-10-14).** Kept for history.

Date: 2026-10-13. Input for the screen-share brainstorm and spec. These are not decisions yet. Details and sources
are in the three reports next to this file: capture-encode, render, and server-quality.

## What Greg wants (so far)

Mostly Discord-style "Go Live" for games, sometimes "look at this" for text and code. Pickers for quality and fps.
720p probably as the default.

## The big finding: the video side belongs in WebView2

| Concern | LiveKit Rust SDK (where voice lives) | WebView2 (Chromium) |
|---|---|---|
| Encode on Windows | Software only (VP8/VP9/OpenH264/AV1). Hardware needs the pre-encoded pass-through plus our own Media Foundation/NVENC code, single layer only. | Hardware H.264 via `getDisplayMedia`. Chromium captures with WGC, which is anti-cheat safe. |
| Decode on Windows | Software only (FFmpeg H.264 / libvpx / dav1d), into CPU frames | Hardware (D3D11), zero copy to a `<video>` |
| Getting frames to the UI | Tauri IPC runs at ~50 MB/s against the 187-332 MB/s needed. A local WebSocket into WebGL2 works (1.3 GB/s measured in Chrome). | Native: `<video>`, with fullscreen and HTML overlays for free |
| Linux (dev only) | Works: Rust decode, then WebSocket, then WebGL2 | WebKitGTK has no WebRTC |

So the likely architecture:

- **Video:** `livekit-client` (JS) runs in WebView2 as a second, hidden participant per user (`<uid>#v`), for
  screen-share video only. It publishes via `getDisplayMedia` with hardware H.264 and views with hardware decode.
  The server's webhook already ignores non-user identities.
- **Voice stays in Rust,** exactly as now.
- **Stream audio stays in Rust:** Windows process-loopback capture of everything except Pulse itself (so friends
  don't hear their own voices), published as a `ScreenshareAudio` track. Viewers mix it through our own mixer and
  echo canceller. Keeping it in sync with the video is ours to do.
- **Linux viewing (dev):** the fallback path (Rust decode, then WebSocket, then WebGL2). Linux publishing isn't
  needed for the friends.

## Server and network

- **Load:** LiveKit only forwards packets. 2 streamers × 6 viewers at 1080p60, plus voice, is about 10-11k
  packets/s and 80-100 Mbps out. That's an estimated 30-45% of the single A1 OCPU for LiveKit alone, so **resize to
  the free 2 OCPU / 12 GB first**.
- **Kernel UDP buffers:** raise them to 16 MiB. Today's 208 KB cap is likely smaller than one 1440p keyframe.
- **Egress cost:** about 16 GB per hour for one 1080p60 stream to 6 viewers, so the group's month stays well inside
  Oracle's free 10 TB. Beyond that, Johannesburg costs $0.05/GB. Add a budget alert.
- **Settings to set explicitly:** the Rust SDK's defaults are wrong for this.
  - `auto_subscribe` must go off for video, or every client software-decodes every stream. Viewers opt in with
    "Watch", like Discord.
  - Use dynacast.
  - `adaptive_stream` starts viewers on the LOW layer, so set layers explicitly.
  - The default encoding is capped at 1080p30 / 3 Mbps, with a 3 fps fallback layer.
  - If the codec list is ever trimmed (an idea from the optimisation research), H.264, VP8 and AV1 must stay
    enabled.

## Starting presets (to tune in the spike)

| Preset | Bitrate | Sharer's upload |
|---|---|---|
| 720p60 (default for games) | 4.5 Mbps | ~8 Mbps |
| 1080p30 "sharp text" (keeps resolution, drops frames) | 4 Mbps | ~8 Mbps |
| 1080p60 | 7 Mbps | ~12 Mbps |
| Source, up to 1440p60 | 11 Mbps | ~20 Mbps |

All presets are H.264 with one fallback layer at 15-30 fps, plus an FPS picker (15/30/60). Expected latency is
80-200 ms glass to glass, and 250-600 ms to the first frame.

## Spike (about 2 days) before the spec is final

- **Day 1 (Windows):**
  - a WebView2 viewer and publisher (`getDisplayMedia` on a real game; `lk` bot at 1080p60 H.264);
  - check hardware encode and decode are in use (`getStats` implementation names), CPU, GPU video-engine load,
    fps, freezes and glass-to-glass latency;
  - check the voice connection and the hidden video connection coexist;
  - check the WGC capture border/prompt and Vanguard;
  - check process-loopback audio on Windows 10 22H2.
- **Day 2:** the Linux viewing fallback (Rust, then WebSocket, then WebGL2), plus the real A1 CPU per packet with a
  1080p60 load test.

## Known upstream issues

- LiveKit rust-sdks #1277: each video publish retains about 40 MB until the room closes (only if publishing from
  Rust).
- #1309: C++ exceptions abort the process.
- The pre-encoded frame-dropper freeze is fixed in #1511 (2026-10-12).
- Not exposed in the Rust API: `content_hint`. The Rust doc comments on `DegradationPreference` are swapped (the
  values are right).
