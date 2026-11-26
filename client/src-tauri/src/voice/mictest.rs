//! "Let's check": hear your own processed mic after a short delay, locally (no server needed).

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::task::JoinHandle;

use super::backlog::QueueGauge;
use super::controls::Controls;
use super::devices::{AudioConfig, AudioIo, INTERNAL_RATE, Shared};
use super::leveller::Leveller;
use super::mixer::{MixerInput, SourceKind};
use super::opusloop::OpusRoundTrip;
use super::{EventSink, VoiceError, VoiceEvent};

const DELAY_MS: u32 = 300;

/// "Hear yourself as others do" (#659): the receive side a friend's client puts your voice through, before their
/// mixer (which the test's own mixer already is: your gain at 100 %, then the master limiter, #589). Runs on the
/// processor's thread, not the audio callback; it allocates nothing and takes no lock per frame either.
pub struct ReceiveChain {
    /// The codec round trip; `None` if libopus couldn't be set up (the rest still runs).
    opus: Option<OpusRoundTrip>,
    leveller: Leveller,
}

impl Default for ReceiveChain {
    fn default() -> Self {
        Self::new()
    }
}

impl ReceiveChain {
    pub fn new() -> Self {
        let opus = OpusRoundTrip::new(super::DEFAULT_MIC_BPS as i32)
            .inspect_err(|e| tracing::warn!(error = %e, "mic test: no opus round trip"))
            .ok();
        Self {
            opus,
            leveller: Leveller::new(),
        }
    }

    /// One frame through it in place; `level` = their "Even out voice volumes".
    pub fn process(&mut self, frame: &mut [i16], level: bool) {
        if let Some(o) = &mut self.opus {
            o.process(frame);
        }
        self.leveller.process(frame, level);
    }
}

/// The mic test's "speaker": your processed mic, delayed, into the local mixer.
struct Loopback {
    delay: DelayLine,
    mixer: Arc<MixerInput>,
    /// On with "Hear yourself as others do".
    chain: Option<ReceiveChain>,
    /// The frame being put through `chain` (sized once; a frame never outgrows it).
    scratch: Vec<i16>,
}

impl Loopback {
    fn new(mixer: Arc<MixerInput>, as_others_hear: bool) -> Self {
        // a push to an unregistered source is dropped
        mixer.add_source(mixer.generation(), "self", "self", SourceKind::Voice);
        Self {
            delay: DelayLine::new((INTERNAL_RATE * DELAY_MS / 1000) as usize),
            mixer,
            chain: as_others_hear.then(ReceiveChain::new),
            scratch: Vec::with_capacity(INTERNAL_RATE as usize / 10),
        }
    }
}

