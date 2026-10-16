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
/// A broken device can fail every callback; log its error at most this often.
const STREAM_ERROR_EVERY: std::time::Duration = std::time::Duration::from_secs(10);

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

/// Played audio (mono i16 at the output device rate) on its way to the echo canceller.
#[derive(Default)]
struct EchoRef {
    rate: u32,
    samples: std::collections::VecDeque<i16>,
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

    /// Streams were just reopened: judge them from now, not from the old device's last callback
    /// (a slow open would otherwise look stalled at once and be torn down again).
    pub fn rearm(&self, now: Instant) {
        self.input_tick(now);
        self.output_tick(now);
    }

    pub fn stalled(&self, now: Instant) -> bool {
        self.stalled_sides(now).any()
    }

    /// Which side has gone quiet (no callbacks for `STALL_AFTER`).
    pub fn stalled_sides(&self, now: Instant) -> Sides {
        let now = self.ms(now);
        let limit = STALL_AFTER.as_millis() as u64;
        Sides {
            input: now.saturating_sub(self.last_in_ms.load(Ordering::Relaxed)) > limit,
            output: now.saturating_sub(self.last_out_ms.load(Ordering::Relaxed)) > limit,
        }
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

/// A device's name as settings and lists use it. cpal 0.18 dropped `name()`: on Linux (ALSA) the
/// stable name is the PCM id ("default", what 0.15 returned and `usable` filters on); elsewhere the
/// friendly name (`description().name()`, the same property 0.15 read on Windows).
pub fn device_name(d: &cpal::Device) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        d.id().ok().map(|id| id.id().to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        d.description().ok().map(|desc| desc.name().to_string())
    }
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
            it.filter_map(|d| device_name(&d))
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
        host.default_input_device().and_then(|d| device_name(&d)),
    )
}

pub fn list_outputs() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    list(
        host.output_devices().ok(),
        host.default_output_device().and_then(|d| device_name(&d)),
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
    /// What the speakers played, waiting for the processor to feed the echo canceller.
    echo_ref: Mutex<EchoRef>,
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
    /// Counters for the `voice: path` line (atomics: safe in the audio callbacks).
    pub stats: super::stats::Pipeline,
    /// Where the processor's frames go: the current room's mic track (`None` between rooms).
    pub(crate) mic_sink: Mutex<Option<Box<dyn super::processor::FrameSink>>>,
    /// Mic audio waiting to be processed (ms), worst per stats window: the send-side delay we own.
    pub mic_queue: super::backlog::QueueGauge,
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
            echo_ref: Mutex::new(EchoRef::default()),
            gate_open: Default::default(),
            watchdog: Watchdog::new(Instant::now()),
            apm_warned: Default::default(),
            mic_meter: Mutex::new(Meter::default()),
            spk_meter: Mutex::new(Meter::default()),
            sent_meter: Mutex::new(Meter::default()),
            stats: Default::default(),
            mic_sink: Mutex::new(None),
            mic_queue: Default::default(),
        })
    }

    pub fn set_mic_sink(&self, sink: Option<Box<dyn super::processor::FrameSink>>) {
        *self.mic_sink.lock().unwrap() = sink;
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
        self.stats
            .in_frames
            .record((data.len() / channels.max(1)) as u64);
        let chunk = (rate / 100) as usize;
        for v in downmix(data, channels) {
            pending.push(v);
            if pending.len() == chunk {
                let block = std::mem::replace(pending, Vec::with_capacity(chunk));
                let _ = raw_tx.send((rate, block, Instant::now()));
            }
        }
    }

    /// Playback path: mix peers (48 kHz) into `out` at the device rate, feed the echo canceller.
    fn on_output(
        &self,
        out: &mut [f32],
        channels: usize,
        rate: u32,
        scratch: &mut Vec<f32>,
        playout: &mut super::playout::Playout,
    ) {
        self.watchdog.output_tick(Instant::now());
        self.stats
            .out_frames
            .record((out.len() / channels.max(1)) as u64);
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
        // Hand what we played to the echo canceller via a queue the mic processor drains: never
        // take the APM lock here (this callback is real-time; the processor can hold it for ms).
        let mut meter = self.spk_meter.lock().unwrap();
        let mut echo = self.echo_ref.lock().unwrap();
        if echo.rate != rate {
            echo.rate = rate;
            echo.samples.clear();
        }
        for v in downmix(out, channels) {
            meter.add(v);
            echo.samples.push_back(to_i16(v));
        }
        let cap = (rate / 5) as usize; // 200 ms: bounded even if nothing drains it
        let excess = echo.samples.len().saturating_sub(cap);
        echo.samples.drain(..excess);
    }

    /// Feed the echo canceller everything played since the last call (whole 10 ms chunks). Called
    /// by the mic processor before each capture block, so render and capture stay in order.
    pub(crate) fn feed_echo_reference(&self) {
        let (rate, mut played) = {
            let mut echo = self.echo_ref.lock().unwrap();
            let chunk = (echo.rate / 100).max(1) as usize;
            let whole = echo.samples.len() / chunk * chunk;
            (echo.rate, echo.samples.drain(..whole).collect::<Vec<i16>>())
        };
        if played.is_empty() {
            return;
        }
        let chunk = (rate / 100) as usize;
        let mut apm = self.apm.lock().unwrap();
        for c in played.chunks_mut(chunk) {
            let r = apm.process_reverse_stream(c, rate as i32, 1);
            self.warn_apm_once(r, "playback");
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
    input: Option<Half>,
    output: Option<Half>,
    null: Option<tokio::task::JoinHandle<()>>,
    shared: Arc<Shared>,
    raw_tx: RawTx,
}

/// Something that must live and die on its own thread (cpal streams are !Send), plus a value
/// describing it. `close` waits until the thread has really dropped it; dropping a `Parked`
/// only asks.
pub struct Parked<R> {
    pub value: R,
    stop: std::sync::mpsc::Sender<()>,
    done: std::sync::mpsc::Receiver<()>,
}

impl<R: Send + 'static> Parked<R> {
    /// Run `build` on a new thread; give up after `open_timeout` (a flapping Bluetooth device
    /// can block cpal for a long time; if it ever finishes, it's closed straight away).
    pub fn spawn<T: 'static>(
        name: &str,
        open_timeout: Duration,
        build: impl FnOnce() -> anyhow::Result<(T, R)> + Send + 'static,
    ) -> anyhow::Result<Self> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<anyhow::Result<R>>();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                match build() {
                    Ok((resource, value)) => {
                        let _ = ready_tx.send(Ok(value));
                        let _ = stop_rx.recv(); // park until told to stop (or the handle is dropped)
                        drop(resource);
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                }
                let _ = done_tx.send(());
            })?;
        match ready_rx.recv_timeout(open_timeout) {
            Ok(r) => Ok(Self {
                value: r?,
                stop: stop_tx,
                done: done_rx,
            }),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let _ = stop_tx.send(());
                Err(anyhow!("didn't open within {} s", open_timeout.as_secs()))
            }
            Err(_) => Err(anyhow!("audio thread died")),
        }
    }
}

