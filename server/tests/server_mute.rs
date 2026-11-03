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
    let chans: Vec<Channel> =
        get_json(app, &alex.1, &format!("/servers/{}/channels", s.id)).await;
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
            if let Some(ServerFrame::Event(Event::ServerVoiceChanged { state })) = rx.recv().await
            {
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
    post_json(&app, &kim.1, &format!("/servers/{}/join", w.server), json!({})).await;
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
    for body in [json!({"mute": true}), json!({"mute": false}), json!({"deafen": false})] {
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
    set(&app, &w.alex.1, w.server, w.sam.0, json!({"mute": true, "deafen": true})).await;
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
