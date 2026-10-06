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

use super::controls::Controls;
use super::denoise::NsLevel;
use super::meter::Meter;
use super::mixer::Mixer;
use super::processor::RawTx;

pub const STALL_AFTER: Duration = Duration::from_secs(2);
pub const INTERNAL_RATE: u32 = 48_000;

/// Mic chunks carry the device rate they were captured at.
pub type MicChunk = (u32, Vec<i16>);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioConfig {
    /// Device names; `None` = system default.
    pub input: Option<String>,
    pub output: Option<String>,
    /// 50..=400 (%), applied after noise suppression, soft-limited — for quiet mics.
    pub input_gain_pct: u16,
    /// Manual mode: RMS threshold below which the mic is gated (0 = always open).
    pub sensitivity: f32,
    pub echo_cancel: bool,
    #[serde(default)]
    pub noise_suppression: NsLevel,
    /// Gate on detected speech above the room's noise floor (else on `sensitivity`).
    #[serde(default = "default_true")]
    pub auto_sensitivity: bool,
    pub auto_gain: bool,
    /// Pre-2026-10 on/off noise toggle: only read, to migrate old settings (see `normalized`).
    #[serde(default, skip_serializing)]
    pub noise_suppress: Option<bool>,
}

fn default_true() -> bool {
    true
}

impl AudioConfig {
    /// Fold legacy fields into the current ones.
    pub fn normalized(mut self) -> Self {
        if self.noise_suppress.take() == Some(false) {
            self.noise_suppression = NsLevel::Off;
        }
        self
    }
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input: None,
            output: None,
            input_gain_pct: 100,
            sensitivity: 0.02,
            echo_cancel: true,
            noise_suppression: NsLevel::default(),
            auto_sensitivity: true,
            auto_gain: false,
            noise_suppress: None,
        }
    }
}

/// What the mic processor should run; `version` bumps on every change.
#[derive(Clone, Debug)]
pub struct ProcCfg {
    pub level: NsLevel,
    pub auto: bool,
    pub threshold: f32,
    pub gain: f32,
    pub version: u64,
}

/// Which suppression is actually running (it can differ from the chosen level), and why.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NsStatus {
    pub active: NsLevel,
    pub note: Option<String>,
}

pub fn input_gain(pct: u16) -> f32 {
    pct.clamp(50, 400) as f32 / 100.0
}

#[derive(Clone, Debug, Serialize)]
pub struct DeviceInfo {
    pub name: String,
    pub is_default: bool,
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

/// What to do about a stalled device. Pure, so the escalation is testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StallAction {
    Reopen,
    /// Tell the user (once per stall episode).
    Report,
    /// Healthy again after a reported stall.
    Recovered,
}

/// Reopen once right away; if still dead, report and keep retrying with backoff. Reopening tears
/// down and re-acquires the device (a BT transport, say), so hammering it can keep it broken.
#[derive(Default)]
pub struct StallPolicy {
    reopens: usize,
    last_reopen: Option<Instant>,
    reported: bool,
}

impl StallPolicy {
    /// After a reopen, give the new streams this long to start calling back before judging them.
    pub const GRACE: Duration = Duration::from_secs(3);
    const BACKOFF: [Duration; 3] = [
        Duration::from_secs(5),
        Duration::from_secs(10),
        Duration::from_secs(30),
    ];

    pub fn check(&mut self, stalled: bool, now: Instant) -> Option<StallAction> {
        let Some(last) = self.last_reopen else {
            return stalled.then(|| self.reopen(now));
        };
        let since = now.saturating_duration_since(last);
        if since < Self::GRACE {
            return None;
        }
        if !stalled {
            let was_reported = self.reported;
            *self = Self::default();
            return was_reported.then_some(StallAction::Recovered);
        }
        if !self.reported {
            self.reported = true;
            return Some(StallAction::Report);
        }
        let wait = Self::BACKOFF[(self.reopens - 1).min(Self::BACKOFF.len() - 1)];
        (since >= wait).then(|| self.reopen(now))
    }