impl<R> Parked<R> {
    /// Stop it and wait (up to 1 s) until it's really gone: reopening the same device (or a
    /// Bluetooth headset, which can't be open twice) must not race the old stream's close.
    pub fn close(self) {
        let _ = self.stop.send(());
        let _ = self.done.recv_timeout(Duration::from_secs(1));
    }
}

impl<R> Drop for Parked<R> {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}

/// One direction's stream: each side can be reopened without touching the other.
struct HalfInfo {
    rate: u32,
    name: String,
    /// Which open this is (diagnostics: a side that wasn't reopened keeps its number).
    generation: u64,
}

type Half = Parked<HalfInfo>;

static OPENS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Open one direction on its own thread (cpal streams are !Send: built, played and dropped there).
fn open_half(
    input: bool,
    name: Option<String>,
    shared: Arc<Shared>,
    raw_tx: RawTx,
) -> anyhow::Result<Half> {
    let side = if input { "input" } else { "output" };
    Parked::spawn(
        if input { "pulse-audio-in" } else { "pulse-audio-out" },
        Duration::from_secs(5),
        move || {
            let dev = find_device(input, name.as_deref())?;
            let cfg = if input {
                prefer_internal_rate(dev.default_input_config()?, dev.supported_input_configs()?)
            } else {
                prefer_internal_rate(dev.default_output_config()?, dev.supported_output_configs()?)
            };
            let (rate, ch) = (cfg.sample_rate(), cfg.channels() as usize);
            let stream = if input {
                let mut pending: Vec<f32> = Vec::new();
                build_input(&dev, &cfg, move |d: &[f32], latency| {
                    if let Some(l) = latency {
                        shared.stats.in_latency_ms.record(l.as_millis() as u64);
                    }
                    shared.on_input(d, ch, rate, &mut pending, &raw_tx)
                })?
            } else {
                let (mut mixbuf, mut playout) = (Vec::new(), super::playout::Playout::new(rate));
                build_output(&dev, &cfg, move |d: &mut [f32], latency| {
                    if let Some(l) = latency {
                        shared.stats.out_latency_ms.record(l.as_millis() as u64);
                    }
                    shared.on_output(d, ch, rate, &mut mixbuf, &mut playout)
                })?
            };
            stream.play()?;
            let name = device_name(&dev).unwrap_or_default();
            tracing::info!(device = %name, rate, ch, fmt = ?cfg.sample_format(), "audio {side} opened");
            let generation = OPENS.fetch_add(1, Ordering::Relaxed) + 1;
            Ok((stream, HalfInfo { rate, name, generation }))
        },
    )
    .map_err(|e| anyhow!("audio {side}: {e}"))
}

/// The devices actually opened (names as the system reports them).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenedDevices {
    pub input: String,
    pub output: String,
}

/// Where the user chose "default", follow the system: when Windows' default device changes
/// (headset plugged in, switched in Sonar), the streams must move with it, like Discord does.
pub fn default_moved(
    cfg: &AudioConfig,
    opened: &OpenedDevices,
    defaults: (Option<String>, Option<String>),
) -> bool {
    moved_sides(cfg, opened, defaults).any()
}

/// Which sides `default_moved` applies to.
pub fn moved_sides(
    cfg: &AudioConfig,
    opened: &OpenedDevices,
    defaults: (Option<String>, Option<String>),
) -> Sides {
    // an empty name couldn't be read: unknown, never "different"
    let moved = |chosen: &Option<String>, now: Option<String>, open: &str| {
        chosen.is_none() && !open.is_empty() && now.is_some_and(|n| n != open)
    };
    Sides {
        input: moved(&cfg.input, defaults.0, &opened.input),
        output: moved(&cfg.output, defaults.1, &opened.output),
    }
}

