# Noise Suppression + Voice Gate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Off / Standard (RNNoise) / Strong (DeepFilterNet) noise suppression plus a speech-driven voice gate, on a dedicated mic processing thread.

**Architecture:** The cpal capture callback only downmixes and hands 10 ms blocks to a new processing thread. That thread resamples to 48 kHz, runs WebRTC APM (HPF/AEC/AGC; its NS always off), a `Denoiser`, input gain + soft limit, and a `VoiceGate`, then sends 48 kHz frames to the existing mic → LiveKit task. Denoisers are swapped live; Strong's model loads on a helper thread; an overload detector downgrades Strong → Standard.

**Tech Stack:** Rust (cpal, livekit 0.9 APM, `nnnoiseless` 0.5, `deep_filter`/libDF git `d375b2d` with tract + bundled DFN3 model, `ndarray` 0.15, rubato already present), Tauri 2, Svelte 5, vitest.

**Spec:** `docs/superpowers/specs/2026-10-06-noise-suppression-design.md`

## Global Constraints

- Permissive licenses only (public repo): `nnnoiseless` BSD-3-Clause, `deep_filter` MIT/Apache-2.0, fixture public domain.
- Must cross-build for `x86_64-pc-windows-msvc` (`cargo xwin check -p pulse-client --target x86_64-pc-windows-msvc`).
- `deep_filter` is a git dependency pinned to `rev = "d375b2d8309e0935d165700c91da9de862a99c31"`; never a branch.
- Frames are 10 ms = 480 samples at 48 kHz mono, f32 in [-1, 1] inside the processor.
- Gate constants: open prob ≥ 0.6, close prob < 0.35, floor margin 6 dB, hold 300 ms, fade-out 100 ms, pre-roll 20 ms (2 blocks).
- Overload: mean processing time > 70% of 10 ms (7 ms) over a full 3 s window (300 blocks) → Strong downgrades to Standard.
- Backlog: never more than 6 queued raw blocks (60 ms); drop oldest.
- WebRTC APM noise suppression is always off; `Off` means no suppression at all.
- Settings migrate: old `noise_suppress: false` → `off`; old `true` or missing → default level.
- No read receipts or unrelated features; bitrate stays 64 kbps (parked).
- Every commit ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`; `just check` must pass before each commit (fmt, clippy -D warnings, tests, svelte-check).

## Review Focus

1. **Rapid level switching while Strong's model loads** (Strong → Off → Strong in < 1 s): the newest choice must win; a stale loader result must never be installed. Test: `stale_model_load_is_ignored` in Task 6.
2. **Device rate changes mid-call** (BT call mode 16 kHz ↔ 48 kHz): the processor must keep producing exact 480-sample 48 kHz frames. Test: `processor_handles_rate_change` in Task 6.
3. **Mute while the gate is open / during pre-roll**: nothing of the user's audio may leak after mute. Test: `mute_silences_immediately_including_preroll` in Task 4.
4. **Leaving voice / closing the mic test**: the processor thread (and a 36 MB-class model) must not outlive the session. Test: `processor_exits_when_senders_drop` in Task 6.
5. **Very loud input with 4× gain**: output never exceeds ±1.0 (no wrap/clip noise). Test: `loud_input_with_max_gain_stays_in_range` in Task 4.

---

## File Structure

| File | Responsibility |
|---|---|
| `client/src-tauri/src/voice/denoise.rs` (new) | `NsLevel`, `Denoiser` trait, `Passthrough`, `Rnnoise`, `DeepFilter`, `make_fast()` |
| `client/src-tauri/src/voice/gate.rs` (new) | `VoiceGate`: speech/level gating, hysteresis, floor tracker, hold, fade, pre-roll |
| `client/src-tauri/src/voice/processor.rs` (new) | `OverloadDetector`, `MicChain` (pure per-block chain), processing thread + live config |
| `client/src-tauri/src/voice/devices.rs` | `AudioConfig` fields; capture callback becomes "downmix + hand off"; `Shared` loses `gate`/`gain`, gains processor config/status; APM NS off |
| `client/src-tauri/src/voice/mod.rs` | wire processor into sessions; `VoiceEvent::NoiseSuppression`; `Levels.gate_open` |
| `client/src-tauri/src/voice/mictest.rs` | mic test uses the same processor |
| `client/src-tauri/tests/denoise_bench.rs` (new) | `#[ignore]` measurement benchmark |
| `client/src-tauri/tests/fixtures/speech_48k_mono_s16le.raw` (new) + `README.md` | public-domain speech fixture |
| `client/ui/src/lib/voiceui.ts` (+ test) | `NsLevel`, new config fields, migration |
| `client/ui/src/lib/voice.svelte.ts` | new event kinds, `voice.ns`, `voice.gateOpen` |
| `client/ui/src/components/Settings.svelte` | level picker, auto-sensitivity toggle, status note |
| `spikes/voice/FINDINGS.md` | measurement results + decisions |

---

### Task 1: Dependencies, speech fixture, measurement (decision gate)

**Files:**
- Modify: `client/src-tauri/Cargo.toml`
- Create: `client/src-tauri/tests/fixtures/speech_48k_mono_s16le.raw`, `client/src-tauri/tests/fixtures/README.md`
- Create: `client/src-tauri/tests/denoise_bench.rs`
- Modify: `spikes/voice/FINDINGS.md`

**Interfaces:**
- Produces: deps `nnnoiseless`, `deep_filter` (lib name `df`), `ndarray`; cargo feature `dfn-ll`; fixture path `tests/fixtures/speech_48k_mono_s16le.raw` (s16le, 48 kHz, mono, 3 s); decisions recorded: Strong model (normal | ll) and default level (strong | standard).

- [ ] **Step 1: Add dependencies.** In `client/src-tauri/Cargo.toml` `[dependencies]` add:

```toml
nnnoiseless = { version = "0.5", default-features = false }
deep_filter = { git = "https://github.com/Rikorose/DeepFilterNet", rev = "d375b2d8309e0935d165700c91da9de862a99c31", default-features = false, features = ["tract", "default-model"] }
ndarray = "0.15"
```

and add (create the table if absent):

```toml
[features]
# Strong uses DeepFilterNet3's low-latency model (36 MB) instead of the normal one (8 MB).
dfn-ll = ["deep_filter/default-model-ll"]
```

Run: `cd client/src-tauri && cargo build 2>&1 | tail -5`
Expected: builds. If libDF fails with unresolved `log`, add `"logging"` to its `features` list and rebuild.

- [ ] **Step 2: Create the speech fixture** (public domain LibriVox, "The Art of War", read by Moira Fogarty):

```bash
S=/tmp/claude-1000/-home-greghofmeyr/976e3032-6fbd-4099-bced-b680082520ec/scratchpad/fixture; mkdir -p $S
curl -sL -o $S/aow.mp3 https://archive.org/download/art_of_war_librivox/art_of_war_01-02_sun_tzu_64kb.mp3
mkdir -p client/src-tauri/tests/fixtures
ffmpeg -loglevel error -ss 30 -t 3 -i $S/aow.mp3 -ac 1 -ar 48000 -f s16le client/src-tauri/tests/fixtures/speech_48k_mono_s16le.raw
ls -l client/src-tauri/tests/fixtures/speech_48k_mono_s16le.raw   # expect 288000 bytes
```

Listen once to confirm it's continuous speech (`ffplay -f s16le -ar 48000 -ac 1 <file>`); if it lands on a pause, change `-ss` and redo. Create `client/src-tauri/tests/fixtures/README.md`:

```markdown
# Test fixtures

`speech_48k_mono_s16le.raw` — 3 s of speech, raw signed 16-bit little-endian, 48 kHz, mono.
Excerpt (from 0:30) of "The Art of War" chapters 1–2, LibriVox recording, **public domain**:
https://archive.org/details/art_of_war_librivox
```

- [ ] **Step 3: Write the benchmark** `client/src-tauri/tests/denoise_bench.rs`:

