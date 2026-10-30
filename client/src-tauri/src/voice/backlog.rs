//! How much audio is waiting in one of our own queues: the part of the delay WebRTC's stats can't see.

use std::sync::atomic::{AtomicU32, Ordering};

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

/// A gap in what we send longer than this is a stalled mic, not a burst: it's filled with silence.
const GAP_LIMIT: std::time::Duration = std::time::Duration::from_millis(50);
const FRAME: std::time::Duration = std::time::Duration::from_millis(10);

/// The send path hands each frame straight to WebRTC, so if the mic stops (a stall, a device
/// reopen) nothing goes out at all, and listeners' jitter buffers grow to cover the hole. Keep the
/// stream continuous: a gap past `GAP_LIMIT` is covered with silent 10 ms frames, then (until the
/// mic is back) one frame per 10 ms, so the stream keeps real-time cadence instead of bursts.
pub struct GapFill {
    last: std::time::Instant,
    /// A gap was declared and is being bridged: every whole frame of gap is due at once.
    filling: bool,
}

impl GapFill {
    pub fn new(now: std::time::Instant) -> Self {
        Self {
            last: now,
            filling: false,
        }
    }

    /// A real frame went out.
    pub fn sent(&mut self, now: std::time::Instant) {
        self.last = now;
        self.filling = false;
    }

    /// How many silent frames to send now (0 unless the gap is past the limit, or a frame long
    /// while already filling).
    pub fn silence_due(&self, now: std::time::Instant) -> usize {
        let gap = now.saturating_duration_since(self.last);
        if !self.filling && gap <= GAP_LIMIT {
            return 0;
        }
        (gap.as_millis() / FRAME.as_millis()) as usize
    }

    pub fn silence_sent(&mut self, frames: usize) {
        if frames > 0 {
            self.last += FRAME * frames as u32;
            self.filling = true;
        }
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
    fn a_stalled_mic_is_bridged_with_silence() {
        let t0 = std::time::Instant::now();
        let ms = |n| t0 + std::time::Duration::from_millis(n);
        let mut g = GapFill::new(t0);
        // mic bursts (a BT device delivers 30 ms at a time) are normal: nothing to fill
        g.sent(ms(30));
        assert_eq!(g.silence_due(ms(60)), 0);
        // the mic stopped: once the gap passes the limit, cover all of it in 10 ms frames
        assert_eq!(g.silence_due(ms(130)), 10, "100 ms since the last frame");
        g.silence_sent(10);
        assert_eq!(g.silence_due(ms(135)), 0, "caught up");
        assert_eq!(
            g.silence_due(ms(140)),
            1,
            "still filling: each frame's worth is due"
        );
    }

    /// Once a stall has been declared and bridged, the stream continues at real-time cadence
    /// (one frame per 10 ms check), not in bursts of 5-6 every time the gap passes the limit again.
    #[test]
    fn a_long_stall_is_bridged_one_frame_per_tick_after_the_first_fill() {
        let t0 = std::time::Instant::now();
        let ms = |n| t0 + std::time::Duration::from_millis(n);
        let mut g = GapFill::new(t0);
        let mut per_tick = Vec::new();
        for tick in 1..=30 {
            let due = g.silence_due(ms(tick * 10));
            g.silence_sent(due);
            per_tick.push(due);
        }
        let first = per_tick.iter().position(|&n| n > 0).unwrap();
        assert_eq!(first, 5, "declared once the gap passes 50 ms: {per_tick:?}");
        assert_eq!(per_tick[first], 6, "the first fill covers the whole gap");
        assert!(
            per_tick[first + 1..].iter().all(|&n| n == 1),
            "then one frame per tick: {per_tick:?}"
        );
        assert_eq!(per_tick.iter().sum::<usize>(), 30, "all 300 ms covered");
        // a real frame ends the fill: the next gap must pass the limit again
        g.sent(ms(300));
        assert_eq!(g.silence_due(ms(320)), 0);
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
    fn worst_is_kept_until_taken_then_restarts_from_now() {
        let g = QueueGauge::default();
        g.record(500);
        g.record(20);
        assert_eq!(g.current(), 20);
        assert_eq!(g.take_worst(), 500);
        assert_eq!(g.take_worst(), 20);
    }
}