/// A device the user picked was missing at the last open (we fell back to the default) and is
/// available again: switch back to it.
pub fn wanted_back(
    cfg: &AudioConfig,
    opened: &OpenedDevices,
    inputs: &[String],
    outputs: &[String],
) -> bool {
    back_sides(cfg, opened, inputs, outputs).any()
}

/// Which sides `wanted_back` applies to.
pub fn back_sides(
    cfg: &AudioConfig,
    opened: &OpenedDevices,
    inputs: &[String],
    outputs: &[String],
) -> Sides {
    let back = |chosen: &Option<String>, open: &str, available: &[String]| {
        !open.is_empty()
            && chosen
                .as_deref()
                .is_some_and(|c| c != open && available.iter().any(|a| a == c))
    };
    Sides {
        input: back(&cfg.input, &opened.input, inputs),
        output: back(&cfg.output, &opened.output, outputs),
    }
}

/// Which directions to (re)open. Reopening only the one that changed matters: tearing down a
/// Bluetooth headset's mic for a speaker change makes it renegotiate (a gap, maybe a profile flip).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sides {
    pub input: bool,
    pub output: bool,
}

impl Sides {
    pub const BOTH: Self = Self {
        input: true,
        output: true,
    };

    pub fn any(self) -> bool {
        self.input || self.output
    }

    pub fn or(self, o: Self) -> Self {
        Self {
            input: self.input || o.input,
            output: self.output || o.output,
        }
    }

    /// The device choices that differ between two settings.
    pub fn changed(old: &AudioConfig, new: &AudioConfig) -> Self {
        Self {
            input: old.input != new.input,
            output: old.output != new.output,
        }
    }
}

/// Whether a chosen device is not the one open (only then is enumerating worth it).
pub fn on_fallback(cfg: &AudioConfig, opened: &OpenedDevices) -> bool {
    let off = |chosen: &Option<String>, open: &str| {
        !open.is_empty() && chosen.as_deref().is_some_and(|c| c != open)
    };
    off(&cfg.input, &opened.input) || off(&cfg.output, &opened.output)
}

/// How long a device that wouldn't open is left alone before trying it again.
pub const RETRY_FAILED: Duration = Duration::from_secs(60);

/// Device names per side that a switch was aimed at (`None` = no particular one).
pub type Targets = (Option<String>, Option<String>);

/// Devices a switch was aimed at but didn't end up on (it wouldn't open; we fell back). Without
/// this, following would see "the default moved" / "your device is back" on every check and tear
/// the working stream down again and again.
#[derive(Default)]
pub struct FailedTargets {
    input: Option<(String, Instant)>,
    output: Option<(String, Instant)>,
}

impl FailedTargets {
    /// After reopening `sides` aiming at `targets`: remember the misses, forget the hits.
    pub fn record(
        &mut self,
        sides: Sides,
        targets: &Targets,
        opened: &OpenedDevices,
        now: Instant,
    ) {
        let note =
            |slot: &mut Option<(String, Instant)>, wanted: bool, t: &Option<String>, got: &str| {
                if let (true, Some(t)) = (wanted, t) {
                    *slot = (t != got).then(|| (t.clone(), now));
                }
            };
        note(&mut self.input, sides.input, &targets.0, &opened.input);
        note(&mut self.output, sides.output, &targets.1, &opened.output);
    }

    /// `sides` without those aimed at a device that failed recently.
    pub fn filter(&self, sides: Sides, targets: &Targets, now: Instant) -> Sides {
        let blocked = |slot: &Option<(String, Instant)>, t: &Option<String>| {
            matches!((slot, t), (Some((name, at)), Some(t))
                if name == t && now.saturating_duration_since(*at) < RETRY_FAILED)
        };
        Sides {
            input: sides.input && !blocked(&self.input, &targets.0),
            output: sides.output && !blocked(&self.output, &targets.1),
        }
    }
}

/// Act only on a condition seen on two checks in a row: a Bluetooth device that flaps in and out
/// for a moment shouldn't tear the call's audio down.
#[derive(Default)]
pub struct Debounce {
    last: Option<Sides>,
}

impl Debounce {
    /// `seen`: the switch this check asks for. True when the previous check asked for the same.
    pub fn confirm(&mut self, seen: Option<Sides>) -> bool {
        if seen.is_some() && seen == self.last {
            self.last = None;
            return true;
        }
        self.last = seen;
        false
    }
}

/// Names of all input and output devices (blocking: device enumeration).
pub fn device_names() -> (Vec<String>, Vec<String>) {
    fn names(devs: Result<impl Iterator<Item = cpal::Device>, cpal::Error>) -> Vec<String> {
        devs.map(|d| d.filter_map(|d| device_name(&d)).collect())
            .unwrap_or_default()
    }
    let host = cpal::default_host();
    (names(host.input_devices()), names(host.output_devices()))
}

/// The system's current default devices' names (blocking: device enumeration).
pub fn default_device_names() -> (Option<String>, Option<String>) {
    let host = cpal::default_host();
    (
        host.default_input_device().and_then(|d| device_name(&d)),
        host.default_output_device().and_then(|d| device_name(&d)),
    )
}

