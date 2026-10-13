# Pulse client voice pipeline: optimisation findings

Scope: `client/src-tauri/src/voice/*`, `spikes/voice/FINDINGS.md`, and the dependency sources actually locked:
livekit 0.9.3, libwebrtc 0.3.50, webrtc-sys 0.3.47, cpal 0.15.3, rubato 5.0.1, nnnoiseless 0.5.2,
libDF @ d375b2d with tract 0.21.4. Everything here was read-only. Nothing was compiled.
Date: 2026-10-13.

## TL;DR

The pipeline is already cheap on CPU. Strong mean is 459 µs per 10 ms (FINDINGS), RNNoise is 37 µs, and our own
allocations and locks are in the microseconds. **The real wins are latency (about 60–90 ms one way is ours to cut)
and thread-scheduling robustness on Windows.** Allocation churn is a determinism and tidiness issue, not a CPU one.

Biggest levers, in order:
1. **The gate pre-roll is a fixed 20 ms delay on every sample.** With Strong, DFN's lookahead already gives us
   that pre-roll for free if the gate decides on the undelayed signal.
2. **The mixer's standing queue** (target 40 ms + 15 ms slack, and nothing rebuffers on underrun) can run at
   about 20 ms with drift-compensated playout.
3. **cpal 0.15.3 never actually raises the WASAPI thread priority** (it passes a thread *ID* as a *handle*), and our
   processor and Strong threads run at normal priority. When friends game while talking, this is the likeliest
   cause of glitches and MAX_LAG drops. cpal ≥ 0.16 fixes the cpal side.
4. **Strong (DFN3 normal) costs +20 ms over Standard.** The only lower-latency DFN is the LL model: +10 ms total,
   but 3.5× the CPU and +87 MB.
5. **Opus 20 ms packets** are the WebRTC default (26.5 ms algorithmic). 10 ms ptime needs SDP munging in a livekit
   fork, so it's a later item.
6. **Measure first:** the stereo=1 munging in single-PC mode might make our mono mic encode as *stereo* Opus.
   Check the send codec's `channels`/`sdpFmtpLine`.

---

## 1. Latency budget (one way, mouth → ear)

Typical case: Windows 10/11, wired or USB headset, devices at 48 kHz, Strong, WASAPI shared mode with cpal
defaults. "Ours?" means the code or config is in our hands.

| # | Stage | Where | Estimate | Ours? | Notes |
|---|---|---|---|---|---|
| 1 | ADC + driver + audio engine (capture) | Windows | 10–20 ms (BT HFP 30–60+, Sonar +? ) | partly | Default shared-mode period is 10 ms. IAudioClient3 can drop it to 2.67 ms on the inbox HDAudio driver (MS doc). |
| 2 | Callback → 10 ms block chunking | `devices.rs:343-360` | 0–10 ms (≈0 when the period is 480 frames) | yes | Chunks at the device rate. |
| 3 | Processor queue | `processor.rs:299-305` | ≈0 standing, 60 ms cap | yes | Age-based trim only. |
| 4 | Capture resample (non-48k only) | `resampler.rs:21` | 5 ms (rubato `output_delay = fft_size_out/2` = 240 frames) | yes | 0 at 48 kHz. 44.1k and 16k both give fft_out 480. |
| 5 | APM (HPF + AEC3) | `processor.rs:359-372` | <1 ms | — | AEC3 adds no algorithmic delay at the 10 ms API. |
| 6 | Denoiser | `denoise.rs` | **Strong 30 ms**, Standard 10 ms, Off 0 | yes (choice) | DFN3: 2-frame lookahead (20 ms) + 10 ms OLA (FINDINGS). |
| 7 | **Gate pre-roll delay line** | `gate.rs:20,100` | **20 ms always** | **yes** | Constant delay, even while the gate is open. |
| 8 | mic_tx → tokio sender → `capture_frame` | `mod.rs:456-515` | <1 ms (more under runtime load) | yes | Extra thread hop. |
| 9 | Opus framing + lookahead | libwebrtc | **26.5 ms** (20 ms frame + 6.5 ms) | config | The WebRTC default frame is 20 ms with no `a=ptime`. |
| 10 | Pacer / UDP send | libwebrtc | ~0–2 ms | no | |
| 11 | Network up + SFU + down (JNB) | ISP | ~10–40 ms (≈ the `rtt` in our path log) | no | |
| 12 | NetEq jitter buffer | libwebrtc | 20–80 ms (our `buffer_ms` log) | barely | Not configurable through livekit 0.9.3. `ForcePlayoutDelay` is video-only. |
| 13 | SyntheticAudioDevice 10 ms pull phase | `webrtc-sys synthetic_audio_device.cpp:55-71` | 0–10 ms (avg 5) | no | A hidden ADM timer thread drives NetEq. See H4. |
| 14 | NativeAudioStream queue | libwebrtc (10 frames max) | ≈0 | — | We drain it immediately. |
| 15 | **Mixer queue** | `mixer.rs:35-38,155-178` | **0–55 ms (target 40, trims only above 55)** | **yes** | |
| 16 | Playout resample (non-48k) + fifo | `playout.rs:28-40` | 5 ms + 0–10 ms fifo | yes | 0 at 48 kHz. |
| 17 | Engine + DAC (render) | Windows | 20–30 ms shared (cpal fills the *whole* free buffer each event); BT A2DP 100–200 ms | partly | IAudioClient3 / period-sized fills. |

**Typical total:** about 15 + 30 + 20 + 26.5 + 25 + 40 + 5 + 40 + 25 ≈ **225 ms**. That's 175 ms with Standard
(DFN → RNNoise), and BT A2DP output adds 100+ ms.

