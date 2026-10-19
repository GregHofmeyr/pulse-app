//! "Let's check": hear your own processed mic after a short delay, locally (no server needed).

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::task::JoinHandle;

use super::backlog::QueueGauge;
use super::controls::Controls;
use super::devices::{AudioConfig, AudioIo, INTERNAL_RATE, Shared};
use super::mixer::{MixerInput, SourceKind};
use super::{EventSink, VoiceError, VoiceEvent};

const DELAY_MS: u32 = 300;

/// The mic test's "speaker": your processed mic, delayed, into the local mixer.
struct Loopback {
    delay: DelayLine,
    mixer: Arc<MixerInput>,
}

impl Loopback {
    fn new(mixer: Arc<MixerInput>) -> Self {
        // a push to an unregistered source is dropped
        mixer.add_source(mixer.generation(), "self", "self", SourceKind::Voice);
        Self {
            delay: DelayLine::new((INTERNAL_RATE * DELAY_MS / 1000) as usize),
            mixer,
        }
    }
}

impl super::processor::FrameSink for Loopback {
    fn send(&mut self, frame: &[i16]) -> bool {
        let out = self.delay.process(frame);
        self.mixer.push("self", &out);
        true
    }
}

/// Fixed delay: output = input delayed by `n` samples (silence first).
pub struct DelayLine {
    buf: VecDeque<i16>,
}

impl DelayLine {
    pub fn new(n: usize) -> Self {
        Self {
            buf: std::iter::repeat_n(0, n).collect(),
        }
    }

    pub fn process(&mut self, input: &[i16]) -> Vec<i16> {
        self.buf.extend(input.iter().copied());
        self.buf.drain(..input.len()).collect()
    }
}

pub struct MicTest {
    running: Mutex<Option<(AudioIo, Vec<JoinHandle<()>>)>>,
    /// Audio waiting to be played (after the deliberate delay): any of it is extra delay.
    queue: Arc<QueueGauge>,
}

impl Default for MicTest {
    fn default() -> Self {
        Self {
            running: Mutex::new(None),
            queue: Arc::new(QueueGauge::default()),
        }
    }
}

impl MicTest {
    pub fn start(&self, cfg: &AudioConfig, events: EventSink) -> Result<(), VoiceError> {
        self.stop();
        let shared = Shared::new(cfg, Arc::new(Mutex::new(Controls::default())), 1000);
        // Same pipeline as a call: devices → processor (denoise, gate) → loopback. The AudioIo holds
        // the only sender into the processor, so stopping the test ends it.
        // The mixer runs at 48 kHz; the output edge resamples to the device.
        shared.set_mic_sink(Some(Box::new(Loopback::new(shared.mix_in.clone()))));
        let (raw_tx, _processor) = super::processor::spawn(shared.clone());
        let io = AudioIo::start(cfg, shared.clone(), raw_tx)
            .map_err(|e| VoiceError::Device(e.to_string()))?;
        let queue = self.queue.clone();
        queue.record(0); // this run's worst starts from nothing, not the last run's queue
        queue.take_worst();
        let levels = tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(100));
            loop {
                tick.tick().await;
                let queued = shared.mix_in.max_buffered();
                queue.record((queued * 1000 / INTERNAL_RATE as usize) as u32);
                let (mic, speaker) = shared.take_levels();
                let gate_open = shared.gate_open.load(Ordering::Relaxed);
                events(VoiceEvent::Levels {
                    mic,
                    speaker,
                    gate_open,
                });
                if let Some(st) = shared.take_ns_status_change() {
                    events(VoiceEvent::NoiseSuppression {
                        active: st.active,
                        note: st.note,
                    });
                }
            }
        });
        *self.running.lock().unwrap() = Some((io, vec![levels]));
        Ok(())
    }

    /// Audio queued for playback right now, in ms (on top of the deliberate delay).
    pub fn queue_ms(&self) -> u32 {
        self.queue.current()
    }

    pub fn stop(&self) {
        if let Some((io, tasks)) = self.running.lock().unwrap().take() {
            tracing::info!(
                worst_queue_ms = self.queue.take_worst(),
                "mic test: stopped"
            );
            for t in tasks {
                t.abort();
            }
            drop(io);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The loopback registers its source: an unregistered push is dropped (silence).
    #[test]
    fn the_loopback_is_heard() {
        use super::super::processor::FrameSink;
        let input = Arc::new(super::super::mixer::mixer(48_000, 200).0);
        let mut lb = Loopback::new(input.clone());
        lb.send(&[100; 480]);
        assert_eq!(input.pushed_total(), 480);
    }

    #[test]
    fn delay_line_delays_by_n_samples() {
        let mut d = DelayLine::new(3);
        assert_eq!(d.process(&[1, 2]), vec![0, 0]);
        assert_eq!(d.process(&[3, 4, 5]), vec![0, 1, 2]);
        assert_eq!(d.process(&[6]), vec![3]);
    }
}
