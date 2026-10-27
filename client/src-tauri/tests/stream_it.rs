//! Stream-room isolation against the dev LiveKit (`just dev-livekit`). Run: `just voice-it`.
//!
//! The stream room (`<channel>~s`) must never carry voice, and each ticket may publish only its own source.

use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use livekit::options::TrackPublishOptions;
use livekit::prelude::*;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use pulse_client::api::Api;
use pulse_client::stream::audio::{MixerSource, Outlet, StreamAudio};
use pulse_client::voice::devices::AudioConfig;
use pulse_client::voice::{AudioMode, VoiceManager};
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use pulse_protocol::rest::StreamRole;
use pulse_server::testing;

mod common;
use common::livekit_turn;

const DEV_SECRET: &str = "pulse-dev-secret-0123456789abcdefghij";

/// The dev LiveKit's address: `PULSE_DEV_LIVEKIT_PORT` when set (`just voice-it-alt`, beside another
/// checkout's LiveKit), else 7880 as in docker/livekit.yaml.
fn dev_livekit(scheme: &str) -> String {
    let port = std::env::var("PULSE_DEV_LIVEKIT_PORT").unwrap_or_else(|_| "7880".into());
    format!("{scheme}://127.0.0.1:{port}")
}

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
                &dev_livekit("ws"),
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
    uid: UserId,
    token: String,
    server: ServerId,
    channel: ChannelId,
    app: testing::TestApp,
}

/// A server against the dev LiveKit, with alex registered and counted as in the voice channel.
async fn fixture() -> Fixture {
    let mut cfg = testing::test_config();
    cfg.livekit_url = dev_livekit("ws");
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
        uid,
        token,
        server: s.id,
        channel: lounge.id,
        app,
    }
}

/// Another user, a member of the server and counted as in the voice channel.
async fn member(f: &Fixture, name: &str) -> (UserId, String) {
    let (uid, token) = testing::register(&f.app, name).await;
    let joined = testing::post_json(
        &f.app,
        &token,
        &format!("/servers/{}/join", f.server),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(joined.status(), 204, "{name} joins the server");
    f.app.voice.join(f.channel, uid, "PA_test");
    (uid, token)
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
    let _turn = livekit_turn().await;
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
    let _turn = livekit_turn().await;
    let f = fixture().await;
    let (room, _events) = connect(&f, StreamRole::V, RoomOptions::default()).await;
    assert!(!audio_publish_accepted(&room, TrackSource::Microphone).await);
    assert!(!audio_publish_accepted(&room, TrackSource::ScreenshareAudio).await);
    let _ = room.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_ticket_publishes_screenshare_audio() {
    let _turn = livekit_turn().await;
    let f = fixture().await;
    let (room, _events) = connect(&f, StreamRole::A, RoomOptions::default()).await;
    assert!(audio_publish_accepted(&room, TrackSource::ScreenshareAudio).await);
    let _ = room.close().await;
}

/// `voice`'s mixer, as the app hands it to `StreamAudio`.
fn mixer_of(voice: &Arc<VoiceManager>) -> MixerSource {
    let voice = voice.clone();
    Arc::new(move || {
        let voice = voice.clone();
        Box::pin(async move { voice.mixer_input().await })
    })
}

fn no_mixer() -> MixerSource {
    Arc::new(|| Box::pin(async { None }))
}

/// A call joined with no devices (`AudioMode::Null`): its mixer is what stream audio plays into.
async fn voice_call(f: &Fixture, token: &str) -> Arc<VoiceManager> {
    let vm = Arc::new(VoiceManager::new(Arc::new(|_| {})));
    vm.join(
        &f.api,
        token,
        f.channel,
        AudioConfig::default(),
        AudioMode::Null(48_000),
    )
    .await
    .expect("voice join");
    vm
}

/// Feeds `outlet` 10 ms stereo chunks (a test-only square wave) in real time until dropped.
struct Feeder(Arc<AtomicBool>, Option<std::thread::JoinHandle<()>>);

impl Feeder {
    fn start(outlet: Outlet) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let t = std::thread::spawn(move || {
            let chunk: Vec<i16> = (0..960)
                .map(|i| if (i / 2) % 48 < 24 { 6000 } else { -6000 })
                .collect();
            let start = std::time::Instant::now();
            let mut sent = 0u32;
            while !stop2.load(Ordering::Relaxed) {
                outlet.send(chunk.clone());
                sent += 1;
                let due = start + Duration::from_millis(10 * sent as u64);
                if let Some(wait) = due.checked_duration_since(std::time::Instant::now()) {
                    std::thread::sleep(wait);
                }
            }
        });
        Self(stop, Some(t))
    }
}

impl Drop for Feeder {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
        if let Some(t) = self.1.take() {
            let _ = t.join();
        }
    }
}

