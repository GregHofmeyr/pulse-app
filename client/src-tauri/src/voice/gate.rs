//! Voice gate: decides when the mic is "on". Automatic mode opens on speech (model probability with
//! hysteresis) that is also clearly above the room's noise floor; manual mode on an RMS threshold.
//! Both hold 450 ms, fade out over 100 ms, and keep 20 ms of pre-roll so first syllables survive.
//! The pre-roll is only for the opening edge: once open, the gate catches up in the first quiet
//! moment (a pause between words) and then passes audio straight through, with no delay.

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
const HOLD: Duration = Duration::from_millis(450);
const FADE_BLOCKS: f32 = 10.0;
/// Pre-roll in front of the audio the gate opens on: 20 ms, so first syllables survive. Part or all
/// of it can come from the denoiser's own delay.
pub const PREROLL_BLOCKS: usize = 2;
/// With a manual threshold of 0 (always open) only this quiet counts as a pause to catch up in.
const QUIET_DB: f32 = -50.0;

pub struct VoiceGate {
    auto: bool,
    threshold: f32,
    speech: bool,
    /// Room noise floor in dB; learned from the first block.
    floor_db: Option<f32>,
    open_until: Option<Instant>,
    /// Gain applied at the start of the next emitted block (fades towards 0 when closed).
    gain: f32,
    /// The last `preroll` input blocks, oldest first: what an opening edge releases.
    history: VecDeque<Vec<f32>>,
    /// How many blocks behind the input the output runs now: the full pre-roll while closed,
    /// down to 0 once an open gate has caught up.
    lag: usize,
    /// Scratch for the block going out (no allocation per block).
    out: Vec<f32>,
    /// Blocks since the gate last opened from silence: no catching up until the pre-roll and
    /// the block that opened it have gone out (the pre-roll is what an opening is for).
    since_open: usize,
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
            history: (0..PREROLL_BLOCKS).map(|_| vec![0.0; FRAME]).collect(),
            lag: PREROLL_BLOCKS,
            out: vec![0.0; FRAME],
            since_open: 0,
        }
    }

    pub fn configure(&mut self, auto: bool, threshold: f32) {
        self.auto = auto;
        self.threshold = threshold;
    }

    /// Automatic (speech) mode, as opposed to a manual threshold.
    pub fn is_auto(&self) -> bool {
        self.auto
    }

    /// Forget buffered audio and any open state (mute: nothing may leak afterwards).
    pub fn reset(&mut self) {
        self.open_until = None;
        self.gain = 0.0;
        self.speech = false;
        self.history.iter_mut().for_each(|b| b.fill(0.0));
        self.lag = self.history.len();
        self.since_open = 0;
    }

    /// Blocks of delay the gate adds right now (0 while open and caught up).
    pub fn lag_blocks(&self) -> usize {
        self.lag
    }

    /// Quiet enough to skip without anyone hearing it: well under what would open the gate.
    fn quiet(&self, b: &[f32]) -> bool {
        let level = rms(b);
        if self.auto {
            let floor = self.floor_db.unwrap_or(FLOOR_MIN_DB);
            db(level) <= floor + FLOOR_MARGIN_DB
        } else if self.threshold > 0.0 {
            db(level) <= db(self.threshold) - FLOOR_MARGIN_DB
        } else {
            db(level) <= QUIET_DB // always open: only near-silence
        }
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
        self.floor_db = Some(if level_db < floor {
            level_db
        } else {
            (floor + rise).min(level_db)
        });
        match prob {
            Some(p) if p >= OPEN_PROB => self.speech = true,
            Some(p) if p < CLOSE_PROB => self.speech = false,
            Some(_) => {}
            None => self.speech = true,
        }
        self.speech && above_floor
    }

    /// How many blocks of delay the gate itself adds (the rest of the pre-roll is the denoiser's).
    pub fn set_preroll(&mut self, blocks: usize) {
        if self.history.len() == blocks {
            return;
        }
        while self.history.len() > blocks {
            self.history.pop_front();
        }
        while self.history.len() < blocks {
            self.history.push_front(vec![0.0; FRAME]);
        }
        self.lag = self.lag.min(blocks);
    }

    /// The block from `ago` blocks back (0 = `block` itself).
    fn back<'a>(&'a self, block: &'a [f32], ago: usize) -> &'a [f32] {
        if ago == 0 {
            block
        } else {
            &self.history[self.history.len() - ago]
        }
    }

    /// Gate one block in place (input in, the block from `lag_blocks()` ago out), deciding on
    /// `level`: the newest audio's, which may run ahead of `block`. Returns whether audio is sent.
    ///
    /// Closed, the output runs the full pre-roll behind, so an opening edge releases the audio
    /// just before it. Open, every quiet moment (both the block that would play and the next one
    /// under the opening level, and no speech) catches up one block: the quiet one is skipped,
    /// crossfaded into the next. Then audio passes straight through. Fully closed again (faded to
    /// 0), the output falls back the full pre-roll behind, unheard.
    pub fn process(
        &mut self,
        block: &mut [f32],
        level: f32,
        prob: Option<f32>,
        now: Instant,
    ) -> bool {
        if self.qualifies(level, prob) {
            self.open_until = Some(now + HOLD);
        }
        let open = self.open_until.is_some_and(|t| now < t);
        if !open && self.gain == 0.0 {
            self.lag = self.history.len(); // silent: re-arm the pre-roll for the next opening
            self.since_open = 0;
        }
        let mut out = std::mem::take(&mut self.out);
        out.resize(block.len(), 0.0); // FRAME in practice: no allocation
        let catch_up = open
            && self.lag > 0
            && self.since_open > self.history.len()
            && !(self.auto && self.speech)
            && self.quiet(self.back(block, self.lag))
            && self.quiet(self.back(block, self.lag - 1));
        if catch_up {
            let (skip, next) = (self.back(block, self.lag), self.back(block, self.lag - 1));
            let n = out.len() as f32;
            for (i, o) in out.iter_mut().enumerate() {
                let t = i as f32 / n;
                *o = skip[i] * (1.0 - t) + next[i] * t;
            }
            self.lag -= 1;
        } else {
            out.copy_from_slice(self.back(block, self.lag));
        }
        // the newest block joins the history (its oldest buffer reused: nothing allocated)
        if let Some(mut oldest) = self.history.pop_front() {
            oldest.resize(block.len(), 0.0);
            oldest.copy_from_slice(block);
            self.history.push_back(oldest);
        }
        let start = self.gain;
        let end = if open {
            1.0
        } else {
            (start - 1.0 / FADE_BLOCKS).max(0.0)
        };
        let n = out.len() as f32;
        for (i, (o, v)) in block.iter_mut().zip(&out).enumerate() {
            *o = v * (start + (end - start) * i as f32 / n);
        }
        self.out = out;
        if open {
            self.since_open += 1;
        }
        self.gain = end;
        start > 0.0 || end > 0.0
    }
}

