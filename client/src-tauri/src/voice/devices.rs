//! Audio devices: cpal capture/playback, APM (echo cancel / noise / AGC), sensitivity gate, watchdog.
//!
//! cpal streams are !Send, so they live on a dedicated audio thread; `AudioIo` is a handle to it.
//! Everything between the devices and LiveKit runs at `INTERNAL_RATE`; devices are resampled at the
//! edge, so a device switching rate mid-call (BT headset mode = 16 kHz) can't break pitch or speed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use livekit::webrtc::native::apm::AudioProcessingModule;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::controls::Controls;
use super::meter::Meter;
use super::mixer::Mixer;

const GATE_HOLD: Duration = Duration::from_millis(300);
pub const STALL_AFTER: Duration = Duration::from_secs(2);
pub const INTERNAL_RATE: u32 = 48_000;

/// Mic chunks carry the device rate they were captured at.
pub type MicChunk = (u32, Vec<i16>);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioConfig {
    /// Device names; `None` = system default.
    pub input: Option<String>,
    pub output: Option<String>,
    /// 50..=400 (%), applied before processing — for quiet mics.
    pub input_gain_pct: u16,
    /// RMS threshold below which the mic is gated (0 = always open).
    pub sensitivity: f32,
    pub echo_cancel: bool,
    pub noise_suppress: bool,
    pub auto_gain: bool,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input: None,
            output: None,
            input_gain_pct: 100,
            sensitivity: 0.01,
            echo_cancel: true,
            noise_suppress: true,
            auto_gain: false,
        }
    }
}

pub fn input_gain(pct: u16) -> f32 {
    pct.clamp(50, 400) as f32 / 100.0
}

/// Linear resample of one block (good enough for speech at these ratios).
pub fn resample(input: &[i16], from: u32, to: u32) -> Vec<i16> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let out_len = (input.len() as u64 * to as u64 / from as u64) as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * from as f64 / to as f64;
            let (j, frac) = (pos.floor() as usize, pos.fract());
            let a = input[j.min(input.len() - 1)] as f64;
            let b = input[(j + 1).min(input.len() - 1)] as f64;
            (a + (b - a) * frac) as i16
        })
        .collect()
}

#[derive(Clone, Debug, Serialize)]
pub struct DeviceInfo {
    pub name: String,
    pub is_default: bool,
}

/// Voice-activity gate with a hold so word endings aren't chopped.
pub struct Gate {
    threshold: f32,
    open_until: Option<Instant>,
}

impl Gate {
    pub fn new(threshold: f32) -> Self {
        Self {
            threshold,
            open_until: None,
        }
    }

    pub fn set_threshold(&mut self, t: f32) {
        self.threshold = t;
    }

    /// Feed one chunk's RMS; returns whether audio should be sent.
    pub fn process(&mut self, rms: f32, now: Instant) -> bool {
        if self.threshold <= 0.0 || rms >= self.threshold {
            self.open_until = Some(now + GATE_HOLD);
            return true;
        }
        self.open_until.is_some_and(|t| now < t)
    }
}

/// FINDINGS rule 2: a device that stops calling back (BT profile switch, unplug) must be noticed.
pub struct Watchdog {
    start: Instant,
    last_in_ms: AtomicU64,
    last_out_ms: AtomicU64,
}

impl Watchdog {
    pub fn new(now: Instant) -> Self {
        Self {
            start: now,
            last_in_ms: AtomicU64::new(0),
            last_out_ms: AtomicU64::new(0),
        }
    }

    fn ms(&self, t: Instant) -> u64 {
        t.saturating_duration_since(self.start).as_millis() as u64
    }

    pub fn input_tick(&self, now: Instant) {
        self.last_in_ms.store(self.ms(now), Ordering::Relaxed);
    }

    pub fn output_tick(&self, now: Instant) {
        self.last_out_ms.store(self.ms(now), Ordering::Relaxed);
    }

    pub fn stalled(&self, now: Instant) -> bool {
        let now = self.ms(now);
        let limit = STALL_AFTER.as_millis() as u64;
        now.saturating_sub(self.last_in_ms.load(Ordering::Relaxed)) > limit
            || now.saturating_sub(self.last_out_ms.load(Ordering::Relaxed)) > limit
    }
}

