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

/// Not `Send`: DeepFilterNet's model can't change threads, and chains live on one thread.
pub trait Denoiser {
    /// Clean `block` in place; speech probability, or `None` without a model (Off).
    fn process(&mut self, block: &mut [f32]) -> Option<f32>;

    /// The engine has stopped working (e.g. its worker thread died); the caller should replace it.
    fn failed(&self) -> bool {
        false
    }
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
        self.inb
            .iter_mut()
            .zip(block)
            .for_each(|(d, s)| *d = s * I16_SCALE);
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
        block
            .iter_mut()
            .zip(&self.outb)
            .for_each(|(o, v)| *o = v / I16_SCALE);
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
        anyhow::ensure!(
            df.hop_size == FRAME,
            "unexpected DeepFilterNet hop size {}",
            df.hop_size
        );
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
        self.noisy
            .row_mut(0)
            .iter_mut()
            .zip(block.iter())
            .for_each(|(d, s)| *d = *s);
        // On a model error, send the block uncleaned rather than silence.
        if self
            .df
            .process(self.noisy.view(), self.enh.view_mut())
            .is_ok()
        {
            block
                .iter_mut()
                .zip(self.enh.row(0).iter())
                .for_each(|(o, v)| *o = *v);
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
                floor[i]
                    + if k < 96 {
                        0.5 * (1.0 - k as f32 / 96.0) * if k % 2 == 0 { 1.0 } else { -1.0 }
                    } else {
                        0.0
                    }
            })
            .collect()
    }

    fn run(d: &mut dyn Denoiser, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut out = Vec::new();
        let mut probs = Vec::new();
        for block in input.as_chunks::<FRAME>().0 {
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
        20.0 * ((x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32)
            .sqrt()
            .max(1e-9))
        .log10()
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
        assert!(
            level_db(&std_out) < before,
            "standard: {} -> {}",
            before,
            level_db(&std_out)
        );
        assert!(
            before - level_db(&strong_out) >= 15.0,
            "strong only cut {:.1} dB",
            before - level_db(&strong_out)
        );
    }

    #[test]
    fn speech_is_preserved_within_3_db() {
        let input = speech();
        for (name, mut d) in engines() {
            let (out, _) = run(d.as_mut(), &input);
            let change = level_db(&out) - level_db(&input[..out.len()]);
            assert!(
                change.abs() <= 3.0,
                "{name}: speech level changed {change:.1} dB"
            );
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
            assert!(
                mean(&sp) > 0.5 && mean(&np) < 0.5,
                "{name}: speech {:.2} noise {:.2}",
                mean(&sp),
                mean(&np)
            );
        }
    }

    #[test]
    fn make_fast_never_loads_the_big_model() {
        assert!(make_fast(NsLevel::Off).process(&mut [0.0; FRAME]).is_none());
        assert!(
            make_fast(NsLevel::Strong)
                .process(&mut [0.0; FRAME])
                .is_some()
        );
    }
}
