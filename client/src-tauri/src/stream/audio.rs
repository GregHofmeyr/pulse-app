//! The `#a` connection to the stream room (`<channel>~s`, identity `<uid>#a`): publishes this
//! machine's stream audio and plays the stream audio of whoever we watch through the voice mixer.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::future::BoxFuture;
use livekit::options::TrackPublishOptions;
use livekit::prelude::*;
use livekit::webrtc::audio_frame::AudioFrame;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::StreamRole;
use serde_json::json;
use tokio::task::JoinHandle;

use super::loopback::Engine;
use super::plan::{Capture, PlanInput, Proc, Sound, Source, Tier, app_name, capture_plan, playing};
use super::procs;
use crate::api::{Api, ApiError};
use crate::voice::devices::INTERNAL_RATE;
use crate::voice::mixer::{MixerInput, SourceKind, percent_to_gain};

/// Published stream audio: 48 kHz stereo. A 100 ms queue, unlike the mic's 0: WASAPI hands
/// over bursts.
const OUT_RATE: u32 = 48_000;
const OUT_CHANNELS: u32 = 2;
const OUT_QUEUE_MS: u32 = 100;
/// How often the capture plan is recomputed (audio processes come and go).
const RESCAN_EVERY: Duration = Duration::from_secs(2);
/// How often the `audio_out` / `audio_in` lines are written.
const LOG_EVERY: Duration = Duration::from_secs(5);
/// How long connecting `#a` may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How often a receive task with no current mixer asks for one again.
const REBIND_EVERY: Duration = Duration::from_secs(1);

/// The live call's mixer input and its generation (`VoiceManager::mixer_input`), asked for
/// whenever a received track needs one.
pub type MixerSource =
    Arc<dyn Fn() -> BoxFuture<'static, Option<(Arc<MixerInput>, u64)>> + Send + Sync>;

fn log(v: serde_json::Value) {
    tracing::info!(target: "share", "share: {}", v);
}

/// The person behind a stream-room identity: `"<uid>#a"` → `"<uid>"`.
fn owner_of(identity: &str) -> &str {
    identity.split('#').next().unwrap_or_default()
}

/// A stream's volume: only the `Stream` kind, so it never changes how loud their voice is.
fn set_stream_volume(mixer: &MixerInput, user_id: &str, pct: u16) {
    mixer.set_gain(user_id, SourceKind::Stream, percent_to_gain(pct));
}

/// Why `#a` couldn't connect. The server's own answers stay `Api`, so a dead login (401) can
/// log out like it does for every other command. Serialized as its message, like theirs.
#[derive(Debug, thiserror::Error)]
pub enum StreamAudioError {
    #[error(transparent)]
    Api(#[from] ApiError),
    #[error("{0}")]
    Other(String),
}

impl From<String> for StreamAudioError {
    fn from(e: String) -> Self {
        Self::Other(e)
    }
}

impl serde::Serialize for StreamAudioError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// Told the channel whose `#a` connection LiveKit closed under us (never when we leave).
pub type OnClosed = Arc<dyn Fn(ChannelId) + Send + Sync>;

/// The `#a` connection. Every operation takes `state`, so two quick clicks can't both connect
/// or both publish (the first one's capture would be orphaned).
#[derive(Default)]
pub struct StreamAudio {
    /// In an `Arc` so the room's event task can tear down a connection LiveKit closed.
    state: Arc<tokio::sync::Mutex<State>>,
    /// Stream volume per person (percent), kept with no connection so a slider set early applies.
    volumes: Arc<Mutex<HashMap<String, u16>>>,
    on_closed: Option<OnClosed>,
}

#[derive(Default)]
struct State {
    conn: Option<Conn>,
    publishing: Option<Publishing>,
    /// Numbers connections, so a late "room closed" can't tear down a newer one.
    next_conn_id: u64,
}

struct Conn {
    id: u64,
    channel: ChannelId,
    /// Connected as a ghost (`g_<id>#a`, subscribe only), from outside the voice channel.
    ghost: bool,
    room: Room,
    watch: Arc<Watch>,
    events: JoinHandle<()>,
}

/// Who we watch and what we receive from them, shared with the room's event task.
struct Watch {
    /// The streamers' uids.
    streamers: Mutex<HashSet<String>>,
    /// Subscribed stream-audio tracks by SID.
    subs: Mutex<HashMap<String, Sub>>,
    mixer: MixerSource,
    volumes: Arc<Mutex<HashMap<String, u16>>>,
}

struct Publishing {
    track: LocalAudioTrack,
    sid: TrackSid,
    capturing: Capturing<Engine>,
    /// What the sharer chose and where it stands (`None` for the raw test track).
    sound: Option<SoundState>,
}

/// Counts for the `audio_out` line.
#[derive(Default)]
struct OutCounts {
    captures: AtomicUsize,
    chunks: AtomicU64,
    silent: AtomicU64,
    /// Chunks with sound in them since the source last changed (the "no sound yet" note).
    audible: AtomicU64,
}

/// A window share's identity from the page: the capture source's window handle, when the
/// webview gave one, and the title it showed.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowRef {
    pub hwnd: Option<u64>,
    pub title: String,
}

