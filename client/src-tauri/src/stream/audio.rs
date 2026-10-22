//! The `#a` connection to the stream room (`<channel>~s`, identity `<uid>#a`): publishes this
//! machine's stream audio and plays the stream audio of whoever we watch through the voice mixer.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
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
use super::plan::{Capture, PlanInput, capture_plan};
use super::procs;
use crate::api::Api;
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

/// The `#a` connection. Every operation takes `state`, so two quick clicks can't both connect
/// or both publish (the first one's capture would be orphaned).
#[derive(Default)]
pub struct StreamAudio {
    state: tokio::sync::Mutex<State>,
    /// Stream volume per person (percent), kept with no connection so a slider set early applies.
    volumes: Arc<Mutex<HashMap<String, u16>>>,
}

#[derive(Default)]
struct State {
    conn: Option<Conn>,
    publishing: Option<Publishing>,
}

struct Conn {
    channel: ChannelId,
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
    /// Logs `audio_out` and, for a capture plan that can change, rescans it.
    ticker: JoinHandle<()>,
    engine: Option<Arc<Engine>>,
}

/// Counts for the `audio_out` line.
#[derive(Default)]
struct OutCounts {
    captures: AtomicUsize,
    chunks: AtomicU64,
    silent: AtomicU64,
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
                        log(json!({"ev": "audio_disconnected", "reason": format!("{reason:?}")}));
                        watch.clear();
                    }
                    _ => {}
                }
            }
            _ = tick.tick() => watch.log_frames(),
        }
    }
}

/// The capture plan for now, and whether it should be rescanned (not when it follows a window).
fn plan_now(window_title: Option<&str>) -> (Vec<Capture>, bool) {
    let pulse_pids = procs::pulse_pids();
    let pulse_roots = procs::pulse_roots();
    let session_pids = procs::audio_session_pids();
    let window_pid = window_title.and_then(procs::window_pid);
    let from_window = window_pid.is_some_and(|p| !pulse_pids.contains(&p));
    let plan = capture_plan(&PlanInput {
        own_pid: std::process::id(),
        pulse_pids: &pulse_pids,
        pulse_roots: &pulse_roots,
        session_pids: &session_pids,
        window_pid,
    });
    (plan, !from_window)
}

/// Logs `audio_out` every `LOG_EVERY`; with `rescan`, also moves `engine` to the current plan
/// every `RESCAN_EVERY`.
async fn run_ticker(
    counts: Arc<OutCounts>,
    engine: Option<Arc<Engine>>,
    window_title: Option<String>,
    rescan: bool,
) {
    let mut log_tick = tokio::time::interval(LOG_EVERY);
    let mut scan_tick = tokio::time::interval(RESCAN_EVERY);
    log_tick.tick().await;
    scan_tick.tick().await;
    loop {
        tokio::select! {
            _ = log_tick.tick() => log(json!({
                "ev": "audio_out",
                "captures": counts.captures.load(Ordering::Relaxed),
                "chunks": counts.chunks.swap(0, Ordering::Relaxed),
                "silentChunks": counts.silent.swap(0, Ordering::Relaxed),
            })),
            _ = scan_tick.tick(), if rescan && engine.is_some() => {
                let (engine, counts, title) = (engine.clone(), counts.clone(), window_title.clone());
                // apply can block while stopped captures wind down
                let _ = tokio::task::spawn_blocking(move || {
                    let (plan, _) = plan_now(title.as_deref());
                    counts.captures.store(plan.len(), Ordering::Relaxed);
                    if let Some(e) = engine {
                        e.apply(plan);
                    }
                })
                .await;
            }
        }
    }
}

impl StreamAudio {
    /// Connects `#a` to `channel`'s stream room, once: a call for the channel it is already in
    /// does nothing; one for another channel leaves that one first.
    pub async fn ensure_connected(
        &self,
        api: &Api,
        token: &str,
        channel: ChannelId,
        mixer: MixerSource,
    ) -> Result<(), String> {
        let mut st = self.state.lock().await;
        if let Some(c) = &st.conn {
            if c.channel == channel && c.room.connection_state() != ConnectionState::Disconnected {
                return Ok(());
            }
            Self::leave_locked(&mut st).await;
        }
        let t = super::ticket_with_retry(api, token, channel, StreamRole::A)
            .await
            .map_err(|e| e.to_string())?;
        let mut opts = RoomOptions::default();
        opts.auto_subscribe = false; // only the streams we watch
        let (room, events) = Room::connect(&t.url, &t.token, opts)
            .await
            .map_err(|e| e.to_string())?;
        let watch = Arc::new(Watch {
            streamers: Mutex::new(HashSet::new()),
            subs: Mutex::new(HashMap::new()),
            mixer,
            volumes: self.volumes.clone(),
        });
        let events = tokio::spawn(run_events(events, watch.clone()));
        st.conn = Some(Conn {
            channel,
            room,
            watch,
            events,
        });
        Ok(())
    }

    /// The channel whose stream room `#a` is in, if any.
    pub async fn connected_channel(&self) -> Option<ChannelId> {
        self.state.lock().await.conn.as_ref().map(|c| c.channel)
    }

    /// Publishes this machine's stream audio: `window_title`'s process when it names a window
    /// that isn't Pulse's, else everything but Pulse. Replaces a stream already publishing.
    pub async fn start_publishing(&self, window_title: Option<String>) -> Result<(), String> {
        let mut st = self.state.lock().await;
        let (outlet, counts) = Self::publish_locked(&mut st).await?;
        let title = window_title.clone();
        let (plan, rescan) = tokio::task::spawn_blocking(move || plan_now(title.as_deref()))
            .await
            .map_err(|e| e.to_string())?;
        counts.captures.store(plan.len(), Ordering::Relaxed);
        let engine = Arc::new(Engine::start(plan, move |c| outlet.send(c)));
        let p = st.publishing.as_mut().expect("published above");
        p.engine = Some(engine.clone());
        p.ticker = tokio::spawn(run_ticker(counts, Some(engine), window_title, rescan));
        Ok(())
    }

    /// Publishes the stream-audio track with no capture behind it and hands back its input.
    /// For the integration tests (CI can't capture), which feed it frames of their own.
    #[doc(hidden)]
    pub async fn start_publishing_raw(&self) -> Result<Outlet, String> {
        let mut st = self.state.lock().await;
        let (outlet, counts) = Self::publish_locked(&mut st).await?;
        let p = st.publishing.as_mut().expect("published above");
        p.ticker = tokio::spawn(run_ticker(counts, None, None, false));
        Ok(outlet)
    }

    async fn publish_locked(st: &mut State) -> Result<(Outlet, Arc<OutCounts>), String> {
        Self::stop_publishing_locked(st).await;
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
        st.publishing = Some(Publishing {
            track,
            sid: publication.sid(),
            ticker: tokio::spawn(async {}),
            engine: None,
        });
        let outlet = Outlet {
            source,
            rt: tokio::runtime::Handle::current(),
            counts: counts.clone(),
        };
        Ok((outlet, counts))
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
        p.ticker.abort();
        if let Some(engine) = p.engine {
            // joining the capture threads blocks
            let _ = tokio::task::spawn_blocking(move || match Arc::try_unwrap(engine) {
                Ok(e) => e.stop(),
                Err(shared) => drop(shared), // a rescan still has it: the last one stops it
            })
            .await;
        }
        if let Some(c) = &st.conn {
            let _ = c.room.local_participant().unpublish_track(&p.sid).await;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::mixer::{SourceKind, mixer, percent_to_gain};

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