```rust
//! Measures the denoisers (run: `cargo test --release --test denoise_bench -- --ignored --nocapture`,
//! and again with `--features dfn-ll`). Results go into spikes/voice/FINDINGS.md.

use std::time::{Duration, Instant};

const FRAME: usize = 480;

fn speech() -> Vec<f32> {
    include_bytes!("fixtures/speech_48k_mono_s16le.raw")
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect()
}

/// 60 s: the speech clip looped, plus deterministic noise and a click every 150 ms.
fn workload() -> Vec<f32> {
    let s = speech();
    let mut x: u32 = 1;
    (0..48_000 * 60)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let noise = (x as f32 / u32::MAX as f32 - 0.5) * 0.02;
            let click = if i % 7200 < 96 { 0.4 * (1.0 - (i % 7200) as f32 / 96.0) } else { 0.0 };
            s[i % s.len()] * 0.5 + noise + click
        })
        .collect()
}

fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).map(str::to_owned))
        .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        .map_or(0.0, |kb| kb / 1024.0)
}

fn report(name: &str, mut times: Vec<Duration>) {
    times.sort();
    let mean = times.iter().sum::<Duration>() / times.len() as u32;
    let p99 = times[times.len() * 99 / 100];
    let max = *times.last().unwrap();
    println!("{name}: per 10 ms block mean {mean:?} p99 {p99:?} max {max:?} ({:.1}% of budget)",
        mean.as_secs_f64() * 100.0 / 0.010);
}

/// Lag (ms) maximising the cross-correlation of output against input over 0..100 ms.
fn delay_ms(input: &[f32], output: &[f32]) -> f32 {
    let n = input.len().min(output.len());
    let best = (0..4800)
        .max_by(|&a, &b| {
            let c = |lag: usize| (0..n - lag).step_by(4).map(|i| input[i] * output[i + lag]).sum::<f32>();
            c(a).total_cmp(&c(b))
        })
        .unwrap();
    best as f32 / 48.0
}

#[test]
#[ignore]
fn bench_denoisers() {
    let w = workload();

    let mut st = nnnoiseless::DenoiseState::new();
    let (mut inb, mut outb) = (vec![0f32; FRAME], vec![0f32; FRAME]);
    let mut times = Vec::new();
    let mut rn_out = Vec::with_capacity(w.len());
    for block in w.chunks_exact(FRAME) {
        inb.iter_mut().zip(block).for_each(|(d, s)| *d = s * 32767.0);
        let t = Instant::now();
        st.process_frame(&mut outb, &inb);
        times.push(t.elapsed());
        rn_out.extend(outb.iter().map(|v| v / 32767.0));
    }
    report("rnnoise", times);
    println!("rnnoise: delay {:.1} ms", delay_ms(&w[..48_000 * 3], &rn_out[..48_000 * 3]));

    let r0 = rss_mb();
    let t = Instant::now();
    let mut df = df::tract::DfTract::new(
        df::tract::DfParams::default(),
        &df::tract::RuntimeParams::default_with_ch(1),
    )
    .expect("load DeepFilterNet");
    println!("deepfilter ({}): load {:?}, rss +{:.1} MB, hop {}, lookahead {}, fft {}",
        if cfg!(feature = "dfn-ll") { "low-latency" } else { "normal" },
        t.elapsed(), rss_mb() - r0, df.hop_size, df.lookahead, df.fft_size);
    assert_eq!(df.hop_size, FRAME);
    let mut noisy = ndarray::Array2::<f32>::zeros((1, FRAME));
    let mut enh = ndarray::Array2::<f32>::zeros((1, FRAME));
    let mut times = Vec::new();
    let mut df_out = Vec::with_capacity(w.len());
    for block in w.chunks_exact(FRAME) {
        noisy.row_mut(0).iter_mut().zip(block).for_each(|(d, s)| *d = *s);
        let t = Instant::now();
        df.process(noisy.view(), enh.view_mut()).expect("df process");
        times.push(t.elapsed());
        df_out.extend(enh.row(0).iter().copied());
    }
    report("deepfilter", times);
    println!("deepfilter: delay {:.1} ms", delay_ms(&w[..48_000 * 3], &df_out[..48_000 * 3]));
}

/// Strong's model is built on a helper thread and handed to the processor: it must be Send.
#[test]
fn deepfilter_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<df::tract::DfTract>();
}
```

- [ ] **Step 4: Run it, both models.**

```bash
cd client/src-tauri
cargo test --release --test denoise_bench -- --include-ignored --nocapture 2>&1 | grep -E "rnnoise|deepfilter|test result"
cargo test --release --features dfn-ll --test denoise_bench -- --include-ignored --nocapture 2>&1 | grep -E "deepfilter|test result"
```

Expected: both pass; numbers printed. If `deepfilter_is_send` fails to compile, record it — Task 6 then loads the model on the processor thread instead of a helper (see Task 6 note).

- [ ] **Step 5: Windows cross-check with libDF.**

Run: `cd ~/Projects/pulse-app && XWIN_ACCEPT_LICENSE=1 cargo xwin check -p pulse-client --target x86_64-pc-windows-msvc 2>&1 | tail -3`
Expected: `Finished`. A failure here is a stop: report to Greg before continuing.

- [ ] **Step 6: Decide and record.** Append to `spikes/voice/FINDINGS.md`:

```markdown
## Noise suppression measurements (2026-10-06, this laptop, release build)

| Engine | mean / p99 / max per 10 ms | % of budget | load | RSS added | delay |
|---|---|---|---|---|---|
| RNNoise | … | … | — | — | … ms |
| DeepFilterNet3 normal (8 MB) | … | … | … | … MB | … ms |
| DeepFilterNet3 low-latency (36 MB) | … | … | … | … MB | … ms |

Windows cross-check with libDF: pass/fail.

**Decisions:** Strong = <normal|low-latency> because …; default level = <strong|standard> because …
```

Fill every `…` from Step 4 output. Decision rules: default = Strong if DeepFilterNet's mean ≤ 20% of budget (2 ms) and p99 ≤ 50% (5 ms); otherwise Standard. Strong uses low-latency only if it saves ≥ 15 ms delay at ≤ 1.5× the normal model's CPU (it costs 28 MB more); otherwise normal. If DeepFilterNet's mean > 50% of budget, STOP and report to Greg (the `ort` fallback in the spec). If Strong = low-latency, add `default = ["dfn-ll"]` to `[features]`.

- [ ] **Step 7: Commit.**

```bash
cd ~/Projects/pulse-app && just check && git add client/src-tauri/Cargo.toml Cargo.lock client/src-tauri/tests spikes/voice/FINDINGS.md && git commit -m "spike(voice): denoiser deps, speech fixture, measurements

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Denoisers

**Files:**
- Create: `client/src-tauri/src/voice/denoise.rs`
- Modify: `client/src-tauri/src/voice/mod.rs` (add `pub mod denoise;` after `pub mod controls;`)

**Interfaces:**
- Produces:
  - `pub const FRAME: usize = 480;`
  - `pub enum NsLevel { Off, Standard, Strong }` — `Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize`, `#[serde(rename_all = "snake_case")]`, `#[default]` = the Task 1 decision (Strong unless Task 1 chose Standard).
  - `pub trait Denoiser: Send { fn process(&mut self, block: &mut [f32]) -> Option<f32>; }` — `block.len() == FRAME`; returns speech probability in [0, 1], or `None` when the engine has no model (Off).
  - `pub struct Passthrough;` `pub struct Rnnoise;` (`Rnnoise::new()`), `pub struct DeepFilter;` (`DeepFilter::new() -> anyhow::Result<Self>`)
  - `pub fn make_fast(level: NsLevel) -> Box<dyn Denoiser>` — Off → Passthrough, Standard and Strong → Rnnoise (Strong's model is loaded separately, see Task 6).

- [ ] **Step 1: Write the failing tests** at the bottom of `denoise.rs` (create the file with just the tests module and `use` lines first):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn speech() -> Vec<f32> {
        include_bytes!("../../tests/fixtures/speech_48k_mono_s16le.raw")
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect()
    }

    fn noise(n: usize, amp: f32) -> Vec<f32> {
        let mut x: u32 = 7;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f32 / u32::MAX as f32 - 0.5) * 2.0 * amp
            })
            .collect()
    }

    /// Keyboard-like: a 2 ms decaying burst every 150 ms on near-silence.
    fn clicks(n: usize) -> Vec<f32> {
        let floor = noise(n, 0.001);
        (0..n)
            .map(|i| {
                let k = i % 7200;
                floor[i] + if k < 96 { 0.5 * (1.0 - k as f32 / 96.0) * if k % 2 == 0 { 1.0 } else { -1.0 } } else { 0.0 }
            })
            .collect()
    }

    fn run(d: &mut dyn Denoiser, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut out = Vec::new();
        let mut probs = Vec::new();
        for block in input.chunks_exact(FRAME) {
            let mut b = block.to_vec();
            if let Some(p) = d.process(&mut b) {
                probs.push(p);
            }
            out.extend(b);
        }
        (out, probs)
    }

    /// RMS in dB, skipping the first 0.5 s (models warm up).
    fn level_db(x: &[f32]) -> f32 {
        let x = &x[24_000.min(x.len())..];
        20.0 * ((x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt().max(1e-9)).log10()
    }

    fn engines() -> Vec<(&'static str, Box<dyn Denoiser>)> {
        vec![
            ("standard", Box::new(Rnnoise::new())),
            ("strong", Box::new(DeepFilter::new().expect("model loads"))),
        ]
    }

    #[test]
    fn every_engine_keeps_frame_size_and_probability_range() {
        let mut all = engines();
        all.push(("off", Box::new(Passthrough)));
        for (name, mut d) in all {
            let (out, probs) = run(d.as_mut(), &speech());
            assert_eq!(out.len(), speech().len() / FRAME * FRAME, "{name}");
            assert!(probs.iter().all(|p| (0.0..=1.0).contains(p)), "{name}");
        }
    }

    #[test]
    fn off_is_bit_exact_and_has_no_probability() {
        let input = speech();
        let (out, probs) = run(&mut Passthrough, &input);
        assert_eq!(out, input[..out.len()]);
        assert!(probs.is_empty());
    }

    #[test]
    fn strong_removes_clicks_and_standard_reduces_them() {
        let input = clicks(48_000 * 3);
        let before = level_db(&input);
        let (std_out, _) = run(&mut Rnnoise::new(), &input);
        let (strong_out, _) = run(&mut DeepFilter::new().unwrap(), &input);
        assert!(level_db(&std_out) < before, "standard: {} -> {}", before, level_db(&std_out));
        assert!(before - level_db(&strong_out) >= 15.0, "strong only cut {:.1} dB", before - level_db(&strong_out));
    }

    #[test]
    fn speech_is_preserved_within_3_db() {
        let input = speech();
        for (name, mut d) in engines() {
            let (out, _) = run(d.as_mut(), &input);
            let change = level_db(&out) - level_db(&input[..out.len()]);
            assert!(change.abs() <= 3.0, "{name}: speech level changed {change:.1} dB");
        }
    }

    #[test]
    fn stationary_noise_is_reduced_by_10_db() {
        let input = noise(48_000 * 3, 0.05);
        for (name, mut d) in engines() {
            let (out, _) = run(d.as_mut(), &input);
            let cut = level_db(&input) - level_db(&out);
            assert!(cut >= 10.0, "{name}: only {cut:.1} dB");
        }
    }

    #[test]
    fn speech_scores_higher_than_noise() {
        for (name, mut d) in engines() {
            let (_, sp) = run(d.as_mut(), &speech());
            let (_, np) = run(d.as_mut(), &noise(48_000 * 3, 0.05));
            let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
            assert!(mean(&sp) > 0.5 && mean(&np) < 0.5, "{name}: speech {:.2} noise {:.2}", mean(&sp), mean(&np));
        }
    }

    #[test]
    fn make_fast_never_loads_the_big_model() {
        assert!(make_fast(NsLevel::Off).process(&mut [0.0; FRAME]).is_none());
        assert!(make_fast(NsLevel::Strong).process(&mut [0.0; FRAME]).is_some());
    }
}
```

- [ ] **Step 2: Run to verify failure.**

Run: `cd ~/Projects/pulse-app && cargo test -p pulse-client --lib denoise 2>&1 | grep -E "^error" | head -3`
Expected: compile errors (`cannot find type Rnnoise` …).

- [ ] **Step 3: Implement** (top of `denoise.rs`, above the tests):

```rust
//! Noise suppression engines behind one interface. All take one 10 ms block (480 samples, 48 kHz,
//! mono, f32 in [-1, 1]) in place and return a speech probability.

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};

