//! The stream liveness check against the dev LiveKit (`just voice-it`; `just voice-it-alt` moves
//! it): a streamer's `#v` connection that LiveKit has keeps the stream, and once the client dies
//! (its connection dropped without a goodbye) and LiveKit closes the empty room, the stream ends
//! after the grace. The check's own rules are covered against a fake RoomService in streams.rs;
//! this pins the real API's answers.

mod common;

use std::time::{Duration, Instant};

use common::*;
use livekit_api::access_token::{AccessToken, VideoGrants};
use livekit_api::services::room::RoomClient;
use pulse_protocol::rest::Channel;
use pulse_server::streams::liveness::{LiveKitRooms, StreamRooms, check};
use serde_json::json;

/// docker/livekit.yaml's dev key pair.
const DEV_KEY: &str = "devkey";
const DEV_SECRET: &str = "pulse-dev-secret-0123456789abcdefghij";
const GRACE: Duration = Duration::from_secs(60);

/// Wait (up to `limit`) until LiveKit does (or doesn't) list `identity` in `room`.
async fn until_listed(
    rooms: &LiveKitRooms,
    room: &str,
    identity: &str,
    listed: bool,
    limit: Duration,
) {
    let start = Instant::now();
    loop {
        let now = rooms
            .connected(room)
            .await
            .expect("dev LiveKit up? (just voice-it)");
        if now.contains(identity) == listed {
            println!("{identity} listed={listed} after {:?}", start.elapsed());
            return;
        }
        assert!(
            start.elapsed() < limit,
            "{identity} still listed={} after {limit:?}",
            !listed
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::test]
#[ignore]
async fn a_stream_lives_while_livekit_has_its_video_connection_and_ends_after_the_client_dies() {
    let port = std::env::var("PULSE_DEV_LIVEKIT_PORT").unwrap_or_else(|_| "7880".into());
    let mut cfg = test_config();
    cfg.livekit_api_url = format!("http://127.0.0.1:{port}");
    cfg.livekit_key = DEV_KEY.into();
    cfg.livekit_secret = DEV_SECRET.into();
    let app = spawn_with(cfg).await;
    let (a_id, a) = register(&app, "alex").await;
    let server = create_server(&app, &a, "Main").await;
    let chans: Vec<Channel> = get_json(&app, &a, &format!("/servers/{}/channels", server.id)).await;
    let lounge = chans
        .iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    app.voice.join(lounge.id, a_id, "PA_voice"); // what the voice join webhook does

    // alex's `#v` connection (signalling only: LiveKit lists a participant from its join)
    let room = format!("{}~s", lounge.id);
    let identity = format!("{a_id}#v");
    let jwt = AccessToken::with_api_key(DEV_KEY, DEV_SECRET)
        .with_identity(&identity)
        .with_grants(VideoGrants {
            room_join: true,
            room: room.clone(),
            ..Default::default()
        })
        .to_jwt()
        .unwrap();
    let url = format!(
        "ws://127.0.0.1:{port}/rtc?access_token={jwt}&protocol=16&auto_subscribe=0&sdk=rust"
    );
    let (ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("dev LiveKit up? (just voice-it)");
    let rooms = LiveKitRooms::new(&app.cfg);
    until_listed(&rooms, &room, &identity, true, Duration::from_secs(10)).await;

    let live = post_json(
        &app,
        &a,
        &format!("/streams/{}/live", lounge.id),
        json!({"settings": {"height": 1080, "fps": 60, "audio": true}}),
    )
    .await;
    assert_eq!(live.status(), 200);
    let s = app.state();
    let t0 = Instant::now();
    for n in 0..3 {
        let c = check(&s, &rooms, t0 + GRACE * n, GRACE).await;
        assert_eq!(
            (c.ended, c.answered, c.unknown),
            (0, vec![lounge.id], vec![])
        );
    }
    assert!(app.streams.get(lounge.id, a_id).is_some());

    // the client dies: the connection drops with no leave
    drop(ws);
    until_listed(&rooms, &room, &identity, false, Duration::from_secs(60)).await;
    // and LiveKit closes the now-empty room (after its departure timeout): a room it doesn't
    // have must read as nobody there, not as an error
    let admin = RoomClient::with_api_key(&app.cfg.livekit_api_url, DEV_KEY, DEV_SECRET);
    let start = Instant::now();
    while admin
        .list_rooms(vec![room.clone()])
        .await
        .unwrap()
        .iter()
        .any(|r| r.name == room)
    {
        assert!(
            start.elapsed() < Duration::from_secs(90),
            "room never closed"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    println!("room closed after {:?}", start.elapsed());
    // LiveKit's own answer, not just ours: an empty list, no error (a LiveKit upgrade that
    // changes this needs a look at liveness.rs and voice/restore.rs)
    assert!(admin.list_participants(&room).await.unwrap().is_empty());
    assert!(rooms.connected(&room).await.unwrap().is_empty());
    let t1 = t0 + GRACE * 3;
    let c = check(&s, &rooms, t1, GRACE).await;
    assert_eq!((c.ended, c.unknown.len()), (0, 0), "{c:?}");
    let c = check(&s, &rooms, t1 + GRACE, GRACE).await;
    assert_eq!((c.ended, c.unknown.len()), (1, 0), "{c:?}");
    assert!(app.streams.get(lounge.id, a_id).is_none());
}
