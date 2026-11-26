//! Per-speaker loudness for the voice-reactive speaking rings (#651, spec §5.2).
//!
//! Fed by the ring tick in `voice/mod.rs` (~14 Hz). Not an audio callback, but it runs all call long:
//! unchanged frames allocate nothing and send nothing.

use std::collections::{BTreeMap, HashMap};

/// Release per tick: a full ring falls back in about 0.6-0.8 s at 70 ms ticks.
const RELEASE: f32 = 0.7;

/// RMS to 0..1 for the ring: -44 dBFS (about `SPEAKING_RMS`, the speaking threshold) is 0, -14 dBFS
/// (loud speech after the leveller) is 1.
pub fn ring_level(rms: f32) -> f32 {
    if rms <= 0.0 {
        return 0.0;
    }
    ((20.0 * rms.log10() + 44.0) / 30.0).clamp(0.0, 1.0)
}

/// Rounded to steps of 0.05: bars fall back smoothly and unchanged frames can be skipped.
fn step(v: f32) -> f32 {
    (v * 20.0).round() / 20.0
}

#[derive(Default)]
pub struct RingLevels {
    /// Smoothed level per current speaker, unrounded (rounding it would stall the release at 0.05).
    shown: HashMap<String, f32>,
    /// The last frame sent; `None` while unwatched, so a new watcher gets the next frame.
    last_sent: Option<BTreeMap<String, f32>>,
}

impl RingLevels {
    /// One tick: `levels` = (id, rms) as measured, `speaking` = the tracker's current set (sorted),
    /// `watched` = a reactive ring is on screen. Returns the frame to send, if any: the speaking ids'
    /// levels when they changed, or one `{}` when the last speaker stops.
    pub fn frame(
        &mut self,
        levels: &[(String, f32)],
        speaking: &[String],
        watched: bool,
    ) -> Option<BTreeMap<String, f32>> {
        self.frame_from(
            levels.iter().map(|(id, rms)| (id.as_str(), *rms)),
            speaking,
            watched,
        )
    }

