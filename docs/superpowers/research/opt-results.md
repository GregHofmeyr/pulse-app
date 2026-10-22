# Optimisation run: results

Baseline: Windows 2026-10-15 (two instances, one PC, Sonar mic, ISP RTT 160 ms). Linux numbers: laptop, local
LiveKit and server on loopback (RTT about 0), so Linux join and switch times are not comparable with the Windows
column in absolute terms; only before vs after on the same machine is.

Before = `main` (de4b2ea). After = `optimisation-run` (c81e41c). Both built `--release` with
`--features tauri/custom-protocol` (what `cargo tauri build` ships; without it the binary loads the dev URL).

| Metric | Baseline (Windows) | Linux before | Linux after | Windows after |
|---|---|---|---|---|
| Startup → Ready | 2.4 s | 229 ms (224–239, n=5) | 100 ms (99–103, n=5) | 2.17 s (profile 1); 1.25 s (2nd profile) |
| Join (elapsed_ms) | 2.0 s | 103 ms (83–209, n=4) | 53 ms (51–97, n=4) | 1.07–1.09 s (n=3); 1.50 s (2nd profile) |
| room_connect_ms | 1.6 s | not logged | 45–49 ms | 1.07–1.09 s (n=3); 1.50 s |
| Channel switch | — | 100 ms (91–116, n=4), no pipeline reuse | 41 ms (32–65, n=4), `pipeline_reused=true` | 1.07–1.08 s (n=2), `pipeline_reused=true`, devices 0 ms |
| NetEq buffer / target | 80–100 / 20–40 ms | 34 / 20 ms (n=23) | 30 / 27 ms (n=46) | 49 / 20 ms (n=194); 30 / 20 ms 2nd profile (n=194) |
| recv queue p50 | 20 ms | 0 ms (max 10; n=23) | 10 ms (max 20; n=46) | 20 ms (max 21); 11 ms 2nd profile |
| Strong ns+gate p50/max | 0.57 / 1.9 ms | 2.1 / 2.7 ms per 10 ms (n=46) | 2.1 / 2.4 ms (n=46) | 0.51 / 1.6 ms (n=209) |
| ns attenuation / gate open | — | — (field added in R4) | -7.0 dB / 0 % (n=46; silent mic input) | -49.9 dB / 30 % (n=209, real speech + a show) |
| Linux binary / Windows installer | — / — | 94.4 MB / — | 62.1 MB / || kit zip 74.3 MB |

NetEq verdict (R5): see opt-neteq-buffer.md.

Not like-for-like: recv p50, underrun windows and the NetEq target changed meaning between before and after (after
counts per-SSRC windows, including join/leave windows and up to MAX_ABSENT_WINDOWS after a stream vanishes; underrun
now counts rebuffer frames; recv is sampled at the post-pull low point, which is the 10 ms target), so read those rows
as indicative only, and read underrun together with `adjust`/`mix_busy` and the count of underrun events.

## How the Linux numbers were measured

- Startup → Ready: the real binary, a throwaway profile with a stored session for a local dev server, five
  launches each, alternating before/after; `startup phase="ready"` `since_start_ms` from the profile log. Before
  waits for the webview to ask for the session; after connects the gateway from `setup` (a31a28f), so Ready no
  longer waits for the webview.
- Join, switch, path lines: no GUI. A scratch test (not committed) drives two `VoiceManager`s with `AudioMode::Null`
  (no devices, silent mic) against `just dev-server`-style server + `just dev-livekit`, as two stand-in "profiles".
  Both join one voice channel, a LiveKit CLI bot loops `tone.ogg` into the room for 60 s so each client has a real
  stream to receive, then both switch to a second voice channel, wait 10 s and leave. Two runs per build (n=4
  joins and switches per build, first joins include cold start).