    fn reopen(&mut self, now: Instant) -> StallAction {
        self.reopens += 1;
        self.last_reopen = Some(now);
        StallAction::Reopen
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
    pub(crate) apm: Mutex<AudioProcessingModule>,
    /// (echo, agc) the APM was built with — rebuilt only when these change (rebuilding resets AEC).
    apm_cfg: Mutex<(bool, bool)>,
    /// What the mic processor should run (it polls `version`).
    pub proc_cfg: Mutex<ProcCfg>,
    ns_status: Mutex<(Option<NsStatus>, bool)>, // (current, changed since last take)
    /// Whether the last processed block was sent (the gate indicator in Settings).
    pub gate_open: std::sync::atomic::AtomicBool,
    pub watchdog: Watchdog,
    /// Audio processing errors are logged once per device session, not per 10 ms chunk.
    apm_warned: std::sync::atomic::AtomicBool,
    mic_meter: Mutex<Meter>,
    spk_meter: Mutex<Meter>,
    /// Level of what we actually send (after mute + gate): your own speaking ring.
    sent_meter: Mutex<Meter>,
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
            apm: Mutex::new(AudioProcessingModule::new(
                cfg.echo_cancel,
                cfg.auto_gain,
                true,
                false,
            )),
            apm_cfg: Mutex::new((cfg.echo_cancel, cfg.auto_gain)),
            proc_cfg: Mutex::new(ProcCfg {
                level: cfg.noise_suppression,
                auto: cfg.auto_sensitivity,
                threshold: cfg.sensitivity,
                gain: input_gain(cfg.input_gain_pct),
                version: 0,
            }),
            ns_status: Mutex::new((None, false)),
            gate_open: Default::default(),
            watchdog: Watchdog::new(Instant::now()),
            apm_warned: Default::default(),
            mic_meter: Mutex::new(Meter::default()),
            spk_meter: Mutex::new(Meter::default()),
            sent_meter: Mutex::new(Meter::default()),
        })
    }

    /// (mic rms, speaker rms) since the last call.
    pub fn take_sent_level(&self) -> f32 {
        self.sent_meter.lock().unwrap().take().0
    }

    pub fn take_levels(&self) -> (f32, f32) {
        (
            self.mic_meter.lock().unwrap().take().0,
            self.spk_meter.lock().unwrap().take().0,
        )
    }

    /// Capture path: downmix and hand 10 ms blocks to the mic processor. Nothing heavy here —
    /// a slow callback is an audible glitch.
    fn on_input(
        &self,
        data: &[f32],
        channels: usize,
        rate: u32,
        pending: &mut Vec<f32>,
        raw_tx: &RawTx,
    ) {
        self.watchdog.input_tick(Instant::now());
        let chunk = (rate / 100) as usize;
        for v in downmix(data, channels) {
            pending.push(v);
            if pending.len() == chunk {
                let block = std::mem::replace(pending, Vec::with_capacity(chunk));
                let _ = raw_tx.send((rate, block));
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
        playout: &mut super::playout::Playout,
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
            scratch.resize(frames, 0.0);
            playout.fill(scratch, |buf| {
                self.mixer.lock().unwrap().mix_into(buf, 1, master)
            });
            for (frame, v) in out.chunks_mut(ch).zip(scratch.iter()) {
                frame.fill(*v);
            }
        }
        let chunk = (rate / 100) as usize;
        let mut meter = self.spk_meter.lock().unwrap();
        for v in downmix(out, channels) {
            meter.add(v);
            reverse.push(to_i16(v));
            if reverse.len() == chunk {
                let r = self
                    .apm
                    .lock()
                    .unwrap()
                    .process_reverse_stream(reverse, rate as i32, 1);
                self.warn_apm_once(r, "playback");
                reverse.clear();
            }
        }
    }

    pub(crate) fn warn_apm_once<E: std::fmt::Display>(&self, r: Result<(), E>, path: &str) {
        if let Err(e) = r
            && !self.apm_warned.swap(true, Ordering::Relaxed)
        {
            tracing::warn!(error = %e, path, "audio processing failed (echo/noise/gain not applied)");
        }
    }

    pub fn set_ns_status(&self, s: NsStatus) {
        let mut st = self.ns_status.lock().unwrap();
        if st.0.as_ref() != Some(&s) {
            *st = (Some(s), true);
        }
    }

    pub fn take_ns_status_change(&self) -> Option<NsStatus> {
        let mut st = self.ns_status.lock().unwrap();
        if !st.1 {
            return None;
        }
        st.1 = false;
        st.0.clone()
    }

    /// The processor's per-block report: mic meter, own speaking ring, gate indicator.
    pub fn add_mic_level(&self, level: f32, sent: bool) {
        self.mic_meter.lock().unwrap().add(level);
        self.sent_meter
            .lock()
            .unwrap()
            .add(if sent { level } else { 0.0 });
        self.gate_open.store(sent, Ordering::Relaxed);
    }

    pub fn apply_config(&self, cfg: &AudioConfig) {
        {
            let mut p = self.proc_cfg.lock().unwrap();
            *p = ProcCfg {
                level: cfg.noise_suppression,
                auto: cfg.auto_sensitivity,
                threshold: cfg.sensitivity,
                gain: input_gain(cfg.input_gain_pct),
                version: p.version + 1,
            };
        }
        let wanted = (cfg.echo_cancel, cfg.auto_gain);
        let mut current = self.apm_cfg.lock().unwrap();
        if *current != wanted {
            // WebRTC's own noise suppression stays off: Off means none, and suppressors never stack.
            *self.apm.lock().unwrap() =
                AudioProcessingModule::new(cfg.echo_cancel, cfg.auto_gain, true, false);
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

/// The device's default config, but at `INTERNAL_RATE` when it supports that with the same channel
/// count and sample format: no resampling at all is the best quality there is.
pub fn prefer_internal_rate(
    default: cpal::SupportedStreamConfig,
    ranges: impl IntoIterator<Item = cpal::SupportedStreamConfigRange>,
) -> cpal::SupportedStreamConfig {
    let want = cpal::SampleRate(INTERNAL_RATE);
    ranges
        .into_iter()
        .filter(|r| {
            r.channels() == default.channels() && r.sample_format() == default.sample_format()
        })
        .find_map(|r| r.try_with_sample_rate(want))
        .unwrap_or(default)
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
    pub fn start(cfg: &AudioConfig, shared: Arc<Shared>, raw_tx: RawTx) -> anyhow::Result<Self> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<anyhow::Result<(u32, u32)>>();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let cfg = cfg.clone();
        std::thread::Builder::new()
            .name("pulse-audio".into())
            .spawn(move || {
                let built = (|| -> anyhow::Result<(cpal::Stream, cpal::Stream, u32, u32)> {
                    let input = find_device(true, cfg.input.as_deref())?;
                    let output = find_device(false, cfg.output.as_deref())?;
                    let in_cfg = prefer_internal_rate(
                        input.default_input_config()?,
                        input.supported_input_configs()?,
                    );
                    let out_cfg = prefer_internal_rate(
                        output.default_output_config()?,
                        output.supported_output_configs()?,
                    );
                    let (in_rate, in_ch) = (in_cfg.sample_rate().0, in_cfg.channels() as usize);
                    let (out_rate, out_ch) = (out_cfg.sample_rate().0, out_cfg.channels() as usize);

                    let sh = shared.clone();
                    let mut pending: Vec<f32> = Vec::new();
                    let in_stream = match in_cfg.sample_format() {
                        cpal::SampleFormat::F32 => input.build_input_stream(
                            &in_cfg.config(),
                            move |d: &[f32], _| {
                                sh.on_input(d, in_ch, in_rate, &mut pending, &raw_tx)
                            },
                            |e| tracing::warn!(error = %e, "input stream error"),
                            None,
                        )?,
                        cpal::SampleFormat::I16 => input.build_input_stream(
                            &in_cfg.config(),
                            move |d: &[i16], _| {
                                let f: Vec<f32> =
                                    d.iter().map(|s| *s as f32 / i16::MAX as f32).collect();
                                sh.on_input(&f, in_ch, in_rate, &mut pending, &raw_tx)
                            },
                            |e| tracing::warn!(error = %e, "input stream error"),
                            None,
                        )?,
                        other => return Err(anyhow!("unsupported mic sample format {other:?}")),
                    };
                    let sh = shared.clone();
                    let mut reverse = Vec::new();
                    let mut mixbuf = Vec::new();
                    let mut playout = super::playout::Playout::new(out_rate);
                    let out_stream = match out_cfg.sample_format() {
                        cpal::SampleFormat::F32 => output.build_output_stream(
                            &out_cfg.config(),
                            move |d: &mut [f32], _| {
                                sh.on_output(d, out_ch, out_rate, &mut reverse, &mut mixbuf, &mut playout)
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
                                        &mut playout,
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
        // A flapping Bluetooth device can block cpal for a long time: give up rather than hang the call.
        let (input_rate, output_rate) = match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(r) => r?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let _ = stop_tx.send(()); // if it ever finishes opening, close straight away
                return Err(anyhow!("audio device didn't open within 5 s"));
            }
            Err(_) => return Err(anyhow!("audio thread died")),
        };
        Ok(Self {
            stop: Some(stop_tx),
            input_rate,
            output_rate,
            null: None,
        })
    }

    /// Test/headless constructor: no devices. Pulls the mixer every 10 ms and sends 10 ms of silence.
    pub fn start_null(rate: u32, shared: Arc<Shared>, raw_tx: RawTx) -> Self {
        let h = tokio::spawn(async move {
            let chunk = (rate / 100) as usize;
            let mut out = vec![0f32; chunk];
            let mut reverse = Vec::new();
            let mut mixbuf = Vec::new();
            let mut playout = super::playout::Playout::new(rate);
            let mut tick = tokio::time::interval(Duration::from_millis(10));
            loop {
                tick.tick().await;
                shared.watchdog.input_tick(Instant::now());
                shared.on_output(&mut out, 1, rate, &mut reverse, &mut mixbuf, &mut playout);
                let _ = raw_tx.send((rate, vec![0f32; chunk]));
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
    use crate::voice::denoise::NsLevel;

    #[test]
    fn legacy_noise_toggle_off_migrates_to_off() {
        let old = r#"{"input":null,"output":null,"input_gain_pct":100,"sensitivity":0.02,
                      "echo_cancel":true,"noise_suppress":false,"auto_gain":false}"#;
        let cfg: AudioConfig = serde_json::from_str::<AudioConfig>(old)
            .unwrap()
            .normalized();
        assert_eq!(cfg.noise_suppression, NsLevel::Off);
        assert!(cfg.auto_sensitivity, "new field defaults on");
    }

    #[test]
    fn legacy_noise_toggle_on_gets_the_default_level() {
        let old = r#"{"input":null,"output":null,"input_gain_pct":100,"sensitivity":0.02,
                      "echo_cancel":true,"noise_suppress":true,"auto_gain":false}"#;
        let cfg: AudioConfig = serde_json::from_str::<AudioConfig>(old)
            .unwrap()
            .normalized();
        assert_eq!(cfg.noise_suppression, NsLevel::default());
    }

    #[test]
    fn ns_status_change_is_reported_once() {
        let s = Shared::new(
            &AudioConfig::default(),
            Default::default(),
            Arc::new(Mutex::new(Mixer::new(48_000, 200))),
        );
        let st = NsStatus {
            active: NsLevel::Standard,
            note: Some("Strong unavailable".into()),
        };
        s.set_ns_status(st.clone());
        assert_eq!(s.take_ns_status_change(), Some(st.clone()));
        assert_eq!(s.take_ns_status_change(), None);
        s.set_ns_status(st);
        assert_eq!(
            s.take_ns_status_change(),
            None,
            "same status again is not a change"
        );
    }

    #[test]
    fn apply_config_bumps_processor_version() {
        let s = Shared::new(
            &AudioConfig::default(),
            Default::default(),
            Arc::new(Mutex::new(Mixer::new(48_000, 200))),
        );
        let v0 = s.proc_cfg.lock().unwrap().version;
        s.apply_config(&AudioConfig {
            noise_suppression: NsLevel::Off,
            ..Default::default()
        });
        let p = s.proc_cfg.lock().unwrap();
        assert!(p.version > v0);
        assert_eq!(p.level, NsLevel::Off);
    }

    use std::time::{Duration, Instant};

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

    /// Opening at 48 kHz when the device can avoids resampling entirely (best quality, no work).
    #[test]
    fn prefers_internal_rate_when_supported() {
        use cpal::{SampleFormat::F32, SampleRate, SupportedBufferSize::Unknown};
        let default = cpal::SupportedStreamConfig::new(2, SampleRate(44_100), Unknown, F32);
        let range = |ch, lo, hi| {
            cpal::SupportedStreamConfigRange::new(ch, SampleRate(lo), SampleRate(hi), Unknown, F32)
        };

        let picked = prefer_internal_rate(default.clone(), vec![range(2, 8_000, 192_000)]);
        assert_eq!(picked.sample_rate().0, INTERNAL_RATE);
        assert_eq!(picked.channels(), 2, "keeps the default's channel count");

        let only_44k = prefer_internal_rate(default.clone(), vec![range(2, 44_100, 44_100)]);
        assert_eq!(only_44k, default, "can't do 48 kHz: keep the default");

        let other_channels = prefer_internal_rate(default.clone(), vec![range(6, 8_000, 192_000)]);
        assert_eq!(
            other_channels, default,
            "never trade channel layout for the rate"
        );
    }

    #[test]
    fn stall_policy_reopens_reports_backs_off_and_recovers() {
        use StallAction::*;
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut p = StallPolicy::default();
        assert_eq!(p.check(false, at(0)), None);
        assert_eq!(
            p.check(true, at(500)),
            Some(Reopen),
            "first stall: reopen at once"
        );
        assert_eq!(p.check(true, at(1500)), None, "grace for the new streams");
        assert_eq!(
            p.check(true, at(4000)),
            Some(Report),
            "still dead after a reopen: tell the user"
        );
        assert_eq!(p.check(true, at(4500)), None, "reported once only");
        assert_eq!(
            p.check(true, at(5000)),
            None,
            "no hammering: waits out the backoff"
        );
        assert_eq!(
            p.check(true, at(5500)),
            Some(Reopen),
            "retry 5 s after the last reopen"
        );
        assert_eq!(p.check(true, at(9000)), None);
        assert_eq!(p.check(true, at(15500)), Some(Reopen), "then 10 s");
        assert_eq!(p.check(true, at(40000)), None);
        assert_eq!(p.check(true, at(45500)), Some(Reopen), "then every 30 s");
        assert_eq!(
            p.check(false, at(49000)),
            Some(Recovered),
            "healthy again clears the warning"
        );
        assert_eq!(
            p.check(true, at(50000)),
            Some(Reopen),
            "a new episode starts fresh"
        );
        assert_eq!(
            p.check(false, at(54000)),
            None,
            "recovered without ever reporting: silent"
        );
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
