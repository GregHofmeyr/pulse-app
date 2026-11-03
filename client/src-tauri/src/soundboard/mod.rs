//! Soundboard playback: decoding a sound's Ogg Opus file and keeping recently played ones decoded.

pub mod decode;
pub mod encode;
pub mod pcm;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::voice::mixer::combined_gain;
#[cfg(test)]
use crate::voice::mixer::percent_to_gain;
use pcm::Pcm;

/// Why adding sounds is unavailable (no ffmpeg to trim and normalise with).
pub const UNSUPPORTED: &str =
    "Adding sounds needs Pulse's audio tools (Windows), or ffmpeg on Linux.";

/// Decoded sounds kept for instant replays (a 6 s mono sound is about 1.1 MB, so ~27 MB at most).
const CACHE_ENTRIES: usize = 24;

/// Linear gain for a sound: the volume mixer's Calls master × its Soundboard notch × the sound's
/// own volume (10-200 %), capped at the mixer's safe maximum.
pub fn gain(calls_pct: u16, soundboard_pct: u16, sound_pct: u16) -> f32 {
    combined_gain(&[calls_pct, soundboard_pct, sound_pct.clamp(10, 200)])
}

/// Most sounds one warm-up fetches (a call's worth of servers' sounds; each is ~50 KB on disk).
pub const WARM_MAX: usize = 200;

/// Where a sound's samples came from, and what getting them cost (the play-delay log).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoadTiming {
    /// Already decoded (in memory), so nothing was fetched or decoded.
    pub cached: bool,
    pub fetch_ms: f64,
    /// From the file being in hand to its first [`decode::START_SAMPLES`] being playable.
    pub start_ms: f64,
}

const CACHED: LoadTiming = LoadTiming {
    cached: true,
    fetch_ms: 0.0,
    start_ms: 0.0,
};