/// What is being shared, resolved once when sharing starts.
#[derive(Clone, Debug, Default)]
struct Target {
    window: bool,
    /// The window's owners; empty when nobody could be identified (or a screen share).
    owners: Vec<u32>,
    /// The app's name for the controls ("Firefox").
    app: Option<String>,
}

impl Target {
    fn source(&self) -> Source {
        if self.window {
            Source::Window(self.owners.clone())
        } else {
            Source::Screen
        }
    }
}

/// Before going live: can the window's app be told apart, and is it playing now.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub identified: bool,
    pub app: Option<String>,
    pub playing: bool,
}

/// While live: which source is heard, and whether anything has been since it began.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SoundStatus {
    pub tier: Tier,
    pub app: Option<String>,
    /// Anything audible since this source began.
    pub heard: bool,
    /// Since this source began.
    pub ms: u64,
}

struct Choice {
    sound: Sound,
    tier: Tier,
    since: Instant,
}

/// The sharer's choice, shared with the rescans.
#[derive(Clone)]
struct SoundState {
    target: Arc<Target>,
    choice: Arc<Mutex<Choice>>,
    counts: Arc<OutCounts>,
}

impl SoundState {
    fn new(target: Target, sound: Sound, tier: Tier, counts: Arc<OutCounts>) -> Self {
        SoundState {
            target: Arc::new(target),
            choice: Arc::new(Mutex::new(Choice {
                sound,
                tier,
                since: Instant::now(),
            })),
            counts,
        }
    }

    /// A new source: it starts unheard.
    fn switch(&self, sound: Sound) {
        let mut c = self.choice.lock().unwrap();
        c.sound = sound;
        c.since = Instant::now();
        self.counts.audible.store(0, Ordering::Relaxed);
    }

    fn status(&self) -> SoundStatus {
        let c = self.choice.lock().unwrap();
        SoundStatus {
            tier: c.tier,
            app: match c.tier {
                Tier::App | Tier::Related => self.target.app.clone(),
                Tier::Everything | Tier::LastResort => None,
            },
            heard: self.counts.audible.load(Ordering::Relaxed) > 0,
            ms: c.since.elapsed().as_millis() as u64,
        }
    }

    /// Plans for the choice now and records its tier, logging a change.
    fn plan(&self) -> Vec<Capture> {
        let sound = self.choice.lock().unwrap().sound;
        let (plan, tier) = plan_now(&self.target, sound);
        let mut c = self.choice.lock().unwrap();
        if c.tier != tier {
            log(json!({"ev": "audio_tier", "tier": tier}));
            c.tier = tier;
        }
        plan
    }
}

/// Where published stream audio goes in: 10 ms chunks of interleaved stereo i16.
#[derive(Clone)]
pub struct Outlet {
    source: NativeAudioSource,
    rt: tokio::runtime::Handle,
    counts: Arc<OutCounts>,
}

impl Outlet {
    /// Sends one chunk. Blocks until it is queued: call it from a plain thread, not a task.
    pub fn send(&self, chunk: Vec<i16>) {
        self.counts.chunks.fetch_add(1, Ordering::Relaxed);
        if chunk.iter().all(|s| *s == 0) {
            self.counts.silent.fetch_add(1, Ordering::Relaxed);
        } else {
            self.counts.audible.fetch_add(1, Ordering::Relaxed);
        }
        let frame = AudioFrame {
            samples_per_channel: (chunk.len() / OUT_CHANNELS as usize) as u32,
            data: chunk.into(),
            sample_rate: OUT_RATE,
            num_channels: OUT_CHANNELS,
        };
        let _ = self.rt.block_on(self.source.capture_frame(&frame));
    }
}

/// One received stream-audio track's place in the mixer.
struct Slot {
    sid: String,
    owner: String,
    mixer: Option<(Arc<MixerInput>, u64)>,
}

impl Slot {
    fn new(sid: &str, owner: &str) -> Self {
        Slot {
            sid: sid.to_string(),
            owner: owner.to_string(),
            mixer: None,
        }
    }

    /// Plays this track through `mixer` (room `generation`) at `volume` percent, if one is set.
    fn bind(&mut self, mixer: Arc<MixerInput>, generation: u64, volume: Option<u16>) {
        self.unbind();
        if let Some(pct) = volume {
            set_stream_volume(&mixer, &self.owner, pct);
        }
        mixer.add_source(generation, &self.sid, &self.owner, SourceKind::Stream);
        self.mixer = Some((mixer, generation));
    }

    fn unbind(&mut self) {
        if let Some((m, g)) = self.mixer.take() {
            m.remove_source(g, &self.sid);
        }
    }

    /// Bound to a mixer whose room is still the current one.
    fn is_current(&self) -> bool {
        self.mixer
            .as_ref()
            .is_some_and(|(m, g)| m.generation() == *g)
    }

