//! Mouth-to-ear latency, stage by stage: how long each part of the path holds audio, averaged
//! over a minute and logged once a minute (debug, `voice: latency`), so a user's log shows where
//! the delay is. Pure accounting: nothing here changes what the audio does. The audio side only
//! touches atomics.
//!
//! What we can't measure from here is written in as the usual figure: Opus's 20 ms frame plus
//! 6.5 ms lookahead on the way out, and the 0–10 ms phase of WebRTC's 10 ms receive pull.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Duration;

/// How often the breakdown is logged in a call.
pub const LOG_EVERY: Duration = Duration::from_secs(60);

/// Opus framing on the send side: the 20 ms packet WebRTC uses by default plus the codec's 6.5 ms lookahead.
const OPUS_MS: f64 = 26.5;
/// WebRTC pulls each received stream every 10 ms on its own timer: on average audio waits half of that.
const PULL_PHASE_MS: f64 = 5.0;
/// One 10 ms block.
const BLOCK_US: u64 = 10_000;

/// Mean and largest of what was recorded since the last take (microseconds).
#[derive(Default)]
pub struct Mean {
    sum: AtomicU64,
    n: AtomicU64,
    max: AtomicU64,
}

impl Mean {
    pub fn record_us(&self, us: u64) {
        self.sum.fetch_add(us, Relaxed);
        self.n.fetch_add(1, Relaxed);
        self.max.fetch_max(us, Relaxed);
    }

    pub fn record_ms(&self, ms: f64) {
        self.record_us((ms.max(0.0) * 1000.0) as u64);
    }