/// The most recently used decoded sounds, by content hash (most recent last), plus a gate per
/// sound being loaded so two plays fetch and decode it once. An entry may still be decoding.
#[derive(Default)]
pub struct Cache {
    lru: Mutex<Vec<(String, Arc<Pcm>)>>,
    loading: Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Cache {
    /// The decoded sound for `hash`, marked as just used.
    pub fn get(&self, hash: &str) -> Option<Arc<Pcm>> {
        let mut c = self.lru.lock().unwrap_or_else(|e| e.into_inner());
        let i = c.iter().position(|(h, _)| h == hash)?;
        let entry = c.remove(i);
        let samples = entry.1.clone();
        c.push(entry);
        Some(samples)
    }

    fn contains(&self, hash: &str) -> bool {
        let c = self.lru.lock().unwrap_or_else(|e| e.into_inner());
        c.iter().any(|(h, _)| h == hash)
    }

    /// The decoded sound for `hash`: from memory, else `fetch` it and start decoding it (on a
    /// blocking thread; ready to play after its first 100 ms) and remember it, dropping the least
    /// recently used beyond the limit. Loads of one hash queue behind each other, so `fetch` runs
    /// once. The cache's clone is what keeps a sound's data alive while the audio callback
    /// finishes with (and drops) its own.
    pub async fn load<F, Fut>(&self, hash: &str, fetch: F) -> Result<(Arc<Pcm>, LoadTiming), String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<u8>, String>>,
    {
        if let Some(s) = self.get(hash) {
            return Ok((s, CACHED));
        }
        let gate = {
            let mut l = self.loading.lock().unwrap_or_else(|e| e.into_inner());
            l.entry(hash.to_string()).or_default().clone()
        };
        let _turn = gate.lock().await;
        // the load we waited for may have done it
        if let Some(s) = self.get(hash) {
            return Ok((s, CACHED));
        }
        let result = async {
            let t = std::time::Instant::now();
            let bytes = fetch().await?;
            let fetch_ms = t.elapsed().as_secs_f64() * 1000.0;
            let t = std::time::Instant::now();
            let samples = decode::decode_streaming(bytes).await?;
            let start_ms = t.elapsed().as_secs_f64() * 1000.0;
            self.put(hash, samples.clone());
            Ok((
                samples,
                LoadTiming {
                    cached: false,
                    fetch_ms,
                    start_ms,
                },
            ))
        }
        .await;
        // still holding our turn: anyone queued behind finds it in memory (or tries again)
        self.loading
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(hash);
        result
    }

    /// Get sounds ready before anyone plays them (joining a call): in order, fetch each one not
    /// already decoded, which fills the media disk cache, so a first play needs no network. It
    /// decodes nothing: a streamed decode starts playing in milliseconds, while decoding a call's
    /// worth of sounds ahead would burn ~0.8 s of CPU per 6 s sound on every join. Failures are
    /// skipped.
    pub async fn warm<F, Fut>(&self, hashes: &[String], fetch: F)
    where
        F: Fn(String) -> Fut,
        Fut: Future<Output = Result<Vec<u8>, String>>,
    {
        for hash in hashes.iter().take(WARM_MAX) {
            if self.contains(hash) {
                continue;
            }
            if let Err(e) = fetch(hash.clone()).await {
                tracing::debug!(error = %e, "soundboard: warm-up skipped a sound");
            }
        }
    }

    fn put(&self, hash: &str, samples: Arc<Pcm>) {
        let mut c = self.lru.lock().unwrap_or_else(|e| e.into_inner());
        c.retain(|(h, _)| h != hash);
        c.push((hash.to_string(), samples));
        if c.len() > CACHE_ENTRIES {
            c.remove(0);
        }
    }
}

/// The trimmer's one playback: its stop flag, and its samples (kept here so a stopped clip's
/// buffer is never freed on the audio thread).
#[derive(Default)]
pub struct Trimming(Mutex<Option<(Arc<std::sync::atomic::AtomicBool>, Arc<[f32]>)>>);

impl Trimming {
    /// A new playback of `samples`: stops the last one and returns the new one's stop flag.
    pub fn start(&self, samples: Arc<[f32]>) -> Arc<std::sync::atomic::AtomicBool> {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((old, _)) = g.replace((stop.clone(), samples)) {
            old.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        stop
    }

    /// Pause: the current playback (if any) ends on its next pull.
    pub fn stop(&self) {
        let g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((stop, _)) = g.as_ref() {
            stop.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

/// The play-to-call clicks of this session, so a play can log click → queued.
#[derive(Default)]
pub struct Clicks(Mutex<Vec<(String, std::time::Instant)>>);

impl Clicks {
    /// How long a click is worth matching to its play.
    const WINDOW: std::time::Duration = std::time::Duration::from_secs(10);

    pub fn clicked(&self, sound_id: &str) {
        let mut c = self.0.lock().unwrap_or_else(|e| e.into_inner());
        c.retain(|(_, at)| at.elapsed() < Self::WINDOW);
        c.push((sound_id.to_string(), std::time::Instant::now()));
    }

    /// The oldest recent click on `sound_id`, taken.
    pub fn take(&self, sound_id: &str) -> Option<std::time::Instant> {
        let mut c = self.0.lock().unwrap_or_else(|e| e.into_inner());
        c.retain(|(_, at)| at.elapsed() < Self::WINDOW);
        let i = c.iter().position(|(id, _)| id == sound_id)?;
        Some(c.remove(i).1)
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
    fn gain_combines_calls_soundboard_and_the_sound() {
        assert_eq!(gain(100, 0, 100), 0.0);
        assert_eq!(gain(0, 100, 100), 0.0, "Calls master at 0 silences it");
        assert_eq!(gain(100, 100, 100), 1.0);
        assert_eq!(
            gain(100, 200, 100),
            percent_to_gain(200),
            "the notch goes to 200 %"
        );
        assert_eq!(gain(50, 100, 100), percent_to_gain(50));
        assert_eq!(
            gain(100, 100, 0),
            percent_to_gain(10),
            "a sound is never below 10 %"
        );
        assert_eq!(gain(100, 100, 500), percent_to_gain(200));
        assert_eq!(
            gain(200, 200, 200),
            crate::voice::mixer::max_gain(),
            "capped"
        );
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

    #[tokio::test]
    async fn cache_refuses_garbage() {
        let c = Cache::default();
        let r = c.load("x", || async { Ok(b"nope".to_vec()) }).await;
        assert!(r.is_err());
        assert!(c.get("x").is_none());
    }

    #[test]
    fn trimming_replaces_and_stops() {
        let t = Trimming::default();
        let one: Arc<[f32]> = Arc::from(vec![0.0f32; 4]);
        let a = t.start(one.clone());
        assert!(!a.load(SeqCst));
        let b = t.start(one.clone());
        assert!(a.load(SeqCst), "a new play stops the last");
        assert!(!b.load(SeqCst));
        t.stop();
        assert!(b.load(SeqCst));
        t.stop(); // nothing playing: fine
    }

    #[test]
    fn clicks_match_once() {
        let c = Clicks::default();
        assert!(c.take("a").is_none());
        c.clicked("a");
        assert!(c.take("b").is_none());
        assert!(c.take("a").is_some());
        assert!(c.take("a").is_none(), "taken");
    }

    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

    /// A fetch that counts its calls and takes `ms` to answer with `bytes`.
    fn counting(
        calls: &Arc<AtomicUsize>,
        bytes: Option<Vec<u8>>,
        ms: u64,
    ) -> impl Fn(String) -> std::pin::Pin<Box<dyn Future<Output = Result<Vec<u8>, String>> + Send>> + Clone
    {
        let calls = calls.clone();
        move |_hash: String| {
            calls.fetch_add(1, SeqCst);
            let bytes = bytes.clone();
            Box::pin(async move {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                bytes.ok_or_else(|| "offline".to_string())
            })
        }
    }

    #[tokio::test]
    async fn two_loads_of_one_sound_fetch_once() {
        let Some(ogg) = decode::tests::sine_ogg_secs(1) else {
            return;
        };
        let c = Cache::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let f = counting(&calls, Some(ogg), 50);
        let (a, b) = tokio::join!(c.load("h", || f("h".into())), c.load("h", || f("h".into())));
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(calls.load(SeqCst), 1, "the second waited for the first");
        assert!(Arc::ptr_eq(&a.0, &b.0));
        assert!(a.1.cached != b.1.cached, "one fetched, one reused");
    }

    #[tokio::test]
    async fn load_reports_cached_on_the_second_play() {
        let Some(ogg) = decode::tests::sine_ogg_secs(1) else {
            return;
        };
        let c = Cache::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let f = counting(&calls, Some(ogg), 0);
        let (_, first) = c.load("h", || f("h".into())).await.unwrap();
        assert!(!first.cached);
        let (_, second) = c.load("h", || f("h".into())).await.unwrap();
        assert!(second.cached);
        assert_eq!(calls.load(SeqCst), 1);
    }

    #[tokio::test]
    async fn warm_fetches_what_isnt_decoded_and_evicts_nothing() {
        let c = Cache::default();
        let one = Arc::new(Pcm::complete(&[0.0]));
        for i in 0..CACHE_ENTRIES {
            c.put(&format!("played{i}"), one.clone());
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let f = counting(&calls, Some(vec![1, 2, 3]), 0);
        c.warm(&["a".into(), "b".into(), "played3".into()], f).await;
        assert_eq!(calls.load(SeqCst), 2, "a decoded sound isn't fetched again");
        assert!(c.get("a").is_none(), "fetched (disk cache), not decoded");
        for i in 0..CACHE_ENTRIES {
            assert!(
                c.get(&format!("played{i}")).is_some(),
                "warm-up evicts nothing"
            );
        }
        let many: Vec<String> = (0..WARM_MAX + 50).map(|i| format!("m{i}")).collect();
        calls.store(0, SeqCst);
        c.warm(&many, counting(&calls, Some(vec![1]), 0)).await;
        assert_eq!(calls.load(SeqCst), WARM_MAX, "bounded");
    }

    #[tokio::test]
    async fn warm_skips_failures() {
        let Some(ogg) = decode::tests::sine_ogg_secs(1) else {
            return;
        };
        let c = Cache::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let bad = counting(&calls, None, 0);
        let good = counting(&calls, Some(ogg), 0);
        let f = move |h: String| if h == "bad" { bad(h) } else { good(h) };
        c.warm(&["bad".into(), "good".into()], f).await;
        assert_eq!(
            calls.load(SeqCst),
            2,
            "the failure didn't stop the next one"
        );
    }

    /// The delay this round set out to remove: a first play waits on fetch + decode, a warmed or
    /// replayed one on neither. Prints the numbers (`--nocapture`).
    #[tokio::test]
    async fn first_play_vs_cached_timing() {
        let Some(ogg) = decode::tests::sine_ogg_secs(6) else {
            return;
        };
        let c = Cache::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let f = counting(&calls, Some(ogg.clone()), 0);
        let t = std::time::Instant::now();
        let (_, first) = c.load("h", || f("h".into())).await.unwrap();
        let first_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = std::time::Instant::now();
        let (_, again) = c.load("h", || f("h".into())).await.unwrap();
        let again_ms = t.elapsed().as_secs_f64() * 1000.0;
        // what a first play waited for before streaming: the whole decode
        let t = std::time::Instant::now();
        decode::decode(&ogg).unwrap();
        let whole_ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!(
            "soundboard timing (6 s sound, fetch excluded): first play ready after {first_ms:.2} ms \
             (streamed; decoding all of it first takes {whole_ms:.2} ms), cached {again_ms:.3} ms"
        );
        assert!(
            first.start_ms < whole_ms / 4.0,
            "a first play no longer waits for the whole decode"
        );
        assert!(again.cached);
        assert!(
            again_ms < 1.0,
            "a cached play is ready at once ({again_ms} ms)"
        );
    }

    #[test]
    fn cache_forgets_the_least_recently_used() {
        let c = Cache::default();
        let one = Arc::new(Pcm::complete(&[0.0]));
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