    /// Downmixes `data` (interleaved, `channels` wide) into `mono` and queues it. The mixer is
    /// mono for now. False when there is no current mixer to take it.
    fn push(&self, data: &[i16], channels: usize, mono: &mut Vec<i16>) -> bool {
        if !self.is_current() {
            return false;
        }
        let Some((m, g)) = &self.mixer else {
            return false;
        };
        let ch = channels.max(1);
        mono.clear();
        mono.extend(
            data.chunks_exact(ch)
                .map(|c| (c.iter().map(|s| *s as i32).sum::<i32>() / ch as i32) as i16),
        );
        m.push_in(*g, &self.sid, mono);
        true
    }
}

/// One subscribed stream-audio track: dropping it stops its task and takes it out of the mixer.
struct Sub {
    owner: String,
    task: JoinHandle<()>,
    slot: Arc<Mutex<Slot>>,
    /// Frames since the last `audio_in` line.
    frames: Arc<AtomicU64>,
    /// Frames with sound in them, ever (WebRTC hands over silent frames before any packet).
    audible: Arc<AtomicU64>,
}

impl Drop for Sub {
    fn drop(&mut self) {
        self.task.abort();
        self.slot.lock().unwrap().unbind();
    }
}

impl Watch {
    fn watches(&self, identity: &str) -> bool {
        identity.ends_with("#a") && self.streamers.lock().unwrap().contains(owner_of(identity))
    }

    fn subscribe(&self, track: RemoteAudioTrack, sid: String, owner: String) {
        let slot = Arc::new(Mutex::new(Slot::new(&sid, &owner)));
        let (frames, audible) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
        let (slot2, frames2, audible2, mixer, volumes) = (
            slot.clone(),
            frames.clone(),
            audible.clone(),
            self.mixer.clone(),
            self.volumes.clone(),
        );
        let task = tokio::spawn(async move {
            let mut stream = NativeAudioStream::new(track.rtc_track(), INTERNAL_RATE as i32, 2);
            let mut mono = Vec::new();
            let mut last_try: Option<Instant> = None;
            while let Some(f) = stream.next().await {
                frames2.fetch_add(1, Ordering::Relaxed);
                if f.data.iter().any(|s| *s != 0) {
                    audible2.fetch_add(1, Ordering::Relaxed);
                }
                // No mixer yet, or the voice call was rejoined (the mixer cleared): find the
                // current one, at most once a second.
                let current = slot2.lock().unwrap().is_current();
                if !current && last_try.is_none_or(|t| t.elapsed() >= REBIND_EVERY) {
                    last_try = Some(Instant::now());
                    if let Some((m, g)) = mixer().await {
                        let owner = slot2.lock().unwrap().owner.clone();
                        let volume = volumes.lock().unwrap().get(&owner).copied();
                        slot2.lock().unwrap().bind(m, g, volume);
                    }
                }
                slot2
                    .lock()
                    .unwrap()
                    .push(&f.data, f.num_channels as usize, &mut mono);
            }
        });
        // a re-subscribe under the same SID replaces (and so cleans up) the old entry
        let old = self.subs.lock().unwrap().insert(
            sid,
            Sub {
                owner,
                task,
                slot,
                frames,
                audible,
            },
        );
        drop(old);
    }

    fn log_frames(&self) {
        for s in self.subs.lock().unwrap().values() {
            let k = s.frames.swap(0, Ordering::Relaxed);
            log(json!({"ev": "audio_in", "streamer": s.owner, "frames": k}));
        }
    }

    fn clear(&self) {
        self.streamers.lock().unwrap().clear();
        let subs: Vec<Sub> = self.subs.lock().unwrap().drain().map(|(_, s)| s).collect();
        drop(subs);
    }
}

async fn run_events(
    mut events: tokio::sync::mpsc::UnboundedReceiver<RoomEvent>,
    watch: Arc<Watch>,
    state: Weak<tokio::sync::Mutex<State>>,
    conn_id: u64,
    on_closed: Option<(OnClosed, ChannelId)>,
) {
    let mut tick = tokio::time::interval(LOG_EVERY);
    tick.tick().await;
    loop {
        tokio::select! {
            ev = events.recv() => {
                let Some(ev) = ev else { break };
                match ev {
                    RoomEvent::TrackPublished { publication, participant }
                        if publication.source() == TrackSource::ScreenshareAudio
                            && watch.watches(participant.identity().as_str()) =>
                    {
                        publication.set_subscribed(true);
                    }
                    RoomEvent::TrackSubscribed {
                        track: RemoteTrack::Audio(t),
                        publication,
                        participant,
                    } if publication.source() == TrackSource::ScreenshareAudio => {
                        let identity = participant.identity().to_string();
                        if watch.watches(&identity) {
                            let owner = owner_of(&identity).to_string();
                            watch.subscribe(t, publication.sid().to_string(), owner);
                        } else {
                            // unwatched while the subscription was on its way
                            publication.set_subscribed(false);
                        }
                    }
                    RoomEvent::TrackUnsubscribed { publication, .. }
                    | RoomEvent::TrackUnpublished { publication, .. } => {
                        let sub = watch.subs.lock().unwrap().remove(&publication.sid().to_string());
                        drop(sub);
                    }
                    RoomEvent::Disconnected { reason } => {
                        log(json!({"ev": "audio_room_closed", "reason": format!("{reason:?}")}));
                        watch.clear();
                        // Stop publishing into the dead room. Not from this task: the teardown
                        // aborts it.
                        if let Some(state) = state.upgrade() {
                            let on_closed = on_closed.clone();
                            tokio::spawn(async move {
                                let mut st = state.lock().await;
                                if st.conn.as_ref().is_some_and(|c| c.id == conn_id) {
                                    StreamAudio::leave_locked(&mut st).await;
                                    drop(st);
                                    // the UI decides whether to connect again
                                    if let Some((f, channel)) = on_closed {
                                        f(channel);
                                    }
                                }
                            });
                        }
                    }
                    _ => {}
                }
            }
            _ = tick.tick() => watch.log_frames(),
        }
    }
}

