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
        self.observe(levels.iter().map(|(id, rms)| (id.as_str(), *rms)), now)
    }

    /// `update` from borrowed levels. A steady set allocates nothing: ids are copied only when
    /// someone new gets loud, and the set is collected only when it changed.
    pub fn observe<'a>(
        &mut self,
        levels: impl IntoIterator<Item = (&'a str, f32)>,
        now: Instant,
    ) -> Option<Vec<String>> {
        for (id, rms) in levels {
            if rms >= SPEAKING_RMS {
                match self.loud_at.get_mut(id) {
                    Some(t) => *t = now,
                    None => {
                        self.loud_at.insert(id.to_owned(), now);
                    }
                }
            }
        }
        self.loud_at.retain(|_, t| now.duration_since(*t) < HOLD);
        if self.loud_at.len() == self.current.len()
            && self.current.iter().all(|id| self.loud_at.contains_key(id))
        {
            return None;
        }
        let mut speaking: Vec<String> = self.loud_at.keys().cloned().collect();
        speaking.sort();
        self.current = speaking.clone();
        Some(speaking)
    }

    /// The speaking set as last reported by `update` (sorted).
    pub fn current(&self) -> &[String] {
        &self.current
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
    fn current_is_the_last_reported_set() {
        let t0 = Instant::now();
        let mut s = SpeakingTracker::default();
        assert!(s.current().is_empty());
        s.update(&[("b".into(), 0.1), ("a".into(), 0.1)], t0);
        assert_eq!(s.current(), &["a".to_string(), "b".to_string()]);
        s.update(&[("a".into(), 0.0)], t0 + Duration::from_millis(400));
        assert!(s.current().is_empty());
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