pub const FRAME: usize = 480;

/// RNNoise works on i16-scaled floats.
const I16_SCALE: f32 = 32767.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NsLevel {
    Off,
    Standard,
    #[default]
    Strong,
}

pub trait Denoiser: Send {
    /// Clean `block` in place; speech probability, or `None` without a model (Off).
    fn process(&mut self, block: &mut [f32]) -> Option<f32>;
}

/// Off: no suppression at all.
pub struct Passthrough;

impl Denoiser for Passthrough {
    fn process(&mut self, _block: &mut [f32]) -> Option<f32> {
        None
    }
}

/// Standard: RNNoise (light; softens clicks). Its voice-activity output is the probability.
pub struct Rnnoise {
    st: Box<nnnoiseless::DenoiseState<'static>>,
    inb: Vec<f32>,
    outb: Vec<f32>,
}

impl Rnnoise {
    pub fn new() -> Self {
        Self {
            st: nnnoiseless::DenoiseState::new(),
            inb: vec![0.0; FRAME],
            outb: vec![0.0; FRAME],
        }
    }

    /// Speech probability for `block` without changing it (Strong uses this as its VAD).
    fn probability(&mut self, block: &[f32]) -> f32 {
        self.inb.iter_mut().zip(block).for_each(|(d, s)| *d = s * I16_SCALE);
        self.st.process_frame(&mut self.outb, &self.inb)
    }
}

impl Default for Rnnoise {
    fn default() -> Self {
        Self::new()
    }
}

impl Denoiser for Rnnoise {
    fn process(&mut self, block: &mut [f32]) -> Option<f32> {
        let p = self.probability(block);
        block.iter_mut().zip(&self.outb).for_each(|(o, v)| *o = v / I16_SCALE);
        Some(p)
    }
}

/// Strong: DeepFilterNet3 for the audio; RNNoise alongside only for the speech probability
/// (DeepFilterNet reports a local SNR, not a probability).
pub struct DeepFilter {
    df: df::tract::DfTract,
    vad: Rnnoise,
    noisy: ndarray::Array2<f32>,
    enh: ndarray::Array2<f32>,
}

impl DeepFilter {
    /// Loads the bundled model (a few hundred ms; call off the audio path).
    pub fn new() -> anyhow::Result<Self> {
        let df = std::panic::catch_unwind(|| {
            df::tract::DfTract::new(
                df::tract::DfParams::default(),
                &df::tract::RuntimeParams::default_with_ch(1),
            )
        })
        .map_err(|_| anyhow!("DeepFilterNet panicked while loading its model"))?
        .context("loading DeepFilterNet")?;
        anyhow::ensure!(df.hop_size == FRAME, "unexpected DeepFilterNet hop size {}", df.hop_size);
        Ok(Self {
            df,
            vad: Rnnoise::new(),
            noisy: ndarray::Array2::zeros((1, FRAME)),
            enh: ndarray::Array2::zeros((1, FRAME)),
        })
    }
}

impl Denoiser for DeepFilter {
    fn process(&mut self, block: &mut [f32]) -> Option<f32> {
        let p = self.vad.probability(block);
        self.noisy.row_mut(0).iter_mut().zip(block.iter()).for_each(|(d, s)| *d = *s);
        // On a model error, send the block uncleaned rather than silence.
        if self.df.process(self.noisy.view(), self.enh.view_mut()).is_ok() {
            block.iter_mut().zip(self.enh.row(0).iter()).for_each(|(o, v)| *o = *v);
        }
        Some(p)
    }
}

/// An engine that's instant to build: Strong starts on RNNoise until its model has loaded.
pub fn make_fast(level: NsLevel) -> Box<dyn Denoiser> {
    match level {
        NsLevel::Off => Box::new(Passthrough),
        NsLevel::Standard | NsLevel::Strong => Box::new(Rnnoise::new()),
    }
}
```

If Task 1 chose Standard as the default level, move `#[default]` to `Standard`.

- [ ] **Step 4: Run tests (release: DeepFilterNet is slow in debug).**

Run: `cd ~/Projects/pulse-app && cargo test -p pulse-client --release --lib denoise 2>&1 | grep -E "^test |panicked|test result"`
Expected: 7 passed. If `speech_scores_higher_than_noise` or the 3 dB/10 dB/15 dB bounds fail, print the measured numbers and report them to Greg before changing a threshold — the thresholds come from the spec.

- [ ] **Step 5: Commit** (`just check`; then `git add client/src-tauri/src/voice/denoise.rs client/src-tauri/src/voice/mod.rs && git commit -m "feat(voice): RNNoise and DeepFilterNet denoisers behind one interface" ` + attribution line).

---

### Task 3: Voice gate

**Files:**
- Create: `client/src-tauri/src/voice/gate.rs`
- Modify: `client/src-tauri/src/voice/mod.rs` (add `pub mod gate;`)

**Interfaces:**
- Produces:
  - `pub struct VoiceGate` with `pub fn new(auto: bool, threshold: f32) -> Self`, `pub fn configure(&mut self, auto: bool, threshold: f32)`, `pub fn process(&mut self, block: &mut [f32], prob: Option<f32>, now: Instant) -> bool` (returns whether the emitted block carries audio), `pub fn reset(&mut self)` (drops pre-roll + fade state: used on mute).
  - Behaviour: output is the input delayed by 2 blocks (pre-roll); the open/close decision uses the newest block.

- [ ] **Step 1: Write the failing tests** (`gate.rs` tests module):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn block(amp: f32) -> Vec<f32> {
        (0..FRAME).map(|i| if i % 2 == 0 { amp } else { -amp }).collect()
    }
    fn rms(b: &[f32]) -> f32 {
        (b.iter().map(|v| v * v).sum::<f32>() / b.len() as f32).sqrt()
    }
    /// Feed `n` blocks of `amp`/`prob` starting at block index `*k`; returns the output blocks.
    fn feed(g: &mut VoiceGate, t0: Instant, k: &mut u64, n: usize, amp: f32, prob: Option<f32>) -> Vec<Vec<f32>> {
        (0..n)
            .map(|_| {
                let mut b = block(amp);
                g.process(&mut b, prob, t0 + Duration::from_millis(*k * 10));
                *k += 1;
                b
            })
            .collect()
    }

    #[test]
    fn auto_opens_on_loud_speech_after_learning_the_room() {
        let (t0, mut k) = (Instant::now(), 0);
        let mut g = VoiceGate::new(true, 0.02);
        feed(&mut g, t0, &mut k, 100, 0.001, Some(0.0)); // 1 s of quiet room
        let out = feed(&mut g, t0, &mut k, 10, 0.1, Some(0.9));
        assert!(rms(out.last().unwrap()) > 0.05, "speech should pass");
    }

    #[test]
    fn auto_ignores_speech_barely_above_the_floor() {
        let (t0, mut k) = (Instant::now(), 0);
        let mut g = VoiceGate::new(true, 0.02);
        feed(&mut g, t0, &mut k, 100, 0.01, Some(0.0)); // room at 0.01 rms
        let out = feed(&mut g, t0, &mut k, 10, 0.012, Some(0.9)); // distant voice, +1.6 dB
        assert!(out.iter().all(|b| rms(b) == 0.0), "quiet background speech must not open the gate");
    }

    #[test]
    fn auto_ignores_loud_non_speech() {
        let (t0, mut k) = (Instant::now(), 0);
        let mut g = VoiceGate::new(true, 0.02);
        feed(&mut g, t0, &mut k, 100, 0.001, Some(0.0));
        let out = feed(&mut g, t0, &mut k, 10, 0.3, Some(0.1)); // a loud click/clatter
        assert!(out.iter().all(|b| rms(b) == 0.0));
    }

    #[test]
    fn hysteresis_keeps_it_open_between_close_and_open_probabilities() {
        let (t0, mut k) = (Instant::now(), 0);
        let mut g = VoiceGate::new(true, 0.02);
        feed(&mut g, t0, &mut k, 100, 0.001, Some(0.0));
        feed(&mut g, t0, &mut k, 5, 0.1, Some(0.9));
        let out = feed(&mut g, t0, &mut k, 60, 0.1, Some(0.45)); // 600 ms: past the hold
        assert!(rms(out.last().unwrap()) > 0.05, "0.45 is above the close threshold");
    }

    #[test]
    fn holds_300ms_then_fades_over_100ms() {
        let (t0, mut k) = (Instant::now(), 0);
        let mut g = VoiceGate::new(true, 0.02);
        feed(&mut g, t0, &mut k, 100, 0.001, Some(0.0));
        feed(&mut g, t0, &mut k, 10, 0.1, Some(0.9));
        let out = feed(&mut g, t0, &mut k, 50, 0.1, Some(0.0)); // speech prob drops
        // 2 blocks of pre-roll delay + 30 blocks hold: still full level
        assert!((rms(&out[25]) - 0.1).abs() < 1e-3, "inside hold");
        let fading = rms(&out[37]);
        assert!(fading > 0.0 && fading < 0.09, "fading: {fading}");
        assert_eq!(rms(&out[45]), 0.0, "closed after 100 ms fade");
    }

    #[test]
    fn preroll_releases_the_20ms_before_the_gate_opened() {
        let (t0, mut k) = (Instant::now(), 0);
        let mut g = VoiceGate::new(false, 0.05); // manual: open at rms >= 0.05
        feed(&mut g, t0, &mut k, 10, 0.0, None);
        let mut onset = feed(&mut g, t0, &mut k, 1, 0.03, None); // soft first syllable (below threshold)
        onset.extend(feed(&mut g, t0, &mut k, 1, 0.03, None));
        let out = feed(&mut g, t0, &mut k, 3, 0.1, None); // loud: opens
        assert!(rms(&out[1]) > 0.02 && rms(&out[2]) > 0.02, "the soft onset is sent, not clipped");
    }

    #[test]
    fn manual_threshold_zero_is_always_open() {
        let (t0, mut k) = (Instant::now(), 0);
        let mut g = VoiceGate::new(false, 0.0);
        let out = feed(&mut g, t0, &mut k, 5, 0.001, None);
        assert!(rms(&out[4]) > 0.0);
    }

    #[test]
    fn off_level_without_probability_gates_on_level_above_floor() {
        let (t0, mut k) = (Instant::now(), 0);
        let mut g = VoiceGate::new(true, 0.02);
        feed(&mut g, t0, &mut k, 100, 0.001, None);
        let out = feed(&mut g, t0, &mut k, 10, 0.1, None);
        assert!(rms(out.last().unwrap()) > 0.05);
    }

    #[test]
    fn floor_adapts_up_to_a_louder_room() {
        let (t0, mut k) = (Instant::now(), 0);
        let mut g = VoiceGate::new(true, 0.02);
        feed(&mut g, t0, &mut k, 100, 0.001, Some(0.0));
        feed(&mut g, t0, &mut k, 1500, 0.02, Some(0.0)); // 15 s in a noisier room
        let out = feed(&mut g, t0, &mut k, 10, 0.024, Some(0.9)); // +1.6 dB over the new room
        assert!(out.iter().all(|b| rms(b) == 0.0), "floor should have risen to the room");
    }
}
```

- [ ] **Step 2: Run to verify failure.** Run: `cargo test -p pulse-client --lib gate:: 2>&1 | grep -E "^error" | head -2` → compile errors.

- [ ] **Step 3: Implement** (top of `gate.rs`):

```rust
//! Voice gate: decides when the mic is "on". Automatic mode opens on speech (model probability with
//! hysteresis) that is also clearly above the room's noise floor; manual mode on an RMS threshold.
//! Both hold 300 ms, fade out over 100 ms, and keep 20 ms of pre-roll so first syllables survive.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::denoise::FRAME;