/// Who owns the shared window (`None`: a screen share), and how they were found. Blocking.
fn resolve(window: Option<&WindowRef>) -> (Target, procs::How) {
    let Some(w) = window else {
        return (Target::default(), procs::How::None);
    };
    target_for(w, &procs::pulse_pids(), &procs::processes())
}

/// `w`'s owners among `all`, Pulse (`pulse`) left out, with the app's name.
fn target_for(w: &WindowRef, pulse: &[u32], all: &[Proc]) -> (Target, procs::How) {
    let (owners, how) = procs::resolve_window(w.hwnd, Some(&w.title), pulse, all);
    let app = owners
        .first()
        .and_then(|o| all.iter().find(|p| p.pid == *o))
        .map(|p| app_name(&p.exe))
        .filter(|n| !n.is_empty());
    let target = Target {
        window: true,
        owners,
        app,
    };
    (target, how)
}

/// The capture plan for `target` and `sound` now, and its tier. Blocking.
fn plan_now(target: &Target, sound: Sound) -> (Vec<Capture>, Tier) {
    let pulse_pids = procs::pulse_pids();
    let pulse_roots = procs::pulse_roots();
    let session_pids = procs::audio_session_pids();
    // the process table (one open per process) only where a tier reads it
    let all = if target.window && sound == Sound::App {
        procs::processes()
    } else {
        Vec::new()
    };
    capture_plan(&PlanInput {
        own_pid: std::process::id(),
        pulse_pids: &pulse_pids,
        pulse_roots: &pulse_roots,
        session_pids: &session_pids,
        procs: &all,
        source: target.source(),
        sound,
    })
}

/// Whether the window's app can be told apart, and is playing now (the picker's hint). Blocking.
pub fn probe(window: &WindowRef) -> Probe {
    let pulse_pids = procs::pulse_pids();
    let all = procs::processes();
    let (target, _) = target_for(window, &pulse_pids, &all);
    let session_pids = procs::audio_session_pids();
    let playing = playing(&PlanInput {
        own_pid: std::process::id(),
        pulse_pids: &pulse_pids,
        pulse_roots: &[],
        session_pids: &session_pids,
        procs: &all,
        source: target.source(),
        sound: Sound::App,
    });
    Probe {
        identified: !target.owners.is_empty(),
        app: target.app,
        playing,
    }
}

/// What the ticker rescans: the capture engine (a fake in tests). Dropping it stops capture
/// and joins its threads, so it must be dropped off the async workers.
trait Rescan: Send + Sync + 'static {
    fn apply(&self, plan: Vec<Capture>);
}

impl Rescan for Engine {
    fn apply(&self, plan: Vec<Capture>) {
        Engine::apply(self, plan)
    }
}

type Planner = Arc<dyn Fn() -> Vec<Capture> + Send + Sync>;

/// The capture behind a publication and its ticker, which logs `audio_out` every `LOG_EVERY`
/// and, given `rescan_every`, moves the engine to `planner`'s plan that often.
struct Capturing<E: Rescan> {
    stop: tokio::sync::oneshot::Sender<()>,
    ticker: JoinHandle<()>,
    engine: Option<Arc<E>>,
    /// Held across planning and applying, by the rescans and a switch alike: a rescan that
    /// planned for the old source can't apply after a switch has.
    applying: Arc<Mutex<()>>,
}

/// Plans and applies, under `applying`, on a blocking thread, awaited to the end (so
/// `Capturing::stop` never races it). Apply can block while stopped captures wind down.
async fn rescan_now<E: Rescan>(
    applying: Arc<Mutex<()>>,
    engine: Option<Arc<E>>,
    counts: Arc<OutCounts>,
    planner: Planner,
) {
    let _ = tokio::task::spawn_blocking(move || {
        let _held = applying.lock().unwrap();
        let plan = planner();
        counts.captures.store(plan.len(), Ordering::Relaxed);
        if let Some(e) = engine {
            e.apply(plan);
        }
    })
    .await;
}

impl<E: Rescan> Capturing<E> {
    fn start(
        counts: Arc<OutCounts>,
        engine: Option<E>,
        planner: Planner,
        rescan_every: Option<Duration>,
    ) -> Self {
        let engine = engine.map(Arc::new);
        let applying = Arc::new(Mutex::new(()));
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let ticker = tokio::spawn(run_ticker(
            counts,
            engine.clone(),
            planner,
            rescan_every,
            applying.clone(),
            stopped,
        ));
        Capturing {
            stop,
            ticker,
            engine,
            applying,
        }
    }

