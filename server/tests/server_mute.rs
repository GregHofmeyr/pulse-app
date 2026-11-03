//! Server mute, server deafen and disconnect (#140): anyone in a server may do them to anyone
//! else in its voice channels. Mute/deafen is stored per (server, user) and enforced at LiveKit
//! (no publishing); disconnect removes the participant now and stores nothing.

mod common;

use std::time::Duration;

use common::*;
use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use pulse_protocol::rest::{Channel, ServerVoiceMod};
use serde_json::json;
use tokio::sync::mpsc::Receiver;

struct World {
    server: ServerId,
    lounge: Channel,
    alex: (UserId, String),
    sam: (UserId, String),
}

/// alex's server "Main", which sam has joined; nobody in voice yet.
async fn world(app: &TestApp) -> World {
    let alex = register(app, "alex").await;
    let sam = register(app, "sam").await;
    let s = create_server(app, &alex.1, "Main").await;
    let r = post_json(app, &sam.1, &format!("/servers/{}/join", s.id), json!({})).await;
    assert!(r.status().is_success());
    let chans: Vec<Channel> = get_json(app, &alex.1, &format!("/servers/{}/channels", s.id)).await;
    let lounge = chans
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    World {
        server: s.id,
        lounge,
        alex,
        sam,
    }
}

async fn set(
    app: &TestApp,
    token: &str,
    server: ServerId,
    user: UserId,
    body: serde_json::Value,
) -> reqwest::Response {
    post_json(app, token, &format!("/servers/{server}/voice/{user}"), body).await
}

/// The next `ServerVoiceChanged` on `rx` (other events skipped), within a second.
async fn next_change(rx: &mut Receiver<ServerFrame>) -> ServerVoiceMod {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(ServerFrame::Event(Event::ServerVoiceChanged { state })) = rx.recv().await {
                return state;
            }
        }
    })
    .await
    .expect("no ServerVoiceChanged")
}

async fn rows(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM voice_moderation")
        .fetch_one(&app.db)
        .await
        .unwrap()
}

fn in_voice(app: &TestApp, channel: ChannelId, user: UserId) {
    app.voice.join(channel, user, "PA_test");
}

#[tokio::test]
async fn mute_sets_row_and_broadcasts() {
    let app = spawn().await;
    let w = world(&app).await;
    in_voice(&app, w.lounge.id, w.sam.0);
    let mut rx = app.hub.register(w.sam.0, String::new()).rx;
    let r = set(&app, &w.alex.1, w.server, w.sam.0, json!({"mute": true})).await;
    assert_eq!(r.status(), 200);
    let got: ServerVoiceMod = r.json().await.unwrap();
    let want = ServerVoiceMod {
        server_id: w.server,
        user_id: w.sam.0,
        muted_by: Some(w.alex.0),
        deafened_by: None,
    };
    assert_eq!(got, want);
    assert_eq!(next_change(&mut rx).await, want);
    // deafen keeps the mute (and who did it)
    let r = set(&app, &w.alex.1, w.server, w.sam.0, json!({"deafen": true})).await;
    let got: ServerVoiceMod = r.json().await.unwrap();
    assert_eq!(
        (got.muted_by, got.deafened_by),
        (Some(w.alex.0), Some(w.alex.0))
    );
}

#[tokio::test]
async fn setting_it_again_keeps_who_did_it_and_publishes_nothing() {
    let app = spawn().await;
    let w = world(&app).await;
    let kim = register(&app, "kim").await;
    post_json(
        &app,
        &kim.1,
        &format!("/servers/{}/join", w.server),
        json!({}),
    )
    .await;
    in_voice(&app, w.lounge.id, w.sam.0);
    set(&app, &w.alex.1, w.server, w.sam.0, json!({"mute": true})).await;
    let mut rx = app.hub.register(w.sam.0, String::new()).rx;
    let r = set(&app, &kim.1, w.server, w.sam.0, json!({"mute": true})).await;
    let got: ServerVoiceMod = r.json().await.unwrap();
    assert_eq!(got.muted_by, Some(w.alex.0));
    assert!(
        tokio::time::timeout(Duration::from_millis(200), next_change(&mut rx))
            .await
            .is_err(),
        "nothing changed, nothing published"
    );
}

