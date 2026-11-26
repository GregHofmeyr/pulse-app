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
    /// DeepFilterNet's bundled model (Strong's) held to a gentle cut, [`DF_LIGHT_ATTEN_LIM_DB`] (#601): for mics that
    /// already clean up their own sound (SteelSeries Sonar, NVIDIA Broadcast), where Strong's 24 dB carves the voice.
    /// Strong's CPU; steps down to Standard like Strong does.
    Light,
    Standard,
    #[default]
    Strong,
    /// DeepFilterNet3's low-delay model (#77): 10 ms instead of 30, for about 3x Strong's CPU and +120 MB. Not
    /// bundled: downloaded on first use (`crate::models`); Strong runs until it's there.
    StrongLowDelay,
}

impl NsLevel {
    /// Runs DeepFilterNet (Light, Strong or Strong (low delay)).
    pub fn is_deep_filter(self) -> bool {
        matches!(self, Self::Light | Self::Strong | Self::StrongLowDelay)
    }
}

/// Not `Send`: DeepFilterNet's model can't change threads, and chains live on one thread.
pub trait Denoiser {
    /// Clean `block` in place; speech probability, or `None` without a model (Off).
    fn process(&mut self, block: &mut [f32]) -> Option<f32>;

    /// The engine has stopped working (e.g. its worker thread died); the caller should replace it.
    fn failed(&self) -> bool {
        false
    }

    /// It failed by not answering in time (a worker starved for a moment: a game loading, the model paged out), not
    /// by crashing: worth trying again later, unlike a crash.
    fn stalled(&self) -> bool {
        false
    }

    /// How many 10 ms blocks `decision_level` runs ahead of the cleaned audio `process` returns.
    /// The gate decides on that level, so the engine's own delay becomes the gate's pre-roll.
    fn gate_lead_blocks(&self) -> usize {
        0
    }

    /// Level (RMS, unscaled) for the gate to decide on, from the newest input; `None` = the raw
    /// input's (the chain measures it).
    fn decision_level(&self) -> Option<f32> {
        None
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
    /// RMS of the last cleaned block (unscaled).
    level: f32,
}

impl Rnnoise {
    pub fn new() -> Self {
        Self {
            st: nnnoiseless::DenoiseState::new(),
            inb: vec![0.0; FRAME],
            outb: vec![0.0; FRAME],
            level: 0.0,
        }
    }

