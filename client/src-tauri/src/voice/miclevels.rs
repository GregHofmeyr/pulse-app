//! Mic levels and clipping, stage by stage, for the 5 s `voice: mic` line (#583): what the device hands over (peak per
//! channel, L/R correlation, samples at full scale), what the denoiser leaves, how often the gain drives the soft
//! limiter (`limiter.rs`) turns the voice down and by how much, and the peak of what is sent. A crackly mic can then be pinned to the stage that adds it.
//!
//! The audio side only does per-block arithmetic into atomics: no locks, no allocation (the input half runs in the
//! device callback).

use std::fmt;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering::Relaxed};

/// −0.1 dBFS: a device sample at or above this is clipping (or one step from it).
pub const CLIP: f32 = 0.988_553_1;
/// Sums of squares are stored as integers ×1e9 so the audio side stays lock-free (as `stats::Pipeline` does).
const SCALE: f64 = 1e9;
/// Quieter than 16-bit silence reads as this, not −∞.
const FLOOR_DB: f64 = -96.0;
/// L/R correlation is only meaningful with some signal on both sides (a sum of squares of 1e-4 is ≈ −87 dBFS over a
/// 5 s window).
const CORR_MIN_ENERGY: i64 = 100_000;

/// One device callback's input, summed before it touches the shared counters.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InputScan {
    /// Peak of the first two channels (a mono device fills only `[0]`).
    pub peak: [f32; 2],
    /// Samples, on any channel, at or above [`CLIP`].
    pub clipped: u64,
    /// Σl², Σr², Σlr over the first two channels (stereo and up only).
    pub ll: f64,
    pub rr: f64,
    pub lr: f64,
}

/// Scan an interleaved device buffer: per-channel peaks, clipped samples and the L/R sums.
pub fn scan_input(data: &[f32], channels: usize) -> InputScan {
    let ch = channels.max(1);
    let mut s = InputScan::default();
    for f in data.chunks(ch) {
        for v in f {
            if v.abs() >= CLIP {
                s.clipped += 1;
            }
        }
        let l = f[0];
        s.peak[0] = s.peak[0].max(l.abs());
        if let Some(&r) = f.get(1) {
            s.peak[1] = s.peak[1].max(r.abs());
            let (l, r) = (l as f64, r as f64);
            s.ll += l * l;
            s.rr += r * r;
            s.lr += l * r;
        }
    }
    s
}

/// A peak kept as its f32 bits: for non-negative floats the bit patterns order like the values, so `fetch_max` works.
#[derive(Default)]
struct Peak(AtomicU32);

impl Peak {
    fn record(&self, v: f32) {
        if v > 0.0 {
            self.0.fetch_max(v.to_bits(), Relaxed);
        }
    }

    fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Relaxed))
    }
}

#[derive(Default)]
pub struct MicLevels {
    /// Channels the device last delivered (0 = no input this window).
    channels: AtomicU32,
    in_peak: [Peak; 2],
    in_clipped: AtomicU64,
    in_ll: AtomicI64,
    in_rr: AtomicI64,
    in_lr: AtomicI64,
    /// After the denoiser (before gain): peak, Σx² and samples. Mic-open blocks only.
    ns_peak: Peak,
    ns_energy: AtomicU64,
    ns_samples: AtomicU64,
    /// After gain: samples the limiter turned down (its gain below 1), of how many, and its deepest cut as
    /// `1 − gain` (kept as a peak). Mic-open blocks only.
    limited: AtomicU64,
    cut: Peak,
    gained: AtomicU64,
    /// What went out (after the gate). Mic-open blocks only.
    out_peak: Peak,
}

impl MicLevels {
    /// Input half: one device callback's buffer. Real-time safe (arithmetic and atomics only).
    pub fn record_input(&self, data: &[f32], channels: usize) {
        let s = scan_input(data, channels);
        self.channels.store(channels.max(1) as u32, Relaxed);
        self.in_peak[0].record(s.peak[0]);
        self.in_peak[1].record(s.peak[1]);
        if s.clipped > 0 {
            self.in_clipped.fetch_add(s.clipped, Relaxed);
        }
        if channels >= 2 {
            self.in_ll.fetch_add((s.ll * SCALE) as i64, Relaxed);
            self.in_rr.fetch_add((s.rr * SCALE) as i64, Relaxed);
            self.in_lr.fetch_add((s.lr * SCALE) as i64, Relaxed);
        }
    }