#[tokio::test]
async fn cannot_target_yourself() {
    let app = spawn().await;
    let w = world(&app).await;
    in_voice(&app, w.lounge.id, w.alex.0);
    for body in [
        json!({"mute": true}),
        json!({"mute": false}),
        json!({"deafen": false}),
    ] {
        let r = set(&app, &w.alex.1, w.server, w.alex.0, body).await;
        assert_eq!(r.status(), 400);
    }
}

#[tokio::test]
async fn outsider_gets_403() {
    let app = spawn().await;
    let w = world(&app).await;
    let jo = register(&app, "jo").await;
    in_voice(&app, w.lounge.id, w.sam.0);
    let r = set(&app, &jo.1, w.server, w.sam.0, json!({"mute": true})).await;
    assert_eq!(r.status(), 403);
    // and a target who isn't a member is not found
    let r = set(&app, &w.alex.1, w.server, jo.0, json!({"mute": true})).await;
    assert_eq!(r.status(), 404);
    assert_eq!(rows(&app).await, 0);
}

#[tokio::test]
async fn mute_needs_target_in_this_servers_voice() {
    let app = spawn().await;
    let w = world(&app).await;
    let r = set(&app, &w.alex.1, w.server, w.sam.0, json!({"mute": true})).await;
    assert_eq!(r.status(), 409);
    // in another server's voice doesn't count
    let other = create_server(&app, &w.sam.1, "Other").await;
    let chans: Vec<Channel> =
        get_json(&app, &w.sam.1, &format!("/servers/{}/channels", other.id)).await;
    let elsewhere = chans
        .iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    in_voice(&app, elsewhere.id, w.sam.0);
    let r = set(&app, &w.alex.1, w.server, w.sam.0, json!({"deafen": true})).await;
    assert_eq!(r.status(), 409);
    assert_eq!(rows(&app).await, 0);
}

#[tokio::test]
async fn undo_without_call_clears_row() {
    let app = spawn().await;
    let w = world(&app).await;
    in_voice(&app, w.lounge.id, w.sam.0);
    set(
        &app,
        &w.alex.1,
        w.server,
        w.sam.0,
        json!({"mute": true, "deafen": true}),
    )
    .await;
    assert_eq!(rows(&app).await, 1);
    app.voice.leave(w.lounge.id, w.sam.0, "");
    let mut rx = app.hub.register(w.alex.0, String::new()).rx;
    let r = set(&app, &w.alex.1, w.server, w.sam.0, json!({"mute": false})).await;
    assert_eq!(r.status(), 200);
    let got: ServerVoiceMod = r.json().await.unwrap();
    assert_eq!((got.muted_by, got.deafened_by), (None, Some(w.alex.0)));
    assert_eq!(next_change(&mut rx).await, got);
    let r = set(&app, &w.alex.1, w.server, w.sam.0, json!({"deafen": false})).await;
    assert_eq!(r.status(), 200);
    assert_eq!(rows(&app).await, 0, "nothing in force: no row");
    let gone = next_change(&mut rx).await;
    assert_eq!((gone.muted_by, gone.deafened_by), (None, None));
}

#[tokio::test]
async fn ready_carries_server_voice() {
    let app = spawn().await;
    let w = world(&app).await;
    in_voice(&app, w.lounge.id, w.sam.0);
    set(&app, &w.alex.1, w.server, w.sam.0, json!({"mute": true})).await;
    let ready = pulse_server::gateway::socket::build_ready(
        &app.db,
        &app.voice,
        &app.streams,
        &app.hub,
        w.sam.0,
    )
    .await
    .unwrap();
    assert_eq!(
        ready.server_voice,
        vec![ServerVoiceMod {
            server_id: w.server,
            user_id: w.sam.0,
            muted_by: Some(w.alex.0),
            deafened_by: None,
        }]
    );
}

// --- LiveKit enforcement and disconnect -------------------------------------------------------

use common::livekit::{Removed, SharedRooms, Updated, fake_livekit_full, fake_livekit_hanging};
use livekit_api::access_token::{AccessToken, TokenVerifier};
use livekit_protocol as lkp;
use pulse_protocol::rest::VoiceTokenResponse;

