//! A decoded sound that can be played while it is still being decoded.
//!
//! `opus-decoder` (pure Rust) decodes a 6 s sound in ~0.8 s even optimised, so waiting for the whole
//! file made a first play that late (#138). A [`Pcm`] is sized up front from the file's stated
//! length; one decoder thread fills it in order and publishes how far it got, and the audio
//! callback plays up to there. Samples are stored as `f32` bits in atomics, so the writer and the
//! reader share it without locks or `unsafe`.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

pub struct Pcm {
    data: Box<[AtomicU32]>,
    /// Samples written so far (Release by the writer, Acquire by readers).
    ready: AtomicUsize,
    /// No more samples will come; `ready` is the final length.
    done: AtomicBool,
}

impl Pcm {
    /// Room for `capacity` samples (the file's stated length); nothing ready yet.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: (0..capacity).map(|_| AtomicU32::new(0)).collect(),
            ready: AtomicUsize::new(0),
            done: AtomicBool::new(false),
        }
    }

    /// A finished one holding `samples`.
    pub fn complete(samples: &[f32]) -> Self {
        let p = Self::with_capacity(samples.len());
        p.push(samples);
        p.finish();
        p
    }

    /// Append (one writer only). Anything past the capacity is dropped.
    pub fn push(&self, samples: &[f32]) {
        let at = self.ready.load(Ordering::Relaxed);
        let room = self.data.len().saturating_sub(at);
        let n = samples.len().min(room);
        for (slot, v) in self.data[at..at + n].iter().zip(samples) {
            slot.store(v.to_bits(), Ordering::Relaxed);
        }
        self.ready.store(at + n, Ordering::Release);
    }

    /// No more samples are coming.
    pub fn finish(&self) {
        self.done.store(true, Ordering::Release);
    }

    pub fn ready(&self) -> usize {
        self.ready.load(Ordering::Acquire)
    }

    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }

    pub fn is_full(&self) -> bool {
        self.ready() >= self.data.len()
    }

    /// Sample `i`, if it has been written.
    pub fn get(&self, i: usize) -> Option<f32> {
        (i < self.ready()).then(|| f32::from_bits(self.data[i].load(Ordering::Relaxed)))
    }

    /// Everything written so far.
    pub fn to_vec(&self) -> Vec<f32> {
        let n = self.ready();
        self.data[..n]
            .iter()
            .map(|v| f32::from_bits(v.load(Ordering::Relaxed)))
            .collect()
    }

    /// Wait (async) until at least `n` samples are ready or the decode has ended.
    pub async fn wait_for(&self, n: usize) {
        while self.ready() < n && !self.is_done() {
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
    }

    /// Wait (async) for the decode to end.
    pub async fn wait_done(&self) {
        while !self.is_done() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_what_is_ready() {
        let p = Pcm::with_capacity(4);
        assert_eq!((p.ready(), p.get(0)), (0, None));
        p.push(&[0.5, -0.25]);
        assert_eq!(
            (p.get(0), p.get(1), p.get(2)),
            (Some(0.5), Some(-0.25), None)
        );
        p.push(&[1.0, 2.0, 3.0]); // one too many: dropped
        assert!(p.is_full() && !p.is_done());
        p.finish();
        assert!(p.is_done());
        assert_eq!(p.to_vec(), vec![0.5, -0.25, 1.0, 2.0]);
    }

    #[test]
    fn a_writer_thread_and_a_reader_agree() {
        let p = std::sync::Arc::new(Pcm::with_capacity(48_000));
        let w = p.clone();
        let t = std::thread::spawn(move || {
            for i in 0..100 {
                let chunk: Vec<f32> = (0..480).map(|j| (i * 480 + j) as f32).collect();
                w.push(&chunk);
            }
            w.finish();
        });
        let mut i = 0;
        while i < 48_000 {
            if let Some(v) = p.get(i) {
                assert_eq!(v, i as f32);
                i += 1;
            }
        }
        t.join().unwrap();
    }
}
