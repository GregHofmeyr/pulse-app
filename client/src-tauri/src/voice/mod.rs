//! Voice: our own capture/playback pipeline over LiveKit (rules: spikes/voice/FINDINGS.md).

pub mod backlog;
pub mod controls;
pub mod denoise;
pub mod devices;
pub mod gate;
pub mod linkstats;
pub mod meter;
pub mod mictest;
pub mod mixer;
pub mod netstats;
pub mod playout;
pub mod processor;
pub mod rejoin;
pub mod resampler;
pub mod rtprio;
pub mod rx;
pub mod speaking;
pub mod stats;
pub mod tokens;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::StreamExt;
use livekit::options::TrackPublishOptions;
use livekit::prelude::*;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use pulse_protocol::ids::ChannelId;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::api::{Api, ApiError};
use controls::Controls;
use devices::{AudioConfig, AudioIo, INTERNAL_RATE, Shared, Sides};
use mixer::{SourceKind, percent_to_gain};
use rx::RxTasks;

/// Max audio buffered per peer before the oldest is dropped (bounds latency).
const PEER_BUFFER_MS: u32 = 200;
/// How long a bug report waits for the voice session before reporting it as busy.
const DIAGNOSTICS_WAIT: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error(transparent)]
    Api(#[from] ApiError),
    #[error("couldn't connect to voice: {0}")]
    Connect(String),
    #[error("audio device problem: {0}")]
    Device(String),
    #[error("cancelled")]
    Cancelled,
    #[error("timed out {0}")]
    Timeout(&'static str),
}

/// How often to check whether the devices should follow the system default (or a returning one).
const FOLLOW_EVERY: Duration = Duration::from_secs(2);
/// Device enumeration can block on a Bluetooth device mid-switch: skip the check rather than wait.
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(10);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a teardown waits for a device reopen in flight: `open_half`'s own 5 s limit plus a close.
const SETTLE_WAIT: Duration = Duration::from_secs(6);

/// Opening devices blocks (cpal + a possibly-flapping Bluetooth headset): never on an async worker.
async fn start_audio(
    cfg: &AudioConfig,
    shared: Arc<Shared>,
    raw_tx: processor::RawTx,
) -> Result<AudioIo, VoiceError> {
    let cfg = cfg.clone();
    tokio::task::spawn_blocking(move || AudioIo::start(&cfg, shared, raw_tx))
        .await
        .map_err(|e| VoiceError::Device(e.to_string()))?
        .map_err(|e| VoiceError::Device(e.to_string()))
}

/// Open the devices while the room connects (neither needs the other): a join costs the slower
/// of the two, not their sum. If one fails, the other is undone: opened devices are dropped
/// (closing them), a connected room is closed (dropping a Room doesn't disconnect it). One
/// failing doesn't abandon the other midway (a half-done `Room::connect`); a cancelled join
/// drops both, so a connected room must be guarded on its own (`OpenRoom`).
async fn side_by_side<D, R, F>(
    devices: impl Future<Output = Result<D, VoiceError>>,
    room: impl Future<Output = Result<R, VoiceError>>,
    undo_room: impl FnOnce(R) -> F,
) -> Result<(D, R), VoiceError>
where
    F: Future<Output = ()>,
{
    match tokio::join!(devices, room) {
        (Ok(d), Ok(r)) => Ok((d, r)),
        (Err(e), Ok(r)) => {
            undo_room(r).await;
            Err(e)
        }
        (_, Err(e)) => Err(e), // opened devices (if any) drop here
    }
}

/// Closing a room (a trait so tests can stand in for a livekit `Room`).
trait CloseRoom: Send + Sync + 'static {
    fn close_room(&self) -> impl Future<Output = ()> + Send;
}

impl CloseRoom for Room {
    async fn close_room(&self) {
        let _ = self.close().await;
    }
}

/// A connected room that no session owns yet. A dropped livekit `Room` stays connected (its task
/// only ends on `close()`): others would see a ghost. So a guard dropped with its room still in
/// it (a cancelled join) closes the room in the background; `disarm` once a session owns it.
struct OpenRoom<R: CloseRoom>(Option<Arc<R>>);

impl<R: CloseRoom> OpenRoom<R> {
    fn new(room: R) -> Self {
        Self(Some(Arc::new(room)))
    }

    fn room(&self) -> &Arc<R> {
        self.0
            .as_ref()
            .expect("an OpenRoom holds its room until closed or disarmed")
    }

    /// Close it now (bounded wait).
    async fn close(mut self) {
        if let Some(room) = self.0.take() {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, room.close_room()).await;
        }
    }

    /// A session owns the room now (and closes it on leave): stop guarding it.
    fn disarm(mut self) {
        self.0.take();
    }
}

impl<R: CloseRoom> Drop for OpenRoom<R> {
    fn drop(&mut self) {
        if let Some(room) = self.0.take() {
            close_in_background(room);
        }
    }
}

/// Close `room` without waiting for it (bounded by `CLOSE_TIMEOUT`).
fn close_in_background<R: CloseRoom>(room: Arc<R>) {
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        rt.spawn(async move {
            if tokio::time::timeout(CLOSE_TIMEOUT, room.close_room())
                .await
                .is_err()
            {
                tracing::warn!("voice: room close timed out; dropping it");
            }
        });
    }
}

/// How many failed publishes in a row a rejoin may hand back to the rejoin loop before the call
/// ends (each hand-back restarts the loop's attempts: unbounded, it could cycle forever).
const PUBLISH_RETRIES: u32 = 2;

/// The call is over for this session, and its pipeline with it: no more mic into the room,
/// devices closed, receivers stopped. `pipeline_alive` cleared: the next join builds a fresh one.
fn stop_audio(
    pipeline_alive: &AtomicBool,
    shared: &Shared,
    io: &Mutex<Option<AudioIo>>,
    rx: &Mutex<RxTasks>,
) {
    stop_audio_keep_devices(pipeline_alive, shared, rx);
    io.lock().unwrap().take();
}

/// `stop_audio`, but the devices stay open in the pipeline: for whoever takes the dead session
/// over (a rejoin's `leave_locked(false)`, a join's `keep_if_usable`) to close with a wait, since
/// it opens the same (Bluetooth) devices next. Nothing is fed or played meanwhile.
fn stop_audio_keep_devices(pipeline_alive: &AtomicBool, shared: &Shared, rx: &Mutex<RxTasks>) {
    pipeline_alive.store(false, Ordering::SeqCst);
    shared.set_mic_sink(None);
    rx.lock().unwrap().clear();
}

/// LiveKit dropped the call (or we gave up on its reconnect): mark the session dead, stop its
/// audio (the next join builds a fresh pipeline), then `hand_over` it to the rejoin loop (true if
/// taken). Handed over, its devices are left for the rejoin to close with a wait; otherwise they
/// close now. A session already ended (left, or switched away from: its pipeline went on to the
/// next room) keeps its hands off the pipeline. Returns whether it was handed over.
fn drop_call(
    alive: &AtomicBool,
    (pipeline_alive, shared): (&AtomicBool, &Shared),
    io: &Mutex<Option<AudioIo>>,
    rx: &Mutex<RxTasks>,
    hand_over: impl FnOnce() -> bool,
) -> bool {
    if !alive.swap(false, Ordering::SeqCst) {
        return false;
    }
    stop_audio_keep_devices(pipeline_alive, shared, rx);
    let handed_over = hand_over();
    if !handed_over {
        io.lock().unwrap().take();
    }
    handed_over
}

/// What the background mic publish needs to report how it went.
struct PublishWatch<R: CloseRoom> {
    alive: Arc<AtomicBool>,
    room: Arc<R>,
    events: EventSink,
    controls: Arc<Mutex<Controls>>,
    channel: ChannelId,
    /// A rejoin's session: (its epoch, the rejoin loop). A failed publish goes back there.
    rejoin: Option<(u64, Option<mpsc::UnboundedSender<u64>>)>,
    /// Failed rejoin publishes handed back in a row (the manager's `publish_retries`).
    retries: Arc<AtomicU32>,
}

/// Tell the UI how the background mic publish went, once its join has shown "connected"
/// (`connected` fires; dropped = the join never got there: say nothing, its room guard closes the
/// room). A session already torn down (`alive` cleared by a leave, a drop, or a rejoin replacing
/// it) says nothing either. A failed publish ends the session: `teardown` stops its audio and the
/// room is closed. A rejoin's goes back to the rejoin loop (still "reconnecting": right after a
/// drop is when a publish most likely times out), at most `PUBLISH_RETRIES` times in a row;
/// otherwise the call ends and the UI is told why.
async fn report_publish<R: CloseRoom>(
    published: Result<(), String>,
    connected: tokio::sync::oneshot::Receiver<()>,
    w: &PublishWatch<R>,
    teardown: impl FnOnce(),
) {
    if connected.await.is_err() {
        return;
    }
    let state = |connection, channel_id| VoiceEvent::State {
        channel_id,
        connection,
        controls: *w.controls.lock().unwrap(),
    };
    let e = match published {
        Ok(()) => {
            w.retries.store(0, Ordering::SeqCst);
            if w.alive.load(Ordering::SeqCst) {
                (w.events)(VoiceEvent::Stage {
                    stage: JoinStage::Connected,
                    attempt: None,
                });
                (w.events)(VoiceEvent::Mic {
                    live: true,
                    error: None,
                });
            }
            return;
        }
        Err(e) => e,
    };
    if !w.alive.swap(false, Ordering::SeqCst) {
        return;
    }
    teardown();
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, w.room.close_room()).await;
    if let Some((epoch, dropped)) = &w.rejoin
        && w.retries.load(Ordering::SeqCst) < PUBLISH_RETRIES
        && dropped.as_ref().is_some_and(|d| d.send(*epoch).is_ok())
    {
        w.retries.fetch_add(1, Ordering::SeqCst);
        tracing::warn!(error = %e, "voice: publishing the mic failed; rejoining again");
        (w.events)(state(Connection::Reconnecting, Some(w.channel)));
        return;
    }
    // a call nobody can hear us in isn't a call: end it, and say why
    tracing::warn!(error = %e, "voice: publishing the mic failed; leaving");
    (w.events)(VoiceEvent::Mic {
        live: false,
        error: Some(format!("Couldn't send your mic: {e}")),
    });
    (w.events)(state(Connection::Disconnected, None));
}

/// Reopens the pipeline's devices (stays in the room). Stall recovery, device following and
/// settings changes can all want one: they take turns, and each reads the newest settings.
struct Reopener {
    cfg: Arc<Mutex<AudioConfig>>,
    io: Arc<Mutex<Option<AudioIo>>>,
    shared: Arc<Shared>,
    /// Into the mic processor; device (re)opens feed it. Dropping the last one ends the processor.
    raw_tx: processor::RawTx,
    alive: Arc<AtomicBool>,
    one_at_a_time: tokio::sync::Mutex<()>,
    /// Devices a switch couldn't open lately: following leaves them alone for a while.
    failed: Mutex<devices::FailedTargets>,
    /// How many reopens so far (diagnostics: following mustn't churn).
    count: std::sync::atomic::AtomicU32,
}

impl Reopener {
    /// Reopen `sides` (stays in the room). A side that fails goes back to the device it had, so
    /// the call keeps working audio; the error is still returned (a device picked in settings that
    /// won't open should say so).
    /// `targets`: the devices each side is meant to end up on, if known (a miss is remembered).
    async fn reopen(&self, sides: Sides, targets: devices::Targets) -> Result<(), VoiceError> {
        let _turn = self.one_at_a_time.lock().await;
        if !self.alive.load(Ordering::SeqCst) {
            return Ok(()); // left (or LiveKit dropped us) while waiting: nothing to open for
        }
        self.count.fetch_add(1, Ordering::Relaxed);
        let cfg = self.cfg.lock().unwrap().clone();
        let current = self.io.lock().unwrap().take();
        let (shared, raw_tx, alive) =
            (self.shared.clone(), self.raw_tx.clone(), self.alive.clone());
        // opening blocks (cpal, a flapping Bluetooth device): never on an async worker
        let reopened = tokio::task::spawn_blocking(move || {
            // nothing open (an earlier open failed outright): open both sides, keeping whichever works
            let (mut io, sides) = match current {
                Some(io) => (io, sides),
                None => (AudioIo::empty(shared, raw_tx), Sides::BOTH),
            };
            let r = io.reopen(&cfg, sides, || alive.load(Ordering::SeqCst));
            (Some(io), r)
        })
        .await;
        let (io, result) = match reopened {
            Ok(r) => r,
            Err(e) => (None, Err(anyhow::anyhow!("audio thread: {e}"))),
        };
        match io {
            Some(io) if self.alive.load(Ordering::SeqCst) => {
                tracing::info!(?sides, "voice: devices reopened");
                self.shared.watchdog.rearm(Instant::now());
                self.failed
                    .lock()
                    .unwrap()
                    .record(sides, &targets, &io.opened(), Instant::now());
                *self.io.lock().unwrap() = Some(io);
            }
            // the pipeline ended meanwhile: really close (a rejoin may open the same device next)
            Some(io) => {
                let _ = tokio::task::spawn_blocking(move || io.close()).await;
            }
            None => {}
        }
        result.map_err(|e| VoiceError::Device(e.to_string()))
    }

