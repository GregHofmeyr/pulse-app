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

    /// Gate one block in place (input in, the block from 20 ms ago out). Returns whether audio is sent.
    pub fn process(&mut self, block: &mut [f32], prob: Option<f32>, now: Instant) -> bool {
        if self.qualifies(rms(block), prob) {
            self.open_until = Some(now + HOLD);
        }
        let open = self.open_until.is_some_and(|t| now < t);
        self.delay.push_back(block.to_vec());
        let out = self.delay.pop_front().expect("pre-roll buffer");
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
        self.gain = end;
        start > 0.0 || end > 0.0
    }
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
}
