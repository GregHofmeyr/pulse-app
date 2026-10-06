# Noise suppression + voice gate — design

Date: 2026-10-06 · Status: approved in conversation, pending written-spec review

## 1. Goal

Friends hear *you*, not your environment, while voice stays as crisp as it is after the
2026-10-05 DSP fixes (48 kHz native devices, band-limited resampling, 64 kbps).

**Priorities**

1. Keyboard/mouse clicks and general noise (fans, hum, clatter): remove properly.
2. Background conversations (busy office): reduce as far as is practical. Every suppressor,
   Krisp included, treats nearby voices as speech, so "reduced" is the honest target.

**Constraints**

- Lightweight: Strong must fit in a few % of one CPU core and tens of MB; Standard and Off
  must cost ~nothing. Pulse's reason to exist is not being Discord's 2 GB.
- Public repo: permissive licenses only.
- Windows is the primary platform; Linux is dev. Everything must cross-build for
  `x86_64-pc-windows-msvc`.
- Added latency is acceptable where it buys real quality, but it must never accumulate.

**Success**

- Typing and clicks are mostly gone on Strong.
- No clipped first syllables, no hiss "breathing" between words.
- Toggling Off / Standard / Strong in the mic test makes the difference obvious.

## 2. Background (why the current pipeline can't do this)

- libwebrtc's noise suppressor targets *stationary* noise. Its transient (keyboard)
  suppressor was deleted upstream, so no WebRTC setting removes clicks.
- The gate opens on loudness (`sensitivity`, an RMS threshold) and holds 300 ms. A click is
  loud, opens the gate, and its whole tail is sent.
- Processing runs inside the cpal capture callback, which is fine for WebRTC's light work but
  unsafe for a neural model (callback overruns become audible glitches).
- Mic gain is applied and hard-clipped *before* processing, which feeds clipped audio to the
  echo canceller and boosts clicks.

## 3. Approach

Two neural denoisers, both pure Rust (no native DLLs), behind one interface:

| Level | Engine | License | Notes |
|---|---|---|---|
| Off | none: no suppression at all | — | honest baseline; improved gate still applies |
| Standard | RNNoise via `nnnoiseless` 0.5 (crates.io) | BSD-3-Clause | ~1% of a core, 10 ms frames, no lookahead; softens clicks |
| Strong | DeepFilterNet3 via `libDF` (`tract` + bundled model) | MIT / Apache-2.0 | removes clicks; heavier; normal or low-latency model per measurement |

