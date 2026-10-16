//! Counters for our own audio path, drained into each `voice: path` line: what the device hands
//! over, how full our queues are, what gets dropped or made up, and what each stage costs. The
//! audio side only touches atomics (and one short lock in the 100 ms loop, never in a callback).

use std::fmt;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// Smallest and largest value since the last take.
pub struct MinMax {
    min: AtomicU64,
    max: AtomicU64,
}

impl Default for MinMax {
    fn default() -> Self {
        Self {
            min: AtomicU64::new(u64::MAX),
            max: AtomicU64::new(0),
        }
    }
}

impl MinMax {
    pub fn record(&self, v: u64) {
        self.min.fetch_min(v, Relaxed);
        self.max.fetch_max(v, Relaxed);
    }

    fn take(&self) -> Option<(u64, u64)> {
        let (min, max) = (self.min.swap(u64::MAX, Relaxed), self.max.swap(0, Relaxed));
        (min != u64::MAX).then_some((min, max))
    }
}

/// Microseconds spent in one stage, per 10 ms block.
#[derive(Default)]
pub struct StageTime {
    total_us: AtomicU64,
    blocks: AtomicU64,
}

impl StageTime {
    pub fn record(&self, d: std::time::Duration) {
        self.total_us.fetch_add(d.as_micros() as u64, Relaxed);
        self.blocks.fetch_add(1, Relaxed);
    }

    fn take_mean(&self) -> Option<u64> {
        let (us, n) = (self.total_us.swap(0, Relaxed), self.blocks.swap(0, Relaxed));
        (n > 0).then(|| us / n)
    }
}

#[derive(Default)]
pub struct Pipeline {
    /// Device frames per capture / playback callback (big = big device buffers).
    pub in_frames: MinMax,
    pub out_frames: MinMax,
    /// Device + driver buffering each way, from the callback timestamps (WASAPI reports them).
    pub in_latency_ms: MinMax,
    pub out_latency_ms: MinMax,
    /// 10 ms mic blocks dropped as too late before processing (the processor fell behind).
    pub lag_drops: AtomicU64,
    /// 10 ms chunks dropped before sending (a stall backed them up).
    pub send_drops: AtomicU64,
    /// Silent 10 ms frames sent because the mic delivered nothing (stall / reopen).
    pub silence_fill: AtomicU64,
    pub apm: StageTime,
    /// Noise suppression + gate.
    pub chain: StageTime,
    /// The denoiser's input and output energy (sum of squares × ENERGY_SCALE), mic open only.
    ns_in: AtomicU64,
    ns_out: AtomicU64,
    /// Mic-open 10 ms blocks the gate judged, and how many it let through.
    gate_judged: AtomicU64,
    gate_passed: AtomicU64,
    /// The receive queue (fullest peer), in ms, sampled every 100 ms.
    recv_ms: Mutex<Vec<u32>>,
}

/// Block energies are f32 sums of squares; stored as integers so the audio side stays lock-free.
const ENERGY_SCALE: f64 = 1e9;
/// Below this the window had no real input (≈ −100 dBFS over a 5 s window).
const SILENT_ENERGY: u64 = 1_000;
/// "Removed everything" reads as this instead of −∞.
const ATTEN_FLOOR_DB: f64 = -60.0;

/// Level change through the denoiser over the window, in dB (negative = removed).
fn atten_db(e_in: u64, e_out: u64) -> Option<f64> {
    (e_in >= SILENT_ENERGY)
        .then(|| (10.0 * (e_out.max(1) as f64 / e_in as f64).log10()).max(ATTEN_FLOOR_DB))
}

impl Pipeline {
    /// One processed mic block: the denoiser's input/output energy and the gate's verdict. Muted
    /// blocks (`judged == false`) don't count: nothing was judged or sent.
    pub fn record_mic_block(&self, ns_in: f32, ns_out: f32, judged: bool, passed: bool) {
        if !judged {
            return;
        }
        self.ns_in
            .fetch_add((ns_in as f64 * ENERGY_SCALE) as u64, Relaxed);
        self.ns_out
            .fetch_add((ns_out as f64 * ENERGY_SCALE) as u64, Relaxed);
        self.gate_judged.fetch_add(1, Relaxed);
        if passed {
            self.gate_passed.fetch_add(1, Relaxed);
        }
    }

    pub fn sample_recv_queue(&self, ms: u32) {
        self.recv_ms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(ms);
    }

