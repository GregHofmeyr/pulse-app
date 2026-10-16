//! Voice: our own capture/playback pipeline over LiveKit (rules: spikes/voice/FINDINGS.md).

pub mod backlog;
pub mod controls;
pub mod denoise;
pub mod devices;
pub mod gate;
pub mod meter;
pub mod mictest;
pub mod mixer;
pub mod netstats;
pub mod playout;
pub mod processor;
pub mod resampler;
pub mod rtprio;
pub mod rx;
pub mod speaking;
pub mod stats;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
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
use tokio::task::JoinHandle;

use crate::api::{Api, ApiError};
use controls::Controls;
use devices::{AudioConfig, AudioIo, INTERNAL_RATE, Shared, Sides};
use mixer::{Mixer, percent_to_gain};
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

/// Reopens the session's devices (stays in the room). Stall recovery, device following and
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
        if let Some(io) = io
            && self.alive.load(Ordering::SeqCst)
        {
            tracing::info!(?sides, "voice: devices reopened");
            self.shared.watchdog.rearm(Instant::now());
            self.failed
                .lock()
                .unwrap()
                .record(sides, &targets, &io.opened(), Instant::now());
            *self.io.lock().unwrap() = Some(io);
        }
        result.map_err(|e| VoiceError::Device(e.to_string()))
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
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VoiceEvent {
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
}

pub type EventSink = Arc<dyn Fn(VoiceEvent) + Send + Sync>;