    /// Wait (bounded) until no reopen is in flight. Call after clearing `alive`: a reopen that
    /// finishes now closes what it opened instead of keeping it.
    async fn settle(&self) {
        if tokio::time::timeout(SETTLE_WAIT, self.one_at_a_time.lock())
            .await
            .is_err()
        {
            tracing::warn!("voice: a device reopen is still running; going ahead");
        }
    }

    /// Whether the devices should change (and why): the system default moved, or a chosen device
    /// is back. `None` also when the system was too slow to answer (a BT device mid-switch).
    async fn wants_switch(
        &self,
        probe: &mut Option<JoinHandle<DeviceProbe>>,
    ) -> Option<(&'static str, Sides, devices::Targets)> {
        let opened = self
            .io
            .lock()
            .unwrap()
            .as_ref()
            .map(|io| io.opened().clone())?;
        let cfg = self.cfg.lock().unwrap().clone();
        let handle = match probe.take() {
            // still stuck from last time: don't pile up blocked threads
            Some(h) if !h.is_finished() => {
                *probe = Some(h);
                return None;
            }
            // finished late: use it (a couple of seconds old at most), or a slow machine
            // (lots of Sonar endpoints) would never get to act
            Some(h) => h,
            None => {
                let fallback = devices::on_fallback(&cfg, &opened);
                tokio::task::spawn_blocking(move || {
                    (
                        devices::default_device_names(),
                        fallback.then(devices::device_names),
                    )
                })
            }
        };
        let mut handle = handle;
        let (defaults, available) = match tokio::time::timeout(PROBE_TIMEOUT, &mut handle).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => return None,
            Err(_) => {
                tracing::debug!("voice: device check skipped (the system was slow to answer)");
                *probe = Some(handle);
                return None;
            }
        };
        let failed = self.failed.lock().unwrap();
        let now = Instant::now();
        if let Some((ins, outs)) = available {
            let targets = (cfg.input.clone(), cfg.output.clone());
            let back = devices::back_sides(&cfg, &opened, &ins, &outs);
            let back = failed.filter(back, &targets, now);
            if back.any() {
                return Some(("chosen device is back", back, targets));
            }
        }
        let moved = devices::moved_sides(&cfg, &opened, defaults.clone());
        let moved = failed.filter(moved, &defaults, now);
        moved
            .any()
            .then_some(("system default device changed", moved, defaults))
    }
}

/// (system defaults, all device names if needed)
type DeviceProbe = (
    (Option<String>, Option<String>),
    Option<(Vec<String>, Vec<String>)>,
);

/// What a call needs locally, room or no room: the devices, the mic processor (echo canceller,
/// noise model, gate) and the mixer. Kept across a channel switch, so a switch neither reopens
/// the devices nor reloads Strong's model; dropped on Leave (and given up when LiveKit drops us).
struct Pipeline {
    shared: Arc<Shared>,
    cfg: Arc<Mutex<AudioConfig>>,
    io: Arc<Mutex<Option<AudioIo>>>,
    reopener: Arc<Reopener>,
    real: bool,
    /// Cleared when dropped, on Leave, or when the call ended underneath it (devices closed).
    alive: Arc<AtomicBool>,
    /// Device watchdog and following: they outlive any one room.
    tasks: Vec<JoinHandle<()>>,
    /// A device stall was reported and hasn't recovered (the next room shows it again).
    stalled: Arc<AtomicBool>,
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        for t in &self.tasks {
            t.abort();
        }
        self.shared.set_mic_sink(None);
        self.io.lock().unwrap().take(); // closes the devices; the processor ends with its senders
    }
}

impl Pipeline {
    async fn build(
        cfg: &AudioConfig,
        mode: AudioMode,
        controls: Arc<Mutex<Controls>>,
        events: EventSink,
    ) -> Result<Self, VoiceError> {
        let shared = Shared::new(cfg, controls, PEER_BUFFER_MS);
        // devices → processor (denoise, gate) → the room's mic sink, once there is one
        let (raw_tx, _processor) = processor::spawn(shared.clone());
        let io = match mode {
            AudioMode::Real => start_audio(cfg, shared.clone(), raw_tx.clone()).await?,
            AudioMode::Null(rate) => AudioIo::start_null(rate, shared.clone(), raw_tx.clone()),
            AudioMode::SlowNull(rate, open) => {
                tokio::time::sleep(open).await;
                AudioIo::start_null(rate, shared.clone(), raw_tx.clone())
            }
        };
        let real = matches!(mode, AudioMode::Real);
        let alive = Arc::new(AtomicBool::new(true));
        let io = Arc::new(Mutex::new(Some(io)));
        let cfg = Arc::new(Mutex::new(cfg.clone()));
        let reopener = Arc::new(Reopener {
            cfg: cfg.clone(),
            io: io.clone(),
            shared: shared.clone(),
            raw_tx,
            alive: alive.clone(),
            one_at_a_time: tokio::sync::Mutex::new(()),
            failed: Default::default(),
            count: Default::default(),
        });
        let mut tasks = Vec::new();
        let stalled = Arc::new(AtomicBool::new(false));
        if real {
            let watch = (shared.clone(), reopener.clone(), stalled.clone());
            tasks.push(spawn_watchdog(watch, events));
            tasks.push(spawn_follow(reopener.clone()));
        }
        Ok(Self {
            shared,
            cfg,
            io,
            reopener,
            real,
            alive,
            tasks,
            stalled,
        })
    }

    /// New settings (a switch carries the join's settings; or Settings changed): processing
    /// changes live, a changed device reopens only its side.
    async fn adopt(&self, cfg: &AudioConfig) -> Result<(), VoiceError> {
        let changed = Sides::changed(&self.cfg.lock().unwrap(), cfg);
        self.shared.apply_config(cfg);
        *self.cfg.lock().unwrap() = cfg.clone();
        if changed.any() && self.real {
            let targets = (cfg.input.clone(), cfg.output.clone());
            self.reopener.reopen(changed, targets).await?;
        }
        Ok(())
    }

    /// Whether a join in `mode` can carry on with this pipeline (not if its call ended under it).
    fn usable_for(&self, mode: AudioMode) -> bool {
        self.alive.load(Ordering::SeqCst)
            && self.real == matches!(mode, AudioMode::Real)
            && self.io.lock().unwrap().is_some()
    }
}

/// The kept pipeline a join can go on with. A reopen in flight (devices taken out, so not
/// usable mid-way) is waited out first: it takes turns with this check (Settings' `adopt` too).
/// One that can't be used is let go before the join builds another: no reopen of it still
/// running, its devices closed (waited for: the new pipeline opens the same ones).
async fn keep_if_usable(kept: Option<Arc<Pipeline>>, mode: AudioMode) -> Option<Arc<Pipeline>> {
    let p = kept?;
    let turn = tokio::time::timeout(SETTLE_WAIT, p.reopener.one_at_a_time.lock()).await;
    let waited_out = turn.is_ok();
    if turn.is_ok_and(|_turn| p.usable_for(mode)) {
        return Some(p);
    }
    // its watchdog and following stop on `alive` too (Settings may still hold a handle on it)
    p.alive.store(false, Ordering::SeqCst);
    if waited_out {
        // a reopen that started since (the watchdog's, Settings') ends now: it closes what it opened
        p.reopener.settle().await;
    } else {
        // already waited SETTLE_WAIT: it closes what it opens once it's done
        tracing::warn!("voice: a device reopen is still running; not keeping its devices");
    }
    p.shared.set_mic_sink(None);
    let io = p.io.lock().unwrap().take();
    if let Some(io) = io {
        let _ = tokio::task::spawn_blocking(move || io.close()).await;
    }
    None
}

/// Reopen stalled devices (and say so if that doesn't help). Lives as long as the pipeline.
/// `stalled`: set while a reported stall lasts.
fn spawn_watchdog(
    (shared, reopener, stalled): (Arc<Shared>, Arc<Reopener>, Arc<AtomicBool>),
    events: EventSink,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        // after a slow reopen, carry on from now (a burst of catch-up ticks would judge the new
        // streams at once)
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut stall = devices::StallPolicy::default();
        loop {
            tick.tick().await;
            if !reopener.alive.load(Ordering::SeqCst) {
                break;
            }
            let now = Instant::now();
            match stall.check(shared.watchdog.stalled(now), now) {
                None => {}
                Some(devices::StallAction::Report) => {
                    tracing::error!("audio device still stalled after reopening");
                    stalled.store(true, Ordering::SeqCst);
                    events(VoiceEvent::DeviceStalled);
                }
                Some(devices::StallAction::Recovered) => {
                    tracing::info!("voice: audio device recovered");
                    stalled.store(false, Ordering::SeqCst);
                    events(VoiceEvent::DeviceRecovered);
                }
                Some(devices::StallAction::Reopen) => {
                    let mut sides = shared.watchdog.stalled_sides(Instant::now());
                    if !sides.any() {
                        sides = Sides::BOTH; // recovered meanwhile: be thorough anyway
                    }
                    tracing::warn!(?sides, "audio device stalled; reopening");
                    let _ = reopener.reopen(sides, (None, None)).await;
                }
            }
        }
    })
}

/// Follow the system default device / a returning chosen device (own task: enumeration can
/// block, and must not stall the watchdog).
fn spawn_follow(reopener: Arc<Reopener>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(FOLLOW_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let (mut debounce, mut probe) = (devices::Debounce::default(), None);
        loop {
            tick.tick().await;
            if !reopener.alive.load(Ordering::SeqCst) {
                break;
            }
            let switch = reopener.wants_switch(&mut probe).await;
            if debounce.confirm(switch.as_ref().map(|(_, sides, _)| *sides))
                && let Some((reason, sides, targets)) = switch
            {
                tracing::info!(reason, ?sides, "voice: switching devices");
                let _ = reopener.reopen(sides, targets).await;
            }
        }
    })
}

impl Serialize for VoiceError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[derive(Clone, Copy, Debug)]
pub enum AudioMode {
    Real,
    /// No devices (tests / headless): mixer pulled on a timer at this rate.
    Null(u32),
    /// `Null` that takes this long to open (tests: a slow device open during a join).
    SlowNull(u32, Duration),
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
}

/// Where a join is, for the UI's progress copy. `Disconnected` and LiveKit's own reconnecting
/// stay `State` events.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JoinStage {
    RequestingAccess,
    Connecting,
    StartingAudio,
    Connected,
    Reconnecting,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VoiceEvent {
    /// A join stage; `attempt` numbers a rejoin (1-based).
    Stage {
        stage: JoinStage,
        attempt: Option<u32>,
    },
    /// Ping, jitter, loss and connection type, while the status panel is open.
    Link(linkstats::LinkReport),
    State {
        channel_id: Option<ChannelId>,
        connection: Connection,
        controls: Controls,
    },
    Speaking {
        user_ids: Vec<String>,
    },
    Quality {
        user_id: String,
        quality: String,
    },
    Levels {
        mic: f32,
        speaker: f32,
        /// Whether the voice gate is letting your mic through right now.
        gate_open: bool,
    },
    /// Which noise suppression is actually running (Strong may be loading or have fallen back).
    NoiseSuppression {
        active: denoise::NsLevel,
        note: Option<String>,
    },
    DeviceStalled,
    DeviceRecovered,
    /// Whether our mic is in the room. "Connected" comes first; the publish follows ~1 RTT later.
    /// A failed publish ends the call (`error` says why).
    Mic {
        live: bool,
        error: Option<String>,
    },
}

pub type EventSink = Arc<dyn Fn(VoiceEvent) + Send + Sync>;

/// This tick's meter update, if a meter is on screen (each one is a JSON eval in the webview: 10 a
/// second for a whole call otherwise). The levels are taken either way, so a meter that opens
/// mid-call starts from the last 100 ms, not an average over the call so far.
fn meter_update(shared: &Shared) -> Option<VoiceEvent> {
    let (mic, speaker) = shared.take_levels();
    let shown = shared.controls.lock().unwrap().meters_shown();
    shown.then(|| VoiceEvent::Levels {
        mic,
        speaker,
        gate_open: shared.gate_open.load(Ordering::Relaxed),
    })
}

/// `R`: the room (a stand-in in tests).
struct Session<R: CloseRoom = Room> {
    channel: ChannelId,
    room: Arc<R>,
    /// Devices, processor and mixer: handed to the next room on a switch.
    pipeline: Arc<Pipeline>,
    rx: Arc<Mutex<RxTasks>>,
    /// Cleared when LiveKit disconnects us for good: the session is then torn down.
    alive: Arc<AtomicBool>,
    tasks: Vec<JoinHandle<()>>,
}

