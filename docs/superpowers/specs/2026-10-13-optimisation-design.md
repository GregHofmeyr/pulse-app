# Optimisation pass: design

Date: 2026-10-13. Status: draft for Greg's review.

## Why and how

Pulse works; now make it fast, light and robust where users feel it. The work is based on four research passes (the
voice pipeline, the client footprint, the server/wire/infra, and end-to-end "time to X" journeys), which read every
relevant file plus the locked dependency sources and cite outside sources. The raw reports are kept with the plan.

Greg's constraint: **one big dev run, then one big Windows test**, not many small change-and-test rounds. To still
get before/after numbers on Windows, the pass starts with **instrumentation only** (step 0). It ships in tonight's
Windows build, so tonight's test is the baseline. Everything else is measured against it.

What the research found, in one paragraph: CPU is not the problem anywhere. The voice DSP costs under 0.5 ms per
10 ms, and the server's share of any request is about 1-5 ms. The time users feel is **round trips** (a voice join
is about 11 sequential ones at ~160 ms, and startup is two serial chains), **standing buffers in our audio path**
(about 60-90 ms of the ~225 ms mouth-to-ear is ours to cut), and **robustness** (deploys hard-kill the server,
voice presence is lost on restart, Windows audio threads never get raised priority, and a silent network drop takes
up to 75 s to notice). Size and memory have clear, cheap wins (no release profile at all; the webview never shrinks
when Pulse is in the background).

## Step 0: instrumentation (ships tonight, before any change)

Log lines and counters only, no behaviour change. Everything goes into the existing `voice: path` line or new
one-line timings, so bug reports carry it.

- **Voice join phases:** `token_ms`, `devices_ms`, `room_connect_ms` and `publish_ms` on "voice: joined"; then
  `first_subscribed_ms` and `first_remote_audio_ms` per peer.
- **Startup phases:** setup start → window built → restore start/end → gateway open → Ready (with Ready's size in
  bytes) → first UI paint (the UI reports it via a command).
- **Audio path, per 5 s window:**
  - the mixer queue as min/p50/max instead of a single sample, plus underrun samples;
  - frames per cpal callback (min/max) and device latency from callback timestamps;
  - MAX_LAG drops, send drops and output underruns;
  - the send codec's channels and fmtp, and packets per second (are we encoding stereo? 20 ms packets?);
  - NetEq target and minimum delay, and concealed/accelerated samples.
- **CPU split per stage** (APM, denoiser, gate), as a mean per 5 s in µs.
- **REST:** whether each request reused a connection.

## Step 1: the big dev run

Ordered by what users feel. Every item has a test or a measurement that proves it; nothing here changes the UI's
look.

### A. Robustness (bugs found by the research)

1. **Deploys stop cleanly:** `stop_signal: SIGINT` in compose (Docker's SIGTERM is ignored, so every deploy waits
   10 s and then hard-kills, and clients never get the "server restarting" close).
2. **Voice presence survives a server restart:** rebuild it from LiveKit's room list at startup. Today a deploy
   shows nobody in voice while calls carry on.
3. **Dead connections are noticed in seconds, not up to 75 s:** a heartbeat-ack deadline, and probing when the OS
   says the network is back or the window regains focus. The first reconnect after a healthy session is immediate.
4. **Voice comes back by itself** after LiveKit gives up reconnecting, instead of needing a click.
5. **SQLite:** each handler is one transaction (`BEGIN IMMEDIATE` for read-then-write, which removes the spurious
   "database is locked" 500s), with `synchronous=NORMAL` (WAL-safe; 3-5 fsyncs per message become 0-1).
6. **No double-posted messages** when the outbox retries after a proxy error: a unique (author, nonce) index.
7. **Windows audio threads get real-time priority:** upgrade cpal (0.15 passes the wrong kind of value, so the
   WASAPI threads were never raised) and register our mic and noise-suppression threads with Windows' audio
   scheduler (MMCSS). This is the likeliest cause of glitches while friends game. The output callback stops
   blocking on locks the network tasks also take.

### B. Latency

8. **Voice join, from about 1.75 s to about 1.0-1.2 s:**
   - keep the HTTPS connection warm (HTTP/2, long idle, keepalive), worth about −320 ms after 90 s idle;
   - open devices while connecting to the room;
   - prefetch the voice token on hover;
   - keep LiveKit's runtime alive between calls;
   - show "connected" once the room is connected and publish the mic right after.
9. **Switching channels:** the old room closes in the background (−160 ms, up to −5 s on bad links), and the
   audio pipeline and noise-suppression model are kept across the switch.
10. **Startup:** the gateway starts with the stored session at once, with no separate `/me` round trip first
    (Ready arrives 0.6-1.0 s sooner).
11. **Mouth-to-ear:**
    - the voice gate decides on the undelayed signal, so the denoiser's lookahead doubles as the pre-roll
      (−20 ms with Strong, −10 ms with Standard);
    - drift-compensated playout with a ~20 ms target replaces the 40-55 ms standing queue (−15-35 ms, and no
      drift clicks);
    - mic frames go to WebRTC straight from the processor thread (one hop and three copies fewer).
    - Expected total: about 225 → 170-180 ms on a typical wired headset.

### C. Footprint

12. **Release profile:** LTO, one codegen unit, strip. Expect a few MB off the exe and about 25 MB off the Linux
    binary.
13. **Webview memory:** WebView2 drops to its low memory target when Pulse is minimised or in the background
    (another Tauri app measured 122 → 36 MB).
14. **UI work per event:**
    - the store stops deep-proxying (`$state.raw`; the reducer is already immutable);
    - rendered markdown is cached per message, so a typing indicator no longer re-renders every message;
    - long histories are trimmed.
15. **In-call:** voice levels are sent to the UI only while something shows them (now 10 per second for the whole
    call), and libwebrtc's logging is filtered at the source if it turns out to be chatty.
16. **Small items:** latin-only fonts (−500 KB), sounds shipped once, no 1 s timer in every open channel, skip the
    denoiser while muted, and one duplicate crate fewer.

## Not in this pass (decisions for later)

- **Download the Strong model on first use:** the installer shrinks by about 35% (22.5 → ~14.5 MB), but Strong then
  needs network once. Greg's call.
- **Low-latency DeepFilterNet as an option:** −20 ms, but 3.5× the CPU and +87 MB. Possibly offered as "Strong (low
  delay)".
- **10 ms Opus packets:** −10 ms and maybe a smaller jitter buffer, but it needs a LiveKit SDK fork and doubles
  packets through the server.
- **Lower WASAPI periods (IAudioClient3):** −5-15 ms per side on some hardware; cpal doesn't support it.
- **Server:**
  - `use_ice_lite` on LiveKit, as an A/B test (Oracle's 1:1 NAT makes it uncertain);
  - Ready compression and gateway resume (pays off at 40+ users);
  - raising the kernel UDP buffers on prod (a one-line sysctl; needs Greg's OK on prod);
  - resizing to the free 2 OCPU / 12 GB in the OCI console.
- **Voice quality (clarity, codec settings, AEC tuning)** gets its own research pass later, as agreed.

## Testing

- Every change has a unit or integration test where behaviour changes (reconnect timing, transactions, nonce
  dedupe, playout drift, gate timing, store slices), plus before/after numbers from step 0's logging. Locally the
  numbers come from scripted runs: two clients, the LiveKit bot, `tc netem` for 80/160 ms RTT, and `strace` for
  fsyncs.
- Greg's one Windows test after the dev run: a dedicated 60 s call with both instances and mics open, a channel
  switch, minimise for a minute, then a report from each instance. Compared against tonight's baseline reports.