    /// Processor half: one mic-open 10 ms block's figures from the chain (see `processor::BlockOut`).
    pub fn record_chain(
        &self,
        ns_peak: f32,
        ns_energy: f32,
        samples: usize,
        limited: u32,
        limiter_min_gain: f32,
    ) {
        self.cut.record(1.0 - limiter_min_gain);
        self.ns_peak.record(ns_peak);
        self.ns_energy
            .fetch_add((ns_energy as f64 * SCALE) as u64, Relaxed);
        self.ns_samples.fetch_add(samples as u64, Relaxed);
        self.limited.fetch_add(limited as u64, Relaxed);
        self.gained.fetch_add(samples as u64, Relaxed);
    }

    /// The peak of a block that went out (silence included: a closed gate sends zeros).
    pub fn record_out(&self, frame: &[i16]) {
        let peak = frame.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
        self.out_peak.record(peak as f32 / 32768.0);
    }

    /// Everything since the last take; counters restart. As in `stats::Pipeline`, a block recorded mid-take may land
    /// half in each window: fine for a diagnostic.
    pub fn take(&self) -> MicSummary {
        let channels = self.channels.swap(0, Relaxed);
        let (ll, rr, lr) = (
            self.in_ll.swap(0, Relaxed),
            self.in_rr.swap(0, Relaxed),
            self.in_lr.swap(0, Relaxed),
        );
        let (ns_energy, ns_samples) = (
            self.ns_energy.swap(0, Relaxed),
            self.ns_samples.swap(0, Relaxed),
        );
        let (limited, gained) = (self.limited.swap(0, Relaxed), self.gained.swap(0, Relaxed));
        let ns_peak = self.ns_peak.take();
        let cut = self.cut.take();
        let out_peak = self.out_peak.take();
        let [l, r] = [self.in_peak[0].take(), self.in_peak[1].take()];
        MicSummary {
            channels,
            in_peak: (channels > 0).then_some([l, r]),
            in_clipped: self.in_clipped.swap(0, Relaxed),
            corr: (channels >= 2 && ll >= CORR_MIN_ENERGY && rr >= CORR_MIN_ENERGY)
                .then(|| lr as f64 / ((ll as f64) * (rr as f64)).sqrt()),
            ns: (ns_samples > 0).then(|| {
                let rms = ((ns_energy as f64 / SCALE) / ns_samples as f64).sqrt();
                (ns_peak, rms as f32)
            }),
            limited_pct: (gained > 0).then(|| limited as f64 * 100.0 / gained as f64),
            limiter_max_db: (gained > 0).then(|| {
                let db = dbfs((1.0 - cut).max(0.0));
                if db == 0.0 { 0.0 } else { db } // no −0.0
            }),
            out_peak: (gained > 0).then_some(out_peak),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct MicSummary {
    /// The device's channel count (0: no input this window).
    pub channels: u32,
    /// Raw input peak, per channel (L, R; a mono device's is `[peak, 0]`), linear.
    pub in_peak: Option<[f32; 2]>,
    /// Input samples at or above −0.1 dBFS.
    pub in_clipped: u64,
    /// L/R correlation, −1 to 1 (stereo devices with signal on both sides): near −1 is out-of-phase stereo, which
    /// the downmix cancels.
    pub corr: Option<f64>,
    /// (peak, RMS) after the denoiser, linear; mic-open time only.
    pub ns: Option<(f32, f32)>,
    /// Share of samples the limiter turned down (its gain below 1) after gain.
    pub limited_pct: Option<f64>,
    /// The limiter's deepest gain reduction, dB (0 = never engaged, negative = turned down).
    pub limiter_max_db: Option<f64>,
    /// Peak of what was sent, linear.
    pub out_peak: Option<f32>,
}

pub fn dbfs(v: f32) -> f64 {
    if v <= 0.0 {
        FLOOR_DB
    } else {
        (20.0 * (v as f64).log10()).max(FLOOR_DB)
    }
}

impl fmt::Display for MicSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let db = |v: f32| format!("{:.1}", dbfs(v));
        match self.in_peak {
            None => write!(f, "in -")?,
            Some([p, _]) if self.channels < 2 => write!(f, "in pk {}dBFS", db(p))?,
            Some([l, r]) => {
                let corr = self.corr.map_or("-".into(), |c| format!("{c:.2}"));
                let ch = if self.channels > 2 {
                    format!("{}ch ", self.channels)
                } else {
                    String::new()
                };
                write!(f, "in {ch}pk L {} R {}dBFS corr {corr}", db(l), db(r))?
            }
        }
        write!(f, " clip {}", self.in_clipped)?;
        match self.ns {
            Some((pk, rms)) => write!(f, " ns pk {} rms {}dBFS", db(pk), db(rms))?,
            None => write!(f, " ns -")?,
        }
        match (self.limited_pct, self.limiter_max_db) {
            (Some(p), Some(db)) => write!(f, " limit {p:.1}% max {db:.1}dB")?,
            _ => write!(f, " limit -")?,
        }
        match self.out_peak {
            Some(p) => write!(f, " out pk {}dBFS", db(p)),
            None => write!(f, " out -"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stereo_buffer_scans_per_channel() {
        // L full scale (clipping), R at half, perfectly in phase with L's sign
        let data = [1.0, 0.5, -1.0, -0.5, 0.25, 0.125];
        let s = scan_input(&data, 2);
        assert_eq!(s.peak, [1.0, 0.5]);
        assert_eq!(s.clipped, 2);
        assert!((s.lr / (s.ll * s.rr).sqrt() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_mono_buffer_has_no_right_side() {
        let s = scan_input(&[0.1, -0.3, 0.99], 1);
        assert_eq!(s.peak, [0.99, 0.0]);
        assert_eq!(s.clipped, 1, "0.99 is above −0.1 dBFS");
        assert_eq!((s.ll, s.rr, s.lr), (0.0, 0.0, 0.0));
    }

    #[test]
    fn out_of_phase_stereo_reads_as_negative_correlation() {
        let m = MicLevels::default();
        let data: Vec<f32> = (0..960)
            .flat_map(|i| {
                let v = (i as f32 * 0.05).sin() * 0.5;
                [v, -v]
            })
            .collect();
        m.record_input(&data, 2);
        let s = m.take();
        assert!(s.corr.unwrap() < -0.99, "{s:?}");
        assert!(
            s.to_string()
                .starts_with("in pk L -6.0 R -6.0dBFS corr -1.00 clip 0"),
            "{s}"
        );
    }

    #[test]
    fn a_window_summarises_every_stage_then_starts_over() {
        let m = MicLevels::default();
        m.record_input(&[0.5, -0.25, 1.0], 1);
        // 480 samples after NS at RMS 0.1 (Σx² = 4.8), peak 0.2; the limiter turned 48 down, at most to half (−6 dB)
        m.record_chain(0.2, 4.8, 480, 48, 0.5);
        m.record_out(&[0, 16384, -32768]);
        let s = m.take();
        assert_eq!(s.in_clipped, 1);
        assert_eq!(s.limited_pct, Some(10.0));
        assert_eq!(
            s.to_string(),
            "in pk 0.0dBFS clip 1 ns pk -14.0 rms -20.0dBFS limit 10.0% max -6.0dB out pk 0.0dBFS"
        );
        let next = m.take();
        assert_eq!(next.to_string(), "in - clip 0 ns - limit - out -");
    }

    #[test]
    fn a_multichannel_device_says_so_and_silence_reads_as_the_floor() {
        let m = MicLevels::default();
        m.record_input(&[0.0; 12], 6);
        m.record_chain(0.0, 0.0, 480, 0, 1.0);
        m.record_out(&[0; 480]);
        assert_eq!(
            m.take().to_string(),
            "in 6ch pk L -96.0 R -96.0dBFS corr - clip 0 ns pk -96.0 rms -96.0dBFS limit 0.0% max 0.0dB out pk -96.0dBFS"
        );
    }
}