`libDF`'s real-time inference is not on crates.io (`deep_filter` 0.2.5 lacks the `tract`
feature), so it is a **git dependency pinned to an exact commit** of
`github.com/Rikorose/DeepFilterNet`. That repo has been quiet since 2024-10; the risk is
contained by the interface (Strong's engine can be swapped without touching anything else).
Rejected: DeepFilterNet via ONNX Runtime (`ort`) — a native library per platform plus a
re-implementation of DeepFilterNet's STFT/ERB pre- and post-processing. It stays the fallback if
`libDF` measures too slow.

## 4. Pipeline

Per 10 ms block (480 samples at 48 kHz, mono):

1. **Capture** (cpal callback): downmix, copy, hand off. No processing in the callback.
2. **Processing thread** (dedicated OS thread, owns all state below):
   1. Resample to 48 kHz if the device isn't (normally it is).
   2. **WebRTC APM**: high-pass filter always; echo cancellation if enabled; auto gain if
      enabled. WebRTC's own noise suppressor is **always off**: Off means truly no
      suppression (an honest A/B baseline), and two suppressors are never stacked. The
      far-end (reverse) stream feed from playback is unchanged.
   3. **Denoiser** (level-dependent): returns the cleaned block and a speech probability
      in `[0, 1]`.
   4. **Input gain** (moved here, after cleaning), with the existing soft limiter instead of a
      hard clamp.
   5. **Voice gate** (section 5).
   6. Out to the existing mic → LiveKit task (exact 10 ms frames, 64 kbps, DTX + RED).
3. **Meters**: the mic meter shows the level after cleaning, so what you see is what is sent.

The mic test in Settings runs the same pipeline, so it previews exactly what friends hear.

### Components

- `voice/denoise.rs`: `trait Denoiser { fn process(&mut self, block: &mut [f32; 480]) -> f32 /* speech prob */ }`
  with `Passthrough`, `Rnnoise`, `DeepFilter`. Construction is fallible (model load).
- `voice/gate.rs`: the gate state machine (replaces `devices::Gate`), pure, time passed in.
- `voice/processor.rs`: the processing thread: owns the APM, denoiser, gain and gate; applies
  config changes; times every block; reports level/speaking/downgrade events.
- `devices.rs`: the capture callback shrinks to "downmix + send"; the APM moves into the
  processor. Playback and the watchdog are unchanged.

## 5. Voice gate

**Automatic mode (default).** Open when **both**:

- speech probability ≥ 0.6 (close only below 0.35: hysteresis, no flicker), **and**
- the block's level exceeds a tracked **noise floor** by a margin (start: 6 dB).
  At level Off there is no model, so the probability condition is treated as met and the
  gate opens on level above the floor alone. The floor
  follows the quietest recent levels (fast down, slow up). Nearer/louder speech (you) clears
  it; quieter background speech (colleagues) tends not to.

**Both modes:**

- Hold 300 ms after the last qualifying block.
- Fade-out over ~100 ms instead of a hard cut (no "breathing").
- Pre-roll: the last ~20 ms are always buffered and released when the gate opens (no
  clipped first syllables). This adds 20 ms of latency.
- While closed, send digital silence (DTX makes it nearly free).

**Manual mode:** the existing sensitivity slider (RMS threshold) decides "speech", with the
same hold, fade and pre-roll.

**UI:** a toggle "Automatically determine sensitivity" (default on); off reveals the slider.
The level meter stays and shows when the gate is open.

## 6. Settings and defaults

- `AudioConfig.noise_suppress: bool` → `noise_suppression: Off | Standard | Strong`.
  Migration: old `true` → the default level, old `false` → `Off`. Both the Rust config
  (serde) and the UI's stored settings migrate; nothing breaks for existing users.
- New `auto_sensitivity: bool`, default `true`. `sensitivity` keeps its meaning for manual
  mode.
- Default level: **Strong** if the step-1 measurements are comfortable (a few % of one core,
  acceptable latency), otherwise **Standard**. Recorded in `spikes/voice/FINDINGS.md`.
- Echo cancellation default unchanged (on).
- Level changes apply **instantly** on the processing thread: no device reopen, no blip.
- The DeepFilterNet model loads only when Strong is selected and is dropped when switched
  away.

## 7. Safety nets

1. **Model fails to load** → fall back to Standard, log a warning, show "Strong unavailable,
   using Standard" in Settings.
2. **PC too slow for Strong** → the processor times each block. If processing exceeds ~70%
   of the 10 ms budget **sustained over ~3 s** (not one spike), it switches to Standard and
   tells the user (a `NoiseSuppressionDowngraded` voice event; Settings shows why).
3. **Backlog** → if the hand-off queue holds more than ~60 ms, the oldest blocks are dropped.
   Latency never accumulates.
4. Processing errors from APM keep the existing warn-once logging.

## 8. Testing

**Step 1 — measurement (gates the rest).** A benchmark (`#[ignore]` test or example) runs
`Rnnoise`, `DeepFilter` (normal) and `DeepFilter` (low-latency) over 60 s of audio, reporting
per-block time (mean, p99, max), RSS added, model load time and algorithmic latency. Also
`cargo xwin build` for Windows with `libDF` included. Results go into
`spikes/voice/FINDINGS.md` and decide Strong's model and the default level. Bad numbers → stop
and revisit (the `ort` fallback) before UI work.

**Automated (test-first):**

- Denoiser contract: 480 in → 480 out, probability in `[0, 1]`, for every engine.
- Strong attenuates a synthetic click train by ≥ 15 dB; Standard reduces it; speech in a short
  real clip is preserved within ~3 dB; stationary noise is reduced. The speech fixture is a
  few seconds of openly licensed speech (e.g. LibriSpeech, CC BY 4.0, attributed in the repo),
  ≤ ~300 KB.
- Gate: opens on probability + floor margin; ignores quieter speech; hysteresis; hold 300 ms;
  fade 100 ms; pre-roll 20 ms released on open; floor tracker adapts; manual mode.
- Safety: a deliberately slow fake denoiser triggers the downgrade after sustained overload
  but not after a single spike; load failure falls back to Standard; backlog drops keep
  latency bounded.
- Config migration (Rust + vitest).
- Existing suites stay green: `just check`, `just voice-it`, the Windows cross-check.

**Ear test (Greg):** in the mic test, type while talking and flip Off / Standard / Strong.

## 9. Out of scope (parked)

- Configurable bitrate (per voice channel) — waits on the hosting decision.
- Auto-disabling echo cancellation for headphones (Windows endpoint form factor).
- Per-voice-channel audio settings.
- AGC2-after-NS as a separate stage (APM's AGC stays where it is, off by default).
