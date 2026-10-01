//! Speaking indicator from real audio levels: lit = audible (LiveKit's own detector uses a stricter
//! threshold, so the ring and what you hear disagreed). Short hold so it doesn't flicker between words.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// RMS above which someone counts as audible.
pub const SPEAKING_RMS: f32 = 0.006;
const HOLD: Duration = Duration::from_millis(300);

#[derive(Default)]
pub struct SpeakingTracker {
    loud_at: HashMap<String, Instant>,
    current: Vec<String>,
}

impl SpeakingTracker {
    /// Feed the latest (id, rms) levels; returns the new speaking set only when it changed.
    pub fn update(&mut self, levels: &[(String, f32)], now: Instant) -> Option<Vec<String>> {
        for (id, rms) in levels {
            if *rms >= SPEAKING_RMS {
                self.loud_at.insert(id.clone(), now);
            }
        }
        self.loud_at.retain(|_, t| now.duration_since(*t) < HOLD);
        let mut speaking: Vec<String> = self.loud_at.keys().cloned().collect();
        speaking.sort();
        if speaking == self.current {
            return None;
        }
        self.current = speaking.clone();
        Some(speaking)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn lights_above_threshold_and_holds_briefly() {
        let t0 = Instant::now();
        let mut s = SpeakingTracker::default();
        assert_eq!(
            s.update(&[("a".into(), 0.001)], t0),
            None,
            "quiet: no change"
        );
        assert_eq!(
            s.update(&[("a".into(), 0.05)], t0),
            Some(vec!["a".to_string()])
        );
        assert_eq!(
            s.update(&[("a".into(), 0.0)], t0 + Duration::from_millis(200)),
            None,
            "held"
        );
        assert_eq!(
            s.update(&[("a".into(), 0.0)], t0 + Duration::from_millis(400)),
            Some(vec![])
        );
    }

    #[test]
    fn only_reports_changes_and_sorts() {
        let t0 = Instant::now();
        let mut s = SpeakingTracker::default();
        assert_eq!(
            s.update(&[("b".into(), 0.1), ("a".into(), 0.1)], t0),
            Some(vec!["a".into(), "b".into()])
        );
        assert_eq!(s.update(&[("b".into(), 0.1), ("a".into(), 0.1)], t0), None);
    }
}