impl<R: CloseRoom> Drop for Session<R> {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

impl<R: CloseRoom> Session<R> {
    /// Leave this room for another one: stop feeding it and playing its people, close it in the
    /// background (others may see us in both rooms for about a round trip) and report nothing
    /// (the switch goes straight to connecting the next room). The pipeline is handed back for
    /// the next room: its devices stay open.
    async fn detach(self) -> Arc<Pipeline> {
        tracing::info!(channel = %self.channel, "voice: leaving for another channel");
        self.alive.store(false, Ordering::SeqCst);
        let pipeline = self.pipeline.clone();
        pipeline.shared.set_mic_sink(None);
        self.rx.lock().unwrap().clear();
        pipeline.shared.mix_in.clear();
        if !pipeline.alive.load(Ordering::SeqCst) {
            // the call already ended under it (it won't be reused): a reopen in flight must
            // finish closing before the next join opens the same (Bluetooth) device
            pipeline.reopener.settle().await;
        }
        let room = self.room.clone();
        // its tasks stop here: our own close must not look like LiveKit dropping us (a rejoin)
        drop(self);
        close_in_background(room);
        pipeline
    }

    /// Tear this session down for good: out of its room, devices closed, the pipeline with it.
    /// `announce` (the user's Leave): out of the room first, then wait out a reopen in flight
    /// (up to `SETTLE_WAIT`), and don't wait on the devices. Otherwise (a rejoin, which opens the
    /// same devices next) wait out the reopen first and for the devices to really close.
    async fn end(self, announce: bool) {
        // first: a reopen in flight must not open devices for a session that's gone
        self.alive.store(false, Ordering::SeqCst);
        let p = self.pipeline.clone();
        p.alive.store(false, Ordering::SeqCst);
        p.shared.set_mic_sink(None);
        if !announce {
            p.reopener.settle().await;
        }
        if tokio::time::timeout(CLOSE_TIMEOUT, self.room.close_room())
            .await
            .is_err()
        {
            tracing::warn!("voice: room close timed out; dropping it");
        }
        if announce {
            p.reopener.settle().await;
        }
        self.rx.lock().unwrap().clear();
        let io = p.io.lock().unwrap().take();
        if let Some(io) = io {
            if announce {
                drop(io); // the user's Leave: don't wait on the devices
            } else {
                // a rejoin opens the same devices next: wait until these are really closed
                let _ = tokio::task::spawn_blocking(move || io.close()).await;
            }
        }
        drop((self, p)); // the pipeline goes with the session: the processor ends
    }
}

/// A joined session, the guard on its room (until the session is stored), and the signal that
/// "connected" has been shown (the background mic publish reports only after it).
type Connected = (Session, OpenRoom<Room>, tokio::sync::oneshot::Sender<()>);

/// The user's last join, kept so a dropped call can be rejoined with the same settings.
#[derive(Clone)]
struct JoinArgs {
    api: Api,
    token: String,
    channel: ChannelId,
    cfg: AudioConfig,
    mode: AudioMode,
}

/// How a rejoin ended.
enum Rejoined {
    Yes,
    /// The user left or joined something else meanwhile: their action reported its own state.
    Superseded,
    GaveUp,
}

pub struct VoiceManager {
    /// Serialises join/leave: two fast clicks can't leave a ghost connection behind.
    op: tokio::sync::Mutex<()>,
    /// Cancels an in-flight join (Leave must never queue behind a stuck connect).
    /// (which join, its cancel). Its lock also covers bumping `epoch`, so a rejoin's epoch check
    /// and its taking this slot are one step a user's join/leave can't land in between.
    pending_join: Mutex<Option<(u64, tokio::sync::oneshot::Sender<()>)>>,
    /// Numbers joins, so a join only ever clears its own `pending_join`.
    join_seq: std::sync::atomic::AtomicU64,
    session: tokio::sync::Mutex<Option<Session>>,
    /// One source of truth, shared with the audio callbacks. Survives across sessions (stay muted
    /// when switching channels, like Discord).
    controls: Arc<Mutex<Controls>>,
    /// Per-user volume (0..=200 %), keyed by user id.
    volumes: Arc<Mutex<HashMap<String, u16>>>,
    events: EventSink,
    /// The current session's `alive` (cleared on leave or when LiveKit drops us): the truth for
    /// "in voice", readable without the session lock.
    live: Mutex<Option<Arc<AtomicBool>>>,
    /// The join token fetched while hovering a voice channel.
    tokens: tokens::TokenCache,
    /// WebRTC's runtime (peer-connection factory and its threads), kept for the app's life:
    /// LiveKit only holds it weakly, so it was rebuilt on every join after a leave.
    lk: Arc<std::sync::OnceLock<Arc<livekit::rtc_engine::lk_runtime::LkRuntime>>>,
    /// Bumped by every user join/leave: a rejoin for an older one stands down.
    epoch: std::sync::atomic::AtomicU64,
    /// What the user last joined (the rejoin target); cleared on Leave.
    last_join: Mutex<Option<JoinArgs>>,
    /// Calls LiveKit dropped for good → `run_rejoins` (None until that loop runs: no rejoins).
    dropped_tx: Mutex<Option<mpsc::UnboundedSender<u64>>>,
    /// The channel being rejoined right now (the UI shows "reconnecting" for it).
    rejoining: Mutex<Option<ChannelId>>,
    rejoin_count: std::sync::atomic::AtomicU32,
    /// Failed mic publishes a rejoin handed back to the rejoin loop in a row (capped by
    /// `PUBLISH_RETRIES`); reset by a published mic and by the user's own join.
    publish_retries: Arc<AtomicU32>,
    /// Scales the rejoin backoff (1 s in production; small in tests).
    rejoin_unit: Duration,
    /// How long LiveKit may stay "reconnecting" before we rejoin ourselves (a field so tests can
    /// shorten it).
    reconnect_give_up: Duration,
    /// Audio pipelines built so far (a switch reuses the one it has).
    built: AtomicU64,
    /// Link figures for the status panel (reported only while it is open).
    link: Arc<linkstats::LinkState>,
}

impl VoiceManager {
    pub fn new(events: EventSink) -> Self {
        Self {
            op: tokio::sync::Mutex::new(()),
            pending_join: Mutex::new(None),
            join_seq: Default::default(),
            session: tokio::sync::Mutex::new(None),
            controls: Arc::new(Mutex::new(Controls::default())),
            volumes: Arc::new(Mutex::new(HashMap::new())),
            events,
            live: Mutex::new(None),
            tokens: Default::default(),
            lk: Default::default(),
            epoch: Default::default(),
            last_join: Mutex::new(None),
            dropped_tx: Mutex::new(None),
            rejoining: Mutex::new(None),
            rejoin_count: Default::default(),
            publish_retries: Default::default(),
            rejoin_unit: Duration::from_secs(1),
            reconnect_give_up: rejoin::RECONNECT_GIVE_UP,
            built: Default::default(),
            link: Default::default(),
        }
    }

    /// Pipelines built by this manager (tests: a switch must reuse the one it has).
    pub fn pipelines_built(&self) -> u64 {
        self.built.load(Ordering::Relaxed)
    }

    /// Hovering a voice channel: fetch its join token now, so a click right after doesn't wait
    /// a round trip for it.
    pub fn prefetch(&self, api: &Api, token: &str, channel: ChannelId) {
        self.tokens.prefetch(api, token, channel, Instant::now());
        self.warm_runtime();
    }

    /// Build WebRTC's runtime if needed, and keep it.
    pub fn keep_runtime(&self) {
        self.lk
            .get_or_init(livekit::rtc_engine::lk_runtime::LkRuntime::instance);
    }

    /// Same, on the blocking pool (it takes tens of ms): for a hover, ahead of the click.
    pub fn warm_runtime(&self) {
        let lk = self.lk.clone();
        tokio::task::spawn_blocking(move || {
            lk.get_or_init(livekit::rtc_engine::lk_runtime::LkRuntime::instance);
        });
    }

    /// Signed out: no token of this session may be used by the next.
    pub fn forget_tokens(&self) {
        self.tokens.clear();
    }

    fn emit_stage(&self, stage: JoinStage, attempt: Option<u32>) {
        (self.events)(VoiceEvent::Stage { stage, attempt });
    }

    fn emit_state(&self, channel: Option<ChannelId>, connection: Connection) {
        (self.events)(VoiceEvent::State {
            channel_id: channel,
            connection,
            controls: *self.controls.lock().unwrap(),
        });
    }

    /// Join `channel`, leaving any current room first (one voice connection per client).
    pub async fn join(
        &self,
        api: &Api,
        token: &str,
        channel: ChannelId,
        cfg: AudioConfig,
        mode: AudioMode,
    ) -> Result<(), VoiceError> {
        {
            // the user's choice: supersedes any rejoin in progress
            let _pending = self.pending_join.lock().unwrap();
            self.epoch.fetch_add(1, Ordering::SeqCst);
            self.publish_retries.store(0, Ordering::SeqCst);
            *self.last_join.lock().unwrap() = Some(JoinArgs {
                api: api.clone(),
                token: token.to_string(),
                channel,
                cfg: cfg.clone(),
                mode,
            });
        }
        self.join_inner(api, token, channel, cfg, mode, None).await
    }

    /// `rejoin_of`: Some(epoch) for an automatic rejoin. It stands down if the user acted since,
    /// and it reports "reconnecting" instead of "connecting"/"disconnected".
    async fn join_inner(
        &self,
        api: &Api,
        token: &str,
        channel: ChannelId,
        cfg: AudioConfig,
        mode: AudioMode,
        rejoin_of: Option<u64>,
    ) -> Result<(), VoiceError> {
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let id = self.join_seq.fetch_add(1, Ordering::Relaxed) + 1;
        {
            let mut pending = self.pending_join.lock().unwrap();
            if let Some(e) = rejoin_of {
                // stale (the user acted since), or a join of the user's is still going: stand
                // down, never cancel it
                if e != self.epoch.load(Ordering::SeqCst) || pending.is_some() {
                    return Err(VoiceError::Cancelled);
                }
            }
            // a newer join supersedes an older pending one
            if let Some((_, old)) = pending.replace((id, cancel_tx)) {
                let _ = old.send(());
            }
        }
        let _op = self.op.lock().await;
        if rejoin_of.is_some_and(|e| e != self.epoch.load(Ordering::SeqCst)) {
            self.clear_pending_join(id);
            return Err(VoiceError::Cancelled); // the user left or switched while we waited
        }
        // a switch carries the devices, processor and noise model over to the next room
        let kept = if rejoin_of.is_some() {
            // the dead session's devices are opened again next: wait until they're closed
            self.leave_locked(false).await;
            None
        } else {
            self.detach_locked().await
        };
        tracing::info!(%channel, rejoin = rejoin_of.is_some(), "voice: joining");
        let trying = if rejoin_of.is_some() {
            Connection::Reconnecting
        } else {
            Connection::Connecting
        };
        self.emit_state(Some(channel), trying);
        if rejoin_of.is_none() {
            self.emit_stage(JoinStage::RequestingAccess, None);
        }
        let started = Instant::now();
        // the room stays guarded until the session holding it is stored
        let outcome = tokio::select! {
            r = self.connect(api, token, channel, cfg, mode, rejoin_of.is_some(), kept) => r,
            _ = cancel_rx => Err(VoiceError::Cancelled),
        };
        self.clear_pending_join(id);
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match &outcome {
            Ok(_) => tracing::info!(elapsed_ms, "voice: joined"),
            Err(e) => tracing::warn!(error = %e, elapsed_ms, "voice: join failed"),
        }
        match outcome {
            Ok((session, open_room, connected)) => {
                // a stall that outlived the last room: the UI cleared its banner on joining
                let stalled = session.pipeline.stalled.load(Ordering::SeqCst);
                *self.live.lock().unwrap() = Some(session.alive.clone());
                *self.session.lock().await = Some(session);
                open_room.disarm();
                self.emit_stage(JoinStage::StartingAudio, None);
                self.emit_state(Some(channel), Connection::Connected);
                if stalled {
                    (self.events)(VoiceEvent::DeviceStalled);
                }
                // the mic publish reports from here on, so never before "connected"
                let _ = connected.send(());
                Ok(())
            }
            Err(e) => {
                if rejoin_of.is_none() {
                    self.emit_state(None, Connection::Disconnected);
                }
                Err(e)
            }
        }
    }

    /// Done with join `id`: drop its cancel handle (if a newer join hasn't replaced it).
    fn clear_pending_join(&self, id: u64) {
        let mut pending = self.pending_join.lock().unwrap();
        if pending.as_ref().is_some_and(|(p, _)| *p == id) {
            pending.take();
        }
    }

    /// Brings voice back after LiveKit gives up: rejoins the same channel with backoff until it
    /// works, the user leaves or switches, or `rejoin::MAX_ATTEMPTS` fail. Runs for the app's life.
    pub async fn run_rejoins(&self) {
        let (tx, mut rx) = mpsc::unbounded_channel();
        *self.dropped_tx.lock().unwrap() = Some(tx);
        while let Some(epoch) = rx.recv().await {
            self.rejoin(epoch).await;
        }
    }

