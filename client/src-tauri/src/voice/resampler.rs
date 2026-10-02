//! Streaming mono resampler between a device rate and `INTERNAL_RATE`.

use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

use super::devices::INTERNAL_RATE;

/// Band-limited (FFT/sinc) resampling: keeps the highs that make speech crisp, and carries its
/// state across calls so block edges are seamless. Input is consumed in fixed 10 ms chunks.
pub struct StreamResampler {
    rs: Option<Fft<f32>>,
    chunk: usize,
    pending: Vec<f32>,
    buf: Vec<f32>,
}

impl StreamResampler {
    pub fn new(from: u32, to: u32) -> Self {
        let chunk = (from / 100) as usize;
        let rs = (from != to).then(|| {
            Fft::<f32>::new(from as usize, to as usize, chunk, 1, FixedSync::Input)
                .expect("valid resampler rates")
        });
        let buf = vec![0.0; rs.as_ref().map_or(0, |r| r.output_frames_max())];
        Self {
            rs,
            chunk,
            pending: Vec::with_capacity(chunk),
            buf,
        }
    }

    /// Feed any number of input samples; appends whatever output is ready to `out`.
    pub fn push(&mut self, mut input: &[f32], out: &mut impl Extend<f32>) {
        let Some(rs) = self.rs.as_mut() else {
            out.extend(input.iter().copied());
            return;
        };
        while !input.is_empty() {
            let take = (self.chunk - self.pending.len()).min(input.len());
            self.pending.extend_from_slice(&input[..take]);
            input = &input[take..];
            if self.pending.len() < self.chunk {
                break;
            }
            let cap = self.buf.len();
            let inp = InterleavedSlice::new(&self.pending, 1, self.chunk).expect("chunk sized");
            let mut outp = InterleavedSlice::new_mut(&mut self.buf, 1, cap).expect("buf sized");
            let (_, written) = rs
                .process_into_buffer(&inp, &mut outp, None)
                .expect("sized buffers");
            out.extend(self.buf[..written].iter().copied());
            self.pending.clear();
        }
    }
}

/// Mic chunks (i16 at their device's rate) → `INTERNAL_RATE`, rebuilt when the device rate changes.
#[derive(Default)]
pub struct ToInternal {
    rate: u32,
    rs: Option<StreamResampler>,
    out: Vec<f32>,
}

impl ToInternal {
    pub fn process(&mut self, rate: u32, chunk: &[i16]) -> Vec<i16> {
        if rate == INTERNAL_RATE {
            self.rs = None;
            return chunk.to_vec();
        }
        if self.rs.is_none() || self.rate != rate {
            self.rate = rate;
            self.rs = Some(StreamResampler::new(rate, INTERNAL_RATE));
        }
        let input: Vec<f32> = chunk.iter().map(|s| *s as f32 / i16::MAX as f32).collect();
        self.out.clear();
        if let Some(rs) = self.rs.as_mut() {
            rs.push(&input, &mut self.out);
        }
        self.out
            .iter()
            .map(|v| {
                (v * i16::MAX as f32)
                    .round()
                    .clamp(i16::MIN as f32, i16::MAX as f32) as i16
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn tone(freq: f32, rate: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| 0.5 * (TAU * freq * i as f32 / rate as f32).sin())
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    /// Push `input` in 10 ms blocks; return everything that came out.
    fn run(from: u32, to: u32, input: &[f32]) -> Vec<f32> {
        let mut r = StreamResampler::new(from, to);
        let mut out = Vec::new();
        for block in input.chunks(from as usize / 100) {
            r.push(block, &mut out);
        }
        out
    }

    /// Sibilants ("s", "t") live up to ~16 kHz and are what makes voice sound crisp. A crude
    /// resampler dulls them; a proper one keeps their level.
    #[test]
    fn keeps_high_frequencies_both_ways() {
        for (from, to) in [(44_100, 48_000), (48_000, 44_100)] {
            let input = tone(15_000.0, from, from as usize); // 1 s
            let out = run(from, to, &input);
            let steady = &out[out.len() / 4..out.len() * 3 / 4];
            let loss_db = 20.0 * (rms(steady) / rms(&input)).log10();
            assert!(
                loss_db.abs() < 0.5,
                "{from}->{to}: 15 kHz changed by {loss_db:.2} dB"
            );
        }
    }

    /// Mic chunks carry their device's rate, which changes when the user switches devices.
    #[test]
    fn to_internal_follows_device_rate_changes() {
        let mut m = ToInternal::default();
        let mut n = 0;
        for _ in 0..100 {
            n += m.process(44_100, &[1000; 441]).len(); // 1 s at 44.1 kHz
        }
        assert!((47_000..=48_000).contains(&n), "44.1 kHz second gave {n}");
        let native = m.process(48_000, &[7; 480]);
        assert_eq!(native, vec![7; 480], "48 kHz passes through untouched");
    }

    /// Voice is latency-sensitive: a click must come out within 25 ms (our 10 ms chunking included).
    #[test]
    fn adds_little_latency() {
        for (from, to) in [(44_100u32, 48_000u32), (48_000, 44_100)] {
            let mut input = vec![0.0f32; from as usize / 10];
            input[0] = 1.0;
            let out = run(from, to, &input);
            let peak = (0..out.len())
                .max_by(|&a, &b| out[a].abs().total_cmp(&out[b].abs()))
                .unwrap();
            let ms = peak as f32 * 1000.0 / to as f32;
            println!("{from}->{to}: click delayed {ms:.1} ms");
            assert!(ms <= 25.0, "{from}->{to}: click delayed {ms:.1} ms");
        }
    }

    /// Over time, output length must track the rate ratio exactly (no drift, no lost samples).
    #[test]
    fn output_length_tracks_the_ratio() {
        for (from, to) in [(44_100, 48_000), (48_000, 44_100)] {
            let input = tone(440.0, from, from as usize * 10);
            let out = run(from, to, &input);
            let expected = input.len() as i64 * to as i64 / from as i64;
            let diff = out.len() as i64 - expected;
            assert!(
                diff.abs() <= (to / 50) as i64,
                "{from}->{to}: {} out vs {expected} expected",
                out.len()
            );
        }
    }
}
