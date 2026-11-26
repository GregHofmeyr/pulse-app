//! Audio devices: cpal capture/playback, APM (echo cancel / noise / AGC), sensitivity gate, watchdog.
//!
//! cpal streams are !Send, so they live on a dedicated audio thread; `AudioIo` is a handle to it.
//! Everything between the devices and LiveKit runs at `INTERNAL_RATE`; devices are resampled at the
//! edge, so a device switching rate mid-call (BT headset mode = 16 kHz) can't break pitch or speed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use livekit::webrtc::native::apm::AudioProcessingModule;
use serde::{Deserialize, Serialize};

use super::controls::Controls;
use super::denoise::NsLevel;
use super::echo::{EchoMode, Output, echo_on};
use super::meter::Meter;
use super::mixer::{Mixer, MixerInput};
use super::processor::RawTx;

pub const STALL_AFTER: Duration = Duration::from_secs(2);
pub const INTERNAL_RATE: u32 = 48_000;

/// A broken device can fail every callback; log its error at most this often.
const STREAM_ERROR_EVERY: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioConfig {
    /// Device names; `None` = system default.
    pub input: Option<String>,
    pub output: Option<String>,
    /// 50..=400 (%), applied after noise suppression, soft-limited — for quiet mics.
    pub input_gain_pct: u16,
    /// Manual mode: RMS threshold below which the mic is gated (0 = always open).
    pub sensitivity: f32,
    /// Echo cancellation: Auto (on for every output since #354), On or Off.
    #[serde(default)]
    pub echo_mode: EchoMode,
    #[serde(default)]
    pub noise_suppression: NsLevel,
    /// Gate on detected speech above the room's noise floor (else on `sensitivity`).
    #[serde(default = "default_true")]
    pub auto_sensitivity: bool,
    pub auto_gain: bool,
    /// "Even out voice volumes" (#355 (3)): level each remote voice towards one loudness.
    #[serde(default = "default_true")]
    pub even_out_voices: bool,
    /// "Voice clarity EQ" (#590): the mic's tone stage (`tone`) after noise suppression.
    #[serde(default = "default_true")]
    pub voice_clarity_eq: bool,
    /// The mic test's "Hear yourself as others do" (#659): its loopback through the receive side first (Opus
    /// round trip, the leveller). Off by default; only the mic test reads it.
    #[serde(default)]
    pub mic_test_as_others_hear: bool,
}

fn default_true() -> bool {
    true
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input: None,
            output: None,
            input_gain_pct: 100,
            sensitivity: 0.02,
            echo_mode: EchoMode::Auto,
            noise_suppression: NsLevel::default(),
            auto_sensitivity: true,
            auto_gain: false,
            even_out_voices: true,
            voice_clarity_eq: true,
            mic_test_as_others_hear: false,
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
    /// The tone stage (#590) on.
    pub tone: bool,
    pub version: u64,
}

/// Which suppression is actually running (it can differ from the chosen level), and why.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NsStatus {
    pub active: NsLevel,
    pub note: Option<String>,
}

/// Lock `m` even if a panic poisoned it. For the locks the audio callbacks, the mic processor and the call's meter
/// loop take: what they guard is plain values (queues, meters, the APM), and one panic must not turn every later lock
/// into another panic, least of all on the real-time output thread (speakers gone for the rest of the call).
pub(crate) fn relock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn input_gain(pct: u16) -> f32 {
    pct.clamp(50, 400) as f32 / 100.0
}

#[derive(Clone, Debug, Serialize)]
pub struct DeviceInfo {
    pub name: String,
    pub is_default: bool,
    /// An input that already cleans up its own sound, and what does it (#588): Voice settings suggests Light
    /// (#601) over Strong for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processed_by: Option<&'static str>,
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

/// What an output buffer plays, as mono: the average of each frame's front pair, which is all the mixer writes
/// ([`super::mixer::fill_front`]). Averaging a 5.1 frame's silent rear channels in would make the echo reference
/// (and the speaker meter) three times too quiet.
pub fn front_mono(data: &[f32], channels: usize) -> impl Iterator<Item = f32> + '_ {
    data.chunks(channels.max(1)).map(|f| {
        let front = &f[..f.len().min(2)];
        front.iter().sum::<f32>() / front.len().max(1) as f32
    })
}

fn to_i16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

/// Where a device comes from, for naming it. On Linux cpal talks to the sound server over the
/// PulseAudio protocol (PipeWire serves it too), which knows real devices; ALSA is only the fallback
/// when no sound server answers, and lists plumbing names (jack, pipewire, hdmi:CARD=…).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Pulse and ALSA exist on Linux only
enum Backend {
    Pulse,
    Alsa,
    /// WASAPI and the rest: friendly names.
    Other,
}

fn backend(d: &cpal::Device) -> Backend {
    match d.id().map(|id| id.host()) {
        #[cfg(target_os = "linux")]
        Ok(cpal::HostId::PulseAudio) => Backend::Pulse,
        #[cfg(target_os = "linux")]
        Ok(cpal::HostId::Alsa) => Backend::Alsa,
        _ => Backend::Other,
    }
}

/// The name settings store and lists show. ALSA keeps the PCM id ("default", what cpal 0.15
/// returned); everything else the friendly name, falling back to the id.
fn display_name(backend: Backend, id: Option<&str>, description: Option<&str>) -> Option<String> {
    match backend {
        Backend::Alsa => id,
        Backend::Pulse | Backend::Other => description.or(id),
    }
    .map(str::to_owned)
}

/// Offered in the device lists: on ALSA only the system default (the OS picks the device); on the
/// sound server everything but the monitors of outputs, which aren't microphones.
fn offered(backend: Backend, id: &str) -> bool {
    match backend {
        Backend::Alsa => id == "default",
        Backend::Pulse => !id.ends_with(".monitor"),
        Backend::Other => true,
    }
}

/// A device's name as settings and lists use it (see `display_name`).
pub fn device_name(d: &cpal::Device) -> Option<String> {
    let id = d.id().ok();
    let desc = d.description().ok();
    display_name(
        backend(d),
        id.as_ref().map(|i| i.id()),
        desc.as_ref().map(|d| d.name()),
    )
}

fn usable(d: &cpal::Device) -> bool {
    d.id().is_ok_and(|id| offered(backend(d), id.id()))
}

/// The sound server's default buffering is up to 2 s; ask for 10 ms callbacks instead (ALSA and
/// WASAPI already run near that by default).
fn stream_config(dev: &cpal::Device, cfg: &cpal::SupportedStreamConfig) -> cpal::StreamConfig {
    let mut c = cfg.config();
    if backend(dev) == Backend::Pulse {
        c.buffer_size = cpal::BufferSize::Fixed(c.sample_rate / 100);
    }
    c
}