**Ours to cut, without hurting quality:**

| Item | Gain | Cost |
|---|---|---|
| Gate pre-roll (Strong) | −20 ms | free (H1) |
| Gate pre-roll (Standard/Off) | −10 ms | small |
| Mixer target 40 → ~20 ms with drift compensation | −15–30 ms | medium (H2) |
| Opus ptime 10 ms | −10 ms (+ maybe −10 ms of NetEq) | livekit fork (M5) |
| IAudioClient3 small periods | −5–15 ms per side | cpal fork or new backend (M6) |
| Resampler sub-chunks (non-48k only) | −3 ms per side | trivial (L4) |
| DFN LL model | −20 ms | +1.1 ms CPU per 10 ms, +87 MB (M1) |

---

## 2. Findings ranked by impact

### HIGH

#### H1. The gate pre-roll adds a constant 20 ms. With Strong it's redundant
- **Where:** `gate.rs:20` (`PREROLL_BLOCKS = 2`), `gate.rs:96-110` (delay line: every block goes in, the block
  from 20 ms ago comes out), `processor.rs:97-110` (MicChain calls the denoiser, then the gate).
- **Current cost:** +20 ms one-way latency on *every* sample, whether or not the gate is opening. There's also one
  `Vec` alloc per block (`block.to_vec()`, `gate.rs:100`).
- **Why it's redundant:** in Strong, `DeepFilter::process` (`denoise.rs:119-139`) computes the RNNoise probability
  on the *current raw* block, while the DFN output is 30 ms old (2-frame lookahead + OLA). The probability already
  looks ahead of the audio by about 20 ms relative to RNNoise and 30 ms relative to the input. But the gate's
  *level* test uses `rms(block)` of the delayed DFN output (`gate.rs:96`), so the gate can't open until the delayed
  audio is loud. Only then does the 20 ms delay line restore the onset.
- **Proposed change:**
  - **Strong:** decide `qualifies()` on undelayed features. Use the RNNoise prob plus the RMS of RNNoise's own
    cleaned output, which `Rnnoise::probability` already computes into `outb` and throws away
    (`denoise.rs:58-64`), or the raw input × gain. Then apply the gain envelope to the DFN output with
    `PREROLL_BLOCKS = 0`. The DFN lookahead becomes the pre-roll.
  - **Standard:** RNNoise output lags its input by about 10 ms. Decide on the raw-input level and keep 1 block of
    delay, which saves 10 ms.
  - **Off:** keep 2 blocks, or 1 if testing shows first syllables survive.
- **Expected gain:** −20 ms with Strong (the default), −10 ms with Standard. CPU is the same or slightly lower.
- **Risk:** low to medium.
  - The floor learning and the `above_floor` test would move to a different signal (pre-DFN level includes noise
    DFN would remove). Mitigation: use RNNoise-cleaned RMS, which is what Standard users get today anyway.
  - Hold and fade timing shift by 20–30 ms relative to the audio. Hold is 450 ms, so word endings stay safe.
  - Re-run the gate tests and add a "Strong onset not clipped" test with the speech fixture.
- **Measure:** a click/impulse test through `MicChain` (the processor test harness): the time from the input
  impulse to the output impulse. Expect 50 ms → 30 ms for Strong. Ear test: speak "t-t-t" onsets with the gate in
  auto and confirm nothing is clipped. Log the gate-open lag (first block with prob ≥ 0.6 vs first sent non-zero
  block).