    /// Speech probability for `block` without changing it (Strong uses this as its VAD).
    fn probability(&mut self, block: &[f32]) -> f32 {
        self.inb
            .iter_mut()
            .zip(block)
            .for_each(|(d, s)| *d = s * I16_SCALE);
        let p = self.st.process_frame(&mut self.outb, &self.inb);
        // what RNNoise left of this input: 1 block behind it, 2 ahead of DeepFilterNet's output
        self.level =
            (self.outb.iter().map(|v| v * v).sum::<f32>() / FRAME as f32).sqrt() / I16_SCALE;
        p
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

    /// Its output lags its input by one block; the raw input decides.
    fn gate_lead_blocks(&self) -> usize {
        1
    }
}

/// Strong: DeepFilterNet3 for the audio; RNNoise alongside only for the speech probability
/// (DeepFilterNet reports a local SNR, not a probability).
pub struct DeepFilter {
    df: df::tract::DfTract,
    vad: Rnnoise,
    noisy: ndarray::Array2<f32>,
    enh: ndarray::Array2<f32>,
    dither: Dither,
}

/// How hard DeepFilterNet may cut, in dB (libDF's default, 100, means no limit). Mics that are already cleaned
/// (a virtual mic with its own suppression, a quiet room) have no noise left for it to remove, and unlimited it
/// starts carving the voice itself. 24 dB still takes a fan or keyboard well under the voice; the gate handles the
/// gaps between words (#586).
pub const DF_ATTEN_LIM_DB: f32 = 24.0;

/// Light's limit (#601): about a quarter of the amplitude cut, enough to take the hiss and hum a virtual mic leaves
/// without touching the voice it has already cleaned.
pub const DF_LIGHT_ATTEN_LIM_DB: f32 = 12.0;

/// DeepFilterNet's runtime settings (#586). The model's local-SNR estimate lives in [-15, 35] dB (`lsnr_min` /
/// `lsnr_max` in both models' config.ini), and libDF picks a stage per 10 ms frame from it with no hysteresis:
/// below `min_db_thresh` a zero mask, above `max_db_erb_thresh` the raw input, above `max_db_df_thresh` the ERB mask
/// only, otherwise ERB + deep filter. Clean speech sits right on the default 20/30 dB edges, so its frames flipped
/// between processed and raw timbre (heard as watery, patchy grain). At -15/35/35 no frame can cross those edges:
/// every frame gets the full ERB + deep-filter path (the deep filter is what rebuilds a low voice's harmonics), and
/// noise-only frames get the model's own mask, bounded by the attenuation limit, rather than a hard zero. Costs the
/// two decoders on every frame (they were skipped on clean frames before). The post-filter stays off: it deepens
/// attenuation in low-SNR bins, the opposite of what an already-cleaned mic needs.
fn runtime_params(atten_lim_db: f32) -> df::tract::RuntimeParams {
    df::tract::RuntimeParams::default_with_ch(1)
        .with_atten_lim(atten_lim_db)
        .with_thresholds(-15.0, 35.0, 35.0)
        .with_post_filter(0.0)
}

/// About -66 dBFS RMS of triangular (TPDF) noise, added to DeepFilterNet's input only (#586). libDF treats a frame
/// whose mean square is below 1e-7 (-70 dBFS) as silence, and after 5 of them returns zeros *before* its analysis,
/// so its STFT overlap and deep-filter history freeze on the end of the last phrase. A pre-gated mic (one with its
/// own noise gate) hands it exactly that between phrases, and the next phrase's first frames came out garbled. With
/// this floor every frame's mean square is about 2.5e-7, so libDF stays on its normal path; the model treats it as
/// the noise it is (and the attenuation limit puts what's left near -90 dBFS, under the gate that closes on it).
const DITHER_AMP: f32 = 1.228e-3; // each uniform in [-A/2, A/2]: TPDF RMS = A / sqrt(6) ≈ 5.0e-4 = -66 dBFS

/// A tiny xorshift PRNG: deterministic, allocation-free, cheap enough for every sample.
struct Dither(u32);

impl Dither {
    fn new() -> Self {
        Self(0x9E37_79B9)
    }

    /// Uniform in [-0.5, 0.5).
    #[inline]
    fn uniform(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 * (1.0 / 16_777_216.0) - 0.5
    }

    #[inline]
    fn next(&mut self) -> f32 {
        (self.uniform() + self.uniform()) * DITHER_AMP
    }
}

impl DeepFilter {
    /// Strong: loads the bundled model (a few hundred ms; call off the audio path).
    pub fn new() -> anyhow::Result<Self> {
        Self::load(DF_ATTEN_LIM_DB, || Ok(df::tract::DfParams::default()))
    }

    /// Light (#601): the bundled model, limited to [`DF_LIGHT_ATTEN_LIM_DB`].
    pub fn light() -> anyhow::Result<Self> {
        Self::load(DF_LIGHT_ATTEN_LIM_DB, || Ok(df::tract::DfParams::default()))
    }

    /// Strong (low delay): the downloaded model's bytes (a `.tar.gz` as libDF ships it; `crate::models` has already
    /// checked its hash). Slower to load than Strong; call off the audio path.
    pub fn low_delay(model: &[u8]) -> anyhow::Result<Self> {
        Self::load(DF_ATTEN_LIM_DB, || {
            df::tract::DfParams::from_bytes(model).context("reading the model")
        })
    }

    fn load(
        atten_lim_db: f32,
        params: impl FnOnce() -> anyhow::Result<df::tract::DfParams>,
    ) -> anyhow::Result<Self> {
        let df = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            df::tract::DfTract::new(params()?, &runtime_params(atten_lim_db))
        }))
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
            dither: Dither::new(),
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
            .for_each(|(d, s)| *d = *s + self.dither.next());
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

