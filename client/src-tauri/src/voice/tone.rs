//! The mic's tone stage (#590): "Voice clarity EQ". Three gentle biquads after the denoiser, before the gain and
//! the limiter, at 48 kHz.
//!
//! - **High-pass, 65 Hz, 2nd order (Butterworth, Q 0.707).** Rumble, desk thumps and plosive pops carry a lot of
//!   energy and little speech. Left in, they set the peak the limiter reacts to, so the whole voice gets turned
//!   down for a "p". WebRTC's APM already runs its own 2nd-order high-pass (~100 Hz upstream; the corner isn't
//!   verified for this build) *before* the denoiser; this one sits after it, so whatever reaches the gain is
//!   guaranteed rumble-free whichever suppression runs, and together they roll off at ~24 dB/octave lower down.
//!   It was 80 Hz; stacked on the APM's it thinned deep voices, whose fundamental (~85–110 Hz) sat on the slope,
//!   so it is 65 Hz now (the APM's own corner is still to be measured). Above 150 Hz it costs < 0.2 dB.
//! - **Low-mid cut, −2 dB at 220 Hz (peaking, Q 1.0).** The "boom" of a deep voice, or a mic close up (proximity
//!   effect), piles up around 150–300 Hz. A bell there, not a low shelf, leaves the fundamental under it and the
//!   warmth above it mostly alone: the whole stage is −1.1 dB at 110 Hz, −2.1 dB at 220 Hz, −0.4 dB at 500 Hz. −2 dB is about the smallest broad cut that's
//!   audible: enough to take the muddiness off without making a light voice thin.
//! - **Presence lift, +1.5 dB at 3.5 kHz (peaking, Q 0.8).** Consonants live here; a little more makes speech easier
//!   to follow through Opus and over game audio. Kept at +1.5 dB (not more) because there is no de-esser: more lift
//!   would sharpen "s" sounds too.
//!
//! No de-esser: a safe one is a split-band compressor (sidechain detection, attack/release, threshold tuned per
//! voice); a fixed cut would dull every voice and a badly tuned dynamic one lisps. Left for a follow-up.
//!
//! Off, the block passes through untouched. No allocation: three biquads' state. Denormal-safe: the state is
//! zeroed once it has decayed below 1e-20 (checked once per block). Cost: 15 multiply-adds per sample.

use super::leveller::Biquad;

pub const HIGH_PASS_HZ: f64 = 65.0;
pub const LOW_MID_HZ: f64 = 220.0;
pub const LOW_MID_DB: f64 = -2.0;
pub const PRESENCE_HZ: f64 = 3_500.0;
pub const PRESENCE_DB: f64 = 1.5;

/// (b0, b1, b2, a1, a2) normalised by a0: what `Biquad::run` takes.
type Coeffs = [f64; 5];

fn normalise(b: [f64; 3], a: [f64; 3]) -> Coeffs {
    [
        b[0] / a[0],
        b[1] / a[0],
        b[2] / a[0],
        a[1] / a[0],
        a[2] / a[0],
    ]
}