- **Sources:** code. DFN lookahead per the DeepFilterNet papers (2-frame lookahead,
  https://arxiv.org/pdf/2110.05588, https://arxiv.org/pdf/2305.08227) and FINDINGS (30 ms measured).

#### H2. Mixer standing delay of up to 55 ms, with no rebuffer and no drift compensation
- **Where:** `mixer.rs:35-38` (`TARGET_MS = 40`, `SLACK_MS = 15`), `mixer.rs:155-178` (trims only when the
  1-second minimum exceeds target + slack; an empty queue just yields zeros).
- **Current cost:** the queue settles wherever the startup phase leaves it. Anything up to 55 ms is never trimmed.
  On the other side, if the output device clock runs *faster* than the system clock that paces
  SyntheticAudioDevice (H4), the queue drains to 0 and then underruns a few samples at a time, forever. Those are
  tiny zero-insert clicks, because nothing re-primes it. 100 ppm means 4.8 samples/s short.
- **Proposed change:**
  - Make the playout rate adaptive: a small ratio trim (±0.1–0.5 %) driven by the queue level vs the target.
    Options are rubato's async resampler with `set_resample_ratio_relative`, or simply always running Playout
    through a resampler with a tiny ratio adjust, even at 48 kHz.
  - Lower the target to about 20 ms (2 frames: one 10 ms ADM pull + one ~10 ms WASAPI period).
  - Keep the hard trim only for big excursions such as device reopens.
  - Optionally re-prime on underrun (insert 10 ms of silence once, rather than dribbling).
- **Precedent:** Zed's livekit client (same SDK, cpal) moved to an i16 ring per source with
  `target_fill = 2 frames` (20 ms), `capacity = 10 frames`, and gradual catch-up draining
  (commit 5713dbf, https://github.com/zed-industries/zed/commit/5713dbfd39a200cd30400091e37717476468dd63).
- **Expected gain:** −15 to −35 ms of receive latency. Removes the drift underrun clicks.
- **Risk:** medium.
  - A lower target needs callback-cadence data (see the measure-first list). BT outputs with 20–40 ms periods need
    a per-device target, which could be adaptive: target = p99 of (pull-size + push-burst) over the last 10 s.
  - The ratio trim is inaudible at ≤0.5 %.
- **Measure:** we already log `recv_ms` (the queue at a 5 s sample). Add:
  - the per-peer queue min, max and p50 over each 5 s window;
  - an underrun counter (samples zero-filled in `mix_into` while the peer exists);
  - trims per minute.
  - Compare 40 vs 20 ms in a 30-minute call with each friend's device.
- **Sources:** code. Zed commit above.

#### H3. Audio threads at normal priority on Windows (cpal 0.15.3 bug, plus our own threads)
- **Where:** `~/.cargo/registry/.../cpal-0.15.3/src/host/wasapi/stream.rs:324-333`. `boost_current_thread_priority`
  calls `SetThreadPriority(HANDLE(GetCurrentThreadId()), TIME_CRITICAL)`. A thread ID isn't a handle, so the call
  fails silently and the WASAPI callback threads stay at normal priority. Our own threads `pulse-mic`
  (`processor.rs:258`), `pulse-strong` (`processor.rs:208`), the tokio sender/rx tasks (`mod.rs:456`, `mod.rs:545`)
  and WebRTC's SyntheticAudioDevice task queue (NORMAL priority) are all normal too.
- **Current cost:** no CPU cost. The cost is glitches under load (a game in the foreground):
  - the WASAPI callback misses its event → capture overrun or render underrun;
  - the processor falls behind → MAX_LAG drops (`processor.rs:119`);
  - the mixer lock gets held by a preempted tokio worker → the output callback blocks (priority inversion; see M3).
- **Proposed change:**
  - Upgrade to cpal ≥ 0.16. 0.16.0 (2025-06-07) "Set realtime priority for stream threads". Current master uses
    `audio_thread_priority::promote_current_thread_to_real_time` (MMCSS) behind the `realtime` feature, with a
    correct `GetCurrentThread()` fallback.
  - Promote `pulse-mic` and `pulse-strong` the same way (the `audio_thread_priority` crate, as used by
    Firefox/cubeb), or at least `THREAD_PRIORITY_HIGHEST`.
  - Move the send path off tokio (M4).
  - Note: cpal 0.18 changed default-device rerouting and device naming, which interacts with our own following
    logic in `mod.rs` and `devices.rs`. Upgrade deliberately (0.16 or 0.17 is the smaller step).
- **Expected gain:** fewer xruns and drops under CPU contention, and it lets us safely shrink the buffers in H2/M6.
  No direct latency gain.
- **Risk:** low. MMCSS is designed for this. A runaway realtime thread could starve the system, but MMCSS reserves
  20 % for non-RT work.
- **Measure:** run a CPU stress (e.g. a game, or `prime95` on all cores) during a call. Count the
  "mic audio backed up, dropped" warnings (`mod.rs:497`), the processor `trim_backlog` return values (add a
  counter), cpal stream errors, and output underruns (H2 counter). Compare before and after. On Windows, also use
  WPA/ETW traces of the audio threads' ready-to-run latency.
- **Sources:** cpal source (above). cpal CHANGELOG
  (https://raw.githubusercontent.com/RustAudio/cpal/master/CHANGELOG.md: 0.16.0 realtime priority, 0.18.0
  timestamps include hardware latency, 0.18.2 xrun reporting). cpal master `stream.rs` (fetched 2026-10-13). MS
  Low-Latency Audio doc recommends MMCSS / RT work queues "Audio"/"Pro Audio"
  (https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/low-latency-audio).

#### H4. A hidden ADM timer thread is the receive clock: a second clock domain we have to buffer against
- **Where:**
  - `webrtc-sys-0.3.47/src/synthetic_audio_device.cpp:45-71`: when not using PlatformAudio, the AdmProxy runs a
    NORMAL-priority task queue that calls `NeedMorePlayData` every 10 ms on the system clock. That pull runs NetEq
    for every remote stream, mixes them in WebRTC's AudioMixer (discarded), and on the way delivers each stream's
    decoded 10 ms to our `NativeAudioSink`.
  - `webrtc-sys .../peer_connection_factory.cpp:193-195`: the factory also builds WebRTC's own APM; see M7.
- **Current cost:**
  - CPU: NetEq decode is unavoidable. The extra mix of all streams in AudioMixerImpl (plus the voice engine's
    reverse-stream APM call) is wasted but small: about one mix of N × 480 samples per 10 ms.
  - Latency: 0–10 ms phase, plus whatever *cadence jitter* the timer has on Windows. If WebRTC's Windows task
    queue fires at 15.6 ms timer granularity, frames arrive in bursts of 1–2 and our mixer must hold extra.
- **Proposed change:** we can't remove it without forking webrtc-sys. A fork could pull NetEq from our cpal
  callback via a custom ADM that we drive, which is the browser model and has no second clock. What we can do:
  - **measure** the NativeAudioStream inter-arrival times on Windows (histogram) and size the H2 target from it;
  - make the drift compensation in H2 follow *this* clock vs the device clock.
- **Expected gain:** it's what makes H2 safe. A fork-level ADM is about −10 ms and one less thread, but it's
  high effort.
- **Risk:** none for measuring. A fork is high risk.
- **Measure:** in the rx task (`mod.rs:545-553`), record the `Instant` deltas between frames per peer. Log
  p50/p95/p99/max every 5 s. Expect 10 ms p50. Bursts (0 ms followed by 20 ms) mean timer quantisation.
- **Sources:** webrtc-sys source (as cited).

### MEDIUM

#### M1. Strong = DFN3 normal: 30 ms algorithmic delay. What exists to reduce it
- **Where:** `denoise.rs:93-115`, `Cargo.toml` feature `dfn-ll` (`deep_filter/default-model-ll`). libDF
  `tract.rs:309-313`: `lookahead = max(conv_lookahead, df_lookahead)`.
- **Current:** 459 µs mean / 768 µs p99 per 10 ms, 33.6 MB RSS, 30 ms delay (FINDINGS, release build, Greg's
  laptop).
- **Options:**
  - (a) **The LL model** is already wired behind `dfn-ll`: 10 ms delay, 1.59 ms mean / 3.05 ms p99, 121 MB. It
    saves 20 ms for +1.1 ms CPU per 10 ms (~11 % of one core) and +87 MB. Offer it as a "Strong (low latency)"
    choice, not the default.
  - (b) **Standard (RNNoise):** 10 ms delay, 37 µs. Weaker on clicks (FINDINGS test: Standard only "reduces"
    keyboard clicks).
  - (c) **RNNoise 0.2 (xiph, April 2024)** retrained the model and added AVX2/SSE4.1 with runtime detection.
    nnnoiseless 0.5 is a Rust port of the older code (unverified whether it carries 0.2 weights). A retrained
    Standard might close the gap to Strong on noise at 10 ms delay. Worth an A/B, through C rnnoise or by loading
    0.2 weights if nnnoiseless can.
  - (d) **No runtime knob in libDF cuts the normal model's lookahead.** It's baked into the model (conv padding +
    DF order). `RuntimeParams` (`min_db_thresh` -10, `max_db_erb_thresh` 30, `max_db_df_thresh` 20, `atten_lim`)
    only skip work, not delay. See M2.
- **Expected gain:** up to −20 ms (with (a) or (b)).
- **Risk:** quality/CPU trade-off. This is a product decision. Keep Strong as the default unless friends prefer
  lower latency.
- **Measure:** `cargo test --release --test denoise_bench` exists. Run it on friends' actual CPUs (the weakest PC
  matters), plus the processor's OverloadDetector stats in logs.
- **Sources:** FINDINGS. libDF source. Phoronix on RNNoise 0.2
  (https://www.phoronix.com/news/RNNoise-0.2-Released). DFN papers (above).

#### M2. Denoise CPU spent while muted, and while the gate is closed for long stretches
- **Where:** `processor.rs:97-110`. `MicChain::process` runs the denoiser (and APM, line 364) *before* checking
  `mic_open`. When muted or deafened, Strong still costs 459 µs per 10 ms (4.6 % of a core).
- **Proposed change:**
  1. **Muted/deafened:** skip the denoiser, and compute the meter on the post-APM signal. Keep APM running (AEC
     must stay converged so there's no echo burst on unmute; ~free). On unmute, either keep feeding DFN a few
     blocks before un-gating or accept a ~50 ms warm-up. The gate is reset on unmute anyway, and the H1 pre-roll
     lives in the DFN lookahead.
  2. **(Optional, needs an ear test) "lazy Strong":** while the gate has been closed for > 1 s *and* the RNNoise
     prob < CLOSE_PROB, run RNNoise only. Wake DFN when prob ≥ CLOSE_PROB. The risk is stale GRU state at
     utterance onsets. libDF already skips both decoders when lsnr < −10 dB (`tract.rs apply_stages`), so the
     saving is the encoder + STFT share. Measure that split with libDF's `timings` feature before bothering.
- **Expected gain:** (1) ~4.6 % of a core → ~0.5 % while muted, which matters for the "sit muted in a channel"
  pattern. (2) Perhaps 50–80 % of the Strong cost in typical group calls where each person talks 10–30 % of the
  time.
- **Risk:** (1) low. (2) medium (onset artifacts).
- **Measure:** process CPU % (Windows: Task Manager → Details, or `typeperf "\Process(pulse-app)\% Processor Time"`)
  in a call, muted vs unmuted, Strong. Log per-block `chain.process` time split by gate state.

#### M3. Locks in the real-time output callback
- **Where:** `devices.rs:363-405` `on_output` takes 4 mutexes per callback: `controls`, `mixer` (contended by
  every rx task's `push` at `mod.rs:551`, the 100 ms levels task, and the 5 s stats task), `spk_meter`, and
  `echo_ref` (contended by `feed_echo_reference`, `devices.rs:409-425`, but only for a drain).
- **Current cost:** uncontended std Mutex is ~20 ns, so the CPU is irrelevant. The risk is that an rx task on a
  tokio worker is preempted while holding the mixer lock. The output callback then blocks for a scheduler quantum
  (up to ~15 ms on Windows) → an underrun click. H3 makes this worse in relative terms: a high-priority callback
  waits on a normal-priority holder.
- **Proposed change:**
  - Per-peer lock-free SPSC rings (`rtrb`, already a transitive dependency of libwebrtc). The rx task is the
    producer, the output callback the consumer. Peer add/remove goes through an `arc_swap`, or a command ring the
    callback drains.
  - Make `controls` an `AtomicBool` (playout_on).
  - Meters: atomics, or accumulate locally in the callback and publish with `try_lock`.
  - Echo ref: an SPSC ring too.
  - Cheap interim: `try_lock()` the mixer in the callback and output silence or repeat on failure, counting
    occurrences.
- **Expected gain:** removes a class of rare clicks under load.
- **Risk:** medium (refactor). The interim `try_lock` is low risk.
- **Measure:** a counter of `try_lock` failures (interim) or blocked-time histograms. Underrun counts during a
  stress test.

#### M4. Send path: an extra thread hop + tokio dependency + redundant copies
- **Where:**
  - `processor.rs:395-396`: i16 `Vec` per block → unbounded tokio channel.
  - `mod.rs:456-515`: the async task. `ToInternal::process` *always* takes the `rate == INTERNAL_RATE` branch
    (the processor only ever emits 48 k), so `resampler.rs:70` `chunk.to_vec()` is a pure copy + alloc. Then
    `ready.extend`, then `ready.drain(..).collect()` (`mod.rs:510`), another alloc. That's 3 allocs + 3 copies per
    10 ms. Then `capture_frame` (fast path, synchronous: `audio_source.rs` queue 0 → C++ `OnData` → WebRTC encoder
    task queue).
- **Current cost:** about 3 allocs/10 ms (negligible CPU), plus tokio scheduling latency. That's µs normally, but
  ms when Tauri's runtime is busy (IPC bursts, device probes, bug-report gathering). `GapFill` relies on a 10 ms
  tokio interval, which on Windows may tick at timer granularity.
- **Proposed change:** call `source.capture_frame(&frame)` directly from `pulse-mic`. The fast path never awaits:
  use `futures::FutureExt::now_or_never()` and treat `None` as an error, or `block_on`. Keep a reusable
  `[i16; 480]` and build `AudioFrame { data: Cow::Borrowed(&buf) }`. Move GapFill into the processor
  (`rx.recv_timeout(50 ms)` → send silence frames). Delete `ToInternal` from the call path (keep it for the mic
  test if needed), the `drop_stale` dance, and the `ready` buffer. The processor's MAX_LAG already bounds backlog.
- **Expected gain:** 0–5 ms of jitter removed from send timing, about 3 fewer allocs and copies per 10 ms, simpler
  code.
- **Risk:** low. `NativeAudioSource` is `Clone + Send` (SharedPtr). `capture_frame` takes a mutex in C++ (the
  `InternalSource` mutex), which is fine off the RT callback.
- **Measure:** the `send_ms` gauge (`mod.rs:508`) should drop to 0. Add a histogram of intervals between
  successive `capture_frame` calls: p99 close to 10 ms is the target.

#### M5. Opus: 20 ms packets, RED, bitrate, and possibly stereo
- **Where:** `mod.rs:885-895` (`max_bitrate 64_000`, `dtx`, `red`), `mod.rs:404` (mono source), livekit
  `local_participant.rs:382-445`, livekit `peer_transport.rs:339-386` (single-PC mode munges `stereo=1` into
  *our offer*), and `RoomOptions::default()` → `single_peer_connection: true` (`livekit room/mod.rs:463-480`).
- **Facts:**
  - `TrackPublishOptions` for audio in 0.9.3 exposes only `audio_encoding.max_bitrate`, `dtx`, `red`,
    `preconnect_buffer`. There's no ptime, stereo, FEC or complexity knob. `max_bitrate` becomes
    `RtpEncodingParameters.max_bitrate`, which caps the encoder. The actual target comes from send-side BWE between
    min and max.
  - Packet size is the WebRTC default 20 ms unless the remote description carries `a=ptime`. That makes 26.5 ms
    algorithmic delay (Opus wiki, default 20 ms frames).
  - RED in libwebrtc re-sends the previous frame(s) inside each packet: no sender latency, roughly 2× payload while
    talking. DTX: our gate emits exact zeros, so DTX engages fully during silence (good). LiveKit disables RED
    automatically with E2EE.
  - In single-PC mode the SDK adds `stereo=1` to the Opus fmtp in the offer. Whether the server's answer for *our
    send* m-line then carries `stereo=1` decides if WebRTC configures a 2-channel Opus encoder for our mono source
    (`AudioEncoderOpus` reads `stereo` from the negotiated params). If it does, we waste encode CPU (~1.5×),
    bitrate is split across two identical channels, and every receiver decodes stereo then remixes to mono in
    `NativeAudioSink::OnData` (`audio_track.cpp:93-127`, `RemixAndResample`).
- **Proposed changes:**
  - **Measure stereo first** (see the list below). If it's stereo, set `RoomOptions.single_peer_connection = false`
    (dual-PC: the publisher offer has no stereo munge), or ask upstream for an option.
  - **Bitrate:** 64 kbps mono Opus for speech is generous. 32–40 kbps is transparent for voice. The main cost is
    upstream bandwidth with RED (~130 kbps while talking). There's little CPU difference. Keep 64 k if the
    friends' uplinks are fine. It's a quality knob, not a CPU one.
  - **ptime 10 ms:** −10 ms of encode delay, and NetEq's floor can drop too, since its minimum target scales with
    packet duration. Costs:
    - packet rate doubles: +~24–30 kbps of RTP/UDP/IP/SRTP overhead per speaker, and 2× SFU packets per second.
      That's still tiny for 5–8 people on the A1 box, but it's the one thing on the server that scales with
      packets;
    - Opus is slightly less efficient at 10 ms (xiph issue "Quality is poorer when FrameSize is 10 ms",
      https://gitlab.xiph.org/xiph/opus/issues/1962).
    - It requires munging `a=ptime:10` into the *remote answer* before `setRemoteDescription`, which means a
      livekit-rust fork (no hook in 0.9.3), or LiveKit server support.
    - Mumble's docs frame 10 vs 20 ms "audio per packet" as a latency/bandwidth trade-off and note some drivers
      struggle at 10 ms (https://www.mumble.info/documentation/user/audio-settings/). Discord's 2018 write-up
      (native WebRTC, VAD-gated sending,
      https://discord.com/blog/how-discord-handles-two-and-half-million-concurrent-voice-users-using-webrtc)
      doesn't state its ptime.
    - Verdict: do it last, after H1/H2. Medium effort, medium gain.
  - `adaptive_stream` / `dynacast` (default false) only affect video layers. They're irrelevant for audio-only.
    `auto_subscribe: true` is right for us.
  - `AudioSourceOptions::default()` (all false) is right: it tells the voice engine's internal APM to disable
    AEC/NS/AGC (see M7).
- **Risk:** stereo off has no risk. ptime 10 means a fork plus SFU load.
- **Measure:** `room.get_stats()` already runs every 5 s (`mod.rs:427-447`). Additionally log:
  - for each `RtcStats::Codec` on the publisher: `channels`, `sdp_fmtp_line`, `payload_type`;
  - `OutboundRtp.target_bitrate`, and bytes/packets sent per second (packets/s ≈ 50 means 20 ms ptime);
  - inbound `jitter_buffer_target_delay` / `jitter_buffer_minimum_delay` / `concealed_samples` /
    `inserted_samples_for_deceleration` / `removed_samples_for_acceleration` (all present in libwebrtc
    `stats.rs:376-390`).
- **Sources:** livekit/libwebrtc sources as cited. Opus latency (https://en.wikipedia.org/wiki/Opus_(audio_format)).
  RFC 7587 ptime semantics (summarised in the W3C ORTC archive,
  https://lists.w3.org/Archives/Public/public-ortc/2015Nov/0011.html).

#### M6. WASAPI buffer and period: what cpal 0.15 can and can't do
- **Where:** `devices.rs:580-600` (configs from `default_*_config` → `BufferSize::Default`), cpal
  `wasapi/device.rs:565-600` (`Initialize(SHARED, EVENTCALLBACK, hnsBufferDuration = 0 for Default)`), cpal
  `stream.rs:443-490` (render fills **all** `buffer - padding` frames each event).
- **Facts:**
  - Shared mode with `hnsBufferDuration = 0` already gives the engine's minimum buffer.
    `BufferSize::Fixed(n)` only *requests a buffer at least that big*. It doesn't change the 10 ms engine period,
    so **Fixed can't lower latency below Default in shared mode.** It's only useful to *raise* it for fragile
    devices.
  - Because cpal fills the whole free render buffer, render latency ≈ the full buffer (~10–22 ms) + engine + DAC.
  - Real reductions need IAudioClient3 `InitializeSharedAudioStream` with a smaller period. The inbox HDAudio
    driver supports 128–480 frames (2.67–10 ms at 48 k), but *every app on that endpoint* then runs at the small
    period (more wakeups and power). cpal (0.15 through 0.18.x, current master) doesn't implement IAudioClient3.
  - GStreamer only uses it when low latency is explicitly requested, because slow machines and 1-CPU VMs glitch.
    PortAudio's maintainer argues it's mostly unnecessary. JUCE users report the 10 ms shared floor.
  - Bluetooth and virtual devices (Sonar) generally won't offer small periods. Sonar adds its own processing
    latency on top, and BT A2DP output is 100–200 ms whatever we do. HFP gives 16 kHz (resampled, 5 ms) at 30–60 ms.
  - cpal 0.15 also has no AUTOCONVERTPCM, so a 44.1 k mix format means we resample (5 ms per side). Most devices
    are 48 k. Check the "audio input/output opened rate=" log lines from friends.
- **Proposed change:** this is a later item. The options are a small custom WASAPI backend (IAudioClient3 with a
  period around 5 ms + MMCSS) for wired/USB devices only, falling back to cpal; or contributing IAudioClient3 to
  cpal.
- **Expected gain:** −5–15 ms per direction on capable wired devices. Nothing on BT or Sonar.
- **Risk:** medium-high: driver quirks, and power use on laptops.
- **Measure (first):** with cpal ≥ 0.18 the callback timestamps include hardware latency. Even on 0.15, log
  `info.timestamp().playback - callback` (output) and `callback - capture` (input) once per second. Our callbacks
  currently ignore the `_` info argument (`devices.rs:894-917`, `devices.rs:938-960`). That gives each friend's
  actual device latency, which tells us whether this item matters.
- **Sources:** MS low-latency doc (above, includes the "10 ms default; inbox HDAudio 128–480 samples; all apps on
  the endpoint switch" facts). GStreamer IAudioClient3 commits
  (https://git.fasttube.de/FaSTTUBe/GST-Tensordecoder-ov_ep/commit/b7653925db56cadbf7224e2310ab4408ade00f3d).
  PortAudio issue #385 (https://github.com/PortAudio/portaudio/issues/385). JUCE issue #560
  (https://github.com/WeAreROLI/JUCE/issues/560).

#### M7. The voice engine's own APM exists. Make sure it stays idle
- **Where:** `webrtc-sys peer_connection_factory.cpp:195` (`BuiltinAudioProcessingBuilder`). WebRtcVoiceEngine
  defaults AEC/AGC/NS/HPF to on at init. Our `AudioSourceOptions::default()` (`mod.rs:404`) passes *explicit
  false* for AEC/NS/AGC (`audio_track.cpp:40-46`), which turns those off when the track is attached. The render
  side still calls `ProcessReverseStream` on the synthetic ADM's mixed output every 10 ms (cheap with AEC off). The
  capture side is unused (NativeAudioSource bypasses `RecordedDataIsAvailable`).
- **Proposed change:** none in code. Just don't set `AudioSourceOptions` to true, or WebRTC would run a second AEC3
  on the playback mix. Add a comment at `mod.rs:404`.
- **Gain/Risk:** nothing to gain or risk. This is a guard against a regression.
- **Measure:** a CPU profile in a call (Windows: WPR/WPA, or Superluminal) looking for
  `EchoCanceller3::ProcessCapture` on non-`pulse-mic` threads.

### LOW

#### L1. Echo reference: feed the 48 k mono mix instead of the device-rate, device-channel output
- **Where:** `devices.rs:390-405` (downmix of `out` at the device rate → i16 → queue), `devices.rs:409-425`
  (`process_reverse_stream(c, rate)` at 44.1 k etc.; `collect::<Vec<i16>>()` alloc per capture block).
- **Change:** in `Playout`/`on_output`, push the 48 k mono mix (pre-resample) into the echo ring. The APM then
  doesn't resample render internally, and the multichannel downmix is skipped. Drain into a reusable buffer.
- **Gain:** a little CPU (APM's render resampler at 44.1 k devices), 1 alloc per 10 ms. AEC alignment is fine
  either way. The playout resampler adds a constant ~5 ms that AEC3's delay estimator absorbs.
- **Risk:** low.
- **Also:** `set_stream_delay_ms` exists (`libwebrtc apm.rs`) but is unused. AEC3 estimates delay itself, so feeding
  the cpal-reported (playback − capture) delay is optional. Only try it if echo leaks after device switches.
- **Measure:** an AEC A/B with speakers and a far-end talker (the open FINDINGS echo test).

#### L2. Per-block allocations, for determinism rather than CPU
Our code makes about 8 allocs per 10 ms on the mic path plus 1 per peer per 10 ms on receive:

| Where | Alloc |
|---|---|
| `devices.rs:356` | `Vec::with_capacity(chunk)` per block, in the **RT capture callback** |
| `processor.rs:357` | `pending.drain(..FRAME).collect()` |
| `devices.rs:414` | echo `collect` |
| `processor.rs:183` | Strong `block.to_vec()` per block, plus the returned Vec |
| `gate.rs:100` | `block.to_vec()` |
| `processor.rs:395` | i16 `collect` |
| `resampler.rs:70` | `to_vec` |
| `mod.rs:510` | `collect` |
| `mixer.rs:97` | `entry(id.to_string())` allocates a String on **every push** |

libDF itself allocates more (Tensor clones and `to_owned` in `tract.rs:520-640`). libwebrtc allocates per received
frame (`data.to_owned()` in `audio_stream.rs`).

- **Change:**
  - Use fixed `[f32; 480]` arrays. Recycle Vecs through a return channel, for Strong and for the capture callback:
    send a `Box<[f32; N]>` and get it back.
  - Turn the gate delay into a ring of 2 fixed arrays.
  - `if let Some(p) = self.peers.get_mut(id)` before `entry`.
- **Gain:** about 1 µs per 10 ms of CPU. The real benefit is no heap calls in the capture RT callback (the Windows
  heap can take locks).
- **Risk:** none.
- **Measure:** a counting allocator in a release bench (`dhat` or a custom `GlobalAlloc` counter) over 60 s of
  `processor::run` with Null audio. Expect about 800/s → about 0/s.

#### L3. f32 ↔ i16 round trips
- **Where:** `processor.rs:361-372` (f32 → i16 → APM → i16 → f32), then `processor.rs:395` (→ i16 for Opus).
  webrtc-sys exposes only the int16 APM API (`apm.cpp:33-51`), and the APM converts to float internally anyway.
- **Change:** none without patching webrtc-sys to expose the float `ProcessStream`. The cost is about 3 × 480
  conversions = ~1 µs, plus negligible 16-bit quantisation before DFN. **Not worth doing.**

#### L4. Resampler delay on non-48 k devices
- **Where:** `resampler.rs:21`: `Fft::new(from, to, chunk=rate/100, 1 ch, FixedSync::Input)` auto-selects
  sub_chunks = chunk/256 = 1. For 44.1↔48 k, fft_in/out = 441/480 → delay 240 output frames (5 ms). For 16 k → 48 k,
  also 480 → 5 ms.
- **Change:** `Fft::new_custom(..., sub_chunks = 3, ...)`: fft 147/160, delay 80 frames ≈ 1.7 ms, at the cost of a
  lower anti-alias cutoff. Verify with the existing `keeps_high_frequencies_both_ways` (15 kHz within 0.5 dB) and
  `adds_little_latency` tests. If 15 kHz drops, try sub_chunks = 2. Better still, open devices at 48 k (cpal
  ≥ 0.17.2 / 0.18.2 added WASAPI-side resampling and format conversion; check whether it's opt-in).
- **Gain:** −3 ms per resampled direction, only on 44.1 k or BT devices.
- **Risk:** a slight HF roll-off. The tests guard it.
- **Measure:** the existing `adds_little_latency` print. Field logs of the opened device rates.

#### L5. Deafen and mute could stop network decode and encode
- Deafened: we still receive, NetEq-decode and mix everything (master = 0, `devices.rs:372`).
  `RemoteTrackPublication::set_subscribed(false)` on deafen would save decode CPU and downlink. Muted: we send
  zeros (DTX keeps it at about 1 packet per 400 ms). `LocalTrackPublication::mute()` would also tell the server,
  and other clients, explicitly.
- **Gain:** small CPU and bandwidth. It's rare-state only.
- **Risk:** resubscribe latency on undeafen (~100–300 ms) and SDP renegotiation churn. **Probably not worth it**,
  but listed for completeness.

#### L6. RNNoise VAD alongside DFN
- `denoise.rs:120`: 37 µs per 10 ms (0.4 %). DFN's `lsnr` could replace it, but it isn't a probability (FINDINGS)
  and H1 wants the RNNoise output anyway. **Keep it.**

#### L7. Build profile
- No `[profile.release]` in the workspace `Cargo.toml` (only dev overrides for tract/DF/nnnoiseless). Release is
  opt-level 3, thin-local LTO, 16 codegen units. tract-linalg and rustfft runtime-dispatch AVX2/FMA kernels
  (`tract-linalg x86_64_fma.rs:128-132`), so `target-cpu` isn't needed. nnnoiseless is scalar but tiny.
- **Change:** `lto = "fat"`, `codegen-units = 1`, `panic` unchanged (DFN load uses `catch_unwind`). Expect 0–10 %
  on the Rust DSP (DFN glue, gate, resampler) and a smaller binary (FINDINGS already notes this as a possible
  trim).
- **Risk:** only build time.
- **Measure:** re-run `denoise_bench` with each profile.

#### L8. Strong worker round trip
- `processor.rs:181-197`: synchronous send → recv across threads for each block. That's 2 wakeups per 10 ms
  (~5–30 µs each on Windows via WaitOnAddress) and no parallelism gained. Fine as is. If H3 promotes `pulse-mic`,
  promote `pulse-strong` too, or the RT thread blocks on a normal-priority one (priority inversion).

---

## 3. What others do (cited)

| Technique | Who | Applies to us |
|---|---|---|
| Per-source ring, **target 2 frames (20 ms)**, cap 10 frames, gradual catch-up drain; RT-priority receive and transmit tasks; `NativeAudioSource` queue 10 ms; APM with AEC + AGC2 + HPF + NS; reverse stream fed at the device rate | Zed `livekit_client` (same Rust SDK + cpal), commit 5713dbf and `playback.rs` on main | H2 target, H3 priorities. They use queue 10 ms, we use 0 (our choice is lower latency; keep it). |
| Realtime/MMCSS priority for WASAPI threads | cpal 0.16.0+ (`audio_thread_priority`), MS low-latency guidance | H3 |
| IAudioClient3 small shared periods, only when low latency is requested | MS doc; GStreamer wasapi; JUCE #560; PortAudio #385 (skeptical) | M6 |
| 10 vs 20 ms packets as a latency/overhead trade-off; some drivers glitch at 10 | Mumble audio settings docs | M5 |
| VAD-gated sending, nothing sent in silence, native WebRTC engine | Discord 2018 engineering blog | We already do this (gate + DTX). |
| Opus 26.5 ms total algorithmic latency at 20 ms frames | Opus (Wikipedia / libopus defaults) | M5 |
| DFN: 2-frame lookahead, ~40 ms in DF1/2 papers, RTF 0.19 on an i5-8250U (DF1) | DeepFilterNet papers | M1. FINDINGS measured 30 ms for DF3 in libDF. |
| RNNoise 0.2: retrained model, AVX2/SSE4.1 + RTCD | Xiph / Phoronix, April 2024 | M1(c) |
| rubato FFT delay = fft_size_out/2; sub_chunks trades delay for cutoff | rubato 5 docs/source | L4 |

Searches with nothing primary to report:
- livekit rust-sdks issues specifically on NativeAudioStream latency;
- a Discord blog post on jitter-buffer tuning (only the 2018 architecture post exists);
- AEC3 per-frame CPU benchmarks.

These are measure-ourselves items.

---

## 4. Measure first (numbers we need before deciding)

1. **Send codec channels and fmtp** (M5 stereo question). Log publisher `RtcStats::Codec { channels, sdp_fmtp_line }`
   and outbound packets/s (50 means 20 ms ptime) and `target_bitrate`. One call is enough.
2. **Per-friend device facts.** Opened rates and channels are already logged. Add the cpal callback buffer size
   (frames per callback, min/max) and the hardware latency from callback timestamps (`playback − callback`,
   `callback − capture`). This sizes H2 and decides whether M6 matters, especially for Sonar and BT users.
3. **NativeAudioStream inter-arrival histogram on Windows** (H4): p50/p99/max per peer. This decides the H2 target.
4. **Mixer queue stats** (H2): min/p50/max per 5 s, and zero-fill underrun samples per minute in a 30-minute call.
   Today's `recv_ms` is a single 5 s sample.
5. **NetEq numbers:** `jitter_buffer_target_delay/emitted`, `jitter_buffer_minimum_delay`, concealed and
   accel/decel samples, and `rtt`. This tells us how much of the budget is the network vs us.
6. **Glitch counters under load** (H3/M3): MAX_LAG drops (`trim_backlog` return), send drops, output underruns,
   mixer `try_lock` misses, cpal stream errors. Run a game or a CPU-stress call before and after the priority work.
7. **CPU split per stage** on the weakest friend PC: APM `process_stream` µs, `feed_echo_reference` µs, denoiser
   µs (DFN encoder vs decoders via libDF `timings`), gate µs. Plus whole-process CPU while muted vs talking vs
   listening. This decides M2 and how much AEC3 actually costs.
8. **End-to-end latency, measured acoustically:** two PCs side by side. Clap into A's mic and record B's headphone
   output and the clap on one recorder (phone at 48 k). Measure the onset delta. Do it for Strong/Standard/Off and
   before/after H1/H2. This is the ground truth for the budget table.
9. **Allocation count per second** in `processor::run` (L2): only if we want a hard "no alloc in RT" guarantee.

---

## 5. Suggested order

1. Measurements 1, 2, 4, 5, 6: logging only.
2. H1 gate redesign: −20 ms.
3. H3 cpal ≥ 0.16 + MMCSS for pulse-mic/strong. Then M3 interim `try_lock`.
4. H2 drift-compensated playout + 20 ms target: −20–35 ms.
5. M4 direct `capture_frame` from the processor.
6. M2 skip DFN when muted.
7. Then decide: M5 stereo fix (if needed) → M1 LL-model option → M5 ptime 10 (fork) → M6 IAudioClient3.
