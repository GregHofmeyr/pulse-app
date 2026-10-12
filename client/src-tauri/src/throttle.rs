//! Log-flood guard: an error that repeats (an audio callback failing hundreds of times a second)
//! is logged once, then at most once per window with how many repeats were swallowed.

use std::time::{Duration, Instant};

pub struct Throttle {
    window: Duration,
    last: Option<Instant>,
    suppressed: u64,
}

impl Throttle {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            last: None,
            suppressed: 0,
        }
    }

    /// `Some(n)`: log now (n repeats were swallowed since the last line). `None`: stay quiet.
    pub fn hit(&mut self, now: Instant) -> Option<u64> {
        match self.last {
            Some(at) if now.saturating_duration_since(at) < self.window => {
                self.suppressed += 1;
                None
            }
            _ => {
                self.last = Some(now);
                Some(std::mem::take(&mut self.suppressed))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_hit_logs() {
        let mut t = Throttle::new(Duration::from_secs(10));
        assert_eq!(t.hit(Instant::now()), Some(0));
    }

    #[test]
    fn repeats_inside_the_window_are_counted_then_reported_once() {
        let start = Instant::now();
        let mut t = Throttle::new(Duration::from_secs(10));
        assert_eq!(t.hit(start), Some(0));
        for i in 1..=500 {
            assert_eq!(t.hit(start + Duration::from_millis(i)), None);
        }
        assert_eq!(t.hit(start + Duration::from_secs(11)), Some(500));
        assert_eq!(t.hit(start + Duration::from_secs(12)), None);
    }
}
