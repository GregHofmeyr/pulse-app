//! Receive-side loudness levelling (#355 (3)): brings each remote voice towards one loudness, so a
//! quiet mic and a loud one land closer together without anyone touching a slider.
//!
//! One [`Leveller`] per received voice track, owned by that track's receive task (network side,
//! before the samples reach the mixer's ring), so it needs no lock and never runs on the audio
//! callback; it still allocates nothing per frame. The person's own volume (and the volume mixer's)
//! applies after it, in the mixer. Stream audio and the soundboard never pass through it.
//!
//! How it measures: EBU R128 / BS.1770 K-weighting (a high shelf and a high-pass, two biquads per
//! sample: cheap at 48 kHz, so no need for a cruder plain RMS) summed over 10 ms blocks. Only blocks
//! that look like speech count: above an absolute gate (−50 LUFS) and well above the track's own
//! noise floor (which follows the quietest blocks down at once and creeps up slowly). The speech
//! blocks feed a ~3 s loudness average (EBU's short-term window), and the gain moves towards
//! `TARGET − loudness` (clamped to ±12 dB) with seconds-long time constants, only while speech is
//! heard: in silence or steady noise it holds, so it never winds up on a quiet line and never
//! boosts noise. Each block's peak is looked at before its gain is applied (#589), so a boost is
//! pulled back in the very block someone starts shouting, not the one after; a peak guard then keeps
//! it back through the gaps between words, and an envelope limiter ([`Limiter`], as on the mic
//! side) catches anything left before the samples go back to i16. The mixer's own limiter still
//! guards the sum of everyone.

#[cfg(test)]
use super::limiter::CEILING;
use super::limiter::Limiter;
#[cfg(test)]
use super::mixer::soft_limit;

/// Where voices are steered to: about a well set-up mic, so most people barely move and only the
/// outliers do.
pub const TARGET_LUFS: f32 = -20.0;
/// Most a quiet voice is raised.
pub const MAX_BOOST_DB: f32 = 12.0;
/// Most a loud voice is lowered.
pub const MAX_CUT_DB: f32 = 12.0;

/// 10 ms at 48 kHz: the measuring block (and how often the gain is re-aimed).
const BLOCK: usize = 480;
const BLOCKS_PER_S: f32 = 100.0;
/// Below this a block is silence, whatever its noise floor.
const ABS_GATE_LUFS: f32 = -50.0;
/// A block this far over the noise floor counts as speech.
const OVER_FLOOR_DB: f32 = 10.0;
/// How fast the noise floor creeps up (it drops to a quieter block at once): pauses between words
/// pull it back down, a steady noise is reached within seconds.
const FLOOR_RISE_DB_PER_S: f32 = 4.0;
/// The loudness average's window (EBU R128 short-term).
const LOUDNESS_S: f32 = 3.0;
/// Speech heard before the gain moves at all (one EBU momentary window).
const MIN_SPEECH_S: f32 = 0.4;
/// Time constants of the gain, counted in speech: lowering a too-loud voice is quicker than
/// raising a quiet one (a sudden blast is worse than a quiet moment).
const CUT_TAU_S: f32 = 1.5;
const BOOST_TAU_S: f32 = 3.0;
/// A boost is pulled back so the block's peak stays under this (−1 dBFS) ...
const PEAK_CEILING: f32 = 0.89;
/// ... and allowed back up at this rate once the peak has passed.
const GUARD_RELEASE_DB_PER_S: f32 = 30.0;
/// When a block arrives too hot for the gain in use, the gain comes down over at least this many
/// samples (1 ms), never in one step (a click); the limiter covers any sample before it lands.
const FAST_RAMP: usize = 48;

/// BS.1770 K-weighting at 48 kHz: stage 1 high shelf, stage 2 high-pass (RLB). (b0, b1, b2, a1, a2)
const SHELF: [f64; 5] = [
    1.535_124_859_586_97,
    -2.691_696_189_406_38,
    1.198_392_810_852_85,
    -1.690_659_293_182_41,
    0.732_480_774_215_85,
];
const HIGH_PASS: [f64; 5] = [1.0, -2.0, 1.0, -1.990_047_454_833_98, 0.990_072_250_366_21];

