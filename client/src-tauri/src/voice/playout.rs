//! Device-rate playback fed from the 48 kHz mixer.

use std::collections::VecDeque;

use super::devices::INTERNAL_RATE;
use super::resampler::StreamResampler;

/// Pulls the mixer in whole 10 ms blocks only when the device needs more, so over time we take
/// exactly what the device consumes (never more: over-pulling drains the peer queues into clicks).
pub struct Playout {
    passthrough: bool,
    rs: StreamResampler,
    block: Vec<f32>,
    fifo: VecDeque<f32>,
}

impl Playout {
    pub fn new(device_rate: u32) -> Self {
        Self {
            passthrough: device_rate == INTERNAL_RATE,
            rs: StreamResampler::new(INTERNAL_RATE, device_rate),
            block: vec![0.0; (INTERNAL_RATE / 100) as usize],
            fifo: VecDeque::new(),
        }
    }

    /// Fill mono `out` at the device rate; `pull` mixes 48 kHz mono into the buffer it's given.
    pub fn fill(&mut self, out: &mut [f32], mut pull: impl FnMut(&mut [f32])) {
        if self.passthrough {
            pull(out);
            return;
        }
        while self.fifo.len() < out.len() {
            pull(&mut self.block);
            self.rs.push(&self.block, &mut self.fifo);
        }
        for o in out.iter_mut() {
            *o = self.fifo.pop_front().unwrap_or(0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The device consumes audio at its own rate; over time we must take from the 48 kHz mixer
    /// exactly what that consumption corresponds to. Taking more drains the peer queues, which
    /// then run dry and insert silence: audible clicks.
    #[test]
    fn pulls_from_the_mixer_at_exactly_the_device_rate() {
        let mut p = Playout::new(44_100);
        let mut pulled = 0u64;
        let mut consumed = 0u64;
        let mut out = vec![0.0; 1102]; // a typical PipeWire callback at 44.1 kHz
        for _ in 0..2400 {
            // ~60 s
            p.fill(&mut out, |buf| pulled += buf.len() as u64);
            consumed += out.len() as u64;
        }
        let expected = consumed * 48_000 / 44_100;
        let drift = pulled as i64 - expected as i64;
        assert!(
            drift.abs() <= 480,
            "pulled {pulled} vs {expected} expected: drift {drift} samples after 60 s"
        );
    }
}