    /// Moves the engine to `planner`'s plan now (the sharer switched source).
    async fn replan(&self, counts: Arc<OutCounts>, planner: Planner) {
        rescan_now(self.applying.clone(), self.engine.clone(), counts, planner).await;
    }

    /// Stops the ticker (letting a rescan in flight finish), then the capture, on a blocking
    /// thread. Once this returns no capture is running.
    async fn stop(self) {
        let _ = self.stop.send(());
        let _ = self.ticker.await; // its engine handle is gone once it has ended
        if let Some(engine) = self.engine {
            debug_assert_eq!(Arc::strong_count(&engine), 1, "the ticker has let go");
            let _ = tokio::task::spawn_blocking(move || drop(engine)).await;
        }
    }
}

async fn run_ticker<E: Rescan>(
    counts: Arc<OutCounts>,
    engine: Option<Arc<E>>,
    planner: Planner,
    rescan_every: Option<Duration>,
    applying: Arc<Mutex<()>>,
    mut stopped: tokio::sync::oneshot::Receiver<()>,
) {
    let mut log_tick = tokio::time::interval(LOG_EVERY);
    let mut scan_tick = tokio::time::interval(rescan_every.unwrap_or(LOG_EVERY));
    log_tick.tick().await;
    scan_tick.tick().await;
    let rescan = rescan_every.is_some() && engine.is_some();
    loop {
        tokio::select! {
            biased;
            _ = &mut stopped => break,
            _ = log_tick.tick() => log(json!({
                "ev": "audio_out",
                "captures": counts.captures.load(Ordering::Relaxed),
                "chunks": counts.chunks.swap(0, Ordering::Relaxed),
                "silentChunks": counts.silent.swap(0, Ordering::Relaxed),
            })),
            _ = scan_tick.tick(), if rescan => {
                rescan_now(applying.clone(), engine.clone(), counts.clone(), planner.clone()).await;
            }
        }
    }
}

impl StreamAudio {
    /// `on_closed` hears about connections LiveKit closes under us.
    pub fn new(on_closed: OnClosed) -> Self {
        StreamAudio {
            on_closed: Some(on_closed),
            ..Default::default()
        }
    }

    /// Connects `#a` to `channel`'s stream room, once: a call for the channel it is already in,
    /// as the same kind of viewer, does nothing; one for another channel, or a ghost becoming
    /// themselves (they joined that call), leaves that one first.
    pub async fn ensure_connected(
        &self,
        api: &Api,
        token: &str,
        channel: ChannelId,
        ghost: bool,
        mixer: MixerSource,
    ) -> Result<(), StreamAudioError> {
        let mut st = self.state.lock().await;
        if let Some(c) = &st.conn {
            if reuses(c.channel, c.ghost, channel, ghost)
                && c.room.connection_state() != ConnectionState::Disconnected
            {
                return Ok(());
            }
            Self::leave_locked(&mut st).await;
        }
        let t = super::ticket_with_retry(api, token, channel, StreamRole::A, ghost).await?;
        let mut opts = crate::voice::room_options();
        opts.auto_subscribe = false; // only the streams we watch
        // bounded: the state lock is held meanwhile, and leave (on exit, too) waits for it
        let (room, events) =
            tokio::time::timeout(CONNECT_TIMEOUT, Room::connect(&t.url, &t.token, opts))
                .await
                .map_err(|_| "stream audio timed out connecting".to_string())?
                .map_err(|e| e.to_string())?;
        let watch = Arc::new(Watch {
            streamers: Mutex::new(HashSet::new()),
            subs: Mutex::new(HashMap::new()),
            mixer,
            volumes: self.volumes.clone(),
        });
        st.next_conn_id += 1;
        let id = st.next_conn_id;
        let events = tokio::spawn(run_events(
            events,
            watch.clone(),
            Arc::downgrade(&self.state),
            id,
            self.on_closed.clone().map(|f| (f, channel)),
        ));
        st.conn = Some(Conn {
            id,
            channel,
            ghost,
            room,
            watch,
            events,
        });
        Ok(())
    }

    /// Whether `#a` is in a stream room as a ghost, and watching anyone.
    pub async fn ghost_watching(&self) -> (bool, bool) {
        let st = self.state.lock().await;
        match &st.conn {
            Some(c) => (c.ghost, !c.watch.streamers.lock().unwrap().is_empty()),
            None => (false, false),
        }
    }

    /// The channel whose stream room `#a` is in, if any.
    pub async fn connected_channel(&self) -> Option<ChannelId> {
        self.state.lock().await.conn.as_ref().map(|c| c.channel)
    }