const OPEN_PROB: f32 = 0.6;
const CLOSE_PROB: f32 = 0.35;
const FLOOR_MARGIN_DB: f32 = 6.0;
const FLOOR_MIN_DB: f32 = -80.0;
/// The floor snaps down to any quieter block and rises towards louder rooms: ~10 dB/s while the
/// model says "not speech", ~1 dB/s at level Off (no model to ask). Never above the current level.
const FLOOR_RISE_QUIET_DB: f32 = 0.1;
const FLOOR_RISE_UNKNOWN_DB: f32 = 0.01;
const HOLD: Duration = Duration::from_millis(300);
const FADE_BLOCKS: f32 = 10.0;
const PREROLL_BLOCKS: usize = 2;

pub struct VoiceGate {
    auto: bool,
    threshold: f32,
    speech: bool,
    /// Room noise floor in dB; learned from the first block.
    floor_db: Option<f32>,
    open_until: Option<Instant>,
    /// Gain applied at the start of the next emitted block (fades towards 0 when closed).
    gain: f32,
    delay: VecDeque<Vec<f32>>,
}

fn rms(b: &[f32]) -> f32 {
    (b.iter().map(|v| v * v).sum::<f32>() / b.len().max(1) as f32).sqrt()
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-6).log10()
}

impl VoiceGate {
    pub fn new(auto: bool, threshold: f32) -> Self {
        Self {
            auto,
            threshold,
            speech: false,
            floor_db: None,
            open_until: None,
            gain: 0.0,
            delay: (0..PREROLL_BLOCKS).map(|_| vec![0.0; FRAME]).collect(),
        }
    }

    pub fn configure(&mut self, auto: bool, threshold: f32) {
        self.auto = auto;
        self.threshold = threshold;
    }

    /// Forget buffered audio and any open state (mute: nothing may leak afterwards).
    pub fn reset(&mut self) {
        self.open_until = None;
        self.gain = 0.0;
        self.speech = false;
        self.delay.iter_mut().for_each(|b| b.fill(0.0));
    }

    fn qualifies(&mut self, level: f32, prob: Option<f32>) -> bool {
        if !self.auto {
            return self.threshold <= 0.0 || level >= self.threshold;
        }
        let level_db = db(level).max(FLOOR_MIN_DB);
        let floor = *self.floor_db.get_or_insert(level_db);
        let above_floor = level_db > floor + FLOOR_MARGIN_DB;
        let rise = match prob {
            Some(p) if p >= CLOSE_PROB => 0.0, // maybe us talking: don't learn from it
            Some(_) => FLOOR_RISE_QUIET_DB,
            None => FLOOR_RISE_UNKNOWN_DB,
        };
        self.floor_db = Some(if level_db < floor { level_db } else { (floor + rise).min(level_db) });
        match prob {
            Some(p) if p >= OPEN_PROB => self.speech = true,
            Some(p) if p < CLOSE_PROB => self.speech = false,
            Some(_) => {}
            None => self.speech = true,
        }
        self.speech && above_floor
    }

    /// Gate one block in place (input in, the block from 20 ms ago out). Returns whether audio is sent.
    pub fn process(&mut self, block: &mut [f32], prob: Option<f32>, now: Instant) -> bool {
        if self.qualifies(rms(block), prob) {
            self.open_until = Some(now + HOLD);
        }
        let open = self.open_until.is_some_and(|t| now < t);
        self.delay.push_back(block.to_vec());
        let out = self.delay.pop_front().expect("pre-roll buffer");
        let start = self.gain;
        let end = if open { 1.0 } else { (start - 1.0 / FADE_BLOCKS).max(0.0) };
        let n = out.len() as f32;
        for (i, (o, v)) in block.iter_mut().zip(&out).enumerate() {
            *o = v * (start + (end - start) * i as f32 / n);
        }
        self.gain = end;
        start > 0.0 || end > 0.0
    }
}
```

Note on fade-in: opening ramps from the current gain to 1 across one 10 ms block, which avoids a click without clipping the onset (the pre-roll already contains it).

- [ ] **Step 4: Run.** `cargo test -p pulse-client --lib gate:: 2>&1 | grep -E "^test |panicked|test result"` → 9 passed.

- [ ] **Step 5: Commit** (`just check`; `git add client/src-tauri/src/voice/gate.rs client/src-tauri/src/voice/mod.rs`; message `feat(voice): speech-driven voice gate with floor tracking, hold, fade and pre-roll` + attribution).

---

### Task 4: Overload detector and the per-block chain

**Files:**
- Create: `client/src-tauri/src/voice/processor.rs` (this task: pure parts only)
- Modify: `client/src-tauri/src/voice/mod.rs` (add `pub mod processor;`)

**Interfaces:**
- Consumes: `Denoiser`, `NsLevel`, `FRAME`, `make_fast` (Task 2); `VoiceGate` (Task 3); `mixer::soft_limit(f32) -> f32` (existing).
- Produces:
  - `pub struct OverloadDetector` — `new()`, `record(&mut self, took: Duration) -> bool` (true once the full 300-block window's mean exceeds 7 ms), `reset(&mut self)`.
  - `pub struct MicChain` — `new(level: NsLevel, auto: bool, threshold: f32, gain: f32) -> Self`; `set_denoiser(&mut self, level: NsLevel, d: Box<dyn Denoiser>)`; `level(&self) -> NsLevel`; `configure(&mut self, auto: bool, threshold: f32, gain: f32)`; `process(&mut self, block: &mut [f32], mic_open: bool, now: Instant) -> BlockOut`.
  - `pub struct BlockOut { pub level: f32, pub sent: bool }` — `level` = RMS after cleaning + gain (mic meter), `sent` = gate passed audio and mic is open.

- [ ] **Step 1: Failing tests** (`processor.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::denoise::{Denoiser, FRAME, NsLevel};
    use std::time::{Duration, Instant};

    #[test]
    fn one_spike_does_not_downgrade() {
        let mut o = OverloadDetector::new();
        let mut tripped = false;
        for i in 0..600 {
            let took = if i == 350 { Duration::from_millis(40) } else { Duration::from_millis(2) };
            tripped |= o.record(took);
        }
        assert!(!tripped);
    }

    #[test]
    fn sustained_overload_downgrades_after_a_full_window() {
        let mut o = OverloadDetector::new();
        let first = (1..=600).find(|_| o.record(Duration::from_millis(8)));
        assert_eq!(first, Some(300), "trips exactly when 3 s of blocks are in");
    }

    #[test]
    fn under_budget_never_downgrades() {
        let mut o = OverloadDetector::new();
        assert!((0..2000).all(|_| !o.record(Duration::from_micros(6500))));
    }

    struct Loud;
    impl Denoiser for Loud {
        fn process(&mut self, block: &mut [f32]) -> Option<f32> {
            block.fill(0.9);
            Some(1.0)
        }
    }

    #[test]
    fn loud_input_with_max_gain_stays_in_range() {
        let mut c = MicChain::new(NsLevel::Off, false, 0.0, 4.0);
        c.set_denoiser(NsLevel::Standard, Box::new(Loud));
        let t0 = Instant::now();
        for k in 0..10 {
            let mut b = vec![0.9f32; FRAME];
            c.process(&mut b, true, t0 + Duration::from_millis(k * 10));
            assert!(b.iter().all(|v| v.abs() <= 1.0), "block {k} exceeds full scale");
        }
    }

    #[test]
    fn mute_silences_immediately_including_preroll() {
        let mut c = MicChain::new(NsLevel::Off, false, 0.0, 1.0); // gate always open
        let t0 = Instant::now();
        for k in 0..5 {
            let mut b = vec![0.5f32; FRAME];
            c.process(&mut b, true, t0 + Duration::from_millis(k * 10));
        }
        for k in 5..10 {
            let mut b = vec![0.5f32; FRAME];
            let out = c.process(&mut b, false, t0 + Duration::from_millis(k * 10));
            assert!(b.iter().all(|v| *v == 0.0) && !out.sent, "block {k} leaked after mute");
        }
    }

    #[test]
    fn meter_level_is_after_cleaning_and_gain() {
        let mut c = MicChain::new(NsLevel::Off, false, 0.0, 2.0);
        let mut b = vec![0.1f32; FRAME];
        let out = c.process(&mut b, true, Instant::now());
        assert!((out.level - 0.2).abs() < 1e-3, "{}", out.level);
    }
}
```

- [ ] **Step 2: Run, expect compile errors** (`cargo test -p pulse-client --lib processor 2>&1 | grep -E "^error" | head -2`).

- [ ] **Step 3: Implement** (top of `processor.rs`):

```rust
//! Mic processing off the audio callback: resample → APM → denoiser → gain → gate, per 10 ms block.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::denoise::{Denoiser, NsLevel, make_fast};
use super::gate::VoiceGate;
use super::mixer::soft_limit;

