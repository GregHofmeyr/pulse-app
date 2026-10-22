//! THROWAWAY (S9 A/V sync): find beep onsets in received stream audio. 1 ms RMS windows; an onset is
//! a window above 0.1 after at least 200 ms below 0.02 (the test pattern beeps 100 ms every 2 s).

pub struct Detector {
    win: usize,
    buf: Vec<f32>,
    quiet_windows: usize,
    need_quiet: usize,
}

impl Detector {
    pub fn new(rate: u32) -> Self {
        let win = (rate / 1000) as usize;
        Self {
            win,
            buf: Vec::with_capacity(win),
            // start "after silence": a beep in the very first window counts
            quiet_windows: usize::MAX / 2,
            need_quiet: 200,
        }
    }

    /// Feed samples; returns the offsets (into THIS chunk) where a beep starts.
    pub fn feed(&mut self, x: &[f32]) -> Vec<usize> {
        let mut out = Vec::new();
        for (i, v) in x.iter().enumerate() {
            self.buf.push(*v);
            if self.buf.len() == self.win {
                let rms = (self.buf.iter().map(|s| s * s).sum::<f32>() / self.win as f32).sqrt();
                self.buf.clear();
                if rms > 0.1 {
                    if self.quiet_windows >= self.need_quiet {
                        // a window that straddles chunks reports offset 0 of this chunk
                        out.push((i + 1).saturating_sub(self.win));
                    }
                    self.quiet_windows = 0;
                } else if rms < 0.02 {
                    self.quiet_windows = self.quiet_windows.saturating_add(1);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (i as f32 * 2.0 * std::f32::consts::PI * 1000.0 / 48_000.0).sin())
            .collect()
    }

    #[test]
    fn finds_a_beep_after_silence_at_the_right_sample() {
        let mut d = Detector::new(48_000);
        let mut x = vec![0.0; 4800];
        x.extend(tone(2400, 0.5));
        let hits = d.feed(&x);
        assert_eq!(hits.len(), 1);
        assert!(
            (hits[0] as i64 - 4800).abs() <= 48,
            "within 1 ms: {}",
            hits[0]
        );
    }

    #[test]
    fn a_long_tone_is_one_onset_and_quiet_noise_is_none() {
        let mut d = Detector::new(48_000);
        assert!(d.feed(&vec![0.001; 48_000]).is_empty());
        assert_eq!(d.feed(&tone(48_000, 0.5)).len(), 1);
    }

    #[test]
    fn onsets_across_chunk_boundaries_count_once() {
        let mut d = Detector::new(48_000);
        let mut x = vec![0.0; 4800];
        x.extend(tone(4800, 0.5));
        let (a, b) = x.split_at(4900);
        assert_eq!(d.feed(a).len() + d.feed(b).len(), 1);
    }

    #[test]
    fn an_onset_window_straddling_two_chunks_reports_the_chunk_start() {
        let mut d = Detector::new(48_000);
        assert!(d.feed(&vec![0.0; 4790]).is_empty()); // leaves 38 samples in the open window
        assert_eq!(d.feed(&tone(4800, 0.5)), vec![0]);
    }
}
