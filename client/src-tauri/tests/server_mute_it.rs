//! Server mute against the dev LiveKit (#140): taking publishing away drops the mic track at
//! LiveKit, giving it back has the client publish its mic again, and a join while server muted
//! connects without publishing. Needs the dev LiveKit (`just dev-livekit`); run with
//! `cargo test -p pulse-client --test server_mute_it -- --ignored`.

use std::time::Duration;

use livekit_api::services::room::RoomClient;
use pulse_client::api::Api;
use pulse_client::voice::devices::AudioConfig;
use pulse_client::voice::{AudioMode, VoiceManager};
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::SetServerVoiceRequest;
use pulse_server::testing;

mod common;
use common::livekit_turn;

const DEV_SECRET: &str = "pulse-dev-secret-0123456789abcdefghij";

fn dev_livekit(scheme: &str) -> String {
    let port = std::env::var("PULSE_DEV_LIVEKIT_PORT").unwrap_or_else(|_| "7880".into());
    format!("{scheme}://127.0.0.1:{port}")
}

/// What LiveKit says about `identity` in `room`: (may publish, tracks published).
async fn at_livekit(lk: &RoomClient, room: &str, identity: &str) -> Option<(bool, usize)> {
    let ps = lk.list_participants(room).await.ok()?;
    let p = ps.into_iter().find(|p| p.identity == identity)?;
    Some((
        p.permission.as_ref().is_none_or(|p| p.can_publish),
        p.tracks.len(),
    ))
}

