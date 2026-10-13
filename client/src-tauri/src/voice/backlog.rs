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

/// Drop the oldest queued items until at most `keep` remain; returns how many were dropped.
/// Queued audio only drains at real time, so a backlog left alone is delay for the rest of the call.
pub fn drop_stale<T>(rx: &mut UnboundedReceiver<T>, keep: usize) -> usize {
    let mut dropped = 0;
    while rx.len() > keep && rx.try_recv().is_ok() {
        dropped += 1;
    }
    dropped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_audio_beyond_the_slack_is_dropped_oldest_first() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        for i in 0..30 {
            tx.send(i).unwrap();
        }
        assert_eq!(drop_stale(&mut rx, 10), 20);
        assert_eq!(rx.try_recv().unwrap(), 20, "the newest audio is kept");
        assert_eq!(rx.len(), 9);
    }

    #[test]
    fn a_queue_within_the_slack_is_left_alone() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        for i in 0..10 {
            tx.send(i).unwrap();
        }
        assert_eq!(drop_stale(&mut rx, 10), 0);
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