async fn wait_for<F, Fut>(what: &str, secs: u64, mut cond: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let end = tokio::time::Instant::now() + Duration::from_secs(secs);
    while !cond().await {
        assert!(
            tokio::time::Instant::now() < end,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// alex publishes stream audio (synthetic frames through the real `NativeAudioSource` path:
/// CI can't capture). sam watches before it is published, kim after; both hear it through
/// their voice mixer as one source that never lights the speaking ring. Unwatching removes it.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn published_stream_audio_reaches_a_watcher() {
    let _turn = livekit_turn().await;
    let f = fixture().await;
    let (_, sam_token) = member(&f, "sam").await;
    let (_, kim_token) = member(&f, "kim").await;

    // sam: in the call, watching alex before alex publishes (subscribes on TrackPublished)
    let sam_vm = voice_call(&f, &sam_token).await;
    let sam = StreamAudio::default();
    sam.ensure_connected(&f.api, &sam_token, f.channel, mixer_of(&sam_vm))
        .await
        .unwrap();
    sam.watch(f.uid, true).await.unwrap();

    let alex = StreamAudio::default();
    alex.ensure_connected(&f.api, &f.token, f.channel, no_mixer())
        .await
        .unwrap();
    let feeder = Feeder::start(alex.start_publishing_raw().await.unwrap());

    wait_for("sam to hear alex's stream audio", 20, || async {
        sam.heard_frames().await > 50
    })
    .await;
    assert_eq!(
        sam_vm.mixer_sources().await,
        1,
        "one stream source, no voice"
    );
    let pushed = sam_vm.mixer_pushed().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        sam_vm.mixer_pushed().await > pushed + 48_000 / 4,
        "stream audio keeps flowing into the mixer"
    );
    assert!(
        sam_vm.mixer_levels().await.is_empty(),
        "stream audio never lights the speaking ring"
    );

    sam.watch(f.uid, false).await.unwrap();
    wait_for("unwatching to take the source out", 5, || async {
        sam_vm.mixer_sources().await == 0
    })
    .await;
    sam.leave().await;
    sam_vm.leave().await;

    // kim: watches once it is already published (subscribes straight away)
    let kim_vm = voice_call(&f, &kim_token).await;
    let kim = StreamAudio::default();
    kim.ensure_connected(&f.api, &kim_token, f.channel, mixer_of(&kim_vm))
        .await
        .unwrap();
    // the room state (alex's track) arrives just after connecting
    wait_for("kim to hear alex's stream audio", 20, || async {
        kim.watch(f.uid, true).await.unwrap();
        kim.heard_frames().await > 50
    })
    .await;
    assert_eq!(kim_vm.mixer_sources().await, 1);

    drop(feeder);
    kim.leave().await;
    kim_vm.leave().await;
    alex.leave().await;
}

/// Leaving voice leaves the stream room too: the `#a` participant is gone for everyone in it.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn leave_voice_closes_the_stream_connection() {
    let _turn = livekit_turn().await;
    let f = fixture().await;
    let (closed, on_closed) = closed_log();
    let alex = StreamAudio::new(on_closed);
    alex.ensure_connected(&f.api, &f.token, f.channel, no_mixer())
        .await
        .unwrap();
    let _outlet = alex.start_publishing_raw().await.unwrap();
    assert_eq!(alex.connected_channel().await, Some(f.channel));

    // an observer in the stream room (alex's own `#v` connection) sees `#a` come and go
    let (observer, mut events) = connect(&f, StreamRole::V, RoomOptions::default()).await;
    let a_identity = format!("{}#a", f.uid);
    wait_for("the observer to see #a", 10, || async {
        observer
            .remote_participants()
            .keys()
            .any(|k| k.as_str() == a_identity)
    })
    .await;

    alex.leave().await;
    assert_eq!(alex.connected_channel().await, None);
    let gone = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(ev) = events.recv().await {
            if let RoomEvent::ParticipantDisconnected(p) = ev
                && p.identity().as_str() == a_identity
            {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(gone, "#a never left the stream room");
    assert!(
        alex.start_publishing_raw().await.is_err(),
        "nothing publishes after leaving"
    );
    assert!(
        closed.lock().unwrap().is_empty(),
        "our own leave is not a room closed under us"
    );
    let _ = observer.close().await;
}

/// The channels `StreamAudio`'s on-closed hook reported.
fn closed_log() -> (
    Arc<std::sync::Mutex<Vec<ChannelId>>>,
    pulse_client::stream::audio::OnClosed,
) {
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let l2 = log.clone();
    (log, Arc::new(move |c| l2.lock().unwrap().push(c)))
}

/// LiveKit closing the room under `#a` (here: the participant is removed) stops publishing
/// and drops the connection, instead of capturing into a dead room until the next stop, and
/// tells the UI (which connects again).
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_room_closed_by_livekit_stops_publishing() {
    let _turn = livekit_turn().await;
    let f = fixture().await;
    let (closed, on_closed) = closed_log();
    let alex = StreamAudio::new(on_closed);
    alex.ensure_connected(&f.api, &f.token, f.channel, no_mixer())
        .await
        .unwrap();
    let feeder = Feeder::start(alex.start_publishing_raw().await.unwrap());
    assert!(alex.publishing().await);

    let (room, identity) = (format!("{}~s", f.channel), format!("{}#a", f.uid));
    let st = tokio::task::spawn_blocking(move || {
        Command::new("docker")
            .args([
                "run",
                "--rm",
                "--network",
                "host",
                "livekit/livekit-cli:v2.18",
                "room",
                "participants",
                "remove",
                "--url",
                &dev_livekit("http"),
                "--api-key",
                "devkey",
                "--api-secret",
                DEV_SECRET,
                "--room",
                &room,
                &identity,
            ])
            .output()
            .expect("docker")
    })
    .await
    .unwrap();
    assert!(
        st.status.success(),
        "lk remove failed: {}",
        String::from_utf8_lossy(&st.stderr)
    );

    wait_for("publishing to stop after the room closed", 10, || async {
        !alex.publishing().await && alex.connected_channel().await.is_none()
    })
    .await;
    wait_for("the UI to be told", 5, || async {
        !closed.lock().unwrap().is_empty()
    })
    .await;
    assert_eq!(*closed.lock().unwrap(), vec![f.channel]);
    drop(feeder);
}