/// Polls LiveKit until `identity` is as `want`, for up to 15 s; the last state seen.
async fn wait_for(
    lk: &RoomClient,
    room: &str,
    identity: &str,
    want: (bool, usize),
) -> Option<(bool, usize)> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let now = at_livekit(lk, room, identity).await;
        if now == Some(want) || tokio::time::Instant::now() >= deadline {
            return now;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn join(vm: &VoiceManager, api: &Api, token: &str, channel: ChannelId) {
    vm.join(
        api,
        token,
        channel,
        AudioConfig::default(),
        AudioMode::Null(48_000),
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn server_mute_drops_the_mic_at_livekit_and_undoing_it_publishes_again() {
    let _turn = livekit_turn().await;
    let mut cfg = testing::test_config();
    cfg.livekit_url = dev_livekit("ws");
    cfg.livekit_api_url = dev_livekit("http");
    cfg.livekit_secret = DEV_SECRET.into();
    let app = testing::spawn_with(cfg).await;
    let (alex_id, alex) = testing::register(&app, "alex").await;
    let (_, sam) = testing::register(&app, "sam").await;
    let api = Api::new(&format!("http://{}", app.addr));
    let s = testing::create_server(&app, &alex, "Main").await;
    api.join_server(&sam, s.id).await.unwrap();
    let lounge = api
        .channels(&alex, s.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    let (room, me) = (lounge.id.to_string(), alex_id.to_string());
    let lk = RoomClient::with_api_key(&dev_livekit("http"), "devkey", DEV_SECRET);
    let set = |mute: bool| {
        let (api, sam) = (api.clone(), sam.clone());
        async move {
            let req = SetServerVoiceRequest {
                mute: Some(mute),
                deafen: None,
            };
            api.set_server_voice(&sam, s.id, alex_id, &req)
                .await
                .unwrap()
        }
    };

    let vm = VoiceManager::new(std::sync::Arc::new(|_| {}));
    join(&vm, &api, &alex, lounge.id).await;
    // the dev LiveKit's webhooks go to the dev server, not this one: place alex in the call
    in_call(&app, lounge.id, alex_id);
    assert_eq!(wait_for(&lk, &room, &me, (true, 1)).await, Some((true, 1)));

    set(true).await;
    assert_eq!(
        wait_for(&lk, &room, &me, (false, 0)).await,
        Some((false, 0)),
        "server muted: LiveKit drops the mic track"
    );
    set(false).await;
    assert_eq!(
        wait_for(&lk, &room, &me, (true, 1)).await,
        Some((true, 1)),
        "undone: the client publishes its mic again"
    );

    // a join while server muted connects without publishing, and publishes once undone
    set(true).await;
    vm.leave().await;
    join(&vm, &api, &alex, lounge.id).await;
    assert!(vm.in_voice(), "the call is up");
    assert_eq!(
        wait_for(&lk, &room, &me, (false, 0)).await,
        Some((false, 0))
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(vm.in_voice(), "not ended as a failed publish");
    set(false).await;
    assert_eq!(wait_for(&lk, &room, &me, (true, 1)).await, Some((true, 1)));
    vm.leave().await;
}

/// A dev-LiveKit world for the deafen tests: the server, alex and sam in Main, Lounge.
struct World {
    app: testing::TestApp,
    api: Api,
    server: pulse_protocol::ids::ServerId,
    lounge: ChannelId,
    alex: (UserId, String),
    sam: (UserId, String),
}

async fn world() -> World {
    let mut cfg = testing::test_config();
    cfg.livekit_url = dev_livekit("ws");
    cfg.livekit_api_url = dev_livekit("http");
    cfg.livekit_secret = DEV_SECRET.into();
    let app = testing::spawn_with(cfg).await;
    let alex = testing::register(&app, "alex").await;
    let sam = testing::register(&app, "sam").await;
    let api = Api::new(&format!("http://{}", app.addr));
    let s = testing::create_server(&app, &alex.1, "Main").await;
    api.join_server(&sam.1, s.id).await.unwrap();
    let lounge = api
        .channels(&alex.1, s.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap()
        .id;
    World {
        app,
        api,
        server: s.id,
        lounge,
        alex,
        sam,
    }
}

impl World {
    /// sam server-deafens (or undeafens) alex.
    async fn deafen_alex(&self, deafen: bool) {
        let req = SetServerVoiceRequest {
            mute: None,
            deafen: Some(deafen),
        };
        self.api
            .set_server_voice(&self.sam.1, self.server, self.alex.0, &req)
            .await
            .unwrap();
    }

    /// sam, as a bare SDK client, publishing a tone on his mic until the room is dropped.
    async fn sam_talks(&self) -> (livekit::prelude::Room, tokio::task::JoinHandle<()>) {
        use livekit::options::TrackPublishOptions;
        use livekit::prelude::*;
        use livekit::webrtc::audio_frame::AudioFrame;
        use livekit::webrtc::audio_source::native::NativeAudioSource;
        use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};

        let vt = self
            .api
            .voice_token(&self.sam.1, self.lounge)
            .await
            .unwrap();
        let mut opts = RoomOptions::default();
        opts.auto_subscribe = false;
        let (room, _events) = Room::connect(&vt.url, &vt.token, opts).await.unwrap();
        let src = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 100);
        let track = LocalAudioTrack::create_audio_track("mic", RtcAudioSource::Native(src.clone()));
        room.local_participant()
            .publish_track(
                LocalTrack::Audio(track),
                TrackPublishOptions {
                    source: TrackSource::Microphone,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let feed = tokio::spawn(async move {
            let tone: Vec<i16> = (0..480)
                .map(|i| ((i as f32 * 0.13).sin() * 8000.0) as i16)
                .collect();
            loop {
                let frame = AudioFrame {
                    data: tone.clone().into(),
                    sample_rate: 48_000,
                    num_channels: 1,
                    samples_per_channel: 480,
                };
                if src.capture_frame(&frame).await.is_err() {
                    return;
                }
            }
        });
        (room, feed)
    }
}

/// Polls `f` for up to `secs` seconds.
async fn eventually<F: std::future::Future<Output = bool>>(secs: u64, f: impl Fn() -> F) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        if f().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// #145, what LiveKit v1.13 itself does with `can_subscribe` (asked straight at LiveKit, not
/// through Pulse): taking it away stops the participant's audio; giving it back starts it again
/// on its own (LiveKit's `SetPermission` revokes every subscription, then `ReconcileAll` on the
/// grant). Judged by sound actually arriving and the SDK's subscribe events, not the SDK's
/// `track()`, which outlives a server-side revoke.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn livekit_stops_and_restarts_audio_with_can_subscribe() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use futures::StreamExt;
    use livekit::prelude::*;
    use livekit::webrtc::audio_stream::native::NativeAudioStream;

    let _turn = livekit_turn().await;
    let w = world().await;
    let (_sam_room, feed) = w.sam_talks().await;
    // alex: a bare SDK client with LiveKit's default auto-subscribe
    let vt = w.api.voice_token(&w.alex.1, w.lounge).await.unwrap();
    let (alex, mut events) = Room::connect(&vt.url, &vt.token, RoomOptions::default())
        .await
        .unwrap();
    // sound frames heard, and subscribe / unsubscribe events seen
    let (heard, subs, unsubs) = (
        Arc::new(AtomicU32::new(0)),
        Arc::new(AtomicU32::new(0)),
        Arc::new(AtomicU32::new(0)),
    );
    {
        let (heard, subs, unsubs) = (heard.clone(), subs.clone(), unsubs.clone());
        tokio::spawn(async move {
            while let Some(ev) = events.recv().await {
                match ev {
                    RoomEvent::TrackSubscribed {
                        track: RemoteTrack::Audio(t),
                        ..
                    } => {
                        subs.fetch_add(1, Ordering::SeqCst);
                        let heard = heard.clone();
                        tokio::spawn(async move {
                            let mut s = NativeAudioStream::new(t.rtc_track(), 48_000, 1);
                            while let Some(f) = s.next().await {
                                if f.data.iter().any(|x| *x != 0) {
                                    heard.fetch_add(1, Ordering::SeqCst);
                                }
                            }
                        });
                    }
                    RoomEvent::TrackUnsubscribed { .. } => {
                        unsubs.fetch_add(1, Ordering::SeqCst);
                    }
                    _ => {}
                }
            }
        });
    }
    // frames with sound in the next second
    let hearing = || async {
        let before = heard.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(1)).await;
        heard.load(Ordering::SeqCst) - before > 10
    };
    let (room, me) = (w.lounge.to_string(), w.alex.0.to_string());
    let lk = RoomClient::with_api_key(&dev_livekit("http"), "devkey", DEV_SECRET);
    let may_subscribe = |on: bool| {
        let (lk, room, me) = (&lk, room.clone(), me.clone());
        async move {
            let permission = livekit_protocol::ParticipantPermission {
                can_subscribe: on,
                can_publish: true,
                can_publish_sources: vec![livekit_protocol::TrackSource::Microphone as i32],
                ..Default::default()
            };
            lk.update_participant(
                &room,
                &me,
                livekit_api::services::room::UpdateParticipantOptions {
                    permission: Some(permission),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
    };
    assert!(eventually(15, hearing).await, "alex never heard sam");

    may_subscribe(false).await;
    let stopped = eventually(10, || async { !hearing().await }).await;
    let has_track = alex
        .remote_participants()
        .values()
        .flat_map(|p| p.track_publications().into_values().collect::<Vec<_>>())
        .any(|p| p.track().is_some());
    eprintln!(
        "#145 probe: revoked: sound stopped {stopped}; unsubscribe events {}; SDK still holds a track {has_track}",
        unsubs.load(Ordering::SeqCst)
    );
    let subs_before = subs.load(Ordering::SeqCst);
    may_subscribe(true).await;
    let back = eventually(10, hearing).await;
    eprintln!(
        "#145 probe: granted again: sound back on its own {back}; new subscribe events {}",
        subs.load(Ordering::SeqCst) - subs_before
    );
    feed.abort();
    assert!(stopped, "can_subscribe taken away: sound keeps coming");
    assert!(
        back,
        "can_subscribe given back: sound doesn't come back on its own"
    );
}

/// Whether sam's tone reaches alex's mixer within a second (sound, not just WebRTC's silence).
async fn hears(vm: &VoiceManager) -> bool {
    vm.mixer_levels().await; // a fresh window
    tokio::time::sleep(Duration::from_secs(1)).await;
    vm.mixer_levels().await.iter().any(|(_, l)| *l > 0.01)
}

/// #145 through Pulse: a server deafen stops sam's audio reaching alex at LiveKit (not just his
/// playout), and the undeafen brings it back without a rejoin, also for a join made while
/// deafened.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn server_deafen_stops_audio_at_livekit_and_undeafen_hears_again() {
    let _turn = livekit_turn().await;
    let w = world().await;
    let (_sam_room, feed) = w.sam_talks().await;
    let vm = VoiceManager::new(std::sync::Arc::new(|_| {}));
    join(&vm, &w.api, &w.alex.1, w.lounge).await;
    in_call(&w.app, w.lounge, w.alex.0);
    assert!(eventually(15, || hears(&vm)).await, "alex never heard sam");

    w.deafen_alex(true).await;
    assert!(
        eventually(10, || async { !hears(&vm).await }).await,
        "server deafened: LiveKit stops sending sam to alex"
    );
    // and stays stopped
    for _ in 0..3 {
        assert!(!hears(&vm).await, "sam came back while alex is deafened");
    }
    assert!(vm.in_voice(), "still in the call");

    w.deafen_alex(false).await;
    assert!(
        eventually(15, || hears(&vm)).await,
        "undeafened: alex hears sam again"
    );

    // joined while deafened: connected, hearing nothing until the undeafen
    w.deafen_alex(true).await;
    vm.leave().await;
    w.app.voice.leave(w.lounge, w.alex.0, "");
    join(&vm, &w.api, &w.alex.1, w.lounge).await;
    in_call(&w.app, w.lounge, w.alex.0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(vm.in_voice());
    for _ in 0..3 {
        assert!(!hears(&vm).await, "the deafened token can't subscribe");
    }
    w.deafen_alex(false).await;
    assert!(
        eventually(15, || hears(&vm)).await,
        "undeafened after joining deafened: alex hears sam"
    );
    vm.leave().await;
    feed.abort();
}

fn in_call(app: &testing::TestApp, channel: ChannelId, user: UserId) {
    app.voice.join(channel, user, "PA_it");
}
