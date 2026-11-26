//! The mic's peak limiter (#586): keeps the gained-up voice under a −1 dBFS ceiling by turning the gain down
//! for a moment, instead of bending the waveform the way `mixer::soft_limit`'s tanh does.
//!
//! The tanh is memoryless: every peak past its knee is reshaped on the spot, which adds odd harmonics and
//! intermodulation (heard as crackle or fuzz), worst on deep voices whose low harmonics reach the knee first. A
//! limiter instead follows the signal's peak envelope and applies one smooth gain, so a sine stays a sine: quieter,
//! not squarer. Below the ceiling the gain is exactly 1, so quiet audio passes bit for bit.
//!
//! Per sample: a peak detector (instant rise, a short hold, then a ~100 ms release) → the gain that would put that
//! peak at the ceiling → that gain approached with a ~1 ms attack. No look-ahead, so the first millisecond of a
//! sudden loud onset can poke over the ceiling while the gain comes down: those few samples are clamped to it.
//! No allocation; all state is three numbers.

/// −1 dBFS: room for the i16 conversion and Opus's own overshoot.
pub const CEILING: f32 = 0.891_250_9;
const ATTACK_S: f32 = 0.001;
const RELEASE_S: f32 = 0.100;
/// The peak is held this long before it starts to release: longer than one period of a deep voice's fundamental
/// (~67 Hz), so the gain doesn't sag and snap back between every pitch pulse (that ripple would be distortion).
const HOLD_S: f32 = 0.015;
/// Below this the detector's state is zeroed rather than decaying into denormals.
const TINY: f32 = 1e-9;

pub struct Limiter {
    /// One-pole coefficients per sample: how much of the remaining distance is kept each sample.
    attack: f32,
    release: f32,
    hold: u32,
    /// The held, released peak of |input|.
    env: f32,
    held_for: u32,
    /// The gain applied right now (1.0 = untouched).
    gain: f32,
}

impl Limiter {
    pub fn new(rate: u32) -> Self {
        let per_sample = |secs: f32| (-1.0 / (secs * rate as f32)).exp();
        Self {
            attack: per_sample(ATTACK_S),
            release: per_sample(RELEASE_S),
            hold: (HOLD_S * rate as f32) as u32,
            env: 0.0,
            held_for: 0,
            gain: 1.0,
        }
    }

    /// Forget the past (after a mute, a device change): the next sample starts at unity gain.
    pub fn reset(&mut self) {
        self.env = 0.0;
        self.held_for = 0;
        self.gain = 1.0;
    }

