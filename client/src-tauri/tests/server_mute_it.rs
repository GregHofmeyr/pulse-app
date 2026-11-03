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

fn in_call(app: &testing::TestApp, channel: ChannelId, user: UserId) {
    app.voice.join(channel, user, "PA_it");
}