const OVERLOAD_WINDOW: usize = 300; // 3 s of 10 ms blocks
const OVERLOAD_MEAN: Duration = Duration::from_micros(7_000); // 70% of the 10 ms budget

/// Detects a PC that can't keep up: the mean over a full 3 s window above 70% of the budget.
pub struct OverloadDetector {
    times: VecDeque<Duration>,
    sum: Duration,
}

impl OverloadDetector {
    pub fn new() -> Self {
        Self { times: VecDeque::with_capacity(OVERLOAD_WINDOW), sum: Duration::ZERO }
    }

    pub fn reset(&mut self) {
        self.times.clear();
        self.sum = Duration::ZERO;
    }

    pub fn record(&mut self, took: Duration) -> bool {
        self.times.push_back(took);
        self.sum += took;
        if self.times.len() > OVERLOAD_WINDOW {
            self.sum -= self.times.pop_front().unwrap_or_default();
        }
        self.times.len() == OVERLOAD_WINDOW && self.sum / OVERLOAD_WINDOW as u32 > OVERLOAD_MEAN
    }
}

impl Default for OverloadDetector {
    fn default() -> Self {
        Self::new()
    }
}

pub struct BlockOut {
    /// RMS after cleaning + gain (the mic meter: what you see is what is sent).
    pub level: f32,
    /// Whether audio (not silence) went out.
    pub sent: bool,
}

/// One block's journey after APM: denoise → gain (soft-limited) → gate → mute.
pub struct MicChain {
    level: NsLevel,
    denoiser: Box<dyn Denoiser>,
    gate: VoiceGate,
    gain: f32,
}

impl MicChain {
    pub fn new(level: NsLevel, auto: bool, threshold: f32, gain: f32) -> Self {
        Self { level, denoiser: make_fast(level), gate: VoiceGate::new(auto, threshold), gain }
    }

    pub fn level(&self) -> NsLevel {
        self.level
    }

    pub fn set_denoiser(&mut self, level: NsLevel, d: Box<dyn Denoiser>) {
        self.level = level;
        self.denoiser = d;
    }

    pub fn configure(&mut self, auto: bool, threshold: f32, gain: f32) {
        self.gate.configure(auto, threshold);
        self.gain = gain;
    }

    pub fn process(&mut self, block: &mut [f32], mic_open: bool, now: Instant) -> BlockOut {
        let prob = self.denoiser.process(block);
        for v in block.iter_mut() {
            *v = soft_limit(*v * self.gain);
        }
        let level = (block.iter().map(|v| v * v).sum::<f32>() / block.len() as f32).sqrt();
        if !mic_open {
            self.gate.reset();
            block.fill(0.0);
            return BlockOut { level, sent: false };
        }
        let sent = self.gate.process(block, prob, now);
        BlockOut { level, sent }
    }
}
```

- [ ] **Step 4: Run.** `cargo test -p pulse-client --lib processor 2>&1 | grep -E "^test |panicked|test result"` → 6 passed.

- [ ] **Step 5: Commit** (`feat(voice): overload detector and per-block mic chain` + attribution).

---

### Task 5: Settings model, APM noise suppression off, processor config + status in `Shared`

**Files:**
- Modify: `client/src-tauri/src/voice/devices.rs`

**Interfaces:**
- Consumes: `NsLevel` (Task 2).
- Produces:
  - `AudioConfig { …, pub noise_suppression: NsLevel, pub auto_sensitivity: bool, pub noise_suppress: Option<bool> /* legacy, read-only */ }` and `AudioConfig::normalized(self) -> Self`.
  - `pub struct ProcCfg { pub level: NsLevel, pub auto: bool, pub threshold: f32, pub gain: f32, pub version: u64 }` held in `Shared::proc_cfg: Mutex<ProcCfg>`.
  - `pub struct NsStatus { pub active: NsLevel, pub note: Option<String> }` (`Clone, Debug, PartialEq, Serialize`); `Shared::set_ns_status(&self, s: NsStatus)`, `Shared::take_ns_status_change(&self) -> Option<NsStatus>`.
  - `Shared::gate_open: AtomicBool`, `Shared::add_mic_level(&self, level: f32, sent: bool)` (feeds `mic_meter`, `sent_meter`, `gate_open`).
  - `Shared::apm` and `Shared::warn_apm_once` become `pub(crate)` (the processor calls them).

- [ ] **Step 1: Failing tests** (append to `devices.rs` tests):

```rust
    #[test]
    fn legacy_noise_toggle_off_migrates_to_off() {
        let old = r#"{"input":null,"output":null,"input_gain_pct":100,"sensitivity":0.02,
                      "echo_cancel":true,"noise_suppress":false,"auto_gain":false}"#;
        let cfg: AudioConfig = serde_json::from_str::<AudioConfig>(old).unwrap().normalized();
        assert_eq!(cfg.noise_suppression, NsLevel::Off);
        assert!(cfg.auto_sensitivity, "new field defaults on");
    }

    #[test]
    fn legacy_noise_toggle_on_gets_the_default_level() {
        let old = r#"{"input":null,"output":null,"input_gain_pct":100,"sensitivity":0.02,
                      "echo_cancel":true,"noise_suppress":true,"auto_gain":false}"#;
        let cfg: AudioConfig = serde_json::from_str::<AudioConfig>(old).unwrap().normalized();
        assert_eq!(cfg.noise_suppression, NsLevel::default());
    }

    #[test]
    fn ns_status_change_is_reported_once() {
        let s = Shared::new(&AudioConfig::default(), Default::default(), Arc::new(Mutex::new(Mixer::new(48_000, 200))));
        let st = NsStatus { active: NsLevel::Standard, note: Some("Strong unavailable".into()) };
        s.set_ns_status(st.clone());
        assert_eq!(s.take_ns_status_change(), Some(st.clone()));
        assert_eq!(s.take_ns_status_change(), None);
        s.set_ns_status(st);
        assert_eq!(s.take_ns_status_change(), None, "same status again is not a change");
    }

    #[test]
    fn apply_config_bumps_processor_version() {
        let s = Shared::new(&AudioConfig::default(), Default::default(), Arc::new(Mutex::new(Mixer::new(48_000, 200))));
        let v0 = s.proc_cfg.lock().unwrap().version;
        s.apply_config(&AudioConfig { noise_suppression: NsLevel::Off, ..Default::default() });
        let p = s.proc_cfg.lock().unwrap();
        assert!(p.version > v0);
        assert_eq!(p.level, NsLevel::Off);
    }
```

Add `use super::denoise::NsLevel;` to the tests module imports if not covered by `use super::*`.

- [ ] **Step 2: Run, expect compile errors** (`cargo test -p pulse-client --lib devices 2>&1 | grep -E "^error" | head -3`).

- [ ] **Step 3: Implement in `devices.rs`.**

Replace the `AudioConfig` struct and `Default` impl with:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioConfig {
    /// Device names; `None` = system default.
    pub input: Option<String>,
    pub output: Option<String>,
    /// 50..=400 (%), applied after noise suppression, soft-limited — for quiet mics.
    pub input_gain_pct: u16,
    /// Manual mode: RMS threshold below which the mic is gated (0 = always open).
    pub sensitivity: f32,
    pub echo_cancel: bool,
    #[serde(default)]
    pub noise_suppression: NsLevel,
    /// Gate on detected speech above the room's noise floor (else on `sensitivity`).
    #[serde(default = "default_true")]
    pub auto_sensitivity: bool,
    pub auto_gain: bool,
    /// Pre-2026-10 on/off noise toggle: only read, to migrate old settings (see `normalized`).
    #[serde(default, skip_serializing)]
    pub noise_suppress: Option<bool>,
}

fn default_true() -> bool {
    true
}

impl AudioConfig {
    /// Fold legacy fields into the current ones.
    pub fn normalized(mut self) -> Self {
        if self.noise_suppress.take() == Some(false) {
            self.noise_suppression = NsLevel::Off;
        }
        self
    }
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input: None,
            output: None,
            input_gain_pct: 100,
            sensitivity: 0.02,
            echo_cancel: true,
            noise_suppression: NsLevel::default(),
            auto_sensitivity: true,
            auto_gain: false,
            noise_suppress: None,
        }
    }
}

/// What the mic processor should run; `version` bumps on every change.
#[derive(Clone, Debug)]
pub struct ProcCfg {
    pub level: NsLevel,
    pub auto: bool,
    pub threshold: f32,
    pub gain: f32,
    pub version: u64,
}

/// Which suppression is actually running (it can differ from the chosen level), and why.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NsStatus {
    pub active: NsLevel,
    pub note: Option<String>,
}
```