/// One biquad, direct form I (f64 state: the high-pass's poles sit very close to 1). Also the mic's tone stage
/// (`tone`, #590).
#[derive(Clone, Copy, Default)]
pub(super) struct Biquad {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    /// One sample through coefficients `c` = (b0, b1, b2, a1, a2), normalised so a0 = 1.
    pub(super) fn run(&mut self, c: &[f64; 5], x: f64) -> f64 {
        let y = c[0] * x + c[1] * self.x1 + c[2] * self.x2 - c[3] * self.y1 - c[4] * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    /// Zero state that has decayed to nothing, so a filter left ringing into silence never reaches denormals (slow on
    /// x86). Far below anything audible (an i16 step is 3e-5).
    /// The state (x1, x2, y1, y2), for tests.
    #[cfg(test)]
    pub(super) fn state(&self) -> [f64; 4] {
        [self.x1, self.x2, self.y1, self.y2]
    }

    pub(super) fn flush_tiny(&mut self) {
        const TINY: f64 = 1e-20;
        for v in [&mut self.x1, &mut self.x2, &mut self.y1, &mut self.y2] {
            if v.abs() < TINY {
                *v = 0.0;
            }
        }
    }
}

fn lufs(mean_square: f64) -> f32 {
    (-0.691 + 10.0 * (mean_square + 1e-12).log10()) as f32
}

fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// One voice track's levelling state. Fixed size: nothing is allocated after `new`.
pub struct Leveller {
    shelf: Biquad,
    high_pass: Biquad,
    /// K-weighted energy and peak of the block so far, and its length.
    energy: f64,
    peak: f32,
    filled: usize,
    /// The track's noise floor (LUFS); `None` until the first block.
    floor: Option<f32>,
    /// K-weighted mean square of recent speech, and how many speech blocks have been heard.
    speech_power: f64,
    speech_blocks: u32,
    /// The levelling gain aimed at (dB), moved slowly.
    gain_db: f32,
    /// The peak guard's ceiling on a boost (dB).
    guard_db: f32,
    /// The gain the levelling and the guard want for this block (linear), set at each block's end.
    aim: f32,
    /// The gain applied to the current sample (linear), ramping by `step` per sample for `ramp`
    /// more samples towards the block's target, so a gain change never clicks.
    gain: f32,
    step: f32,
    ramp: usize,
    target: f32,
    /// The safety net after the gain: holds the output under −1 dBFS without bending the waveform.
    limiter: Limiter,
}

impl Default for Leveller {
    fn default() -> Self {
        Self::new()
    }
}

impl Leveller {
    pub fn new() -> Self {
        Self {
            shelf: Biquad::default(),
            high_pass: Biquad::default(),
            energy: 0.0,
            peak: 0.0,
            filled: 0,
            floor: None,
            speech_power: 0.0,
            speech_blocks: 0,
            gain_db: 0.0,
            guard_db: MAX_BOOST_DB,
            aim: 1.0,
            gain: 1.0,
            step: 0.0,
            ramp: 0,
            target: 1.0,
            limiter: Limiter::new(BLOCKS_PER_S as u32 * BLOCK as u32),
        }
    }

    /// The levelling gain now (dB), before the peak guard.
    pub fn gain_db(&self) -> f32 {
        self.gain_db
    }

    /// Level 48 kHz mono `samples` in place. Off: they're left exactly as they are, and the gain
    /// goes back to 0 dB, so switching it on again eases in from where the person really is
    /// (the loudness it has measured so far is kept).
    pub fn process(&mut self, samples: &mut [i16], on: bool) {
        if !on {
            self.gain_db = 0.0;
            self.aim = 1.0;
            self.gain = 1.0;
            self.target = 1.0;
            self.step = 0.0;
            self.ramp = 0;
            self.limiter.reset();
            return;
        }
        // one stretch per measuring block (a whole block when frames are 10 ms, as they are)
        let mut rest = samples;
        while !rest.is_empty() {
            let n = (BLOCK - self.filled).min(rest.len());
            let (stretch, tail) = rest.split_at_mut(n);
            self.process_stretch(stretch);
            rest = tail;
        }
    }

