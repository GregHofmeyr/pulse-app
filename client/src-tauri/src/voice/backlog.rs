//! How much audio is waiting in one of our own queues: the part of the delay WebRTC's stats can't see.

use std::sync::atomic::{AtomicU32, Ordering};

use tokio::sync::mpsc::UnboundedReceiver;

#[derive(Default)]
pub struct QueueGauge {
    now_ms: AtomicU32,
    worst_ms: AtomicU32,
}

impl QueueGauge {
    pub fn record(&self, ms: u32) {
        self.now_ms.store(ms, Ordering::Relaxed);
        self.worst_ms.fetch_max(ms, Ordering::Relaxed);
    }

    pub fn current(&self) -> u32 {
        self.now_ms.load(Ordering::Relaxed)
    }

    /// The worst seen since the last call (one stats line each).
    pub fn take_worst(&self) -> u32 {
        self.worst_ms.swap(self.current(), Ordering::Relaxed)
    }
}

/// Crossfade length when skipping audio: 5 ms at 48 kHz, long enough not to click.
pub const FADE: usize = 240;

/// Skipping audio jumps from one point of the signal to another: an audible click. Fade `old`
/// (what would have played next) into the start of `new` (where we skip to) instead.
pub fn crossfade(old: &[i16], new: &mut [i16]) {
    let n = FADE.min(old.len()).min(new.len());
    for k in 0..n {
        let w = k as f32 / n as f32;
        new[k] = (old[k] as f32 * (1.0 - w) + new[k] as f32 * w) as i16;
    }
}

/// Drop the oldest queued items until at most `keep` remain. Returns how many were dropped and
/// the first of them: it continues the audio sent just before, so it's what to fade from.
/// Queued audio only drains at real time, so a backlog left alone is delay for the rest of the call.
pub fn drop_stale<T>(rx: &mut UnboundedReceiver<T>, keep: usize) -> (usize, Option<T>) {
    let (mut dropped, mut first) = (0, None);
    while rx.len() > keep {
        let Ok(item) = rx.try_recv() else { break };
        if first.is_none() {
            first = Some(item);
        }
        dropped += 1;
    }
    (dropped, first)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The largest step between neighbouring samples: what a click is.
    pub fn max_step(s: &[i16]) -> i32 {
        s.windows(2)
            .map(|w| (w[1] as i32 - w[0] as i32).abs())
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn a_crossfade_joins_two_signals_without_a_step() {
        // what would have played next (high) fades into what we skip to (low)
        let old = vec![10_000i16; 480];
        let mut new = vec![-10_000i16; 480];
        crossfade(&old, &mut new);
        assert_eq!(new[0], 10_000, "starts where the old audio was");
        assert!(max_step(&new) < 200, "step {}", max_step(&new));
        assert_eq!(
            new[FADE..],
            vec![-10_000; 480 - FADE][..],
            "the rest untouched"
        );
    }

    #[test]
    fn stale_audio_beyond_the_slack_is_dropped_oldest_first() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        for i in 0..30 {
            tx.send(i).unwrap();
        }
        assert_eq!(
            drop_stale(&mut rx, 10),
            (20, Some(0)),
            "count + the first dropped (to fade from)"
        );
        assert_eq!(rx.try_recv().unwrap(), 20, "the newest audio is kept");
        assert_eq!(rx.len(), 9);
    }

    #[test]
    fn a_queue_within_the_slack_is_left_alone() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        for i in 0..10 {
            tx.send(i).unwrap();
        }
        assert_eq!(drop_stale(&mut rx, 10), (0, None));
        assert_eq!(rx.len(), 10);
    }

    #[test]
    fn worst_is_kept_until_taken_then_restarts_from_now() {
        let g = QueueGauge::default();
        g.record(500);
        g.record(20);
        assert_eq!(g.current(), 20);
        assert_eq!(g.take_worst(), 500);
        assert_eq!(g.take_worst(), 20);
    }
}