Add `use super::denoise::NsLevel;` to the imports. Delete `const GATE_HOLD`, the whole `Gate` struct + impl and its tests (`gate_opens_on_speech_and_holds_300ms` and the "threshold 0 = always open" test) — `gate.rs` replaces them.

In `Shared`: remove fields `gate: Mutex<Gate>` and `gain: Mutex<f32>`; make `apm` `pub(crate) apm`; change `apm_cfg` to `Mutex<(bool, bool)>` (echo, agc); add:

```rust
    pub proc_cfg: Mutex<ProcCfg>,
    ns_status: Mutex<(Option<NsStatus>, bool)>, // (current, changed since last take)
    pub gate_open: std::sync::atomic::AtomicBool,
```

In `Shared::new`, replace the `gate`/`gain`/`apm`/`apm_cfg` initialisers with:

```rust
            apm: Mutex::new(AudioProcessingModule::new(cfg.echo_cancel, cfg.auto_gain, true, false)),
            apm_cfg: Mutex::new((cfg.echo_cancel, cfg.auto_gain)),
            proc_cfg: Mutex::new(ProcCfg {
                level: cfg.noise_suppression,
                auto: cfg.auto_sensitivity,
                threshold: cfg.sensitivity,
                gain: input_gain(cfg.input_gain_pct),
                version: 0,
            }),
            ns_status: Mutex::new((None, false)),
            gate_open: Default::default(),
```

Add methods to `impl Shared`:

```rust
    pub fn set_ns_status(&self, s: NsStatus) {
        let mut st = self.ns_status.lock().unwrap();
        if st.0.as_ref() != Some(&s) {
            *st = (Some(s), true);
        }
    }

    pub fn take_ns_status_change(&self) -> Option<NsStatus> {
        let mut st = self.ns_status.lock().unwrap();
        if !st.1 {
            return None;
        }
        st.1 = false;
        st.0.clone()
    }

    /// The processor's per-block report: mic meter, own speaking ring, gate indicator.
    pub fn add_mic_level(&self, level: f32, sent: bool) {
        self.mic_meter.lock().unwrap().add(level);
        self.sent_meter.lock().unwrap().add(if sent { level } else { 0.0 });
        self.gate_open.store(sent, Ordering::Relaxed);
    }
```

Change `warn_apm_once` from `fn` to `pub(crate) fn`. Replace `apply_config` with:

```rust
    pub fn apply_config(&self, cfg: &AudioConfig) {
        {
            let mut p = self.proc_cfg.lock().unwrap();
            *p = ProcCfg {
                level: cfg.noise_suppression,
                auto: cfg.auto_sensitivity,
                threshold: cfg.sensitivity,
                gain: input_gain(cfg.input_gain_pct),
                version: p.version + 1,
            };
        }
        let wanted = (cfg.echo_cancel, cfg.auto_gain);
        let mut current = self.apm_cfg.lock().unwrap();
        if *current != wanted {
            // WebRTC's own noise suppression stays off: Off means none, and suppressors never stack.
            *self.apm.lock().unwrap() =
                AudioProcessingModule::new(cfg.echo_cancel, cfg.auto_gain, true, false);
            *current = wanted;
        }
    }
```

`on_input` still references `self.gate`/`self.gain`: leave it compiling for now by replacing its body's gating with a temporary pass-through is NOT allowed — instead Task 6 rewrites `on_input`. To keep this task green, make the minimal edit in `on_input`: replace `let gain = *self.gain.lock().unwrap();` with `let gain = self.proc_cfg.lock().unwrap().gain;`, and replace `let speaking = self.gate.lock().unwrap().process(rms, now);` with `let speaking = true;` plus the comment `// gating moves to the processor (next commit)`; this intermediate state is committed only together with Task 6 (do not commit Task 5 alone).

Also update callers that construct `AudioConfig` with `noise_suppress:` (grep `noise_suppress` in `client/src-tauri`): use `noise_suppression: NsLevel::…` instead. In `commands.rs`, call `.normalized()` on every incoming `config: AudioConfig` before use (`set_audio_config`, `join_voice`, `start_mic_test`).

- [ ] **Step 4: Run.** `cargo test -p pulse-client --lib 2>&1 | grep -E "^error|FAILED|test result"` → all pass (the 4 new tests included).

- [ ] **Step 5: No commit yet** — continue directly with Task 6 (they commit together).

---

### Task 6: The processing thread, wired in

**Files:**
- Modify: `client/src-tauri/src/voice/processor.rs` (thread + tests)
- Modify: `client/src-tauri/src/voice/devices.rs` (`on_input`, `AudioIo::start`, `start_null`)
- Modify: `client/src-tauri/src/voice/mod.rs` (sessions use the processor)
- Modify: `client/src-tauri/src/voice/mictest.rs`

**Interfaces:**
- Consumes: Tasks 2–5.
- Produces:
  - `pub type RawBlock = (u32, Vec<f32>);` `pub type RawTx = std::sync::mpsc::Sender<RawBlock>;` (in `processor.rs`)
  - `pub fn spawn(shared: Arc<Shared>, mic_tx: mpsc::UnboundedSender<MicChunk>) -> (RawTx, std::thread::JoinHandle<()>)` — the thread exits when every `RawTx` is dropped.
  - `AudioIo::start(cfg, shared, raw_tx: RawTx)` and `AudioIo::start_null(rate, shared, raw_tx: RawTx)` (were `mic_tx`).
  - Processor output: `(48_000, Vec<i16>)` chunks of exactly 480 samples on `mic_tx`.

- [ ] **Step 1: Failing tests** (append to `processor.rs` tests):

```rust
    use crate::voice::devices::{AudioConfig, Shared};
    use crate::voice::mixer::Mixer;
    use std::sync::{Arc, Mutex};

    fn shared_with(cfg: AudioConfig) -> Arc<Shared> {
        Shared::new(&cfg, Default::default(), Arc::new(Mutex::new(Mixer::new(48_000, 200))))
    }

    fn recv_n(rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::voice::devices::MicChunk>, n: usize) -> Vec<(u32, usize)> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut got = Vec::new();
        while got.len() < n && Instant::now() < deadline {
            match rx.try_recv() {
                Ok((rate, buf)) => got.push((rate, buf.len())),
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        got
    }

    #[test]
    fn processor_emits_exact_48k_frames() {
        let (mic_tx, mut mic_rx) = tokio::sync::mpsc::unbounded_channel();
        let (raw_tx, _h) = spawn(shared_with(AudioConfig { noise_suppression: NsLevel::Off, ..Default::default() }), mic_tx);
        for _ in 0..10 {
            raw_tx.send((48_000, vec![0.0; 480])).unwrap();
        }
        assert_eq!(recv_n(&mut mic_rx, 10), vec![(48_000, 480); 10]);
    }

    #[test]
    fn processor_handles_rate_change() {
        let (mic_tx, mut mic_rx) = tokio::sync::mpsc::unbounded_channel();
        let (raw_tx, _h) = spawn(shared_with(AudioConfig { noise_suppression: NsLevel::Off, ..Default::default() }), mic_tx);
        for _ in 0..50 {
            raw_tx.send((16_000, vec![0.0; 160])).unwrap(); // BT call mode
        }
        for _ in 0..50 {
            raw_tx.send((48_000, vec![0.0; 480])).unwrap();
        }
        let got = recv_n(&mut mic_rx, 90);
        assert!(got.len() >= 90 && got.iter().all(|c| *c == (48_000, 480)), "{:?}", got.len());
    }

    #[test]
    fn processor_exits_when_senders_drop() {
        let (mic_tx, _mic_rx) = tokio::sync::mpsc::unbounded_channel();
        let (raw_tx, h) = spawn(shared_with(AudioConfig::default()), mic_tx);
        drop(raw_tx);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !h.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(h.is_finished(), "processor thread outlived its senders");
    }

    #[test]
    fn backlog_is_trimmed_to_60ms() {
        let mut q: VecDeque<u32> = (0..20).collect();
        let dropped = trim_backlog(&mut q);
        assert_eq!(dropped, 14);
        assert_eq!(q, (14..20).collect::<VecDeque<_>>(), "keeps the newest");
    }

    #[test]
    fn stale_model_load_is_ignored() {
        let mut s = Switcher::new(NsLevel::Off);
        let first = s.request(NsLevel::Strong); // starts a load, generation g1
        s.request(NsLevel::Off); // user flips back before it finishes
        assert!(!s.accept_loaded(first), "an outdated load must not be installed");
        let second = s.request(NsLevel::Strong);
        assert!(s.accept_loaded(second));
    }

    #[test]
    fn strong_load_failure_falls_back_to_standard_with_a_note() {
        let sh = shared_with(AudioConfig::default());
        report_load_failure(&sh, "boom");
        let st = sh.take_ns_status_change().unwrap();
        assert_eq!(st.active, NsLevel::Standard);
        assert!(st.note.unwrap().contains("Strong unavailable"));
    }
```

- [ ] **Step 2: Run, expect compile errors** (`cargo test -p pulse-client --lib processor 2>&1 | grep -E "^error" | head -3`).

- [ ] **Step 3: Implement the thread** (add to `processor.rs`, below `MicChain`; add imports `use std::sync::Arc; use std::sync::mpsc as std_mpsc; use tokio::sync::mpsc; use super::denoise::{DeepFilter, FRAME}; use super::devices::{INTERNAL_RATE, MicChunk, NsStatus, Shared}; use super::resampler::StreamResampler;`):