    async fn rejoin(&self, epoch: u64) {
        let Some(channel) = self
            .last_join
            .lock()
            .unwrap()
            .as_ref()
            .filter(|_| self.epoch.load(Ordering::SeqCst) == epoch)
            .map(|a| a.channel)
        else {
            return;
        };
        *self.rejoining.lock().unwrap() = Some(channel);
        let outcome = self.rejoin_attempts(epoch).await;
        *self.rejoining.lock().unwrap() = None;
        if let Rejoined::GaveUp = outcome {
            tracing::warn!(%channel, "voice: couldn't rejoin; giving up");
            self.emit_state(None, Connection::Disconnected);
        }
    }

    async fn rejoin_attempts(&self, epoch: u64) -> Rejoined {
        for attempt in 0..rejoin::MAX_ATTEMPTS {
            let wait = rejoin::delay(attempt, rand::random::<f64>())
                .mul_f64(self.rejoin_unit.as_secs_f64());
            tokio::time::sleep(wait).await;
            let args = self.last_join.lock().unwrap().clone();
            let Some(a) = args.filter(|_| self.epoch.load(Ordering::SeqCst) == epoch) else {
                return Rejoined::Superseded;
            };
            self.rejoin_count.fetch_add(1, Ordering::Relaxed);
            self.emit_stage(JoinStage::Reconnecting, Some(attempt + 1));
            tracing::info!(attempt, channel = %a.channel, "voice: rejoining (connection lost)");
            match self
                .join_inner(
                    &a.api,
                    &a.token,
                    a.channel,
                    a.cfg.clone(),
                    a.mode,
                    Some(epoch),
                )
                .await
            {
                Ok(()) => return Rejoined::Yes,
                Err(VoiceError::Cancelled) => return Rejoined::Superseded,
                Err(VoiceError::Api(ApiError::Unauthorized)) => return Rejoined::GaveUp,
                Err(e) => tracing::warn!(attempt, error = %e, "voice: rejoin failed"),
            }
        }
        Rejoined::GaveUp
    }

    /// Rejoin attempts so far (diagnostics).
    pub fn rejoins(&self) -> u32 {
        self.rejoin_count.load(Ordering::Relaxed)
    }

    /// The channel being rejoined right now, if any.
    pub fn rejoining(&self) -> Option<ChannelId> {
        *self.rejoining.lock().unwrap()
    }

