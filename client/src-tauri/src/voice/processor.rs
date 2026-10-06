//! Mic processing off the audio callback: resample → APM → denoiser → gain → gate, per 10 ms block.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use super::denoise::{DeepFilter, Denoiser, FRAME, NsLevel, make_fast};
use super::devices::{INTERNAL_RATE, MicChunk, NsStatus, Shared};
use super::gate::VoiceGate;
use super::mixer::soft_limit;
use super::resampler::StreamResampler;

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
}

/// One block's journey after APM: denoise → gain (soft-limited) → gate → mute.
pub struct MicChain {
    level: NsLevel,
    denoiser: Box<dyn Denoiser>,
    gate: VoiceGate,
    gain: f32,
}

impl MicChain {
    pub fn new(level: NsLevel, auto: bool, threshold: f32, gain: f32) -> Self {
        Self {
            level,
            denoiser: make_fast(level),
            gate: VoiceGate::new(auto, threshold),
            gain,
        }
    }

    pub fn level(&self) -> NsLevel {
        self.level
    }

    pub fn set_denoiser(&mut self, level: NsLevel, d: Box<dyn Denoiser>) {
        self.level = level;
        self.denoiser = d;
    }

    pub fn configure(&mut self, auto: bool, threshold: f32, gain: f32) {
        self.gate.configure(auto, threshold);
        self.gain = gain;
    }

    pub fn process(&mut self, block: &mut [f32], mic_open: bool, now: Instant) -> BlockOut {
        let prob = self.denoiser.process(block);
        for v in block.iter_mut() {
            *v = soft_limit(*v * self.gain);
        }
        let level = (block.iter().map(|v| v * v).sum::<f32>() / block.len() as f32).sqrt();
        if !mic_open {
            self.gate.reset();
            block.fill(0.0);
            return BlockOut { level, sent: false };
        }
        let sent = self.gate.process(block, prob, now);
        BlockOut { level, sent }
    }
}

pub type RawBlock = (u32, Vec<f32>);
pub type RawTx = std_mpsc::Sender<RawBlock>;

/// At most 60 ms of raw audio may wait; older blocks are dropped so latency never grows.
const MAX_BACKLOG: usize = 6;

pub fn trim_backlog<T>(q: &mut VecDeque<T>) -> usize {
    let excess = q.len().saturating_sub(MAX_BACKLOG);
    q.drain(..excess);
    excess
}

/// Tracks which level is wanted, so a slow model load can't overwrite a newer choice.
pub struct Switcher {
    wanted: NsLevel,
    generation: u64,
}

impl Switcher {
    pub fn new(level: NsLevel) -> Self {
        Self {
            wanted: level,
            generation: 0,
        }
    }

    /// Record a new wanted level; returns the generation a load for it must present.
    pub fn request(&mut self, level: NsLevel) -> u64 {
        self.wanted = level;
        self.generation += 1;
        self.generation
    }

    pub fn wanted(&self) -> NsLevel {
        self.wanted
    }

    pub fn accept_loaded(&self, generation: u64) -> bool {
        generation == self.generation && self.wanted == NsLevel::Strong
    }
}

pub fn report_load_failure(shared: &Shared, err: &str) {
    tracing::warn!(
        error = err,
        "noise suppression: Strong failed to load, using Standard"
    );
    shared.set_ns_status(NsStatus {
        active: NsLevel::Standard,
        note: Some("Strong unavailable, using Standard".into()),
    });
}

/// Strong's model can't change threads (DeepFilterNet's tract plan isn't `Send`), so it lives on
/// its own worker thread, which loads it and cleans blocks there. This handle round-trips each
/// block; dropping it ends the worker and frees the model.
pub struct StrongWorker {
    tx: std_mpsc::Sender<Vec<f32>>,
    rx: std_mpsc::Receiver<(Vec<f32>, Option<f32>)>,
}

impl Denoiser for StrongWorker {
    fn process(&mut self, block: &mut [f32]) -> Option<f32> {
        if self.tx.send(block.to_vec()).is_err() {
            return None; // worker gone: pass audio through
        }
        let (out, p) = self.rx.recv().ok()?;
        block.copy_from_slice(&out);
        p
    }
}

type LoadResult = (u64, anyhow::Result<StrongWorker>);

