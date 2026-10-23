# Screen share spike: results

Date: 2026-10-22 (runs), written 2026-10-23. Build: `screenshare-spike` c1405a2, Windows kit, `livekit-client`
2.22.4, prod LiveKit with the strict H.264 codec list (`just spike-codecs on`).

Setup: one PC, the owner's (RTX 2080 Ti, driver 32.0.16.1088, no iGPU, Windows 11 build 26200). It ran **both**
instances, the streamer (profile a) and the viewer (profile b), so both of them shared one GPU, one CPU and one
audio stack. The owner was also in a Teams call. WebView2 reports `Edg/155.0.0.0`; the reduced user agent hides the
build number. Voice RTT to the server is 173 ms. The 4070 Ti Super second PC wasn't used.

Reports: 1WQJ6J (streamer, S1/S2/S21, a busy YouTube video), 2724NY (viewer, S0/S4/S7/S12), 8QR53M (streamer, after
the game and audio tests), 01PFCV (viewer, same window). These four are the only spike reports. The earlier reports
on 2026-10-22 belong to the optimisation run (opt-results.md).

Verdict: **the WebView2 path holds.** Hardware H.264 encode works on NVIDIA, so the native capture/encode path stays
out of v1. Two things fail as specified, and both have fixes: watching from a cold start takes too long (pre-connect
`#v`), and the stream-audio exclude has to cover every Pulse process. The game-FPS question (S2) is still open,
because one PC can't answer it.

## Results

| Test | PC and GPU | WebView2 | Measured | Pass bar | Result | What it changes |
|---|---|---|---|---|---|---|
| S0 setup | 2080 Ti, 32.0.16.1088 | 155 | GPU and driver typed in; UA logged | record both | done | — |
| S1 hardware encode | 2080 Ti | 155 | `MediaFoundationVideoEncodeAccelerator (NVIDIA H.264 Encoder MFT)` on both layers, `powerEfficientEncoder: true`, fmtp `profile-level-id=42001f`. Encode ms per frame (full layer): 7.1 at 1448×1440 (YouTube), 8.0 at 1920×1080 (game); low layer 2.1 and 3.8. `qualityLimitationReason: none` throughout. Task Manager Video Encode 14-16 % at 1440p | MF encoder named; power-efficient; fmtp not `42e01f`; voice unchanged | **PASS** (strict codec list only; the default list and the voice RED/RTX check were not logged) | Native path stays out of v1. Encoder check: §3.2 |
| S2 game-FPS cost | 2080 Ti, shared by both instances | 155 | No PresentMon CSVs. Game window capture delivered 38 fps median (p10 33, p90 45) against a 60 fps target | ≤5 % avg and 1 % low at 720p60 | **inconclusive** | Rerun on two PCs with PresentMon |
| S4 viewer, clean link | 2080 Ti (same PC) | 155 | 1440p window: 55 fps median (p10 53), 7 freezes, 1.98 s in total over 6 min; game: 37 fps median (the source rate), 0 freezes. 0 packets lost, 0 NACK, 0 PLI. Jitter buffer 27-32 ms per frame. Started on the 362×360 low layer, then moved up | no freeze ≥ 2.5 s at 1-5 % loss | **PASS on a clean link**; the impairment profiles (clumsy) were not run | None yet |
| S7 viewer decode | 2080 Ti | 155 | `decoderImplementation` and `powerEfficientDecoder` were absent even after the `getUserMedia` unlock. Task Manager Video Decode about 40 % (29 % at the start) at 1440p55 | D3D11 decoder, power-efficient; viewer CPU < 5 % of 6 cores | **can't be measured in-app**; decode is on the GPU by Task Manager; CPU not recorded | §4.3: no in-app decode check |
| S8 picker | 2080 Ti | 155 | Window share 1920×1080, `displaySurface: window`, picked in 3.8-3.9 s. Track label `window:<id>:0`, not the window title, so `spike_window_pid` found no pid | label matches a window | **FAIL (gap 4)**: fall back to EXCLUDE | §3.3 already covers it; the exclude set grows (below) |
| S9 stream audio | 2080 Ti, Win 11 | 155 | EXCLUDE mode (no pid, see S8). Feedback: the second Pulse instance's playback was captured and looped back. A/V skew from the flash-and-beep pairs: **+32 and +68 ms** (audio late), median 50 ms, n = 2. The onset detector paired only 2 of 12 flashes. Stereo tone not run; INCLUDE not run; 2 % loss not run | no echo; skew 0 to +125 ms; L≠R | skew **PASS** (thin sample); echo **FAIL** (another Pulse); stereo not run | §2 sync: none needed in v1. §3.3: exclude every Pulse process |
| S12 lifecycle (cold watch) | 2080 Ti | 155 | Stream-room connect 1304-1563 ms (n = 5); subscribe to first painted frame 602, 901 and 1548 ms (n = 3). Cold Watch to first frame 1.95-2.85 s | ≤ 1.5 s cold | **FAIL cold**; with `#v` already connected, 0.6-1.5 s | §2: pre-connect `#v` with voice |
| S12 reconnect ×5 | — | — | not run | no orphaned participant | **deferred** | — |
| S18 pop-out | — | — | not run | blip about 1 s | **deferred** | — |
| S21 step-down signals | 2080 Ti | 155 | `lowFps` fired in 19 of 169 samples on YouTube (the start, and a paused stretch); in 145 of 145 on the game and flash-test share; `encodeHeavy` once; `cpuLimited` never | signals fire before visible stutter, and only then | **FAIL as specified**: fires on still and source-limited content | §6.2 strain against the captured rate |
| S3, S5, S6, S10, S13, S19, S20 | — | — | not run | — | **not run** | Next session, two PCs |