```rust
pub type RawBlock = (u32, Vec<f32>);
pub type RawTx = std_mpsc::Sender<RawBlock>;

/// At most 60 ms of raw audio may wait; older blocks are dropped so latency never grows.
const MAX_BACKLOG: usize = 6;

pub fn trim_backlog<T>(q: &mut VecDeque<T>) -> usize {
    let excess = q.len().saturating_sub(MAX_BACKLOG);
    q.drain(..excess);
    excess
}

/// Tracks which level is wanted, so a slow model load can't overwrite a newer choice.
pub struct Switcher {
    wanted: NsLevel,
    generation: u64,
}

impl Switcher {
    pub fn new(level: NsLevel) -> Self {
        Self { wanted: level, generation: 0 }
    }
    /// Record a new wanted level; returns the generation a load for it must present.
    pub fn request(&mut self, level: NsLevel) -> u64 {
        self.wanted = level;
        self.generation += 1;
        self.generation
    }
    pub fn wanted(&self) -> NsLevel {
        self.wanted
    }
    pub fn accept_loaded(&self, generation: u64) -> bool {
        generation == self.generation && self.wanted == NsLevel::Strong
    }
}

pub fn report_load_failure(shared: &Shared, err: &str) {
    tracing::warn!(error = err, "noise suppression: Strong failed to load, using Standard");
    shared.set_ns_status(NsStatus {
        active: NsLevel::Standard,
        note: Some("Strong unavailable, using Standard".into()),
    });
}

/// Start the mic processor. It runs until every returned sender is dropped.
pub fn spawn(shared: Arc<Shared>, mic_tx: mpsc::UnboundedSender<MicChunk>) -> (RawTx, std::thread::JoinHandle<()>) {
    let (tx, rx) = std_mpsc::channel::<RawBlock>();
    let h = std::thread::Builder::new()
        .name("pulse-mic".into())
        .spawn(move || run(shared, rx, mic_tx))
        .expect("spawn mic processor");
    (tx, h)
}

fn run(shared: Arc<Shared>, rx: std_mpsc::Receiver<RawBlock>, mic_tx: mpsc::UnboundedSender<MicChunk>) {
    let cfg = shared.proc_cfg.lock().unwrap().clone();
    let mut version = cfg.version;
    let mut chain = MicChain::new(cfg.level, cfg.auto, cfg.threshold, cfg.gain);
    let mut switcher = Switcher::new(cfg.level);
    let (load_tx, load_rx) = std_mpsc::channel::<(u64, anyhow::Result<DeepFilter>)>();
    let start_load = |generation: u64, tx: std_mpsc::Sender<(u64, anyhow::Result<DeepFilter>)>| {
        std::thread::spawn(move || {
            let _ = tx.send((generation, DeepFilter::new()));
        });
    };
    if cfg.level == NsLevel::Strong {
        start_load(switcher.request(NsLevel::Strong), load_tx.clone());
    }
    shared.set_ns_status(NsStatus { active: if cfg.level == NsLevel::Strong { NsLevel::Standard } else { cfg.level }, note: None });

    let mut overload = OverloadDetector::new();
    let mut resampler: Option<(u32, StreamResampler)> = None;
    let mut pending: Vec<f32> = Vec::with_capacity(2 * FRAME);
    let mut queue: VecDeque<RawBlock> = VecDeque::new();
    let mut i16buf = vec![0i16; FRAME];

    while let Ok(first) = rx.recv() {
        queue.push_back(first);
        queue.extend(rx.try_iter());
        trim_backlog(&mut queue);

        // live config
        let cfg = shared.proc_cfg.lock().unwrap().clone();
        if cfg.version != version {
            version = cfg.version;
            chain.configure(cfg.auto, cfg.threshold, cfg.gain);
            if cfg.level != switcher.wanted() {
                let g = switcher.request(cfg.level);
                chain.set_denoiser(cfg.level, make_fast(cfg.level));
                overload.reset();
                if cfg.level == NsLevel::Strong {
                    start_load(g, load_tx.clone());
                }
                let active = if cfg.level == NsLevel::Strong { NsLevel::Standard } else { cfg.level };
                shared.set_ns_status(NsStatus { active, note: None });
            }
        }
        // a finished model load
        while let Ok((g, result)) = load_rx.try_recv() {
            match result {
                Ok(df) if switcher.accept_loaded(g) => {
                    chain.set_denoiser(NsLevel::Strong, Box::new(df));
                    overload.reset();
                    shared.set_ns_status(NsStatus { active: NsLevel::Strong, note: None });
                }
                Ok(_) => {} // outdated: the user changed level meanwhile
                Err(e) if switcher.accept_loaded(g) => report_load_failure(&shared, &e.to_string()),
                Err(_) => {}
            }
        }

        for (rate, samples) in queue.drain(..) {
            if rate == INTERNAL_RATE {
                resampler = None;
                pending.extend_from_slice(&samples);
            } else {
                if resampler.as_ref().map(|(r, _)| *r) != Some(rate) {
                    resampler = Some((rate, StreamResampler::new(rate, INTERNAL_RATE)));
                }
                if let Some((_, rs)) = resampler.as_mut() {
                    rs.push(&samples, &mut pending);
                }
            }
        }

        while pending.len() >= FRAME {
            let mut block: Vec<f32> = pending.drain(..FRAME).collect();
            // WebRTC APM (HPF, AEC, AGC) works on i16
            for (d, v) in i16buf.iter_mut().zip(&block) {
                *d = (v * 32767.0).round().clamp(-32768.0, 32767.0) as i16;
            }
            let r = shared.apm.lock().unwrap().process_stream(&mut i16buf, INTERNAL_RATE as i32, 1);
            shared.warn_apm_once(r, "capture");
            for (d, v) in block.iter_mut().zip(&i16buf) {
                *d = *v as f32 / 32767.0;
            }

            let mic_open = shared.controls.lock().unwrap().mic_open();
            let t = Instant::now();
            let out = chain.process(&mut block, mic_open, t);
            if chain.level() == NsLevel::Strong && switcher.wanted() == NsLevel::Strong && overload.record(t.elapsed()) {
                tracing::warn!("noise suppression: this PC can't keep up with Strong, switching to Standard");
                chain.set_denoiser(NsLevel::Standard, make_fast(NsLevel::Standard));
                overload.reset();
                shared.set_ns_status(NsStatus {
                    active: NsLevel::Standard,
                    note: Some("Strong was too heavy for this PC, using Standard".into()),
                });
            }
            shared.add_mic_level(out.level, out.sent);
            let chunk: Vec<i16> = block
                .iter()
                .map(|v| (v * 32767.0).round().clamp(-32768.0, 32767.0) as i16)
                .collect();
            if mic_tx.send((INTERNAL_RATE, chunk)).is_err() {
                return; // session gone
            }
        }
    }
}
```

If Task 1 found `DfTract` is **not** `Send`, replace `start_load`'s helper thread with a synchronous `DeepFilter::new()` on this thread (accepting a one-time gap while it loads, absorbed by `trim_backlog`), and keep the `Switcher` checks.

- [ ] **Step 4: Rewrite the capture callback** in `devices.rs`. Replace the whole `on_input` method with:

```rust
    /// Capture path: downmix and hand 10 ms blocks to the mic processor. Nothing heavy here —
    /// a slow callback is an audible glitch.
    fn on_input(&self, data: &[f32], channels: usize, rate: u32, pending: &mut Vec<f32>, raw_tx: &RawTx) {
        self.watchdog.input_tick(Instant::now());
        let chunk = (rate / 100) as usize;
        for v in downmix(data, channels) {
            pending.push(v);
            if pending.len() == chunk {
                let _ = raw_tx.send((rate, std::mem::replace(pending, Vec::with_capacity(chunk))));
            }
        }
    }
```

Add `use super::processor::RawTx;`. In `AudioIo::start`, rename the parameter `mic_tx: mpsc::UnboundedSender<MicChunk>` to `raw_tx: RawTx`; in both input-stream closures pass `&raw_tx` and change `let mut pending = Vec::new();` to `let mut pending: Vec<f32> = Vec::new();`; the I16 input closure passes its converted `&f` unchanged. In `start_null`, change the parameter to `raw_tx: RawTx` and the send to `let _ = raw_tx.send((rate, vec![0f32; chunk]));`. Remove the now-unused `mpsc`/`MicChunk` imports if clippy flags them (keep `MicChunk` exported: `mod.rs` uses it).

- [ ] **Step 5: Wire sessions in `mod.rs`.**
  - `start_audio(cfg, shared, mic_tx: mpsc::UnboundedSender<MicChunk>)` → parameter `raw_tx: processor::RawTx`, passing it to `AudioIo::start`.
  - In `connect()`, right after `let (mic_tx, mut mic_rx) = mpsc::unbounded_channel::<MicChunk>();` add `let (raw_tx, _processor) = processor::spawn(shared.clone(), mic_tx.clone());` and change both device starts to use `raw_tx.clone()` (`start_audio(&cfg, shared.clone(), raw_tx.clone())`, `AudioIo::start_null(rate, shared.clone(), raw_tx.clone())`).
  - The watchdog task clones `(cfg.clone(), mic_tx.clone(), alive.clone())`: change `mic_tx` there to `raw_tx` and its reopen call to `start_audio(&cfg, shared.clone(), raw_tx.clone())`.
  - `Session` field `mic_tx: mpsc::UnboundedSender<MicChunk>` → `raw_tx: processor::RawTx` (and the struct literal `mic_tx,` → `raw_tx,`); `set_audio_config`'s reopen uses `s.raw_tx.clone()`.
  - Drop the extra `mic_tx` clone held only for reopen; the processor holds the sender to the mic→LiveKit task, which ends when the processor ends (all `RawTx` dropped on leave).

- [ ] **Step 6: Mic test.** In `mictest.rs` `start()`: after `let (tx, mut rx) = mpsc::unbounded_channel::<MicChunk>();` add `let (raw_tx, _proc) = super::processor::spawn(shared.clone(), tx);` and start devices with `AudioIo::start(cfg, shared.clone(), raw_tx)`. The loopback keeps using `rx` (chunks are now 48 kHz; `ToInternal` passes them through). The processor ends when the `AudioIo` (holding the last `RawTx`) is dropped on stop.

- [ ] **Step 7: Run everything.**