/// Load Strong on a new worker thread; the result (tagged with `generation`) arrives on `done`.
pub fn start_strong(generation: u64, done: std_mpsc::Sender<LoadResult>) {
    let spawned = std::thread::Builder::new()
        .name("pulse-strong".into())
        .spawn(move || match DeepFilter::new() {
            Err(e) => {
                let _ = done.send((generation, Err(e)));
            }
            Ok(mut df) => {
                let (in_tx, in_rx) = std_mpsc::channel::<Vec<f32>>();
                let (out_tx, out_rx) = std_mpsc::channel();
                let handle = StrongWorker {
                    tx: in_tx,
                    rx: out_rx,
                };
                if done.send((generation, Ok(handle))).is_err() {
                    return;
                }
                while let Ok(mut block) = in_rx.recv() {
                    let p = df.process(&mut block);
                    if out_tx.send((block, p)).is_err() {
                        return;
                    }
                }
            }
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "noise suppression: couldn't start the Strong worker");
    }
}

/// Start the mic processor. It runs until every returned sender is dropped.
pub fn spawn(
    shared: Arc<Shared>,
    mic_tx: mpsc::UnboundedSender<MicChunk>,
) -> (RawTx, std::thread::JoinHandle<()>) {
    let (tx, rx) = std_mpsc::channel::<RawBlock>();
    let h = std::thread::Builder::new()
        .name("pulse-mic".into())
        .spawn(move || run(shared, rx, mic_tx))
        .expect("spawn mic processor");
    (tx, h)
}

fn to_i16(v: f32) -> i16 {
    (v * 32767.0).round().clamp(-32768.0, 32767.0) as i16
}

/// While Strong loads (or if it's unavailable), Standard runs in its place.
fn shown(level: NsLevel) -> NsLevel {
    if level == NsLevel::Strong {
        NsLevel::Standard
    } else {
        level
    }
}