    /// Tests: act as if LiveKit just gave up on a call in `channel` the user had joined.
    #[cfg(test)]
    async fn simulate_drop(&self, api: &Api, token: &str, channel: ChannelId) {
        let epoch = {
            let _pending = self.pending_join.lock().unwrap();
            self.epoch.fetch_add(1, Ordering::SeqCst) + 1
        };
        *self.last_join.lock().unwrap() = Some(JoinArgs {
            api: api.clone(),
            token: token.into(),
            channel,
            cfg: AudioConfig::default(),
            mode: AudioMode::Null(48_000),
        });
        let tx = loop {
            if let Some(tx) = self.dropped_tx.lock().unwrap().clone() {
                break tx;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        };
        self.emit_state(Some(channel), Connection::Reconnecting);
        tx.send(epoch).unwrap();
    }

    #[allow(clippy::too_many_arguments)] // a join's inputs, each used once
    async fn connect(
        &self,
        api: &Api,
        token: &str,
        channel: ChannelId,
        cfg: AudioConfig,
        mode: AudioMode,
        rejoin: bool,
        kept: Option<Arc<Pipeline>>,
    ) -> Result<Connected, VoiceError> {
        // join phases, for finding where a slow join's time goes (bug reports carry the log)
        let started = Instant::now();
        let ms = |t: Instant| t.elapsed().as_millis() as u64;
        let prefetched = self.tokens.take(token, channel, Instant::now()).await;
        let token_prefetched = prefetched.is_some();
        let vt = match prefetched {
            Some(vt) => vt,
            None => api.voice_token(token, channel).await?,
        };
        let token_ms = ms(started);
        if !rejoin {
            self.emit_stage(JoinStage::Connecting, None);
        }

        self.keep_runtime();
        let (controls, events) = (self.controls.clone(), self.events.clone());
        // each phase logs its own time; they overlap, so `joined elapsed_ms` is the end-to-end one
        let pipeline = async {
            let t = Instant::now();
            // a switch keeps the last room's pipeline (devices, processor, noise model); one
            // whose call ended under it is let go and a fresh one built
            let p = match keep_if_usable(kept, mode).await {
                Some(p) => {
                    if let Err(e) = p.adopt(&cfg).await {
                        tracing::warn!(error = %e, "voice: the join's devices didn't open; kept the current ones");
                    }
                    (p, true)
                }
                None => {
                    self.built.fetch_add(1, Ordering::Relaxed);
                    let p = Pipeline::build(&cfg, mode, controls, events).await?;
                    (Arc::new(p), false)
                }
            };
            Ok::<_, VoiceError>((p, ms(t)))
        };
        let room = async {
            let t = Instant::now();
            let (room, events) = tokio::time::timeout(
                CONNECT_TIMEOUT,
                Room::connect(&vt.url, &vt.token, RoomOptions::default()),
            )
            .await
            .map_err(|_| VoiceError::Timeout("connecting to voice"))?
            .map_err(|e| VoiceError::Connect(e.to_string()))?;
            // guarded from here: a cancelled join closes it
            Ok::<_, VoiceError>((OpenRoom::new(room), events, ms(t)))
        };
        let (
            ((pipeline, pipeline_reused), devices_ms),
            (open_room, mut room_events, room_connect_ms),
        ) = side_by_side(pipeline, room, |(room, _, _)| room.close()).await?;
        // the panel's "voice server": the host only
        self.link.clear();
        *self.link.server.lock().unwrap() = reqwest::Url::parse(&vt.url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned));
        let room = open_room.room().clone();
        let shared = pipeline.shared.clone();
        shared.mix_in.clear(); // the last room's people
        // this room's receive tasks push under it: one of the last room's still mid-push is dropped
        let mix_gen = shared.mix_in.generation();
        tracing::info!(
            token_ms,
            token_prefetched,
            devices_ms,
            room_connect_ms,
            pipeline_reused,
            "voice: room connected"
        );
        // Fixed 48 kHz towards LiveKit, whatever the device does.
        // queue 0 = WebRTC's fast path: each 10 ms frame goes straight out. A queue there drains on
        // the system clock and is never trimmed, so any backlog in it (a burst, a mic clock running
        // fast) stays as delay; our own queue (the processor's, by age) is bounded and measured instead.
        let source = NativeAudioSource::new(AudioSourceOptions::default(), INTERNAL_RATE, 1, 0);
        let track =
            LocalAudioTrack::create_audio_track("mic", RtcAudioSource::Native(source.clone()));
        // from now on the mic processor hands each 10 ms frame straight to WebRTC
        shared.set_mic_sink(Some(Box::new(processor::LiveKitSink::new(source.clone()))));

        let mut tasks = Vec::new();
        let rx = Arc::new(Mutex::new(RxTasks::default()));
        let (io, pipeline_alive) = (pipeline.io.clone(), pipeline.alive.clone());
        let alive = Arc::new(AtomicBool::new(true));
        // fires once the join has shown "connected" (dropped if it never does)
        let (connected_tx, connected_rx) = tokio::sync::oneshot::channel();
        // a drop LiveKit gives up on is handed to the rejoin loop (if it runs) under this join's epoch
        let (epoch, dropped) = (
            self.epoch.load(Ordering::SeqCst),
            self.dropped_tx.lock().unwrap().clone(),
        );
        // publish in the background: "connected" doesn't wait the round trip for it. The task is
        // the session's (aborted with it); the room is closed by its guard or the session's leave.
        {
            let watch = PublishWatch {
                alive: alive.clone(),
                room: room.clone(),
                events: self.events.clone(),
                controls: self.controls.clone(),
                channel,
                rejoin: rejoin.then(|| (epoch, dropped.clone())),
                retries: self.publish_retries.clone(),
            };
            let (io, rx, shared, pipeline_alive) = (
                io.clone(),
                rx.clone(),
                shared.clone(),
                pipeline_alive.clone(),
            );
            tasks.push(tokio::spawn(async move {
                let t = Instant::now();
                let published = tokio::time::timeout(
                    PUBLISH_TIMEOUT,
                    watch
                        .room
                        .local_participant()
                        .publish_track(LocalTrack::Audio(track), mic_publish_options()),
                )
                .await;
                let published = match published {
                    Ok(Ok(_)) => {
                        let (publish_ms, since_join_ms) = (ms(t), ms(started));
                        tracing::info!(publish_ms, since_join_ms, "voice: mic published");
                        Ok(())
                    }
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err("timed out".to_string()),
                };
                report_publish(published, connected_rx, &watch, || {
                    stop_audio(&pipeline_alive, &shared, &io, &rx)
                })
                .await;
            }));
        }
        // what WebRTC measures about the path, for telling network lag from local lag
        {
            let room = room.clone();
            let shared = shared.clone();
            let (events, link) = (self.events.clone(), self.link.clone());
            tasks.push(tokio::spawn(async move {
                // per-task counters, shared into each tick's future (never held across an await)
                let state = Arc::new(Mutex::new((
                    netstats::Totals::default(),
                    None::<String>,
                    linkstats::LossWindow::default(),
                )));
                linkstats::run_ticks(link.clone(), |tick| {
                    let (room, shared, events, link, state) = (
                        room.clone(),
                        shared.clone(),
                        events.clone(),
                        link.clone(),
                        state.clone(),
                    );
                    async move {
                        let s = match room.get_stats().await {
                            Ok(s) => s,
                            Err(e) => {
                                tracing::debug!(error = %e, "voice: link stats unavailable");
                                return;
                            }
                        };
                        let now = tokio::time::Instant::now();
                        let (totals, last_codec, loss) = &mut *state.lock().unwrap();
                        let sample = linkstats::sample(&s.publisher_stats, loss, now);
                        if let Some(rtt) = sample.rtt_ms {
                            link.history.lock().unwrap().push(now, rtt);
                        }
                        if tick.report {
                            events(VoiceEvent::Link(link.report(sample, now)));
                        }
                        if !tick.log {
                            return;
                        }
                        // which codec our mic is sent with (stereo? FEC/DTX?), once per change
                        if let Some(c) = netstats::send_codec(&s.publisher_stats)
                            && last_codec.as_ref() != Some(&c)
                        {
                            tracing::info!(codec = %c, "voice: send codec");
                            *last_codec = Some(c);
                        }
                        let (line, t) =
                            netstats::summarize(&s.publisher_stats, &s.subscriber_stats, totals);
                        *totals = t;
                        // ours: worst send backlog this window, and the pipeline's counters
                        let send_ms = shared.mic_queue.take_worst();
                        let underrun = shared.mix_in.take_underrun_samples();
                        let adjust = shared.mix_in.take_adjusted_samples();
                        let mix_busy = shared.take_mix_busy();
                        let ours = shared.stats.take(underrun * 1000 / INTERNAL_RATE as u64);
                        tracing::info!(%line, send_ms, adjust, mix_busy, %ours, "voice: path");
                    }
                })
                .await;
            }));
        }

        // room events: subscriptions (FINDINGS rule 1), speaking, quality, connection state
        {
            let (rx, mix_in, volumes, events) = (
                rx.clone(),
                shared.mix_in.clone(),
                self.volumes.clone(),
                self.events.clone(),
            );
            let (controls, io, alive) = (self.controls.clone(), io.clone(), alive.clone());
            let (shared, pipeline_alive) = (shared.clone(), pipeline_alive.clone());
            let mut give_up = rejoin::GiveUpTimer::new(self.reconnect_give_up);
            tasks.push(tokio::spawn(async move {
                let state = |connection, channel_id| VoiceEvent::State {
                    channel_id,
                    connection,
                    controls: *controls.lock().unwrap(),
                };
                // The call is gone: handed to the rejoin loop if `rejoin` (and that loop runs).
                let lost = |rejoin: bool| {
                    let handed_over =
                        drop_call(&alive, (&pipeline_alive, &shared), &io, &rx, || {
                            rejoin && dropped.as_ref().is_some_and(|d| d.send(epoch).is_ok())
                        });
                    if handed_over {
                        events(state(Connection::Reconnecting, Some(channel)));
                    } else {
                        events(state(Connection::Disconnected, None));
                    }
                };
                loop {
                    let ev = tokio::select! {
                        ev = room_events.recv() => match ev {
                            Some(ev) => ev,
                            None => break,
                        },
                        // a server that doesn't answer at all can keep LiveKit's own reconnect
                        // going for minutes: cut it short and rejoin
                        _ = give_up.fired() => {
                            tracing::warn!(
                                after_s = give_up.after().as_secs(),
                                "voice: LiveKit still reconnecting; giving up on it"
                            );
                            lost(true);
                            break;
                        }
                    };
                    match ev {
                        RoomEvent::TrackSubscribed {
                            track: RemoteTrack::Audio(t),
                            publication,
                            participant,
                        } => {
                            let owner = participant.identity().to_string();
                            let source = publication.sid().to_string();
                            let kind = source_kind(publication.source());
                            if kind == SourceKind::Voice {
                                let pct =
                                    volumes.lock().unwrap().get(&owner).copied().unwrap_or(100);
                                mix_in.set_gain(&owner, kind, percent_to_gain(pct));
                            }
                            // a re-subscribe under the same SID: stop the old task before the
                            // new ring exists, or its last frame lands in it
                            rx.lock().unwrap().drop_track(&source);
                            // before the task starts: a push for an unregistered source is dropped
                            mix_in.add_source(mix_gen, &source, &owner, kind);
                            let (m, src, peer) = (mix_in.clone(), source.clone(), owner.clone());
                            let subscribed = Instant::now();
                            tracing::info!(
                                peer = %owner,
                                track = %source,
                                ?kind,
                                since_join_ms = started.elapsed().as_millis() as u64,
                                "voice: subscribed"
                            );
                            let task = tokio::spawn(async move {
                                let mut stream =
                                    NativeAudioStream::new(t.rtc_track(), INTERNAL_RATE as i32, 1);
                                let mut heard = false;
                                while let Some(f) = stream.next().await {
                                    // WebRTC hands over (silent) frames before any packet arrives:
                                    // the first one with sound is when they're really audible
                                    if !heard && f.data.iter().any(|s| *s != 0) {
                                        heard = true;
                                        tracing::info!(
                                            %peer,
                                            track = %src,
                                            ?kind,
                                            since_subscribed_ms =
                                                subscribed.elapsed().as_millis() as u64,
                                            "voice: first sound"
                                        );
                                    }
                                    m.push_in(mix_gen, &src, &f.data);
                                }
                            });
                            rx.lock().unwrap().replace(source, owner, task);
                        }
                        RoomEvent::TrackUnsubscribed { publication, .. } => {
                            let source = publication.sid().to_string();
                            rx.lock().unwrap().drop_track(&source);
                            mix_in.remove_source(mix_gen, &source);
                        }
                        RoomEvent::ParticipantDisconnected(p) => {
                            let owner = p.identity().to_string();
                            rx.lock().unwrap().drop_owner(&owner);
                            mix_in.remove_owner(mix_gen, &owner);
                        }
                        RoomEvent::ConnectionQualityChanged {
                            quality,
                            participant,
                        } => {
                            events(VoiceEvent::Quality {
                                user_id: participant.identity().to_string(),
                                quality: format!("{quality:?}").to_lowercase(),
                            });
                        }
                        RoomEvent::Reconnecting => {
                            tracing::warn!("voice: reconnecting");
                            give_up.arm();
                            events(state(Connection::Reconnecting, Some(channel)))
                        }
                        RoomEvent::Reconnected => {
                            tracing::info!("voice: reconnected");
                            give_up.disarm();
                            events(state(Connection::Connected, Some(channel)))
                        }
                        RoomEvent::Disconnected { reason } => {
                            tracing::warn!(?reason, "voice disconnected by LiveKit");
                            lost(rejoin::worth_rejoining(reason));
                            break;
                        }
                        _ => {}
                    }
                }
            }));
        }

        // levels for the UI meter + speaking ring (FINDINGS rule 2); the device watchdog is the
        // pipeline's
        let me_id = room.local_participant().identity().to_string();
        {
            let (shared, events, alive) = (shared.clone(), self.events.clone(), alive.clone());
            tasks.push(tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_millis(100));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut speaking = speaking::SpeakingTracker::default();
                loop {
                    tick.tick().await;
                    if !alive.load(Ordering::SeqCst) {
                        break;
                    }
                    let queued = shared.mix_in.max_buffered();
                    shared
                        .stats
                        .sample_recv_queue((queued * 1000 / INTERNAL_RATE as usize) as u32);
                    if let Some(levels) = meter_update(&shared) {
                        events(levels);
                    }
                    if let Some(st) = shared.take_ns_status_change() {
                        events(VoiceEvent::NoiseSuppression {
                            active: st.active,
                            note: st.note,
                        });
                    }
                    // Ring = actually audible: received levels per peer + what we really send.
                    let mut levels = shared.mix_in.take_peer_levels();
                    levels.push((me_id.clone(), shared.take_sent_level()));
                    if let Some(user_ids) = speaking.update(&levels, Instant::now()) {
                        events(VoiceEvent::Speaking { user_ids });
                    }
                }
            }));
        }

        let session = Session {
            channel,
            room,
            pipeline,
            rx,
            alive,
            tasks,
        };
        Ok((session, open_room, connected_tx))
    }

    pub async fn leave(&self) {
        let was_joined = {
            let mut pending = self.pending_join.lock().unwrap();
            // the user's choice: a rejoin in progress stands down
            self.epoch.fetch_add(1, Ordering::SeqCst);
            // Cancel a join that's still connecting first, so we never queue behind it.
            if let Some((_, cancel)) = pending.take() {
                tracing::info!("voice: leave cancels an in-flight join");
                let _ = cancel.send(());
            }
            self.last_join.lock().unwrap().take().is_some()
        };
        let _op = self.op.lock().await;
        if !self.leave_locked(true).await && was_joined {
            // nothing to tear down (a rejoin was between attempts): still say we're out
            self.emit_state(None, Connection::Disconnected);
        }
    }

    /// Leave the current room (if any) for another one: closed in the background, no
    /// "disconnected" (a plain Leave and a rejoin wait for the close: `leave_locked`). Returns
    /// its pipeline for the next room.
    async fn detach_locked(&self) -> Option<Arc<Pipeline>> {
        self.link.clear();
        let s = self.session.lock().await.take()?;
        Some(s.detach().await)
    }

    /// Tear down the session, if any (true if there was one). `announce`: tell the UI
    /// "disconnected" (a rejoin replaces the dead session without saying so).
    async fn leave_locked(&self, announce: bool) -> bool {
        // also after a join that never got a session
        self.link.clear();
        let Some(s) = self.session.lock().await.take() else {
            return false;
        };
        tracing::info!("voice: leaving");
        s.end(announce).await;
        if announce {
            self.emit_state(None, Connection::Disconnected);
        }
        true
    }

    /// Whether we're in a voice channel right now (no lock waits: safe from any thread).
    pub fn in_voice(&self) -> bool {
        self.live
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|a| a.load(Ordering::SeqCst))
    }

    /// For bug reports: where voice is and which devices it was told to use.
    pub async fn diagnostics(&self) -> serde_json::Value {
        // A device reopen can hang holding the session lock — exactly when someone reports a voice
        // problem. Don't let the report wait on it.
        let Ok(s) = tokio::time::timeout(DIAGNOSTICS_WAIT, self.session.lock()).await else {
            return serde_json::json!({ "state": "busy", "controls": self.controls() });
        };
        let (channel, cfg) = match s.as_ref() {
            Some(s) if s.alive.load(Ordering::SeqCst) => (
                Some(s.channel.to_string()),
                Some(s.pipeline.cfg.lock().unwrap().clone()),
            ),
            _ => (None, None),
        };
        serde_json::json!({
            "channel": channel,
            "controls": self.controls(),
            "input": cfg.as_ref().and_then(|c| c.input.clone()).unwrap_or_else(|| "default".into()),
            "output": cfg.as_ref().and_then(|c| c.output.clone()).unwrap_or_else(|| "default".into()),
        })
    }

    /// The live session's channel (a session LiveKit already dropped doesn't count).
    pub async fn current_channel(&self) -> Option<ChannelId> {
        self.session
            .lock()
            .await
            .as_ref()
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .map(|s| s.channel)
    }

    async fn toggled(&self, f: impl FnOnce(&mut Controls)) -> Controls {
        let c = {
            let mut c = self.controls.lock().unwrap();
            f(&mut c);
            *c
        };
        // The audio callbacks share `controls`, so this already took effect; just tell the UI.
        let (channel_id, connection) = match (self.current_channel().await, self.rejoining()) {
            (Some(c), _) => (Some(c), Connection::Connected),
            (None, Some(c)) => (Some(c), Connection::Reconnecting),
            (None, None) => (None, Connection::Disconnected),
        };
        (self.events)(VoiceEvent::State {
            channel_id,
            connection,
            controls: c,
        });
        c
    }

    pub async fn toggle_mute(&self) -> Controls {
        self.toggled(Controls::toggle_mute).await
    }

    pub async fn toggle_deafen(&self) -> Controls {
        self.toggled(Controls::toggle_deafen).await
    }

    pub fn controls(&self) -> Controls {
        *self.controls.lock().unwrap()
    }

    /// A voice meter appeared in (`true`) or left (`false`) the UI. Survives sessions, like mute.
    pub fn watch_meters(&self, on: bool) {
        self.controls.lock().unwrap().watch_meters(on);
    }

    /// The status panel opened (`true`) or closed: link figures are sent only while it is open.
    pub fn watch_link(&self, on: bool) {
        self.link.set_watching(on);
    }

    /// A fresh page (first load or a webview reload): the meters and status panel the previous
    /// page showed are gone, and their close calls will never come.
    pub fn reset_page_views(&self) {
        self.controls.lock().unwrap().reset_meters();
        self.link.set_watching(false);
    }

    pub async fn set_peer_volume(&self, user_id: String, pct: u16) {
        self.volumes
            .lock()
            .unwrap()
            .insert(user_id.clone(), pct.min(200));
        if let Some(s) = self.session.lock().await.as_ref() {
            s.pipeline
                .shared
                .mix_in
                .set_gain(&user_id, SourceKind::Voice, percent_to_gain(pct));
        }
    }

    /// Apply new device/processing settings: processing changes live; device changes reopen the streams.
    pub async fn set_audio_config(&self, cfg: AudioConfig) -> Result<(), VoiceError> {
        // a rejoin uses the newest settings
        if let Some(a) = self.last_join.lock().unwrap().as_mut() {
            a.cfg = cfg.clone();
        }
        let Some(p) = self
            .session
            .lock()
            .await
            .as_ref()
            .map(|s| s.pipeline.clone())
        else {
            return Ok(());
        };
        // Hot-swap the side that changed, stay in the room (rates may differ; the 48 kHz edge
        // absorbs it). Not under the session lock: opening can take seconds, and Leave, mute and
        // volume must not wait on it (the reopener takes turns and checks `alive`).
        p.adopt(&cfg).await
    }

    /// The live call's mixer input and its current generation, so stream audio received on the
    /// `#a` connection plays through the same speakers. `None` outside a live call.
    pub async fn mixer_input(&self) -> Option<(Arc<mixer::MixerInput>, u64)> {
        self.session
            .lock()
            .await
            .as_ref()
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .map(|s| {
                let m = s.pipeline.shared.mix_in.clone();
                let g = m.generation();
                (m, g)
            })
    }

    // --- diagnostics (tests) ---
    pub async fn mixer_pushed(&self) -> u64 {
        match self.session.lock().await.as_ref() {
            Some(s) => s.pipeline.shared.mix_in.pushed_total(),
            None => 0,
        }
    }

    /// Mic audio waiting to go to LiveKit right now, in ms (tests; the log has the worst per window).
    pub async fn send_queue_ms(&self) -> u32 {
        match self.session.lock().await.as_ref() {
            Some(s) => s.pipeline.shared.mic_queue.current(),
            None => 0,
        }
    }

    /// Device reopens of the current pipeline so far.
    pub async fn reopens(&self) -> u32 {
        match self.session.lock().await.as_ref() {
            Some(s) => s.pipeline.reopener.count.load(Ordering::Relaxed),
            None => 0,
        }
    }

    pub async fn rx_len(&self) -> usize {
        match self.session.lock().await.as_ref() {
            Some(s) => s.rx.lock().unwrap().len(),
            None => 0,
        }
    }

    /// Loudest voice level per person since the last call (tests: sound, not just frames).
    pub async fn mixer_levels(&self) -> Vec<(String, f32)> {
        match self.session.lock().await.as_ref() {
            Some(s) => s.pipeline.shared.mix_in.take_peer_levels(),
            None => Vec::new(),
        }
    }

    /// Sources in the mixer: with `rx_len`, shows each track is wired once.
    pub async fn mixer_sources(&self) -> usize {
        match self.session.lock().await.as_ref() {
            Some(s) => s.pipeline.shared.mix_in.source_count(),
            None => 0,
        }
    }
}

/// How the mic is sent. The SDK's default (`None`) is 48 kbps; Discord defaults to 64 kbps.
/// DTX makes silence ~free and RED (redundant audio) hides packet loss.
/// Also used by tests/neteq_probe.rs, so the probe sends exactly like the app.
pub fn mic_publish_options() -> TrackPublishOptions {
    TrackPublishOptions {
        source: TrackSource::Microphone,
        audio_encoding: Some(livekit::options::AudioEncoding {
            max_bitrate: 64_000,
        }),
        dtx: true,
        red: true,
        ..Default::default()
    }
}