pub fn downmix(data: &[f32], channels: usize) -> impl Iterator<Item = f32> + '_ {
    let ch = channels.max(1);
    data.chunks(ch)
        .map(move |f| f.iter().sum::<f32>() / ch as f32)
}

fn to_i16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

/// cpal's Linux (ALSA) backend lists plumbing names (jack, pipewire, hdmi:CARD=…), not real devices —
/// on Linux we offer the system default only and the OS picks the device. Windows lists friendly names.
fn usable(name: &str) -> bool {
    !cfg!(target_os = "linux") || name == "default"
}

fn list(
    names: Option<impl Iterator<Item = cpal::Device>>,
    default: Option<String>,
) -> Vec<DeviceInfo> {
    names
        .map(|it| {
            it.filter_map(|d| d.name().ok())
                .filter(|n| usable(n))
                .map(|name| DeviceInfo {
                    is_default: Some(&name) == default.as_ref(),
                    name,
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn list_inputs() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    list(
        host.input_devices().ok(),
        host.default_input_device().and_then(|d| d.name().ok()),
    )
}

pub fn list_outputs() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    list(
        host.output_devices().ok(),
        host.default_output_device().and_then(|d| d.name().ok()),
    )
}

/// State shared between the audio callbacks and the rest of the app.
pub struct Shared {
    /// The same Arc the VoiceManager holds: one source of truth for mute/deafen (no stale copies).
    pub controls: Arc<Mutex<Controls>>,
    pub mixer: Arc<Mutex<Mixer>>,
    gate: Mutex<Gate>,
    gain: Mutex<f32>,
    apm: Mutex<AudioProcessingModule>,
    /// (echo, agc, ns) the APM was built with — rebuilt only when these change (rebuilding resets AEC).
    apm_cfg: Mutex<(bool, bool, bool)>,
    pub watchdog: Watchdog,
    mic_meter: Mutex<Meter>,
    spk_meter: Mutex<Meter>,
}

impl Shared {
    pub fn new(
        cfg: &AudioConfig,
        controls: Arc<Mutex<Controls>>,
        mixer: Arc<Mutex<Mixer>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            controls,
            mixer,
            gate: Mutex::new(Gate::new(cfg.sensitivity)),
            gain: Mutex::new(input_gain(cfg.input_gain_pct)),
            apm: Mutex::new(AudioProcessingModule::new(
                cfg.echo_cancel,
                cfg.auto_gain,
                true,
                cfg.noise_suppress,
            )),
            apm_cfg: Mutex::new((cfg.echo_cancel, cfg.auto_gain, cfg.noise_suppress)),
            watchdog: Watchdog::new(Instant::now()),
            mic_meter: Mutex::new(Meter::default()),
            spk_meter: Mutex::new(Meter::default()),
        })
    }

    /// (mic rms, speaker rms) since the last call.
    pub fn take_levels(&self) -> (f32, f32) {
        (
            self.mic_meter.lock().unwrap().take().0,
            self.spk_meter.lock().unwrap().take().0,
        )
    }

    /// Capture path for one block of interleaved f32 samples. Emits 10 ms mono i16 chunks at `rate`.
    fn on_input(
        &self,
        data: &[f32],
        channels: usize,
        rate: u32,
        pending: &mut Vec<i16>,
        mic_tx: &mpsc::UnboundedSender<MicChunk>,
    ) {
        let now = Instant::now();
        self.watchdog.input_tick(now);
        let gain = *self.gain.lock().unwrap();
        let chunk = (rate / 100) as usize;
        for v in downmix(data, channels) {
            pending.push(to_i16(v * gain));
            if pending.len() == chunk {
                let mut buf = std::mem::replace(pending, Vec::with_capacity(chunk));
                let _ = self
                    .apm
                    .lock()
                    .unwrap()
                    .process_stream(&mut buf, rate as i32, 1);
                let rms = (buf
                    .iter()
                    .map(|s| (*s as f32 / i16::MAX as f32).powi(2))
                    .sum::<f32>()
                    / chunk as f32)
                    .sqrt();
                self.mic_meter.lock().unwrap().add(rms);
                let open = self.controls.lock().unwrap().mic_open();
                let speaking = self.gate.lock().unwrap().process(rms, now);
                if !(open && speaking) {
                    // Send silence rather than nothing: the source expects a steady stream (DTX makes it ~free).
                    buf.iter_mut().for_each(|s| *s = 0);
                }
                let _ = mic_tx.send((rate, buf));
            }
        }
    }

    /// Playback path: mix peers (48 kHz) into `out` at the device rate, feed the echo canceller.
    fn on_output(
        &self,
        out: &mut [f32],
        channels: usize,
        rate: u32,
        reverse: &mut Vec<i16>,
        scratch: &mut Vec<f32>,
    ) {
        self.watchdog.output_tick(Instant::now());
        let master = if self.controls.lock().unwrap().playout_on() {
            1.0
        } else {
            0.0
        };
        let ch = channels.max(1);
        let frames = out.len() / ch;
        if rate == INTERNAL_RATE || frames == 0 {
            self.mixer.lock().unwrap().mix_into(out, channels, master);
        } else {
            // Pull the matching amount of 48 kHz audio, then stretch it to the device rate.
            let need = (frames as u64 * INTERNAL_RATE as u64)
                .div_ceil(rate as u64)
                .max(1) as usize;
            scratch.resize(need, 0.0);
            self.mixer.lock().unwrap().mix_into(scratch, 1, master);
            for (i, frame) in out.chunks_mut(ch).enumerate() {
                let pos = i as f64 * need as f64 / frames as f64;
                let (j, frac) = (pos.floor() as usize, pos.fract() as f32);
                let a = scratch[j.min(need - 1)];
                let b = scratch[(j + 1).min(need - 1)];
                frame.fill(a + (b - a) * frac);
            }
        }
        let chunk = (rate / 100) as usize;
        let mut meter = self.spk_meter.lock().unwrap();
        for v in downmix(out, channels) {
            meter.add(v);
            reverse.push(to_i16(v));
            if reverse.len() == chunk {
                let _ = self
                    .apm
                    .lock()
                    .unwrap()
                    .process_reverse_stream(reverse, rate as i32, 1);
                reverse.clear();
            }
        }
    }

    pub fn apply_config(&self, cfg: &AudioConfig) {
        *self.gain.lock().unwrap() = input_gain(cfg.input_gain_pct);
        self.gate.lock().unwrap().set_threshold(cfg.sensitivity);
        let wanted = (cfg.echo_cancel, cfg.auto_gain, cfg.noise_suppress);
        let mut current = self.apm_cfg.lock().unwrap();
        if *current != wanted {
            *self.apm.lock().unwrap() = AudioProcessingModule::new(
                cfg.echo_cancel,
                cfg.auto_gain,
                true,
                cfg.noise_suppress,
            );
            *current = wanted;
        }
    }
}

/// Handle to the audio thread. Dropping it stops capture and playback.
pub struct AudioIo {
    stop: Option<std::sync::mpsc::Sender<()>>,
    input_rate: u32,
    output_rate: u32,
    null: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for AudioIo {
    fn drop(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
        if let Some(h) = self.null.take() {
            h.abort();
        }
    }
}

fn find_device(input: bool, name: Option<&str>) -> anyhow::Result<cpal::Device> {
    let host = cpal::default_host();
    if let Some(name) = name {
        let mut devs = if input {
            host.input_devices()?
        } else {
            host.output_devices()?
        };
        if let Some(d) = devs.find(|d| d.name().ok().as_deref() == Some(name)) {
            return Ok(d);
        }
        // Remembered device is gone (unplugged): fall back to the default.
    }
    let d = if input {
        host.default_input_device()
    } else {
        host.default_output_device()
    };
    d.context(if input {
        "no microphone found"
    } else {
        "no speakers/headphones found"
    })
}

impl AudioIo {
    pub fn start(
        cfg: &AudioConfig,
        shared: Arc<Shared>,
        mic_tx: mpsc::UnboundedSender<MicChunk>,
    ) -> anyhow::Result<Self> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<anyhow::Result<(u32, u32)>>();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let cfg = cfg.clone();
        std::thread::Builder::new()
            .name("pulse-audio".into())
            .spawn(move || {
                let built = (|| -> anyhow::Result<(cpal::Stream, cpal::Stream, u32, u32)> {
                    let input = find_device(true, cfg.input.as_deref())?;
                    let output = find_device(false, cfg.output.as_deref())?;
                    let in_cfg = input.default_input_config()?;
                    let out_cfg = output.default_output_config()?;
                    let (in_rate, in_ch) = (in_cfg.sample_rate().0, in_cfg.channels() as usize);
                    let (out_rate, out_ch) = (out_cfg.sample_rate().0, out_cfg.channels() as usize);

                    let sh = shared.clone();
                    let mut pending = Vec::new();
                    let in_stream = match in_cfg.sample_format() {
                        cpal::SampleFormat::F32 => input.build_input_stream(
                            &in_cfg.config(),
                            move |d: &[f32], _| {
                                sh.on_input(d, in_ch, in_rate, &mut pending, &mic_tx)
                            },
                            |e| tracing::warn!(error = %e, "input stream error"),
                            None,
                        )?,
                        cpal::SampleFormat::I16 => input.build_input_stream(
                            &in_cfg.config(),
                            move |d: &[i16], _| {
                                let f: Vec<f32> =
                                    d.iter().map(|s| *s as f32 / i16::MAX as f32).collect();
                                sh.on_input(&f, in_ch, in_rate, &mut pending, &mic_tx)
                            },
                            |e| tracing::warn!(error = %e, "input stream error"),
                            None,
                        )?,
                        other => return Err(anyhow!("unsupported mic sample format {other:?}")),
                    };
                    let sh = shared.clone();
                    let mut reverse = Vec::new();
                    let mut mixbuf = Vec::new();
                    let out_stream = match out_cfg.sample_format() {
                        cpal::SampleFormat::F32 => output.build_output_stream(
                            &out_cfg.config(),
                            move |d: &mut [f32], _| {
                                sh.on_output(d, out_ch, out_rate, &mut reverse, &mut mixbuf)
                            },
                            |e| tracing::warn!(error = %e, "output stream error"),
                            None,
                        )?,
                        cpal::SampleFormat::I16 => {
                            let mut scratch = Vec::new();
                            output.build_output_stream(
                                &out_cfg.config(),
                                move |d: &mut [i16], _| {
                                    scratch.resize(d.len(), 0.0);
                                    sh.on_output(
                                        &mut scratch,
                                        out_ch,
                                        out_rate,
                                        &mut reverse,
                                        &mut mixbuf,
                                    );
                                    for (o, v) in d.iter_mut().zip(&scratch) {
                                        *o = to_i16(*v);
                                    }
                                },
                                |e| tracing::warn!(error = %e, "output stream error"),
                                None,
                            )?
                        }
                        other => {
                            return Err(anyhow!("unsupported speaker sample format {other:?}"));
                        }
                    };
                    in_stream.play()?;
                    out_stream.play()?;
                    tracing::info!(
                    input = %input.name().unwrap_or_default(), in_rate, in_ch, in_fmt = ?in_cfg.sample_format(),
                    output = %output.name().unwrap_or_default(), out_rate, out_ch, out_fmt = ?out_cfg.sample_format(),
                    "audio devices opened"
                );
                Ok((in_stream, out_stream, in_rate, out_rate))
                })();
                match built {
                    Ok((_in, _out, ir, or)) => {
                        let _ = ready_tx.send(Ok((ir, or)));
                        let _ = stop_rx.recv(); // park until told to stop (or the handle is dropped)
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                }
            })?;
        let (input_rate, output_rate) = ready_rx.recv().context("audio thread died")??;
        Ok(Self {
            stop: Some(stop_tx),
            input_rate,
            output_rate,
            null: None,
        })
    }

    /// Test/headless constructor: no devices. Pulls the mixer every 10 ms and sends 10 ms of silence.
    pub fn start_null(
        rate: u32,
        shared: Arc<Shared>,
        mic_tx: mpsc::UnboundedSender<MicChunk>,
    ) -> Self {
        let h = tokio::spawn(async move {
            let chunk = (rate / 100) as usize;
            let mut out = vec![0f32; chunk];
            let mut reverse = Vec::new();
            let mut mixbuf = Vec::new();
            let mut tick = tokio::time::interval(Duration::from_millis(10));
            loop {
                tick.tick().await;
                shared.watchdog.input_tick(Instant::now());
                shared.on_output(&mut out, 1, rate, &mut reverse, &mut mixbuf);
                let _ = mic_tx.send((rate, vec![0i16; chunk]));
            }
        });
        Self {
            stop: None,
            input_rate: rate,
            output_rate: rate,
            null: Some(h),
        }
    }

    pub fn input_rate(&self) -> u32 {
        self.input_rate
    }

    pub fn output_rate(&self) -> u32 {
        self.output_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn gate_opens_on_speech_and_holds_300ms() {
        let t0 = Instant::now();
        let mut g = Gate::new(0.02);
        assert!(!g.process(0.001, t0));
        assert!(g.process(0.05, t0 + Duration::from_millis(10)));
        assert!(
            g.process(0.001, t0 + Duration::from_millis(200)),
            "held open"
        );
        assert!(
            !g.process(0.001, t0 + Duration::from_millis(400)),
            "closed after hold"
        );
        g.set_threshold(0.0);
        assert!(
            g.process(0.0, t0 + Duration::from_millis(500)),
            "threshold 0 = always open"
        );
    }

    #[test]
    fn stalled_after_2s_without_callbacks() {
        let t0 = Instant::now();
        let w = Watchdog::new(t0);
        assert!(!w.stalled(t0 + Duration::from_millis(1900)));
        assert!(w.stalled(t0 + Duration::from_millis(2100)));
        w.input_tick(t0 + Duration::from_millis(2000));
        assert!(
            w.stalled(t0 + Duration::from_millis(2100)),
            "output still silent"
        );
        w.output_tick(t0 + Duration::from_millis(2050));
        assert!(!w.stalled(t0 + Duration::from_millis(2100)));
    }

    #[test]
    fn downmix_interleaved_to_mono() {
        let stereo = [0.2f32, 0.4, -1.0, 1.0];
        let mono: Vec<f32> = downmix(&stereo, 2).collect();
        assert_eq!(mono.len(), 2);
        assert!((mono[0] - 0.3).abs() < 1e-6 && mono[1].abs() < 1e-6);
        let same: Vec<f32> = downmix(&[0.5, 0.25], 1).collect();
        assert_eq!(same, vec![0.5, 0.25]);
    }

    #[test]
    fn resample_lengths_and_endpoints() {
        let x: Vec<i16> = (0..441).map(|i| i as i16).collect(); // 10 ms @ 44.1 kHz
        let y = resample(&x, 44_100, 48_000);
        assert_eq!(y.len(), 480);
        assert_eq!(y[0], 0);
        assert!(*y.last().unwrap() >= 438);
        assert_eq!(resample(&x, 48_000, 48_000), x);
        assert_eq!(resample(&[1000; 160], 16_000, 48_000).len(), 480);
    }

    #[test]
    fn input_gain_pct_maps_linearly() {
        assert!((input_gain(100) - 1.0).abs() < 1e-6);
        assert!((input_gain(250) - 2.5).abs() < 1e-6);
        assert!((input_gain(10) - 0.5).abs() < 1e-6, "clamped to 50%");
        assert!((input_gain(900) - 4.0).abs() < 1e-6, "clamped to 400%");
    }
}

#[cfg(test)]
mod smoke {
    /// `cargo test -p pulse-client -- --ignored print_devices --nocapture` — lists devices, opens nothing.
    #[test]
    #[ignore]
    fn print_devices() {
        println!("inputs: {:?}", super::list_inputs());
        println!("outputs: {:?}", super::list_outputs());
    }
}
