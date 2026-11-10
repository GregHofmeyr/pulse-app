//! Mic processing off the audio callback: resample → APM → denoiser → gain → gate, per 10 ms block.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use livekit::webrtc::audio_frame::AudioFrame;
use livekit::webrtc::audio_source::native::NativeAudioSource;

use super::backlog::GapFill;
use super::denoise::{DeepFilter, Denoiser, FRAME, NsLevel, make_fast};
use super::devices::{INTERNAL_RATE, NsStatus, Shared};
use super::gate::{PREROLL_BLOCKS, VoiceGate};
use super::micsilence;
use super::mixer::soft_limit;
use super::mutedtalk;
use super::resampler::StreamResampler;
use crate::models::{LoadError, ModelStore};

const OVERLOAD_WINDOW: usize = 300; // 3 s of 10 ms blocks
const OVERLOAD_MEAN: Duration = Duration::from_micros(7_000); // 70% of the 10 ms budget

/// Detects a PC that can't keep up: the mean over a full 3 s window above 70% of the budget.
pub struct OverloadDetector {
    times: VecDeque<Duration>,
    sum: Duration,
}

impl OverloadDetector {
    pub fn new() -> Self {
        Self {
            times: VecDeque::with_capacity(OVERLOAD_WINDOW),
            sum: Duration::ZERO,
        }
    }

    pub fn reset(&mut self) {
        self.times.clear();
        self.sum = Duration::ZERO;
    }

    pub fn record(&mut self, took: Duration) -> bool {
        self.times.push_back(took);
        self.sum += took;
        if self.times.len() > OVERLOAD_WINDOW {
            self.sum -= self.times.pop_front().unwrap_or_default();
        }
        self.times.len() == OVERLOAD_WINDOW && self.sum / OVERLOAD_WINDOW as u32 > OVERLOAD_MEAN
    }
}

impl Default for OverloadDetector {
    fn default() -> Self {
        Self::new()
    }
}

pub struct BlockOut {
    /// RMS after cleaning + gain (the mic meter: what you see is what is sent).
    pub level: f32,
    /// Whether audio (not silence) went out.
    pub sent: bool,
    /// Sum of squares into and out of the denoiser (its attenuation, for the path line).
    pub ns_in: f32,
    pub ns_out: f32,
    /// Whether the gate judged this block (false while muted).
    pub judged: bool,
    /// RMS of the block as it came in (before cleaning and gain): with `level`, what suppression removed.
    pub in_level: f32,
}

fn rms(b: &[f32]) -> f32 {
    (b.iter().map(|v| v * v).sum::<f32>() / b.len().max(1) as f32).sqrt()
}

/// Blocks kept while the denoiser idles (muted, no meter open), replayed into it on waking so its
/// look-back holds the last 80 ms of real audio, not what came just before the mute (that would
/// leak out as a blip on unmute). DeepFilterNet looks back df_order + lookahead = 7 hops; 8 covers it.
const PRIME_BLOCKS: usize = 8;

/// One block's journey after APM: denoise → gain (soft-limited) → gate → mute.
pub struct MicChain {
    level: NsLevel,
    denoiser: Box<dyn Denoiser>,
    gate: VoiceGate,
    gain: f32,
    /// The latest blocks seen while idle, oldest first (at most PRIME_BLOCKS).
    recent: VecDeque<Vec<f32>>,
    idle: bool,
}

impl MicChain {
    pub fn new(level: NsLevel, auto: bool, threshold: f32, gain: f32) -> Self {
        Self {
            level,
            denoiser: make_fast(level),
            gate: VoiceGate::new(auto, threshold),
            gain,
            recent: VecDeque::with_capacity(PRIME_BLOCKS),
            idle: false,
        }
    }

    pub fn level(&self) -> NsLevel {
        self.level
    }

    pub fn set_denoiser(&mut self, level: NsLevel, d: Box<dyn Denoiser>) {
        self.level = level;
        self.denoiser = d;
    }

    pub fn denoiser_failed(&self) -> bool {
        self.denoiser.failed()
    }

    pub fn configure(&mut self, auto: bool, threshold: f32, gain: f32) {
        self.gate.configure(auto, threshold);
        self.gain = gain;
    }

    /// Muted with no meter on screen: send silence (as a muted `process` does) without running the
    /// denoiser, and remember the block for waking up. Like a muted block, nothing is judged and
    /// nothing counts toward the path line's suppression figure.
    pub fn idle(&mut self, block: &mut [f32]) -> BlockOut {
        let in_level = rms(block);
        let mut kept = if self.recent.len() == PRIME_BLOCKS {
            self.recent.pop_front().unwrap_or_default()
        } else {
            Vec::with_capacity(block.len())
        };
        kept.clear();
        kept.extend_from_slice(block);
        self.recent.push_back(kept);
        self.idle = true;
        self.gate.reset();
        block.fill(0.0);
        BlockOut {
            level: 0.0,
            sent: false,
            ns_in: 0.0,
            ns_out: 0.0,
            judged: false,
            in_level,
        }
    }

    pub fn process(&mut self, block: &mut [f32], mic_open: bool, now: Instant) -> BlockOut {
        if self.idle {
            // Back from idle: bring the denoiser's state up to date (output discarded) first.
            self.idle = false;
            for b in self.recent.iter_mut() {
                self.denoiser.process(b);
            }
            self.recent.clear(); // keeps its capacity for the next mute
        }
        let energy = |b: &[f32]| b.iter().map(|v| v * v).sum::<f32>();
        let ns_in = energy(block);
        let in_level = (ns_in / block.len().max(1) as f32).sqrt();
        let prob = self.denoiser.process(block);
        let ns_out = energy(block);
        let newest = self.denoiser.decision_level().unwrap_or(in_level) * self.gain;
        for v in block.iter_mut() {
            *v = soft_limit(*v * self.gain);
        }
        let level = rms(block);
        if !mic_open {
            self.gate.reset();
            block.fill(0.0);
            return BlockOut {
                level,
                sent: false,
                ns_in,
                ns_out,
                judged: false,
                in_level,
            };
        }
        // Auto decides on the newest audio, not the cleaned block (which lags it): the engine's
        // delay then doubles as the gate's pre-roll instead of adding to it. A manual threshold
        // sits on the mic meter, so it judges what the meter shows behind the full pre-roll.
        let (decide, preroll) = if self.gate.is_auto() {
            let lead = self.denoiser.gate_lead_blocks();
            (newest, PREROLL_BLOCKS.saturating_sub(lead))
        } else {
            (level, PREROLL_BLOCKS)
        };
        self.gate.set_preroll(preroll);
        let sent = self.gate.process(block, decide, prob, now);
        BlockOut {
            level,
            sent,
            ns_in,
            ns_out,
            judged: true,
            in_level,
        }
    }
}

/// (device rate, 10 ms of mono samples, when the device handed it over).
pub type RawBlock = (u32, Vec<f32>, Instant);
pub type RawTx = std_mpsc::Sender<RawBlock>;

/// Audio that has waited longer than this is dropped so latency never grows. Measured by age, not
/// count: a device with big buffers hands over many blocks at once, which isn't lag.
const MAX_LAG: Duration = Duration::from_millis(60);

pub fn trim_backlog(q: &mut VecDeque<RawBlock>, now: Instant) -> usize {
    let stale = q
        .iter()
        .take_while(|(_, _, at)| now.saturating_duration_since(*at) > MAX_LAG)
        .count();
    q.drain(..stale);
    stale
}

/// Which DeepFilterNet model a worker runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    /// Strong: the bundled model.
    Normal,
    /// Strong (low delay): the downloaded one (#77).
    LowDelay,
}

impl Model {
    pub fn level(self) -> NsLevel {
        match self {
            Self::Normal => NsLevel::Strong,
            Self::LowDelay => NsLevel::StrongLowDelay,
        }
    }
}

/// Tracks which level is wanted and which one is being aimed for (they differ after a fallback: low delay → Strong →
/// Standard), so a slow model load can't overwrite a newer choice or fallback.
pub struct Switcher {
    wanted: NsLevel,
    target: NsLevel,
    generation: u64,
    /// Why `target` isn't `wanted` (shown while the fallback runs).
    note: Option<String>,
}

impl Switcher {
    pub fn new(level: NsLevel) -> Self {
        Self {
            wanted: level,
            target: level,
            generation: 0,
            note: None,
        }
    }

    /// Record a new wanted level; returns the generation a load for it must present.
    pub fn request(&mut self, level: NsLevel) -> u64 {
        self.wanted = level;
        self.target = level;
        self.note = None;
        self.generation += 1;
        self.generation
    }

