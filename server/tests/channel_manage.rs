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

// ---------- bitrate (#75) ----------

/// An authed PUT with a body; the whole response.
async fn put_json(
    app: &TestApp,
    token: &str,
    path: &str,
    body: serde_json::Value,
) -> reqwest::Response {
    app.http
        .put(app.url(path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

/// Anyone in the server sets a voice channel's bitrate; everyone hears it (`ChannelUpdated`), and
/// the listings and Ready carry it from then on. Back to Default sends no bitrate at all.
#[tokio::test]
async fn anyone_in_the_server_sets_a_voice_channels_bitrate() {
    let w = world().await;
    let lounge = create_channel(&w.app, &w.alex.1, w.server, "voice", "Music").await;
    assert_eq!(lounge.bitrate, None, "a new channel is at Default");
    let mut rx = w.app.hub.register(w.jo.0, String::new()).rx;
    let path = format!("/channels/{}/bitrate", lounge.id);
    let r = put_json(&w.app, &w.sam.1, &path, json!({ "bitrate": 256 })).await;
    assert_eq!(r.status(), 200);
    let ch: Channel = r.json().await.unwrap();
    assert_eq!(ch.bitrate, Some(256));
    assert_eq!((ch.id, ch.name.as_deref()), (lounge.id, Some("Music")));
    match next_event(&mut rx).await {
        Event::ChannelUpdated { channel } => assert_eq!(channel, ch),
        other => panic!("unexpected {other:?}"),
    }
    let listed: Vec<Channel> =
        get_json(&w.app, &w.jo.1, &format!("/servers/{}/channels", w.server)).await;
    assert_eq!(
        listed.iter().find(|c| c.id == lounge.id).unwrap().bitrate,
        Some(256)
    );
    let ready = pulse_server::gateway::socket::build_ready(
        &w.app.db,
        &w.app.voice,
        &w.app.streams,
        &w.app.hub,
        w.sam.0,
    )
    .await
    .unwrap();
    assert_eq!(
        ready
            .channels
            .iter()
            .find(|c| c.id == lounge.id)
            .unwrap()
            .bitrate,
        Some(256)
    );
    // a rename (what older clients send) leaves it alone
    let r = patch(
        &w.app,
        &w.alex.1,
        &format!("/channels/{}", lounge.id),
        json!({ "name": "Jam" }),
    )
    .await;
    assert_eq!(r.json::<Channel>().await.unwrap().bitrate, Some(256));
    let _ = next_event(&mut rx).await;
    // back to Default
    let r = put_json(&w.app, &w.alex.1, &path, json!({ "bitrate": null })).await;
    assert_eq!(r.status(), 200);
    let text = r.text().await.unwrap();
    assert!(!text.contains("bitrate"), "{text}");
    match next_event(&mut rx).await {
        Event::ChannelUpdated { channel } => assert_eq!(channel.bitrate, None),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn setting_a_bitrate_is_checked() {
    let w = world().await;
    let lounge = create_channel(&w.app, &w.alex.1, w.server, "voice", "Music").await;
    let path = format!("/channels/{}/bitrate", lounge.id);
    // only the presets (Default is null, never its number)
    for b in [
        json!(64),
        json!(0),
        json!(100),
        json!(384),
        json!(510),
        json!(-1),
        json!("128"),
    ] {
        let st = put_json(&w.app, &w.alex.1, &path, json!({ "bitrate": b }))
            .await
            .status();
        assert!(st == 400 || st == 422, "{b}: {st}");
    }
    for b in pulse_protocol::rest::CHANNEL_BITRATES_KBPS {
        let r = put_json(&w.app, &w.alex.1, &path, json!({ "bitrate": b })).await;
        assert_eq!(r.status(), 200, "{b}");
    }
    // not in the server
    assert_eq!(
        put_json(&w.app, &w.jo.1, &path, json!({ "bitrate": 128 }))
            .await
            .status(),
        403
    );
    // a channel without a call
    let g = general(&w.app, &w.alex.1, w.server).await;
    assert_eq!(
        put_json(
            &w.app,
            &w.alex.1,
            &format!("/channels/{}/bitrate", g.id),
            json!({ "bitrate": 128 })
        )
        .await
        .status(),
        400
    );
    // no such channel
    assert_eq!(
        put_json(
            &w.app,
            &w.alex.1,
            &format!("/channels/{}/bitrate", ChannelId::new()),
            json!({ "bitrate": 128 })
        )
        .await
        .status(),
        404
    );
    // a deleted one is gone
    assert_eq!(
        delete(&w.app, &w.alex.1, &format!("/channels/{}", lounge.id)).await,
        204
    );
    assert_eq!(
        put_json(&w.app, &w.alex.1, &path, json!({ "bitrate": 128 }))
            .await
            .status(),
        404
    );
}

/// A DM or group call has no channel settings: its bitrate stays Default.
#[tokio::test]
async fn a_conversation_has_no_bitrate() {
    let w = world().await;
    let r = post_json(&w.app, &w.alex.1, "/dms", json!({ "user_ids": [w.sam.0] })).await;
    assert!(r.status().is_success(), "{}", r.status());
    let dm: Channel = r.json().await.unwrap();
    assert_eq!(
        put_json(
            &w.app,
            &w.alex.1,
            &format!("/channels/{}/bitrate", dm.id),
            json!({ "bitrate": 128 })
        )
        .await
        .status(),
        400
    );
}

// ---------- delete ----------

/// A delete moves the channel to Recently deleted: gone for everyone at once, everything in it
/// kept (a restore brings it all back).
#[tokio::test]
async fn deleting_a_text_channel_moves_it_to_recently_deleted() {
    let w = world().await;
    let old = create_channel(&w.app, &w.alex.1, w.server, "text", "old").await;
    let first = send(&w.app, &w.alex.1, old.id, "findable words @sam").await;
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
    // Invisible: channel lists, the ready snapshot (channels and read states), search.
    let listed: Vec<Channel> = get_json(
        &w.app,
        &w.alex.1,
        &format!("/servers/{}/channels", w.server),
    )
    .await;
    assert!(!listed.iter().any(|c| c.id == old.id));
    let ready = pulse_server::gateway::socket::build_ready(
        &w.app.db,
        &w.app.voice,
        &w.app.streams,
        &w.app.hub,
        w.sam.0,
    )
    .await
    .unwrap();
    assert!(!ready.channels.iter().any(|c| c.id == old.id));
    assert!(!ready.read_states.iter().any(|r| r.channel_id == old.id));
    for q in [
        "q=findable".to_string(),
        format!("q=findable&in={}", old.id),
    ] {
        let found: pulse_protocol::rest::SearchResults = get_json(
            &w.app,
            &w.alex.1,
            &format!("/servers/{}/search?{q}", w.server),
        )
        .await;
        assert!(found.hits.is_empty(), "{q}: {found:?}");
    }
    // Nothing reaches it: reading, posting, reacting, renaming, deleting again.
    let r = w
        .app
        .http
        .get(w.app.url(&format!("/channels/{}/messages", old.id)))
        .bearer_auth(&w.alex.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/channels/{}/messages", old.id),
        json!({ "content": "hello?" }),
    )
    .await;
    assert_eq!(r.status(), 404);
    let r = put(
        &w.app,
        &w.alex.1,
        &format!("/messages/{}/reactions/%F0%9F%91%8D", first.id),
        None,
    )
    .await;
    assert_eq!(r.as_u16(), 404);
    assert_eq!(
        patch(
            &w.app,
            &w.alex.1,
            &format!("/channels/{}", old.id),
            json!({ "name": "x" })
        )
        .await
        .status(),
        404
    );
    assert_eq!(
        delete(&w.app, &w.alex.1, &format!("/channels/{}", old.id)).await,
        404,
        "already deleted"
    );
    // ...but nothing is purged.
    let deleted_by: Option<String> = sqlx::query_scalar(
        "SELECT deleted_by FROM channels WHERE id = ? AND deleted_at IS NOT NULL",
    )
    .bind(&id)
    .fetch_one(&w.app.db)
    .await
    .unwrap();
    assert_eq!(deleted_by, Some(w.sam.0.to_string()));
    for table in ["messages", "read_states", "search_docs"] {
        let q = format!("SELECT COUNT(*) FROM {table} WHERE channel_id = ?");
        assert!(count(&w.app, &q, &id).await > 0, "{table}");
    }
    let first = first.id.to_string();
    for table in ["reactions", "mentions"] {
        let q = format!("SELECT COUNT(*) FROM {table} WHERE message_id = ?");
        assert_eq!(count(&w.app, &q, &first).await, 1, "{table}");
    }
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM notification_prefs WHERE target_kind = 'channel' AND target_id = ?",
            &id
        )
        .await,
        1
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
    let (_, deleted) = livekit::serve_fake_deleting(listener, rooms.clone());
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
    // and jo, whose join webhook hasn't applied yet (#109): LiveKit has them, our voice state
    // doesn't, so no snapshot of it would take them out
    people.push(livekit_protocol::ParticipantInfo {
        identity: w.jo.0.to_string(),
        sid: "PA_j".into(),
        ..Default::default()
    });
    rooms.lock().unwrap().push((roomid.clone(), people));
    // alex's screen share and a ghost watching it, in the stream room
    let in_stream = |identity: String| livekit_protocol::ParticipantInfo {
        identity,
        ..Default::default()
    };
    rooms.lock().unwrap().push((
        format!("{roomid}~s"),
        vec![
            in_stream(format!("{}#v", w.alex.0)),
            in_stream("g_ghost#v".into()),
        ],
    ));

    assert_eq!(
        delete(&w.app, &w.sam.1, &format!("/channels/{}", room.id)).await,
        204
    );

    assert!(w.app.voice_members(room.id).is_empty());
    // Their sessions end (the rows stay, as the channel does, until it is purged).
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM voice_sessions WHERE channel_id = ? AND left_at IS NULL",
            &roomid
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM voice_sessions WHERE channel_id = ?",
            &roomid
        )
        .await,
        2
    );
    // Nobody can join it while it is deleted.
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/voice/{}/token", room.id),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 404);
    // LiveKit is told (in the background) to close its voice and stream rooms, which takes out
    // everyone in them, jo and the ghost included (#109)
    livekit::wait_rooms_deleted(&deleted, &rooms, &roomid).await;
}

// ---------- Recently deleted: list, restore, purge ----------

use pulse_protocol::rest::DeletedChannel;

/// kim joins the server (a member who is neither the owner nor the deleter).
async fn kim(w: &World) -> (UserId, String) {
    let kim = register(&w.app, "kim").await;
    let r = post_json(
        &w.app,
        &kim.1,
        &format!("/servers/{}/join", w.server),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 204);
    kim
}

async fn deleted_list(w: &World, token: &str) -> Vec<DeletedChannel> {
    get_json(
        &w.app,
        token,
        &format!("/servers/{}/deleted-channels", w.server),
    )
    .await
}

async fn status_of(r: reqwest::RequestBuilder, token: &str) -> u16 {
    r.bearer_auth(token).send().await.unwrap().status().as_u16()
}

async fn restore(w: &World, token: &str, id: ChannelId) -> reqwest::Response {
    post_json(&w.app, token, &format!("/channels/{id}/restore"), json!({})).await
}

async fn purge_now(w: &World, token: &str, id: ChannelId) -> u16 {
    delete(&w.app, token, &format!("/channels/{id}/permanent")).await
}

#[tokio::test]
async fn recently_deleted_lists_who_when_and_what_each_viewer_may_do() {
    let w = world().await;
    let kim = kim(&w).await;
    let old = create_channel(&w.app, &w.alex.1, w.server, "voice", "Old room").await;
    assert!(deleted_list(&w, &w.alex.1).await.is_empty());
    assert_eq!(
        delete(&w.app, &w.sam.1, &format!("/channels/{}", old.id)).await,
        204
    );

    let mine = |list: Vec<DeletedChannel>| {
        assert_eq!(list.len(), 1, "{list:?}");
        list.into_iter().next().unwrap()
    };
    // alex owns the server: restore and purge
    let d = mine(deleted_list(&w, &w.alex.1).await);
    assert_eq!(d.channel, old);
    assert_eq!(d.deleted_by, Some(w.sam.0));
    assert_eq!(
        (d.can_restore, d.can_purge, d.held.clone()),
        (true, true, None)
    );
    let at = chrono::DateTime::parse_from_rfc3339(&d.deleted_at).unwrap();
    let purge = chrono::DateTime::parse_from_rfc3339(&d.purge_at).unwrap();
    assert_eq!(purge - at, chrono::Duration::days(7));
    // sam deleted it: restore only
    let d = mine(deleted_list(&w, &w.sam.1).await);
    assert_eq!((d.can_restore, d.can_purge), (true, false));
    // kim: neither
    let d = mine(deleted_list(&w, &kim.1).await);
    assert_eq!((d.can_restore, d.can_purge), (false, false));
    // jo isn't in the server; there's no such server
    let path = format!("/servers/{}/deleted-channels", w.server);
    assert_eq!(
        status_of(w.app.http.get(w.app.url(&path)), &w.jo.1).await,
        403
    );
    let path = format!("/servers/{}/deleted-channels", ServerId::new());
    assert_eq!(
        status_of(w.app.http.get(w.app.url(&path)), &w.alex.1).await,
        404
    );
}

#[tokio::test]
async fn the_deleter_or_the_owner_restores_and_everyone_hears() {
    let w = world().await;
    let kim = kim(&w).await;
    let old = create_channel(&w.app, &w.alex.1, w.server, "text", "old").await;
    send(&w.app, &w.alex.1, old.id, "still here").await;
    let r = put(
        &w.app,
        &w.sam.1,
        "/mutes",
        Some(json!({ "target_kind": "channel", "target_id": old.id, "until": null })),
    )
    .await;
    assert!(r.is_success(), "mute {r}");
    assert_eq!(
        delete(&w.app, &w.sam.1, &format!("/channels/{}", old.id)).await,
        204
    );
    // A channel of the same name meanwhile: names needn't be unique.
    let twin = create_channel(&w.app, &w.alex.1, w.server, "text", "old").await;

    // Neither the owner nor the deleter, or not in the server: no.
    assert_eq!(restore(&w, &kim.1, old.id).await.status(), 403);
    assert_eq!(restore(&w, &w.jo.1, old.id).await.status(), 403);

    let mut jo_rx = w.app.hub.register(w.jo.0, String::new()).rx;
    let mut sam_rx = w.app.hub.register(w.sam.0, String::new()).rx;
    let r = restore(&w, &w.sam.1, old.id).await;
    assert_eq!(r.status(), 200);
    // Back in its place in the list (#54): the bottom of its area, below the twin made meanwhile.
    let back = Channel {
        position: twin.position + 1,
        ..old.clone()
    };
    assert_eq!(r.json::<Channel>().await.unwrap(), back);

    match next_event(&mut jo_rx).await {
        Event::ChannelRestored { channel } => assert_eq!(channel, back),
        other => panic!("unexpected {other:?}"),
    }
    match next_event(&mut sam_rx).await {
        Event::ChannelRestored { channel } => assert_eq!(channel, back),
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(
        next_event(&mut sam_rx).await,
        Event::LayoutChanged { .. }
    ));
    // sam's own counts and mutes come back with it
    let mut got_counts = false;
    let mut got_mutes = false;
    while !(got_counts && got_mutes) {
        match next_event(&mut sam_rx).await {
            Event::ReadStateUpdated {
                user_id,
                channel_id,
                unread,
                ..
            } => {
                assert_eq!((user_id, channel_id, unread), (w.sam.0, old.id, Some(1)));
                got_counts = true;
            }
            Event::MutesChanged { user_id, mutes } => {
                assert_eq!(user_id, w.sam.0);
                assert!(mutes.iter().any(|m| m.target_id == old.id.to_string()));
                got_mutes = true;
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    let listed: Vec<Channel> =
        get_json(&w.app, &kim.1, &format!("/servers/{}/channels", w.server)).await;
    assert!(listed.contains(&back) && listed.contains(&twin));
    let page: Vec<Message> =
        get_json(&w.app, &kim.1, &format!("/channels/{}/messages", old.id)).await;
    assert_eq!(page.len(), 1);
    assert!(deleted_list(&w, &w.alex.1).await.is_empty());
    assert_eq!(
        restore(&w, &w.sam.1, old.id).await.status(),
        404,
        "not deleted"
    );
    assert_eq!(purge_now(&w, &w.alex.1, old.id).await, 404, "not deleted");

    // The owner restores what someone else deleted.
    assert_eq!(
        delete(&w.app, &kim.1, &format!("/channels/{}", old.id)).await,
        204
    );
    assert_eq!(restore(&w, &w.alex.1, old.id).await.status(), 200);
}

#[tokio::test]
async fn deleting_now_is_for_the_owner_and_purges_everything() {
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
    // Only from Recently deleted.
    assert_eq!(purge_now(&w, &w.alex.1, old.id).await, 404);
    assert_eq!(
        delete(&w.app, &w.sam.1, &format!("/channels/{}", old.id)).await,
        204
    );
    // Only the owner, even for the one who deleted it.
    assert_eq!(purge_now(&w, &w.sam.1, old.id).await, 403);
    assert_eq!(purge_now(&w, &w.jo.1, old.id).await, 403);
    assert_eq!(purge_now(&w, &w.alex.1, old.id).await, 204);

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
        count(
            &w.app,
            "SELECT COUNT(*) FROM notification_prefs WHERE target_kind = 'channel' AND target_id = ?",
            &id
        )
        .await,
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
    assert!(deleted_list(&w, &w.alex.1).await.is_empty());
    assert_eq!(restore(&w, &w.alex.1, old.id).await.status(), 404);
    assert_eq!(purge_now(&w, &w.alex.1, old.id).await, 404, "already gone");
}

#[tokio::test]
async fn the_sweeper_purges_after_seven_days() {
    let w = world().await;
    let old = create_channel(&w.app, &w.alex.1, w.server, "text", "old").await;
    let live = create_channel(&w.app, &w.alex.1, w.server, "text", "live").await;
    send(&w.app, &w.alex.1, old.id, "bye").await;
    assert_eq!(
        delete(&w.app, &w.sam.1, &format!("/channels/{}", old.id)).await,
        204
    );
    let s = w.app.state();
    let now = chrono::Utc::now();
    let swept = pulse_server::servers::channels::purge_expired(&s, now + chrono::Duration::days(6))
        .await
        .unwrap();
    assert_eq!(swept, 0);
    assert_eq!(deleted_list(&w, &w.alex.1).await.len(), 1);
    let swept = pulse_server::servers::channels::purge_expired(&s, now + chrono::Duration::days(8))
        .await
        .unwrap();
    assert_eq!(swept, 1);
    let id = old.id.to_string();
    assert_eq!(
        count(&w.app, "SELECT COUNT(*) FROM channels WHERE id = ?", &id).await,
        0
    );
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM messages WHERE channel_id = ?",
            &id
        )
        .await,
        0
    );
    // live channels are never touched
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM channels WHERE id = ?",
            &live.id.to_string()
        )
        .await,
        1
    );
}

/// Clients drop a channel's clip view with it, so a restore sends it back to whoever set one.
#[tokio::test]
async fn a_restore_brings_back_your_clip_view() {
    let w = world().await;
    let clips = create_channel(&w.app, &w.alex.1, w.server, "clips", "clips").await;
    let r = put(
        &w.app,
        &w.sam.1,
        &format!("/channels/{}/clip-view", clips.id),
        Some(json!({ "view": "list" })),
    )
    .await;
    assert!(r.is_success(), "clip view {r}");
    assert_eq!(
        delete(&w.app, &w.sam.1, &format!("/channels/{}", clips.id)).await,
        204
    );
    let mut rx = w.app.hub.register(w.sam.0, String::new()).rx;
    assert_eq!(restore(&w, &w.alex.1, clips.id).await.status(), 200);
    loop {
        match next_event(&mut rx).await {
            Event::ClipViewChanged {
                user_id,
                channel_id,
                view,
            } => {
                assert_eq!(
                    (user_id, channel_id, view),
                    (w.sam.0, clips.id, pulse_protocol::rest::ClipView::List)
                );
                break;
            }
            Event::ChannelRestored { .. }
            | Event::LayoutChanged { .. }
            | Event::ReadStateUpdated { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}
