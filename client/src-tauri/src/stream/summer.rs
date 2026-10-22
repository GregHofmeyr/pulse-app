//! Sums the 10 ms stereo chunks of several captures into one stream. Pure.

use std::collections::{HashMap, VecDeque};

use super::plan::Capture;

/// 10 ms of stereo at 48 kHz: 480 frames x 2 channels.
pub const CHUNK_SAMPLES: usize = 960;
const SAMPLES_PER_MS: usize = 96;

pub struct Summer {
    lag_cap: usize,
    bufs: HashMap<Capture, VecDeque<i16>>,
}

impl Summer {
    /// `lag_cap_ms`: how far a capture may trail the fullest one before it is zero-filled.
    pub fn new(lag_cap_ms: u32) -> Self {
        Self {
            lag_cap: lag_cap_ms as usize * SAMPLES_PER_MS,
            bufs: HashMap::new(),
        }
    }

    pub fn push(&mut self, id: Capture, chunk: &[i16]) {
        self.bufs
            .entry(id)
            .or_default()
            .extend(chunk.iter().copied());
    }

    pub fn remove(&mut self, id: Capture) {
        self.bufs.remove(&id);
    }

    /// The next summed chunk, once the fullest capture has one. A capture with less than a chunk
    /// is waited for until it trails by more than the lag cap, then contributes zeros for the gap.
    pub fn pop(&mut self) -> Option<Vec<i16>> {
        let fullest = self.bufs.values().map(VecDeque::len).max()?;
        if fullest < CHUNK_SAMPLES {
            return None;
        }
        if self
            .bufs
            .values()
            .any(|b| b.len() < CHUNK_SAMPLES && fullest - b.len() <= self.lag_cap)
        {
            return None;
        }
        let mut out = vec![0i16; CHUNK_SAMPLES];
        for buf in self.bufs.values_mut() {
            for (o, s) in out
                .iter_mut()
                .zip(buf.drain(..buf.len().min(CHUNK_SAMPLES)))
            {
                *o = o.saturating_add(s);
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Capture::*;

    fn chunk(v: i16) -> Vec<i16> {
        vec![v; CHUNK_SAMPLES]
    }

    #[test]
    fn single_capture_passes_through_unchanged() {
        let mut s = Summer::new(30);
        let c: Vec<i16> = (0..CHUNK_SAMPLES as i16).collect();
        s.push(Include(1), &c);
        assert_eq!(s.pop(), Some(c));
        assert_eq!(s.pop(), None);
    }

    #[test]
    fn two_captures_sum_and_saturate() {
        let mut s = Summer::new(30);
        s.push(Include(1), &chunk(100));
        assert_eq!(s.pop(), Some(chunk(100)));
        s.push(Include(1), &chunk(100));
        s.push(Include(2), &chunk(-20));
        assert_eq!(s.pop(), Some(chunk(80)));
        s.push(Include(1), &chunk(30000));
        s.push(Include(2), &chunk(30000));
        assert_eq!(s.pop(), Some(chunk(i16::MAX)));
        s.push(Include(1), &chunk(-30000));
        s.push(Include(2), &chunk(-30000));
        assert_eq!(s.pop(), Some(chunk(i16::MIN)));
    }

    #[test]
    fn waits_for_a_capture_that_is_only_slightly_behind() {
        let mut s = Summer::new(30);
        s.push(Include(1), &chunk(1));
        s.push(Include(1), &chunk(1));
        s.push(Include(2), &chunk(1));
        assert_eq!(s.pop(), Some(chunk(2)));
        // Capture 2 is 10 ms behind, inside the 30 ms cap: hold the chunk.
        assert_eq!(s.pop(), None);
    }

    #[test]
    fn lagging_capture_is_zero_filled_after_cap() {
        let mut s = Summer::new(30);
        for _ in 0..5 {
            s.push(Include(1), &chunk(5));
        }
        s.push(Include(2), &chunk(7));
        assert_eq!(s.pop(), Some(chunk(12)));
        // Capture 2 now trails by 40 ms, past the cap: capture 1 flows alone.
        assert_eq!(s.pop(), Some(chunk(5)));
        // Capture 2 catches up: it joins the sum again.
        s.push(Include(2), &chunk(3));
        assert_eq!(s.pop(), Some(chunk(8)));
    }

    #[test]
    fn removed_capture_stops_contributing() {
        let mut s = Summer::new(30);
        s.push(Include(1), &chunk(5));
        s.push(Include(2), &chunk(7));
        assert_eq!(s.pop(), Some(chunk(12)));
        s.remove(Include(2));
        s.push(Include(1), &chunk(5));
        assert_eq!(s.pop(), Some(chunk(5)));
    }
}