    /// Up to the end of one measuring block. Its peak is found first (the samples are all here: a
    /// look-ahead for free), and the gain is capped so `peak × gain` stays under the ceiling.
    fn process_stretch(&mut self, stretch: &mut [i16]) {
        let level = |s: &i16| (*s as f32 / 32768.0).abs();
        let peak = stretch.iter().map(level).fold(0.0f32, f32::max);
        // it only ever takes a boost back: unboosted audio is as it arrived
        let cap = (PEAK_CEILING / peak.max(1e-6)).max(1.0);
        self.target = self.aim.min(cap);
        self.ramp = if self.target < self.gain && peak * self.gain > PEAK_CEILING {
            // too hot for the gain in use: down before the first sample that would overshoot
            let hot = stretch
                .iter()
                .position(|s| level(s) * self.gain > PEAK_CEILING)
                .unwrap_or(stretch.len());
            hot.max(FAST_RAMP).min(stretch.len())
        } else {
            stretch.len()
        };
        self.step = (self.target - self.gain) / self.ramp as f32;
        for s in stretch {
            let x = *s as f32 / 32768.0;
            let k = self
                .high_pass
                .run(&HIGH_PASS, self.shelf.run(&SHELF, x as f64));
            self.energy += k * k;
            self.peak = self.peak.max(x.abs());
            self.filled += 1;
            if self.ramp > 1 {
                self.gain += self.step;
                self.ramp -= 1;
            } else {
                self.gain = self.target;
                self.ramp = 0;
            }
            let y = self.limiter.process(x * self.gain);
            *s = (y * 32767.0).round().clamp(-32768.0, 32767.0) as i16;
            if self.filled == BLOCK {
                self.end_block();
            }
        }
    }