```bash
cd ~/Projects/pulse-app && cargo test -p pulse-client --lib 2>&1 | grep -E "^error|FAILED|panicked|test result"
just voice-it 2>&1 | grep -E "round|test result"
```

Expected: all lib tests pass (6 new processor tests); voice-it 2 passed with rounds ≈ 0.85–1.0×.

- [ ] **Step 8: Commit Tasks 5 + 6 together** (`just check`; `git add client/src-tauri/src`; message `feat(voice): mic processor thread — denoise levels, auto gate, live switching, safety nets` + attribution).

---

### Task 7: Events to the UI

**Files:**
- Modify: `client/src-tauri/src/voice/mod.rs`, `client/src-tauri/src/voice/mictest.rs`

**Interfaces:**
- Produces: `VoiceEvent::Levels { mic: f32, speaker: f32, gate_open: bool }`; new `VoiceEvent::NoiseSuppression { active: NsLevel, note: Option<String> }` (serialised `{"kind":"noise_suppression","active":"standard","note":…}`).

- [ ] **Step 1: Failing test** (`mod.rs` tests):

```rust
    #[test]
    fn noise_suppression_event_serialises_for_the_ui() {
        let e = VoiceEvent::NoiseSuppression {
            active: crate::voice::denoise::NsLevel::Standard,
            note: Some("Strong unavailable, using Standard".into()),
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"kind":"noise_suppression","active":"standard","note":"Strong unavailable, using Standard"})
        );
        let l = VoiceEvent::Levels { mic: 0.0, speaker: 0.0, gate_open: true };
        assert_eq!(serde_json::to_value(&l).unwrap()["gate_open"], true);
    }
```

- [ ] **Step 2: Run, expect compile error.**
- [ ] **Step 3: Implement.** Add the variant `NoiseSuppression { active: denoise::NsLevel, note: Option<String> },` and the field `gate_open: bool` to `Levels`. In the session levels loop, replace `events(VoiceEvent::Levels { mic, speaker });` with:

```rust
                    let gate_open = shared.gate_open.load(Ordering::Relaxed);
                    events(VoiceEvent::Levels { mic, speaker, gate_open });
                    if let Some(st) = shared.take_ns_status_change() {
                        events(VoiceEvent::NoiseSuppression { active: st.active, note: st.note });
                    }
```

Do the same in `mictest.rs`'s levels task (it has `shared` in scope; add `use std::sync::atomic::Ordering;`).

- [ ] **Step 4: Run** `cargo test -p pulse-client --lib 2>&1 | grep -E "FAILED|test result"` → pass.
- [ ] **Step 5: Commit** (`feat(voice): noise suppression status and gate-open events` + attribution).

---

### Task 8: Settings UI

**Files:**
- Modify: `client/ui/src/lib/voiceui.ts`, `client/ui/src/lib/voiceui.test.ts`, `client/ui/src/lib/voice.svelte.ts`, `client/ui/src/components/Settings.svelte`

**Interfaces:**
- Consumes: event shapes from Task 7; config fields from Task 5.
- Produces: `export type NsLevel = 'off' | 'standard' | 'strong'`; `AudioConfig` with `noise_suppression`, `auto_sensitivity` (no `noise_suppress`); `migrateAudioConfig(raw: Record<string, unknown>): AudioConfig`; `voice.ns: { active: NsLevel | null; note: string | null }`, `voice.gateOpen: boolean`.

- [ ] **Step 1: Failing tests** (append to `voiceui.test.ts`; import `migrateAudioConfig, defaultAudioConfig` from `./voiceui`):

```ts
describe('migrateAudioConfig', () => {
  it('maps the old noise toggle off to level off', () => {
    const c = migrateAudioConfig({ noise_suppress: false, sensitivity: 0.05 })
    expect(c.noise_suppression).toBe('off')
    expect(c.sensitivity).toBe(0.05)
    expect('noise_suppress' in c).toBe(false)
  })
  it('gives old toggle-on users the default level and auto sensitivity', () => {
    const c = migrateAudioConfig({ noise_suppress: true })
    expect(c.noise_suppression).toBe(defaultAudioConfig.noise_suppression)
    expect(c.auto_sensitivity).toBe(true)
  })
  it('keeps an explicit new level', () => {
    expect(migrateAudioConfig({ noise_suppression: 'standard', noise_suppress: false }).noise_suppression).toBe('standard')
  })
})
```

- [ ] **Step 2: Run** `cd client/ui && pnpm vitest run src/lib/voiceui.test.ts` → FAIL (`migrateAudioConfig` not exported).

- [ ] **Step 3: Implement `voiceui.ts`.** Replace the `AudioConfig` type, defaults and loader with:

```ts
export type NsLevel = 'off' | 'standard' | 'strong'
export type AudioConfig = {
  input: string | null
  output: string | null
  input_gain_pct: number
  sensitivity: number
  echo_cancel: boolean
  noise_suppression: NsLevel
  auto_sensitivity: boolean
  auto_gain: boolean
}
export const defaultAudioConfig: AudioConfig = {
  input: null, output: null, input_gain_pct: 100, sensitivity: 0.02, echo_cancel: true,
  noise_suppression: 'strong', auto_sensitivity: true, auto_gain: false,
}
/** Settings saved before levels existed carried `noise_suppress: boolean`. */
export function migrateAudioConfig(raw: Record<string, unknown>): AudioConfig {
  const { noise_suppress, ...rest } = raw
  const c = { ...defaultAudioConfig, ...rest } as AudioConfig
  if (raw.noise_suppression === undefined && noise_suppress === false) c.noise_suppression = 'off'
  return c
}
export const loadAudioConfig = () => migrateAudioConfig(load<Record<string, unknown>>('pulse.audio', {}))
```

(If Task 1 chose Standard as default, use `'standard'` in `defaultAudioConfig`.)

- [ ] **Step 4: `voice.svelte.ts`.** Extend the event union and state:

```ts
  | { kind: 'levels'; mic: number; speaker: number; gate_open: boolean }
  | { kind: 'noise_suppression'; active: NsLevel; note: string | null }
```

(import `type NsLevel` from `./voiceui`), add to `voice`: `ns: { active: null as NsLevel | null, note: null as string | null },` and `gateOpen: false,`; in the switch: `case 'levels': voice.levels = { mic: e.mic, speaker: e.speaker }; voice.gateOpen = e.gate_open; break` and `case 'noise_suppression': voice.ns = { active: e.active, note: e.note }; break`.

- [ ] **Step 5: `Settings.svelte`.**
  - Remove the `noise_suppress` entry from `toggles` (type becomes `'echo_cancel' | 'auto_gain'`).
  - Below the input-gain/sensitivity block (before "Let's check"), add:

```svelte
  <div class="field"><span class="lbl">NOISE SUPPRESSION</span>
    <div class="seg" role="radiogroup" aria-label="Noise suppression">
      {#each levels as l}
        <button role="radio" aria-checked={cfg.noise_suppression === l.value} class:on={cfg.noise_suppression === l.value}
          onclick={() => { cfg.noise_suppression = l.value; apply() }}>{l.label}</button>
      {/each}
    </div>
    <small>{levels.find((l) => l.value === cfg.noise_suppression)?.help}</small>
    {#if voice.ns.note}<p class="warn" role="status">{voice.ns.note}</p>{/if}
  </div>
```

with, in the script: `const levels: { value: NsLevel; label: string; help: string }[] = [ { value: 'off', label: 'Off', help: 'No noise suppression: your mic as it is.' }, { value: 'standard', label: 'Standard', help: 'Light: removes fans and hum, softens clicks. Easy on older PCs.' }, { value: 'strong', label: 'Strong', help: 'Removes keyboard clicks and most background noise. Uses more CPU.' } ]` (import `type NsLevel`).
  - In the sensitivity field: add an "Automatically determine sensitivity" switch row above the meter (same markup as `.toggles .row`, bound to `cfg.auto_sensitivity`, calling `apply()`); render the range `<input>` only `{#if !cfg.auto_sensitivity}`; in auto mode the meter fill uses `class:over={voice.gateOpen}` and the hint reads "Pulse sends your voice when it hears you speaking." (manual keeps today's text and `class:over={voice.levels.mic >= cfg.sensitivity}`).
  - Styles: `.seg { display: inline-flex; border: 1px solid var(--bg-4); border-radius: 10px; overflow: hidden; } .seg button { padding: 8px 14px; background: transparent; color: var(--text-2); border: 0; } .seg button.on { background: var(--accent); color: var(--bg-0); }` (match existing tokens; check they exist in the file's palette and reuse the switch's accent).

- [ ] **Step 6: Run** `cd ~/Projects/pulse-app && just check` → vitest (44 = 41 + 3), svelte-check 0 errors, Rust green.
- [ ] **Step 7: Commit** (`feat(ui): noise suppression levels, automatic sensitivity, status note` + attribution).

---

### Task 9: Whole-branch verification and hand-off

- [ ] **Step 1:** `just check` → green; `just voice-it` → 2 passed; `cargo test -p pulse-client --release --lib denoise` → 7 passed.
- [ ] **Step 2:** Windows: `XWIN_ACCEPT_LICENSE=1 cargo xwin check -p pulse-client --target x86_64-pc-windows-msvc` → Finished.
- [ ] **Step 3:** Release build: `cd client/src-tauri && cargo tauri build --no-bundle` → OK; record `ls -l target/release/pulse-app` size vs before (39–42 MB) in FINDINGS.
- [ ] **Step 4:** Memory check: run the release app, join voice on Strong, read `grep VmRSS /proc/$(pidof -s pulse-app)/status`; compare Off vs Strong; add to FINDINGS.
- [ ] **Step 5:** Fresh whole-branch review (one reviewer), fix findings, commit.
- [ ] **Step 6:** Ear test hand-off to Greg: mic test, type while talking, flip Off / Standard / Strong; auto vs manual sensitivity; busy office. Merge to `main` only after his OK; then rebuild the Windows kit.