    /// Aim for `level` instead of the wanted one, saying why; returns the generation a load for it must present.
    pub fn fall_back(&mut self, level: NsLevel, note: &str) -> u64 {
        self.target = level;
        self.note = Some(note.to_string());
        self.generation += 1;
        self.generation
    }

    pub fn wanted(&self) -> NsLevel {
        self.wanted
    }

    pub fn target(&self) -> NsLevel {
        self.target
    }

    pub fn note(&self) -> Option<String> {
        self.note.clone()
    }

    pub fn accept_loaded(&self, generation: u64, model: Model) -> bool {
        generation == self.generation && self.target == model.level()
    }
}

const LL_MISSING: &str = "Strong (low delay) isn't downloaded yet, using Strong";
const LL_UNAVAILABLE: &str = "Strong (low delay) unavailable, using Strong";
const LL_TOO_HEAVY: &str = "Strong (low delay) was too heavy for this PC, using Strong";
const LL_STOPPED: &str = "Strong (low delay) stopped working, using Strong";
const STRONG_UNAVAILABLE: &str = "Strong unavailable, using Standard";
const STRONG_TOO_HEAVY: &str = "Strong was too heavy for this PC, using Standard";
const STRONG_STOPPED: &str = "Strong stopped working, using Standard";

/// Noise suppression's engine: what the user picked, what can run, and the model loads in between. The fallback
/// chain is Strong (low delay) → Strong → Standard: a missing, damaged, failed or too-heavy low-delay model gives
/// Strong (with a note), and Strong's own failures give Standard, as before.
pub struct NsControl {
    switcher: Switcher,
    load_tx: std_mpsc::Sender<LoadResult>,
    load_rx: std_mpsc::Receiver<LoadResult>,
    models: Option<Arc<ModelStore>>,
    /// The models' epoch last seen: a download that finishes mid-call is picked up.
    epoch: u64,
}

impl NsControl {
    pub fn new(level: NsLevel, models: Option<Arc<ModelStore>>) -> Self {
        let (load_tx, load_rx) = std_mpsc::channel();
        Self {
            switcher: Switcher::new(level),
            load_tx,
            load_rx,
            epoch: models.as_ref().map_or(0, |m| m.epoch()),
            models,
        }
    }

    pub fn switcher(&self) -> &Switcher {
        &self.switcher
    }

    fn start(&self, generation: u64, model: Model) {
        start_strong(generation, model, self.models.clone(), self.load_tx.clone());
    }

    fn low_delay_present(&self) -> bool {
        self.models.as_ref().is_some_and(|m| m.present())
    }

    /// The user picked `level` (also at start). Between Strong and low delay the running model carries on until the
    /// other has loaded; from Off or Standard, Standard runs meanwhile.
    pub fn choose(&mut self, level: NsLevel, chain: &mut MicChain, shared: &Shared) {
        let g = self.switcher.request(level);
        let running = chain.level();
        let model = match level {
            NsLevel::Off | NsLevel::Standard => {
                chain.set_denoiser(level, make_fast(level));
                return set_status(shared, level, None);
            }
            NsLevel::Strong => Model::Normal,
            NsLevel::StrongLowDelay if !self.low_delay_present() => {
                return self.fall_back_to_strong(LL_MISSING, chain, shared);
            }
            NsLevel::StrongLowDelay => Model::LowDelay,
        };
        if running == level {
            return set_status(shared, level, None);
        }
        if !running.is_deep_filter() {
            chain.set_denoiser(NsLevel::Standard, make_fast(NsLevel::Standard));
        }
        self.start(g, model);
        set_status(shared, chain.level(), None);
    }

    /// Aim for Strong instead of low delay. A Strong that's running already carries on.
    fn fall_back_to_strong(&mut self, note: &str, chain: &mut MicChain, shared: &Shared) {
        let g = self.switcher.fall_back(NsLevel::Strong, note);
        if chain.level() != NsLevel::Strong {
            chain.set_denoiser(NsLevel::Standard, make_fast(NsLevel::Standard));
            self.start(g, Model::Normal);
        }
        set_status(shared, chain.level(), Some(note.to_string()));
    }

    fn fall_back_to_standard(&mut self, note: &str, chain: &mut MicChain, shared: &Shared) {
        self.switcher.fall_back(NsLevel::Standard, note);
        chain.set_denoiser(NsLevel::Standard, make_fast(NsLevel::Standard));
        set_status(shared, NsLevel::Standard, Some(note.to_string()));
    }

    /// Install finished model loads (outdated ones are dropped, which frees their model); a failed one falls back.
    /// True if the engine changed.
    pub fn take_loaded(&mut self, chain: &mut MicChain, shared: &Shared) -> bool {
        let mut changed = false;
        while let Ok((g, model, result)) = self.load_rx.try_recv() {
            if !self.switcher.accept_loaded(g, model) {
                continue;
            }
            changed = true;
            match (model, result) {
                (_, Ok(worker)) => {
                    chain.set_denoiser(model.level(), Box::new(worker));
                    set_status(shared, model.level(), self.switcher.note());
                }
                (Model::LowDelay, Err(e)) => {
                    let missing = e.downcast_ref::<LoadError>() == Some(&LoadError::Missing);
                    tracing::warn!(error = %e, "noise suppression: Strong (low delay) failed to load, using Strong");
                    let note = if missing { LL_MISSING } else { LL_UNAVAILABLE };
                    self.fall_back_to_strong(note, chain, shared);
                }
                (Model::Normal, Err(e)) => {
                    report_load_failure(shared, &e.to_string());
                    self.switcher
                        .fall_back(NsLevel::Standard, STRONG_UNAVAILABLE);
                    if chain.level() != NsLevel::Standard {
                        chain.set_denoiser(NsLevel::Standard, make_fast(NsLevel::Standard));
                    }
                }
            }
        }
        changed
    }

    /// The low-delay model was downloaded meanwhile: load it if it's wanted and not running (what runs carries on
    /// until it has). True if a load started.
    pub fn check_models(&mut self, chain: &MicChain, shared: &Shared) -> bool {
        let Some(epoch) = self.models.as_ref().map(|m| m.epoch()) else {
            return false;
        };
        if epoch == self.epoch {
            return false;
        }
        self.epoch = epoch;
        if self.switcher.wanted() != NsLevel::StrongLowDelay
            || self.switcher.target() == NsLevel::StrongLowDelay
        {
            return false;
        }
        let g = self.switcher.request(NsLevel::StrongLowDelay);
        self.start(g, Model::LowDelay);
        set_status(shared, chain.level(), None);
        true
    }

    /// Whether this block's time counts towards the overload check: only while the aimed-for model runs.
    pub fn measures(&self, chain: &MicChain) -> bool {
        chain.level().is_deep_filter() && chain.level() == self.switcher.target()
    }

    /// This PC can't keep up: low delay → Strong, Strong → Standard.
    pub fn overloaded(&mut self, chain: &mut MicChain, shared: &Shared) {
        if chain.level() == NsLevel::StrongLowDelay {
            tracing::warn!(
                "noise suppression: this PC can't keep up with Strong (low delay), switching to Strong"
            );
            self.fall_back_to_strong(LL_TOO_HEAVY, chain, shared);
        } else {
            tracing::warn!(
                "noise suppression: this PC can't keep up with Strong, switching to Standard"
            );
            self.fall_back_to_standard(STRONG_TOO_HEAVY, chain, shared);
        }
    }

    /// Replace an engine that stopped working: low delay → Strong, Strong → Standard. True if it replaced one.
    pub fn recover(&mut self, chain: &mut MicChain, shared: &Shared) -> bool {
        if !chain.denoiser_failed() {
            return false;
        }
        if chain.level() == NsLevel::StrongLowDelay {
            tracing::warn!("noise suppression: Strong (low delay) stopped working, using Strong");
            self.fall_back_to_strong(LL_STOPPED, chain, shared);
        } else {
            recover_failed(chain, shared);
            self.switcher.fall_back(NsLevel::Standard, STRONG_STOPPED);
        }
        true
    }
}

fn set_status(shared: &Shared, active: NsLevel, note: Option<String>) {
    shared.set_ns_status(NsStatus { active, note });
}

pub fn report_load_failure(shared: &Shared, err: &str) {
    tracing::warn!(
        error = err,
        "noise suppression: Strong failed to load, using Standard"
    );
    shared.set_ns_status(NsStatus {
        active: NsLevel::Standard,
        note: Some(STRONG_UNAVAILABLE.into()),
    });
}