    /// A 10 ms block is in: judge it, move the gain if it was speech, and aim the next block.
    fn end_block(&mut self) {
        let mean_square = self.energy / BLOCK as f64;
        let peak = self.peak;
        (self.energy, self.peak, self.filled) = (0.0, 0.0, 0);
        let level = lufs(mean_square);
        let floor = match self.floor {
            Some(f) if level >= f => f + FLOOR_RISE_DB_PER_S / BLOCKS_PER_S,
            _ => level,
        };
        self.floor = Some(floor);
        let speech = level > ABS_GATE_LUFS && level > floor + OVER_FLOOR_DB;
        if speech {
            self.speech_blocks = self.speech_blocks.saturating_add(1);
            // a running mean until the window is full, then a moving one
            let alpha =
                (1.0 / self.speech_blocks as f64).max(1.0 / (LOUDNESS_S * BLOCKS_PER_S) as f64);
            self.speech_power += alpha * (mean_square - self.speech_power);
            if self.speech_blocks as f32 >= MIN_SPEECH_S * BLOCKS_PER_S {
                let want = (TARGET_LUFS - lufs(self.speech_power)).clamp(-MAX_CUT_DB, MAX_BOOST_DB);
                let tau = if want < self.gain_db {
                    CUT_TAU_S
                } else {
                    BOOST_TAU_S
                };
                self.gain_db += (want - self.gain_db) / (tau * BLOCKS_PER_S);
            }
        }
        // The peak guard: keeps a boost pulled back after a loud block (each block's own peak is
        // already capped as it plays), and only relaxes while someone is audible: a gap between
        // words keeps it, or every syllable after a shout would be raised and pulled back again.
        if level > ABS_GATE_LUFS {
            let room = 20.0 * (PEAK_CEILING / peak.max(1e-6)).log10();
            self.guard_db = room.min(self.guard_db + GUARD_RELEASE_DB_PER_S / BLOCKS_PER_S);
        }
        // it only ever takes a boost back: unboosted audio is as it arrived
        self.aim = db_to_gain(self.gain_db.min(self.guard_db.max(0.0)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: usize = 48_000;

    /// Speech-like test signal: three tones (300 Hz, 1 kHz, 2.2 kHz) in 250 ms syllables with
    /// 100 ms gaps of digital silence, `amp` the peak of the sum (0..1).
    fn speech(amp: f32, secs: f32) -> Vec<i16> {
        let n = (secs * RATE as f32) as usize;
        let (on, period) = (RATE / 4, RATE * 35 / 100);
        (0..n)
            .map(|i| {
                let p = i % period;
                if p >= on {
                    return 0;
                }
                // 5 ms ramps at each end of a syllable
                let edge = (p.min(on - p) as f32 / (RATE as f32 * 0.005)).min(1.0);
                let t = i as f32 / RATE as f32;
                let tau = std::f32::consts::TAU;
                let v =
                    ((tau * 300.0 * t).sin() + (tau * 1000.0 * t).sin() + (tau * 2200.0 * t).sin())
                        / 3.0;
                (v * amp * edge * 32767.0) as i16
            })
            .collect()
    }

    /// Run `x` through `lev` in 10 ms frames, as the receive task hands them over.
    fn run(lev: &mut Leveller, x: &[i16], on: bool) -> Vec<i16> {
        let mut out = x.to_vec();
        for f in out.chunks_mut(480) {
            lev.process(f, on);
        }
        out
    }

    /// What the levelling wants for `x`: TARGET minus its gated speech loudness, clamped.
    fn speech_lufs(x: &[i16]) -> f32 {
        let (mut a, mut b) = (Biquad::default(), Biquad::default());
        let (mut e, mut n) = (0.0f64, 0usize);
        for s in x.iter().filter(|s| **s != 0) {
            let k = b.run(&HIGH_PASS, a.run(&SHELF, *s as f64 / 32768.0));
            e += k * k;
            n += 1;
        }
        lufs(e / n as f64)
    }

    fn rms_db(x: &[i16]) -> f32 {
        let e: f64 = x.iter().map(|s| (*s as f64 / 32768.0).powi(2)).sum::<f64>() / x.len() as f64;
        10.0 * (e + 1e-12).log10() as f32
    }

    #[test]
    fn k_weighting_is_flat_at_1_khz_on_the_bs1770_reference() {
        // BS.1770: a 0 dBFS 1 kHz sine on one channel reads −3.01 LUFS
        let x: Vec<i16> = (0..RATE)
            .map(|i| {
                ((std::f32::consts::TAU * 1000.0 * i as f32 / RATE as f32).sin() * 32767.0) as i16
            })
            .collect();
        let (mut a, mut b) = (Biquad::default(), Biquad::default());
        let e: f64 = x
            .iter()
            .map(|s| {
                b.run(&HIGH_PASS, a.run(&SHELF, *s as f64 / 32768.0))
                    .powi(2)
            })
            .skip(RATE / 10)
            .sum::<f64>()
            / (RATE - RATE / 10) as f64;
        assert!((lufs(e) - -3.01).abs() < 0.2, "{}", lufs(e));
    }

    #[test]
    fn a_quiet_speaker_is_raised_towards_the_target_within_ten_seconds() {
        let x = speech(0.04, 12.0);
        let want = (TARGET_LUFS - speech_lufs(&x)).clamp(-MAX_CUT_DB, MAX_BOOST_DB);
        assert!(
            want > 6.0,
            "the test speaker should be well under the target: {want}"
        );
        let mut lev = Leveller::new();
        let early = run(&mut lev, &x[..RATE], true);
        let after_1s = lev.gain_db();
        assert!(
            after_1s < want / 2.0,
            "slow, not a jump: {after_1s} of {want} dB after 1 s"
        );
        let _ = early;
        let out = run(&mut lev, &x[RATE..10 * RATE], true);
        assert!(
            (lev.gain_db() - want).abs() < 1.5,
            "{} dB after 10 s, want {want}",
            lev.gain_db()
        );
        // and the audio really is louder by about that
        let tail = 8 * RATE..9 * RATE;
        let louder = rms_db(&out[tail.start - RATE..tail.end - RATE]) - rms_db(&x[tail]);
        assert!(
            (louder - want).abs() < 2.0,
            "{louder} dB louder, want {want}"
        );
    }

    #[test]
    fn a_loud_speaker_is_lowered_towards_the_target() {
        let x = speech(0.9, 10.0);
        let want = (TARGET_LUFS - speech_lufs(&x)).clamp(-MAX_CUT_DB, MAX_BOOST_DB);
        assert!(
            want < -6.0,
            "the test speaker should be well over the target: {want}"
        );
        let mut lev = Leveller::new();
        run(&mut lev, &x, true);
        assert!(
            (lev.gain_db() - want).abs() < 1.0,
            "{} dB, want {want}",
            lev.gain_db()
        );
    }

    #[test]
    fn a_whisper_is_raised_at_most_twelve_db() {
        let x = speech(0.015, 30.0);
        let mut lev = Leveller::new();
        run(&mut lev, &x, true);
        assert!(lev.gain_db() <= MAX_BOOST_DB + 1e-3);
        assert!(lev.gain_db() > MAX_BOOST_DB - 1.0, "{}", lev.gain_db());
    }

    #[test]
    fn silence_is_never_raised() {
        let mut lev = Leveller::new();
        let out = run(&mut lev, &vec![0i16; 10 * RATE], true);
        assert!(out.iter().all(|s| *s == 0));
        assert_eq!(lev.gain_db(), 0.0);
    }

    #[test]
    fn steady_noise_is_not_raised() {
        // white noise around −40 dBFS (above the absolute gate), from a fixed seed
        let mut seed = 0x1234_5678u32;
        let x: Vec<i16> = (0..10 * RATE)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((seed >> 16) as i16 as f32 * 0.02) as i16
            })
            .collect();
        let mut lev = Leveller::new();
        let out = run(&mut lev, &x, true);
        assert!(lev.gain_db() <= 0.0, "{}", lev.gain_db());
        assert!(rms_db(&out[9 * RATE..]) <= rms_db(&x[9 * RATE..]) + 0.1);
    }

    #[test]
    fn the_gain_holds_through_a_long_silence() {
        let mut lev = Leveller::new();
        run(&mut lev, &speech(0.04, 10.0), true);
        let raised = lev.gain_db();
        run(&mut lev, &vec![0i16; 20 * RATE], true);
        assert_eq!(lev.gain_db(), raised, "no drift while nobody talks");
    }

    #[test]
    fn a_raised_speaker_who_shouts_never_clips_or_wraps() {
        let mut lev = Leveller::new();
        run(&mut lev, &speech(0.01, 20.0), true);
        assert!(lev.gain_db() > 10.0, "raised first: {}", lev.gain_db());
        let shout = speech(0.7, 2.0); // +12 dB on this would be far past full scale
        let out = run(&mut lev, &shout, true);
        for (i, (x, y)) in shout.iter().zip(&out).enumerate() {
            // an i16 overflow would flip the sign
            assert!(
                *x == 0 || y.signum() == x.signum() || y.abs() < 64,
                "sample {i}: {x} -> {y}"
            );
        }
        // the guard has pulled the boost back after the first block: peaks sit at the ceiling
        let peak = out[RATE / 50..]
            .iter()
            .map(|s| s.unsigned_abs())
            .max()
            .unwrap();
        assert!(
            peak as f32 <= 0.92 * 32767.0,
            "peak {peak} after the guard caught up"
        );
    }

    #[test]
    fn levelled_voices_summed_stay_within_full_scale() {
        // two raised speakers talking at once, through the mixer's limiter
        let (mut a, mut b) = (Leveller::new(), Leveller::new());
        run(&mut a, &speech(0.02, 15.0), true);
        run(&mut b, &speech(0.02, 15.0), true);
        let xa = run(&mut a, &speech(0.6, 1.0), true);
        let xb = run(&mut b, &speech(0.6, 1.0), true);
        let mut lim = Limiter::new(RATE as u32);
        for (p, q) in xa.iter().zip(&xb) {
            let sum = lim.process(*p as f32 / 32767.0 + *q as f32 / 32767.0);
            assert!(sum.abs() <= CEILING);
        }
    }

    /// A deep voice: 90 Hz with harmonics at 180 and 270 Hz, scaled so its peak is `peak`; `from` in samples.
    fn deep(peak: f32, from: usize, n: usize) -> Vec<f32> {
        let raw = |i: usize| {
            let t = i as f32 / RATE as f32;
            let tau = std::f32::consts::TAU;
            (tau * 90.0 * t).sin() + 0.5 * (tau * 180.0 * t).sin() + 0.33 * (tau * 270.0 * t).sin()
        };
        let top = (0..RATE).map(|i| raw(i).abs()).fold(0.0f32, f32::max);
        (from..from + n).map(|i| peak * raw(i) / top).collect()
    }

    fn to_i16(x: &[f32]) -> Vec<i16> {
        x.iter().map(|v| (v * 32767.0).round() as i16).collect()
    }

    /// Harmonics `hs` of 90 Hz in `x` (which must hold whole periods), as amplitudes.
    fn harmonic(x: &[f32], h: f32) -> f32 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, v) in x.iter().enumerate() {
            let ph = (std::f32::consts::TAU * h * 90.0 * i as f32 / RATE as f32) as f64;
            re += *v as f64 * ph.cos();
            im += *v as f64 * ph.sin();
        }
        ((re * re + im * im).sqrt() * 2.0 / x.len() as f64) as f32
    }

