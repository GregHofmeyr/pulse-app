//! Voice: our own capture/playback pipeline over LiveKit (rules: spikes/voice/FINDINGS.md).

pub mod controls;
pub mod denoise;
pub mod devices;
pub mod gate;
pub mod meter;
pub mod mictest;
pub mod mixer;
pub mod playout;
pub mod processor;
pub mod resampler;
pub mod rx;
pub mod speaking;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::StreamExt;
use livekit::options::TrackPublishOptions;
use livekit::prelude::*;
use livekit::webrtc::audio_frame::AudioFrame;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use pulse_protocol::ids::ChannelId;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::api::{Api, ApiError};
use controls::Controls;
use devices::{AudioConfig, AudioIo, INTERNAL_RATE, MicChunk, Shared};
use mixer::{Mixer, percent_to_gain};
use rx::RxTasks;

/// Max audio buffered per peer before the oldest is dropped (bounds latency).
const PEER_BUFFER_MS: u32 = 200;

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
    },
    DeviceStalled,
    DeviceRecovered,
}

pub type EventSink = Arc<dyn Fn(VoiceEvent) + Send + Sync>;

struct Session {
    channel: ChannelId,
    room: Room,
    shared: Arc<Shared>,
    io: Arc<Mutex<Option<AudioIo>>>,
    cfg: Arc<Mutex<AudioConfig>>,
    /// Into the mic processor; device (re)opens feed it. Dropping the last one ends the processor.
    raw_tx: processor::RawTx,
    real: bool,
    rx: Arc<Mutex<RxTasks>>,
    /// Cleared when LiveKit disconnects us for good: the session is then torn down.
    alive: Arc<AtomicBool>,
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
        let vt = api.voice_token(token, channel).await?;

        let mixer = Arc::new(Mutex::new(Mixer::new(INTERNAL_RATE, PEER_BUFFER_MS)));
        let shared = Shared::new(&cfg, self.controls.clone(), mixer.clone());
        let (mic_tx, mut mic_rx) = mpsc::unbounded_channel::<MicChunk>();
        // devices → processor (denoise, gate) → mic_tx → LiveKit
        let (raw_tx, _processor) = processor::spawn(shared.clone(), mic_tx);
        let io = match mode {
            AudioMode::Real => start_audio(&cfg, shared.clone(), raw_tx.clone()).await?,
            AudioMode::Null(rate) => AudioIo::start_null(rate, shared.clone(), raw_tx.clone()),
        };

        let (room, mut room_events) = tokio::time::timeout(
            CONNECT_TIMEOUT,
            Room::connect(&vt.url, &vt.token, RoomOptions::default()),
        )
        .await
        .map_err(|_| VoiceError::Timeout("connecting to voice"))?
        .map_err(|e| VoiceError::Connect(e.to_string()))?;
        tracing::info!("voice: room connected");
        // Fixed 48 kHz towards LiveKit, whatever the device does.
        let source = NativeAudioSource::new(AudioSourceOptions::default(), INTERNAL_RATE, 1, 100);
        let track =
            LocalAudioTrack::create_audio_track("mic", RtcAudioSource::Native(source.clone()));
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
        tracing::info!("voice: mic published");

        let mut tasks = Vec::new();
        // mic → LiveKit, resampled to 48 kHz and sent in exact 10 ms frames
        tasks.push(tokio::spawn(async move {
            let frame_len = (INTERNAL_RATE / 100) as usize;
            let mut to_internal = resampler::ToInternal::default();
            let mut ready: Vec<i16> = Vec::with_capacity(2 * frame_len);
            while let Some((rate, buf)) = mic_rx.recv().await {
                ready.extend(to_internal.process(rate, &buf));
                while ready.len() >= frame_len {
                    let frame = AudioFrame {
                        data: ready.drain(..frame_len).collect::<Vec<_>>().into(),
                        sample_rate: INTERNAL_RATE,
                        num_channels: 1,
                        samples_per_channel: frame_len as u32,
                    };
                    let _ = source.capture_frame(&frame).await;
                }
            }
        }));

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
                            let task = tokio::spawn(async move {
                                let mut stream =
                                    NativeAudioStream::new(t.rtc_track(), INTERNAL_RATE as i32, 1);
                                while let Some(f) = stream.next().await {
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
        {
            let (shared, io, events) = (shared.clone(), io.clone(), self.events.clone());
            let (cfg, raw_tx, alive) = (cfg.clone(), raw_tx.clone(), alive.clone());
            tasks.push(tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_millis(100));
                let mut stall = devices::StallPolicy::default();
                let mut speaking = speaking::SpeakingTracker::default();
                let mut n = 0u32;
                loop {
                    tick.tick().await;
                    if !alive.load(Ordering::SeqCst) {
                        break;
                    }
                    let (mic, speaker) = shared.take_levels();
                    events(VoiceEvent::Levels { mic, speaker });
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
                            tracing::warn!("audio device stalled; reopening");
                            let cfg = cfg.lock().unwrap().clone();
                            io.lock().unwrap().take(); // stop the old streams (lock released right away)
                            match start_audio(&cfg, shared.clone(), raw_tx.clone()).await {
                                Ok(new_io) if alive.load(Ordering::SeqCst) => {
                                    tracing::info!("voice: devices reopened");
                                    *io.lock().unwrap() = Some(new_io);
                                }
                                Ok(_) => {} // session ended meanwhile
                                Err(e) => {
                                    tracing::error!(error = %e, "voice: reopening devices failed")
                                }
                            }
                        }
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
            raw_tx,
            real,
            rx,
            alive,
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
        let devices_changed = {
            let old = s.cfg.lock().unwrap();
            old.input != cfg.input || old.output != cfg.output
        };
        s.shared.apply_config(&cfg);
        *s.cfg.lock().unwrap() = cfg.clone();
        if devices_changed && s.real {
            // Hot-swap: reopen the streams, stay in the room (rates may differ; the 48 kHz edge absorbs it).
            s.io.lock().unwrap().take();
            let new_io = start_audio(&cfg, s.shared.clone(), s.raw_tx.clone()).await?;
            *s.io.lock().unwrap() = Some(new_io);
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

    pub async fn rx_len(&self) -> usize {
        match self.session.lock().await.as_ref() {
            Some(s) => s.rx.lock().unwrap().len(),
            None => 0,
        }
    }
}

/// How the mic is sent. The SDK's default (`None`) is 48 kbps; Discord defaults to 64 kbps.
/// DTX makes silence ~free and RED (redundant audio) hides packet loss.
fn mic_publish_options() -> TrackPublishOptions {
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
