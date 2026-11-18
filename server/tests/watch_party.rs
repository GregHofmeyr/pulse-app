//! YouTube watch parties in voice (#384): starting one, who controls it, joining and leaving it, the host passing on
//! when they leave the call, the last viewer ending it, Ready, and deleting the channel.
mod common;

use std::time::Duration;

use base64::Engine;
use common::*;
use livekit_api::access_token::AccessToken;
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{Channel, ChannelKind, WatchParty};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc::Receiver;

const V: &str = "dQw4w9WgXcQ";
const W: &str = "9bZkp7q19f0";

struct World {
    app: TestApp,
    alex: (UserId, String),
    sam: (UserId, String),
    kim: (UserId, String),
    jo: (UserId, String),
    lounge: Channel,
    /// kim's events (in the server, not in the call).
    rx: Receiver<ServerFrame>,
}

/// alex's server (sam and kim joined, jo didn't) with alex and sam in its Lounge call.
async fn world() -> World {
    let app = spawn().await;
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let kim = register(&app, "kim").await;
    let jo = register(&app, "jo").await;
    let s = create_server(&app, &alex.1, "Main").await;
    for who in [&sam, &kim] {
        let r = post_json(&app, &who.1, &format!("/servers/{}/join", s.id), json!({})).await;
        assert_eq!(r.status(), 204);
    }
    let chans: Vec<Channel> = get_json(&app, &alex.1, &format!("/servers/{}/channels", s.id)).await;
    let lounge = chans
        .into_iter()
        .find(|c| c.kind == ChannelKind::Voice)
        .unwrap();
    app.voice.join(lounge.id, alex.0, "PA_alex");
    app.voice.join(lounge.id, sam.0, "PA_sam");
    let rx = app.hub.register(kim.0, String::new()).rx;
    World {
        app,
        alex,
        sam,
        kim,
        jo,
        lounge,
        rx,
    }
}

impl World {
    fn path(&self) -> String {
        format!("/channels/{}/watch-party", self.lounge.id)
    }

    async fn start(&self, token: &str, video: &str) -> reqwest::Response {
        self.app
            .http
            .put(self.app.url(&self.path()))
            .bearer_auth(token)
            .json(&json!({ "video_id": video, "title": "A video" }))
            .send()
            .await
            .unwrap()
    }

    async fn act(&self, token: &str, action: Value) -> u16 {
        self.app
            .http
            .post(self.app.url(&self.path()))
            .bearer_auth(token)
            .json(&action)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    async fn end(&self, token: &str) -> u16 {
        self.app
            .http
            .delete(self.app.url(&self.path()))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }
}

/// The next watch party event on `rx`, skipping everything else.
async fn next(rx: &mut Receiver<ServerFrame>) -> Event {
    loop {
        let f = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("timed out waiting for a watch party event")
            .expect("hub closed");
        if let ServerFrame::Event(
            e @ (Event::WatchPartyChanged { .. } | Event::WatchPartyEnded { .. }),
        ) = f
        {
            return e;
        }
    }
}

async fn changed(rx: &mut Receiver<ServerFrame>) -> WatchParty {
    match next(rx).await {
        Event::WatchPartyChanged {
            party,
            server_now_ms,
        } => {
            assert!(server_now_ms >= party.set_at_ms);
            party
        }
        e => panic!("unexpected {e:?}"),
    }
}

/// Nothing more from the party on `rx` for a moment.
async fn quiet(rx: &mut Receiver<ServerFrame>) {
    let wait = tokio::time::timeout(Duration::from_millis(300), next(rx)).await;
    assert!(wait.is_err(), "unexpected {:?}", wait.unwrap());
}

async fn ready_of(app: &TestApp, token: &str) -> Ready {
    use futures::SinkExt;
    use tokio_tungstenite::tungstenite::Message as Ws;
    let (mut ws, _) = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap();
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    loop {
        match next_frame_beating(&mut ws, Duration::from_secs(3)).await {
            Some(ServerFrame::Ready(r)) => return r,
            Some(_) => continue,
            None => panic!("expected Ready"),
        }
    }
}

fn signed(app: &TestApp, body: &str) -> String {
    let sum = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body.as_bytes()));
    AccessToken::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .with_sha256(&sum)
        .to_jwt()
        .unwrap()
}