struct Lk {
    rooms: SharedRooms,
    removed: Removed,
    updated: Updated,
}

impl Lk {
    /// LiveKit has `identity` connected in `room`, allowed to publish or not.
    fn connect(&self, room: &str, identity: &str, can_publish: bool) {
        let p = lkp::ParticipantInfo {
            identity: identity.to_owned(),
            sid: format!("PA_{identity}"),
            permission: Some(lkp::ParticipantPermission {
                can_publish,
                can_subscribe: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut rooms = self.rooms.lock().unwrap();
        match rooms.iter_mut().find(|(r, _)| r == room) {
            Some((_, ps)) => ps.push(p),
            None => rooms.push((room.to_owned(), vec![p])),
        }
    }
    fn updates(&self) -> Vec<(String, String, bool)> {
        self.updated.lock().unwrap().clone()
    }
}

async fn spawn_lk() -> (TestApp, Lk) {
    let rooms = SharedRooms::default();
    let (url, removed, updated) = fake_livekit_full(rooms.clone()).await;
    let mut cfg = test_config();
    cfg.livekit_api_url = url;
    (
        spawn_with(cfg).await,
        Lk {
            rooms,
            removed,
            updated,
        },
    )
}

/// Polls `f` for up to a second (background LiveKit calls).
async fn eventually(mut f: impl FnMut() -> bool) -> bool {
    for _ in 0..50 {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn mute_revokes_publish_at_livekit() {
    let (app, lk) = spawn_lk().await;
    let w = world(&app).await;
    let room = w.lounge.id.to_string();
    in_voice(&app, w.lounge.id, w.sam.0);
    lk.connect(&room, &w.sam.0.to_string(), true);
    set(&app, &w.alex.1, w.server, w.sam.0, json!({"mute": true})).await;
    assert_eq!(
        lk.updates(),
        vec![(room.clone(), w.sam.0.to_string(), false)]
    );
    set(&app, &w.alex.1, w.server, w.sam.0, json!({"mute": false})).await;
    assert_eq!(
        lk.updates().last(),
        Some(&(room.clone(), w.sam.0.to_string(), true))
    );
}

#[tokio::test]
async fn deafen_implies_no_publish() {
    let (app, lk) = spawn_lk().await;
    let w = world(&app).await;
    let room = w.lounge.id.to_string();
    in_voice(&app, w.lounge.id, w.sam.0);
    lk.connect(&room, &w.sam.0.to_string(), true);
    set(&app, &w.alex.1, w.server, w.sam.0, json!({"deafen": true})).await;
    assert_eq!(
        lk.updates(),
        vec![(room.clone(), w.sam.0.to_string(), false)]
    );
    // still muted by the deafen: un-muting alone changes nothing at LiveKit
    set(&app, &w.alex.1, w.server, w.sam.0, json!({"mute": true})).await;
    set(&app, &w.alex.1, w.server, w.sam.0, json!({"mute": false})).await;
    assert_eq!(lk.updates().len(), 1, "{:?}", lk.updates());
    set(&app, &w.alex.1, w.server, w.sam.0, json!({"deafen": false})).await;
    assert_eq!(
        lk.updates().last(),
        Some(&(room, w.sam.0.to_string(), true))
    );
}

#[tokio::test]
async fn mute_in_other_server_leaves_live_call_alone() {
    let (app, lk) = spawn_lk().await;
    let w = world(&app).await;
    in_voice(&app, w.lounge.id, w.sam.0);
    set(&app, &w.alex.1, w.server, w.sam.0, json!({"mute": true})).await;
    // sam moves to his own server's call; the mute in Main is undone from there
    app.voice.leave(w.lounge.id, w.sam.0, "");
    let other = create_server(&app, &w.sam.1, "Other").await;
    let chans: Vec<Channel> =
        get_json(&app, &w.sam.1, &format!("/servers/{}/channels", other.id)).await;
    let elsewhere = chans
        .iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    in_voice(&app, elsewhere.id, w.sam.0);
    lk.connect(&elsewhere.id.to_string(), &w.sam.0.to_string(), true);
    let before = lk.updates().len();
    set(&app, &w.alex.1, w.server, w.sam.0, json!({"mute": false})).await;
    assert_eq!(lk.updates().len(), before, "{:?}", lk.updates());
}

fn video_grants(app: &TestApp, t: &VoiceTokenResponse) -> livekit_api::access_token::VideoGrants {
    TokenVerifier::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .verify(&t.token)
        .unwrap()
        .video
}

#[tokio::test]
async fn token_while_muted_cannot_publish() {
    let app = spawn().await;
    let w = world(&app).await;
    let path = format!("/voice/{}/token", w.lounge.id);
    let t: VoiceTokenResponse = post_json(&app, &w.sam.1, &path, json!({}))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(video_grants(&app, &t).can_publish, Some(true));
    in_voice(&app, w.lounge.id, w.sam.0);
    set(&app, &w.alex.1, w.server, w.sam.0, json!({"deafen": true})).await;
    app.voice.leave(w.lounge.id, w.sam.0, "");
    let t: VoiceTokenResponse = post_json(&app, &w.sam.1, &path, json!({}))
        .await
        .json()
        .await
        .unwrap();
    let g = video_grants(&app, &t);
    assert_eq!(g.can_publish, Some(false));
    assert_eq!(g.can_subscribe, Some(true));
}

fn signed(app: &TestApp, body: &str) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let sum = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body.as_bytes()));
    AccessToken::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .with_sha256(&sum)
        .to_jwt()
        .unwrap()
}

/// LiveKit's `participant_joined` for `user` in `room`, whose token let them publish or not.
async fn joined(app: &TestApp, room: ChannelId, user: UserId, can_publish: bool) {
    let grant = grant_livekit(app, &room.to_string(), &user.to_string()).await;
    let body =
        json!({"event": "participant_joined", "id": "EV_test", "room": {"name": room.to_string()},
        "participant": {"identity": user.to_string(), "sid": format!("PA_{user}"),
            "attributes": {"pulse_grant": grant},
            "permission": {"canPublish": can_publish, "canSubscribe": true}}})
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
async fn join_reconciles_publish_permission() {
    let (app, lk) = spawn_lk().await;
    let w = world(&app).await;
    let (room, sam) = (w.lounge.id.to_string(), w.sam.0.to_string());
    in_voice(&app, w.lounge.id, w.sam.0);
    set(&app, &w.alex.1, w.server, w.sam.0, json!({"mute": true})).await;
    app.voice.leave(w.lounge.id, w.sam.0, "");
    // a token fetched before the mute (prefetched): LiveKit lets them publish, we take it away
    lk.connect(&room, &sam, true);
    joined(&app, w.lounge.id, w.sam.0, true).await;
    assert!(
        eventually(|| lk.updates() == vec![(room.clone(), sam.clone(), false)]).await,
        "{:?}",
        lk.updates()
    );
    // a matching permission needs no call
    app.voice.leave(w.lounge.id, w.sam.0, "");
    joined(&app, w.lounge.id, w.sam.0, false).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(lk.updates().len(), 1);
    // a token from while muted, used after the undo: publishing given back
    set(&app, &w.alex.1, w.server, w.sam.0, json!({"mute": false})).await;
    app.voice.leave(w.lounge.id, w.sam.0, "");
    let n = lk.updates().len();
    joined(&app, w.lounge.id, w.sam.0, false).await;
    assert!(
        eventually(|| lk.updates().len() > n
            && lk.updates().last() == Some(&(room.clone(), sam.clone(), true)))
        .await,
        "{:?}",
        lk.updates()
    );
}

#[tokio::test]
async fn disconnect_removes_from_voice_and_stream_rooms_and_tells_target() {
    let (app, lk) = spawn_lk().await;
    let w = world(&app).await;
    let (room, sam) = (w.lounge.id.to_string(), w.sam.0.to_string());
    in_voice(&app, w.lounge.id, w.sam.0);
    lk.connect(&room, &sam, true);
    lk.connect(&format!("{room}~s"), &format!("{sam}#v"), false);
    lk.connect(&format!("{room}~s"), &format!("{sam}#a"), false);
    let mut rx = app.hub.register(w.sam.0, String::new()).rx;
    let mut alex_rx = app.hub.register(w.alex.0, String::new()).rx;
    let r = post_json(
        &app,
        &w.alex.1,
        &format!("/servers/{}/voice/{}/disconnect", w.server, w.sam.0),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 204);
    let mut removed = lk.removed.lock().unwrap().clone();
    removed.sort();
    let mut want = vec![
        (room.clone(), sam.clone()),
        (format!("{room}~s"), format!("{sam}#a")),
        (format!("{room}~s"), format!("{sam}#v")),
    ];
    want.sort();
    assert_eq!(removed, want);
    let told = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(ServerFrame::Event(Event::VoiceDisconnected {
                channel_id,
                user_id,
                by,
            })) = rx.recv().await
            {
                return (channel_id, user_id, by);
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(told, (w.lounge.id, w.sam.0, w.alex.0));
    // only the target hears who did it
    while let Ok(f) = alex_rx.try_recv() {
        assert!(
            !matches!(f, ServerFrame::Event(Event::VoiceDisconnected { .. })),
            "{f:?}"
        );
    }
    assert_eq!(rows(&app).await, 0, "a disconnect stores nothing");
}

#[tokio::test]
async fn disconnect_needs_target_in_voice_and_not_yourself() {
    let app = spawn().await;
    let w = world(&app).await;
    let path = |u: UserId| format!("/servers/{}/voice/{u}/disconnect", w.server);
    let r = post_json(&app, &w.alex.1, &path(w.sam.0), json!({})).await;
    assert_eq!(r.status(), 404);
    in_voice(&app, w.lounge.id, w.alex.0);
    let r = post_json(&app, &w.alex.1, &path(w.alex.0), json!({})).await;
    assert_eq!(r.status(), 400);
    let jo = register(&app, "jo").await;
    in_voice(&app, w.lounge.id, w.sam.0);
    let r = post_json(&app, &jo.1, &path(w.sam.0), json!({})).await;
    assert_eq!(r.status(), 403);
}

/// A LiveKit that doesn't answer delays the mute's own reply, never anyone's chat (the route
/// mustn't hold the app-wide event order while it waits on LiveKit).
#[tokio::test]
async fn a_slow_livekit_never_holds_up_chat() {
    let mut cfg = test_config();
    cfg.livekit_api_url = fake_livekit_hanging().await;
    let app = spawn_with(cfg).await;
    let w = world(&app).await;
    let general = general(&app, &w.sam.1, w.server).await;
    in_voice(&app, w.lounge.id, w.sam.0);
    let mute = {
        let (http, url, token) = (
            app.http.clone(),
            app.url(&format!("/servers/{}/voice/{}", w.server, w.sam.0)),
            w.alex.1.clone(),
        );
        tokio::spawn(async move {
            http.post(url)
                .bearer_auth(token)
                .json(&json!({"mute": true}))
                .send()
                .await
                .unwrap()
                .status()
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    let t = std::time::Instant::now();
    send(&app, &w.sam.1, general.id, "still here").await;
    assert!(
        t.elapsed() < Duration::from_millis(1000),
        "a message waited {:?} on LiveKit",
        t.elapsed()
    );
    assert_eq!(mute.await.unwrap(), 200, "the mute still lands");
    assert_eq!(rows(&app).await, 1);
}

/// Undo racing the target's join: the route saw no call (so told LiveKit nothing) and the
/// join's token still couldn't publish. The join, recorded after, reads the undone row and gives
/// publishing back.
#[tokio::test]
async fn an_undo_racing_a_join_still_gives_publishing_back() {
    let (app, lk) = spawn_lk().await;
    let w = world(&app).await;
    let (room, sam) = (w.lounge.id.to_string(), w.sam.0.to_string());
    in_voice(&app, w.lounge.id, w.sam.0);
    set(&app, &w.alex.1, w.server, w.sam.0, json!({"mute": true})).await;
    app.voice.leave(w.lounge.id, w.sam.0, "");
    set(&app, &w.alex.1, w.server, w.sam.0, json!({"mute": false})).await;
    let n = lk.updates().len();
    lk.connect(&room, &sam, false);
    joined(&app, w.lounge.id, w.sam.0, false).await;
    assert!(
        eventually(|| lk.updates()[n..] == [(room.clone(), sam.clone(), true)]).await,
        "{:?}",
        lk.updates()
    );
}