    /// DeepFilterNet lags its input by its lookahead plus 1 block (Strong: 3, low delay: 1); RNNoise's cleaned level
    /// only by 1. So that level leads by the model's lookahead (Strong: 2, low delay: 0).
    fn gate_lead_blocks(&self) -> usize {
        self.df.lookahead
    }

    fn decision_level(&self) -> Option<f32> {
        Some(self.vad.level)
    }
}

/// An engine that's instant to build: Light, Strong (and low delay) start on RNNoise until the model has loaded.
pub fn make_fast(level: NsLevel) -> Box<dyn Denoiser> {
    match level {
        NsLevel::Off => Box::new(Passthrough),
        NsLevel::Light | NsLevel::Standard | NsLevel::Strong | NsLevel::StrongLowDelay => {
            Box::new(Rnnoise::new())
        }
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
            ("strong_low_delay", Box::new(low_delay())),
        ]
    }

    /// Strong (low delay) on the model libDF's checkout ships (the file `just publish-models` uploads).
    fn low_delay() -> DeepFilter {
        DeepFilter::low_delay(&crate::models::checkout_low_delay_model()).expect("model loads")
    }

    #[test]
    fn low_delay_refuses_bytes_that_are_not_a_model() {
        assert!(DeepFilter::low_delay(b"not a model").is_err());
        assert!(DeepFilter::low_delay(&[]).is_err());
    }

    /// Old settings read exactly as before; the new level has its own name.
    #[test]
    fn levels_keep_their_saved_names() {
        for (level, name) in [
            (NsLevel::Off, "off"),
            (NsLevel::Light, "light"),
            (NsLevel::Standard, "standard"),
            (NsLevel::Strong, "strong"),
            (NsLevel::StrongLowDelay, "strong_low_delay"),
        ] {
            let json = format!("\"{name}\"");
            assert_eq!(serde_json::to_string(&level).unwrap(), json);
            assert_eq!(serde_json::from_str::<NsLevel>(&json).unwrap(), level);
        }
        assert_eq!(
            NsLevel::default(),
            NsLevel::Strong,
            "the default stays Strong"
        );
        assert!(NsLevel::StrongLowDelay.is_deep_filter() && NsLevel::Strong.is_deep_filter());
        assert!(NsLevel::Light.is_deep_filter());
        assert!(!NsLevel::Standard.is_deep_filter() && !NsLevel::Off.is_deep_filter());
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
        let (ll_out, _) = run(&mut low_delay(), &input);
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
        assert!(
            before - level_db(&ll_out) >= 15.0,
            "strong (low delay) only cut {:.1} dB",
            before - level_db(&ll_out)
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

    /// Output-vs-input offset (in whole blocks) where the cleaned speech matches the input best.
    fn lag_blocks(d: &mut dyn Denoiser) -> usize {
        let input = speech();
        let (out, _) = run(d, &input);
        let n = out.len() - 5 * FRAME;
        let score = |lag: usize| {
            let (a, b) = (&input[..n], &out[lag..lag + n]);
            let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
            let norm = |x: &[f32]| x.iter().map(|v| v * v).sum::<f32>().sqrt();
            dot / (norm(a) * norm(b)).max(1e-9)
        };
        (0..=4)
            .max_by(|&x, &y| score(x * FRAME).total_cmp(&score(y * FRAME)))
            .unwrap()
    }

    /// The gate's pre-roll now comes from these delays (`gate_lead_blocks`): they must be real.
    #[test]
    fn engine_delays_match_what_the_gate_assumes() {
        assert_eq!(lag_blocks(&mut Rnnoise::new()), 1, "Standard: 10 ms");
        assert_eq!(
            lag_blocks(&mut DeepFilter::new().unwrap()),
            3,
            "Strong: 30 ms"
        );
        assert_eq!(Rnnoise::new().gate_lead_blocks(), 1);
        assert_eq!(DeepFilter::new().unwrap().gate_lead_blocks(), 2);
        assert_eq!(Passthrough.gate_lead_blocks(), 0);
    }

    /// Low delay has no lookahead: it lags one block, like RNNoise's level it decides on, so the gate gets no lead
    /// from it (#77: the hard-coded 2 was Strong's).
    #[test]
    fn low_delay_delay_matches_what_the_gate_assumes() {
        assert_eq!(lag_blocks(&mut low_delay()), 1, "Strong (low delay): 10 ms");
        assert_eq!(low_delay().gate_lead_blocks(), 0);
    }

    /// Every frame of the dither alone sits above libDF's 1e-7 mean-square silence mark (with margin), so a
    /// pre-gated mic's digital silence never sends DeepFilterNet down its skip path (#586).
    #[test]
    fn dither_keeps_every_frame_above_libdfs_silence_mark() {
        let mut d = Dither::new();
        for _ in 0..2_000 {
            let ms = (0..FRAME).map(|_| d.next().powi(2)).sum::<f32>() / FRAME as f32;
            assert!((1.8e-7..3.5e-7).contains(&ms), "frame mean square {ms:e}");
        }
    }

    /// Digital silence in: on the skip path libDF would hand back exact zeros. With the dither it runs (its output
    /// isn't all zeros) yet what comes out stays far below anything audible, before the gate even closes on it.
    #[test]
    fn deep_filter_runs_through_silence_without_leaking_the_dither() {
        for (name, mut d) in [
            ("strong", DeepFilter::new().unwrap()),
            ("strong_low_delay", low_delay()),
        ] {
            let (out, _) = run(&mut d, &vec![0.0; 48_000 * 3]);
            assert!(out.iter().any(|v| *v != 0.0), "{name}: took the skip path");
            assert!(
                level_db(&out) < -80.0,
                "{name}: dither leaks at {:.1} dB",
                level_db(&out)
            );
        }
    }

    /// The attenuation limit holds: steady noise is cut, but not past DF_ATTEN_LIM_DB (unlimited, it went deeper).
    #[test]
    fn deep_filter_cut_is_bounded_by_the_attenuation_limit() {
        let input = noise(48_000 * 3, 0.05);
        for (name, mut d) in [
            ("strong", DeepFilter::new().unwrap()),
            ("strong_low_delay", low_delay()),
        ] {
            let (out, _) = run(&mut d, &input);
            let cut = level_db(&input) - level_db(&out);
            assert!(cut <= DF_ATTEN_LIM_DB + 1.0, "{name}: cut {cut:.1} dB");
        }
    }

    /// Light (#601) runs with its own, gentler limit: steady noise is still cut, but by no more than
    /// DF_LIGHT_ATTEN_LIM_DB, well short of what Strong takes from the same noise.
    #[test]
    fn light_cut_is_bounded_by_its_own_attenuation_limit() {
        let input = noise(48_000 * 3, 0.05);
        let (light, _) = run(&mut DeepFilter::light().unwrap(), &input);
        let (strong, _) = run(&mut DeepFilter::new().unwrap(), &input);
        let light_cut = level_db(&input) - level_db(&light);
        let strong_cut = level_db(&input) - level_db(&strong);
        assert!(
            (6.0..=DF_LIGHT_ATTEN_LIM_DB + 1.0).contains(&light_cut),
            "light cut {light_cut:.1} dB"
        );
        assert!(
            strong_cut >= light_cut + 6.0,
            "strong {strong_cut:.1} dB vs light {light_cut:.1} dB"
        );
    }

    /// Light keeps the voice and Strong's delay (same model, so the gate's pre-roll is the same).
    #[test]
    fn light_keeps_speech_and_strongs_delay() {
        let input = speech();
        let mut d = DeepFilter::light().unwrap();
        let (out, _) = run(&mut d, &input);
        let change = level_db(&out) - level_db(&input[..out.len()]);
        assert!(
            change.abs() <= 3.0,
            "light: speech level changed {change:.1} dB"
        );
        assert_eq!(d.gate_lead_blocks(), 2);
        assert_eq!(
            lag_blocks(&mut DeepFilter::light().unwrap()),
            3,
            "Light: 30 ms"
        );
    }

    #[test]
    fn make_fast_never_loads_the_big_model() {
        assert!(make_fast(NsLevel::Off).process(&mut [0.0; FRAME]).is_none());
        for level in [NsLevel::Light, NsLevel::Strong, NsLevel::StrongLowDelay] {
            assert!(make_fast(level).process(&mut [0.0; FRAME]).is_some());
        }
    }
}