    /// Publishes this machine's stream audio, `sound` from `window` (`None`: a screen share,
    /// always everything but Pulse); see `plan::capture_plan` for the tiers. Rescanned every
    /// `RESCAN_EVERY` for every share, so sound that starts later is picked up. Replaces a
    /// stream already publishing. `muted` holds from the first frame.
    pub async fn start_publishing(
        &self,
        window: Option<WindowRef>,
        sound: Sound,
        muted: bool,
    ) -> Result<(), String> {
        let mut st = self.state.lock().await;
        Self::stop_publishing_locked(&mut st).await;
        if st.conn.is_none() {
            return Err("stream audio isn't connected".into());
        }
        let (target, how, plan, tier) = tokio::task::spawn_blocking(move || {
            let (target, how) = resolve(window.as_ref());
            let (plan, tier) = plan_now(&target, sound);
            (target, how, plan, tier)
        })
        .await
        .map_err(|e| e.to_string())?;
        // never the title: titles are never logged
        log(
            json!({"ev": "audio_source", "sound": sound, "window": target.window.then_some(how), "tier": tier}),
        );
        let (track, sid, outlet, counts) = Self::publish_locked(&st).await?;
        if muted {
            track.mute(); // before any capture feeds it
        }
        counts.captures.store(plan.len(), Ordering::Relaxed);
        let state = SoundState::new(target, sound, tier, counts.clone());
        let engine = Engine::start(plan, move |c| outlet.send(c));
        let s2 = state.clone();
        let planner: Planner = Arc::new(move || s2.plan());
        let capturing = Capturing::start(counts, Some(engine), planner, Some(RESCAN_EVERY));
        st.publishing = Some(Publishing {
            track,
            sid,
            capturing,
            sound: Some(state),
        });
        Ok(())
    }

    /// Switches the published stream's source now, without republishing (the "no sound yet"
    /// note's one click). Nothing happens when nothing is publishing.
    pub async fn set_sound(&self, sound: Sound) {
        let st = self.state.lock().await;
        let Some(p) = &st.publishing else { return };
        let Some(state) = p.sound.clone() else { return };
        state.switch(sound);
        log(json!({"ev": "audio_switch", "sound": sound}));
        let s2 = state.clone();
        p.capturing
            .replan(state.counts.clone(), Arc::new(move || s2.plan()))
            .await;
        // what the old source sent while the switch applied is not this source being heard
        state.switch(sound);
    }

    /// The published stream's source and whether it has been heard; `None` when not publishing.
    pub async fn sound_status(&self) -> Option<SoundStatus> {
        let st = self.state.lock().await;
        st.publishing
            .as_ref()
            .and_then(|p| p.sound.as_ref())
            .map(SoundState::status)
    }

    /// Publishes the stream-audio track with no capture behind it and hands back its input.
    /// For the integration tests (CI can't capture), which feed it frames of their own.
    #[doc(hidden)]
    pub async fn start_publishing_raw(&self) -> Result<Outlet, String> {
        let mut st = self.state.lock().await;
        Self::stop_publishing_locked(&mut st).await;
        let (track, sid, outlet, counts) = Self::publish_locked(&st).await?;
        st.publishing = Some(Publishing {
            track,
            sid,
            capturing: Capturing::start(counts, None, Arc::new(Vec::new), None),
            sound: None,
        });
        Ok(outlet)
    }