fn rms(b: &[f32]) -> f32 {
    (b.iter().map(|v| v * v).sum::<f32>() / b.len().max(1) as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn block(amp: f32) -> Vec<f32> {
        (0..FRAME)
            .map(|i| if i % 2 == 0 { amp } else { -amp })
            .collect()
    }
    fn rms(b: &[f32]) -> f32 {
        (b.iter().map(|v| v * v).sum::<f32>() / b.len() as f32).sqrt()
    }
    /// Feed `n` blocks of `amp`/`prob` starting at block index `*k`; returns the output blocks.
    fn feed(
        g: &mut VoiceGate,
        t0: Instant,
        k: &mut u64,
        n: usize,
        amp: f32,
        prob: Option<f32>,
    ) -> Vec<Vec<f32>> {
        (0..n)
            .map(|_| {
                let mut b = block(amp);
                let level = rms(&b);
                g.process(&mut b, level, prob, t0 + Duration::from_millis(*k * 10));
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
        assert!(
            out.iter().all(|b| rms(b) == 0.0),
            "quiet background speech must not open the gate"
        );
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
        assert!(
            rms(out.last().unwrap()) > 0.05,
            "0.45 is above the close threshold"
        );
    }

    #[test]
    fn holds_450ms_then_fades_over_100ms() {
        let (t0, mut k) = (Instant::now(), 0);
        let mut g = VoiceGate::new(true, 0.02);
        feed(&mut g, t0, &mut k, 100, 0.001, Some(0.0));
        feed(&mut g, t0, &mut k, 10, 0.1, Some(0.9));
        let out = feed(&mut g, t0, &mut k, 70, 0.1, Some(0.0)); // speech prob drops
        // 2 blocks of pre-roll delay + 45 blocks hold: still full level (word endings survive)
        assert!((rms(&out[40]) - 0.1).abs() < 1e-3, "inside hold");
        let fading = rms(&out[52]);
        assert!(fading > 0.0 && fading < 0.09, "fading: {fading}");
        assert_eq!(rms(&out[60]), 0.0, "closed after 100 ms fade");
    }

    #[test]
    fn preroll_releases_the_20ms_before_the_gate_opened() {
        let (t0, mut k) = (Instant::now(), 0);
        let mut g = VoiceGate::new(false, 0.05); // manual: open at rms >= 0.05
        feed(&mut g, t0, &mut k, 10, 0.0, None);
        let mut onset = feed(&mut g, t0, &mut k, 1, 0.03, None); // soft first syllable (below threshold)
        onset.extend(feed(&mut g, t0, &mut k, 1, 0.03, None));
        let out = feed(&mut g, t0, &mut k, 3, 0.1, None); // loud: opens
        assert!(
            rms(&out[1]) > 0.02 && rms(&out[2]) > 0.02,
            "the soft onset is sent, not clipped"
        );
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
        assert!(
            out.iter().all(|b| rms(b) == 0.0),
            "floor should have risen to the room"
        );
    }

    /// Blocks numbered by their amplitude, so the output says which input block it was.
    fn marked(k: u64) -> Vec<f32> {
        vec![0.1 + k as f32 * 0.001; FRAME]
    }

    #[test]
    fn continuous_speech_keeps_every_sample_behind_the_preroll() {
        let t0 = Instant::now();
        let mut g = VoiceGate::new(false, 0.05);
        for k in 0..50u64 {
            let mut b = marked(k);
            let level = rms(&b);
            g.process(&mut b, level, None, t0 + Duration::from_millis(k * 10));
            if k > PREROLL_BLOCKS as u64 {
                // fully open: block k - 2 comes out whole (nothing skipped mid-speech)
                assert!((b[0] - marked(k - 2)[0]).abs() < 1e-6, "block {k}");
            }
        }
        assert_eq!(g.lag_blocks(), PREROLL_BLOCKS);
    }

    #[test]
    fn once_open_a_pause_catches_up_and_then_audio_passes_straight_through() {
        let (t0, mut k) = (Instant::now(), 0u64);
        let mut g = VoiceGate::new(false, 0.05);
        feed(&mut g, t0, &mut k, 10, 0.0, None);
        feed(&mut g, t0, &mut k, 5, 0.1, None); // opens: 20 ms behind
        assert_eq!(g.lag_blocks(), PREROLL_BLOCKS);
        feed(&mut g, t0, &mut k, 4, 0.0, None); // a pause between words, inside the hold
        assert_eq!(g.lag_blocks(), 0, "caught up in the pause");
        for _ in 0..5 {
            let mut b = marked(k);
            let want = b.clone();
            let level = rms(&b);
            assert!(g.process(&mut b, level, None, t0 + Duration::from_millis(k * 10)));
            assert_eq!(b, want, "block {k}: straight through, no delay");
            k += 1;
        }
    }

    #[test]
    fn catching_up_crossfades_instead_of_jumping() {
        let (t0, mut k) = (Instant::now(), 0u64);
        let mut g = VoiceGate::new(false, 0.05);
        feed(&mut g, t0, &mut k, 10, 0.0, None);
        feed(&mut g, t0, &mut k, 5, 0.1, None);
        let out = feed(&mut g, t0, &mut k, 4, 0.01, None); // quiet tail, still open
        let step = out
            .concat()
            .windows(2)
            .map(|w| (w[1].abs() - w[0].abs()).abs())
            .fold(0.0f32, f32::max);
        assert!(step < 0.1, "a jump of {step}");
    }

    #[test]
    fn closing_rearms_the_preroll_for_the_next_onset() {
        let (t0, mut k) = (Instant::now(), 0u64);
        let mut g = VoiceGate::new(false, 0.05);
        feed(&mut g, t0, &mut k, 10, 0.0, None);
        feed(&mut g, t0, &mut k, 5, 0.1, None);
        feed(&mut g, t0, &mut k, 4, 0.0, None);
        assert_eq!(g.lag_blocks(), 0);
        feed(&mut g, t0, &mut k, 70, 0.0, None); // past the hold and the fade
        assert_eq!(g.lag_blocks(), PREROLL_BLOCKS, "re-armed while silent");
        let mut onset = feed(&mut g, t0, &mut k, 2, 0.03, None); // soft first syllable
        onset.extend(feed(&mut g, t0, &mut k, 3, 0.1, None));
        assert!(
            rms(&onset[3]) > 0.02 && rms(&onset[4]) > 0.02,
            "the soft onset is sent again"
        );
    }

    #[test]
    fn auto_does_not_catch_up_while_the_model_hears_speech() {
        let (t0, mut k) = (Instant::now(), 0u64);
        let mut g = VoiceGate::new(true, 0.02);
        feed(&mut g, t0, &mut k, 100, 0.001, Some(0.0));
        feed(&mut g, t0, &mut k, 5, 0.1, Some(0.9));
        feed(&mut g, t0, &mut k, 3, 0.001, Some(0.9)); // a soft stretch the model calls speech
        assert_eq!(g.lag_blocks(), PREROLL_BLOCKS, "speech is never skipped");
        feed(&mut g, t0, &mut k, 3, 0.001, Some(0.1)); // a real pause
        assert_eq!(g.lag_blocks(), 0);
    }

    #[test]
    fn mute_reset_rearms_the_preroll() {
        let (t0, mut k) = (Instant::now(), 0u64);
        let mut g = VoiceGate::new(false, 0.05);
        feed(&mut g, t0, &mut k, 5, 0.1, None);
        feed(&mut g, t0, &mut k, 4, 0.0, None);
        assert_eq!(g.lag_blocks(), 0);
        g.reset();
        assert_eq!(g.lag_blocks(), PREROLL_BLOCKS);
    }
}