    /// Everything since the last take; counters restart.
    pub fn take(&self, underrun_ms: u64) -> Summary {
        let mut recv = std::mem::take(&mut *self.recv_ms.lock().unwrap_or_else(|e| e.into_inner()));
        recv.sort_unstable();
        let (ns_in, ns_out) = (self.ns_in.swap(0, Relaxed), self.ns_out.swap(0, Relaxed));
        let (judged, passed) = (
            self.gate_judged.swap(0, Relaxed),
            self.gate_passed.swap(0, Relaxed),
        );
        Summary {
            in_frames: self.in_frames.take(),
            out_frames: self.out_frames.take(),
            in_latency_ms: self.in_latency_ms.take(),
            out_latency_ms: self.out_latency_ms.take(),
            recv_ms: (!recv.is_empty())
                .then(|| (recv[0], recv[recv.len() / 2], recv[recv.len() - 1])),
            underrun_ms,
            lag_drops: self.lag_drops.swap(0, Relaxed),
            send_drops: self.send_drops.swap(0, Relaxed),
            silence_fill: self.silence_fill.swap(0, Relaxed),
            apm_us: self.apm.take_mean(),
            chain_us: self.chain.take_mean(),
            ns_atten_db: atten_db(ns_in, ns_out),
            gate_open_pct: (judged > 0).then(|| passed as f64 * 100.0 / judged as f64),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Summary {
    pub in_frames: Option<(u64, u64)>,
    pub out_frames: Option<(u64, u64)>,
    pub in_latency_ms: Option<(u64, u64)>,
    pub out_latency_ms: Option<(u64, u64)>,
    /// min / median / max
    pub recv_ms: Option<(u32, u32, u32)>,
    pub underrun_ms: u64,
    pub lag_drops: u64,
    pub send_drops: u64,
    pub silence_fill: u64,
    pub apm_us: Option<u64>,
    pub chain_us: Option<u64>,
    /// Denoiser output vs input power over the window, dB (negative = removed; floor −60).
    pub ns_atten_db: Option<f64>,
    /// Share of mic-open 10 ms blocks the voice gate let through.
    pub gate_open_pct: Option<f64>,
}

impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let range = |r: Option<(u64, u64)>| r.map_or("-".into(), |(a, b)| format!("{a}-{b}"));
        let us = |v: Option<u64>| v.map_or("-".into(), |v| format!("{v}us"));
        let recv = self
            .recv_ms
            .map_or("-".into(), |(a, b, c)| format!("{a}/{b}/{c}ms"));
        let ns = self.ns_atten_db.map_or("-".into(), |v| format!("{v:.1}dB"));
        let gate = self
            .gate_open_pct
            .map_or("-".into(), |v| format!("{v:.0}%"));
        write!(
            f,
            "frames in {} out {} device ms in {} out {} recv {recv} underrun {}ms drops lag {} send {} fill {} (x10ms) cpu apm {} ns+gate {} ns {ns} gate {gate}",
            range(self.in_frames),
            range(self.out_frames),
            range(self.in_latency_ms),
            range(self.out_latency_ms),
            self.underrun_ms,
            self.lag_drops,
            self.send_drops,
            self.silence_fill,
            us(self.apm_us),
            us(self.chain_us),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_window_summarises_then_starts_over() {
        let p = Pipeline::default();
        for f in [480, 1056, 480] {
            p.in_frames.record(f);
        }
        for ms in [40, 10, 20, 30, 55] {
            p.sample_recv_queue(ms);
        }
        p.lag_drops.fetch_add(2, Relaxed);
        p.chain.record(Duration::from_micros(400));
        p.chain.record(Duration::from_micros(600));
        let s = p.take(15);
        assert_eq!(s.in_frames, Some((480, 1056)));
        assert_eq!(s.recv_ms, Some((10, 30, 55)), "min / median / max");
        assert_eq!((s.lag_drops, s.chain_us, s.underrun_ms), (2, Some(500), 15));
        assert_eq!(
            s.to_string(),
            "frames in 480-1056 out - device ms in - out - recv 10/30/55ms underrun 15ms drops lag 2 send 0 fill 0 (x10ms) cpu apm - ns+gate 500us ns - gate -"
        );
        let next = p.take(0);
        assert_eq!(
            (next.in_frames, next.recv_ms, next.lag_drops, next.chain_us),
            (None, None, 0, None)
        );
    }

    /// R4: what the denoiser removed and how often the gate let the mic through, per window.
    /// Muted blocks count for neither (the gate doesn't judge them, nothing is sent).
    #[test]
    fn denoiser_attenuation_and_gate_open_share_per_window() {
        let p = Pipeline::default();
        for k in 0..4 {
            p.record_mic_block(1.0, 0.1, true, k == 0); // −10 dB, gate passed 1 of 4
        }
        p.record_mic_block(1.0, 1.0, false, false); // muted: ignored
        let s = p.take(0);
        assert_eq!(
            s.ns_atten_db.map(|v| (v * 10.0).round() / 10.0),
            Some(-10.0)
        );
        assert_eq!(s.gate_open_pct.map(f64::round), Some(25.0));
        assert!(s.to_string().ends_with(" ns -10.0dB gate 25%"), "{s}");
        let next = p.take(0);
        assert_eq!((next.ns_atten_db, next.gate_open_pct), (None, None));
    }

    #[test]
    fn fully_removed_reads_as_the_floor_and_silence_as_unmeasured() {
        let p = Pipeline::default();
        p.record_mic_block(1.0, 0.0, true, false);
        let s = p.take(0);
        assert_eq!(s.ns_atten_db, Some(-60.0));
        assert_eq!(s.gate_open_pct, Some(0.0));
        p.record_mic_block(0.0, 0.0, true, false); // digital silence in
        assert_eq!(p.take(0).ns_atten_db, None);
    }
}
