//! Stream-room isolation against the dev LiveKit (`just dev-livekit`). Run: `just voice-it`.
//!
//! The stream room (`<channel>~s`) must never carry voice, and each ticket may publish only its own source.

use std::process::{Command, Stdio};
use std::time::Duration;

use livekit::options::TrackPublishOptions;
use livekit::prelude::*;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use pulse_client::api::Api;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::StreamRole;
use pulse_server::testing;

const DEV_SECRET: &str = "pulse-dev-secret-0123456789abcdefghij";

/// A voice-room bot that keeps publishing the tone until the guard drops.
struct ToneBot(String);

impl ToneBot {
    fn start(room: &str) -> Self {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../spikes/voice");
        let name = format!("pulse-stream-it-bot-{}", std::process::id());
        let st = Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "--name",
                &name,
                "--network",
                "host",
                "-v",
                &format!("{root}:/w"),
                "livekit/livekit-cli:v2.18",
                "room",
                "join",
                "--url",
                "ws://127.0.0.1:7880",
                "--api-key",
                "devkey",
                "--api-secret",
                DEV_SECRET,
                "--identity",
                "bot",
                "--publish",
                "/w/tone.ogg",
                room,
            ])
            .stdout(Stdio::null())
            .output()
            .expect("docker");
        assert!(
            st.status.success(),
            "lk bot failed: {}",
            String::from_utf8_lossy(&st.stderr)
        );
        Self(name)
    }

    /// The bot is still joined and publishing (so an empty stream room means something).
    fn running(&self) -> bool {
        Command::new("docker")
            .args(["inspect", "-f", "{{.State.Running}}", &self.0])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
            .unwrap_or(false)
    }
}

impl Drop for ToneBot {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.0])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

struct Fixture {
    api: Api,
    token: String,
    channel: ChannelId,
    _app: testing::TestApp,
}

/// A server against the dev LiveKit, with alex registered and counted as in the voice channel.
async fn fixture() -> Fixture {
    let mut cfg = testing::test_config();
    cfg.livekit_url = "ws://127.0.0.1:7880".into();
    cfg.livekit_secret = DEV_SECRET.into();
    let app = testing::spawn_with(cfg).await;
    let (uid, token): (UserId, String) = testing::register(&app, "alex").await;
    let s = testing::create_server(&app, &token, "Main").await;
    let api = Api::new(&format!("http://{}", app.addr));
    let lounge = api
        .channels(&token, s.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    // a stream ticket is for people in the voice channel
    app.voice.join(lounge.id, uid, "PA_test");
    Fixture {
        api,
        token,
        channel: lounge.id,
        _app: app,
    }
}

async fn connect(f: &Fixture, role: StreamRole, opts: RoomOptions) -> (Room, RoomEvents) {
    let t = f
        .api
        .stream_ticket(&f.token, f.channel, role)
        .await
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(15),
        Room::connect(&t.url, &t.token, opts),
    )
    .await
    .expect("stream room connect timed out")
    .expect("stream room connect")
}

type RoomEvents = tokio::sync::mpsc::UnboundedReceiver<RoomEvent>;

/// Publish a silent audio track with `source`; true when LiveKit accepted it.
/// A refused source comes back as an error from `publish_track` (see the report); the timeout
/// guards against it surfacing as a publish that is never confirmed.
async fn audio_publish_accepted(room: &Room, source: TrackSource) -> bool {
    let src = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 0);
    let track = LocalAudioTrack::create_audio_track("t", RtcAudioSource::Native(src));
    let opts = TrackPublishOptions {
        source,
        ..Default::default()
    };
    match tokio::time::timeout(
        Duration::from_secs(8),
        room.local_participant()
            .publish_track(LocalTrack::Audio(track), opts),
    )
    .await
    {
        Ok(Ok(_)) => true,
        Ok(Err(e)) => {
            println!("publish {source:?} refused: {e}");
            false
        }
        Err(_) => {
            println!("publish {source:?} never confirmed (timed out)");
            false
        }
    }
}

/// The spec's most important isolation test: voice audio never reaches the stream room.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn stream_room_never_sees_voice_tracks() {
    let f = fixture().await;
    let bot = tokio::task::spawn_blocking({
        let room = f.channel.to_string();
        move || ToneBot::start(&room)
    })
    .await
    .unwrap();
    // Positive control: a voice-room listener must actually receive the bot's track first, or an
    // empty stream room below would prove nothing (the bot joins and publishes asynchronously).
    let vt = f.api.voice_token(&f.token, f.channel).await.unwrap();
    let (voice_room, mut voice_events) = Room::connect(&vt.url, &vt.token, RoomOptions::default())
        .await
        .expect("voice room connect");
    let heard_bot = tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(ev) = voice_events.recv().await {
            if let RoomEvent::TrackSubscribed { participant, .. } = ev
                && participant.identity().as_str() == "bot"
            {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(heard_bot, "the bot never published into the voice room");

    let mut opts = RoomOptions::default();
    opts.auto_subscribe = true;
    let (room, mut events) = connect(&f, StreamRole::A, opts).await;
    // give the bot time to be publishing into the voice room
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while let Ok(Some(ev)) = tokio::time::timeout_at(deadline, events.recv()).await {
        assert!(
            !matches!(
                ev,
                RoomEvent::ParticipantConnected(_)
                    | RoomEvent::TrackPublished { .. }
                    | RoomEvent::TrackSubscribed { .. }
            ),
            "stream room saw voice traffic: {ev:?}"
        );
    }
    assert!(room.remote_participants().is_empty());
    assert!(bot.running(), "the voice bot exited early");
    let _ = room.close().await;
    let _ = voice_room.close().await;
    drop(bot);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn v_ticket_cannot_publish_audio() {
    let f = fixture().await;
    let (room, _events) = connect(&f, StreamRole::V, RoomOptions::default()).await;
    assert!(!audio_publish_accepted(&room, TrackSource::Microphone).await);
    assert!(!audio_publish_accepted(&room, TrackSource::ScreenshareAudio).await);
    let _ = room.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_ticket_publishes_screenshare_audio() {
    let f = fixture().await;
    let (room, _events) = connect(&f, StreamRole::A, RoomOptions::default()).await;
    assert!(audio_publish_accepted(&room, TrackSource::ScreenshareAudio).await);
    let _ = room.close().await;
}
