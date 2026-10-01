//! Voice: our own capture/playback pipeline over LiveKit (rules: spikes/voice/FINDINGS.md).

pub mod controls;
pub mod devices;
pub mod meter;
pub mod mixer;
pub mod rx;

use std::collections::HashMap;
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
use devices::{AudioConfig, AudioIo, Shared};
use mixer::{Mixer, percent_to_gain};
use rx::RxTasks;

/// Max audio buffered per peer before the oldest is dropped (bounds latency).
const PEER_BUFFER_MS: u32 = 200;
const MIXER_RATE_FOR_CAP: u32 = 48_000;

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
    mic_tx: mpsc::UnboundedSender<Vec<i16>>,
    real: bool,
    rx: Arc<Mutex<RxTasks>>,
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
    session: tokio::sync::Mutex<Option<Session>>,
    /// Survives across sessions, like Discord (you stay muted when switching channels).
    controls: Mutex<Controls>,
    /// Per-user volume (0..=200 %), keyed by user id.
    volumes: Arc<Mutex<HashMap<String, u16>>>,
    events: EventSink,
}

impl VoiceManager {
    pub fn new(events: EventSink) -> Self {
        Self {
            session: tokio::sync::Mutex::new(None),
            controls: Mutex::new(Controls::default()),
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
        self.leave().await;
        self.emit_state(Some(channel), Connection::Connecting);
        let vt = api.voice_token(token, channel).await?;

        let mixer = Arc::new(Mutex::new(Mixer::new(MIXER_RATE_FOR_CAP, PEER_BUFFER_MS)));
        let shared = Shared::new(&cfg, *self.controls.lock().unwrap(), mixer.clone());
        let (mic_tx, mut mic_rx) = mpsc::unbounded_channel::<Vec<i16>>();
        let io = match mode {
            AudioMode::Real => AudioIo::start(&cfg, shared.clone(), mic_tx.clone())
                .map_err(|e| VoiceError::Device(e.to_string()))?,
            AudioMode::Null(rate) => AudioIo::start_null(rate, shared.clone(), mic_tx.clone()),
        };
        let (in_rate, out_rate) = (io.input_rate(), io.output_rate());

        let (room, mut room_events) = Room::connect(&vt.url, &vt.token, RoomOptions::default())
            .await
            .map_err(|e| VoiceError::Connect(e.to_string()))?;
        let source = NativeAudioSource::new(AudioSourceOptions::default(), in_rate, 1, 100);
        let track =
            LocalAudioTrack::create_audio_track("mic", RtcAudioSource::Native(source.clone()));
        room.local_participant()
            .publish_track(
                LocalTrack::Audio(track),
                TrackPublishOptions {
                    source: TrackSource::Microphone,
                    dtx: true,
                    red: true,
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| VoiceError::Connect(e.to_string()))?;

        let mut tasks = Vec::new();
        // mic → LiveKit
        tasks.push(tokio::spawn(async move {
            while let Some(buf) = mic_rx.recv().await {
                let n = buf.len() as u32;
                let frame = AudioFrame {
                    data: buf.into(),
                    sample_rate: in_rate,
                    num_channels: 1,
                    samples_per_channel: n,
                };
                let _ = source.capture_frame(&frame).await;
            }
        }));

        // room events: subscriptions (FINDINGS rule 1), speaking, quality, connection state
        let rx = Arc::new(Mutex::new(RxTasks::default()));
        {
            let (rx, mixer, volumes, events) = (
                rx.clone(),
                mixer.clone(),
                self.volumes.clone(),
                self.events.clone(),
            );
            let controls_snapshot = shared.clone();
            tasks.push(tokio::spawn(async move {
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
                                    NativeAudioStream::new(t.rtc_track(), out_rate as i32, 1);
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
                        RoomEvent::Reconnecting => events(VoiceEvent::State {
                            channel_id: Some(channel),
                            connection: Connection::Reconnecting,
                            controls: *controls_snapshot.controls.lock().unwrap(),
                        }),
                        RoomEvent::Reconnected => events(VoiceEvent::State {
                            channel_id: Some(channel),
                            connection: Connection::Connected,
                            controls: *controls_snapshot.controls.lock().unwrap(),
                        }),
                        RoomEvent::Disconnected { .. } => {
                            events(VoiceEvent::State {
                                channel_id: None,
                                connection: Connection::Disconnected,
                                controls: *controls_snapshot.controls.lock().unwrap(),
                            });
                            break;
                        }
                        _ => {}
                    }
                }
            }));
        }

        // levels for the UI meter + device watchdog (FINDINGS rule 2)
        let io = Arc::new(Mutex::new(Some(io)));
        let cfg = Arc::new(Mutex::new(cfg));
        {
            let (shared, io, events) = (shared.clone(), io.clone(), self.events.clone());
            let (cfg, mic_tx) = (cfg.clone(), mic_tx.clone());
            let real = matches!(mode, AudioMode::Real);
            tasks.push(tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_millis(100));
                let mut restarted_at: Option<Instant> = None;
                let mut n = 0u32;
                loop {
                    tick.tick().await;
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
                            restarted_at = Some(now);
                            shared.watchdog.input_tick(now);
                            shared.watchdog.output_tick(now);
                            let mut slot = io.lock().unwrap();
                            slot.take(); // stop the old streams first
                            let cfg = cfg.lock().unwrap().clone();
                            *slot = AudioIo::start(&cfg, shared.clone(), mic_tx.clone()).ok();
                        }
                        // Still stalled after a reopen: tell the user.
                        Some(t) if now.duration_since(t) > devices::STALL_AFTER => {
                            events(VoiceEvent::DeviceStalled);
                            restarted_at = Some(now);
                        }
                        Some(_) => {}
                    }
                }
            }));
        }

        let real = matches!(mode, AudioMode::Real);
        *self.session.lock().await = Some(Session {
            channel,
            room,
            shared,
            io,
            cfg,
            mic_tx,
            real,
            rx,
            tasks,
        });
        self.emit_state(Some(channel), Connection::Connected);
        Ok(())
    }

    pub async fn leave(&self) {
        let s = self.session.lock().await.take();
        if let Some(s) = s {
            let _ = s.room.close().await;
            s.rx.lock().unwrap().clear();
            s.io.lock().unwrap().take();
            drop(s);
            self.emit_state(None, Connection::Disconnected);
        }
    }

    pub async fn current_channel(&self) -> Option<ChannelId> {
        self.session.lock().await.as_ref().map(|s| s.channel)
    }

    fn update_controls(&self, f: impl FnOnce(&mut Controls)) -> Controls {
        let mut c = self.controls.lock().unwrap();
        f(&mut c);
        *c
    }

    async fn push_controls(&self, c: Controls) {
        let channel = match self.session.lock().await.as_ref() {
            Some(s) => {
                *s.shared.controls.lock().unwrap() = c;
                Some(s.channel)
            }
            None => None,
        };
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
    }

    pub async fn toggle_mute(&self) -> Controls {
        let c = self.update_controls(Controls::toggle_mute);
        self.push_controls(c).await;
        c
    }

    pub async fn toggle_deafen(&self) -> Controls {
        let c = self.update_controls(Controls::toggle_deafen);
        self.push_controls(c).await;
        c
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
            // Hot-swap: reopen the streams, stay in the room.
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
