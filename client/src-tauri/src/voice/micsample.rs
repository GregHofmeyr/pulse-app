//! "Record a mic sample" (#583): about 15 s of the mic at two taps, raw (device + downmix at 48 kHz, before APM) and
//! sent (after the gate, what listeners get), saved as two 48 kHz mono 16-bit WAVs in the log folder. Listening to
//! both says whether damage comes from the device or from our chain.
//!
//! The mic processor taps every block, in a call and in the mic test alike (both run the same processor). Its side
//! never blocks or allocates: the buffers are allocated when a recording starts, the processor only `try_lock`s them
//! (the other taker is the command, at start and finish) and writes within their capacity.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use super::devices::INTERNAL_RATE;

pub const SECONDS: usize = 15;
pub const SAMPLES: usize = SECONDS * INTERNAL_RATE as usize;
pub const RAW_FILE: &str = "mic-sample-raw.wav";
pub const SENT_FILE: &str = "mic-sample-sent.wav";

/// The two taps, sample-aligned (each block lands in both at once).
pub struct Takes {
    pub raw: Vec<i16>,
    pub sent: Vec<i16>,
}

pub struct Recorder {
    armed: AtomicBool,
    takes: Mutex<Option<Takes>>,
    /// The last saved recording (unix seconds), for the bug report's details.
    last: Mutex<Option<u64>>,
}

/// The one recorder: whichever processor runs (call or mic test) feeds it.
pub static RECORDER: Recorder = Recorder::new();

fn to_i16(v: f32) -> i16 {
    (v * 32767.0).round().clamp(-32768.0, 32767.0) as i16
}

impl Recorder {
    pub const fn new() -> Self {
        Self {
            armed: AtomicBool::new(false),
            takes: Mutex::new(None),
            last: Mutex::new(None),
        }
    }

    /// Whether the processor should copy its raw block for [`Self::tap`] (one relaxed load per block).
    pub fn armed(&self) -> bool {
        self.armed.load(Ordering::Acquire)
    }

    /// Start a recording of `samples` per tap, its buffers allocated here. `false` if one is already running.
    pub fn start(&self, samples: usize) -> bool {
        let mut t = self.takes.lock().unwrap_or_else(|e| e.into_inner());
        if self.armed.load(Ordering::Acquire) {
            return false;
        }
        *t = Some(Takes {
            raw: Vec::with_capacity(samples),
            sent: Vec::with_capacity(samples),
        });
        self.armed.store(true, Ordering::Release);
        true
    }

    /// One block at both taps (processor thread). Never blocks: if the command holds the buffers right now the block
    /// is skipped at both taps, so they stay aligned. Never allocates: writes stop at the buffers' capacity, and the
    /// recording disarms itself when full.
    pub fn tap(&self, raw: &[f32], sent: &[i16]) {
        if !self.armed() {
            return;
        }
        let Ok(mut g) = self.takes.try_lock() else {
            return;
        };
        let Some(t) = g.as_mut() else {
            return;
        };
        let room = t.raw.capacity() - t.raw.len();
        let n = raw.len().min(sent.len()).min(room);
        t.raw.extend(raw[..n].iter().map(|v| to_i16(*v)));
        t.sent.extend_from_slice(&sent[..n]);
        if t.raw.len() == t.raw.capacity() {
            self.armed.store(false, Ordering::Release);
        }
    }

    /// Samples per tap so far.
    pub fn recorded(&self) -> usize {
        self.takes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map_or(0, |t| t.raw.len())
    }

    /// Whether the recording filled up.
    pub fn full(&self) -> bool {
        !self.armed() && self.recorded() > 0
    }

    /// Stop and hand over what was recorded.
    pub fn finish(&self) -> Option<Takes> {
        let mut g = self.takes.lock().unwrap_or_else(|e| e.into_inner());
        self.armed.store(false, Ordering::Release);
        g.take()
    }

    pub fn saved_at(&self, unix: u64) {
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(unix);
    }