    /// Publishes a stream-audio track (nothing may be publishing yet); the caller records it.
    async fn publish_locked(
        st: &State,
    ) -> Result<(LocalAudioTrack, TrackSid, Outlet, Arc<OutCounts>), String> {
        let room = &st.conn.as_ref().ok_or("stream audio isn't connected")?.room;
        let source = NativeAudioSource::new(
            AudioSourceOptions::default(),
            OUT_RATE,
            OUT_CHANNELS,
            OUT_QUEUE_MS,
        );
        let track = LocalAudioTrack::create_audio_track(
            "stream-audio",
            RtcAudioSource::Native(source.clone()),
        );
        let publication = room
            .local_participant()
            .publish_track(
                LocalTrack::Audio(track.clone()),
                TrackPublishOptions {
                    source: TrackSource::ScreenshareAudio,
                    dtx: false,
                    red: false,
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        let counts = Arc::new(OutCounts::default());
        let outlet = Outlet {
            source,
            rt: tokio::runtime::Handle::current(),
            counts: counts.clone(),
        };
        Ok((track, publication.sid(), outlet, counts))
    }

    /// Stops publishing (capture included); watching carries on.
    pub async fn stop_publishing(&self) {
        let mut st = self.state.lock().await;
        Self::stop_publishing_locked(&mut st).await;
    }

    async fn stop_publishing_locked(st: &mut State) {
        let Some(p) = st.publishing.take() else {
            return;
        };
        // capture first: nothing is fed into a track being unpublished
        p.capturing.stop().await;
        if let Some(c) = &st.conn {
            let _ = c.room.local_participant().unpublish_track(&p.sid).await;
        }
    }

    /// Whether stream audio is being published (diagnostics, tests).
    pub async fn publishing(&self) -> bool {
        self.state.lock().await.publishing.is_some()
    }

    /// Received stream-audio frames with sound in them, over every track playing now
    /// (diagnostics, tests).
    pub async fn heard_frames(&self) -> u64 {
        let st = self.state.lock().await;
        st.conn.as_ref().map_or(0, |c| {
            let subs = c.watch.subs.lock().unwrap();
            subs.values()
                .map(|s| s.audible.load(Ordering::Relaxed))
                .sum()
        })
    }

    /// Mutes the published track (capture keeps running, so unmuting is instant).
    pub async fn set_muted(&self, muted: bool) {
        let st = self.state.lock().await;
        if let Some(p) = &st.publishing {
            if muted {
                p.track.mute();
            } else {
                p.track.unmute();
            }
        }
    }

    /// Starts or stops playing `streamer`'s stream audio: subscribes to their `#a` track now,
    /// or when they publish it later.
    pub async fn watch(&self, streamer: UserId, on: bool) -> Result<(), String> {
        let st = self.state.lock().await;
        let conn = st.conn.as_ref().ok_or("stream audio isn't connected")?;
        let uid = streamer.to_string();
        // the set first: a TrackPublished racing this scan then subscribes it either way
        if on {
            conn.watch.streamers.lock().unwrap().insert(uid.clone());
        } else {
            conn.watch.streamers.lock().unwrap().remove(&uid);
        }
        let identity = format!("{uid}#a");
        for p in conn.room.remote_participants().values() {
            if p.identity().as_str() != identity {
                continue;
            }
            for publication in p.track_publications().values() {
                if publication.source() == TrackSource::ScreenshareAudio {
                    publication.set_subscribed(on);
                }
            }
        }
        if !on {
            let gone: Vec<Sub> = {
                let mut subs = conn.watch.subs.lock().unwrap();
                let sids: Vec<String> = subs
                    .iter()
                    .filter(|(_, s)| s.owner == uid)
                    .map(|(k, _)| k.clone())
                    .collect();
                sids.iter().filter_map(|k| subs.remove(k)).collect()
            };
            drop(gone);
        }
        Ok(())
    }

    /// `user_id`'s stream volume, now and for any stream of theirs played later.
    pub fn set_volume(&self, mixer: Option<&MixerInput>, user_id: &str, pct: u16) {
        let pct = pct.min(200);
        self.volumes
            .lock()
            .unwrap()
            .insert(user_id.to_string(), pct);
        if let Some(m) = mixer {
            set_stream_volume(m, user_id, pct);
        }
    }

    /// Stops publishing and watching and closes the connection.
    pub async fn leave(&self) {
        let mut st = self.state.lock().await;
        Self::leave_locked(&mut st).await;
    }

    /// Leaves unless connected to `keep` (the voice channel we are in now, if any).
    pub async fn leave_unless_in(&self, keep: Option<ChannelId>) {
        let mut st = self.state.lock().await;
        if st.conn.as_ref().is_some_and(|c| Some(c.channel) == keep) {
            return;
        }
        Self::leave_locked(&mut st).await;
    }

    async fn leave_locked(st: &mut State) {
        Self::stop_publishing_locked(st).await;
        if let Some(c) = st.conn.take() {
            c.events.abort();
            c.watch.clear();
            let _ = c.room.close().await;
            log(json!({"ev": "audio_left"}));
        }
    }
}

/// An `#a` connection to `have` (as a ghost or not) serves a call for `want`: the same channel
/// as the same kind of viewer. A ghost who joined that call reconnects as themselves.
fn reuses(have: ChannelId, have_ghost: bool, want: ChannelId, want_ghost: bool) -> bool {
    have == want && have_ghost == want_ghost
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::mixer::{SourceKind, mixer, percent_to_gain};

    #[test]
    fn a_ghost_connection_is_reused_only_by_a_ghost_in_that_channel() {
        let (a, b): (ChannelId, ChannelId) = (
            "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
            "01ARZ3NDEKTSV4RRFFQ69G5FAW".parse().unwrap(),
        );
        assert!(reuses(a, true, a, true));
        assert!(reuses(a, false, a, false));
        assert!(
            !reuses(a, true, a, false),
            "joined the call: reconnect as yourself"
        );
        assert!(!reuses(a, false, a, true));
        assert!(!reuses(a, true, b, true));
    }

    /// A refused login comes back as the server's 401, not a string, so the command can log
    /// the user out like every other command does.
    #[tokio::test]
    async fn a_dead_login_is_unauthorized() {
        let app = pulse_server::testing::spawn().await;
        let api = Api::new(&format!("http://{}", app.addr));
        let channel: ChannelId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
        let none: MixerSource = Arc::new(|| Box::pin(async { None }));
        let r = StreamAudio::default()
            .ensure_connected(&api, "garbage", channel, false, none)
            .await;
        assert!(
            matches!(r, Err(StreamAudioError::Api(ApiError::Unauthorized))),
            "{r:?}"
        );
    }

    #[test]
    fn received_stream_audio_lands_in_the_mixer_as_stream_kind() {
        let (input, _speakers) = mixer(48_000, 200);
        let input = Arc::new(input);
        let mut slot = Slot::new("TR_game", owner_of("01STREAMER#a"));
        slot.bind(input.clone(), input.generation(), None);
        // one loud 10 ms stereo frame
        let frame: Vec<i16> = (0..960)
            .map(|i| if i % 2 == 0 { 20_000 } else { 10_000 })
            .collect();
        let mut mono = Vec::new();
        assert!(slot.push(&frame, 2, &mut mono));
        assert_eq!(mono.len(), 480, "downmixed to mono");
        assert_eq!(mono[0], 15_000);
        assert_eq!(
            input.describe("TR_game"),
            Some(("01STREAMER".to_string(), SourceKind::Stream, 1.0))
        );
        assert_eq!(input.pushed_total(), 480);
        assert!(
            input.take_peer_levels().is_empty(),
            "a stream never lights the speaking ring"
        );
        slot.unbind();
        assert_eq!(input.source_count(), 0);
    }

    #[test]
    fn stream_volume_only_touches_stream_kind() {
        let (input, _speakers) = mixer(48_000, 200);
        let g = input.generation();
        input.add_source(g, "TR_mic", "alex", SourceKind::Voice);
        input.add_source(g, "TR_game", "alex", SourceKind::Stream);
        set_stream_volume(&input, "alex", 50);
        let (_, _, mic) = input.describe("TR_mic").unwrap();
        let (_, _, game) = input.describe("TR_game").unwrap();
        assert_eq!(mic, 1.0);
        assert_eq!(game, percent_to_gain(50));
    }

    /// A capture engine whose rescans take a while; it records what happened, and where.
    struct SlowEngine(Arc<Mutex<Vec<(&'static str, std::thread::ThreadId)>>>);

    impl Rescan for SlowEngine {
        fn apply(&self, _plan: Vec<Capture>) {
            self.0
                .lock()
                .unwrap()
                .push(("apply_start", std::thread::current().id()));
            std::thread::sleep(Duration::from_millis(300));
            self.0
                .lock()
                .unwrap()
                .push(("apply_end", std::thread::current().id()));
        }
    }

    impl Drop for SlowEngine {
        fn drop(&mut self) {
            self.0
                .lock()
                .unwrap()
                .push(("dropped", std::thread::current().id()));
        }
    }

    /// Stopping mid-rescan waits it out, then drops the engine (joining capture threads) on a
    /// blocking thread, never the async worker, and before `stop` returns.
    #[tokio::test] // current-thread: the async worker is this test's thread
    async fn stopping_capture_mid_rescan_drops_the_engine_after_it_off_the_async_worker() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let capturing = Capturing::start(
            Arc::new(OutCounts::default()),
            Some(SlowEngine(seen.clone())),
            Arc::new(Vec::new),
            Some(Duration::from_millis(20)),
        );
        // until a rescan is in flight
        while seen.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        capturing.stop().await;
        let seen = seen.lock().unwrap();
        let steps: Vec<&str> = seen.iter().map(|(s, _)| *s).collect();
        assert_eq!(steps, ["apply_start", "apply_end", "dropped"]);
        assert_ne!(
            seen[2].1,
            std::thread::current().id(),
            "dropped on the async worker"
        );
    }

    #[test]
    fn a_switch_resets_heard_and_the_app_shows_only_for_its_tiers() {
        let counts = Arc::new(OutCounts::default());
        let target = Target {
            window: true,
            owners: vec![40],
            app: Some("Firefox".into()),
        };
        let s = SoundState::new(target, Sound::App, Tier::App, counts.clone());
        let st = s.status();
        assert_eq!(
            (st.tier, st.app.as_deref(), st.heard),
            (Tier::App, Some("Firefox"), false)
        );
        counts.audible.store(3, Ordering::Relaxed);
        assert!(s.status().heard);
        std::thread::sleep(Duration::from_millis(5));
        let before = s.status().ms;
        s.switch(Sound::Everything);
        assert!(!s.status().heard, "a new source starts unheard");
        assert!(s.status().ms < before.max(5));
        s.choice.lock().unwrap().tier = Tier::Everything;
        assert_eq!(s.status().app, None);
    }

    #[test]
    fn a_window_ref_reads_the_page_shape() {
        let w: WindowRef = serde_json::from_str(r#"{"hwnd":1312,"title":"Game"}"#).unwrap();
        assert_eq!((w.hwnd, w.title.as_str()), (Some(1312), "Game"));
        let w: WindowRef = serde_json::from_str(r#"{"hwnd":null,"title":"Game"}"#).unwrap();
        assert_eq!(w.hwnd, None);
        assert_eq!(
            serde_json::to_string(&Tier::LastResort).unwrap(),
            r#""lastResort""#
        );
        assert_eq!(
            serde_json::from_str::<Sound>(r#""everything""#).unwrap(),
            Sound::Everything
        );
    }

    #[test]
    fn a_stale_slot_reports_it_needs_rebinding() {
        let (input, _speakers) = mixer(48_000, 200);
        let input = Arc::new(input);
        let mut slot = Slot::new("TR_game", "alex");
        assert!(!slot.push(&[1, 1], 2, &mut Vec::new()), "unbound");
        slot.bind(input.clone(), input.generation(), Some(50));
        assert_eq!(input.describe("TR_game").unwrap().2, percent_to_gain(50));
        input.clear(); // the voice room was rejoined
        assert!(!slot.push(&[1, 1], 2, &mut Vec::new()), "stale generation");
    }
}