- Path lines: every run logs 32 `voice: path` lines (two clients, 5 s windows); 25 per run fall in the first 60 s,
  before the switch, so 50 per build over the two runs. Only those 50 are used. Both builds had the same run length.
  - Columns buffer / target / recv use the windows that had a stream (buffer present): before 23 (12 + 11), after 46
    (23 + 23). Before has fewer because the stats dropped a stream's values the window it vanished (bot gaps); after
    keeps them for up to three windows (R5), so its windows during bot gaps carry the last values. Medians are
    over those 23 and 46.
  - Underruns, same windows: before 5 of 23 windows (70 ms total, 3.0 ms/window), after 14 of 46 (213 ms total,
    4.6 ms/window). The null "device" is a 10 ms tokio timer, not a real audio callback, so timer jitter produces
    them; not a real-device result, to be judged on Windows (see L7b's note on lowest-fill target).
  - `Strong ns+gate` is the `cpu ... ns+gate Nus` field, present on main too: mean CPU per 10 ms block. p50/max are
    over the 46 of 50 pre-switch windows per build with a value above 500 us; Linux,
    both clients in one process, so not comparable with the Windows figure.
  - `ns attenuation / gate open` (R4) did not exist on main: "-" before. After: median over the 46 of 50 pre-switch windows
    with a numeric value (the first window of each client has "-"), input is silence, so 0 % gate and a few dB are not meaningful.
- Not measured: the `tc netem` run (RTT 80 ms each way). It needs sudo and is pending the owner. Join at a
  160 ms RTT, the headline 2.0 s to about 1.0–1.2 s, is therefore not yet verified.

## Size

| | before | after |
|---|---|---|
| Linux `pulse-app` (release, custom-protocol) | 94.4 MB (94,414,536 B) | 62.1 MB (62,140,720 B) |
| Linux `pulse-app` (release, plain) | 93.7 MB | 61.9 MB |
| UI fonts (woff2, F8) | 583 KB | 146 KB (-437 KB) |
| Windows NSIS installer | — | pending the `release-windows` artifact |

## What changed (35 tasks, 6 phases)

- Server: SIGTERM shutdown (1012 close, no 10 s kill), one write transaction per handler with `synchronous=NORMAL`,
  message send dedupe on (author, nonce), voice presence rebuilt from LiveKit at boot, bigger UDP buffers.
- Client robustness: dead connections noticed in seconds, voice rejoin by itself, mute/deafen re-declared after
  every Ready, startup retries a stored session for 10 s, cpal 0.18 with real-time and MMCSS audio threads,
  24-bit clip fix.
- Diagnostics: `ns`/`gate` on the path line, per-stream jitter-buffer stats with accel/decel/silent, NetEq lag probe
  and its verdict note.
- Voice latency: gate on the newest audio, drift-compensated ~20 ms playout, lock-free receive rings, mic frames
  sent straight from the processor thread, token prefetch on hover, devices opened while the room connects,
  Connected shown before the mic is published, old room closed in the background, devices/processor/noise model
  kept across a channel switch.
- Startup: gateway connected from `setup` with the stored session (Ready replaces `/me`), warm HTTP/2 connection.
- Footprint: raw app state, cached markdown, history trim on leave, meter levels only while shown, denoiser
  skipped while muted, WebView2 low memory target in the background, latin-only fonts, UI sounds shipped once,
  libwebrtc log sink at warning, fat LTO + one codegen unit + strip, one tokio-tungstenite.

## Windows run 2026-10-22 (kit 6d8e451, run 39757553671)

Two profiles on the owner's PC (Windows 11, same network path as the baseline: RTT 170–190 ms, p50 172 ms), reports
MFKAYG, DEQ5B8, VBS657, FSP7WB. Protocol: Task 35 Step 8 minus the network-off step and the Bluetooth join.

- **Join and switch roughly halved:** 2.0 s → about 1.08 s at RTT 172 ms (the plan's 1.0–1.2 s target at 160 ms).
  Join is now almost entirely `room_connect_ms` (token prefetched, devices 42–51 ms, 0 ms on a reused pipeline).
- **Voice path clean:** 0 lost, 0.0 % concealed (one 0.5 % window on the 2nd profile), mix_busy 0. Underruns: 2
  windows, 28 ms total in 18 min on profile 1; none on profile 2. `recv` p50 ≤ 20 ms on a real device (L7b ✓).
- **NetEq buffer** p50 49 ms (profile 1) and 30 ms (profile 2) against a 20 ms target, down from 80–100 ms.
- **Strong while muted (F11 ✓):** the muted profile logged `ns+gate` in only 19 of 196 windows (min 3 µs).
- **MMCSS (R3 ✓):** `pulse-mic` and `pulse-strong` both got "Pro Audio" real-time priority.
- **WebView2 memory target (F2 ✓):** Low on minimise / after background, Normal on restore.
- One `input stream error … underrun or overrun` at the start of each join (`repeated=0`): device start-up, harmless.
- The owner: "testing went very well", per-track audio and keybinds "no hiccups".

## Pending owner checks

- `tc netem` run (80 ms each way) on Linux for the join at 160 ms RTT (largely answered by the Windows run at 172 ms).
- Network-off recovery on Windows (step 5 of the protocol, skipped 2026-10-22).
- Bluetooth headset join (device reopen racing a rejoin).