    /// `frame` from borrowed levels (the ring tick: peers' buffer plus your own level).
    pub fn frame_from<'a>(
        &mut self,
        levels: impl IntoIterator<Item = (&'a str, f32)>,
        speaking: &[String],
        watched: bool,
    ) -> Option<BTreeMap<String, f32>> {
        // only speakers have a ring: everyone else's state is dropped (attack is instant anyway)
        self.shown.retain(|id, _| speaking.contains(id));
        for (id, rms) in levels {
            if !speaking.iter().any(|s| s == id) {
                continue;
            }
            let new = ring_level(rms);
            match self.shown.get_mut(id) {
                Some(v) => *v = new.max(*v * RELEASE),
                None => {
                    self.shown.insert(id.to_owned(), new);
                }
            }
        }
        if !watched {
            self.last_sent = None;
            return None;
        }
        if speaking.is_empty() {
            return match &self.last_sent {
                Some(last) if !last.is_empty() => {
                    self.last_sent = Some(BTreeMap::new());
                    Some(BTreeMap::new())
                }
                _ => None,
            };
        }
        let level = |id: &String| step(self.shown.get(id).copied().unwrap_or(0.0));
        // compare before building: a steady frame allocates nothing (both sides are sorted by id)
        if let Some(last) = &self.last_sent
            && last.len() == speaking.len()
            && last
                .iter()
                .zip(speaking)
                .all(|((k, v), id)| k == id && *v == level(id))
        {
            return None;
        }
        let frame: BTreeMap<String, f32> =
            speaking.iter().map(|id| (id.clone(), level(id))).collect();
        self.last_sent = Some(frame.clone());
        Some(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::speaking::SPEAKING_RMS;
    use std::collections::BTreeMap;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn lv(v: &[(&str, f32)]) -> Vec<(String, f32)> {
        v.iter().map(|(id, rms)| (id.to_string(), *rms)).collect()
    }

    #[test]
    fn ring_level_maps_the_speaking_threshold_to_zero_and_loud_speech_to_one() {
        assert_eq!(ring_level(SPEAKING_RMS), 0.0);
        // -14 dBFS: f32 log10 lands a hair under 1.0
        assert!((ring_level(10f32.powf(-14.0 / 20.0)) - 1.0).abs() < 1e-5);
        assert_eq!(ring_level(1.0), 1.0);
        assert_eq!(ring_level(0.0), 0.0);
        assert_eq!(ring_level(-0.5), 0.0);
        assert_eq!(ring_level(1e-9), 0.0);
    }

    #[test]
    fn ring_level_is_monotonic() {
        let mut rms = 0.001f32;
        let mut last = ring_level(rms);
        while rms <= 1.0 {
            let l = ring_level(rms);
            assert!(l >= last, "{rms}: {l} < {last}");
            last = l;
            rms *= 1.1;
        }
    }

    #[test]
    fn levels_rise_at_once_and_fall_by_0_7_per_tick_in_0_05_steps() {
        let mut r = RingLevels::default();
        let speaking = ids(&["a"]);
        let first = r.frame(&lv(&[("a", 1.0)]), &speaking, true).unwrap();
        assert_eq!(first["a"], 1.0, "instant attack");
        let mut shown = vec![1.0];
        let mut last = 1.0;
        for _ in 0..20 {
            if let Some(m) = r.frame(&lv(&[("a", 0.0)]), &speaking, true) {
                last = m["a"];
                shown.push(last);
            }
        }
        assert_eq!(&shown[..4], &[1.0, 0.7, 0.5, 0.35], "0.49 shows as 0.5");
        for v in &shown {
            assert_eq!((v * 20.0).fract(), 0.0, "{v} is not a 0.05 step");
        }
        assert!(shown.windows(2).all(|w| w[1] < w[0]), "{shown:?}");
        assert_eq!(last, 0.0, "a quiet speaker's ring settles at zero");
    }

    #[test]
    fn a_louder_frame_rises_over_a_falling_one() {
        let mut r = RingLevels::default();
        let speaking = ids(&["a"]);
        r.frame(&lv(&[("a", 1.0)]), &speaking, true);
        let half = 10f32.powf(-29.0 / 20.0); // level 0.5
        assert_eq!(
            r.frame(&lv(&[("a", half)]), &speaking, true).unwrap()["a"],
            0.7
        );
        assert_eq!(
            r.frame(&lv(&[("a", 1.0)]), &speaking, true).unwrap()["a"],
            1.0
        );
    }

    #[test]
    fn only_speaking_ids_are_sent() {
        let mut r = RingLevels::default();
        let m = r
            .frame(&lv(&[("a", 0.5), ("b", 0.5)]), &ids(&["a"]), true)
            .unwrap();
        assert_eq!(m.keys().collect::<Vec<_>>(), vec!["a"]);
    }

    #[test]
    fn nothing_is_sent_in_silence() {
        let mut r = RingLevels::default();
        for _ in 0..10 {
            assert_eq!(r.frame(&lv(&[("a", 0.0)]), &[], true), None);
        }
    }

    #[test]
    fn one_empty_frame_when_the_last_speaker_stops() {
        let mut r = RingLevels::default();
        assert!(r.frame(&lv(&[("a", 0.5)]), &ids(&["a"]), true).is_some());
        assert_eq!(
            r.frame(&lv(&[("a", 0.0)]), &[], true),
            Some(BTreeMap::new())
        );
        assert_eq!(r.frame(&lv(&[("a", 0.0)]), &[], true), None);
    }

    #[test]
    fn nothing_while_unwatched_and_the_first_watched_frame_is_sent() {
        let mut r = RingLevels::default();
        let (levels, speaking) = (lv(&[("a", 0.5)]), ids(&["a"]));
        let sent = r.frame(&levels, &speaking, true).unwrap();
        assert_eq!(r.frame(&levels, &speaking, false), None);
        assert_eq!(
            r.frame(&levels, &speaking, true),
            Some(sent),
            "a new watcher needs the current levels even if unchanged"
        );
    }

    #[test]
    fn an_unchanged_frame_is_skipped() {
        let mut r = RingLevels::default();
        let (levels, speaking) = (lv(&[("a", 0.5)]), ids(&["a"]));
        assert!(r.frame(&levels, &speaking, true).is_some());
        assert_eq!(r.frame(&levels, &speaking, true), None);
        // someone else starts: the frame changed
        let both = r.frame(&lv(&[("a", 0.5), ("b", 0.5)]), &ids(&["a", "b"]), true);
        assert_eq!(both.map(|m| m.len()), Some(2));
    }

    #[test]
    fn a_speaker_with_no_level_reads_zero() {
        let mut r = RingLevels::default();
        let m = r.frame(&[], &ids(&["a"]), true).unwrap();
        assert_eq!(m["a"], 0.0);
    }
}