fn run(
    shared: Arc<Shared>,
    rx: std_mpsc::Receiver<RawBlock>,
    mic_tx: mpsc::UnboundedSender<MicChunk>,
) {
    let cfg = shared.proc_cfg.lock().unwrap().clone();
    let mut version = cfg.version;
    let mut chain = MicChain::new(cfg.level, cfg.auto, cfg.threshold, cfg.gain);
    let mut switcher = Switcher::new(cfg.level);
    let (load_tx, load_rx) = std_mpsc::channel::<LoadResult>();
    if cfg.level == NsLevel::Strong {
        start_strong(switcher.request(NsLevel::Strong), load_tx.clone());
    }
    shared.set_ns_status(NsStatus {
        active: shown(cfg.level),
        note: None,
    });

    let mut overload = OverloadDetector::new();
    let mut resampler: Option<(u32, StreamResampler)> = None;
    let mut pending: Vec<f32> = Vec::with_capacity(2 * FRAME);
    let mut queue: VecDeque<RawBlock> = VecDeque::new();
    let mut i16buf = vec![0i16; FRAME];

    while let Ok(first) = rx.recv() {
        queue.push_back(first);
        queue.extend(rx.try_iter());
        trim_backlog(&mut queue);

        // live config
        let cfg = shared.proc_cfg.lock().unwrap().clone();
        if cfg.version != version {
            version = cfg.version;
            chain.configure(cfg.auto, cfg.threshold, cfg.gain);
            if cfg.level != switcher.wanted() {
                let g = switcher.request(cfg.level);
                chain.set_denoiser(cfg.level, make_fast(cfg.level));
                overload.reset();
                if cfg.level == NsLevel::Strong {
                    start_strong(g, load_tx.clone());
                }
                shared.set_ns_status(NsStatus {
                    active: shown(cfg.level),
                    note: None,
                });
            }
        }
        // a finished model load
        while let Ok((g, result)) = load_rx.try_recv() {
            match result {
                Ok(worker) if switcher.accept_loaded(g) => {
                    chain.set_denoiser(NsLevel::Strong, Box::new(worker));
                    overload.reset();
                    shared.set_ns_status(NsStatus {
                        active: NsLevel::Strong,
                        note: None,
                    });
                }
                Ok(_) => {} // outdated: the user changed level meanwhile (dropping it frees the model)
                Err(e) if switcher.accept_loaded(g) => report_load_failure(&shared, &e.to_string()),
                Err(_) => {}
            }
        }

        for (rate, samples) in queue.drain(..) {
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
            // WebRTC APM (HPF, AEC, AGC) works on i16
            for (d, v) in i16buf.iter_mut().zip(&block) {
                *d = to_i16(*v);
            }
            let r = shared
                .apm
                .lock()
                .unwrap()
                .process_stream(&mut i16buf, INTERNAL_RATE as i32, 1);
            shared.warn_apm_once(r, "capture");
            for (d, v) in block.iter_mut().zip(&i16buf) {
                *d = *v as f32 / 32767.0;
            }

            let mic_open = shared.controls.lock().unwrap().mic_open();
            let t = Instant::now();
            let out = chain.process(&mut block, mic_open, t);
            if chain.level() == NsLevel::Strong
                && switcher.wanted() == NsLevel::Strong
                && overload.record(t.elapsed())
            {
                tracing::warn!(
                    "noise suppression: this PC can't keep up with Strong, switching to Standard"
                );
                chain.set_denoiser(NsLevel::Standard, make_fast(NsLevel::Standard));
                overload.reset();
                shared.set_ns_status(NsStatus {
                    active: NsLevel::Standard,
                    note: Some("Strong was too heavy for this PC, using Standard".into()),
                });
            }
            shared.add_mic_level(out.level, out.sent);
            let chunk: Vec<i16> = block.iter().map(|v| to_i16(*v)).collect();
            if mic_tx.send((INTERNAL_RATE, chunk)).is_err() {
                return; // session gone
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::denoise::{Denoiser, FRAME, NsLevel};
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

    #[test]
    fn meter_level_is_after_cleaning_and_gain() {
        let mut c = MicChain::new(NsLevel::Off, false, 0.0, 2.0);
        let mut b = vec![0.1f32; FRAME];
        let out = c.process(&mut b, true, Instant::now());
        assert!((out.level - 0.2).abs() < 1e-3, "{}", out.level);
    }
    use crate::voice::devices::{AudioConfig, Shared};
    use crate::voice::mixer::Mixer;
    use std::sync::{Arc, Mutex};

    fn shared_with(cfg: AudioConfig) -> Arc<Shared> {
        Shared::new(
            &cfg,
            Default::default(),
            Arc::new(Mutex::new(Mixer::new(48_000, 200))),
        )
    }

    fn recv_n(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::voice::devices::MicChunk>,
        n: usize,
    ) -> Vec<(u32, usize)> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut got = Vec::new();
        while got.len() < n && Instant::now() < deadline {
            match rx.try_recv() {
                Ok((rate, buf)) => got.push((rate, buf.len())),
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        got
    }

    #[test]
    fn processor_emits_exact_48k_frames() {
        let (mic_tx, mut mic_rx) = tokio::sync::mpsc::unbounded_channel();
        let (raw_tx, _h) = spawn(
            shared_with(AudioConfig {
                noise_suppression: NsLevel::Off,
                ..Default::default()
            }),
            mic_tx,
        );
        for _ in 0..10 {
            raw_tx.send((48_000, vec![0.0; 480])).unwrap();
            std::thread::sleep(Duration::from_millis(5)); // devices deliver over time, not in a burst
        }
        assert_eq!(recv_n(&mut mic_rx, 10), vec![(48_000, 480); 10]);
    }

    #[test]
    fn processor_handles_rate_change() {
        let (mic_tx, mut mic_rx) = tokio::sync::mpsc::unbounded_channel();
        let (raw_tx, _h) = spawn(
            shared_with(AudioConfig {
                noise_suppression: NsLevel::Off,
                ..Default::default()
            }),
            mic_tx,
        );
        for _ in 0..50 {
            raw_tx.send((16_000, vec![0.0; 160])).unwrap(); // BT call mode
            std::thread::sleep(Duration::from_millis(2));
        }
        for _ in 0..50 {
            raw_tx.send((48_000, vec![0.0; 480])).unwrap();
            std::thread::sleep(Duration::from_millis(2));
        }
        let got = recv_n(&mut mic_rx, 90);
        assert!(
            got.len() >= 90 && got.iter().all(|c| *c == (48_000, 480)),
            "{:?}",
            got.len()
        );
    }

    #[test]
    fn processor_exits_when_senders_drop() {
        let (mic_tx, _mic_rx) = tokio::sync::mpsc::unbounded_channel();
        let (raw_tx, h) = spawn(shared_with(AudioConfig::default()), mic_tx);
        drop(raw_tx);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !h.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(h.is_finished(), "processor thread outlived its senders");
    }

    #[test]
    fn backlog_is_trimmed_to_60ms() {
        let mut q: VecDeque<u32> = (0..20).collect();
        let dropped = trim_backlog(&mut q);
        assert_eq!(dropped, 14);
        assert_eq!(q, (14..20).collect::<VecDeque<_>>(), "keeps the newest");
    }

    #[test]
    fn stale_model_load_is_ignored() {
        let mut s = Switcher::new(NsLevel::Off);
        let first = s.request(NsLevel::Strong); // starts a load, generation g1
        s.request(NsLevel::Off); // user flips back before it finishes
        assert!(
            !s.accept_loaded(first),
            "an outdated load must not be installed"
        );
        let second = s.request(NsLevel::Strong);
        assert!(s.accept_loaded(second));
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
        start_strong(7, tx);
        let (g, worker) = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("load finishes");
        assert_eq!(g, 7);
        let mut w = worker.expect("model loads");
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
}
