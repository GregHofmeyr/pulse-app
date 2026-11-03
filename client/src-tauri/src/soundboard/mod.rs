//! Soundboard playback: decoding a sound's Ogg Opus file and keeping recently played ones decoded.

pub mod decode;
pub mod encode;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::voice::mixer::percent_to_gain;

/// Why adding sounds is unavailable (no ffmpeg to trim and normalise with).
pub const UNSUPPORTED: &str =
    "Adding sounds needs Pulse's audio tools (Windows), or ffmpeg on Linux.";

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

/// The last encoded span, so Preview then Save doesn't encode twice (a two-pass ffmpeg run).
#[derive(Default)]
pub struct Prepared(Mutex<Option<Span>>);

/// Source file, start and end (ms), and the encoded bytes.
type Span = (PathBuf, u32, u32, Vec<u8>);

impl Prepared {
    pub fn get(&self, path: &Path, start_ms: u32, end_ms: u32) -> Option<Vec<u8>> {
        let g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let (p, s, e, bytes) = g.as_ref()?;
        (p == path && *s == start_ms && *e == end_ms).then(|| bytes.clone())
    }

    pub fn put(&self, path: &Path, start_ms: u32, end_ms: u32, bytes: Vec<u8>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((path.to_path_buf(), start_ms, end_ms, bytes));
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
    fn prepared_is_reused_for_the_same_span() {
        let p = Prepared::default();
        let path = std::path::Path::new("/tmp/a.mp3");
        assert!(p.get(path, 0, 1000).is_none());
        p.put(path, 0, 1000, vec![1, 2, 3]);
        assert_eq!(p.get(path, 0, 1000), Some(vec![1, 2, 3]));
        assert!(
            p.get(path, 0, 2000).is_none(),
            "another span is another encode"
        );
        assert!(p.get(std::path::Path::new("/tmp/b.mp3"), 0, 1000).is_none());
        p.put(path, 0, 2000, vec![9]);
        assert!(
            p.get(path, 0, 1000).is_none(),
            "only the last encode is kept"
        );
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