    /// Distortion added to the deep voice: harmonics it doesn't have (4..=15) over its fundamental, plus how far its
    /// own 2nd and 3rd harmonics moved relative to the fundamental.
    fn added_distortion(x: &[f32]) -> f32 {
        let f = harmonic(x, 1.0);
        let extra: f32 = (4..=15)
            .map(|h| harmonic(x, h as f32).powi(2))
            .sum::<f32>()
            .sqrt();
        extra / f + (harmonic(x, 2.0) / f - 0.5).abs() + (harmonic(x, 3.0) / f - 0.33).abs()
    }

    /// A leveller already raising this voice the full +12 dB (a quiet speaker heard for a while).
    fn boosted() -> Leveller {
        let mut lev = Leveller::new();
        lev.gain_db = MAX_BOOST_DB;
        lev.aim = db_to_gain(MAX_BOOST_DB);
        lev.gain = lev.aim;
        lev
    }

    /// #589: the case that crackled. A deep voice at −3 dBFS, raised +12 dB by the leveller and then +12 dB by a
    /// 200% slider, through the mixer: it comes out under the ceiling and still the same shape (quieter, not
    /// squarer). The tanh pair it replaces adds plenty.
    #[test]
    fn a_deep_voice_boosted_twice_comes_out_under_the_ceiling_and_clean() {
        use super::super::mixer::{SourceKind, max_gain, mixer};
        let peak_in = 10f32.powf(-3.0 / 20.0);
        let mut lev = boosted();
        let (input, mut m) = mixer(RATE as u32, 200);
        input.add_source(input.generation(), "a", "alex", SourceKind::Voice);
        input.set_gain("alex", SourceKind::Voice, max_gain());
        let (mut levelled, mut played) = (Vec::new(), Vec::new());
        let mut out = vec![0.0f32; 480];
        for k in 0..150 {
            let mut f = to_i16(&deep(peak_in, k * 480, 480));
            lev.process(&mut f, true);
            levelled.extend(f.iter().map(|s| *s as f32 / 32767.0));
            input.push("a", &f);
            // one frame queued ahead, so the queue sits at its target and the drift steering
            // never repeats a sample (that would be a glitch of its own, not the limiter's)
            if k > 0 {
                m.mix_into(&mut out, 1, 1.0);
                played.extend_from_slice(&out);
            }
        }
        assert_eq!(input.take_adjusted_samples(), 0);
        let peak = |x: &[f32]| x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        // the i16 conversion may round up by one step
        let one = 1.0 / 32767.0;
        assert!(
            peak(&levelled) <= CEILING + one,
            "leveller peak {}",
            peak(&levelled)
        );
        assert!(peak(&played) <= CEILING, "mixer peak {}", peak(&played));
        // steady state: the last 0.5 s (45 whole periods of 90 Hz)
        let tail = |x: &[f32]| x[x.len() - RATE / 2..].to_vec();
        let input_d = added_distortion(&deep(peak_in, 0, RATE / 2));
        let (lev_d, mix_d) = (
            added_distortion(&tail(&levelled)),
            added_distortion(&tail(&played)),
        );
        assert!(input_d < 0.01, "the test tone itself: {input_d}");
        assert!(lev_d < 0.02, "leveller added {:.2}%", lev_d * 100.0);
        assert!(mix_d < 0.02, "mixer added {:.2}%", mix_d * 100.0);
        assert!(
            peak(&tail(&played)) > 0.8 * CEILING,
            "limited, not silenced"
        );
        // the old chain, for scale
        let tanh: Vec<f32> = deep(peak_in, 0, RATE / 2)
            .iter()
            .map(|v| soft_limit(soft_limit(v * 4.0) * 4.0))
            .collect();
        assert!(added_distortion(&tanh) > 0.1, "{}", added_distortion(&tanh));
    }

