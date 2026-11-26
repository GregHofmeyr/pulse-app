//! The mic's tone stage (#590): "Voice clarity EQ". Three gentle biquads after the denoiser, before the gain and
//! the limiter, at 48 kHz.
//!
//! - **High-pass, 65 Hz, 2nd order (Butterworth, Q 0.707).** Rumble, desk thumps and plosive pops carry a lot of
//!   energy and little speech. Left in, they set the peak the limiter reacts to, so the whole voice gets turned
//!   down for a "p". WebRTC's APM runs its own high-pass *before* the denoiser; this one sits after it, so whatever
//!   reaches the gain is rumble-free whichever suppression runs. It was 80 Hz; it is 65 Hz now (#589's PR) so a
//!   deep voice's fundamental (~85–110 Hz) isn't on its slope. Above 150 Hz it costs < 0.2 dB.
//!
//!   **The APM's high-pass, measured (#660)** on the libwebrtc we link, 48 kHz, 10 ms frames (`measured_low_cut`
//!   below): −3 dB at **58 Hz**, and steep: −21 dB at 50 Hz, −44 dB at 40 Hz, ~−80 dB at 30 Hz. From 60 Hz up it is
//!   flat to within the ±0.5 dB ripple of the APM's 3-band split (−0.2 dB at 90, −0.5 at 110, −0.2 at 150). Not the
//!   ~100 Hz 2nd-order slope once assumed: it sits below a deep voice, so the two stack to only −1.6 dB at 90–150 Hz
//!   (this stage −1.1…−1.4, the APM ≤ −0.5). Both stay on: the APM's takes out what's below 50 Hz before the echo
//!   canceller and the denoiser, this one adds the gentle 50–80 Hz slope for plosives. Echo cancellation keeps the
//!   APM's high-pass running whatever flag it's built with (AEC3 enforces it), and with the clarity EQ off the
//!   APM's alone still cuts 30 Hz rumble by ~80 dB.
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

/// #660: the low cut the mic actually gets, measured: WebRTC's APM high-pass on its own (the build we link),
/// then APM → `MicChain` (denoiser off, so the tone stage is the only other filter) with the clarity EQ on and off.
#[cfg(test)]
mod measured_low_cut {
    use super::super::denoise::{FRAME, NsLevel};
    use super::super::processor::MicChain;
    use livekit::webrtc::native::apm::AudioProcessingModule;
    use std::time::Instant;

    const RATE: usize = 48_000;
    /// 10 ms, what the APM takes and what the processor feeds it.
    const APM_FRAME: usize = RATE / 100;

    fn sine(hz: f64, secs: usize) -> Vec<f32> {
        (0..secs * RATE)
            .map(|i| (0.25 * (std::f64::consts::TAU * hz * i as f64 / RATE as f64).sin()) as f32)
            .collect()
    }

    fn rms(b: &[f32]) -> f64 {
        (b.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / b.len() as f64).sqrt()
    }

    /// Through the APM the way the processor runs it: i16, 10 ms at a time, the reverse (speaker) stream fed
    /// silence first when echo cancellation is on.
    fn through_apm(x: &mut [f32], echo: bool, high_pass: bool) {
        let mut apm = AudioProcessingModule::new(echo, false, high_pass, false);
        let mut far = [0i16; APM_FRAME];
        let mut buf = [0i16; APM_FRAME];
        for block in x.chunks_mut(APM_FRAME) {
            for (d, v) in buf.iter_mut().zip(block.iter()) {
                *d = (*v * 32767.0).round() as i16;
            }
            if echo {
                far.fill(0);
                apm.process_reverse_stream(&mut far, RATE as i32, 1)
                    .unwrap();
            }
            apm.process_stream(&mut buf, RATE as i32, 1).unwrap();
            for (d, v) in block.iter_mut().zip(buf.iter()) {
                *d = *v as f32 / 32767.0;
            }
        }
    }

    /// Gain (dB) on a steady sine, the first second (the filters settling) left out.
    fn gain_db(hz: f64, shape: impl FnOnce(&mut [f32])) -> f64 {
        let x = sine(hz, 3);
        let mut y = x.clone();
        shape(&mut y);
        20.0 * (rms(&y[RATE..]) / rms(&x[RATE..])).log10()
    }

    fn apm_alone(hz: f64) -> f64 {
        gain_db(hz, |y| through_apm(y, false, true))
    }