/// Strong's model can't change threads (DeepFilterNet's tract plan isn't `Send`), so it lives on
/// its own worker thread, which loads it and cleans blocks there. This handle round-trips each
/// block; dropping it ends the worker and frees the model.
pub struct StrongWorker {
    tx: std_mpsc::Sender<Vec<f32>>,
    rx: std_mpsc::Receiver<(Vec<f32>, Option<f32>, Option<f32>)>,
    /// The worker thread is gone (it panicked or was killed): audio passes through until replaced.
    dead: bool,
    /// The worker's last decision level (see `Denoiser::decision_level`).
    level: Option<f32>,
    /// The model's `gate_lead_blocks` (Strong: 2, low delay: 0).
    lead: usize,
}

impl Denoiser for StrongWorker {
    fn process(&mut self, block: &mut [f32]) -> Option<f32> {
        // dead: the block passes raw, so the gate judges its own level (not the worker's last)
        if self.dead || self.tx.send(block.to_vec()).is_err() {
            self.dead = true;
            self.level = None;
            return None;
        }
        match self.rx.recv() {
            Ok((out, p, level)) => {
                block.copy_from_slice(&out);
                self.level = level;
                p
            }
            Err(_) => {
                self.dead = true;
                self.level = None;
                None
            }
        }
    }

    fn failed(&self) -> bool {
        self.dead
    }

    fn gate_lead_blocks(&self) -> usize {
        self.lead // DeepFilter's, on its own thread
    }

    fn decision_level(&self) -> Option<f32> {
        self.level
    }
}

pub type LoadResult = (u64, Model, anyhow::Result<StrongWorker>);

/// Load `model` on a new worker thread (low delay from `models`, its hash checked); the result (tagged with
/// `generation`) arrives on `done`.
pub fn start_strong(
    generation: u64,
    model: Model,
    models: Option<Arc<ModelStore>>,
    done: std_mpsc::Sender<LoadResult>,
) {
    let spawned = std::thread::Builder::new()
        .name("pulse-strong".into())
        .spawn(move || {
            let loaded = match model {
                Model::Normal => DeepFilter::new(),
                Model::LowDelay => models
                    .ok_or(LoadError::Missing)
                    .and_then(|m| m.load())
                    .map_err(anyhow::Error::from)
                    .and_then(|bytes| DeepFilter::low_delay(&bytes)),
            };
            let mut df = match loaded {
                Err(e) => {
                    let _ = done.send((generation, model, Err(e)));
                    return;
                }
                Ok(df) => df,
            };
            {
                let _rt = super::rtprio::promote("pulse-strong");
                let (in_tx, in_rx) = std_mpsc::channel::<Vec<f32>>();
                let (out_tx, out_rx) = std_mpsc::channel();
                let handle = StrongWorker {
                    tx: in_tx,
                    rx: out_rx,
                    dead: false,
                    level: None,
                    lead: df.gate_lead_blocks(),
                };
                if done.send((generation, model, Ok(handle))).is_err() {
                    return;
                }
                while let Ok(mut block) = in_rx.recv() {
                    let p = df.process(&mut block);
                    if out_tx.send((block, p, df.decision_level())).is_err() {
                        return;
                    }
                }
            }
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "noise suppression: couldn't start the Strong worker");
    }
}

/// Replace a denoiser that has stopped working with Standard and say so. True if it replaced one.
pub fn recover_failed(chain: &mut MicChain, shared: &Shared) -> bool {
    if !chain.denoiser_failed() {
        return false;
    }
    tracing::warn!("noise suppression: Strong stopped working, using Standard");
    chain.set_denoiser(NsLevel::Standard, make_fast(NsLevel::Standard));
    shared.set_ns_status(NsStatus {
        active: NsLevel::Standard,
        note: Some(STRONG_STOPPED.into()),
    });
    true
}

/// Where finished 10 ms mic frames (48 kHz mono i16) go, called on the processor thread.
pub trait FrameSink: Send {
    /// Hand over one frame; false if it was refused (counted as a send drop).
    fn send(&mut self, frame: &[i16]) -> bool;
}

/// The room's mic track. The fast path (queue 0) is synchronous: each frame goes straight to
/// WebRTC's encoder from here, with no runtime hop and no copy beyond the i16 conversion.
pub struct LiveKitSink(NativeAudioSource);

impl LiveKitSink {
    pub fn new(source: NativeAudioSource) -> Self {
        Self(source)
    }
}

impl FrameSink for LiveKitSink {
    fn send(&mut self, frame: &[i16]) -> bool {
        use futures::FutureExt;
        let f = AudioFrame {
            data: std::borrow::Cow::Borrowed(frame),
            sample_rate: INTERNAL_RATE,
            num_channels: 1,
            samples_per_channel: frame.len() as u32,
        };
        // queue 0 never awaits: `None` would mean a buffered source, which we never build
        matches!(self.0.capture_frame(&f).now_or_never(), Some(Ok(())))
    }
}

/// How often the processor looks for a stalled mic when no block arrives.
const GAP_CHECK: Duration = Duration::from_millis(10);