async fn left(app: &TestApp, room: ChannelId, user: UserId, sid: &str) {
    let body =
        json!({"event": "participant_left", "id": "EV_test", "room": {"name": room.to_string()},
        "participant": {"identity": user.to_string(), "sid": sid}})
        .to_string();
    let r = app
        .http
        .post(app.url("/livekit/webhook"))
        .header("Authorization", signed(app, &body))
        .header("Content-Type", "application/webhook+json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn starting_one_tells_everyone_and_late_joiners_get_it_in_ready() {
    let mut w = world().await;
    let r = w.start(&w.alex.1, V).await;
    assert_eq!(r.status(), 200);
    let p: WatchParty = r.json().await.unwrap();
    assert_eq!(
        (p.host_id, p.video_id.as_str(), p.playing, p.position_ms),
        (w.alex.0, V, true, 0)
    );
    assert_eq!(p.title.as_deref(), Some("A video"));
    assert_eq!(p.viewers, [w.alex.0]);
    // kim, outside the call, hears it too (the channel list shows it)
    assert_eq!(changed(&mut w.rx).await, p);
    let ready = ready_of(&w.app, &w.kim.1).await;
    assert_eq!(ready.watch_parties, std::slice::from_ref(&p));
    // a second start by someone who may not control it is refused
    assert_eq!(w.start(&w.sam.1, W).await.status(), 409);
    quiet(&mut w.rx).await;
}

#[tokio::test]
async fn only_people_in_the_call_and_only_youtube_ids() {
    let w = world().await;
    // kim is in the server but not the call; jo isn't even in the server
    assert_eq!(w.start(&w.kim.1, V).await.status(), 403);
    assert_eq!(w.start(&w.jo.1, V).await.status(), 403);
    for bad in ["short", "dQw4w9WgXcQ&x", "../../admin", "dQw4w9WgX\"Q"] {
        assert_eq!(w.start(&w.alex.1, bad).await.status(), 400, "{bad}");
    }
    assert!(w.app.voice.watch.of(w.lounge.id).is_none());
    // nothing to control yet
    assert_eq!(w.act(&w.alex.1, json!({"action": "join"})).await, 404);
    w.start(&w.alex.1, V).await;
    assert_eq!(w.act(&w.kim.1, json!({"action": "join"})).await, 403);
    // a text channel has no watch parties
    let text = get_json::<Vec<Channel>>(
        &w.app,
        &w.alex.1,
        &format!("/servers/{}/channels", w.lounge.server_id.unwrap()),
    )
    .await
    .into_iter()
    .find(|c| c.kind == ChannelKind::Text)
    .unwrap();
    let r = w
        .app
        .http
        .put(w.app.url(&format!("/channels/{}/watch-party", text.id)))
        .bearer_auth(&w.alex.1)
        .json(&json!({ "video_id": V }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn the_host_controls_until_everyone_may() {
    let mut w = world().await;
    w.start(&w.alex.1, V).await;
    changed(&mut w.rx).await;
    let pause = json!({"action": "pause", "position_ms": 12_000});
    assert_eq!(w.act(&w.sam.1, pause.clone()).await, 403);
    assert_eq!(w.act(&w.sam.1, json!({"action": "join"})).await, 204);
    assert_eq!(changed(&mut w.rx).await.viewers, [w.alex.0, w.sam.0]);
    assert_eq!(w.act(&w.sam.1, pause.clone()).await, 403);
    assert_eq!(w.act(&w.alex.1, pause).await, 204);
    let p = changed(&mut w.rx).await;
    assert_eq!((p.playing, p.position_ms), (false, 12_000));
    assert_eq!(
        w.act(&w.sam.1, json!({"action": "everyone_controls", "on": true}))
            .await,
        403
    );
    assert_eq!(
        w.act(
            &w.alex.1,
            json!({"action": "everyone_controls", "on": true})
        )
        .await,
        204
    );
    assert!(changed(&mut w.rx).await.everyone_controls);
    assert_eq!(
        w.act(&w.sam.1, json!({"action": "seek", "position_ms": 30_000}))
            .await,
        204
    );
    let p = changed(&mut w.rx).await;
    assert_eq!((p.playing, p.position_ms), (false, 30_000));
    // the queue: anyone watching adds, the next one plays from the start
    assert_eq!(
        w.act(&w.sam.1, json!({"action": "enqueue", "video_id": W}))
            .await,
        204
    );
    assert_eq!(changed(&mut w.rx).await.queue.len(), 1);
    assert_eq!(w.act(&w.sam.1, json!({"action": "next"})).await, 204);
    let p = changed(&mut w.rx).await;
    assert_eq!(
        (p.video_id.as_str(), p.position_ms, p.playing, p.queue.len()),
        (W, 0, true, 0)
    );
    // ending it is the host's
    assert_eq!(w.end(&w.sam.1).await, 403);
    assert_eq!(w.end(&w.alex.1).await, 204);
    assert!(
        matches!(next(&mut w.rx).await, Event::WatchPartyEnded { channel_id } if channel_id == w.lounge.id)
    );
    assert_eq!(w.end(&w.alex.1).await, 404);
}

#[tokio::test]
async fn the_host_leaving_the_call_passes_it_on_and_the_last_one_out_ends_it() {
    let mut w = world().await;
    w.start(&w.alex.1, V).await;
    changed(&mut w.rx).await;
    w.act(&w.sam.1, json!({"action": "join"})).await;
    changed(&mut w.rx).await;
    // hand-off goes only to someone in the call
    assert_eq!(
        w.act(
            &w.alex.1,
            json!({"action": "hand_off", "user_id": w.kim.0.to_string()})
        )
        .await,
        400
    );
    left(&w.app, w.lounge.id, w.alex.0, "PA_alex").await;
    let p = changed(&mut w.rx).await;
    assert_eq!((p.host_id, p.viewers.clone()), (w.sam.0, vec![w.sam.0]));
    // leaving the party, staying in the call: the last viewer out ends it
    assert_eq!(w.act(&w.sam.1, json!({"action": "leave"})).await, 204);
    assert!(matches!(
        next(&mut w.rx).await,
        Event::WatchPartyEnded { .. }
    ));
    assert!(w.app.voice.watch.of(w.lounge.id).is_none());
    assert!(ready_of(&w.app, &w.kim.1).await.watch_parties.is_empty());
}

#[tokio::test]
async fn deleting_the_channel_drops_it() {
    let w = world().await;
    assert_eq!(w.start(&w.alex.1, V).await.status(), 200);
    let r = w
        .app
        .http
        .delete(w.app.url(&format!("/channels/{}", w.lounge.id)))
        .bearer_auth(&w.alex.1)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    assert!(w.app.voice.watch.of(w.lounge.id).is_none());
    assert!(ready_of(&w.app, &w.kim.1).await.watch_parties.is_empty());
}

/// The `saved_at_ms` of `channel`'s snapshot, if it has one.
async fn snapshot_at(app: &TestApp, channel: ChannelId) -> Option<i64> {
    sqlx::query_scalar("SELECT saved_at_ms FROM watch_party_snapshots WHERE channel_id = ?")
        .bind(channel.to_string())
        .fetch_optional(&app.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_restart_picks_it_back_up_with_whoever_is_still_in_the_call() {
    let mut w = world().await;
    assert_eq!(w.start(&w.alex.1, V).await.status(), 200);
    changed(&mut w.rx).await;
    assert_eq!(w.act(&w.sam.1, json!({"action": "join"})).await, 204);
    changed(&mut w.rx).await;
    let enqueue = json!({"action": "enqueue", "video_id": W, "title": "Next up"});
    assert_eq!(w.act(&w.sam.1, enqueue).await, 204);
    let before = changed(&mut w.rx).await;
    assert!(snapshot_at(&w.app, w.lounge.id).await.is_some());

    // a restart: memory is empty, and LiveKit hands back only sam (alex dropped while it was down)
    let voice = pulse_server::voice::VoiceState::default();
    voice.join(w.lounge.id, w.sam.0, "PA_sam");
    let now = chrono::Utc::now().timestamp_millis();
    let n = pulse_server::watch::restore(&w.app.db, &w.app.hub, &voice, now)
        .await
        .unwrap();
    assert_eq!(n, 1);
    let back = voice.watch.of(w.lounge.id).unwrap();
    assert_eq!(back.viewers, [w.sam.0]);
    assert_eq!(back.host_id, w.sam.0, "the host was gone, so it passed on");
    assert_eq!((back.video_id.as_str(), back.queue.len()), (V, 1));
    assert_eq!(
        back.set_at_ms, before.set_at_ms,
        "it carried on playing while down"
    );
    // everyone hears it came back
    assert_eq!(changed(&mut w.rx).await, back);

    // nobody from it in the call: dropped, snapshot and all
    let empty = pulse_server::voice::VoiceState::default();
    let n = pulse_server::watch::restore(&w.app.db, &w.app.hub, &empty, now)
        .await
        .unwrap();
    assert_eq!(n, 0);
    assert!(empty.watch.of(w.lounge.id).is_none());
    assert_eq!(snapshot_at(&w.app, w.lounge.id).await, None);
}

#[tokio::test]
async fn a_stale_snapshot_is_dropped_and_an_ended_party_leaves_none() {
    let mut w = world().await;
    assert_eq!(w.start(&w.alex.1, V).await.status(), 200);
    changed(&mut w.rx).await;
    let saved = snapshot_at(&w.app, w.lounge.id).await.unwrap();

    // too long after it was saved: not restored, and forgotten
    let voice = pulse_server::voice::VoiceState::default();
    voice.join(w.lounge.id, w.alex.0, "PA_alex");
    let late = saved + pulse_server::watch::RESTORE_WITHIN_MS + 1;
    let n = pulse_server::watch::restore(&w.app.db, &w.app.hub, &voice, late)
        .await
        .unwrap();
    assert_eq!(n, 0);
    assert!(voice.watch.of(w.lounge.id).is_none());
    assert_eq!(snapshot_at(&w.app, w.lounge.id).await, None);
    quiet(&mut w.rx).await;

    // a party that ends removes its snapshot
    assert_eq!(
        w.act(&w.alex.1, json!({"action": "pause", "position_ms": 1000}))
            .await,
        204
    );
    changed(&mut w.rx).await;
    assert!(snapshot_at(&w.app, w.lounge.id).await.is_some());
    assert_eq!(w.end(&w.alex.1).await, 204);
    next(&mut w.rx).await;
    assert_eq!(snapshot_at(&w.app, w.lounge.id).await, None);
}

#[tokio::test]
async fn a_pause_sticks_for_everyone_and_play_resumes_where_it_paused() {
    let mut w = world().await;
    w.start(&w.alex.1, V).await;
    changed(&mut w.rx).await;
    assert_eq!(w.act(&w.sam.1, json!({"action": "join"})).await, 204);
    changed(&mut w.rx).await;
    let mut alex_rx = w.app.hub.register(w.alex.0, String::new()).rx;
    assert_eq!(
        w.act(&w.alex.1, json!({"action": "pause", "position_ms": 42_500}))
            .await,
        204
    );
    // everyone hears it, the pauser too, as paused at that moment
    let p = changed(&mut w.rx).await;
    assert_eq!((p.playing, p.position_ms), (false, 42_500));
    assert_eq!(changed(&mut alex_rx).await, p);
    // and it stays paused: nothing re-announces it playing, and its time stands still
    quiet(&mut w.rx).await;
    let now = chrono::Utc::now().timestamp_millis();
    let held = w.app.voice.watch.of(w.lounge.id).unwrap();
    assert!(!held.playing);
    assert_eq!(
        pulse_server::watch::position_at(&held, now + 60_000),
        42_500
    );
    // a late joiner (in Ready) gets it paused at the same moment
    let ready = ready_of(&w.app, &w.kim.1).await;
    let seen = &ready.watch_parties[0];
    assert_eq!((seen.playing, seen.position_ms), (false, 42_500));
    // so does a restart (the snapshot is the paused party)
    let voice = pulse_server::voice::VoiceState::default();
    voice.join(w.lounge.id, w.alex.0, "PA_alex");
    pulse_server::watch::restore(&w.app.db, &w.app.hub, &voice, now)
        .await
        .unwrap();
    let back = voice.watch.of(w.lounge.id).unwrap();
    assert_eq!((back.playing, back.position_ms), (false, 42_500));
    changed(&mut w.rx).await;
    // play picks up where it paused
    assert_eq!(
        w.act(&w.alex.1, json!({"action": "play", "position_ms": 42_500}))
            .await,
        204
    );
    let p = changed(&mut w.rx).await;
    assert_eq!((p.playing, p.position_ms), (true, 42_500));
    assert_eq!(
        pulse_server::watch::position_at(&p, p.set_at_ms + 1_000),
        43_500
    );
}