    /// #589: someone quiet the leveller is raising suddenly gets loud. The block's peak is seen before its gain is
    /// applied, so not even the first block overshoots (the old guard reacted a block late), wherever in a block
    /// the burst starts.
    #[test]
    fn a_sudden_burst_after_quiet_never_overshoots_even_in_its_first_block() {
        for offset in [0, 1, 200, 479] {
            let mut lev = boosted();
            let mut x = to_i16(&deep(0.02, 0, RATE));
            let start = RATE + offset;
            x.resize(start, 0);
            x.extend(to_i16(&deep(0.8, start, RATE / 2)));
            let out = run(&mut lev, &x, true);
            for (i, s) in out.iter().enumerate() {
                assert!(
                    (s.unsigned_abs() as f32) <= CEILING * 32767.0 + 1.0,
                    "offset {offset}: sample {i} = {s}"
                );
            }
            // the burst's first block is turned down, not muted or bent out of shape
            let first = &out[start..start + 480];
            let p = first.iter().map(|s| s.unsigned_abs()).max().unwrap();
            assert!(p as f32 > 0.5 * 32767.0, "offset {offset}: peak {p}");
        }
    }

    #[test]
    fn off_leaves_the_audio_untouched_and_on_again_eases_in() {
        let x = speech(0.04, 10.0);
        let mut lev = Leveller::new();
        run(&mut lev, &x, true);
        assert!(lev.gain_db() > 3.0);
        let out = run(&mut lev, &x[..RATE], false);
        assert_eq!(out, x[..RATE].to_vec(), "bypassed when off");
        assert_eq!(lev.gain_db(), 0.0);
        // back on: starts at 0 dB and moves slowly, no jump to the old gain
        run(&mut lev, &x[..RATE / 10], true);
        assert!(lev.gain_db() < 1.0, "{}", lev.gain_db());
    }

    /// The setting: on by default, on for settings saved before it existed, and it reaches the
    /// mixer input the receive tasks read.
    #[test]
    fn even_out_voices_is_on_by_default_and_reaches_the_mixer() {
        use super::super::devices::{AudioConfig, Shared};
        assert!(AudioConfig::default().even_out_voices);
        let old: AudioConfig = serde_json::from_str(
            r#"{"input":null,"output":null,"input_gain_pct":100,"sensitivity":0.02,"auto_gain":false}"#,
        )
        .unwrap();
        assert!(old.even_out_voices, "saved before the setting: on");
        let s = Shared::new(&AudioConfig::default(), Default::default(), 200);
        assert!(s.mix_in.level_voices());
        let off = AudioConfig {
            even_out_voices: false,
            ..AudioConfig::default()
        };
        let s = Shared::new(&off, Default::default(), 200);
        assert!(!s.mix_in.level_voices());
        s.mix_in.set_level_voices(true);
        assert!(s.mix_in.level_voices());
    }
}