fn list(
    names: Option<impl Iterator<Item = cpal::Device>>,
    default: Option<String>,
    inputs: bool,
) -> Vec<DeviceInfo> {
    names
        .map(|it| {
            it.filter(usable)
                .filter_map(|d| device_name(&d))
                .map(|name| DeviceInfo {
                    is_default: Some(&name) == default.as_ref(),
                    processed_by: inputs
                        .then(|| super::virtualmic::processed_by(&name))
                        .flatten(),
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
        true,
    )
}

pub fn list_outputs() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    list(
        host.output_devices().ok(),
        host.default_output_device().and_then(|d| device_name(&d)),
        false,
    )
}

/// State shared between the audio callbacks and the rest of the app.
pub struct Shared {
    /// The same Arc the VoiceManager holds: one source of truth for mute/deafen (no stale copies).
    pub controls: Arc<Mutex<Controls>>,
    /// Speaker side of the mixer: only the output callback takes it (`try_lock`; the only other
    /// taker is a second output stream during a reopen).
    pub mixer: Mutex<Mixer>,
    /// Receive side: network tasks push here, lock-free towards the speakers.
    pub mix_in: Arc<MixerInput>,
    /// Last mute/deafen state the speaker callback saw (used when a toggle holds the lock).
    playout_on: std::sync::atomic::AtomicBool,
    /// Speaker callbacks that found the mixer busy and played silence (diagnostics).
    mix_busy: AtomicU64,
    pub(crate) apm: Mutex<AudioProcessingModule>,
    /// What the APM is built from, and the (echo, agc) it was built with: rebuilt only when that
    /// changes (rebuilding resets AEC).
    apm_cfg: Mutex<ApmCfg>,
    /// What the mic processor should run (it polls `version`).
    pub proc_cfg: Mutex<ProcCfg>,
    /// What the speakers played, waiting for the processor to feed the echo canceller.
    echo_ref: super::rings::EchoTap,
    /// The processor's buffer for draining `echo_ref` (reused, not allocated per block).
    echo_buf: Mutex<Vec<i16>>,
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
    /// Levels and clipping stage by stage, for the `voice: mic` line (#583; atomics: safe in the audio callbacks).
    pub mic_levels: super::miclevels::MicLevels,
    /// Where the processor's frames go: the current room's mic track (`None` between rooms).
    pub(crate) mic_sink: Mutex<Option<Box<dyn super::processor::FrameSink>>>,
    /// Mic audio waiting to be processed (ms), worst per stats window: the send-side delay we own.
    pub mic_queue: super::backlog::QueueGauge,
    /// "No audio from your mic": fed by the processor, reported by the call's meter loop.
    pub mic_silence: Mutex<super::micsilence::MicSilence>,
    /// "You're muted" (The Doos): fed by the processor, reported by the call's meter loop.
    pub muted_talk: Mutex<super::mutedtalk::MutedTalk>,
    /// The "you're muted" sound (#252): the same judgement on its own, shorter clock.
    pub muted_cue: Mutex<super::mutedtalk::MutedCue>,
    /// Strong (low delay)'s model store (`crate::models`, #77).
    low_delay_model: Option<Arc<crate::models::ModelStore>>,
}

impl Shared {
    pub fn new(
        cfg: &AudioConfig,
        controls: Arc<Mutex<Controls>>,
        peer_buffer_ms: u32,
    ) -> Arc<Self> {
        let (mix_in, mixer) = super::mixer::mixer(INTERNAL_RATE, peer_buffer_ms);
        mix_in.set_level_voices(cfg.even_out_voices);
        let playout_on = controls.lock().unwrap().playout_on();
        Arc::new(Self {
            controls,
            mixer: Mutex::new(mixer),
            mix_in: Arc::new(mix_in),
            playout_on: std::sync::atomic::AtomicBool::new(playout_on),
            mix_busy: AtomicU64::new(0),
            // no output is open yet: Auto starts on, and the output's opening decides
            apm: Mutex::new(AudioProcessingModule::new(
                echo_on(cfg.echo_mode, Output::Unknown),
                cfg.auto_gain,
                APM_HIGH_PASS,
                false,
            )),
            apm_cfg: Mutex::new(ApmCfg {
                mode: cfg.echo_mode,
                output: Output::Unknown,
                device: String::new(),
                agc: cfg.auto_gain,
                built: (echo_on(cfg.echo_mode, Output::Unknown), cfg.auto_gain),
                logged: None,
                status: (
                    EchoStatus {
                        mode: cfg.echo_mode,
                        running: echo_on(cfg.echo_mode, Output::Unknown),
                        output: Output::Unknown,
                    },
                    true,
                ),
            }),
            proc_cfg: Mutex::new(ProcCfg {
                level: cfg.noise_suppression,
                auto: cfg.auto_sensitivity,
                threshold: cfg.sensitivity,
                gain: input_gain(cfg.input_gain_pct),
                tone: cfg.voice_clarity_eq,
                version: 0,
            }),
            ns_status: Mutex::new((None, false)),
            mic_silence: Mutex::default(),
            muted_talk: Mutex::default(),
            muted_cue: Mutex::default(),
            echo_ref: Default::default(),
            echo_buf: Mutex::new(Vec::with_capacity(9_600)),
            gate_open: Default::default(),
            watchdog: Watchdog::new(Instant::now()),
            apm_warned: Default::default(),
            mic_meter: Mutex::new(Meter::default()),
            spk_meter: Mutex::new(Meter::default()),
            sent_meter: Mutex::new(Meter::default()),
            stats: Default::default(),
            mic_levels: Default::default(),
            mic_sink: Mutex::new(None),
            mic_queue: Default::default(),
            low_delay_model: crate::models::low_delay_store(),
        })
    }

    /// Where Strong (low delay)'s downloaded model is (#77); `None` = it can't be had (tests, or before setup).
    pub fn low_delay_model(&self) -> Option<Arc<crate::models::ModelStore>> {
        self.low_delay_model.clone()
    }

    /// Speaker callbacks since the last call that found the mixer busy (played silence).
    pub fn take_mix_busy(&self) -> u64 {
        self.mix_busy.swap(0, Ordering::Relaxed)
    }

    pub fn set_mic_sink(&self, sink: Option<Box<dyn super::processor::FrameSink>>) {
        let old = std::mem::replace(&mut *relock(&self.mic_sink), sink);
        // dropped after the lock: the processor's next frame doesn't wait on its teardown
        drop(old);
    }

    /// (mic rms, speaker rms) since the last call.
    pub fn take_sent_level(&self) -> f32 {
        relock(&self.sent_meter).take().0
    }

    pub fn take_levels(&self) -> (f32, f32) {
        (
            relock(&self.mic_meter).take().0,
            relock(&self.spk_meter).take().0,
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
        // peaks, clipping and L/R correlation of what the device sent, before the downmix
        self.mic_levels.record_input(data, channels);
        let chunk = (rate / 100) as usize;
        for v in downmix(data, channels) {
            pending.push(v);
            if pending.len() == chunk {
                // lock- and allocation-free: a recycled buffer takes the full one's place (#353)
                raw_tx.hand_over(rate, pending, Instant::now());
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
        // Real-time: never wait on a lock another thread may hold (a preempted holder costs a
        // scheduler quantum, an underrun click).
        let on = match self.controls.try_lock() {
            Ok(c) => {
                let on = c.playout_on();
                self.playout_on.store(on, Ordering::Relaxed);
                on
            }
            Err(_) => self.playout_on.load(Ordering::Relaxed), // a toggle in progress
        };
        let master = if on { 1.0 } else { 0.0 };
        let ch = channels.max(1);
        let frames = out.len() / ch;
        let mixer = match self.mixer.try_lock() {
            Ok(mixer) => Some(mixer),
            // A panic mid-mix poisoned it. Its state is plain queues: play on (and say so) rather
            // than play silence for the rest of the call.
            Err(std::sync::TryLockError::Poisoned(e)) => {
                self.mixer.clear_poison();
                tracing::error!("voice: the speaker mix panicked; carrying on");
                Some(e.into_inner())
            }
            Err(std::sync::TryLockError::WouldBlock) => None,
        };
        match mixer {
            Some(mut mixer) => {
                if rate == INTERNAL_RATE || frames == 0 {
                    mixer.mix_into(out, channels, master);
                } else {
                    scratch.resize(frames, 0.0);
                    playout.fill(scratch, |buf| mixer.mix_into(buf, 1, master));
                    for (frame, v) in out.chunks_mut(ch).zip(scratch.iter()) {
                        super::mixer::fill_front(frame, *v);
                    }
                }
            }
            None => {
                out.fill(0.0);
                self.mix_busy.fetch_add(1, Ordering::Relaxed);
            }
        }
        // Hand what we played to the echo canceller via a lock-free ring the mic processor drains
        // (#353): never take the APM lock, or wait on the processor, here (this callback is
        // real-time; the processor can hold its locks for ms). The meter is cosmetic: skip it
        // rather than wait for the levels task.
        let mut meter = self.spk_meter.try_lock().ok();
        let mut echo = self.echo_ref.writer(rate);
        for v in front_mono(out, channels) {
            if let Some(m) = meter.as_mut() {
                m.add(v);
            }
            if let Some(e) = echo.as_mut() {
                e.push(to_i16(v));
            }
        }
    }

    /// Feed the echo canceller everything played since the last call (whole 10 ms chunks). Called
    /// by the mic processor before each capture block, so render and capture stay in order.
    pub(crate) fn feed_echo_reference(&self) {
        let mut played = relock(&self.echo_buf);
        let rate = self.echo_ref.drain_into(&mut played);
        if played.is_empty() {
            return;
        }
        let chunk = (rate / 100) as usize;
        let mut apm = relock(&self.apm);
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
        let mut st = relock(&self.ns_status);
        if st.0.as_ref() != Some(&s) {
            *st = (Some(s), true);
        }
    }

    /// The suppression running now, if the processor has said (bug reports, #605).
    pub fn ns_status(&self) -> Option<NsStatus> {
        relock(&self.ns_status).0.clone()
    }

    /// What echo cancellation decided last (bug reports, #605).
    pub fn echo_status(&self) -> EchoStatus {
        relock(&self.apm_cfg).status.0
    }

    pub fn take_ns_status_change(&self) -> Option<NsStatus> {
        let mut st = relock(&self.ns_status);
        if !st.1 {
            return None;
        }
        st.1 = false;
        st.0.clone()
    }

    /// The processor's per-block report: mic meter, own speaking ring, gate indicator.
    pub fn add_mic_level(&self, level: f32, sent: bool) {
        relock(&self.mic_meter).add(level);
        relock(&self.sent_meter).add(if sent { level } else { 0.0 });
        self.gate_open.store(sent, Ordering::Relaxed);
    }

    pub fn apply_config(&self, cfg: &AudioConfig) {
        // A level or gain change was invisible in the logs, so a report couldn't say what the mic ran at (#586).
        tracing::info!(
            ns = ?cfg.noise_suppression,
            gain_pct = cfg.input_gain_pct,
            auto_gain = cfg.auto_gain,
            auto_sensitivity = cfg.auto_sensitivity,
            sensitivity = cfg.sensitivity,
            clarity_eq = cfg.voice_clarity_eq,
            "voice: mic settings"
        );
        {
            let mut p = relock(&self.proc_cfg);
            *p = ProcCfg {
                level: cfg.noise_suppression,
                auto: cfg.auto_sensitivity,
                threshold: cfg.sensitivity,
                gain: input_gain(cfg.input_gain_pct),
                tone: cfg.voice_clarity_eq,
                version: p.version + 1,
            };
        }
        let mut a = relock(&self.apm_cfg);
        a.mode = cfg.echo_mode;
        a.agc = cfg.auto_gain;
        self.rebuild_apm(&mut a);
    }

    /// The output device just opened (`device`, and what the system says it is): logged and reported; Auto no longer acts on it (#354).
    pub fn set_output(&self, device: &str, output: Output) {
        let mut a = relock(&self.apm_cfg);
        a.output = output;
        a.device = device.to_owned();
        self.rebuild_apm(&mut a);
    }

    /// What echo cancellation decided, if that changed since the last call (the first call always
    /// has it): for the settings page (#172).
    pub fn take_echo_status_change(&self) -> Option<EchoStatus> {
        let mut a = relock(&self.apm_cfg);
        let (status, changed) = &mut a.status;
        std::mem::take(changed).then_some(*status)
    }

    /// Whether echo cancellation runs now.
    #[cfg(test)]
    pub fn echo_running(&self) -> bool {
        relock(&self.apm_cfg).built.0
    }

    fn rebuild_apm(&self, a: &mut ApmCfg) {
        let echo = echo_on(a.mode, a.output);
        // the decision, once per change of what it's made from
        let decision = (a.mode, a.output, a.device.clone());
        if a.logged.as_ref() != Some(&decision) {
            tracing::info!(mode = ?a.mode, output = ?a.output, device = %a.device, echo, "voice: echo cancellation");
            a.logged = Some(decision);
        }
        let status = EchoStatus {
            mode: a.mode,
            running: echo,
            output: a.output,
        };
        if a.status.0 != status {
            a.status = (status, true);
        }
        let wanted = (echo, a.agc);
        if a.built != wanted {
            // WebRTC's own noise suppression stays off: Off means none, and suppressors never stack.
            *relock(&self.apm) = AudioProcessingModule::new(echo, a.agc, APM_HIGH_PASS, false);
            a.built = wanted;
        }
    }
}

/// The APM's high-pass: the mic chain's only low cut (#660). Measured at −3 dB at 58 Hz and steep (−44 dB at
/// 40 Hz), so it sits below a deep voice. `tone.rs` has no high-pass of its own any more, so turning this off
/// would let rumble and plosives through to the gain and limiter whenever echo cancellation is off.
/// `tone::measured_low_cut` fails if it is.
pub(crate) const APM_HIGH_PASS: bool = true;

/// What the echo canceller and AGC are built from (see [`Shared::apm_cfg`]).
struct ApmCfg {
    mode: EchoMode,
    output: Output,
    device: String,
    agc: bool,
    /// (echo, agc) the APM was built with.
    built: (bool, bool),
    /// The last decision logged: (mode, output, device).
    logged: Option<(EchoMode, Output, String)>,
    /// The decision for the settings page (#172), and whether it changed since it was last sent.
    status: (EchoStatus, bool),
}

/// What echo cancellation is doing and why, for the settings page's line under Auto (#172).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EchoStatus {
    pub mode: EchoMode,
    pub running: bool,
    pub output: Output,
}

/// Handle to the audio thread. Dropping it stops capture and playback.
pub struct AudioIo {
    input: Option<Half>,
    output: Option<Half>,
    null: Option<tokio::task::JoinHandle<()>>,
    /// Why a side isn't open (its last open's error), for the call's "No mic" chip (#351).
    errors: DeviceGaps,
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
        Self::spawn_cancellable(name, open_timeout, move |_| build())
    }

    /// [`Self::spawn`], with `build` told whether it was given up on: set once the wait timed out, so a build that
    /// finishes late can skip its side effects (starting the stream, telling echo cancellation about the output)
    /// before it's closed.
    pub fn spawn_cancellable<T: 'static>(
        name: &str,
        open_timeout: Duration,
        build: impl FnOnce(&std::sync::atomic::AtomicBool) -> anyhow::Result<(T, R)> + Send + 'static,
    ) -> anyhow::Result<Self> {
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let gave_up = cancelled.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<anyhow::Result<R>>();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                match build(&cancelled) {
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
                gave_up.store(true, Ordering::SeqCst);
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
    /// What it opened with besides the rate, for bug reports (#605).
    channels: u16,
    format: String,
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
    if input {
        // a different (or reopened) mic gets judged afresh
        shared.mic_silence.lock().unwrap().restart();
    }
    Parked::spawn_cancellable(
        if input { "pulse-audio-in" } else { "pulse-audio-out" },
        Duration::from_secs(5),
        move |cancelled| {
            let dev = find_device(input, name.as_deref())?;
            let cfg = if input {
                prefer_internal_rate(dev.default_input_config()?, dev.supported_input_configs()?)
            } else {
                prefer_internal_rate(dev.default_output_config()?, dev.supported_output_configs()?)
            };
            let (rate, ch) = (cfg.sample_rate(), cfg.channels() as usize);
            let decides = shared.clone();
            let stream = if input {
                let mut pending: Vec<f32> = Vec::with_capacity((rate / 100) as usize);
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
            // given up on while it was opening (a slow Bluetooth headset): don't start it, and don't let a device
            // nobody uses decide echo cancellation. Dropped here, closed.
            if cancelled.load(Ordering::SeqCst) {
                return Err(anyhow!("opened too late"));
            }
            stream.play()?;
            let name = device_name(&dev).unwrap_or_default();
            tracing::info!(device = %name, rate, ch, fmt = ?cfg.sample_format(), "audio {side} opened");
            if input && let Some(by) = super::virtualmic::processed_by(&name) {
                // once per open (#588): a report then says the mic was already cleaned before Pulse's suppression
                tracing::info!(device = %name, processed_by = by, "voice: mic already processes its sound");
            }
            if !input && !cancelled.load(Ordering::SeqCst) {
                // Auto echo cancellation follows the output (#76): every open, so a switch re-decides
                decides.set_output(&name, super::echo::output_of(&dev));
            }
            let generation = OPENS.fetch_add(1, Ordering::Relaxed) + 1;
            Ok((
                stream,
                HalfInfo {
                    rate,
                    name,
                    channels: ch as u16,
                    format: format!("{:?}", cfg.sample_format()).to_lowercase(),
                    generation,
                },
            ))
        },
    )
    .map_err(|e| anyhow!("audio {side}: {e}"))
}

/// A device as it opened, for bug reports (#605): its name and the format it runs at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenedFormat {
    pub device: String,
    pub rate: u32,
    pub channels: u16,
    pub sample_format: String,
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
    moved_sides(cfg, opened, defaults, Sides::default()).any()
}

/// Which sides `default_moved` applies to. `missing`: sides that aren't open at all (#351); one
/// on the system default moves to whatever default there is now (#362), rather than waiting for
/// the gap retry's next go.
pub fn moved_sides(
    cfg: &AudioConfig,
    opened: &OpenedDevices,
    defaults: (Option<String>, Option<String>),
    missing: Sides,
) -> Sides {
    // an open side's empty name couldn't be read: unknown, never "different"
    let moved = |chosen: &Option<String>, now: Option<String>, open: &str, missing: bool| {
        chosen.is_none() && now.is_some_and(|n| missing || (!open.is_empty() && n != open))
    };
    Sides {
        input: moved(&cfg.input, defaults.0, &opened.input, missing.input),
        output: moved(&cfg.output, defaults.1, &opened.output, missing.output),
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

    /// These sides without `o`'s.
    pub fn minus(self, o: Self) -> Self {
        Self {
            input: self.input && !o.input,
            output: self.output && !o.output,
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

/// The sides of a call that aren't open, each with why (#351): the UI's "No mic" chip.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DeviceGaps {
    pub input: Option<String>,
    pub output: Option<String>,
}

impl DeviceGaps {
    pub fn sides(&self) -> Sides {
        Sides {
            input: self.input.is_some(),
            output: self.output.is_some(),
        }
    }
}

/// An open's error as the chip's tooltip says it (without our own "audio input: " prefix).
fn gap_reason(e: &anyhow::Error) -> String {
    let s = e.to_string();
    s.strip_prefix("audio input: ")
        .or_else(|| s.strip_prefix("audio output: "))
        .unwrap_or(&s)
        .to_string()
}

/// When to try a missing side again (#351): a mic another app holds, or privacy off, comes back
/// whenever the user fixes it. Soon at first, then backing off (each try can block up to 5 s).
#[derive(Default)]
pub struct GapRetry {
    tries: usize,
    next: Option<Instant>,
}

impl GapRetry {
    const BACKOFF: [Duration; 4] = [
        Duration::from_secs(2),
        Duration::from_secs(5),
        Duration::from_secs(10),
        Duration::from_secs(30),
    ];

    /// Whether to try now, given whether anything is `missing`.
    pub fn due(&mut self, missing: bool, now: Instant) -> bool {
        if !missing {
            *self = Self::default();
            return false;
        }
        let next = *self.next.get_or_insert(now + Self::BACKOFF[0]);
        if now < next {
            return false;
        }
        self.tries += 1;
        self.next = Some(now + Self::BACKOFF[self.tries.min(Self::BACKOFF.len() - 1)]);
        true
    }
}

/// When the UI must hear which sides are missing (#362): whenever that changes, and once more
/// when a join asks for it (`owed`), so a join that found the devices mid-reopen still gets told
/// though nothing changed since the last report.
#[derive(Default)]
pub struct GapNotice {
    last: Option<DeviceGaps>,
}

impl GapNotice {
    /// What to send, given the gaps now (`None` while a reopen has the devices out: nothing to
    /// say yet, and anything owed stays owed).
    pub fn check(&mut self, now: Option<&DeviceGaps>, owed: bool) -> Option<DeviceGaps> {
        let now = now?;
        if !owed && self.last.as_ref() == Some(now) {
            return None;
        }
        self.last = Some(now.clone());
        Some(now.clone())
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
            if routine_stream_error(e.kind()) {
                tracing::info!(error = %e, repeated, "{what} stream error");
            } else {
                tracing::warn!(error = %e, repeated, "{what} stream error");
            }
        }
    }
}

/// Stream errors that are part of normal life, logged at info rather than warn so a bug report
/// doesn't read them as a regression: an under/overrun (a glitch the stream plays on through) and
/// an invalidated stream (WASAPI's word for "the default device changed"; device following and the
/// watchdog reopen it).
fn routine_stream_error(kind: cpal::ErrorKind) -> bool {
    matches!(
        kind,
        cpal::ErrorKind::Xrun | cpal::ErrorKind::StreamInvalidated
    )
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
    let c = stream_config(dev, cfg);
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
        // cpal 0.18 reads WASAPI 24-bit as a 32-bit container: a packed 3-byte format would be
        // misread. Shared-mode mix formats are float in practice (upstream's to fix if not).
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
    let c = stream_config(dev, cfg);
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
    /// Open both sides; a side that fails is left out (`gaps` says why), never the whole call.
    pub fn start(cfg: &AudioConfig, shared: Arc<Shared>, raw_tx: RawTx) -> Self {
        // both directions at once: each waits on its own device thread anyway
        let (input, output) = std::thread::scope(|s| {
            let (sh, tx, name) = (shared.clone(), raw_tx.clone(), cfg.input.clone());
            let input = s.spawn(move || open_half(true, name, sh, tx));
            let output = open_half(false, cfg.output.clone(), shared.clone(), raw_tx.clone());
            let input = input
                .join()
                .unwrap_or_else(|_| Err(anyhow!("audio input: opening thread panicked")));
            (input, output)
        });
        Self::from_halves(input, output, shared, raw_tx)
    }

    /// Whatever opened (#351): no mic still joins (listen-only), no speakers still joins (nothing
    /// plays). A missing side is reported (`gaps`) and tried again later (the pipeline's watchdog).
    fn from_halves(
        input: anyhow::Result<Half>,
        output: anyhow::Result<Half>,
        shared: Arc<Shared>,
        raw_tx: RawTx,
    ) -> Self {
        let mut errors = DeviceGaps::default();
        let keep = |r: anyhow::Result<Half>, err: &mut Option<String>, side: &str| match r {
            Ok(h) => Some(h),
            Err(e) => {
                tracing::warn!(error = %e, side, "voice: a device didn't open; joining without it");
                *err = Some(gap_reason(&e));
                None
            }
        };
        let input = keep(input, &mut errors.input, "input");
        let output = keep(output, &mut errors.output, "output");
        Self {
            input,
            output,
            null: None,
            errors,
            shared,
            raw_tx,
        }
    }

    /// The sides that aren't open, and why (nothing for test audio).
    pub fn gaps(&self) -> DeviceGaps {
        if self.null.is_some() {
            return DeviceGaps::default();
        }
        let why = |h: &Option<Half>, e: &Option<String>| {
            h.is_none()
                .then(|| e.clone().unwrap_or_else(|| "not open".into()))
        };
        DeviceGaps {
            input: why(&self.input, &self.errors.input),
            output: why(&self.output, &self.errors.output),
        }
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
            let error = if input {
                &mut self.errors.input
            } else {
                &mut self.errors.output
            };
            match open_half(input, name, self.shared.clone(), self.raw_tx.clone()) {
                Ok(h) => {
                    *slot = Some(h);
                    *error = None;
                }
                Err(e) => {
                    *error = Some(gap_reason(&e));
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

    /// Stop both sides and wait (up to 1 s each) until they're really closed. Dropping only asks;
    /// use this when the same devices are about to be opened again.
    pub fn close(mut self) {
        for half in [self.input.take(), self.output.take()]
            .into_iter()
            .flatten()
        {
            half.close();
        }
    }

    /// Nothing open yet (`reopen` fills in the sides).
    pub fn empty(shared: Arc<Shared>, raw_tx: RawTx) -> Self {
        Self {
            input: None,
            output: None,
            null: None,
            errors: DeviceGaps::default(),
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
                let mut block = vec![0f32; chunk];
                tx.hand_over(rate, &mut block, Instant::now());
            }
        });
        Self {
            input: None,
            output: None,
            null: Some(h),
            errors: DeviceGaps::default(),
            shared,
            raw_tx,
        }
    }

    /// Tests: an input whose close takes `delay` (a driver taking its time); `closed` is set once
    /// it's really gone.
    #[cfg(test)]
    pub(crate) fn slow_to_close(
        shared: Arc<Shared>,
        raw_tx: RawTx,
        delay: Duration,
        closed: Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        struct SlowStream(Duration, Arc<std::sync::atomic::AtomicBool>);
        impl Drop for SlowStream {
            fn drop(&mut self) {
                std::thread::sleep(self.0);
                self.1.store(true, Ordering::SeqCst);
            }
        }
        let half = Parked::spawn("test-half", Duration::from_secs(1), move || {
            Ok((
                SlowStream(delay, closed),
                HalfInfo {
                    rate: 48_000,
                    name: "test".into(),
                    channels: 1,
                    format: "f32".into(),
                    generation: 0,
                },
            ))
        })
        .expect("a test half opens");
        Self {
            input: Some(half),
            output: None,
            null: None,
            errors: DeviceGaps::default(),
            shared,
            raw_tx,
        }
    }

    /// The formats the open sides opened with (input, output; `None` for a side that isn't open).
    pub fn formats(&self) -> (Option<OpenedFormat>, Option<OpenedFormat>) {
        let f = |h: &Option<Half>| {
            h.as_ref().map(|h| OpenedFormat {
                device: h.value.name.clone(),
                rate: h.value.rate,
                channels: h.value.channels,
                sample_format: h.value.format.clone(),
            })
        };
        (f(&self.input), f(&self.output))
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

    #[cfg(test)]
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
    use crate::voice::mixer::SourceKind;

    /// The speaker callback is real-time: it must never wait while the mic processor holds the
    /// echo canceller (a preempted processor would otherwise stutter playback).
    #[test]
    fn playback_never_waits_for_the_echo_canceller() {
        let s = Shared::new(&AudioConfig::default(), Default::default(), 200);
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

    /// The speaker callback is real-time: a receive task preempted mid-push, a mute toggle in
    /// progress, or the levels task reading the meter must never make it wait.
    #[test]
    fn playback_never_waits_on_the_network_side() {
        let s = Shared::new(&AudioConfig::default(), Default::default(), 200);
        s.mix_in
            .add_source(s.mix_in.generation(), "a", "a", SourceKind::Voice);
        s.mix_in.push("a", &[100; 480]);
        let held_sources = s.mix_in.sources.lock().unwrap();
        let held_controls = s.controls.lock().unwrap();
        let held_meter = s.spk_meter.lock().unwrap();
        let s2 = s.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut out = vec![0f32; 960]; // 10 ms stereo at 48 kHz
            let mut playout = super::super::playout::Playout::new(48_000);
            s2.on_output(&mut out, 2, 48_000, &mut Vec::new(), &mut playout);
            let _ = tx.send(out);
        });
        let out = rx.recv_timeout(Duration::from_millis(500));
        drop((held_sources, held_controls, held_meter));
        let out = out.expect("the speaker callback blocked on a lock the network side holds");
        // and it didn't just skip the mix: the peer's audio played (unmuted, as last seen)
        let want = 100.0 / i16::MAX as f32;
        assert!(
            out.iter().all(|v| (v - want).abs() < 1e-4),
            "{:?}",
            &out[..4]
        );
    }

    /// A panic while mixing poisons the mixer's lock: the speakers must not then play silence
    /// for the rest of the call.
    #[test]
    fn a_poisoned_mixer_still_plays() {
        let s = Shared::new(&AudioConfig::default(), Default::default(), 200);
        s.mix_in
            .add_source(s.mix_in.generation(), "a", "a", SourceKind::Voice);
        let s2 = s.clone();
        let _ = std::thread::spawn(move || {
            let _held = s2.mixer.lock().unwrap();
            panic!("a bug mid-mix (expected in this test)");
        })
        .join();
        assert!(s.mixer.is_poisoned());
        let mut playout = super::super::playout::Playout::new(48_000);
        for _ in 0..2 {
            s.mix_in.push("a", &[100; 480]);
            let mut out = vec![0f32; 480];
            s.on_output(&mut out, 1, 48_000, &mut Vec::new(), &mut playout);
            assert!(out[0] > 0.0, "silent after the panic");
        }
        assert_eq!(s.take_mix_busy(), 0);
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
        let s = Shared::new(&AudioConfig::default(), Default::default(), 200);
        let mut playout = super::super::playout::Playout::new(48_000);
        let mut out = vec![0.1f32; 1200]; // 25 ms
        s.on_output(&mut out, 1, 48_000, &mut Vec::new(), &mut playout);
        s.feed_echo_reference();
        assert_eq!(
            s.echo_ref.queued(),
            240,
            "2 chunks fed, 5 ms waits for more"
        );
    }

    #[test]
    fn echo_reference_queue_is_capped_at_200ms() {
        let s = Shared::new(&AudioConfig::default(), Default::default(), 200);
        play(&s, 400); // nothing drains it (processor stopped)
        let mut out = Vec::new();
        s.echo_ref.drain_into(&mut out);
        assert_eq!(out.len(), 9600);
    }

    /// #703: the pre-0.2.17 on/off toggles are no longer read; a stored config still holding them parses and they
    /// change nothing (no `deny_unknown_fields` on `AudioConfig`).
    #[test]
    fn stale_pre_0_2_17_toggles_are_ignored() {
        let old = r#"{"input":null,"output":null,"input_gain_pct":100,"sensitivity":0.02,
                      "echo_cancel":false,"noise_suppress":false,"auto_gain":false}"#;
        let cfg: AudioConfig = serde_json::from_str(old).unwrap();
        assert_eq!(cfg.noise_suppression, NsLevel::default());
        assert_eq!(cfg.echo_mode, EchoMode::Auto);
        let saved = serde_json::to_string(&cfg).unwrap();
        assert!(!saved.contains("noise_suppress\""), "{saved}");
        assert!(!saved.contains("echo_cancel"), "{saved}");
        // a saved level and mode win over the stale toggles
        let both = r#"{"input":null,"output":null,"input_gain_pct":100,"sensitivity":0.02,"echo_mode":"on",
                       "noise_suppression":"light","noise_suppress":false,"echo_cancel":false,"auto_gain":false}"#;
        let cfg: AudioConfig = serde_json::from_str(both).unwrap();
        assert_eq!(
            (cfg.noise_suppression, cfg.echo_mode),
            (NsLevel::Light, EchoMode::On)
        );
        assert_eq!(AudioConfig::default().echo_mode, EchoMode::Auto);
    }

    /// #354: Auto stays on through every output, headphones included; only Off turns it off.
    #[test]
    fn auto_echo_stays_on_whatever_the_output_device_says() {
        let s = Shared::new(&AudioConfig::default(), Default::default(), 200);
        assert!(s.echo_running(), "no output yet: on");
        s.set_output("Headset", Output::Headphones);
        assert!(
            s.echo_running(),
            "a headphone jack says this with speakers in it"
        );
        s.set_output("Speakers", Output::Other);
        assert!(s.echo_running());
        s.set_output("Something", Output::Unknown);
        assert!(s.echo_running());
        s.set_output("Headset", Output::Headphones);
        // On and Off override the device; back to Auto is on again
        let mut cfg = AudioConfig {
            echo_mode: EchoMode::On,
            ..AudioConfig::default()
        };
        s.apply_config(&cfg);
        assert!(s.echo_running());
        cfg.echo_mode = EchoMode::Off;
        s.set_output("Speakers", Output::Other);
        s.apply_config(&cfg);
        assert!(!s.echo_running());
        cfg.echo_mode = EchoMode::Auto;
        s.apply_config(&cfg);
        assert!(s.echo_running());
    }

    /// What Auto decided goes to the settings page (#172): once per change of the decision.
    #[test]
    fn echo_decision_is_reported_once_per_change() {
        let s = Shared::new(&AudioConfig::default(), Default::default(), 200);
        let st = |running, output| EchoStatus {
            mode: EchoMode::Auto,
            running,
            output,
        };
        assert_eq!(
            s.take_echo_status_change(),
            Some(st(true, Output::Unknown)),
            "the first decision, before any output opens"
        );
        assert_eq!(s.take_echo_status_change(), None);
        s.set_output("Headset", Output::Headphones);
        assert_eq!(
            s.take_echo_status_change(),
            Some(st(true, Output::Headphones)),
            "#354: still on, but the page hears what the output says"
        );
        s.set_output("Other headset", Output::Headphones);
        assert_eq!(
            s.take_echo_status_change(),
            None,
            "same decision on another device: nothing new to say"
        );
        s.set_output("Speakers", Output::Other);
        assert_eq!(s.take_echo_status_change(), Some(st(true, Output::Other)));
        s.apply_config(&AudioConfig {
            echo_mode: EchoMode::Off,
            ..AudioConfig::default()
        });
        assert_eq!(
            s.take_echo_status_change(),
            Some(EchoStatus {
                mode: EchoMode::Off,
                running: false,
                output: Output::Other
            })
        );
    }

    /// #77, #601: saved levels read back unchanged, the newer ones (low delay, Light) included, and nothing else in the config moves.
    #[test]
    fn saved_noise_levels_read_back_unchanged() {
        for (name, level) in [
            ("off", NsLevel::Off),
            ("light", NsLevel::Light),
            ("standard", NsLevel::Standard),
            ("strong", NsLevel::Strong),
            ("strong_low_delay", NsLevel::StrongLowDelay),
        ] {
            let saved = format!(
                r#"{{"input":"Mic","output":null,"input_gain_pct":150,"sensitivity":0.05,"echo_mode":"on",
                    "noise_suppression":"{name}","auto_sensitivity":false,"auto_gain":true}}"#
            );
            let cfg = serde_json::from_str::<AudioConfig>(&saved).unwrap();
            assert_eq!(cfg.noise_suppression, level, "{name}");
            assert_eq!(
                (
                    cfg.input.as_deref(),
                    cfg.input_gain_pct,
                    cfg.echo_mode,
                    cfg.auto_sensitivity,
                    cfg.auto_gain
                ),
                (Some("Mic"), 150, EchoMode::On, false, true)
            );
            let again: AudioConfig =
                serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
            assert_eq!(again.noise_suppression, level, "{name} round-trips");
        }
    }

    #[test]
    fn ns_status_change_is_reported_once() {
        let s = Shared::new(&AudioConfig::default(), Default::default(), 200);
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
        let s = Shared::new(&AudioConfig::default(), Default::default(), 200);
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

    /// Review 6: a build that finishes after the wait gave up is told so, and can skip its side effects.
    #[test]
    fn a_late_open_is_told_it_was_given_up_on() {
        use std::sync::atomic::AtomicBool;
        let (seen_tx, seen_rx) = std::sync::mpsc::channel();
        let r =
            Parked::<u32>::spawn_cancellable("test", Duration::from_millis(50), move |cancelled| {
                let at_start = cancelled.load(Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(200));
                let _ = seen_tx.send((at_start, cancelled.load(Ordering::SeqCst)));
                Ok(((), 7))
            });
        assert!(r.is_err());
        let (at_start, at_end) = seen_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(!at_start && at_end);
        // one that opens in time never sees it set
        let flag = Arc::new(AtomicBool::new(true));
        let f = flag.clone();
        let p =
            Parked::<u32>::spawn_cancellable("test", Duration::from_secs(1), move |cancelled| {
                f.store(cancelled.load(Ordering::SeqCst), Ordering::SeqCst);
                Ok(((), 7))
            })
            .unwrap();
        assert!(!flag.load(Ordering::SeqCst));
        p.close();
    }

    /// Review 7: the echo reference hears the front pair (all the mixer writes), not an average with silent rears.
    #[test]
    fn the_echo_reference_is_the_front_pair() {
        let surround = [0.4f32, 0.2, 0.0, 0.0, 0.0, 0.0];
        let mono: Vec<f32> = front_mono(&surround, 6).collect();
        assert_eq!(mono.len(), 1);
        assert!((mono[0] - 0.3).abs() < 1e-6);
        let stereo: Vec<f32> = front_mono(&[0.2, 0.4], 2).collect();
        assert!((stereo[0] - 0.3).abs() < 1e-6);
        let one: Vec<f32> = front_mono(&[0.5, 0.25], 1).collect();
        assert_eq!(one, vec![0.5, 0.25]);
    }

    #[test]
    fn a_resource_that_never_opens_times_out() {
        let r = Parked::<u32>::spawn("test", Duration::from_millis(50), || {
            std::thread::sleep(Duration::from_millis(300));
            Ok(((), 1))
        });
        assert!(r.is_err());
    }

    /// A side that never opened (no device then, or it wouldn't open) follows the system default
    /// once there is one (#362); a side whose name couldn't be read still never counts as moved.
    #[test]
    fn a_missing_side_on_the_default_follows_whatever_default_there_is() {
        let never_opened = opened("Arctis Mic", "");
        let speakers = (Some("Arctis Mic".to_string()), Some("Speakers".to_string()));
        let out = Sides {
            input: false,
            output: true,
        };
        assert_eq!(
            moved_sides(
                &AudioConfig::default(),
                &never_opened,
                speakers.clone(),
                out
            ),
            out
        );
        // open but unnamed: unknown, left alone
        assert!(
            !moved_sides(
                &AudioConfig::default(),
                &never_opened,
                speakers.clone(),
                Sides::default()
            )
            .any()
        );
        // no default to go to yet: nothing to do (the gap retry keeps trying)
        assert!(
            !moved_sides(
                &AudioConfig::default(),
                &never_opened,
                (Some("Arctis Mic".into()), None),
                out
            )
            .any()
        );
        // a device the user picked is theirs: the gap retry reopens it, following doesn't
        let picked = AudioConfig {
            output: Some("USB Speakers".into()),
            ..Default::default()
        };
        assert!(!moved_sides(&picked, &never_opened, speakers, out).any());
    }

    #[test]
    fn device_gaps_are_told_on_a_change_or_when_a_join_asks() {
        let mut n = GapNotice::default();
        let no_mic = DeviceGaps {
            input: Some("device in use".into()),
            output: None,
        };
        assert_eq!(n.check(Some(&no_mic), false), Some(no_mic.clone()));
        assert_eq!(n.check(Some(&no_mic), false), None, "unchanged");
        // a join found the devices mid-reopen: nothing to say until they're back
        assert_eq!(n.check(None, true), None);
        // back, the same as before: told anyway, because the join asked
        assert_eq!(n.check(Some(&no_mic), true), Some(no_mic.clone()));
        assert_eq!(n.check(Some(&no_mic), false), None);
        let all = DeviceGaps::default();
        assert_eq!(n.check(Some(&all), false), Some(all));
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
                (Some("Arctis Mic".into()), Some("Arctis".into())),
                Sides::default()
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

    /// The sound server names devices by what they are ("Arctis Nova 7P Mic"), not by their
    /// internal id; ALSA keeps the PCM id settings stored before.
    #[test]
    fn devices_are_named_by_what_they_are() {
        let pulse = Some("alsa_input.usb-SteelSeries_Arctis-00.mono-chat");
        assert_eq!(
            display_name(Backend::Pulse, pulse, Some("Arctis Nova 7P Mic")).as_deref(),
            Some("Arctis Nova 7P Mic")
        );
        assert_eq!(
            display_name(Backend::Pulse, pulse, None).as_deref(),
            pulse,
            "no description: the id"
        );
        assert_eq!(
            display_name(Backend::Alsa, Some("default"), Some("Default Audio Device")).as_deref(),
            Some("default")
        );
        assert_eq!(
            display_name(
                Backend::Other,
                Some("{0.0.1.00000000}"),
                Some("Headset (Arctis)")
            )
            .as_deref(),
            Some("Headset (Arctis)")
        );
    }

    #[test]
    fn output_monitors_and_alsa_plumbing_are_not_offered() {
        assert!(offered(
            Backend::Pulse,
            "alsa_input.usb-SteelSeries_Arctis-00.mono-chat"
        ));
        assert!(!offered(
            Backend::Pulse,
            "alsa_output.pci-0000_00_1f.3.analog-stereo.monitor"
        ));
        assert!(offered(Backend::Alsa, "default"));
        assert!(!offered(Backend::Alsa, "hdmi:CARD=PCH,DEV=0"));
        assert!(offered(Backend::Other, "anything"));
    }

    /// On this machine's sound server: real names, no monitors, the default among them.
    /// Real devices: run explicitly (`--ignored`).
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn linux_lists_real_devices_from_the_sound_server() {
        let inputs = list_inputs();
        let outputs = list_outputs();
        println!("inputs {inputs:?}\noutputs {outputs:?}");
        assert!(!outputs.is_empty(), "no outputs listed");
        assert!(outputs.iter().any(|d| d.is_default), "{outputs:?}");
        for d in inputs.iter().chain(&outputs) {
            assert!(
                !d.name.starts_with("Monitor of"),
                "a monitor offered: {d:?}"
            );
            assert_ne!(d.name, "default", "ALSA fallback, not the sound server");
        }
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

    /// cpal 0.18 reports xruns and device changes through the error callback: at warn, every
    /// bug report would show them as if something broke.
    #[test]
    fn routine_stream_errors_are_not_warnings() {
        use cpal::ErrorKind as K;
        assert!(routine_stream_error(K::Xrun));
        assert!(routine_stream_error(K::StreamInvalidated));
        for k in [K::DeviceNotAvailable, K::RealtimeDenied, K::BackendError] {
            assert!(!routine_stream_error(k), "{k:?}");
        }
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

    /// A rejoin reopens the same devices right after tearing down: `close` must not return until
    /// the old streams are really gone (a Bluetooth headset can't be open twice).
    #[test]
    fn close_waits_until_the_streams_are_really_gone() {
        let gone = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (raw_tx, _raw_rx) = super::super::rings::raw_queue();
        let shared = Shared::new(&AudioConfig::default(), Default::default(), 200);
        let io = AudioIo::slow_to_close(shared, raw_tx, Duration::from_millis(200), gone.clone());
        io.close();
        assert!(
            gone.load(Ordering::SeqCst),
            "returned before the stream closed"
        );
    }

    fn opened_half(name: &str) -> Half {
        let name = name.to_string();
        Parked::spawn("test-half", Duration::from_secs(1), move || {
            Ok((
                (),
                HalfInfo {
                    rate: 48_000,
                    name,
                    channels: 2,
                    format: "f32".into(),
                    generation: 0,
                },
            ))
        })
        .expect("a test half opens")
    }

    /// #351: no mic (privacy off, held exclusively, none plugged in) still joins: the speakers
    /// open on their own and the mic's side says why it's missing.
    #[test]
    fn a_start_without_a_mic_keeps_the_speakers() {
        let (raw_tx, _raw_rx) = super::super::rings::raw_queue();
        let shared = Shared::new(&AudioConfig::default(), Default::default(), 200);
        let io = AudioIo::from_halves(
            Err(anyhow!("audio input: no input device")),
            Ok(opened_half("Speakers")),
            shared,
            raw_tx,
        );
        assert_eq!(io.opened().output, "Speakers");
        assert_eq!(io.input_rate(), None);
        let gaps = io.gaps();
        assert_eq!(gaps.input.as_deref(), Some("no input device"));
        assert_eq!(gaps.output, None);
        assert_eq!(
            gaps.sides(),
            Sides {
                input: true,
                output: false
            }
        );
    }

    /// #351: and the mirror image: speakers that won't open still join with the mic.
    #[test]
    fn a_start_without_speakers_keeps_the_mic() {
        let (raw_tx, _raw_rx) = super::super::rings::raw_queue();
        let shared = Shared::new(&AudioConfig::default(), Default::default(), 200);
        let io = AudioIo::from_halves(
            Ok(opened_half("Headset mic")),
            Err(anyhow!("audio output: device in use")),
            shared,
            raw_tx,
        );
        assert_eq!(io.opened().input, "Headset mic");
        // what a bug report shows (#605): the open side's format, nothing for the missing one
        assert_eq!(
            io.formats(),
            (
                Some(OpenedFormat {
                    device: "Headset mic".into(),
                    rate: 48_000,
                    channels: 2,
                    sample_format: "f32".into(),
                }),
                None
            )
        );
        let gaps = io.gaps();
        assert_eq!(gaps.input, None);
        assert_eq!(gaps.output.as_deref(), Some("device in use"));
    }

    /// Test audio (no devices at all) is never "missing" anything.
    #[tokio::test]
    async fn null_audio_has_no_gaps() {
        let (raw_tx, _raw_rx) = super::super::rings::raw_queue();
        let shared = Shared::new(&AudioConfig::default(), Default::default(), 200);
        let io = AudioIo::start_null(48_000, shared, raw_tx);
        assert_eq!(io.gaps(), DeviceGaps::default());
    }

    /// A missing side is tried again on a backoff (a mic held by another app, privacy off): soon
    /// at first, then every 30 s, and from scratch once it's back.
    #[test]
    fn a_missing_side_is_retried_with_backoff() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut r = GapRetry::default();
        assert!(!r.due(false, t0), "nothing missing: nothing to do");
        assert!(!r.due(true, t0), "first seen: give it a moment");
        assert!(!r.due(true, at(1)));
        assert!(r.due(true, at(2)));
        assert!(!r.due(true, at(6)));
        assert!(r.due(true, at(7)));
        assert!(r.due(true, at(17)));
        assert!(r.due(true, at(47)));
        assert!(!r.due(true, at(70)));
        assert!(r.due(true, at(77)), "capped at 30 s");
        assert!(!r.due(false, at(78)), "back");
        assert!(!r.due(true, at(79)));
        assert!(r.due(true, at(81)), "a new gap starts over");
    }

    /// A side that isn't open has no callbacks: that's a gap (retried), not a stall (reported).
    #[test]
    fn missing_sides_are_not_stalls() {
        let stalled = Sides::BOTH;
        let missing = Sides {
            input: true,
            output: false,
        };
        assert_eq!(
            stalled.minus(missing),
            Sides {
                input: false,
                output: true
            }
        );
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