    /// One sample through the limiter. Non-finite (and denormal) input comes out as silence; non-finite input
    /// leaves the state alone.
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        if !x.is_finite() {
            return 0.0;
        }
        let x = if x.abs() < f32::MIN_POSITIVE { 0.0 } else { x };
        let a = x.abs();
        if a >= self.env {
            self.env = a;
            self.held_for = 0;
        } else if self.held_for < self.hold {
            self.held_for += 1;
        } else {
            self.env = (self.env * self.release).max(a);
            if self.env < TINY {
                self.env = 0.0;
            }
        }
        let target = if self.env > CEILING {
            CEILING / self.env
        } else {
            1.0
        };
        if target < self.gain {
            self.gain = target + (self.gain - target) * self.attack;
        } else {
            // Rising: the envelope's release already makes this smooth (and lands exactly on 1.0).
            self.gain = target;
        }
        (x * self.gain).clamp(-CEILING, CEILING)
    }

    /// A block in place.
    #[cfg(test)]
    pub fn process_block(&mut self, block: &mut [f32]) {
        for v in block.iter_mut() {
            *v = self.process(*v);
        }
    }

    /// The gain being applied now (for tests and diagnostics).
    pub fn gain(&self) -> f32 {
        self.gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::mixer::soft_limit;
    use std::f32::consts::TAU;

    const RATE: u32 = 48_000;

    fn sine(freq: f32, amp: f32, secs: f32) -> Vec<f32> {
        let n = (secs * RATE as f32) as usize;
        (0..n)
            .map(|i| amp * (TAU * freq * i as f32 / RATE as f32).sin())
            .collect()
    }

    /// Total harmonic distortion (harmonics 2..=15 over the fundamental, as an amplitude ratio) of `x`, which must
    /// hold a whole number of `freq`'s periods.
    fn thd(x: &[f32], freq: f32) -> f32 {
        let bin = |h: f32| {
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (i, v) in x.iter().enumerate() {
                let ph = (TAU * h * freq * i as f32 / RATE as f32) as f64;
                re += *v as f64 * ph.cos();
                im += *v as f64 * ph.sin();
            }
            (re * re + im * im).sqrt()
        };
        let fundamental = bin(1.0);
        let harmonics: f64 = (2..=15).map(|h| bin(h as f32).powi(2)).sum::<f64>().sqrt();
        (harmonics / fundamental) as f32
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0, |m, v| m.max(v.abs()))
    }

    #[test]
    fn below_the_ceiling_it_is_bit_exact() {
        let mut l = Limiter::new(RATE);
        let mut x = sine(110.0, CEILING, 1.0);
        x.extend(sine(1000.0, 0.3, 0.5));
        let mut y = x.clone();
        l.process_block(&mut y);
        assert_eq!(x, y);
        assert_eq!(l.gain(), 1.0);
    }

    /// The case that crackled: a deep voice pushed 8 dB past the ceiling by the gain slider. The limiter keeps the
    /// peak under the ceiling with a clean sine; the tanh it replaces squares it off.
    #[test]
    fn a_sine_8_db_over_comes_out_under_the_ceiling_and_clean() {
        let amp = CEILING * 10f32.powf(8.0 / 20.0);
        for freq in [100.0, 200.0, 400.0] {
            let x = sine(freq, amp, 1.0);
            let mut l = Limiter::new(RATE);
            let mut y = x.clone();
            l.process_block(&mut y);
            assert!(peak(&y) <= CEILING, "{freq} Hz peak {}", peak(&y));
            // Steady state: the last 0.5 s (a whole number of periods at each frequency).
            let tail = &y[RATE as usize / 2..];
            assert!(
                peak(tail) > CEILING * 0.97,
                "{freq} Hz: limited to {}, not to the ceiling",
                peak(tail)
            );
            let tanh: Vec<f32> = x[RATE as usize / 2..]
                .iter()
                .map(|v| soft_limit(*v))
                .collect();
            let (ours, theirs) = (thd(tail, freq), thd(&tanh, freq));
            assert!(ours < 0.01, "{freq} Hz: limiter THD {:.2}%", ours * 100.0);
            assert!(
                ours * 10.0 < theirs,
                "{freq} Hz: limiter {:.2}% vs tanh {:.2}%",
                ours * 100.0,
                theirs * 100.0
            );
        }
    }

    #[test]
    fn the_gain_recovers_after_a_loud_passage() {
        let mut l = Limiter::new(RATE);
        let mut loud = sine(150.0, 2.0, 0.5);
        l.process_block(&mut loud);
        assert!(l.gain() < 0.5, "{}", l.gain());
        // 15 ms hold + 100 ms release: well back within half a second.
        let mut quiet = sine(150.0, 0.2, 0.5);
        l.process_block(&mut quiet);
        assert!(l.gain() > 0.99, "{}", l.gain());
        // …and a little later, fully transparent again.
        let x = sine(150.0, 0.2, 0.5);
        let mut y = x.clone();
        l.process_block(&mut y);
        assert_eq!(l.gain(), 1.0);
        let tail = RATE as usize / 4;
        assert_eq!(x[tail..], y[tail..]);
    }

    #[test]
    fn no_nan_or_denormals_into_and_after_silence() {
        let mut l = Limiter::new(RATE);
        let mut x = sine(200.0, 3.0, 0.2);
        x.extend(std::iter::repeat_n(0.0, RATE as usize * 3));
        x.extend([f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e-40, -1e-40]);
        x.extend(sine(200.0, 0.5, 0.1));
        let mut y = x.clone();
        l.process_block(&mut y);
        assert!(
            y.iter()
                .all(|v| v.is_finite() && (*v == 0.0 || v.is_normal()))
        );
        assert!(l.env == 0.0 || l.env.is_normal());
        assert!(l.gain.is_normal());
        assert!(peak(&y) <= CEILING);
    }

    #[test]
    fn reset_returns_to_unity() {
        let mut l = Limiter::new(RATE);
        let mut x = sine(200.0, 3.0, 0.1);
        l.process_block(&mut x);
        l.reset();
        assert_eq!(l.gain(), 1.0);
        assert_eq!(l.process(0.5), 0.5);
    }
}