    /// APM (high-pass on, echo off) → denoiser off → tone stage (clarity EQ `eq`) → gain 1 → limiter → gate.
    fn whole_chain(hz: f64, eq: bool) -> f64 {
        gain_db(hz, |y| {
            through_apm(y, false, true);
            let mut chain = MicChain::new(NsLevel::Off, false, 0.0, 1.0);
            chain.set_tone(eq);
            let t0 = Instant::now();
            for block in y.chunks_mut(FRAME) {
                chain.process(block, true, t0);
            }
        })
    }

    #[test]
    #[ignore = "prints the table for #660; run with --ignored --nocapture"]
    fn print_the_response() {
        println!("   Hz | APM alone | tone EQ | chain, EQ on | chain, EQ off");
        for hz in [
            30.0, 40.0, 50.0, 55.0, 58.0, 60.0, 65.0, 80.0, 90.0, 100.0, 110.0, 120.0, 150.0,
            200.0, 1_000.0,
        ] {
            println!(
                "{hz:>5} | {:>9.2} | {:>7.2} | {:>12.2} | {:>13.2}",
                apm_alone(hz),
                gain_db(hz, |y| super::ToneStage::new(RATE as u32, true).process(y)),
                whole_chain(hz, true),
                whole_chain(hz, false),
            );
        }
    }

    /// Measured on libwebrtc as linked (webrtc-sys 0.3.52): −3 dB at 58 Hz, and steep, not the ~100 Hz
    /// 2nd-order slope we'd assumed. It is −21 dB at 50 Hz and −44 dB at 40 Hz, yet flat to within the ±0.5 dB ripple
    /// of the APM's 3-band split from 60 Hz up, so a deep voice's 85–110 Hz fundamental isn't touched by it.
    #[test]
    fn the_apm_high_pass_corner_is_58_hz_and_steep() {
        let near = |hz: f64, want: f64, tol: f64| {
            let got = apm_alone(hz);
            assert!(
                (got - want).abs() <= tol,
                "{hz} Hz: {got:.2} dB, wanted {want} ± {tol}"
            );
        };
        assert!(apm_alone(30.0) < -60.0, "{}", apm_alone(30.0));
        assert!(apm_alone(40.0) < -40.0, "{}", apm_alone(40.0));
        near(50.0, -21.2, 1.5);
        near(58.0, -3.0, 0.7);
        near(60.0, -0.5, 0.5);
        for hz in [80.0, 100.0, 120.0, 150.0, 200.0] {
            near(hz, 0.0, 0.7);
        }
    }

    /// The echo canceller keeps the high-pass whatever we pass for it (AEC3 enforces it), so the flag only
    /// matters with echo cancellation off: turning it off could never change what the AEC sees.
    #[test]
    fn echo_cancellation_runs_the_high_pass_even_with_the_flag_off() {
        let echo_only = gain_db(50.0, |y| through_apm(y, true, false));
        let echo_hpf = gain_db(50.0, |y| through_apm(y, true, true));
        assert!(echo_only < -15.0, "{echo_only}");
        assert!(
            (echo_only - echo_hpf).abs() < 0.1,
            "{echo_only} vs {echo_hpf}"
        );
        // off and no echo cancellation: the APM passes the audio through untouched
        let bypass = gain_db(50.0, |y| through_apm(y, false, false));
        assert!(bypass.abs() < 0.01, "{bypass}");
    }

    /// A deep voice's fundamental through everything that filters it: the APM costs it ≤ 0.5 dB on top of the
    /// tone stage, so the two together stay well under the 3 dB the issue worried about.
    #[test]
    fn the_whole_chain_keeps_a_deep_voice_fundamental() {
        let near = |hz: f64, want: f64, tol: f64| {
            let got = whole_chain(hz, true);
            assert!(
                (got - want).abs() <= tol,
                "{hz} Hz: {got:.2} dB, wanted {want} ± {tol}"
            );
        };
        near(90.0, -1.6, 0.4);
        near(110.0, -1.6, 0.4);
        near(150.0, -1.6, 0.4);
    }

    /// The clarity EQ off: the APM's high-pass alone still takes rumble out.
    #[test]
    fn with_the_eq_off_rumble_is_still_cut() {
        let got = whole_chain(30.0, false);
        assert!(got <= -12.0, "30 Hz: {got:.2} dB");
    }
}