    /// (mean ms, max ms); `None` if nothing was recorded. Restarts.
    pub fn take(&self) -> Option<Stage> {
        // separate swaps: a record landing between them shifts one sample between windows, which a
        // minute's average doesn't notice
        let (sum, n, max) = (
            self.sum.swap(0, Relaxed),
            self.n.swap(0, Relaxed),
            self.max.swap(0, Relaxed),
        );
        (n > 0).then(|| Stage {
            mean_ms: sum as f64 / n as f64 / 1000.0,
            max_ms: max as f64 / 1000.0,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stage {
    pub mean_ms: f64,
    pub max_ms: f64,
}

/// What the call records as it goes; [`Budget::take`] turns a minute of it into a [`Breakdown`].
#[derive(Default)]
pub struct Budget {
    /// The denoiser's own delay, per block sent.
    denoise: Mean,
    /// The voice gate's pre-roll delay, per block sent (0 once open and caught up).
    gate: Mean,
    /// WebRTC's jitter buffer (NetEq), from the 5 s stats.
    jitter_buffer: Mean,
    /// Round trip to the server, from the 5 s stats: about one way up plus one way down.
    network: Mean,
    /// The speakers' receive queue (fullest person), sampled every 100 ms.
    recv_queue: Mean,
}

impl Budget {
    /// One block went out: how long the denoiser and the gate held it (in 10 ms blocks).
    pub fn mic_block(&self, denoise_blocks: usize, gate_blocks: usize) {
        self.denoise.record_us(denoise_blocks as u64 * BLOCK_US);
        self.gate.record_us(gate_blocks as u64 * BLOCK_US);
    }

    pub fn recv_queue_ms(&self, ms: f64) {
        self.recv_queue.record_ms(ms);
    }

    /// One 5 s link sample: WebRTC's jitter buffer and the round trip, when known.
    pub fn link(&self, jitter_buffer_ms: Option<f64>, rtt_ms: Option<f64>) {
        if let Some(ms) = jitter_buffer_ms {
            self.jitter_buffer.record_ms(ms);
        }
        if let Some(ms) = rtt_ms {
            self.network.record_ms(ms);
        }
    }

    /// The minute so far, with the stages measured elsewhere: capture and output device buffering
    /// (callback timestamps) and the processor's queue (ms). Restarts.
    pub fn take(
        &self,
        capture_ms: Option<f64>,
        processor_queue_ms: Option<f64>,
        output_ms: Option<f64>,
    ) -> Breakdown {
        let mean = |m: &Mean| m.take().map(|s| s.mean_ms);
        Breakdown {
            capture_ms,
            processor_queue_ms,
            denoise_ms: mean(&self.denoise),
            gate: self.gate.take(),
            network_ms: mean(&self.network),
            jitter_buffer_ms: mean(&self.jitter_buffer),
            recv_queue: self.recv_queue.take(),
            output_ms,
        }
    }
}

/// One minute of the path, in ms (mean unless said otherwise).
#[derive(Debug, PartialEq)]
pub struct Breakdown {
    /// Device + driver buffering on the mic side.
    pub capture_ms: Option<f64>,
    /// Waiting for the mic processor.
    pub processor_queue_ms: Option<f64>,
    pub denoise_ms: Option<f64>,
    /// Gate pre-roll (mean, worst).
    pub gate: Option<Stage>,
    /// Round trip to the server (≈ up + down when the other side's link is similar).
    pub network_ms: Option<f64>,
    pub jitter_buffer_ms: Option<f64>,
    /// The speakers' queue (mean, worst).
    pub recv_queue: Option<Stage>,
    /// Device + driver buffering on the speaker side.
    pub output_ms: Option<f64>,
}

impl Breakdown {
    /// Our side of sending: mic to packet. `None` until a block has been sent.
    pub fn send_ms(&self) -> Option<f64> {
        let gate = self.gate?.mean_ms;
        Some(
            self.capture_ms.unwrap_or(0.0)
                + self.processor_queue_ms.unwrap_or(0.0)
                + self.denoise_ms.unwrap_or(0.0)
                + gate
                + OPUS_MS,
        )
    }

    /// Our side of receiving: packet to ear. `None` until something has been received.
    pub fn recv_ms(&self) -> Option<f64> {
        let queue = self.recv_queue?.mean_ms;
        Some(
            self.jitter_buffer_ms.unwrap_or(0.0)
                + PULL_PHASE_MS
                + queue
                + self.output_ms.unwrap_or(0.0),
        )
    }
}

impl fmt::Display for Breakdown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ms = |v: Option<f64>| v.map_or("-".into(), |v| format!("{v:.0}"));
        let stage = |s: Option<Stage>| {
            s.map_or("-".into(), |s| {
                format!("{:.0} (max {:.0})", s.mean_ms, s.max_ms)
            })
        };
        write!(
            f,
            "send {} = capture {} + processor queue {} + denoise {} + gate {} + opus {OPUS_MS}; \
             network {} (rtt); receive {} = jitter buffer {} + pull {PULL_PHASE_MS} + mix queue {} + output {} (ms)",
            ms(self.send_ms()),
            ms(self.capture_ms),
            ms(self.processor_queue_ms),
            ms(self.denoise_ms),
            stage(self.gate),
            ms(self.network_ms),
            ms(self.recv_ms()),
            ms(self.jitter_buffer_ms),
            stage(self.recv_queue),
            ms(self.output_ms),
        )?;
        if let (Some(s), Some(n), Some(r)) = (self.send_ms(), self.network_ms, self.recv_ms()) {
            write!(f, "; mouth to ear ~{:.0}ms", s + n + r)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_and_max_since_the_last_take() {
        let m = Mean::default();
        assert_eq!(m.take(), None);
        m.record_ms(10.0);
        m.record_ms(30.0);
        assert_eq!(
            m.take(),
            Some(Stage {
                mean_ms: 20.0,
                max_ms: 30.0
            })
        );
        assert_eq!(m.take(), None, "restarts");
    }

    #[test]
    fn a_minute_adds_up_stage_by_stage() {
        let b = Budget::default();
        b.mic_block(3, 2); // Strong, gate just opened
        b.mic_block(3, 0); // caught up
        b.link(Some(40.0), Some(30.0));
        b.recv_queue_ms(20.0);
        let d = b.take(Some(12.0), Some(0.0), Some(25.0));
        assert_eq!(d.denoise_ms, Some(30.0));
        assert_eq!(d.gate.map(|g| (g.mean_ms, g.max_ms)), Some((10.0, 20.0)));
        assert_eq!(d.send_ms(), Some(12.0 + 30.0 + 10.0 + OPUS_MS));
        assert_eq!(d.recv_ms(), Some(40.0 + PULL_PHASE_MS + 20.0 + 25.0));
        let line = d.to_string();
        assert!(line.contains("gate 10 (max 20)"), "{line}");
        assert!(line.contains("mouth to ear ~"), "{line}");
        assert_eq!(b.take(None, None, None).send_ms(), None, "restarts");
    }

    #[test]
    fn missing_stages_read_as_dashes_without_a_total() {
        let d = Budget::default().take(None, None, None);
        let line = d.to_string();
        assert!(line.starts_with("send - ="), "{line}");
        assert!(!line.contains("mouth to ear"), "{line}");
    }
}