impl super::processor::FrameSink for Loopback {
    fn send(&mut self, frame: &[i16]) -> bool {
        let out = match &mut self.chain {
            Some(chain) => {
                self.scratch.clear();
                self.scratch.extend_from_slice(frame);
                // the setting a friend has on: the test's mixer carries this one's (`Shared::new`)
                chain.process(&mut self.scratch, self.mixer.level_voices());
                self.delay.process(&self.scratch)
            }
            None => self.delay.process(frame),
        };
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
        shared.set_mic_sink(Some(Box::new(Loopback::new(
            shared.mix_in.clone(),
            cfg.mic_test_as_others_hear,
        ))));
        let (raw_tx, _processor) = super::processor::spawn(shared.clone());
        let io = AudioIo::start(cfg, shared.clone(), raw_tx);
        // a test needs both sides (a call joins without one, #351)
        let gaps = io.gaps();
        if let Some(e) = gaps
            .input
            .map(|e| format!("audio input: {e}"))
            .or(gaps.output.map(|e| format!("audio output: {e}")))
        {
            return Err(VoiceError::Device(e));
        }
        let queue = self.queue.clone();
        queue.record(0); // this run's worst starts from nothing, not the last run's queue
        queue.take_worst();
        let levels = tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(100));
            // the call's 5 s `voice: mic` line, here too: a mic can be diagnosed without a call (#583)
            shared.mic_levels.take();
            let mut ticks = 0u32;
            // "your mic is clipping" (#591), judged afresh for this test
            let mut clip = super::miclevels::ClipHint::default();
            loop {
                tick.tick().await;
                ticks += 1;
                if ticks.is_multiple_of(50) {
                    let levels = shared.mic_levels.take();
                    tracing::info!(%levels, "voice: mic");
                }
                let queued = shared.mix_in.max_buffered();
                queue.record((queued * 1000 / INTERNAL_RATE as usize) as u32);
                let (mic, speaker) = shared.take_levels();
                let gate_open = shared.gate_open.load(Ordering::Relaxed);
                events(VoiceEvent::Levels {
                    mic,
                    speaker,
                    gate_open,
                });
                if let Some(clipping) = clip.update(shared.mic_levels.clipped_total()) {
                    events(VoiceEvent::MicClipping { clipping });
                }
                if let Some(st) = shared.take_ns_status_change() {
                    events(VoiceEvent::NoiseSuppression {
                        active: st.active,
                        note: st.note,
                    });
                }
                if let Some(e) = super::echo_update(&shared) {
                    events(e);
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
        let mut lb = Loopback::new(input.clone(), false);
        lb.send(&[100; 480]);
        assert_eq!(input.pushed_total(), 480);
    }

    const RATE: usize = INTERNAL_RATE as usize;

    /// A deep voice (90 Hz with its harmonics) peaking at `peak` of full scale, as i16.
    fn deep(peak: f32, n: usize) -> Vec<i16> {
        let raw: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                (1..=6)
                    .map(|h| (std::f32::consts::TAU * 90.0 * h as f32 * t).sin() / h as f32)
                    .sum()
            })
            .collect();
        let max = raw.iter().fold(0f32, |m, v| m.max(v.abs()));
        raw.iter()
            .map(|v| (v / max * peak * 32767.0) as i16)
            .collect()
    }

    fn peak(x: &[i16]) -> f32 {
        x.iter()
            .fold(0f32, |m, s| m.max((*s as f32 / 32767.0).abs()))
    }

    /// #659: a loud deep voice through the receive chain comes out under the ceiling with "Even out voice
    /// volumes" on (the leveller's own limiter), and the test's mixer (100 %, master limiter) keeps the played
    /// sound under it too.
    #[test]
    fn as_others_hear_a_loud_deep_voice_comes_out_under_the_ceiling() {
        use super::super::limiter::CEILING;
        use super::super::processor::FrameSink;
        let (input, mut m) = super::super::mixer::mixer(INTERNAL_RATE, 200);
        let input = Arc::new(input);
        let mut lb = Loopback::new(input.clone(), true);
        let mut chain = ReceiveChain::new();
        let tone = deep(0.99, 2 * RATE);
        let (mut chained, mut played) = (Vec::new(), Vec::new());
        let mut out = vec![0.0f32; 480];
        for (k, f) in tone.chunks(480).enumerate() {
            let mut c = f.to_vec();
            chain.process(&mut c, true);
            chained.extend_from_slice(&c);
            lb.send(f);
            if k > 0 {
                m.mix_into(&mut out, 1, 1.0);
                played.extend_from_slice(&out);
            }
        }
        let one = 1.0 / 32767.0;
        let tail = &chained[RATE..]; // past the codec's start and the limiter's first attack
        assert!(peak(tail) <= CEILING + one, "chain peak {}", peak(tail));
        // the leveller also turns a loud voice down towards its target (up to 12 dB): quieter, still there
        assert!(peak(tail) > 0.25, "levelled, not silenced: {}", peak(tail));
        let played_peak = played.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(played_peak <= CEILING, "played peak {played_peak}");
        assert!(played_peak > 0.25, "the loopback is heard: {played_peak}");
        // the loopback runs the chain itself: the master limiter alone would hold this at the ceiling
        let late = played[played.len() - RATE / 2..]
            .iter()
            .fold(0f32, |m, v| m.max(v.abs()));
        assert!(late < 0.8 * CEILING, "levelled in the loopback: {late}");
    }

    /// With "Even out voice volumes" off the leveller leaves the voice alone (as on a friend's client with it
    /// off): only the codec touches it, so a peak above the ceiling still reaches the mixer.
    #[test]
    fn the_leveller_follows_the_even_out_setting() {
        use super::super::limiter::CEILING;
        let mut chain = ReceiveChain::new();
        let mut out = Vec::new();
        for f in deep(0.99, RATE).chunks(480) {
            let mut c = f.to_vec();
            chain.process(&mut c, false);
            out.extend_from_slice(&c);
        }
        assert!(
            peak(&out[RATE / 2..]) > CEILING,
            "{}",
            peak(&out[RATE / 2..])
        );
    }

    /// Off (the default), the loopback is exactly as before: your mic, delayed, untouched.
    #[test]
    fn off_by_default_and_untouched_when_off() {
        use super::super::processor::FrameSink;
        assert!(!AudioConfig::default().mic_test_as_others_hear);
        let old: AudioConfig = serde_json::from_str(
            r#"{"input":null,"output":null,"input_gain_pct":100,"sensitivity":0.02,"auto_gain":false}"#,
        )
        .unwrap();
        assert!(
            !old.mic_test_as_others_hear,
            "saved before the setting: off"
        );
        let (input, mut m) = super::super::mixer::mixer(INTERNAL_RATE, 2_000);
        let input = Arc::new(input);
        let mut lb = Loopback::new(input.clone(), false);
        let delay = (INTERNAL_RATE * DELAY_MS / 1000) as usize;
        let frames = delay / 480 + 2;
        for k in 0..frames {
            lb.send(&[(k as i16 + 1) * 100; 480]);
        }
        let mut out = vec![0.0f32; 480 * frames];
        m.mix_into(&mut out, 1, 1.0);
        let first = out.iter().position(|v| *v != 0.0).expect("heard");
        assert_eq!(first, delay, "only the deliberate delay");
        assert!((out[first] - 100.0 / 32767.0).abs() < 1e-6, "bit for bit");
    }

    #[test]
    fn delay_line_delays_by_n_samples() {
        let mut d = DelayLine::new(3);
        assert_eq!(d.process(&[1, 2]), vec![0, 0]);
        assert_eq!(d.process(&[3, 4, 5]), vec![0, 1, 2]);
        assert_eq!(d.process(&[6]), vec![3]);
    }
}
