//! "Let's check": hear your own processed mic after a short delay, locally (no server needed).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::controls::Controls;
use super::devices::{AudioConfig, AudioIo, INTERNAL_RATE, MicChunk, Shared};
use super::mixer::Mixer;
use super::{EventSink, VoiceError, VoiceEvent};

const DELAY_MS: u32 = 300;

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
}

impl Default for MicTest {
    fn default() -> Self {
        Self {
            running: Mutex::new(None),
        }
    }
}

impl MicTest {
    pub fn start(&self, cfg: &AudioConfig, events: EventSink) -> Result<(), VoiceError> {
        self.stop();
        let mixer = Arc::new(Mutex::new(Mixer::new(INTERNAL_RATE, 1000)));
        let shared = Shared::new(
            cfg,
            Arc::new(Mutex::new(Controls::default())),
            mixer.clone(),
        );
        let (tx, mut rx) = mpsc::unbounded_channel::<MicChunk>();
        let io = AudioIo::start(cfg, shared.clone(), tx)
            .map_err(|e| VoiceError::Device(e.to_string()))?;
        // The mixer runs at 48 kHz; the output edge resamples to the device.
        let mut delay = DelayLine::new((INTERNAL_RATE * DELAY_MS / 1000) as usize);
        let loopback = tokio::spawn(async move {
            let mut to_internal = super::resampler::ToInternal::default();
            while let Some((rate, chunk)) = rx.recv().await {
                let out = delay.process(&to_internal.process(rate, &chunk));
                mixer.lock().unwrap().push("self", &out);
            }
        });
        let levels = tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(100));
            loop {
                tick.tick().await;
                let (mic, speaker) = shared.take_levels();
                events(VoiceEvent::Levels { mic, speaker });
            }
        });
        *self.running.lock().unwrap() = Some((io, vec![loopback, levels]));
        Ok(())
    }

    pub fn stop(&self) {
        if let Some((io, tasks)) = self.running.lock().unwrap().take() {
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

    #[test]
    fn delay_line_delays_by_n_samples() {
        let mut d = DelayLine::new(3);
        assert_eq!(d.process(&[1, 2]), vec![0, 0]);
        assert_eq!(d.process(&[3, 4, 5]), vec![0, 1, 2]);
        assert_eq!(d.process(&[6]), vec![3]);
    }
}