/// RBJ Audio EQ Cookbook high-pass.
fn high_pass(rate: f64, f0: f64, q: f64) -> Coeffs {
    let w = std::f64::consts::TAU * f0 / rate;
    let (cos, alpha) = (w.cos(), w.sin() / (2.0 * q));
    normalise(
        [(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0],
        [1.0 + alpha, -2.0 * cos, 1.0 - alpha],
    )
}

/// RBJ Audio EQ Cookbook peaking EQ.
fn peaking(rate: f64, f0: f64, q: f64, db: f64) -> Coeffs {
    let w = std::f64::consts::TAU * f0 / rate;
    let (cos, alpha) = (w.cos(), w.sin() / (2.0 * q));
    let a = 10f64.powf(db / 40.0);
    normalise(
        [1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a],
        [1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a],
    )
}

pub struct ToneStage {
    on: bool,
    coeffs: [Coeffs; 3],
    state: [Biquad; 3],
}

impl ToneStage {
    pub fn new(rate: u32, on: bool) -> Self {
        let rate = rate as f64;
        Self {
            on,
            coeffs: [
                high_pass(rate, HIGH_PASS_HZ, std::f64::consts::FRAC_1_SQRT_2),
                peaking(rate, LOW_MID_HZ, 1.0, LOW_MID_DB),
                peaking(rate, PRESENCE_HZ, 0.8, PRESENCE_DB),
            ],
            state: [Biquad::default(); 3],
        }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    /// Turning it on starts from silence (no stale ring from before it was turned off).
    pub fn set_on(&mut self, on: bool) {
        if on && !self.on {
            self.state = [Biquad::default(); 3];
        }
        self.on = on;
    }

    /// Shape `block` in place (nothing at all when off).
    pub fn process(&mut self, block: &mut [f32]) {
        if !self.on {
            return;
        }
        let [hp, lm, pr] = &self.coeffs;
        let [s0, s1, s2] = &mut self.state;
        for v in block.iter_mut() {
            let x = *v as f64;
            *v = s2.run(pr, s1.run(lm, s0.run(hp, x))) as f32;
        }
        for s in &mut self.state {
            s.flush_tiny();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// The stage's gain (dB) on a steady sine at `hz`, measured after it has settled.
    fn gain_db(hz: f64) -> f64 {
        let mut t = ToneStage::new(RATE, true);
        let n = RATE as usize; // 1 s: the 65 Hz high-pass settles well within it
        let mut x: Vec<f32> = (0..2 * n)
            .map(|i| (0.25 * (std::f64::consts::TAU * hz * i as f64 / RATE as f64).sin()) as f32)
            .collect();
        let rms = |b: &[f32]| {
            (b.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / b.len() as f64).sqrt()
        };
        let before = rms(&x[n..]);
        for b in x.chunks_mut(480) {
            t.process(b);
        }
        20.0 * (rms(&x[n..]) / before).log10()
    }

    fn assert_near(hz: f64, want: f64, tol: f64) {
        let got = gain_db(hz);
        assert!(
            (got - want).abs() <= tol,
            "{hz} Hz: {got:.2} dB, wanted {want} ± {tol}"
        );
    }

    #[test]
    fn rumble_is_cut_hard() {
        assert!(gain_db(20.0) < -19.5, "{}", gain_db(20.0));
        assert!(gain_db(40.0) < -8.0, "{}", gain_db(40.0));
    }

    #[test]
    fn the_high_pass_corner_is_65_hz() {
        // −3 dB from the Butterworth, plus the low-mid bell's skirt
        assert_near(65.0, -3.2, 0.3);
    }

    #[test]
    fn a_deep_voice_fundamental_is_only_trimmed() {
        // −1.7 dB with the old 80 Hz corner; and a deeper one, at 90 Hz, keeps most of its level too
        assert_near(110.0, -1.1, 0.3);
        assert_near(90.0, -1.4, 0.3);
    }

    #[test]
    fn the_low_mid_cut_is_about_2_db() {
        assert_near(220.0, -2.0, 0.3);
    }

    #[test]
    fn the_midrange_is_left_alone() {
        assert_near(1_000.0, 0.0, 0.3);
    }

    #[test]
    fn presence_is_lifted_about_1_5_db() {
        assert_near(3_500.0, 1.5, 0.2);
    }

    #[test]
    fn the_top_is_close_to_flat() {
        assert_near(12_000.0, 0.0, 0.3);
    }

    #[test]
    fn off_passes_the_block_bit_for_bit() {
        let mut t = ToneStage::new(RATE, false);
        let orig: Vec<f32> = (0..480)
            .map(|i| ((i * 7919) % 2001) as f32 / 1000.0 - 1.0)
            .collect();
        let mut b = orig.clone();
        t.process(&mut b);
        assert_eq!(b, orig);
        // and back off after being on
        t.set_on(true);
        t.process(&mut b);
        assert_ne!(b, orig);
        t.set_on(false);
        let mut b = orig.clone();
        t.process(&mut b);
        assert_eq!(b, orig);
    }

    #[test]
    fn silence_after_sound_settles_to_exact_zero_not_denormals() {
        let mut t = ToneStage::new(RATE, true);
        let mut b = [0.0f32; 480];
        b[0] = 1.0;
        t.process(&mut b);
        let mut quiet = [0.0f32; 480];
        // ~2 s: the ring decays well past 1e-20
        for _ in 0..200 {
            quiet.fill(0.0);
            t.process(&mut quiet);
        }
        assert!(quiet.iter().all(|v| *v == 0.0));
        for s in &t.state {
            assert_eq!(
                s.state(),
                [0.0; 4],
                "decayed state is zeroed, never left as denormals"
            );
        }
    }

    #[test]
    fn turning_it_on_again_starts_from_silence() {
        let mut t = ToneStage::new(RATE, true);
        let mut loud = [0.9f32; 480];
        t.process(&mut loud);
        t.set_on(false);
        t.set_on(true);
        let mut zero = [0.0f32; 480];
        t.process(&mut zero);
        assert!(zero.iter().all(|v| *v == 0.0), "no ring left from before");
    }
}
