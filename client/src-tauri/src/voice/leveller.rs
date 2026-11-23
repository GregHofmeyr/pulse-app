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
//! boosts noise. A peak guard pulls a boost back within a block when someone suddenly shouts, and
//! a soft limiter catches what slips through before the samples go back to i16; the mixer's own
//! limiter still guards the sum of everyone.

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

/// BS.1770 K-weighting at 48 kHz: stage 1 high shelf, stage 2 high-pass (RLB). (b0, b1, b2, a1, a2)
const SHELF: [f64; 5] = [
    1.535_124_859_586_97,
    -2.691_696_189_406_38,
    1.198_392_810_852_85,
    -1.690_659_293_182_41,
    0.732_480_774_215_85,
];
const HIGH_PASS: [f64; 5] = [1.0, -2.0, 1.0, -1.990_047_454_833_98, 0.990_072_250_366_21];

/// One biquad, direct form I (f64 state: the high-pass's poles sit very close to 1).
#[derive(Clone, Copy, Default)]
struct Biquad {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    fn run(&mut self, c: &[f64; 5], x: f64) -> f64 {
        let y = c[0] * x + c[1] * self.x1 + c[2] * self.x2 - c[3] * self.y1 - c[4] * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
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
    /// The gain applied to the current sample (linear), ramping by `step` per sample towards the
    /// block's aim, so a gain change never clicks.
    gain: f32,
    step: f32,
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
            gain: 1.0,
            step: 0.0,
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
            self.gain = 1.0;
            self.step = 0.0;
            return;
        }
        for s in samples {
            let x = *s as f32 / 32768.0;
            let k = self
                .high_pass
                .run(&HIGH_PASS, self.shelf.run(&SHELF, x as f64));
            self.energy += k * k;
            self.peak = self.peak.max(x.abs());
            self.filled += 1;
            let y = soft_limit(x * self.gain);
            self.gain += self.step;
            *s = (y * 32767.0).round() as i16;
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
        // The peak guard: never let a boost push this block's peak past the ceiling. It reacts a
        // block late (the soft limiter covers that block), so it only relaxes while someone is
        // audible: a gap between words keeps it, or every syllable after a shout would overshoot.
        if level > ABS_GATE_LUFS {
            let room = 20.0 * (PEAK_CEILING / peak.max(1e-6)).log10();
            self.guard_db = room.min(self.guard_db + GUARD_RELEASE_DB_PER_S / BLOCKS_PER_S);
        }
        // it only ever takes a boost back: unboosted audio is as it arrived
        let aim = db_to_gain(self.gain_db.min(self.guard_db.max(0.0)));
        self.step = (aim - self.gain) / BLOCK as f32;
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
        for (p, q) in xa.iter().zip(&xb) {
            let sum = soft_limit(*p as f32 / 32767.0 + *q as f32 / 32767.0);
            assert!(sum.abs() <= 1.0);
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
