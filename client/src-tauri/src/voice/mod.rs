//! Voice: our own capture/playback pipeline over LiveKit (rules: spikes/voice/FINDINGS.md).

pub mod controls;
pub mod devices;
pub mod meter;
pub mod mictest;
pub mod mixer;
pub mod rx;

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
use devices::{AudioConfig, AudioIo, INTERNAL_RATE, MicChunk, Shared, resample};
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
}

pub type EventSink = Arc<dyn Fn(VoiceEvent) + Send + Sync>;

struct Session {
    channel: ChannelId,
    room: Room,
    shared: Arc<Shared>,
    io: Arc<Mutex<Option<AudioIo>>>,
    cfg: Arc<Mutex<AudioConfig>>,
    mic_tx: mpsc::UnboundedSender<MicChunk>,
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
        let _op = self.op.lock().await;
        self.leave_locked().await;
        self.emit_state(Some(channel), Connection::Connecting);
        match self.connect(api, token, channel, cfg, mode).await {
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
        let io = match mode {
            AudioMode::Real => AudioIo::start(&cfg, shared.clone(), mic_tx.clone())
                .map_err(|e| VoiceError::Device(e.to_string()))?,
            AudioMode::Null(rate) => AudioIo::start_null(rate, shared.clone(), mic_tx.clone()),
        };

        let (room, mut room_events) = Room::connect(&vt.url, &vt.token, RoomOptions::default())
            .await
            .map_err(|e| VoiceError::Connect(e.to_string()))?;
        // Fixed 48 kHz towards LiveKit, whatever the device does.
        let source = NativeAudioSource::new(AudioSourceOptions::default(), INTERNAL_RATE, 1, 100);
        let track =
            LocalAudioTrack::create_audio_track("mic", RtcAudioSource::Native(source.clone()));
        let published = room
            .local_participant()
            .publish_track(
                LocalTrack::Audio(track),
                TrackPublishOptions {
                    source: TrackSource::Microphone,
                    dtx: true,
                    red: true,
                    ..Default::default()
                },
            )
            .await;
        if let Err(e) = published {
            let _ = room.close().await; // dropping a Room does not disconnect it
            return Err(VoiceError::Connect(e.to_string()));
        }

        let mut tasks = Vec::new();
        // mic → LiveKit (resampled to 48 kHz)
        tasks.push(tokio::spawn(async move {
            while let Some((rate, buf)) = mic_rx.recv().await {
                let buf = resample(&buf, rate, INTERNAL_RATE);
                let n = buf.len() as u32;
                let frame = AudioFrame {
                    data: buf.into(),
                    sample_rate: INTERNAL_RATE,
                    num_channels: 1,
                    samples_per_channel: n,
                };
                let _ = source.capture_frame(&frame).await;
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
                        RoomEvent::ActiveSpeakersChanged { speakers } => {
                            events(VoiceEvent::Speaking {
                                user_ids: speakers
                                    .iter()
                                    .map(|p| p.identity().to_string())
                                    .collect(),
                            });
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
                            events(state(Connection::Reconnecting, Some(channel)))
                        }
                        RoomEvent::Reconnected => {
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

        // levels for the UI meter + device watchdog (FINDINGS rule 2)
        let cfg = Arc::new(Mutex::new(cfg));
        let real = matches!(mode, AudioMode::Real);
        {
            let (shared, io, events) = (shared.clone(), io.clone(), self.events.clone());
            let (cfg, mic_tx, alive) = (cfg.clone(), mic_tx.clone(), alive.clone());
            tasks.push(tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_millis(100));
                let mut restarted_at: Option<Instant> = None;
                let mut n = 0u32;
                loop {
                    tick.tick().await;
                    if !alive.load(Ordering::SeqCst) {
                        break;
                    }
                    let (mic, speaker) = shared.take_levels();
                    events(VoiceEvent::Levels { mic, speaker });
                    n += 1;
                    if !real || !n.is_multiple_of(5) {
                        continue;
                    }
                    let now = Instant::now();
                    if !shared.watchdog.stalled(now) {
                        restarted_at = None;
                        continue;
                    }
                    match restarted_at {
                        // First stall: reopen the devices once.
                        None => {
                            tracing::warn!("audio device stalled; reopening");
                            restarted_at = Some(now);
                            shared.watchdog.input_tick(now);
                            shared.watchdog.output_tick(now);
                            let cfg = cfg.lock().unwrap().clone();
                            let mut slot = io.lock().unwrap();
                            slot.take(); // stop the old streams first
                            *slot = AudioIo::start(&cfg, shared.clone(), mic_tx.clone()).ok();
                        }
                        // Still stalled after a reopen: tell the user.
                        Some(t) if now.duration_since(t) > devices::STALL_AFTER => {
                            tracing::error!("audio device still stalled after reopening");
                            events(VoiceEvent::DeviceStalled);
                            restarted_at = Some(now);
                        }
                        Some(_) => {}
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
            mic_tx,
            real,
            rx,
            alive,
            tasks,
        })
    }

    pub async fn leave(&self) {
        let _op = self.op.lock().await;
        self.leave_locked().await;
    }

    async fn leave_locked(&self) {
        let s = self.session.lock().await.take();
        if let Some(s) = s {
            let _ = s.room.close().await;
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
            let mut slot = s.io.lock().unwrap();
            slot.take();
            *slot = Some(
                AudioIo::start(&cfg, s.shared.clone(), s.mic_tx.clone())
                    .map_err(|e| VoiceError::Device(e.to_string()))?,
            );
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
