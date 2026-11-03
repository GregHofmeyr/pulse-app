//! Renaming and deleting server channels (anyone in the server may, as with creating them).
mod common;

use std::time::Duration;

use common::*;
use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use pulse_protocol::rest::{Channel, Message};
use serde_json::json;
use tokio::sync::mpsc;

async fn patch(
    app: &TestApp,
    token: &str,
    path: &str,
    body: serde_json::Value,
) -> reqwest::Response {
    app.http
        .patch(app.url(path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

async fn delete(app: &TestApp, token: &str, path: &str) -> u16 {
    app.http
        .delete(app.url(path))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn create_channel(
    app: &TestApp,
    token: &str,
    server: ServerId,
    kind: &str,
    name: &str,
) -> Channel {
    let r = post_json(
        app,
        token,
        &format!("/servers/{server}/channels"),
        json!({ "kind": kind, "name": name }),
    )
    .await;
    assert_eq!(r.status(), 200, "create {kind} channel");
    r.json().await.unwrap()
}

/// An authed PUT; returns its status.
async fn put(
    app: &TestApp,
    token: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> reqwest::StatusCode {
    let mut r = app.http.put(app.url(path)).bearer_auth(token);
    if let Some(b) = body {
        r = r.json(&b);
    }
    r.send().await.unwrap().status()
}

async fn count(app: &TestApp, q: &str, id: &str) -> i64 {
    sqlx::query_scalar(q)
        .bind(id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// The next event `rx` gets, skipping presence and new users.
async fn next_event(rx: &mut mpsc::Receiver<ServerFrame>) -> Event {
    loop {
        let f = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("an event arrives")
            .expect("hub open");
        match f {
            ServerFrame::Event(Event::PresenceChanged { .. } | Event::UserCreated { .. }) => {}
            ServerFrame::Event(e) => return e,
            _ => {}
        }
    }
}

/// alex's server "Main", which sam has joined; jo is registered but not in it.
struct World {
    app: TestApp,
    alex: (UserId, String),
    sam: (UserId, String),
    jo: (UserId, String),
    server: ServerId,
}

async fn world_with(app: TestApp) -> World {
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let jo = register(&app, "jo").await;
    let s = create_server(&app, &alex.1, "Main").await;
    let r = post_json(&app, &sam.1, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);
    World {
        app,
        alex,
        sam,
        jo,
        server: s.id,
    }
}

async fn world() -> World {
    world_with(spawn().await).await
}

// ---------- rename ----------

#[tokio::test]
async fn anyone_in_the_server_renames_a_channel_and_everyone_hears() {
    let w = world().await;
    let g = general(&w.app, &w.alex.1, w.server).await;
    let mut rx = w.app.hub.register(w.jo.0, String::new()).rx;
    let r = patch(
        &w.app,
        &w.sam.1,
        &format!("/channels/{}", g.id),
        json!({ "name": "  chat  " }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let ch: Channel = r.json().await.unwrap();
    assert_eq!(ch.name.as_deref(), Some("chat"));
    assert_eq!(
        (ch.id, ch.server_id, ch.kind, ch.position),
        (g.id, g.server_id, g.kind, g.position)
    );
    match next_event(&mut rx).await {
        Event::ChannelUpdated { channel } => assert_eq!(channel, ch),
        other => panic!("unexpected {other:?}"),
    }
    let listed: Vec<Channel> =
        get_json(&w.app, &w.jo.1, &format!("/servers/{}/channels", w.server)).await;
    assert!(
        listed
            .iter()
            .any(|c| c.id == g.id && c.name.as_deref() == Some("chat"))
    );
    // A server channel's rename posts no system line (a group's does).
    let page: Vec<Message> =
        get_json(&w.app, &w.alex.1, &format!("/channels/{}/messages", g.id)).await;
    assert!(page.is_empty(), "{page:?}");
}

#[tokio::test]
async fn voice_channels_rename_too() {
    let w = world().await;
    let lounge = create_channel(&w.app, &w.alex.1, w.server, "voice", "Lounge 2").await;
    let r = patch(
        &w.app,
        &w.alex.1,
        &format!("/channels/{}", lounge.id),
        json!({ "name": "Gaming" }),
    )
    .await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<Channel>().await.unwrap().name.as_deref(),
        Some("Gaming")
    );
}

#[tokio::test]
async fn renaming_a_server_channel_is_checked() {
    let w = world().await;
    let g = general(&w.app, &w.alex.1, w.server).await;
    let path = format!("/channels/{}", g.id);
    // not in the server
    assert_eq!(
        patch(&w.app, &w.jo.1, &path, json!({ "name": "x" }))
            .await
            .status(),
        403
    );
    // a server channel always has a name
    for name in [json!(null), json!(""), json!("   "), json!("x".repeat(65))] {
        assert_eq!(
            patch(&w.app, &w.alex.1, &path, json!({ "name": name }))
                .await
                .status(),
            400,
            "{name}"
        );
    }
    assert_eq!(
        patch(&w.app, &w.alex.1, &path, json!({ "name": "é".repeat(64) }))
            .await
            .status(),
        200
    );
    assert_eq!(
        patch(
            &w.app,
            &w.alex.1,
            &format!("/channels/{}", ChannelId::new()),
            json!({ "name": "x" })
        )
        .await
        .status(),
        404
    );
}

// ---------- delete ----------

#[tokio::test]
async fn deleting_a_text_channel_takes_everything_in_it() {
    let w = world().await;
    let old = create_channel(&w.app, &w.alex.1, w.server, "text", "old").await;
    let keep = general(&w.app, &w.alex.1, w.server).await;
    let first = send(&w.app, &w.alex.1, old.id, "findable words @sam").await;
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/channels/{}/messages", old.id),
        json!({ "content": "a findable reply", "reply_to_id": first.id }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let kept = send(&w.app, &w.alex.1, keep.id, "findable elsewhere").await;
    let r = put(
        &w.app,
        &w.sam.1,
        &format!("/messages/{}/reactions/%F0%9F%91%8D", first.id),
        None,
    )
    .await;
    assert!(r.is_success(), "react {r}");
    let r = put(
        &w.app,
        &w.sam.1,
        "/mutes",
        Some(json!({ "target_kind": "channel", "target_id": old.id, "until": null })),
    )
    .await;
    assert!(r.is_success(), "mute {r}");
    let id = old.id.to_string();
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM search_docs WHERE channel_id = ?",
            &id
        )
        .await,
        2
    );
    let mut rx = w.app.hub.register(w.jo.0, String::new()).rx;

    assert_eq!(
        delete(&w.app, &w.sam.1, &format!("/channels/{}", old.id)).await,
        204
    );

    match next_event(&mut rx).await {
        Event::ChannelDeleted {
            channel_id,
            server_id,
        } => {
            assert_eq!((channel_id, server_id), (old.id, w.server));
        }
        other => panic!("unexpected {other:?}"),
    }
    let listed: Vec<Channel> = get_json(
        &w.app,
        &w.alex.1,
        &format!("/servers/{}/channels", w.server),
    )
    .await;
    assert!(!listed.iter().any(|c| c.id == old.id));
    assert_eq!(
        count(&w.app, "SELECT COUNT(*) FROM channels WHERE id = ?", &id).await,
        0
    );
    for table in ["messages", "read_states", "search_docs"] {
        let q = format!("SELECT COUNT(*) FROM {table} WHERE channel_id = ?");
        assert_eq!(count(&w.app, &q, &id).await, 0, "{table}");
    }
    let first = first.id.to_string();
    for table in ["reactions", "mentions"] {
        let q = format!("SELECT COUNT(*) FROM {table} WHERE message_id = ?");
        assert_eq!(count(&w.app, &q, &first).await, 0, "{table}");
    }
    assert_eq!(
        count(&w.app, "SELECT COUNT(*) FROM notification_prefs WHERE target_kind = 'channel' AND target_id = ?", &id).await,
        0
    );
    // The search index lost the channel's messages and nothing else.
    let docs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM message_search WHERE message_search MATCH 'findable'",
    )
    .fetch_one(&w.app.db)
    .await
    .unwrap();
    assert_eq!(docs, 1);
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM search_docs WHERE message_id = ?",
            &kept.id.to_string()
        )
        .await,
        1
    );
    let r = w
        .app
        .http
        .get(w.app.url(&format!("/channels/{}/messages", old.id)))
        .bearer_auth(&w.alex.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    assert_eq!(
        delete(&w.app, &w.alex.1, &format!("/channels/{}", old.id)).await,
        404,
        "already gone"
    );
}

#[tokio::test]
async fn deleting_is_for_server_channels_and_their_members() {
    let w = world().await;
    let g = general(&w.app, &w.alex.1, w.server).await;
    assert_eq!(
        delete(&w.app, &w.jo.1, &format!("/channels/{}", g.id)).await,
        403
    );
    let dm = create_dm(&w.app, &w.alex.1, &[w.sam.0]).await;
    assert_eq!(
        delete(&w.app, &w.alex.1, &format!("/channels/{}", dm.id)).await,
        400
    );
    let group = create_dm(&w.app, &w.alex.1, &[w.sam.0, w.jo.0]).await;
    assert_eq!(
        delete(&w.app, &w.alex.1, &format!("/channels/{}", group.id)).await,
        400
    );
    // someone else's DM looks missing
    let other = create_dm(&w.app, &w.sam.1, &[w.jo.0]).await;
    assert_eq!(
        delete(&w.app, &w.alex.1, &format!("/channels/{}", other.id)).await,
        404
    );
    assert_eq!(
        delete(
            &w.app,
            &w.alex.1,
            &format!("/channels/{}", ChannelId::new())
        )
        .await,
        404
    );
}

#[tokio::test]
async fn deleting_a_voice_channel_kicks_whoever_is_in_it() {
    let rooms = livekit::SharedRooms::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = test_config();
    cfg.livekit_api_url = format!("http://{}", listener.local_addr().unwrap());
    let removed = livekit::serve_fake_shared(listener, rooms.clone());
    let w = world_with(spawn_with(cfg).await).await;
    let room = create_channel(&w.app, &w.alex.1, w.server, "voice", "Room").await;
    let roomid = room.id.to_string();
    // alex and sam are in the call, as LiveKit and our voice state have it, with a row each
    let mut people = Vec::new();
    for (u, sid) in [(w.alex.0, "PA_a"), (w.sam.0, "PA_s")] {
        assert!(w.app.voice.join(room.id, u, sid));
        sqlx::query(
            "INSERT INTO voice_sessions (id, user_id, channel_id, joined_at) VALUES (?, ?, ?, ?)",
        )
        .bind(pulse_protocol::ids::next_ulid().to_string())
        .bind(u.to_string())
        .bind(&roomid)
        .bind(pulse_server::db::now())
        .execute(&w.app.db)
        .await
        .unwrap();
        people.push(livekit_protocol::ParticipantInfo {
            identity: u.to_string(),
            sid: sid.into(),
            ..Default::default()
        });
    }
    rooms.lock().unwrap().push((roomid.clone(), people));

    assert_eq!(
        delete(&w.app, &w.sam.1, &format!("/channels/{}", room.id)).await,
        204
    );

    assert!(w.app.voice_members(room.id).is_empty());
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM voice_sessions WHERE channel_id = ?",
            &roomid
        )
        .await,
        0
    );
    // LiveKit is told to drop both (in the background)
    let mut want = vec![
        (roomid.clone(), w.alex.0.to_string()),
        (roomid.clone(), w.sam.0.to_string()),
    ];
    want.sort();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let mut got: Vec<(String, String)> = removed.lock().unwrap().clone();
        got.sort();
        if got == want {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "LiveKit removals: {got:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