    /// For the bug report: which files were saved and when (`None`: no sample this session).
    pub fn report_meta(&self) -> Option<serde_json::Value> {
        let at = (*self.last.lock().unwrap_or_else(|e| e.into_inner()))?;
        Some(serde_json::json!({ "files": [RAW_FILE, SENT_FILE], "saved_unix": at }))
    }
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

/// A 48 kHz (or `rate`) mono 16-bit PCM WAV.
pub fn wav_bytes(samples: &[i16], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes()); // bytes per second
    b.extend_from_slice(&2u16.to_le_bytes()); // block align
    b.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

/// Peak of a take in dBFS, for the log line.
pub fn peak_dbfs(samples: &[i16]) -> f64 {
    let peak = samples.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
    super::miclevels::dbfs(peak as f32 / 32768.0)
}

/// Write both takes into `dir` (replacing the last pair). Returns (raw, sent) paths.
pub fn save(dir: &Path, takes: &Takes) -> std::io::Result<(PathBuf, PathBuf)> {
    std::fs::create_dir_all(dir)?;
    crate::reports::write_atomic(dir, RAW_FILE, &wav_bytes(&takes.raw, INTERNAL_RATE))?;
    crate::reports::write_atomic(dir, SENT_FILE, &wav_bytes(&takes.sent, INTERNAL_RATE))?;
    Ok((dir.join(RAW_FILE), dir.join(SENT_FILE)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_both_taps_aligned_until_full_then_stops() {
        let r = Recorder::new();
        assert!(!r.armed());
        r.tap(&[0.5; 4], &[1; 4]); // not armed: ignored
        assert!(r.start(10));
        assert!(!r.start(10), "one at a time");
        let cap = |r: &Recorder| {
            let g = r.takes.lock().unwrap();
            let t = g.as_ref().unwrap();
            (t.raw.capacity(), t.sent.capacity())
        };
        let before = cap(&r);
        for _ in 0..3 {
            r.tap(&[0.5; 4], &[7; 4]);
        }
        assert_eq!(
            cap(&r),
            before,
            "never grows (no allocation on the processor)"
        );
        assert!(r.full());
        r.tap(&[0.5; 4], &[7; 4]); // full: ignored
        let t = r.finish().unwrap();
        assert_eq!(t.raw.len(), 10);
        assert_eq!(t.sent, vec![7; 10]);
        assert_eq!(t.raw[0], 16384);
        assert!(r.start(10), "a new recording can start once finished");
        r.finish();
    }

    #[test]
    fn a_held_buffer_skips_the_block_at_both_taps() {
        let r = Recorder::new();
        r.start(100);
        {
            let _held = r.takes.lock().unwrap();
            r.tap(&[0.1; 4], &[1; 4]);
        }
        r.tap(&[0.1; 4], &[1; 4]);
        let t = r.finish().unwrap();
        assert_eq!((t.raw.len(), t.sent.len()), (4, 4));
    }

    #[test]
    fn a_wav_has_the_standard_header() {
        let b = wav_bytes(&[0, 1, -1], 48_000);
        assert_eq!(b.len(), 44 + 6);
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 36 + 6);
        assert_eq!(&b[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([b[22], b[23]]), 1, "mono");
        assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), 48_000);
        assert_eq!(u16::from_le_bytes([b[34], b[35]]), 16, "16-bit");
        assert_eq!(&b[36..40], b"data");
        assert_eq!(&b[44..], &[0, 0, 1, 0, 0xff, 0xff]);
    }

    #[test]
    fn saving_writes_both_files_and_replaces_the_last_pair() {
        let dir = tempfile::tempdir().unwrap();
        let takes = Takes {
            raw: vec![1; 480],
            sent: vec![2; 480],
        };
        let (raw, sent) = save(dir.path(), &takes).unwrap();
        save(dir.path(), &takes).unwrap();
        assert_eq!(std::fs::read(&raw).unwrap().len(), 44 + 960);
        assert_eq!(std::fs::read(&sent).unwrap()[44..46], [2, 0]);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
