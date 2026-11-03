//! Soundboard playback: decoding a sound's Ogg Opus file and keeping recently played ones decoded.

pub mod decode;

use std::sync::{Arc, Mutex};

use crate::voice::mixer::percent_to_gain;

/// Decoded sounds kept for instant replays (a 6 s mono sound is about 1.1 MB, so ~27 MB at most).
const CACHE_ENTRIES: usize = 24;

/// Linear gain for a sound: the soundboard volume (capped at 100 %) times the sound's own (10-200 %).
pub fn gain(soundboard_pct: u16, sound_pct: u16) -> f32 {
    percent_to_gain(soundboard_pct.min(100)) * percent_to_gain(sound_pct.clamp(10, 200))
}

/// The most recently used decoded sounds, by content hash. Most recent last.
#[derive(Default)]
pub struct Cache(Mutex<Vec<(String, Arc<[f32]>)>>);

impl Cache {
    /// The decoded sound for `hash`, marked as just used.
    pub fn get(&self, hash: &str) -> Option<Arc<[f32]>> {
        let mut c = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let i = c.iter().position(|(h, _)| h == hash)?;
        let entry = c.remove(i);
        let samples = entry.1.clone();
        c.push(entry);
        Some(samples)
    }

    /// Decode `bytes` and remember the result under `hash`, dropping the least recently used
    /// beyond the limit. The cache's clone is what keeps a sound's data alive while the audio
    /// callback finishes with (and drops) its own.
    pub fn insert_decoded(&self, hash: &str, bytes: &[u8]) -> Result<Arc<[f32]>, String> {
        let samples = decode::decode(bytes)?;
        self.put(hash, samples.clone());
        Ok(samples)
    }

    fn put(&self, hash: &str, samples: Arc<[f32]>) {
        let mut c = self.0.lock().unwrap_or_else(|e| e.into_inner());
        c.retain(|(h, _)| h != hash);
        c.push((hash.to_string(), samples));
        if c.len() > CACHE_ENTRIES {
            c.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_combines_the_two_volumes() {
        assert_eq!(gain(0, 100), 0.0);
        assert_eq!(gain(100, 100), 1.0);
        assert_eq!(gain(150, 100), 1.0, "soundboard volume caps at 100 %");
        assert_eq!(
            gain(100, 0),
            percent_to_gain(10),
            "a sound is never below 10 %"
        );
        assert_eq!(gain(100, 500), percent_to_gain(200));
    }

    #[test]
    fn cache_refuses_garbage() {
        let c = Cache::default();
        assert!(c.insert_decoded("x", b"nope").is_err());
        assert!(c.get("x").is_none());
    }

    #[test]
    fn cache_forgets_the_least_recently_used() {
        let c = Cache::default();
        let one: Arc<[f32]> = Arc::from(vec![0.0f32; 1]);
        for i in 0..CACHE_ENTRIES {
            c.put(&format!("h{i}"), one.clone());
        }
        assert!(c.get("h0").is_some(), "h0 is used again, so h1 is oldest");
        c.put("new", one.clone());
        assert!(c.get("h1").is_none());
        assert!(c.get("h0").is_some());
        assert!(c.get("new").is_some());
    }
}
