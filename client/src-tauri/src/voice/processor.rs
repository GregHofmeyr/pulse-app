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
        Self {
            times: VecDeque::with_capacity(OVERLOAD_WINDOW),
            sum: Duration::ZERO,
        }
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
        Self {
            level,
            denoiser: make_fast(level),
            gate: VoiceGate::new(auto, threshold),
            gain,
        }
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
            let took = if i == 350 {
                Duration::from_millis(40)
            } else {
                Duration::from_millis(2)
            };
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
            assert!(
                b.iter().all(|v| v.abs() <= 1.0),
                "block {k} exceeds full scale"
            );
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
            assert!(
                b.iter().all(|v| *v == 0.0) && !out.sent,
                "block {k} leaked after mute"
            );
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