/// Hand `frame` to the room, if there is one. Between rooms there's nowhere to send: the frame
/// is dropped, and the gap clock restarts so a new room doesn't get a burst of "missed" silence.
fn deliver(shared: &Shared, gap: &mut GapFill, frame: &[i16]) {
    let now = Instant::now();
    let sent = shared
        .mic_sink
        .lock()
        .unwrap()
        .as_mut()
        .map(|s| s.send(frame));
    match sent {
        Some(false) => {
            shared
                .stats
                .send_drops
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Some(true) | None => gap.sent(now),
    }
}

/// The mic delivered nothing for a while (stall, device reopen): keep the room's stream
/// continuous with silent frames, or listeners' jitter buffers grow to cover the hole.
fn fill_gap(shared: &Shared, gap: &mut GapFill, silence: &[i16]) {
    let now = Instant::now();
    let mut sink = shared.mic_sink.lock().unwrap();
    let Some(s) = sink.as_mut() else {
        gap.sent(now);
        return;
    };
    let due = gap.silence_due(now);
    let sent = (0..due).filter(|_| s.send(silence)).count();
    if due > 0 {
        gap.silence_sent(due);
        let relaxed = std::sync::atomic::Ordering::Relaxed;
        shared.stats.silence_fill.fetch_add(sent as u64, relaxed);
        shared
            .stats
            .send_drops
            .fetch_add((due - sent) as u64, relaxed);
    }
}

/// Start the mic processor. It runs until every returned sender is dropped; its frames go to
/// `shared`'s mic sink (set per room).
pub fn spawn(shared: Arc<Shared>) -> (RawTx, std::thread::JoinHandle<()>) {
    let (tx, rx) = std_mpsc::channel::<RawBlock>();
    let h = std::thread::Builder::new()
        .name("pulse-mic".into())
        .spawn(move || {
            let _rt = super::rtprio::promote("pulse-mic");
            run(shared, rx)
        })
        .expect("spawn mic processor");
    (tx, h)
}

fn to_i16(v: f32) -> i16 {
    (v * 32767.0).round().clamp(-32768.0, 32767.0) as i16
}

fn run(shared: Arc<Shared>, rx: std_mpsc::Receiver<RawBlock>) {
    let cfg = shared.proc_cfg.lock().unwrap().clone();
    let mut version = cfg.version;
    // Standard runs until a DeepFilterNet model has loaded (`chain.level()` is always what really runs).
    let mut chain = MicChain::new(NsLevel::Off, cfg.auto, cfg.threshold, cfg.gain);
    let mut ns = NsControl::new(cfg.level, shared.low_delay_model());
    ns.choose(cfg.level, &mut chain, &shared);

    let mut overload = OverloadDetector::new();
    let mut resampler: Option<(u32, StreamResampler)> = None;
    let mut pending: Vec<f32> = Vec::with_capacity(2 * FRAME);
    let mut queue: VecDeque<RawBlock> = VecDeque::new();
    let mut i16buf = vec![0i16; FRAME];
    let (mut gap, silence) = (GapFill::new(Instant::now()), vec![0i16; FRAME]);

    loop {
        let first = match rx.recv_timeout(GAP_CHECK) {
            Ok(b) => b,
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                fill_gap(&shared, &mut gap, &silence);
                continue;
            }
            Err(std_mpsc::RecvTimeoutError::Disconnected) => return, // devices closed for good
        };
        queue.push_back(first);
        queue.extend(rx.try_iter());
        let late = trim_backlog(&mut queue, Instant::now());
        shared
            .stats
            .lag_drops
            .fetch_add(late as u64, std::sync::atomic::Ordering::Relaxed);
        // blocks waiting × 10 = ms behind (`on_input` hands over 10 ms blocks at any device rate)
        shared.mic_queue.record(queue.len() as u32 * 10);

        // live config
        let cfg = shared.proc_cfg.lock().unwrap().clone();
        if cfg.version != version {
            version = cfg.version;
            chain.configure(cfg.auto, cfg.threshold, cfg.gain);
            if cfg.level != ns.switcher().wanted() {
                ns.choose(cfg.level, &mut chain, &shared);
                overload.reset();
            }
        }
        // the low-delay model finished downloading; a finished model load
        ns.check_models(&chain, &shared);
        if ns.take_loaded(&mut chain, &shared) {
            overload.reset();
        }

        for (rate, samples, _) in queue.drain(..) {
            if rate == INTERNAL_RATE {
                resampler = None;
                pending.extend_from_slice(&samples);
            } else {
                if resampler.as_ref().map(|(r, _)| *r) != Some(rate) {
                    resampler = Some((rate, StreamResampler::new(rate, INTERNAL_RATE)));
                }
                if let Some((_, rs)) = resampler.as_mut() {
                    rs.push(&samples, &mut pending);
                }
            }
        }

        while pending.len() >= FRAME {
            let mut block: Vec<f32> = pending.drain(..FRAME).collect();
            // raw, before any processing: AGC would lift a near-silent mic
            let raw_peak = micsilence::peak(&block);
            // render before capture: what the speakers played up to now is the echo reference
            shared.feed_echo_reference();
            // WebRTC APM (HPF, AEC, AGC) works on i16
            for (d, v) in i16buf.iter_mut().zip(&block) {
                *d = to_i16(*v);
            }
            let apm_started = Instant::now();
            let r = shared
                .apm
                .lock()
                .unwrap()
                .process_stream(&mut i16buf, INTERNAL_RATE as i32, 1);
            shared.stats.apm.record(apm_started.elapsed());
            shared.warn_apm_once(r, "capture");
            for (d, v) in block.iter_mut().zip(&i16buf) {
                *d = *v as f32 / 32767.0;
            }
            // Muted talk is judged after APM (echo cancelled), before the denoiser: on the raw
            // input, the call playing through speakers would read as you talking.
            let echo_free_rms = mutedtalk::rms(&block);

            let (mic_open, denoise, muted_talk, muted_cue) = {
                let c = shared.controls.lock().unwrap();
                (
                    c.mic_open(),
                    c.denoise_needed(),
                    c.counts_muted_talk(),
                    c.cues_muted_talk(),
                )
            };
            shared
                .mic_silence
                .lock()
                .unwrap()
                .block(raw_peak, mic_open, Instant::now());
            shared
                .muted_talk
                .lock()
                .unwrap()
                // your own mute only: server-muted time is never "talking while muted" (#140)
                .block(echo_free_rms, muted_talk, Instant::now());
            shared
                .muted_cue
                .lock()
                .unwrap()
                .block(echo_free_rms, muted_cue, Instant::now());
            if !denoise {
                // Muted, no meter on screen: skip the denoiser (Strong is most of a call's CPU).
                // APM above keeps running, so echo cancellation is still converged on unmute.
                // Silence still goes out every 10 ms: a send gap would grow listeners' jitter buffers.
                let out = chain.idle(&mut block);
                shared
                    .stats
                    .record_mic_block(out.ns_in, out.ns_out, out.judged, out.sent);
                shared.add_mic_level(out.level, out.sent);
                i16buf.fill(0);
                deliver(&shared, &mut gap, &i16buf);
                continue;
            }
            let t = Instant::now();
            let out = chain.process(&mut block, mic_open, t);
            shared.stats.chain.record(t.elapsed());
            shared
                .stats
                .record_mic_block(out.ns_in, out.ns_out, out.judged, out.sent);
            if ns.measures(&chain) && overload.record(t.elapsed()) {
                ns.overloaded(&mut chain, &shared);
                overload.reset();
            }
            if ns.recover(&mut chain, &shared) {
                overload.reset();
            }
            shared.add_mic_level(out.level, out.sent);
            for (d, v) in i16buf.iter_mut().zip(block.iter()) {
                *d = to_i16(*v);
            }
            deliver(&shared, &mut gap, &i16buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::denoise::{Denoiser, FRAME, NsLevel};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    #[test]
    fn one_spike_does_not_downgrade() {
        let mut o = OverloadDetector::new();
        let mut tripped = false;
        for i in 0..600 {
            let took = if i == 350 {
                Duration::from_millis(40)
            } else {
                Duration::from_millis(2)
            };
            tripped |= o.record(took);
        }
        assert!(!tripped);
    }

    #[test]
    fn sustained_overload_downgrades_after_a_full_window() {
        let mut o = OverloadDetector::new();
        let first = (1..=600).find(|_| o.record(Duration::from_millis(8)));
        assert_eq!(first, Some(300), "trips exactly when 3 s of blocks are in");
    }

    #[test]
    fn under_budget_never_downgrades() {
        let mut o = OverloadDetector::new();
        assert!((0..2000).all(|_| !o.record(Duration::from_micros(6500))));
    }

    struct Halve;
    impl Denoiser for Halve {
        fn process(&mut self, block: &mut [f32]) -> Option<f32> {
            block.iter_mut().for_each(|v| *v *= 0.5);
            Some(1.0)
        }
    }

    /// R4: each block reports the denoiser's input and output energy, and whether the gate judged it.
    #[test]
    fn block_out_reports_denoiser_energy_and_whether_the_gate_judged() {
        let mut c = MicChain::new(NsLevel::Off, false, 0.0, 1.0);
        c.set_denoiser(NsLevel::Standard, Box::new(Halve));
        let mut b = vec![0.2f32; FRAME];
        let out = c.process(&mut b, true, Instant::now());
        // 480 × 0.04 = 19.2 in; 480 × 0.01 = 4.8 out (−6 dB)
        assert!((out.ns_in - 19.2).abs() < 1e-3, "{}", out.ns_in);
        assert!((out.ns_out - 4.8).abs() < 1e-3, "{}", out.ns_out);
        assert!(out.judged);
        let out = c.process(&mut vec![0.2f32; FRAME], false, Instant::now());
        assert!(!out.judged, "muted: the gate didn't judge it");
    }

    /// R4 end to end: the processor feeds the path-line counters.
    #[test]
    fn processor_counts_denoiser_energy_and_gate_verdicts() {
        let shared = shared_with(AudioConfig {
            noise_suppression: NsLevel::Off,
            auto_sensitivity: false,
            sensitivity: 0.0, // manual, threshold 0 = always open
            ..Default::default()
        });
        let seen = collecting(&shared);
        let (raw_tx, _h) = spawn(shared.clone());
        let tone: Vec<f32> = (0..480).map(|i| 0.1 * (i as f32 * 0.0576).sin()).collect();
        for _ in 0..10 {
            raw_tx.send((48_000, tone.clone(), Instant::now())).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(wait_frames(&seen, 10) >= 10);
        let s = shared.stats.take(0);
        assert_eq!(
            s.ns_atten_db.map(f64::round),
            Some(0.0),
            "Off removes nothing"
        );
        assert_eq!(s.gate_open_pct.map(f64::round), Some(100.0));
    }

    /// The path line's ns/gate figures cover mic-open time only: a window spent muted reads "-",
    /// whether the denoiser idled (no meter on screen) or ran for the meter. And the processor
    /// takes the idle branch only while nobody looks at the meter.
    #[test]
    fn a_muted_window_has_no_ns_or_gate_figures() {
        let shared = shared_with(AudioConfig {
            noise_suppression: NsLevel::Off,
            auto_sensitivity: false,
            sensitivity: 0.0, // would always be open
            ..Default::default()
        });
        shared.controls.lock().unwrap().toggle_mute();
        let seen = collecting(&shared);
        let (raw_tx, _h) = spawn(shared.clone());
        let tone: Vec<f32> = (0..480).map(|i| 0.1 * (i as f32 * 0.0576).sin()).collect();
        let feed = |n| {
            for _ in 0..n {
                raw_tx.send((48_000, tone.clone(), Instant::now())).unwrap();
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        feed(10);
        assert!(wait_frames(&seen, 10) >= 10);
        assert_eq!(
            shared.take_levels().0,
            0.0,
            "idle: nothing cleaned, nothing metered"
        );
        shared.controls.lock().unwrap().watch_meters(true);
        feed(10);
        assert!(wait_frames(&seen, 20) >= 20);
        assert!(
            shared.take_levels().0 > 0.0,
            "meter open: the chain runs and the meter shows its level"
        );
        let s = shared.stats.take(0);
        assert_eq!((s.ns_atten_db, s.gate_open_pct), (None, None));
    }

    struct Loud;
    impl Denoiser for Loud {
        fn process(&mut self, block: &mut [f32]) -> Option<f32> {
            block.fill(0.9);
            Some(1.0)
        }
    }

    #[test]
    fn loud_input_with_max_gain_stays_in_range() {
        let mut c = MicChain::new(NsLevel::Off, false, 0.0, 4.0);
        c.set_denoiser(NsLevel::Standard, Box::new(Loud));
        let t0 = Instant::now();
        for k in 0..10 {
            let mut b = vec![0.9f32; FRAME];
            c.process(&mut b, true, t0 + Duration::from_millis(k * 10));
            assert!(
                b.iter().all(|v| v.abs() <= 1.0),
                "block {k} exceeds full scale"
            );
        }
    }

    #[test]
    fn mute_silences_immediately_including_preroll() {
        let mut c = MicChain::new(NsLevel::Off, false, 0.0, 1.0); // gate always open
        let t0 = Instant::now();
        for k in 0..5 {
            let mut b = vec![0.5f32; FRAME];
            c.process(&mut b, true, t0 + Duration::from_millis(k * 10));
        }
        for k in 5..10 {
            let mut b = vec![0.5f32; FRAME];
            let out = c.process(&mut b, false, t0 + Duration::from_millis(k * 10));
            assert!(
                b.iter().all(|v| *v == 0.0) && !out.sent,
                "block {k} leaked after mute"
            );
        }
    }

    /// Records the first sample of every block it is given and of every block it hands back.
    /// With `delay`, it returns the previous block, like a real denoiser's look-back buffers.
    struct Spy {
        seen: Rc<RefCell<Vec<f32>>>,
        emitted: Rc<RefCell<Vec<f32>>>,
        delay: Option<Vec<f32>>,
    }
    impl Denoiser for Spy {
        fn process(&mut self, block: &mut [f32]) -> Option<f32> {
            self.seen.borrow_mut().push(block[0]);
            if let Some(prev) = self.delay.as_mut() {
                let input = block.to_vec();
                block.copy_from_slice(prev);
                *prev = input;
            }
            self.emitted.borrow_mut().push(block[0]);
            Some(1.0)
        }
    }
    type Log = Rc<RefCell<Vec<f32>>>;
    fn spy(delay: bool) -> (Box<Spy>, Log, Log) {
        let (seen, emitted) = (Log::default(), Log::default());
        let d = Spy {
            seen: seen.clone(),
            emitted: emitted.clone(),
            delay: delay.then(|| vec![0.0; FRAME]),
        };
        (Box::new(d), seen, emitted)
    }

    #[test]
    fn idle_skips_the_denoiser_and_sends_silence() {
        let (d, seen, _) = spy(false);
        let mut c = MicChain::new(NsLevel::Off, false, 0.0, 1.0);
        c.set_denoiser(NsLevel::Standard, d);
        for k in 0..20 {
            let mut b = vec![0.5f32; FRAME];
            let out = c.idle(&mut b);
            assert!(
                b.iter().all(|v| *v == 0.0) && !out.sent && out.level == 0.0,
                "block {k} leaked while idle"
            );
            // like a muted block: nothing judged, nothing for the path line's ns/gate
            assert!(!out.judged && out.ns_in == 0.0 && out.ns_out == 0.0);
            assert!(
                (out.in_level - 0.5).abs() < 1e-6,
                "in_level is measured before zeroing"
            );
        }
        assert!(seen.borrow().is_empty(), "the denoiser ran while idle");
    }

    #[test]
    fn waking_replays_the_last_80ms_once_before_the_live_block() {
        let (d, seen, _) = spy(false);
        let mut c = MicChain::new(NsLevel::Off, false, 0.0, 1.0);
        c.set_denoiser(NsLevel::Standard, d);
        for k in 0..20 {
            let mut b = vec![k as f32 / 100.0; FRAME];
            c.idle(&mut b);
        }
        let mut live = vec![0.99f32; FRAME];
        c.process(&mut live, true, Instant::now());
        let want: Vec<f32> = (12..20).map(|k| k as f32 / 100.0).chain([0.99]).collect();
        assert_eq!(*seen.borrow(), want);
        let mut next = vec![0.5f32; FRAME];
        c.process(&mut next, true, Instant::now());
        assert_eq!(seen.borrow().len(), want.len() + 1, "replayed only once");
    }

    /// Muted, then the settings page opens (meter on, still muted): the denoiser wakes with the
    /// same replay, the meter gets the cleaned level, and still nothing goes out.
    #[test]
    fn waking_into_meter_only_replays_once_and_sends_nothing() {
        let (d, seen, _) = spy(false);
        let mut c = MicChain::new(NsLevel::Off, false, 0.0, 1.0);
        c.set_denoiser(NsLevel::Standard, d);
        for k in 0..20 {
            c.idle(&mut vec![k as f32 / 100.0; FRAME]);
        }
        let mut b = vec![0.5f32; FRAME];
        let out = c.process(&mut b, false, Instant::now());
        let want: Vec<f32> = (12..20).map(|k| k as f32 / 100.0).chain([0.5]).collect();
        assert_eq!(*seen.borrow(), want);
        assert!(!out.sent && !out.judged && b.iter().all(|v| *v == 0.0));
        assert!(
            (out.level - 0.5).abs() < 1e-3,
            "the meter shows {}",
            out.level
        );
        c.process(&mut vec![0.5f32; FRAME], false, Instant::now());
        assert_eq!(seen.borrow().len(), want.len() + 1, "replayed only once");
    }

    #[test]
    fn unmute_does_not_leak_audio_from_before_the_mute() {
        let (d, _, emitted) = spy(true);
        let mut c = MicChain::new(NsLevel::Off, false, 0.0, 1.0);
        c.set_denoiser(NsLevel::Standard, d);
        let t0 = Instant::now();
        for k in 0..5 {
            let mut b = vec![0.5f32; FRAME]; // talking
            c.process(&mut b, true, t0 + Duration::from_millis(k * 10));
        }
        for _ in 0..50 {
            let mut b = vec![0.01f32; FRAME]; // muted, quiet room
            c.idle(&mut b);
        }
        let mut b = vec![0.01f32; FRAME];
        c.process(&mut b, true, t0 + Duration::from_millis(600));
        let first_live = *emitted.borrow().last().unwrap();
        assert_eq!(
            first_live, 0.01,
            "the denoiser emitted pre-mute speech on unmute"
        );
    }

    #[test]
    fn meter_level_is_after_cleaning_and_gain() {
        let mut c = MicChain::new(NsLevel::Off, false, 0.0, 2.0);
        let mut b = vec![0.1f32; FRAME];
        let out = c.process(&mut b, true, Instant::now());
        assert!((out.level - 0.2).abs() < 1e-3, "{}", out.level);
    }

    /// Stands in for an engine whose cleaned output lags its input by `delay` blocks and whose
    /// decision level runs `lead` blocks ahead of that output (Strong: 3 and 2; Standard: 1 and 1).
    struct Lagging {
        history: VecDeque<Vec<f32>>,
        delay: usize,
        lead: usize,
        decision: f32,
    }

    impl Lagging {
        fn new(delay: usize, lead: usize) -> Self {
            Self {
                history: (0..delay).map(|_| vec![0.0; FRAME]).collect(),
                delay,
                lead,
                decision: 0.0,
            }
        }
    }

    impl Denoiser for Lagging {
        fn process(&mut self, block: &mut [f32]) -> Option<f32> {
            self.history.push_back(block.to_vec()); // delay + 1 blocks, newest last
            let lag = self.delay - self.lead;
            self.decision = rms(&self.history[self.history.len() - 1 - lag]);
            let out = self.history.pop_front().expect("delay line");
            block.copy_from_slice(&out);
            Some(if self.decision > 0.05 { 0.95 } else { 0.0 })
        }
        fn gate_lead_blocks(&self) -> usize {
            self.lead
        }
        fn decision_level(&self) -> Option<f32> {
            // no lag: that's the raw input, which the chain measures itself
            (self.delay > self.lead).then_some(self.decision)
        }
    }

    /// How many blocks after a loud onset enters does it leave the chain at full level? Feeds 1 s
    /// of quiet room first (long enough for an earlier onset's hold and fade to end).
    fn onset_lag_on(c: &mut MicChain, t0: Instant, k: &mut u64) -> usize {
        let mut step = |amp: f32| {
            let mut b = vec![amp; FRAME];
            c.process(&mut b, true, t0 + Duration::from_millis(*k * 10));
            *k += 1;
            rms(&b)
        };
        for _ in 0..100 {
            step(0.001); // learn the room
        }
        (0..20)
            .position(|_| step(0.1) > 0.09)
            .expect("the onset never got through")
    }

    fn onset_lag_with(auto: bool, delay: usize, lead: usize) -> usize {
        let mut c = MicChain::new(NsLevel::Off, auto, 0.02, 1.0);
        c.set_denoiser(NsLevel::Standard, Box::new(Lagging::new(delay, lead)));
        onset_lag_on(&mut c, Instant::now(), &mut 0)
    }

    fn onset_lag(delay: usize, lead: usize) -> usize {
        onset_lag_with(true, delay, lead)
    }

    /// The engine's lookahead is the gate's pre-roll: Strong's 30 ms, not 30 + 20.
    #[test]
    fn strong_onset_leaves_after_the_engine_delay_only() {
        assert_eq!(onset_lag(3, 2), 3, "Strong");
    }

    #[test]
    fn standard_onset_leaves_after_20ms_not_30() {
        assert_eq!(onset_lag(1, 1), 2, "Standard");
    }

    #[test]
    fn off_keeps_its_20ms_preroll() {
        assert_eq!(onset_lag(0, 0), 2, "Off");
    }

    /// Manual mode compares the threshold with what the meter shows, so it keeps deciding on the
    /// cleaned block behind the full 20 ms pre-roll: the engine's delay adds to it, as before.
    #[test]
    fn manual_onset_lag_is_unchanged() {
        assert_eq!(onset_lag_with(false, 3, 2), 5, "Strong");
        assert_eq!(onset_lag_with(false, 1, 1), 3, "Standard");
        assert_eq!(onset_lag_with(false, 0, 0), 2, "Off");
    }

    /// The manual threshold sits on the mic meter: noise the denoiser takes below it must not
    /// open the gate, however loud the raw input was.
    #[test]
    fn manual_threshold_judges_the_cleaned_level_the_meter_shows() {
        let mut c = MicChain::new(NsLevel::Off, false, 0.05, 1.0);
        c.set_denoiser(NsLevel::Standard, Box::new(Halve));
        let t0 = Instant::now();
        for k in 0..10 {
            let mut b = vec![0.08f32; FRAME]; // raw 0.08 is over; cleaned 0.04 is under
            let out = c.process(&mut b, true, t0 + Duration::from_millis(k * 10));
            assert!(out.level < 0.05 && !out.sent, "block {k} opened the gate");
        }
    }

    /// Switching engines mid-stream resizes the gate's pre-roll without panicking, and each
    /// engine's onset lag holds after the switch.
    #[test]
    fn switching_engines_mid_stream_settles_to_each_engines_lag() {
        let mut c = MicChain::new(NsLevel::Off, true, 0.02, 1.0);
        let (t0, mut k) = (Instant::now(), 0);
        c.set_denoiser(NsLevel::Standard, Box::new(Lagging::new(1, 1)));
        assert_eq!(onset_lag_on(&mut c, t0, &mut k), 2, "Standard");
        c.set_denoiser(NsLevel::Strong, Box::new(Lagging::new(3, 2)));
        assert_eq!(onset_lag_on(&mut c, t0, &mut k), 3, "Strong");
        c.set_denoiser(NsLevel::Standard, Box::new(Lagging::new(1, 1)));
        assert_eq!(onset_lag_on(&mut c, t0, &mut k), 2, "back to Standard");
    }

    /// Settings flip auto ↔ manual mid-call: the pre-roll follows the mode each time (auto's
    /// comes from the engine's delay; manual keeps the full 20 ms behind it).
    #[test]
    fn switching_auto_and_manual_mid_stream_settles_to_each_modes_lag() {
        let mut c = MicChain::new(NsLevel::Off, true, 0.02, 1.0);
        let (t0, mut k) = (Instant::now(), 0);
        c.set_denoiser(NsLevel::Strong, Box::new(Lagging::new(3, 2)));
        assert_eq!(onset_lag_on(&mut c, t0, &mut k), 3, "auto");
        c.configure(false, 0.02, 1.0);
        assert_eq!(onset_lag_on(&mut c, t0, &mut k), 5, "manual");
        c.configure(true, 0.02, 1.0);
        assert_eq!(onset_lag_on(&mut c, t0, &mut k), 3, "auto again");
    }

    /// Mute and unmute after an engine switch: nothing leaks while muted (meter open or not), and
    /// the new engine's onset lag holds after unmuting.
    #[test]
    fn mute_and_unmute_after_an_engine_switch() {
        let mut c = MicChain::new(NsLevel::Off, true, 0.02, 1.0);
        let (t0, mut k) = (Instant::now(), 0);
        c.set_denoiser(NsLevel::Standard, Box::new(Lagging::new(1, 1)));
        assert_eq!(onset_lag_on(&mut c, t0, &mut k), 2, "Standard");
        c.set_denoiser(NsLevel::Strong, Box::new(Lagging::new(3, 2)));
        for _ in 0..20 {
            // muted with the meter open: cleaned and metered, not sent
            let mut b = vec![0.1f32; FRAME];
            let out = c.process(&mut b, false, t0 + Duration::from_millis(k * 10));
            k += 1;
            assert!(
                !out.sent && b.iter().all(|v| *v == 0.0),
                "leaked while muted"
            );
        }
        for _ in 0..20 {
            c.idle(&mut vec![0.001f32; FRAME]); // muted, meter closed
            k += 1;
        }
        assert_eq!(onset_lag_on(&mut c, t0, &mut k), 3, "Strong after unmute");
    }

    #[test]
    fn block_out_reports_the_level_before_cleaning() {
        let mut c = MicChain::new(NsLevel::Off, false, 0.0, 2.0);
        let mut b = vec![0.1f32; FRAME];
        let out = c.process(&mut b, true, Instant::now());
        assert!((out.in_level - 0.1).abs() < 1e-3 && (out.level - 0.2).abs() < 1e-3);
    }
    use crate::voice::devices::{AudioConfig, Shared};
    use std::sync::{Arc, Mutex};

    fn shared_with(cfg: AudioConfig) -> Arc<Shared> {
        Shared::new(&cfg, Default::default(), 200)
    }

    type Seen = Arc<Mutex<Vec<(usize, Option<String>)>>>;

    /// Records each frame's length and the thread it arrived on.
    struct Collect(Seen);

    impl FrameSink for Collect {
        fn send(&mut self, frame: &[i16]) -> bool {
            let thread = std::thread::current().name().map(String::from);
            self.0.lock().unwrap().push((frame.len(), thread));
            true
        }
    }

    fn collecting(sh: &Arc<Shared>) -> Seen {
        let seen: Seen = Default::default();
        sh.set_mic_sink(Some(Box::new(Collect(seen.clone()))));
        seen
    }

    fn wait_frames(seen: &Seen, n: usize) -> usize {
        let deadline = Instant::now() + Duration::from_secs(5);
        while seen.lock().unwrap().len() < n && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        seen.lock().unwrap().len()
    }

    fn off() -> Arc<Shared> {
        shared_with(AudioConfig {
            noise_suppression: NsLevel::Off,
            ..Default::default()
        })
    }

    #[test]
    fn frames_go_straight_to_the_sink_on_the_processor_thread() {
        let sh = off();
        let seen = collecting(&sh);
        let (raw_tx, _h) = spawn(sh.clone());
        for _ in 0..10 {
            raw_tx
                .send((48_000, vec![0.0; 480], Instant::now()))
                .unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(wait_frames(&seen, 10) >= 10);
        let seen = seen.lock().unwrap();
        assert!(
            seen.iter()
                .all(|(n, t)| *n == 480 && t.as_deref() == Some("pulse-mic")),
            "{seen:?}"
        );
    }

    /// Before the room is connected there's nowhere to send: frames are dropped, never queued
    /// (whatever piled up would be sent late for the rest of the call).
    #[test]
    fn without_a_room_frames_are_dropped_not_queued() {
        let sh = off();
        let (raw_tx, _h) = spawn(sh.clone());
        for _ in 0..30 {
            raw_tx
                .send((48_000, vec![0.0; 480], Instant::now()))
                .unwrap();
        }
        std::thread::sleep(Duration::from_millis(100));
        let seen = collecting(&sh);
        raw_tx
            .send((48_000, vec![0.0; 480], Instant::now()))
            .unwrap();
        wait_frames(&seen, 1);
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "only the frame after the room connected"
        );
    }

    /// Silence the room refuses isn't silence sent: it counts as send drops, like a refused
    /// mic frame, so the path line's `fill` stays what listeners actually got.
    #[test]
    fn refused_silence_counts_as_send_drops_not_fill() {
        struct Refuse;
        impl FrameSink for Refuse {
            fn send(&mut self, _: &[i16]) -> bool {
                false
            }
        }
        use std::sync::atomic::Ordering::Relaxed;
        let sh = off();
        sh.set_mic_sink(Some(Box::new(Refuse)));
        let mut gap = GapFill::new(Instant::now() - Duration::from_millis(105));
        fill_gap(&sh, &mut gap, &[0; FRAME]);
        assert_eq!(sh.stats.silence_fill.load(Relaxed), 0);
        assert_eq!(sh.stats.send_drops.load(Relaxed), 10);
    }

    /// A room switch swaps the sink: the old one (a LiveKit source) is dropped after the sink
    /// lock is released, so its teardown can't hold up the processor's next frame.
    #[test]
    fn the_old_mic_sink_is_dropped_outside_the_lock() {
        use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
        struct Probe(std::sync::Weak<Shared>, Arc<AtomicBool>);
        impl FrameSink for Probe {
            fn send(&mut self, _: &[i16]) -> bool {
                true
            }
        }
        impl Drop for Probe {
            fn drop(&mut self) {
                if let Some(s) = self.0.upgrade() {
                    self.1.store(s.mic_sink.try_lock().is_ok(), SeqCst);
                }
            }
        }
        let sh = off();
        let free = Arc::new(AtomicBool::new(false));
        sh.set_mic_sink(Some(Box::new(Probe(Arc::downgrade(&sh), free.clone()))));
        sh.set_mic_sink(None);
        assert!(free.load(SeqCst), "dropped while the sink lock was held");
    }

    /// The mic stalls (a device reopen): the stream to WebRTC stays continuous with silence.
    #[test]
    fn a_stalled_mic_is_covered_with_silence() {
        let sh = off();
        let seen = collecting(&sh);
        let (raw_tx, _h) = spawn(sh.clone());
        for _ in 0..5 {
            raw_tx
                .send((48_000, vec![0.0; 480], Instant::now()))
                .unwrap();
        }
        std::thread::sleep(Duration::from_millis(250));
        assert!(
            wait_frames(&seen, 5 + 10) >= 15,
            "{} frames",
            seen.lock().unwrap().len()
        );
        assert!(
            sh.stats
                .silence_fill
                .load(std::sync::atomic::Ordering::Relaxed)
                >= 10
        );
        drop(raw_tx);
    }

    fn all_480(seen: &Seen) -> bool {
        seen.lock().unwrap().iter().all(|(n, _)| *n == 480)
    }

    #[test]
    fn processor_emits_exact_48k_frames() {
        let sh = off();
        let seen = collecting(&sh);
        let (raw_tx, _h) = spawn(sh.clone());
        for _ in 0..10 {
            raw_tx
                .send((48_000, vec![0.0; 480], Instant::now()))
                .unwrap();
            std::thread::sleep(Duration::from_millis(5)); // devices deliver over time, not in a burst
        }
        assert!(wait_frames(&seen, 10) >= 10);
        assert!(all_480(&seen), "{:?}", seen.lock().unwrap());
    }

    #[test]
    fn processor_handles_rate_change() {
        let sh = off();
        let seen = collecting(&sh);
        let (raw_tx, _h) = spawn(sh.clone());
        for _ in 0..50 {
            raw_tx
                .send((16_000, vec![0.0; 160], Instant::now()))
                .unwrap(); // BT call mode
            std::thread::sleep(Duration::from_millis(2));
        }
        for _ in 0..50 {
            raw_tx
                .send((48_000, vec![0.0; 480], Instant::now()))
                .unwrap();
            std::thread::sleep(Duration::from_millis(2));
        }
        let got = wait_frames(&seen, 90);
        assert!(got >= 90 && all_480(&seen), "{got}");
    }

    #[test]
    fn processor_exits_when_senders_drop() {
        let (raw_tx, h) = spawn(shared_with(AudioConfig::default()));
        drop(raw_tx);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !h.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(h.is_finished(), "processor thread outlived its senders");
    }

    /// Devices with big buffers deliver 100 ms of audio per callback: a burst of 10 blocks at once.
    /// That's not lag, so nothing may be dropped (review I1). Silence fills the gaps between bursts.
    #[test]
    fn bursty_device_callbacks_lose_nothing() {
        let sh = off();
        let seen = collecting(&sh);
        let (raw_tx, _h) = spawn(sh.clone());
        for _ in 0..3 {
            for _ in 0..10 {
                raw_tx
                    .send((48_000, vec![0.0; 480], Instant::now()))
                    .unwrap();
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(wait_frames(&seen, 30) >= 30);
        assert_eq!(
            sh.stats
                .lag_drops
                .load(std::sync::atomic::Ordering::Relaxed),
            0,
            "a burst is not a backlog"
        );
    }

    #[test]
    fn dead_strong_worker_reports_failure_and_passes_audio() {
        let (tx, worker_in) = std::sync::mpsc::channel::<Vec<f32>>();
        drop(worker_in); // the worker thread is gone
        let (_out_tx, rx) = std::sync::mpsc::channel();
        let mut w = StrongWorker {
            tx,
            rx,
            dead: false,
            level: None,
            lead: 2,
        };
        let mut b = vec![0.3f32; FRAME];
        w.process(&mut b);
        assert!(w.failed());
        assert_eq!(b, vec![0.3f32; FRAME], "audio passes through untouched");
    }

    /// A worker that dies mid-call passes the raw block: the gate must judge that block's own
    /// level, not the last level the worker reported before it died.
    #[test]
    fn dead_strong_worker_forgets_its_last_level() {
        let (tx, worker_in) = std::sync::mpsc::channel::<Vec<f32>>();
        drop(worker_in);
        let (_out_tx, rx) = std::sync::mpsc::channel();
        let mut w = StrongWorker {
            tx,
            rx,
            dead: false,
            level: Some(0.5), // loud speech, just before it died
            lead: 2,
        };
        w.process(&mut vec![0.0f32; FRAME]);
        assert_eq!(w.decision_level(), None, "the raw block's level decides");
    }

    struct Dead;
    impl Denoiser for Dead {
        fn process(&mut self, _block: &mut [f32]) -> Option<f32> {
            None
        }
        fn failed(&self) -> bool {
            true
        }
    }

    /// Review I2: a crashed Strong must not silently mean "no suppression" while settings say Strong.
    #[test]
    fn failed_denoiser_falls_back_to_standard_with_a_note() {
        let sh = shared_with(AudioConfig::default());
        let mut c = MicChain::new(NsLevel::Off, true, 0.02, 1.0);
        c.set_denoiser(NsLevel::Strong, Box::new(Dead));
        assert!(recover_failed(&mut c, &sh));
        assert_eq!(c.level(), NsLevel::Standard);
        let st = sh.take_ns_status_change().unwrap();
        assert_eq!(st.active, NsLevel::Standard);
        assert!(st.note.unwrap().contains("stopped working"));
        assert!(!recover_failed(&mut c, &sh), "Standard is healthy");
    }

    #[test]
    fn backlog_drops_only_audio_older_than_60ms() {
        let now = Instant::now();
        let at = |ms: u64| now - Duration::from_millis(ms);
        let mut q: VecDeque<RawBlock> = [200, 120, 61, 59, 10, 0]
            .into_iter()
            .map(|ms| (48_000, vec![0.0; 480], at(ms)))
            .collect();
        assert_eq!(trim_backlog(&mut q, now), 3);
        assert_eq!(q.len(), 3, "keeps everything within 60 ms");
    }

    #[test]
    fn stale_model_load_is_ignored() {
        let mut s = Switcher::new(NsLevel::Off);
        let first = s.request(NsLevel::Strong); // starts a load, generation g1
        s.request(NsLevel::Off); // user flips back before it finishes
        assert!(
            !s.accept_loaded(first, Model::Normal),
            "an outdated load must not be installed"
        );
        let second = s.request(NsLevel::Strong);
        assert!(s.accept_loaded(second, Model::Normal));
    }

    #[test]
    fn strong_load_failure_falls_back_to_standard_with_a_note() {
        let sh = shared_with(AudioConfig::default());
        report_load_failure(&sh, "boom");
        let st = sh.take_ns_status_change().unwrap();
        assert_eq!(st.active, NsLevel::Standard);
        assert!(st.note.unwrap().contains("Strong unavailable"));
    }

    #[test]
    fn strong_worker_cleans_on_its_own_thread() {
        let (tx, rx) = std::sync::mpsc::channel();
        start_strong(7, Model::Normal, None, tx);
        let (g, model, worker) = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("load finishes");
        assert_eq!((g, model), (7, Model::Normal));
        let mut w = worker.expect("model loads");
        assert_eq!(w.gate_lead_blocks(), 2, "Strong's lead");
        let noisy: Vec<f32> = (0..FRAME)
            .map(|i| if i % 3 == 0 { 0.05 } else { -0.03 })
            .collect();
        let mut probs = Vec::new();
        let mut last = noisy.clone();
        for _ in 0..50 {
            last = noisy.clone();
            probs.push(w.process(&mut last));
        }
        assert!(
            probs
                .iter()
                .all(|p| p.is_some_and(|p| (0.0..=1.0).contains(&p)))
        );
        assert_ne!(last, noisy, "the model changed the audio");
    }

    // --- Strong (low delay) and its fallback chain (#77) ---

    use crate::models::ModelStore;

    /// A low-delay model store in a temp dir (empty: not downloaded).
    fn ll_store() -> (tempfile::TempDir, Arc<ModelStore>) {
        let dir = tempfile::tempdir().unwrap();
        let s = Arc::new(ModelStore::low_delay(dir.path().join("models")));
        (dir, s)
    }

    /// Waits for a model load to land (installed or fallen back).
    fn settle(ns: &mut NsControl, chain: &mut MicChain, sh: &Shared) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !ns.take_loaded(chain, sh) {
            assert!(Instant::now() < deadline, "no model load finished");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn status(sh: &Shared) -> (NsLevel, Option<String>) {
        let st = sh.take_ns_status_change().expect("a status change");
        (st.active, st.note)
    }

    fn off_chain() -> MicChain {
        MicChain::new(NsLevel::Off, true, 0.02, 1.0)
    }

    #[test]
    fn low_delay_not_downloaded_runs_strong_with_a_note() {
        let sh = shared_with(AudioConfig::default());
        let (_d, store) = ll_store();
        let mut ns = NsControl::new(NsLevel::StrongLowDelay, Some(store));
        let mut c = off_chain();
        ns.choose(NsLevel::StrongLowDelay, &mut c, &sh);
        assert_eq!(
            status(&sh),
            (NsLevel::Standard, Some(LL_MISSING.into())),
            "Standard while Strong loads"
        );
        settle(&mut ns, &mut c, &sh);
        assert_eq!(c.level(), NsLevel::Strong);
        assert_eq!(status(&sh), (NsLevel::Strong, Some(LL_MISSING.into())));
        assert_eq!(
            ns.switcher().wanted(),
            NsLevel::StrongLowDelay,
            "the choice is kept"
        );
    }

    #[test]
    fn without_a_model_store_low_delay_is_missing() {
        let sh = shared_with(AudioConfig::default());
        let mut ns = NsControl::new(NsLevel::Off, None);
        let mut c = off_chain();
        ns.choose(NsLevel::StrongLowDelay, &mut c, &sh);
        assert_eq!(status(&sh).1.as_deref(), Some(LL_MISSING));
        assert_eq!(ns.switcher().target(), NsLevel::Strong);
    }

    /// Picked while Strong runs and not downloaded: Strong carries on (no reload, no Standard in between), and when
    /// the download finishes mid-call low delay replaces it, Strong still running until it has loaded.
    #[test]
    fn a_download_mid_call_swaps_strong_for_low_delay() {
        let sh = shared_with(AudioConfig::default());
        let (_d, store) = ll_store();
        let mut ns = NsControl::new(NsLevel::Strong, Some(store.clone()));
        let mut c = off_chain();
        ns.choose(NsLevel::Strong, &mut c, &sh);
        settle(&mut ns, &mut c, &sh);
        assert_eq!(c.level(), NsLevel::Strong);
        sh.take_ns_status_change();

        ns.choose(NsLevel::StrongLowDelay, &mut c, &sh);
        assert_eq!(c.level(), NsLevel::Strong, "Strong keeps running");
        assert_eq!(status(&sh), (NsLevel::Strong, Some(LL_MISSING.into())));
        assert!(!ns.take_loaded(&mut c, &sh), "nothing was reloaded");

        assert!(!ns.check_models(&c, &sh), "nothing new downloaded");
        store.put_for_test(&crate::models::checkout_low_delay_model());
        assert!(ns.check_models(&c, &sh), "the download starts a load");
        assert_eq!(c.level(), NsLevel::Strong, "Strong runs while it loads");
        settle(&mut ns, &mut c, &sh);
        assert_eq!(c.level(), NsLevel::StrongLowDelay);
        assert_eq!(status(&sh), (NsLevel::StrongLowDelay, None));
        assert!(!ns.check_models(&c, &sh), "seen once");
    }

    /// A model on disk that isn't the pinned one: deleted, reported as failed, and Strong runs.
    #[test]
    fn a_damaged_low_delay_model_falls_back_to_strong() {
        let sh = shared_with(AudioConfig::default());
        let (_d, store) = ll_store();
        store.put_for_test(b"not the model");
        let mut ns = NsControl::new(NsLevel::Off, Some(store.clone()));
        let mut c = off_chain();
        ns.choose(NsLevel::StrongLowDelay, &mut c, &sh);
        settle(&mut ns, &mut c, &sh);
        assert_eq!(
            status(&sh),
            (NsLevel::Standard, Some(LL_UNAVAILABLE.into())),
            "Standard while Strong loads"
        );
        assert!(!store.present(), "the damaged file is deleted");
        assert!(matches!(
            store.status(),
            crate::models::ModelStatus::Failed { .. }
        ));
        settle(&mut ns, &mut c, &sh);
        assert_eq!(c.level(), NsLevel::Strong);
        assert_eq!(status(&sh), (NsLevel::Strong, Some(LL_UNAVAILABLE.into())));
    }

    /// Too heavy: low delay → Strong (with a note), and then Strong → Standard, as before.
    #[test]
    fn overload_steps_down_low_delay_then_strong() {
        let sh = shared_with(AudioConfig::default());
        let (_d, store) = ll_store();
        store.put_for_test(&crate::models::checkout_low_delay_model());
        let mut ns = NsControl::new(NsLevel::Off, Some(store));
        let mut c = off_chain();
        ns.choose(NsLevel::StrongLowDelay, &mut c, &sh);
        assert!(!ns.measures(&c), "not measured while Standard stands in");
        settle(&mut ns, &mut c, &sh);
        assert_eq!(c.level(), NsLevel::StrongLowDelay);
        assert_eq!(c.denoiser.gate_lead_blocks(), 0, "low delay's lead");
        assert!(ns.measures(&c));
        sh.take_ns_status_change();

        ns.overloaded(&mut c, &sh);
        assert_eq!(status(&sh), (NsLevel::Standard, Some(LL_TOO_HEAVY.into())));
        assert!(!ns.measures(&c));
        settle(&mut ns, &mut c, &sh);
        assert_eq!(c.level(), NsLevel::Strong);
        assert_eq!(status(&sh), (NsLevel::Strong, Some(LL_TOO_HEAVY.into())));
        assert!(ns.measures(&c));

        ns.overloaded(&mut c, &sh);
        assert_eq!(c.level(), NsLevel::Standard);
        assert_eq!(
            status(&sh),
            (NsLevel::Standard, Some(STRONG_TOO_HEAVY.into()))
        );
        assert!(!ns.measures(&c), "Standard isn't measured");
    }

    #[test]
    fn a_dead_low_delay_worker_falls_back_to_strong() {
        let sh = shared_with(AudioConfig::default());
        let mut ns = NsControl::new(NsLevel::Off, None);
        let mut c = off_chain();
        ns.switcher.request(NsLevel::StrongLowDelay);
        c.set_denoiser(NsLevel::StrongLowDelay, Box::new(Dead));
        assert!(ns.recover(&mut c, &sh));
        assert_eq!(status(&sh), (NsLevel::Standard, Some(LL_STOPPED.into())));
        settle(&mut ns, &mut c, &sh);
        assert_eq!(c.level(), NsLevel::Strong);
        assert!(!ns.recover(&mut c, &sh), "Strong is healthy");
    }

    /// Changing level while low delay loads: that load is dropped, not installed.
    #[test]
    fn a_low_delay_load_outdated_by_a_new_choice_is_dropped() {
        let sh = shared_with(AudioConfig::default());
        let (_d, store) = ll_store();
        store.put_for_test(&crate::models::checkout_low_delay_model());
        let mut ns = NsControl::new(NsLevel::Off, Some(store));
        let mut c = off_chain();
        ns.choose(NsLevel::StrongLowDelay, &mut c, &sh);
        ns.choose(NsLevel::Off, &mut c, &sh);
        std::thread::sleep(Duration::from_secs(3));
        assert!(!ns.take_loaded(&mut c, &sh));
        assert_eq!(c.level(), NsLevel::Off);
    }

    /// Back from low delay to Strong: low delay carries on until Strong has loaded.
    #[test]
    fn low_delay_runs_until_strong_has_loaded() {
        let sh = shared_with(AudioConfig::default());
        let (_d, store) = ll_store();
        store.put_for_test(&crate::models::checkout_low_delay_model());
        let mut ns = NsControl::new(NsLevel::Off, Some(store));
        let mut c = off_chain();
        ns.choose(NsLevel::StrongLowDelay, &mut c, &sh);
        settle(&mut ns, &mut c, &sh);
        ns.choose(NsLevel::Strong, &mut c, &sh);
        assert_eq!(c.level(), NsLevel::StrongLowDelay);
        settle(&mut ns, &mut c, &sh);
        assert_eq!(c.level(), NsLevel::Strong);
    }
}