impl Drop for AudioIo {
    fn drop(&mut self) {
        // the halves stop themselves when dropped
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
    let want = INTERNAL_RATE;
    ranges
        .into_iter()
        .filter(|r| {
            r.channels() == default.channels() && r.sample_format() == default.sample_format()
        })
        .find_map(|r| r.try_with_sample_rate(want))
        .unwrap_or(default)
}

pub fn find_device(input: bool, name: Option<&str>) -> anyhow::Result<cpal::Device> {
    let host = cpal::default_host();
    if let Some(name) = name {
        let mut devs = if input {
            host.input_devices()?
        } else {
            host.output_devices()?
        };
        if let Some(d) = devs.find(|d| device_name(d).as_deref() == Some(name)) {
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

/// Device samples (any format) → f32 in `dst`.
pub fn to_f32<T: cpal::Sample>(src: &[T], dst: &mut Vec<f32>)
where
    f32: cpal::FromSample<T>,
{
    dst.clear();
    dst.extend(
        src.iter()
            .map(|s| <f32 as cpal::FromSample<T>>::from_sample_(*s)),
    );
}

/// Largest sample we hand to a device: one 24-bit step below +1.0. dasp's conversions assume
/// `-1.0 <= s < 1.0`; at exactly +1.0 they give I24(8_388_608) / U24(16_777_216), one past the top,
/// which cpal's WASAPI path wraps into a full-scale negative spike. Other formats lose < 1 LSB.
const MAX_OUT: f32 = 8_388_607.0 / 8_388_608.0;

/// f32 → device samples (any format).
pub fn from_f32<T: cpal::Sample + cpal::FromSample<f32>>(src: &[f32], dst: &mut [T]) {
    for (o, v) in dst.iter_mut().zip(src) {
        *o = <T as cpal::FromSample<f32>>::from_sample_(v.clamp(-1.0, MAX_OUT));
    }
}

/// A stream-error callback that logs at most once per `STREAM_ERROR_EVERY` (a broken device can
/// fire thousands per second).
fn stream_errors(what: &'static str) -> impl FnMut(cpal::Error) + Send + 'static {
    let mut t = crate::throttle::Throttle::new(STREAM_ERROR_EVERY);
    move |e| {
        if let Some(repeated) = t.hit(std::time::Instant::now()) {
            tracing::warn!(error = %e, repeated, "{what} stream error");
        }
    }
}

/// The pipeline works in f32; the device may want anything cpal supports.
/// How long captured audio waited before reaching us (device + driver buffering), if the backend says.
fn capture_latency(info: &cpal::InputCallbackInfo) -> Option<Duration> {
    let t = info.timestamp();
    t.callback.checked_duration_since(t.capture)
}

/// How long until what we write is heard (device + driver buffering), if the backend says.
fn playback_latency(info: &cpal::OutputCallbackInfo) -> Option<Duration> {
    let t = info.timestamp();
    t.playback.checked_duration_since(t.callback)
}

fn build_input(
    dev: &cpal::Device,
    cfg: &cpal::SupportedStreamConfig,
    mut on_data: impl FnMut(&[f32], Option<Duration>) + Send + 'static,
) -> anyhow::Result<cpal::Stream> {
    fn typed<T: cpal::SizedSample>(
        dev: &cpal::Device,
        cfg: &cpal::StreamConfig,
        mut on_data: impl FnMut(&[f32], Option<Duration>) + Send + 'static,
    ) -> anyhow::Result<cpal::Stream>
    where
        f32: cpal::FromSample<T>,
    {
        let mut buf = Vec::new();
        Ok(dev.build_input_stream(
            *cfg,
            move |d: &[T], info: &cpal::InputCallbackInfo| {
                to_f32(d, &mut buf);
                on_data(&buf, capture_latency(info))
            },
            stream_errors("input"),
            None,
        )?)
    }
    let c = cfg.config();
    use cpal::SampleFormat as F;
    match cfg.sample_format() {
        // f32 straight through (the common case: no copy)
        F::F32 => Ok(dev.build_input_stream(
            c,
            move |d: &[f32], info: &cpal::InputCallbackInfo| on_data(d, capture_latency(info)),
            stream_errors("input"),
            None,
        )?),
        F::F64 => typed::<f64>(dev, &c, on_data),
        F::I8 => typed::<i8>(dev, &c, on_data),
        F::I16 => typed::<i16>(dev, &c, on_data),
        F::I24 => typed::<cpal::I24>(dev, &c, on_data),
        F::U24 => typed::<cpal::U24>(dev, &c, on_data),
        F::I32 => typed::<i32>(dev, &c, on_data),
        F::I64 => typed::<i64>(dev, &c, on_data),
        F::U8 => typed::<u8>(dev, &c, on_data),
        F::U16 => typed::<u16>(dev, &c, on_data),
        F::U32 => typed::<u32>(dev, &c, on_data),
        F::U64 => typed::<u64>(dev, &c, on_data),
        other => Err(anyhow!("unsupported mic sample format {other:?}")),
    }
}

pub fn build_output(
    dev: &cpal::Device,
    cfg: &cpal::SupportedStreamConfig,
    mut fill: impl FnMut(&mut [f32], Option<Duration>) + Send + 'static,
) -> anyhow::Result<cpal::Stream> {
    fn typed<T: cpal::SizedSample + cpal::FromSample<f32>>(
        dev: &cpal::Device,
        cfg: &cpal::StreamConfig,
        mut fill: impl FnMut(&mut [f32], Option<Duration>) + Send + 'static,
    ) -> anyhow::Result<cpal::Stream> {
        let mut buf = Vec::new();
        Ok(dev.build_output_stream(
            *cfg,
            move |d: &mut [T], info: &cpal::OutputCallbackInfo| {
                buf.resize(d.len(), 0.0);
                fill(&mut buf, playback_latency(info));
                from_f32(&buf, d);
            },
            stream_errors("output"),
            None,
        )?)
    }
    let c = cfg.config();
    use cpal::SampleFormat as F;
    match cfg.sample_format() {
        F::F32 => Ok(dev.build_output_stream(
            c,
            move |d: &mut [f32], info: &cpal::OutputCallbackInfo| fill(d, playback_latency(info)),
            stream_errors("output"),
            None,
        )?),
        F::F64 => typed::<f64>(dev, &c, fill),
        F::I8 => typed::<i8>(dev, &c, fill),
        F::I16 => typed::<i16>(dev, &c, fill),
        F::I24 => typed::<cpal::I24>(dev, &c, fill),
        F::U24 => typed::<cpal::U24>(dev, &c, fill),
        F::I32 => typed::<i32>(dev, &c, fill),
        F::I64 => typed::<i64>(dev, &c, fill),
        F::U8 => typed::<u8>(dev, &c, fill),
        F::U16 => typed::<u16>(dev, &c, fill),
        F::U32 => typed::<u32>(dev, &c, fill),
        F::U64 => typed::<u64>(dev, &c, fill),
        other => Err(anyhow!("unsupported speaker sample format {other:?}")),
    }
}

impl AudioIo {
    pub fn start(cfg: &AudioConfig, shared: Arc<Shared>, raw_tx: RawTx) -> anyhow::Result<Self> {
        let input = open_half(true, cfg.input.clone(), shared.clone(), raw_tx.clone())?;
        let output = open_half(false, cfg.output.clone(), shared.clone(), raw_tx.clone())?;
        Ok(Self {
            input: Some(input),
            output: Some(output),
            null: None,
            shared,
            raw_tx,
        })
    }

    /// Reopen just `sides` on the devices `cfg` names (blocking). A side that fails goes back to
    /// the device it had, so the call keeps working audio; the first error is still returned.
    /// `keep_going` is asked before each side: the session may have ended meanwhile.
    pub fn reopen(
        &mut self,
        cfg: &AudioConfig,
        sides: Sides,
        keep_going: impl Fn() -> bool,
    ) -> anyhow::Result<()> {
        if self.null.is_some() {
            return Ok(());
        }
        let mut first_err = None;
        for (input, wanted) in [(true, sides.input), (false, sides.output)] {
            if !wanted || !keep_going() {
                continue;
            }
            let slot = if input {
                &mut self.input
            } else {
                &mut self.output
            };
            // stop the old stream first (a Bluetooth headset can't be open twice)
            let previous = slot.take().map(|h| {
                let name = h.value.name.clone();
                h.close();
                name
            });
            let name = if input {
                cfg.input.clone()
            } else {
                cfg.output.clone()
            };
            match open_half(input, name, self.shared.clone(), self.raw_tx.clone()) {
                Ok(h) => *slot = Some(h),
                Err(e) => {
                    tracing::error!(error = %e, input, "voice: reopening a device failed");
                    let back = previous.filter(|n| !n.is_empty());
                    if let Ok(h) = open_half(input, back, self.shared.clone(), self.raw_tx.clone())
                    {
                        tracing::warn!(device = %h.value.name, "voice: back on the previous device");
                        *slot = Some(h);
                    }
                    first_err.get_or_insert(e);
                }
            }
        }
        first_err.map_or(Ok(()), Err)
    }

    /// Nothing open yet (`reopen` fills in the sides).
    pub fn empty(shared: Arc<Shared>, raw_tx: RawTx) -> Self {
        Self {
            input: None,
            output: None,
            null: None,
            shared,
            raw_tx,
        }
    }

    /// Test/headless constructor: no devices. Pulls the mixer every 10 ms and sends 10 ms of silence.
    pub fn start_null(rate: u32, shared: Arc<Shared>, raw_tx: RawTx) -> Self {
        let (sh, tx) = (shared.clone(), raw_tx.clone());
        let h = tokio::spawn(async move {
            let chunk = (rate / 100) as usize;
            let mut out = vec![0f32; chunk];
            let mut mixbuf = Vec::new();
            let mut playout = super::playout::Playout::new(rate);
            let mut tick = tokio::time::interval(Duration::from_millis(10));
            loop {
                tick.tick().await;
                sh.watchdog.input_tick(Instant::now());
                sh.on_output(&mut out, 1, rate, &mut mixbuf, &mut playout);
                let _ = tx.send((rate, vec![0f32; chunk], Instant::now()));
            }
        });
        Self {
            input: None,
            output: None,
            null: Some(h),
            shared,
            raw_tx,
        }
    }

    /// The devices open now ("" for a side that isn't).
    pub fn opened(&self) -> OpenedDevices {
        let name = |h: &Option<Half>| h.as_ref().map(|h| h.value.name.clone()).unwrap_or_default();
        OpenedDevices {
            input: name(&self.input),
            output: name(&self.output),
        }
    }

    /// (input, output) open generations: changes only for a side that was reopened.
    pub fn generations(&self) -> (Option<u64>, Option<u64>) {
        (
            self.input.as_ref().map(|h| h.value.generation),
            self.output.as_ref().map(|h| h.value.generation),
        )
    }

    pub fn input_rate(&self) -> Option<u32> {
        self.input.as_ref().map(|h| h.value.rate)
    }

    pub fn output_rate(&self) -> Option<u32> {
        self.output.as_ref().map(|h| h.value.rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::denoise::NsLevel;

    /// The speaker callback is real-time: it must never wait while the mic processor holds the
    /// echo canceller (a preempted processor would otherwise stutter playback).
    #[test]
    fn playback_never_waits_for_the_echo_canceller() {
        let s = Shared::new(
            &AudioConfig::default(),
            Default::default(),
            Arc::new(Mutex::new(Mixer::new(48_000, 200))),
        );
        let held = s.apm.lock().unwrap(); // the processor is busy with it
        let s2 = s.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut out = vec![0f32; 960]; // 10 ms stereo at 48 kHz
            let mut scratch = Vec::new();
            let mut playout = super::super::playout::Playout::new(48_000);
            s2.on_output(&mut out, 2, 48_000, &mut scratch, &mut playout);
            let _ = tx.send(());
        });
        let done = rx.recv_timeout(Duration::from_millis(500)).is_ok();
        drop(held);
        assert!(
            done,
            "the speaker callback blocked on the echo canceller's lock"
        );
    }

    fn play(s: &Shared, ms: usize) {
        let mut playout = super::super::playout::Playout::new(48_000);
        for _ in 0..ms / 10 {
            let mut out = vec![0.1f32; 480];
            s.on_output(&mut out, 1, 48_000, &mut Vec::new(), &mut playout);
        }
    }

    #[test]
    fn echo_reference_is_fed_in_whole_10ms_chunks() {
        let s = Shared::new(
            &AudioConfig::default(),
            Default::default(),
            Arc::new(Mutex::new(Mixer::new(48_000, 200))),
        );
        let mut playout = super::super::playout::Playout::new(48_000);
        let mut out = vec![0.1f32; 1200]; // 25 ms
        s.on_output(&mut out, 1, 48_000, &mut Vec::new(), &mut playout);
        s.feed_echo_reference();
        assert_eq!(
            s.echo_ref.lock().unwrap().samples.len(),
            240,
            "2 chunks fed, 5 ms waits for more"
        );
    }

    #[test]
    fn echo_reference_queue_is_capped_at_200ms() {
        let s = Shared::new(
            &AudioConfig::default(),
            Default::default(),
            Arc::new(Mutex::new(Mixer::new(48_000, 200))),
        );
        play(&s, 1000); // nothing drains it (processor stopped)
        assert_eq!(s.echo_ref.lock().unwrap().samples.len(), 9600);
    }

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

    fn opened(i: &str, o: &str) -> OpenedDevices {
        OpenedDevices {
            input: i.into(),
            output: o.into(),
        }
    }

    #[test]
    fn every_sample_format_converts_to_and_from_f32() {
        let mut f = Vec::new();
        to_f32(&[128u8, 255, 0], &mut f); // u8 is offset binary: 128 = silence
        assert!(f[0].abs() < 0.01 && f[1] > 0.99 && f[2] < -0.99, "{f:?}");
        to_f32(&[i32::MAX, 0, i32::MIN], &mut f);
        assert!(f[0] > 0.99 && f[1] == 0.0 && f[2] <= -0.99, "{f:?}");
        let mut out = [0u8; 2];
        from_f32(&[0.0, 1.0], &mut out);
        assert_eq!(out, [128, 255]);
        let mut out = [0i32; 2];
        from_f32(&[0.0, -1.0], &mut out);
        assert_eq!(out, [0, i32::MIN]);
    }

    #[test]
    fn follows_the_system_default_only_where_no_device_was_chosen() {
        let on_default = AudioConfig::default();
        let now = opened("Sonar Mic", "Sonar Chat");
        // Windows' default moved to the Arctis: switch
        assert!(default_moved(
            &on_default,
            &now,
            (Some("Sonar Mic".into()), Some("Arctis".into()))
        ));
        assert!(default_moved(
            &on_default,
            &now,
            (Some("Arctis Mic".into()), Some("Sonar Chat".into()))
        ));
        // unchanged, or nothing to switch to: stay
        assert!(!default_moved(
            &on_default,
            &now,
            (Some("Sonar Mic".into()), Some("Sonar Chat".into()))
        ));
        assert!(!default_moved(&on_default, &now, (None, None)));
        // a device the user picked stays picked, wherever the default goes
        let picked = AudioConfig {
            input: Some("Sonar Mic".into()),
            output: Some("Sonar Chat".into()),
            ..Default::default()
        };
        assert!(!default_moved(
            &picked,
            &now,
            (Some("Arctis Mic".into()), Some("Arctis".into()))
        ));
    }

    /// Reopening a Bluetooth headset (or falling back to the same device) needs the old stream
    /// really closed first: close() returns only once the thread has dropped it.
    #[test]
    fn closing_waits_for_the_old_stream_to_be_dropped() {
        use std::sync::atomic::AtomicBool;
        struct SlowClose(Arc<AtomicBool>);
        impl Drop for SlowClose {
            fn drop(&mut self) {
                std::thread::sleep(Duration::from_millis(150)); // e.g. WASAPI joining its thread
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let closed = Arc::new(AtomicBool::new(false));
        let c = closed.clone();
        let p = Parked::spawn("test", Duration::from_secs(1), move || {
            Ok((SlowClose(c), 7u32))
        })
        .unwrap();
        assert_eq!(p.value, 7);
        p.close();
        assert!(
            closed.load(Ordering::SeqCst),
            "close returned before the resource was dropped"
        );
    }

    #[test]
    fn a_resource_that_never_opens_times_out() {
        let r = Parked::<u32>::spawn("test", Duration::from_millis(50), || {
            std::thread::sleep(Duration::from_millis(300));
            Ok(((), 1))
        });
        assert!(r.is_err());
    }

    #[test]
    fn a_device_that_wont_open_isnt_retried_every_check() {
        let t0 = Instant::now();
        let mut f = FailedTargets::default();
        let speakers_to_arctis = (None, Some("Arctis".to_string()));
        let out = Sides {
            input: false,
            output: true,
        };
        // tried the Arctis, ended up back on the speakers
        f.record(out, &speakers_to_arctis, &opened("Mic", "Speakers"), t0);
        assert!(
            !f.filter(out, &speakers_to_arctis, t0 + Duration::from_secs(4))
                .any()
        );
        // a different target is fine at once
        let to_sonar = (None, Some("Sonar".to_string()));
        assert_eq!(f.filter(out, &to_sonar, t0 + Duration::from_secs(4)), out);
        // and the Arctis gets another go after a while
        assert_eq!(
            f.filter(
                out,
                &speakers_to_arctis,
                t0 + RETRY_FAILED + Duration::from_secs(1)
            ),
            out
        );
        // a switch that worked clears it
        f.record(out, &speakers_to_arctis, &opened("Mic", "Arctis"), t0);
        assert_eq!(f.filter(out, &speakers_to_arctis, t0), out);
    }

    #[test]
    fn only_the_side_that_changed_is_reopened() {
        let now = opened("Arctis Mic", "Speakers");
        // only the output default moved: the mic (maybe a BT headset) is left alone
        assert_eq!(
            moved_sides(
                &AudioConfig::default(),
                &now,
                (Some("Arctis Mic".into()), Some("Arctis".into()))
            ),
            Sides {
                input: false,
                output: true
            }
        );
        let picked = AudioConfig {
            input: Some("USB Mic".into()),
            ..Default::default()
        };
        assert_eq!(
            back_sides(&picked, &now, &["USB Mic".into()], &[]),
            Sides {
                input: true,
                output: false
            }
        );
        // the settings: what changed between two configs
        let old = AudioConfig::default();
        let new = AudioConfig {
            output: Some("Arctis".into()),
            ..Default::default()
        };
        assert_eq!(
            Sides::changed(&old, &new),
            Sides {
                input: false,
                output: true
            }
        );
        assert!(!Sides::changed(&old, &old).any());
    }

    #[test]
    fn the_watchdog_says_which_side_stalled() {
        let t0 = Instant::now();
        let w = Watchdog::new(t0);
        w.output_tick(t0 + Duration::from_millis(2000)); // speakers alive, mic silent
        assert_eq!(
            w.stalled_sides(t0 + Duration::from_millis(2500)),
            Sides {
                input: true,
                output: false
            }
        );
    }

    #[test]
    fn an_unknown_device_name_never_triggers_a_switch() {
        // a device whose name couldn't be read: "" mustn't look like "a different device"
        let unknown = opened("", "");
        assert!(!default_moved(
            &AudioConfig::default(),
            &unknown,
            (Some("Arctis Mic".into()), Some("Arctis".into()))
        ));
        let picked = AudioConfig {
            output: Some("Arctis".into()),
            ..Default::default()
        };
        assert!(!on_fallback(&picked, &unknown));
        assert!(!wanted_back(&picked, &unknown, &[], &["Arctis".into()]));
    }

    #[test]
    fn on_fallback_only_when_a_chosen_device_isnt_the_open_one() {
        let picked = AudioConfig {
            input: Some("Arctis Mic".into()),
            ..Default::default()
        };
        assert!(on_fallback(&picked, &opened("Laptop Mic", "Speakers")));
        assert!(!on_fallback(&picked, &opened("Arctis Mic", "Speakers")));
        assert!(!on_fallback(
            &AudioConfig::default(),
            &opened("Laptop Mic", "Speakers")
        ));
    }

    #[test]
    fn a_switch_must_be_seen_twice_in_a_row() {
        // a flapping Bluetooth device appears for one check, then vanishes: no reopen
        let out = Sides {
            input: false,
            output: true,
        };
        let inp = Sides {
            input: true,
            output: false,
        };
        let mut d = Debounce::default();
        assert!(!d.confirm(Some(out)));
        assert!(!d.confirm(None));
        assert!(!d.confirm(Some(out)));
        assert!(d.confirm(Some(out)));
        assert!(!d.confirm(Some(out)), "after acting, start counting again");
        // an input flap then an output flap is two different things, not one seen twice
        let mut d = Debounce::default();
        assert!(!d.confirm(Some(inp)));
        assert!(!d.confirm(Some(out)));
        assert!(d.confirm(Some(out)));
    }

    #[test]
    fn rearming_gives_new_streams_time_to_start() {
        let t0 = Instant::now();
        let w = Watchdog::new(t0);
        let reopened = t0 + Duration::from_secs(3); // a slow Bluetooth open
        assert!(w.stalled(reopened));
        w.rearm(reopened);
        assert!(!w.stalled(reopened + Duration::from_millis(1500)));
        assert!(
            w.stalled(reopened + Duration::from_millis(2500)),
            "still judged, just later"
        );
    }

    #[test]
    fn a_chosen_device_is_taken_back_when_it_returns() {
        let picked = AudioConfig {
            output: Some("Arctis".into()),
            ..Default::default()
        };
        // the Arctis was unplugged, so we fell back to the speakers
        let fallen_back = opened("Mic", "Speakers");
        assert!(wanted_back(
            &picked,
            &fallen_back,
            &[],
            &["Speakers".into(), "Arctis".into()]
        ));
        assert!(
            !wanted_back(&picked, &fallen_back, &[], &["Speakers".into()]),
            "still gone"
        );
        assert!(
            !wanted_back(&picked, &opened("Mic", "Arctis"), &[], &["Arctis".into()]),
            "already on it"
        );
        assert!(!wanted_back(
            &AudioConfig::default(),
            &fallen_back,
            &["Arctis".into()],
            &["Arctis".into()]
        ));
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

    /// Opening at 48 kHz when the device can avoids resampling entirely (best quality, no work).
    #[test]
    fn prefers_internal_rate_when_supported() {
        use cpal::{SampleFormat::F32, SupportedBufferSize::Unknown};
        let default = cpal::SupportedStreamConfig::new(2, 44_100, Unknown, F32);
        let range = |ch, lo, hi| cpal::SupportedStreamConfigRange::new(ch, lo, hi, Unknown, F32);

        let picked = prefer_internal_rate(default, vec![range(2, 8_000, 192_000)]);
        assert_eq!(picked.sample_rate(), INTERNAL_RATE);
        assert_eq!(picked.channels(), 2, "keeps the default's channel count");

        let only_44k = prefer_internal_rate(default, vec![range(2, 44_100, 44_100)]);
        assert_eq!(only_44k, default, "can't do 48 kHz: keep the default");

        let other_channels = prefer_internal_rate(default, vec![range(6, 8_000, 192_000)]);
        assert_eq!(
            other_channels, default,
            "never trade channel layout for the rate"
        );
    }

    /// cpal 0.18 dropped `Device::name()`. Settings store names, so a device must keep the name
    /// 0.15 gave it: on Linux the ALSA PCM id (what `usable` filters on), not the description.
    /// Real devices: run explicitly (`--ignored`).
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn linux_device_names_stay_alsa_pcm_ids() {
        use cpal::traits::HostTrait;
        let host = cpal::default_host();
        let names: Vec<String> = host
            .output_devices()
            .into_iter()
            .flatten()
            .filter_map(|d| device_name(&d))
            .collect();
        assert!(!names.is_empty(), "no ALSA devices listed");
        for n in &names {
            assert!(!n.contains(' '), "a description, not a PCM id: {n:?}");
        }
        assert!(names.iter().any(|n| n == "default"), "{names:?}");
    }

    fn round_trip<T: cpal::SizedSample + cpal::FromSample<f32>>(src: &[f32]) -> Vec<f32>
    where
        f32: cpal::FromSample<T>,
    {
        let mut dev = vec![<T as cpal::Sample>::EQUILIBRIUM; src.len()];
        from_f32(src, &mut dev);
        let mut back = Vec::new();
        to_f32(&dev, &mut back);
        back
    }

    /// A clipped +1.0 must land on the format's top code, not one past it: dasp maps +1.0 to
    /// I24(8_388_608) / U24(16_777_216), which cpal's WASAPI path wraps into a full-scale negative
    /// spike. Other formats keep their existing full-scale values.
    #[test]
    fn full_scale_never_overflows_the_device_format() {
        let mut i24 = [<cpal::I24 as cpal::Sample>::EQUILIBRIUM; 2];
        from_f32(&[1.0, -1.0], &mut i24);
        assert_eq!(i24[0].inner(), 8_388_607);
        assert_eq!(i24[1].inner(), -8_388_608);
        let mut u24 = [<cpal::U24 as cpal::Sample>::EQUILIBRIUM; 2];
        from_f32(&[1.0, -1.0], &mut u24);
        assert_eq!(u24[0].inner(), 16_777_215);
        assert_eq!(u24[1].inner(), 0);

        let mut i16s = [0i16; 2];
        from_f32(&[1.0, -1.0], &mut i16s);
        assert_eq!(i16s, [i16::MAX, i16::MIN]);
        let mut i32s = [0i32; 2];
        from_f32(&[1.0, -1.0], &mut i32s);
        assert!(i32s[0] > i32::MAX - 512 && i32s[1] == i32::MIN, "{i32s:?}");
        let mut i8s = [0i8; 2];
        from_f32(&[1.0, -1.0], &mut i8s);
        assert_eq!(i8s, [i8::MAX, i8::MIN]);
        let mut f = [0f32; 1];
        from_f32(&[1.0], &mut f);
        assert!(f[0] > 0.99999 && f[0] < 1.0, "{f:?}");
    }

    /// cpal 0.18 ranks 24-bit formats above I16: a headset whose shared-mode format is 24-bit
    /// integer now goes through these conversions, which must be exact to 24-bit precision.
    #[test]
    fn twenty_four_bit_samples_round_trip() {
        let src = [-1.0f32, -0.5, 0.0, 0.5, 0.999];
        for (name, back) in [
            ("i24", round_trip::<cpal::I24>(&src)),
            ("u24", round_trip::<cpal::U24>(&src)),
        ] {
            assert_eq!(back.len(), src.len(), "{name}");
            for (a, b) in src.iter().zip(&back) {
                assert!((a - b).abs() < 1e-5, "{name}: {a} came back as {b}");
            }
        }
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