## Notes on the numbers

- **Encode ms is cumulative.** The spike sampler logs `totalEncodeTime / framesEncoded` since publish, not per
  interval, so 7-8 ms is a stream average and hides spikes. The first cut's `stats.ts` uses per-interval deltas.
- **One layer started in software.** For the first 12 samples (about 25 s) of the 1440p share, the adapter read
  `SimulcastEncoderAdapter (OpenH264, MediaFoundation… (NVIDIA…))`. Then both layers moved to the NVIDIA MFT and stayed
  there. libwebrtc lists the lowest stream first, so this was most likely the 362×360 low layer, whose odd size came
  from a near-square 1448×1440 window. The 1080p share was hardware on both layers from the first sample.
- **Negotiated profile.** The answer used Baseline `42001f`, not the Main (`4d001f`) that §3.2 prefers. It is still
  hardware, and that is what matters. Main wasn't forced.
- **Bitrates.** YouTube at 1440p: target 9.1 Mbps, sent 1.8 Mbps median, 13.2 Mbps max. Game at 1080p60: target
  8 Mbps, sent 5.7 Mbps median, 9.1 Mbps max. Low layer 375-500 kbps target. Nothing here argues for changing §3.2
  before S2 and the impairment runs.
- **Game capture fps.** 33-43 fps from a 60 fps target, with no `qualityLimitationReason` and encode well under the
  frame interval. So the capture side or the game itself set the rate, not the encoder. One GPU was rendering the
  game, encoding, decoding and painting a second WebView2, so this says nothing yet about a real streamer's PC.
- **Still content.** The A/V flash page dropped capture to about 2 fps (p90 25). The viewer counted 31 freezes,
  23 s in total, in about a minute. That is WebRTC's freeze definition meeting a picture that doesn't change, not a
  stall: 0 loss, 0 PLI.
- **Watch timing** at 173 ms RTT: connect is about 1.3-1.6 s (ticket, signalling, ICE, DTLS), then 0.6-1.5 s to the
  first frame (subscribe, keyframe, decode, paint).
- **A/V skew sample.** Flashes came every 2 s, but the beep onset detector fired on only 2 flashes within ±500 ms.
  The other beeps were missed or swamped (feedback and the Teams call were in the same capture). Two beeps 300 ms
  apart at 20:26:22 look like one beep plus its loop. Treat +32/+68 ms as "inside the bar", not as a distribution;
  p90 needs a clean rerun.

## Side findings (not screen share)

- **Keybind hooks.** The keybinds v2 watchdog logged "hooks stopped seeing input (dropped by Windows?); reinstalling"
  and reinstalled them 7 times in about 45 minutes of streaming and watching. Six of those were on the viewer
  instance; once (19:48:14) both instances did it in the same second. It also happened once, at 17:43, during the
  optimisation run with no streaming. That fits Windows removing a low-level hook whose callback blew
  `LowLevelHooksTimeout` under load. The watchdog recovered every time.
- **Voice buffer crept up.** On the viewer, NetEq `buffer` went from 20 to 70 ms over the first 4 minutes of
  watching (target 20-40), then stayed at 70 with accel 0 and 0 % concealed. The streamer instance showed the same
  thing (70-80 ms). This matches the send-stall mechanism in opt-neteq-buffer.md: a stall on a loaded PC raises the
  buffer, and it never drains. That is about 50 ms of extra voice latency while sharing on one PC. Worth checking on
  two PCs before blaming streaming for it.

## Spec changes made

In `docs/superpowers/specs/2026-10-15-screenshare-design.md`. Each one says whether the first cut on
`screenshare-v1` already does it.
- **§2:** the A/V sync measurement; no sync controller in v1 (the first cut only logs it).
- **§2:** `#v` connects when joining voice, not on first use (S12). The first cut connects on first use. Follow-up.
- **§3.2:** the encoder check judges the full layer, not the adapter string as a whole; Baseline `42001f` is
  acceptable.
- **§3.3:** EXCLUDE covers every Pulse process, not just our own tree. The first cut's `plan.rs` does this
  (per-session INCLUDE when another Pulse exists). The track label never carries the window title (S8).
- **§4.3:** no in-app hardware-decode check; Chromium hides `decoderImplementation`.
- **§6.2 and §6.3:** strain and viewer "frozen" ignore still content. The first cut's `fpsNote 'still'` does this for
  the overlay; step-down isn't in the first cut.

## Still to run (two PCs, the 4070 Ti Super as the viewer)

- S2 with PresentMon, 3 × 60 s per configuration;
- S4 with the clumsy profiles;
- S9 INCLUDE, stereo and skew at 2 % loss, with no other call open;
- S12 reconnect and renderer kill;
- S18;
- S3 (2 h);
- S5, S6, S13, S19, S20, S10.