/// Stream audio (a shared game's sound) gets its own volume and never lights the speaking ring.
fn source_kind(s: TrackSource) -> SourceKind {
    match s {
        TrackSource::ScreenshareAudio => SourceKind::Stream,
        _ => SourceKind::Voice,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_share_audio_is_stream_everything_else_is_voice() {
        assert_eq!(
            source_kind(TrackSource::ScreenshareAudio),
            SourceKind::Stream
        );
        assert_eq!(source_kind(TrackSource::Microphone), SourceKind::Voice);
        // an unknown source plays like a voice: never silently dropped
        assert_eq!(source_kind(TrackSource::Unknown), SourceKind::Voice);
    }

    #[tokio::test(start_paused = true)]
    async fn devices_open_while_the_room_connects() {
        let started = tokio::time::Instant::now();
        let r = side_by_side(
            async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                Ok::<_, VoiceError>("devices")
            },
            async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                Ok::<_, VoiceError>("room")
            },
            |_| async {},
        )
        .await;
        assert!(matches!(r, Ok(("devices", "room"))));
        assert_eq!(
            started.elapsed(),
            Duration::from_millis(300),
            "one after the other"
        );
    }

    /// Devices that won't open after the room connected: the room must be closed (dropping a
    /// Room doesn't disconnect it; others would see a ghost).
    #[tokio::test(start_paused = true)]
    async fn a_device_failure_closes_the_connected_room() {
        let closed = Arc::new(AtomicBool::new(false));
        let c = closed.clone();
        let r = side_by_side(
            async { Err::<(), _>(VoiceError::Device("no mic".into())) },
            async { Ok::<_, VoiceError>("room") },
            move |room| async move {
                assert_eq!(room, "room");
                c.store(true, Ordering::SeqCst);
            },
        )
        .await;
        assert!(matches!(r, Err(VoiceError::Device(_))));
        assert!(closed.load(Ordering::SeqCst));
    }

    /// The room fails: the devices opened meanwhile are closed (dropped), not left running.
    #[tokio::test(start_paused = true)]
    async fn a_room_failure_drops_the_opened_devices() {
        struct Io(Arc<AtomicBool>);
        impl Drop for Io {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let r = side_by_side(
            async { Ok::<_, VoiceError>(Io(dropped.clone())) },
            async { Err::<(), _>(VoiceError::Connect("refused".into())) },
            |_| async {},
        )
        .await;
        assert!(matches!(r, Err(VoiceError::Connect(_))));
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[tokio::test(start_paused = true)]
    async fn both_failing_reports_the_room_error() {
        let r = side_by_side(
            async { Err::<(), _>(VoiceError::Device("no mic".into())) },
            async { Err::<(), _>(VoiceError::Connect("refused".into())) },
            |_| async { unreachable!("no room to undo") },
        )
        .await;
        assert!(matches!(r, Err(VoiceError::Connect(_))));
    }

    struct FakeRoom(Arc<AtomicBool>);
    impl CloseRoom for FakeRoom {
        async fn close_room(&self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    /// A cancelled join drops its guarded room: it must be closed, not left connected (a ghost).
    #[tokio::test]
    async fn a_dropped_open_room_is_closed() {
        let closed = Arc::new(AtomicBool::new(false));
        drop(OpenRoom::new(FakeRoom(closed.clone())));
        for _ in 0..100 {
            if closed.load(Ordering::SeqCst) {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("the dropped room was never closed");
    }

    #[tokio::test]
    async fn a_disarmed_open_room_stays_open() {
        let closed = Arc::new(AtomicBool::new(false));
        OpenRoom::new(FakeRoom(closed.clone())).disarm();
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        assert!(
            !closed.load(Ordering::SeqCst),
            "a session's room was closed"
        );
    }

    /// A room whose close never finishes (a lossy link): records that it was started.
    struct StuckRoom(Arc<AtomicBool>);
    impl CloseRoom for StuckRoom {
        async fn close_room(&self) {
            self.0.store(true, Ordering::SeqCst);
            std::future::pending::<()>().await;
        }
    }

    type Frames = Arc<Mutex<usize>>;
    struct CountFrames(Frames);
    impl processor::FrameSink for CountFrames {
        fn send(&mut self, _frame: &[i16]) -> bool {
            *self.0.lock().unwrap() += 1;
            true
        }
    }

    fn counting(shared: &Shared) -> Frames {
        let n = Frames::default();
        shared.set_mic_sink(Some(Box::new(CountFrames(n.clone()))));
        n
    }

    fn wait_frames(n: &Frames, at_least: usize) -> usize {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while *n.lock().unwrap() < at_least && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        *n.lock().unwrap()
    }

    /// A pipeline with no devices open (`raw_tx` feeds its processor by hand).
    fn deviceless_pipeline() -> (Arc<Pipeline>, processor::RawTx) {
        let cfg = AudioConfig {
            noise_suppression: denoise::NsLevel::Off,
            ..Default::default()
        };
        let shared = Shared::new(&cfg, Default::default(), PEER_BUFFER_MS);
        let (raw_tx, _processor) = processor::spawn(shared.clone());
        let (io, alive) = (Arc::new(Mutex::new(None)), Arc::new(AtomicBool::new(true)));
        let cfg = Arc::new(Mutex::new(cfg));
        let reopener = Arc::new(Reopener {
            cfg: cfg.clone(),
            io: io.clone(),
            shared: shared.clone(),
            raw_tx: raw_tx.clone(),
            alive: alive.clone(),
            one_at_a_time: tokio::sync::Mutex::new(()),
            failed: Default::default(),
            count: Default::default(),
        });
        let p = Pipeline {
            shared,
            cfg,
            io,
            reopener,
            real: false,
            alive,
            tasks: Vec::new(),
            stalled: Default::default(),
        };
        (Arc::new(p), raw_tx)
    }

    /// A switch hands the mic to the new room: the old one gets nothing more, nor are its
    /// people still played; the switch doesn't wait for the old room's close (started in the
    /// background), and the pipeline (processor, devices) comes back for the next room.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_detached_room_gets_no_more_frames() {
        let (pipeline, raw_tx) = deviceless_pipeline();
        let shared = pipeline.shared.clone();
        let closing = Arc::new(AtomicBool::new(false));
        let session = Session {
            channel: ChannelId::new(),
            room: Arc::new(StuckRoom(closing.clone())),
            pipeline,
            rx: Default::default(),
            alive: Arc::new(AtomicBool::new(true)),
            tasks: Vec::new(),
        };
        let old = counting(&shared);
        shared.mix_in.add_source(
            shared.mix_in.generation(),
            "peer",
            "peer",
            SourceKind::Voice,
        );
        shared.mix_in.push("peer", &[100; 960]);
        let block = || (48_000, vec![0.0; 480], std::time::Instant::now());
        for _ in 0..5 {
            raw_tx.send(block()).unwrap();
        }
        assert!(wait_frames(&old, 5) >= 5);
        let kept = tokio::time::timeout(Duration::from_secs(1), session.detach())
            .await
            .expect("the switch waited for the old room's close");
        assert!(
            kept.alive.load(Ordering::SeqCst),
            "the switch gave up the pipeline"
        );
        assert!(
            shared.mix_in.take_peer_levels().is_empty(),
            "the old room's people are still in the mix"
        );
        let before = *old.lock().unwrap();
        for _ in 0..5 {
            raw_tx.send(block()).unwrap();
        }
        std::thread::sleep(Duration::from_millis(300)); // ample: a block takes well under 1 ms
        assert_eq!(
            *old.lock().unwrap(),
            before,
            "the old room kept getting our mic"
        );
        // the processor was still running (frames would have reached the old room)
        let probe = counting(&shared);
        raw_tx.send(block()).unwrap();
        assert!(wait_frames(&probe, 1) >= 1);
        for _ in 0..100 {
            if closing.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the old room was never closed");
    }

    async fn null_pipeline() -> Arc<Pipeline> {
        let p = Pipeline::build(
            &AudioConfig::default(),
            AudioMode::Null(48_000),
            Default::default(),
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
        Arc::new(p)
    }

    /// A device reopen in flight (it holds the reopener's turn, the devices taken out): `run`
    /// goes once it has the turn; the turn is released when it returns.
    async fn reopen_in_flight(
        p: &Arc<Pipeline>,
        run: impl FnOnce(&Pipeline) + Send + 'static,
    ) -> JoinHandle<()> {
        let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
        let p = p.clone();
        let h = tokio::spawn(async move {
            let _turn = p.reopener.one_at_a_time.lock().await;
            let _ = locked_tx.send(());
            run(&p);
        });
        locked_rx.await.unwrap();
        h
    }

    /// A switch that lands while the pipeline is mid-reopen (devices taken out) waits for the
    /// reopen and carries on with that pipeline, rather than throwing it away and opening the
    /// same devices next to it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_switch_waits_out_a_reopen_in_flight_and_keeps_the_pipeline() {
        let p = null_pipeline().await;
        let io = p.io.lock().unwrap().take();
        let reopen = reopen_in_flight(&p, move |p| {
            std::thread::sleep(Duration::from_millis(300));
            *p.io.lock().unwrap() = io;
        })
        .await;
        let t = Instant::now();
        let kept = keep_if_usable(Some(p.clone()), AudioMode::Null(48_000)).await;
        assert!(
            t.elapsed() >= Duration::from_millis(250),
            "{:?}",
            t.elapsed()
        );
        assert!(kept.is_some(), "a pipeline mid-reopen was thrown away");
        assert!(p.alive.load(Ordering::SeqCst));
        reopen.await.unwrap();
    }

    /// A kept pipeline that can't be used is let go before the next one is built: its reopen
    /// in flight has finished and its devices are closed (no two opens of one device at once).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_discarded_pipeline_is_settled_before_a_new_build() {
        let p = null_pipeline().await;
        let done = Arc::new(AtomicBool::new(false));
        let d = done.clone();
        let reopen = reopen_in_flight(&p, move |_| {
            std::thread::sleep(Duration::from_millis(300));
            d.store(true, Ordering::SeqCst);
        })
        .await;
        let kept = keep_if_usable(Some(p.clone()), AudioMode::Real).await; // other mode: not usable
        assert!(kept.is_none());
        assert!(done.load(Ordering::SeqCst), "the reopen was still running");
        assert!(!p.alive.load(Ordering::SeqCst));
        assert!(p.io.lock().unwrap().is_none(), "its devices are still open");
        reopen.await.unwrap();
    }

    /// A room that closes at once, noting when its close started.
    struct TimedRoom(Mutex<Option<Instant>>);
    impl CloseRoom for TimedRoom {
        async fn close_room(&self) {
            self.0.lock().unwrap().get_or_insert_with(Instant::now);
        }
    }

    fn session_on(pipeline: Arc<Pipeline>) -> (Session<TimedRoom>, Arc<TimedRoom>) {
        let room = Arc::new(TimedRoom(Mutex::new(None)));
        let session = Session {
            channel: ChannelId::new(),
            room: room.clone(),
            pipeline,
            rx: Default::default(),
            alive: Arc::new(AtomicBool::new(true)),
            tasks: Vec::new(),
        };
        (session, room)
    }

    /// LiveKit dropped the call and it went to the rejoin loop: the devices stay open for the
    /// rejoin's teardown, which closes them with a wait. The rejoin's own open (right after that
    /// teardown) must not start while the old device is still closing (a Bluetooth headset can't
    /// be open twice).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_rejoin_opens_devices_only_after_the_dropped_calls_have_closed() {
        let (p, raw_tx) = deviceless_pipeline();
        let closed = Arc::new(AtomicBool::new(false));
        *p.io.lock().unwrap() = Some(AudioIo::slow_to_close(
            p.shared.clone(),
            raw_tx,
            Duration::from_millis(300),
            closed.clone(),
        ));
        let (session, _room) = session_on(p.clone());
        let handed = drop_call(
            &session.alive,
            (&p.alive, &p.shared),
            &p.io,
            &session.rx,
            || true,
        );
        assert!(handed);
        assert!(!p.alive.load(Ordering::SeqCst), "the pipeline is done for");
        assert!(
            p.io.lock().unwrap().is_some(),
            "devices dropped without a wait"
        );
        session.end(false).await; // the rejoin's `leave_locked(false)`; it opens devices next
        assert!(
            closed.load(Ordering::SeqCst),
            "the rejoin's open started before the old device had closed"
        );
    }

    /// Not handed over (the rejoin loop isn't running, or LiveKit said not to): nothing is coming
    /// to close the devices, so they close now.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_dropped_call_not_rejoined_closes_its_devices_at_once() {
        let (p, raw_tx) = deviceless_pipeline();
        let closed = Arc::new(AtomicBool::new(false));
        *p.io.lock().unwrap() = Some(AudioIo::slow_to_close(
            p.shared.clone(),
            raw_tx,
            Duration::from_millis(10),
            closed,
        ));
        let (session, _room) = session_on(p.clone());
        assert!(!drop_call(
            &session.alive,
            (&p.alive, &p.shared),
            &p.io,
            &session.rx,
            || false
        ));
        assert!(p.io.lock().unwrap().is_none(), "devices left open");
    }

    /// The user's Leave during a device reopen (up to SETTLE_WAIT): out of the room at once, not
    /// after the reopen; a rejoin's teardown still waits the reopen out first.
    #[tokio::test(flavor = "multi_thread")]
    async fn leave_closes_the_room_before_waiting_out_a_reopen() {
        for announce in [true, false] {
            let (p, _raw_tx) = deviceless_pipeline();
            let reopen =
                reopen_in_flight(&p, |_| std::thread::sleep(Duration::from_millis(300))).await;
            let (session, room) = session_on(p.clone());
            let t = Instant::now();
            session.end(announce).await;
            let close_after = room.0.lock().unwrap().expect("room never closed") - t;
            if announce {
                assert!(
                    close_after < Duration::from_millis(100),
                    "Leave stayed in the room {close_after:?}"
                );
            } else {
                assert!(
                    close_after >= Duration::from_millis(250),
                    "a rejoin's teardown closed before the reopen finished: {close_after:?}"
                );
            }
            reopen.await.unwrap();
        }
    }

    /// LiveKit holds its runtime weakly: without our handle it dies with each call's last Room
    /// and the next join rebuilds it (threads, factory: hundreds of ms on the first join).
    #[test]
    fn the_webrtc_runtime_outlives_its_rooms() {
        use livekit::rtc_engine::lk_runtime::LkRuntime;
        let vm = VoiceManager::new(Arc::new(|_| {}));
        vm.keep_runtime();
        let weak = Arc::downgrade(&LkRuntime::instance());
        assert!(
            weak.upgrade().is_some(),
            "torn down when its last user went"
        );
    }

    #[tokio::test]
    async fn signing_out_forgets_prefetched_tokens() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let lounge = api
            .channels(&token, s.id)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.kind == pulse_protocol::rest::ChannelKind::Voice)
            .unwrap();
        let vm = VoiceManager::new(Arc::new(|_| {}));
        vm.prefetch(&api, &token, lounge.id);
        vm.forget_tokens();
        assert!(
            vm.tokens
                .take(&token, lounge.id, Instant::now())
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn diagnostics_dont_wait_on_a_stuck_voice_session() {
        // a hung device reopen holds the session lock; the bug report must still go out
        let vm = VoiceManager::new(Arc::new(|_| {}));
        let _held = vm.session.lock().await;
        let d = tokio::time::timeout(Duration::from_secs(3), vm.diagnostics())
            .await
            .expect("diagnostics waited on the lock");
        assert_eq!(d["state"], "busy");
    }

    #[test]
    fn mic_event_serialises_for_the_ui() {
        let e = VoiceEvent::Mic {
            live: false,
            error: Some("Couldn't send your mic".into()),
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"kind":"mic","live":false,"error":"Couldn't send your mic"})
        );
    }

    /// What `report_publish` did, run with fakes.
    struct Reported {
        events: Vec<VoiceEvent>,
        closed: bool,
        torn_down: bool,
        /// Epochs handed to the rejoin loop.
        handed_back: Vec<u64>,
        /// `publish_retries` afterwards.
        retries: u32,
    }

    /// `rejoin`: Some(epoch) for a rejoin's session. `retries`: `publish_retries` before.
    async fn reported_as(
        published: Result<(), String>,
        connected: bool,
        alive: bool,
        rejoin: Option<u64>,
        retries: u32,
    ) -> Reported {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let events: EventSink = Arc::new(move |e| sink.lock().unwrap().push(e));
        let (tx, rx) = tokio::sync::oneshot::channel();
        if connected {
            tx.send(()).unwrap();
        } else {
            drop(tx);
        }
        let closed = Arc::new(AtomicBool::new(false));
        let (dropped_tx, mut dropped_rx) = mpsc::unbounded_channel();
        let watch = PublishWatch {
            alive: Arc::new(AtomicBool::new(alive)),
            room: Arc::new(FakeRoom(closed.clone())),
            events,
            controls: Arc::new(Mutex::new(Controls::default())),
            channel: ChannelId::new(),
            rejoin: rejoin.map(|e| (e, Some(dropped_tx))),
            retries: Arc::new(AtomicU32::new(retries)),
        };
        let torn_down = AtomicBool::new(false);
        report_publish(published, rx, &watch, || {
            torn_down.store(true, Ordering::SeqCst)
        })
        .await;
        let mut handed_back = Vec::new();
        while let Ok(e) = dropped_rx.try_recv() {
            handed_back.push(e);
        }
        let events = seen.lock().unwrap().clone();
        Reported {
            events,
            closed: closed.load(Ordering::SeqCst),
            torn_down: torn_down.into_inner(),
            handed_back,
            retries: watch.retries.load(Ordering::SeqCst),
        }
    }

    /// A user's own join.
    async fn reported(
        published: Result<(), String>,
        connected: bool,
        alive: bool,
    ) -> (Vec<VoiceEvent>, bool, bool) {
        let r = reported_as(published, connected, alive, None, 0).await;
        (r.events, r.closed, r.torn_down)
    }

    #[tokio::test]
    async fn a_published_mic_is_reported_live() {
        let (seen, closed, torn_down) = reported(Ok(()), true, true).await;
        assert!(
            matches!(
                &seen[..],
                [
                    VoiceEvent::Stage {
                        stage: JoinStage::Connected,
                        attempt: None
                    },
                    VoiceEvent::Mic {
                        live: true,
                        error: None
                    }
                ]
            ),
            "{seen:?}"
        );
        assert!(!closed && !torn_down);
    }

    /// A mic that made it in ends a streak of failed rejoin publishes.
    #[tokio::test]
    async fn a_published_mic_resets_the_publish_retries() {
        let r = reported_as(Ok(()), true, true, Some(3), 1).await;
        assert_eq!(r.retries, 0);
        assert!(r.handed_back.is_empty());
    }

    /// "Connected" was already shown: a mic that never made it in ends the call, and says why.
    #[tokio::test]
    async fn a_failed_publish_ends_the_call_with_an_error() {
        let r = reported_as(Err("timed out".into()), true, true, None, 0).await;
        assert!(r.closed && r.torn_down);
        assert!(r.handed_back.is_empty(), "a user's join isn't retried");
        let seen = r.events;
        assert!(
            matches!(
                &seen[..],
                [
                    VoiceEvent::Mic { live: false, error: Some(e) },
                    VoiceEvent::State { channel_id: None, connection: Connection::Disconnected, .. },
                ] if e == "Couldn't send your mic: timed out"
            ),
            "{seen:?}"
        );
    }

    /// Right after a drop is when a publish most likely times out: a rejoin's failed publish goes
    /// back to the rejoin loop (still "reconnecting"), not to "disconnected".
    #[tokio::test]
    async fn a_rejoins_failed_publish_is_rejoined_again() {
        let r = reported_as(Err("timed out".into()), true, true, Some(7), 0).await;
        assert!(r.closed && r.torn_down);
        assert_eq!(r.handed_back, vec![7]);
        assert_eq!(r.retries, 1);
        let seen = r.events;
        assert!(
            matches!(
                &seen[..],
                [VoiceEvent::State {
                    channel_id: Some(_),
                    connection: Connection::Reconnecting,
                    ..
                }]
            ),
            "{seen:?}"
        );
    }

    /// Bounded: a rejoin whose publish keeps failing ends the call once the retries are used up.
    #[tokio::test]
    async fn a_rejoins_publish_failing_past_the_cap_ends_the_call() {
        let r = reported_as(
            Err("timed out".into()),
            true,
            true,
            Some(7),
            PUBLISH_RETRIES,
        )
        .await;
        assert!(r.closed && r.torn_down);
        assert!(r.handed_back.is_empty());
        let seen = r.events;
        assert!(
            matches!(
                &seen[..],
                [
                    VoiceEvent::Mic {
                        live: false,
                        error: Some(_)
                    },
                    VoiceEvent::State {
                        channel_id: None,
                        connection: Connection::Disconnected,
                        ..
                    },
                ]
            ),
            "{seen:?}"
        );
    }

    /// The session is already being torn down (a leave, or a rejoin replacing it): its close is
    /// what failed the publish. Say nothing: no "disconnected" flash during a rejoin.
    #[tokio::test]
    async fn a_publish_failing_during_teardown_says_nothing() {
        let (seen, closed, torn_down) = reported(Err("closed".into()), true, false).await;
        assert!(seen.is_empty(), "{seen:?}");
        assert!(!closed && !torn_down);
        let (seen, ..) = reported(Ok(()), true, false).await;
        assert!(seen.is_empty(), "{seen:?}");
        let r = reported_as(Err("closed".into()), true, false, Some(7), 0).await;
        assert!(r.events.is_empty() && r.handed_back.is_empty());
    }

    /// The join never got to "connected" (cancelled): its room guard closes the room; the
    /// publish reports nothing, so no event can land after the join's own "disconnected".
    #[tokio::test]
    async fn a_publish_for_a_join_that_never_connected_says_nothing() {
        for published in [Ok(()), Err("closed".to_string())] {
            let (seen, _, torn_down) = reported(published, false, true).await;
            assert!(seen.is_empty(), "{seen:?}");
            assert!(!torn_down);
        }
    }

    #[test]
    fn noise_suppression_event_serialises_for_the_ui() {
        let e = VoiceEvent::NoiseSuppression {
            active: crate::voice::denoise::NsLevel::Standard,
            note: Some("Strong unavailable, using Standard".into()),
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"kind":"noise_suppression","active":"standard","note":"Strong unavailable, using Standard"})
        );
        let l = VoiceEvent::Levels {
            mic: 0.0,
            speaker: 0.0,
            gate_open: true,
        };
        assert_eq!(serde_json::to_value(&l).unwrap()["gate_open"], true);
    }

    #[test]
    fn levels_go_to_the_ui_only_while_a_meter_is_shown() {
        let controls = Arc::new(Mutex::new(Controls::default()));
        let shared = Shared::new(&AudioConfig::default(), controls.clone(), PEER_BUFFER_MS);
        shared.add_mic_level(0.3, true);
        assert!(meter_update(&shared).is_none(), "nobody is looking");
        controls.lock().unwrap().watch_meters(true);
        shared.add_mic_level(0.3, true);
        match meter_update(&shared) {
            Some(VoiceEvent::Levels { mic, gate_open, .. }) => {
                assert!(
                    (mic - 0.3).abs() < 1e-3,
                    "only this tick's level, got {mic}"
                );
                assert!(gate_open);
            }
            other => panic!("expected levels, got {other:?}"),
        }
        controls.lock().unwrap().watch_meters(false);
        shared.add_mic_level(0.3, true);
        assert!(meter_update(&shared).is_none(), "meter closed");
    }

    #[test]
    fn mic_is_published_at_64_kbps_with_dtx_and_red() {
        let o = mic_publish_options();
        assert_eq!(o.audio_encoding.map(|e| e.max_bitrate), Some(64_000));
        assert!(o.dtx && o.red);
        assert_eq!(o.source, TrackSource::Microphone);
    }
    use pulse_server::testing;

    /// I7: a join that fails (here: text channel → 400, before any LiveKit work) must leave the UI
    /// in "disconnected", not stuck on "connecting".
    #[tokio::test]
    async fn failed_join_reports_disconnected() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let general = testing::general(&app, &token, s.id).await;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let vm = VoiceManager::new(Arc::new(move |e| sink.lock().unwrap().push(e)));
        let api = Api::new(&format!("http://{}", app.addr));
        let r = vm
            .join(
                &api,
                &token,
                general.id,
                AudioConfig::default(),
                AudioMode::Null(48_000),
            )
            .await;
        assert!(r.is_err());
        let last = seen.lock().unwrap().iter().rev().find_map(|e| match e {
            VoiceEvent::State {
                channel_id,
                connection,
                ..
            } => Some((*channel_id, *connection)),
            _ => None,
        });
        assert_eq!(last, Some((None, Connection::Disconnected)));
        assert_eq!(vm.current_channel().await, None);
    }

    /// Field bug 2026-10-02: a join stuck connecting held the op lock and Leave waited forever.
    /// Leave must cancel an in-flight join promptly, and the join must report Disconnected.
    #[tokio::test]
    async fn leave_cancels_a_stuck_join() {
        let mut cfg = testing::test_config();
        cfg.livekit_url = "ws://10.255.255.1:7880".into(); // never answers
        let app = testing::spawn_with(cfg).await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let lounge = api
            .channels(&token, s.id)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.kind == pulse_protocol::rest::ChannelKind::Voice)
            .unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let vm = Arc::new(VoiceManager::new(Arc::new(move |e| {
            sink.lock().unwrap().push(e)
        })));
        let (vm2, api2, token2) = (vm.clone(), api.clone(), token.clone());
        let joining = tokio::spawn(async move {
            vm2.join(
                &api2,
                &token2,
                lounge.id,
                AudioConfig::default(),
                AudioMode::Null(48_000),
            )
            .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let started = std::time::Instant::now();
        tokio::time::timeout(std::time::Duration::from_secs(2), vm.leave())
            .await
            .expect("leave waited behind a stuck join");
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        let r = tokio::time::timeout(std::time::Duration::from_secs(2), joining)
            .await
            .expect("join didn't stop")
            .unwrap();
        assert!(r.is_err(), "cancelled join must fail");
        assert_eq!(vm.current_channel().await, None);
        let last = seen.lock().unwrap().iter().rev().find_map(|e| match e {
            VoiceEvent::State { connection, .. } => Some(*connection),
            _ => None,
        });
        assert_eq!(last, Some(Connection::Disconnected));
    }

    /// `Stage`/`State` events as tags (`stage:connecting`, `stage:reconnecting:2`,
    /// `state:disconnected`), by their wire names.
    fn tags_of(seen: &Mutex<Vec<VoiceEvent>>) -> Vec<String> {
        fn wire(v: &impl Serialize) -> String {
            serde_json::to_value(v)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        }
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                VoiceEvent::Stage { stage, attempt } => Some(match attempt {
                    Some(n) => format!("stage:{}:{n}", wire(stage)),
                    None => format!("stage:{}", wire(stage)),
                }),
                VoiceEvent::State { connection, .. } => Some(format!("state:{}", wire(connection))),
                _ => None,
            })
            .collect()
    }

    fn position(tags: &[String], tag: &str) -> usize {
        tags.iter()
            .position(|t| t == tag)
            .unwrap_or_else(|| panic!("no {tag} in {tags:?}"))
    }

    /// A manager whose events land in the returned list.
    fn collecting_manager() -> (VoiceManager, Arc<Mutex<Vec<VoiceEvent>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let vm = VoiceManager::new(Arc::new(move |e| sink.lock().unwrap().push(e)));
        (vm, seen)
    }

    fn stale_link(vm: &VoiceManager) {
        *vm.link.server.lock().unwrap() = Some("voice.example.test".into());
        vm.link
            .history
            .lock()
            .unwrap()
            .push(tokio::time::Instant::now(), 40.0);
    }

    fn link_is_clear(vm: &VoiceManager) -> bool {
        vm.link.server.lock().unwrap().is_none()
            && vm
                .link
                .history
                .lock()
                .unwrap()
                .points(tokio::time::Instant::now())
                .is_empty()
    }

    /// A webview reload with the status panel open: the old page's close never comes, so the
    /// 1 s link polling must stop with it (the new page reopens it if it shows the panel).
    #[test]
    fn a_reloaded_page_stops_link_watching() {
        let (vm, _) = collecting_manager();
        vm.watch_link(true);
        vm.watch_meters(true);
        vm.reset_page_views();
        assert!(!vm.link.is_watching(), "link polling outlived the page");
        assert!(!vm.controls.lock().unwrap().meters_shown());
    }

    /// A join that never got a session (failed or cancelled) must not leave the last call's
    /// server and ping history behind.
    #[tokio::test]
    async fn leave_without_a_session_still_clears_the_link() {
        let (vm, _) = collecting_manager();
        stale_link(&vm);
        vm.leave().await;
        assert!(link_is_clear(&vm));
    }

    /// A channel switch detaches the old room: its server and history go with it.
    #[tokio::test]
    async fn detach_clears_the_link() {
        let (vm, _) = collecting_manager();
        stale_link(&vm);
        assert!(vm.detach_locked().await.is_none());
        assert!(link_is_clear(&vm));
    }

    /// alex's token, a client for `app`, and the voice channel of alex's new server.
    async fn alex_and_a_voice_channel(app: &testing::TestApp) -> (String, Api, ChannelId) {
        let (_, token) = testing::register(app, "alex").await;
        let s = testing::create_server(app, &token, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let lounge = api
            .channels(&token, s.id)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.kind == pulse_protocol::rest::ChannelKind::Voice)
            .unwrap();
        (token, api, lounge.id)
    }

    /// A config whose LiveKit refuses connections at once.
    fn unreachable_livekit() -> pulse_server::config::Config {
        let mut cfg = testing::test_config();
        cfg.livekit_url = "ws://127.0.0.1:1".into();
        cfg
    }

    async fn join_null(
        vm: &VoiceManager,
        api: &Api,
        token: &str,
        channel: ChannelId,
    ) -> Result<(), VoiceError> {
        vm.join(
            api,
            token,
            channel,
            AudioConfig::default(),
            AudioMode::Null(48_000),
        )
        .await
    }

    #[tokio::test]
    async fn a_failed_token_request_stops_after_requesting_access() {
        let app = testing::spawn().await;
        let (_, api, lounge) = alex_and_a_voice_channel(&app).await;
        let (_, sam) = testing::register(&app, "sam").await;
        let (vm, seen) = collecting_manager();
        assert!(join_null(&vm, &api, &sam, lounge).await.is_err());
        assert_eq!(
            tags_of(&seen),
            [
                "state:connecting",
                "stage:requesting_access",
                "state:disconnected"
            ]
        );
    }

    #[tokio::test]
    async fn a_failed_connect_stops_after_connecting() {
        let app = testing::spawn_with(unreachable_livekit()).await;
        let (token, api, lounge) = alex_and_a_voice_channel(&app).await;
        let (vm, seen) = collecting_manager();
        assert!(join_null(&vm, &api, &token, lounge).await.is_err());
        let tags = tags_of(&seen);
        assert!(
            position(&tags, "stage:requesting_access") < position(&tags, "stage:connecting")
                && position(&tags, "stage:connecting") < position(&tags, "state:disconnected"),
            "{tags:?}"
        );
        assert!(
            !tags.contains(&"stage:starting_audio".to_string()),
            "{tags:?}"
        );
    }

    /// A join that fails leaves no server or ping history (the previous call's are cleared as the
    /// join detaches that room; `connect` stores the host only once the room is up, with no
    /// await after it, so a cancel can't land between the two).
    #[tokio::test]
    async fn a_failed_join_leaves_the_link_clear() {
        let app = testing::spawn_with(unreachable_livekit()).await;
        let (token, api, lounge) = alex_and_a_voice_channel(&app).await;
        let (vm, _) = collecting_manager();
        stale_link(&vm);
        assert!(join_null(&vm, &api, &token, lounge).await.is_err());
        assert!(link_is_clear(&vm));
    }

    #[tokio::test]
    async fn rejoin_attempts_are_numbered() {
        let app = testing::spawn_with(unreachable_livekit()).await;
        let (token, api, lounge) = alex_and_a_voice_channel(&app).await;
        let (mut vm, seen) = collecting_manager();
        vm.rejoin_unit = Duration::from_millis(1);
        let vm = Arc::new(vm);
        let runner = {
            let vm = vm.clone();
            tokio::spawn(async move { vm.run_rejoins().await })
        };
        vm.simulate_drop(&api, &token, lounge).await;
        tokio::time::timeout(Duration::from_secs(10), async {
            while vm.rejoins() < rejoin::MAX_ATTEMPTS || vm.rejoining().is_some() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("rejoin never finished");
        let tags = tags_of(&seen);
        let numbered: Vec<&String> = tags
            .iter()
            .filter(|t| t.starts_with("stage:reconnecting"))
            .collect();
        let expected: Vec<String> = (1..=rejoin::MAX_ATTEMPTS)
            .map(|n| format!("stage:reconnecting:{n}"))
            .collect();
        assert_eq!(numbered, expected.iter().collect::<Vec<_>>());
        assert_eq!(tags.last().map(String::as_str), Some("state:disconnected"));
        runner.abort();
    }

    #[test]
    fn stage_event_serialises_for_the_ui() {
        let e = VoiceEvent::Stage {
            stage: JoinStage::Reconnecting,
            attempt: Some(2),
        };
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            r#"{"kind":"stage","stage":"reconnecting","attempt":2}"#
        );
    }

    async fn states_of(seen: &Mutex<Vec<VoiceEvent>>) -> Vec<(Option<ChannelId>, Connection)> {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                VoiceEvent::State {
                    channel_id,
                    connection,
                    ..
                } => Some((*channel_id, *connection)),
                _ => None,
            })
            .collect()
    }

    /// A4: LiveKit gave up → voice rejoins by itself, says "reconnecting" for that channel the
    /// whole time (no "disconnected" flash), and gives up after MAX_ATTEMPTS. A text channel makes
    /// every attempt fail fast at the token step (no LiveKit needed).
    #[tokio::test]
    async fn a_dropped_call_is_rejoined_then_given_up_on() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let general = testing::general(&app, &token, s.id).await;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let mut vm = VoiceManager::new(Arc::new(move |e| sink.lock().unwrap().push(e)));
        vm.rejoin_unit = Duration::from_millis(5);
        let vm = Arc::new(vm);
        let runner = {
            let vm = vm.clone();
            tokio::spawn(async move { vm.run_rejoins().await })
        };
        let api = Api::new(&format!("http://{}", app.addr));
        vm.simulate_drop(&api, &token, general.id).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while vm.rejoins() < rejoin::MAX_ATTEMPTS || vm.rejoining().is_some() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("rejoin never finished");
        let states = states_of(&seen).await;
        let (last, before) = states.split_last().unwrap();
        assert_eq!(
            *last,
            (None, Connection::Disconnected),
            "gave up: {states:?}"
        );
        assert!(
            before
                .iter()
                .all(|s| *s == (Some(general.id), Connection::Reconnecting)),
            "{states:?}"
        );
        assert_eq!(vm.rejoins(), rejoin::MAX_ATTEMPTS);
        runner.abort();
    }

    /// A4: the user's Leave wins over a rejoin in progress, at once and for good.
    #[tokio::test]
    async fn leaving_stops_a_rejoin() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let general = testing::general(&app, &token, s.id).await;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let mut vm = VoiceManager::new(Arc::new(move |e| sink.lock().unwrap().push(e)));
        vm.rejoin_unit = Duration::from_millis(50);
        let vm = Arc::new(vm);
        let runner = {
            let vm = vm.clone();
            tokio::spawn(async move { vm.run_rejoins().await })
        };
        let api = Api::new(&format!("http://{}", app.addr));
        vm.simulate_drop(&api, &token, general.id).await;
        tokio::time::timeout(Duration::from_secs(3), async {
            while vm.rejoins() < 1 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("no rejoin attempt");
        vm.leave().await;
        let n = vm.rejoins();
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(vm.rejoins(), n, "kept rejoining after Leave");
        assert_eq!(vm.rejoining(), None);
        assert_eq!(
            states_of(&seen).await.last(),
            Some(&(None, Connection::Disconnected))
        );
        runner.abort();
    }

    /// A rejoin attempt that's stale (it checked the epoch just before the user joined something
    /// else) must stand down, not cancel the user's join that's still connecting.
    #[tokio::test]
    async fn a_stale_rejoin_never_cancels_the_users_join() {
        let mut cfg = testing::test_config();
        cfg.livekit_url = "ws://10.255.255.1:7880".into(); // never answers: the join stays pending
        let app = testing::spawn_with(cfg).await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let lounge = api
            .channels(&token, s.id)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.kind == pulse_protocol::rest::ChannelKind::Voice)
            .unwrap();
        let vm = Arc::new(VoiceManager::new(Arc::new(|_| {})));
        let stale = vm.epoch.load(Ordering::SeqCst); // what the rejoin saw
        let (vm2, api2, token2) = (vm.clone(), api.clone(), token.clone());
        let users_join = tokio::spawn(async move {
            vm2.join(
                &api2,
                &token2,
                lounge.id,
                AudioConfig::default(),
                AudioMode::Null(48_000),
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(300)).await; // connecting now
        let r = vm
            .join_inner(
                &api,
                &token,
                lounge.id,
                AudioConfig::default(),
                AudioMode::Null(48_000),
                Some(stale),
            )
            .await;
        assert!(matches!(r, Err(VoiceError::Cancelled)), "{r:?}");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !users_join.is_finished(),
            "the stale rejoin cancelled the user's join"
        );
        // and Leave can still cancel it (the rejoin didn't take its cancel handle)
        tokio::time::timeout(Duration::from_secs(2), vm.leave())
            .await
            .expect("leave hung");
        let r = tokio::time::timeout(Duration::from_secs(2), users_join)
            .await
            .expect("Leave didn't cancel the user's join")
            .unwrap();
        assert!(matches!(r, Err(VoiceError::Cancelled)), "{r:?}");
    }

    fn idle_reopener() -> Arc<Reopener> {
        let cfg = AudioConfig::default();
        let shared = Shared::new(&cfg, Default::default(), PEER_BUFFER_MS);
        let (raw_tx, _raw_rx) = std::sync::mpsc::channel();
        Arc::new(Reopener {
            cfg: Arc::new(Mutex::new(cfg)),
            io: Arc::new(Mutex::new(None)),
            shared,
            raw_tx,
            alive: Arc::new(AtomicBool::new(false)),
            one_at_a_time: tokio::sync::Mutex::new(()),
            failed: Default::default(),
            count: Default::default(),
        })
    }

    /// A rejoin's teardown waits for a device reopen that's still running, so it doesn't open
    /// the same (Bluetooth) device while the reopen still holds it.
    #[tokio::test]
    async fn teardown_waits_for_a_device_reopen_in_flight() {
        let r = idle_reopener();
        let held = {
            let r = r.clone();
            tokio::spawn(async move {
                let _turn = r.one_at_a_time.lock().await; // a reopen in flight
                tokio::time::sleep(Duration::from_millis(300)).await;
            })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        let t = Instant::now();
        r.settle().await;
        assert!(
            t.elapsed() >= Duration::from_millis(250),
            "{:?}",
            t.elapsed()
        );
        held.await.unwrap();
    }
}