struct Session {
    channel: ChannelId,
    room: Arc<Room>,
    shared: Arc<Shared>,
    io: Arc<Mutex<Option<AudioIo>>>,
    cfg: Arc<Mutex<AudioConfig>>,
    real: bool,
    rx: Arc<Mutex<RxTasks>>,
    /// Cleared when LiveKit disconnects us for good: the session is then torn down.
    alive: Arc<AtomicBool>,
    reopener: Arc<Reopener>,
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for Session {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

pub struct VoiceManager {
    /// Serialises join/leave: two fast clicks can't leave a ghost connection behind.
    op: tokio::sync::Mutex<()>,
    /// Cancels an in-flight join (Leave must never queue behind a stuck connect).
    pending_join: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
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
}

impl VoiceManager {
    pub fn new(events: EventSink) -> Self {
        Self {
            op: tokio::sync::Mutex::new(()),
            pending_join: Mutex::new(None),
            session: tokio::sync::Mutex::new(None),
            controls: Arc::new(Mutex::new(Controls::default())),
            volumes: Arc::new(Mutex::new(HashMap::new())),
            events,
            live: Mutex::new(None),
        }
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
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        // a newer join supersedes an older pending one
        if let Some(old) = self.pending_join.lock().unwrap().replace(cancel_tx) {
            let _ = old.send(());
        }
        let _op = self.op.lock().await;
        self.leave_locked().await;
        tracing::info!(%channel, "voice: joining");
        self.emit_state(Some(channel), Connection::Connecting);
        let started = Instant::now();
        let outcome = tokio::select! {
            r = self.connect(api, token, channel, cfg, mode) => r,
            _ = cancel_rx => Err(VoiceError::Cancelled),
        };
        self.pending_join.lock().unwrap().take();
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match &outcome {
            Ok(_) => tracing::info!(elapsed_ms, "voice: joined"),
            Err(e) => tracing::warn!(error = %e, elapsed_ms, "voice: join failed"),
        }
        match outcome {
            Ok(session) => {
                *self.live.lock().unwrap() = Some(session.alive.clone());
                *self.session.lock().await = Some(session);
                self.emit_state(Some(channel), Connection::Connected);
                Ok(())
            }
            Err(e) => {
                self.emit_state(None, Connection::Disconnected);
                Err(e)
            }
        }
    }

    async fn connect(
        &self,
        api: &Api,
        token: &str,
        channel: ChannelId,
        cfg: AudioConfig,
        mode: AudioMode,
    ) -> Result<Session, VoiceError> {
        // join phases, for finding where a slow join's time goes (bug reports carry the log)
        let started = Instant::now();
        let ms = |t: Instant| t.elapsed().as_millis() as u64;
        let vt = api.voice_token(token, channel).await?;
        let token_ms = ms(started);
        let t = Instant::now();

        let mixer = Arc::new(Mutex::new(Mixer::new(INTERNAL_RATE, PEER_BUFFER_MS)));
        let shared = Shared::new(&cfg, self.controls.clone(), mixer.clone());
        // devices → processor (denoise, gate) → the room's mic track, once there is one
        let (raw_tx, _processor) = processor::spawn(shared.clone());
        let io = match mode {
            AudioMode::Real => start_audio(&cfg, shared.clone(), raw_tx.clone()).await?,
            AudioMode::Null(rate) => AudioIo::start_null(rate, shared.clone(), raw_tx.clone()),
        };
        let devices_ms = ms(t);
        let t = Instant::now();

        let (room, mut room_events) = tokio::time::timeout(
            CONNECT_TIMEOUT,
            Room::connect(&vt.url, &vt.token, RoomOptions::default()),
        )
        .await
        .map_err(|_| VoiceError::Timeout("connecting to voice"))?
        .map_err(|e| VoiceError::Connect(e.to_string()))?;
        let room_connect_ms = ms(t);
        let t = Instant::now();
        tracing::info!("voice: room connected");
        // Fixed 48 kHz towards LiveKit, whatever the device does.
        // queue 0 = WebRTC's fast path: each 10 ms frame goes straight out. A queue there drains on
        // the system clock and is never trimmed, so any backlog in it (a burst, a mic clock running
        // fast) stays as delay; our own queue (the processor's, by age) is bounded and measured instead.
        let source = NativeAudioSource::new(AudioSourceOptions::default(), INTERNAL_RATE, 1, 0);
        let track =
            LocalAudioTrack::create_audio_track("mic", RtcAudioSource::Native(source.clone()));
        // from now on the mic processor hands each 10 ms frame straight to WebRTC
        shared.set_mic_sink(Some(Box::new(processor::LiveKitSink::new(source.clone()))));
        let published = tokio::time::timeout(
            PUBLISH_TIMEOUT,
            room.local_participant()
                .publish_track(LocalTrack::Audio(track), mic_publish_options()),
        )
        .await;
        let failure = match published {
            Ok(Ok(_)) => None,
            Ok(Err(e)) => Some(VoiceError::Connect(e.to_string())),
            Err(_) => Some(VoiceError::Timeout("publishing the mic")),
        };
        if let Some(e) = failure {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, room.close()).await; // dropping a Room does not disconnect it
            return Err(e);
        }
        tracing::info!(
            token_ms,
            devices_ms,
            room_connect_ms,
            publish_ms = ms(t),
            "voice: mic published"
        );
        let room = Arc::new(room);

        let mut tasks = Vec::new();
        // what WebRTC measures about the path, for telling network lag from local lag
        {
            let (room, mixer) = (room.clone(), mixer.clone());
            let shared = shared.clone();
            tasks.push(tokio::spawn(async move {
                let mut totals = netstats::Totals::default();
                let mut last_codec = None;
                let mut tick = tokio::time::interval(netstats::WINDOW);
                loop {
                    tick.tick().await;
                    if let Ok(s) = room.get_stats().await {
                        // which codec our mic is sent with (stereo? FEC/DTX?), once per change
                        if let Some(c) = netstats::send_codec(&s.publisher_stats)
                            && last_codec.as_ref() != Some(&c)
                        {
                            tracing::info!(codec = %c, "voice: send codec");
                            last_codec = Some(c);
                        }
                        let (line, t) =
                            netstats::summarize(&s.publisher_stats, &s.subscriber_stats, &totals);
                        totals = t;
                        // ours: worst send backlog this window, and the pipeline's counters
                        let send_ms = shared.mic_queue.take_worst();
                        let (underrun, adjust) = {
                            let mut m = mixer.lock().unwrap();
                            (m.take_underrun_samples(), m.take_adjusted_samples())
                        };
                        let ours = shared.stats.take(underrun * 1000 / INTERNAL_RATE as u64);
                        tracing::info!(%line, send_ms, adjust, %ours, "voice: path");
                    }
                }
            }));
        }
        let rx = Arc::new(Mutex::new(RxTasks::default()));
        let io = Arc::new(Mutex::new(Some(io)));
        let alive = Arc::new(AtomicBool::new(true));

        // room events: subscriptions (FINDINGS rule 1), speaking, quality, connection state
        {
            let (rx, mixer, volumes, events) = (
                rx.clone(),
                mixer.clone(),
                self.volumes.clone(),
                self.events.clone(),
            );
            let (controls, io, alive) = (self.controls.clone(), io.clone(), alive.clone());
            let shared = shared.clone();
            tasks.push(tokio::spawn(async move {
                let state = |connection, channel_id| VoiceEvent::State {
                    channel_id,
                    connection,
                    controls: *controls.lock().unwrap(),
                };
                while let Some(ev) = room_events.recv().await {
                    match ev {
                        RoomEvent::TrackSubscribed {
                            track: RemoteTrack::Audio(t),
                            participant,
                            ..
                        } => {
                            let id = participant.identity().to_string();
                            let pct = volumes.lock().unwrap().get(&id).copied().unwrap_or(100);
                            mixer.lock().unwrap().set_gain(&id, percent_to_gain(pct));
                            let (m, peer) = (mixer.clone(), id.clone());
                            let subscribed = Instant::now();
                            tracing::info!(
                                peer = %id,
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
                                            since_subscribed_ms =
                                                subscribed.elapsed().as_millis() as u64,
                                            "voice: first sound"
                                        );
                                    }
                                    m.lock().unwrap().push(&peer, &f.data);
                                }
                            });
                            rx.lock().unwrap().replace(id, task);
                        }
                        RoomEvent::TrackUnsubscribed { participant, .. } => {
                            let id = participant.identity().to_string();
                            rx.lock().unwrap().drop_for(&id);
                            mixer.lock().unwrap().remove(&id);
                        }
                        RoomEvent::ParticipantDisconnected(p) => {
                            let id = p.identity().to_string();
                            rx.lock().unwrap().drop_for(&id);
                            mixer.lock().unwrap().remove(&id);
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
                            events(state(Connection::Reconnecting, Some(channel)))
                        }
                        RoomEvent::Reconnected => {
                            tracing::info!("voice: reconnected");
                            events(state(Connection::Connected, Some(channel)))
                        }
                        RoomEvent::Disconnected { reason } => {
                            tracing::warn!(?reason, "voice disconnected by LiveKit");
                            // Gone for good: stop the mic/speakers and mark the session dead.
                            alive.store(false, Ordering::SeqCst);
                            shared.set_mic_sink(None);
                            io.lock().unwrap().take();
                            rx.lock().unwrap().clear();
                            events(state(Connection::Disconnected, None));
                            break;
                        }
                        _ => {}
                    }
                }
            }));
        }

        // levels for the UI meter + speaking ring + device watchdog (FINDINGS rule 2)
        let me_id = room.local_participant().identity().to_string();
        let cfg = Arc::new(Mutex::new(cfg));
        let real = matches!(mode, AudioMode::Real);
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
        {
            let (shared, events, alive) = (shared.clone(), self.events.clone(), alive.clone());
            let reopener = reopener.clone();
            tasks.push(tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_millis(100));
                // after a slow reopen, carry on from now (a burst of catch-up ticks would judge
                // the new streams at once)
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut stall = devices::StallPolicy::default();
                let mut speaking = speaking::SpeakingTracker::default();
                let mut n = 0u32;
                loop {
                    tick.tick().await;
                    if !alive.load(Ordering::SeqCst) {
                        break;
                    }
                    let (mic, speaker) = shared.take_levels();
                    let queued = shared.mixer.lock().unwrap().max_buffered();
                    shared
                        .stats
                        .sample_recv_queue((queued * 1000 / INTERNAL_RATE as usize) as u32);
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
                    // Ring = actually audible: received levels per peer + what we really send.
                    let mut levels = shared.mixer.lock().unwrap().take_peer_levels();
                    levels.push((me_id.clone(), shared.take_sent_level()));
                    if let Some(user_ids) = speaking.update(&levels, Instant::now()) {
                        events(VoiceEvent::Speaking { user_ids });
                    }
                    n += 1;
                    if !real || !n.is_multiple_of(5) {
                        continue;
                    }
                    let now = Instant::now();
                    match stall.check(shared.watchdog.stalled(now), now) {
                        None => {}
                        Some(devices::StallAction::Report) => {
                            tracing::error!("audio device still stalled after reopening");
                            events(VoiceEvent::DeviceStalled);
                        }
                        Some(devices::StallAction::Recovered) => {
                            tracing::info!("voice: audio device recovered");
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
            }));
        }
        // follow the system default device / a returning chosen device (own task: enumeration
        // can block, and must not stall the levels and watchdog above)
        if real {
            let reopener = reopener.clone();
            tasks.push(tokio::spawn(async move {
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
            }));
        }

        Ok(Session {
            channel,
            room,
            shared,
            io,
            cfg,
            real,
            rx,
            alive,
            reopener,
            tasks,
        })
    }

    pub async fn leave(&self) {
        // Cancel a join that's still connecting first, so we never queue behind it.
        if let Some(cancel) = self.pending_join.lock().unwrap().take() {
            tracing::info!("voice: leave cancels an in-flight join");
            let _ = cancel.send(());
        }
        let _op = self.op.lock().await;
        self.leave_locked().await;
    }

    async fn leave_locked(&self) {
        let s = self.session.lock().await.take();
        if let Some(s) = s {
            tracing::info!("voice: leaving");
            // first: a reopen in flight must not open devices for a session that's gone
            s.alive.store(false, Ordering::SeqCst);
            s.shared.set_mic_sink(None);
            if tokio::time::timeout(CLOSE_TIMEOUT, s.room.close())
                .await
                .is_err()
            {
                tracing::warn!("voice: room close timed out; dropping it");
            }
            s.rx.lock().unwrap().clear();
            s.io.lock().unwrap().take();
            drop(s);
            self.emit_state(None, Connection::Disconnected);
        }
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
                Some(s.cfg.lock().unwrap().clone()),
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
        let channel = self.current_channel().await;
        let connection = if channel.is_some() {
            Connection::Connected
        } else {
            Connection::Disconnected
        };
        (self.events)(VoiceEvent::State {
            channel_id: channel,
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

    pub async fn set_peer_volume(&self, user_id: String, pct: u16) {
        self.volumes
            .lock()
            .unwrap()
            .insert(user_id.clone(), pct.min(200));
        if let Some(s) = self.session.lock().await.as_ref() {
            s.shared
                .mixer
                .lock()
                .unwrap()
                .set_gain(&user_id, percent_to_gain(pct));
        }
    }

    /// Apply new device/processing settings: processing changes live; device changes reopen the streams.
    pub async fn set_audio_config(&self, cfg: AudioConfig) -> Result<(), VoiceError> {
        let guard = self.session.lock().await;
        let Some(s) = guard.as_ref() else {
            return Ok(());
        };
        let changed = devices::Sides::changed(&s.cfg.lock().unwrap(), &cfg);
        s.shared.apply_config(&cfg);
        *s.cfg.lock().unwrap() = cfg.clone();
        if changed.any() && s.real {
            // Hot-swap the side that changed, stay in the room (rates may differ; the 48 kHz edge
            // absorbs it). Not under the session lock: opening can take seconds, and Leave, mute
            // and volume must not wait on it (the reopener takes turns and checks `alive`).
            let reopener = s.reopener.clone();
            drop(guard);
            let targets = (cfg.input.clone(), cfg.output.clone());
            reopener.reopen(changed, targets).await?;
        }
        Ok(())
    }

    // --- diagnostics (tests) ---
    pub async fn mixer_pushed(&self) -> u64 {
        match self.session.lock().await.as_ref() {
            Some(s) => s.shared.mixer.lock().unwrap().pushed_total(),
            None => 0,
        }
    }

    /// Mic audio waiting to go to LiveKit right now, in ms (tests; the log has the worst per window).
    pub async fn send_queue_ms(&self) -> u32 {
        match self.session.lock().await.as_ref() {
            Some(s) => s.shared.mic_queue.current(),
            None => 0,
        }
    }

    /// Device reopens in this session so far.
    pub async fn reopens(&self) -> u32 {
        match self.session.lock().await.as_ref() {
            Some(s) => s.reopener.count.load(Ordering::Relaxed),
            None => 0,
        }
    }

    pub async fn rx_len(&self) -> usize {
        match self.session.lock().await.as_ref() {
            Some(s) => s.rx.lock().unwrap().len(),
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
