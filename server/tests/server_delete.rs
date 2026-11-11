//! Deleting a server (#184): its owner, with its name typed; 7 days in their Recently deleted,
//! restorable whole; then purged with everything in it.
mod common;

use std::time::Duration;

use common::*;
use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{Channel, DeletedServer, Message, Server};
use serde_json::json;
use tokio::sync::mpsc;

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

async fn count(app: &TestApp, q: &str, id: &str) -> i64 {
    sqlx::query_scalar(q)
        .bind(id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn status(r: reqwest::RequestBuilder, token: &str) -> u16 {
    r.bearer_auth(token).send().await.unwrap().status().as_u16()
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
    assert_eq!(s.owner_id, Some(alex.0), "a new server says who owns it");
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

async fn delete_server(w: &World, token: &str, server: ServerId, name: &str) -> u16 {
    status(
        w.app
            .http
            .delete(w.app.url(&format!("/servers/{server}")))
            .json(&json!({ "name": name })),
        token,
    )
    .await
}

async fn restore(w: &World, token: &str, server: ServerId) -> reqwest::Response {
    post_json(
        &w.app,
        token,
        &format!("/servers/{server}/restore"),
        json!({}),
    )
    .await
}

async fn deleted_list(w: &World, token: &str) -> Vec<DeletedServer> {
    get_json(&w.app, token, "/servers/deleted").await
}

async fn channels(w: &World) -> Vec<Channel> {
    get_json(
        &w.app,
        &w.alex.1,
        &format!("/servers/{}/channels", w.server),
    )
    .await
}

async fn ready_for(w: &World, user: UserId) -> pulse_protocol::gateway::Ready {
    pulse_server::gateway::socket::build_ready(
        &w.app.db,
        &w.app.voice,
        &w.app.streams,
        &w.app.hub,
        user,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn only_the_owner_deletes_and_only_with_its_name() {
    let w = world().await;
    // a member who isn't the owner, and someone not in it
    assert_eq!(delete_server(&w, &w.sam.1, w.server, "Main").await, 403);
    assert_eq!(delete_server(&w, &w.jo.1, w.server, "Main").await, 403);
    // the name must be typed as it is
    assert_eq!(delete_server(&w, &w.alex.1, w.server, "main").await, 400);
    assert_eq!(delete_server(&w, &w.alex.1, w.server, "").await, 400);
    assert_eq!(
        delete_server(&w, &w.alex.1, ServerId::new(), "Main").await,
        404
    );
    // nothing happened
    let listed: Vec<Server> = get_json(&w.app, &w.alex.1, "/servers").await;
    assert!(listed.iter().any(|s| s.id == w.server));
    // surrounding spaces are forgiven
    assert_eq!(delete_server(&w, &w.alex.1, w.server, " Main ").await, 204);
    assert_eq!(
        delete_server(&w, &w.alex.1, w.server, "Main").await,
        404,
        "already deleted"
    );
}

#[tokio::test]
async fn a_deleted_server_is_gone_for_everyone_but_kept_whole() {
    let w = world().await;
    let general = general(&w.app, &w.alex.1, w.server).await;
    let first = send(&w.app, &w.alex.1, general.id, "findable words @sam").await;
    let live = channels(&w).await;
    assert!(live.len() >= 2);
    let mut rx = w.app.hub.register(w.jo.0, String::new()).rx;
    let kim = register(&w.app, "kim").await;

    assert_eq!(delete_server(&w, &w.alex.1, w.server, "Main").await, 204);

    // A ChannelDeleted for each of its channels (what older clients understand), then ServerDeleted.
    let mut gone = Vec::new();
    loop {
        match next_event(&mut rx).await {
            Event::ChannelDeleted {
                channel_id,
                server_id,
            } => {
                assert_eq!(server_id, w.server);
                gone.push(channel_id);
            }
            Event::ServerDeleted { server_id } => {
                assert_eq!(server_id, w.server);
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    gone.sort();
    let mut want: Vec<_> = live.iter().map(|c| c.id).collect();
    want.sort();
    assert_eq!(gone, want);

    // Off everyone's rail and out of Ready.
    let listed: Vec<Server> = get_json(&w.app, &w.sam.1, "/servers").await;
    assert!(!listed.iter().any(|s| s.id == w.server));
    for user in [w.alex.0, w.sam.0] {
        let ready = ready_for(&w, user).await;
        assert!(!ready.servers.iter().any(|s| s.id == w.server));
        assert!(!ready.channels.iter().any(|c| c.server_id == Some(w.server)));
        assert!(!ready.members.iter().any(|m| m.server_id == w.server));
        assert!(!ready.layouts.iter().any(|l| l.server_id == w.server));
        assert!(!ready.read_states.iter().any(|r| r.channel_id == general.id));
    }

    // Nothing in it can be reached.
    let get = |path: String| w.app.http.get(w.app.url(&path));
    assert_eq!(
        status(get(format!("/channels/{}/messages", general.id)), &w.alex.1).await,
        404
    );
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/channels/{}/messages", general.id),
        json!({ "content": "hello?" }),
    )
    .await;
    assert_eq!(r.status(), 404);
    let r = w
        .app
        .http
        .put(
            w.app
                .url(&format!("/messages/{}/reactions/%F0%9F%91%8D", first.id)),
        )
        .bearer_auth(&w.sam.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    for path in [
        format!("/servers/{}/channels", w.server),
        format!("/servers/{}/members", w.server),
        format!("/servers/{}/deleted-channels", w.server),
        format!("/servers/{}/wheels", w.server),
        format!("/servers/{}/search?q=findable", w.server),
    ] {
        assert_eq!(status(get(path.clone()), &w.alex.1).await, 404, "{path}");
    }
    // the soundboard answers "not a member" for a server that isn't there, as it always has
    assert_eq!(
        status(get(format!("/servers/{}/sounds", w.server)), &w.alex.1).await,
        403
    );
    // Nobody can join it (Pulse has no per-server invites: joining is the way in).
    let r = post_json(
        &w.app,
        &kim.1,
        &format!("/servers/{}/join", w.server),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 404);
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/servers/{}/channels", w.server),
        json!({ "kind": "text", "name": "new" }),
    )
    .await;
    assert_eq!(r.status(), 404);
    // nor muted
    let r = w
        .app
        .http
        .put(w.app.url("/mutes"))
        .bearer_auth(&w.sam.1)
        .json(&json!({ "target_kind": "server", "target_id": w.server, "until": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404, "muting a deleted server");

    // ...but nothing is purged.
    let id = w.server.to_string();
    let by: Option<String> = sqlx::query_scalar(
        "SELECT deleted_by FROM servers WHERE id = ? AND deleted_at IS NOT NULL",
    )
    .bind(&id)
    .fetch_one(&w.app.db)
    .await
    .unwrap();
    assert_eq!(by, Some(w.alex.0.to_string()));
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM server_members WHERE server_id = ?",
            &id
        )
        .await,
        2
    );
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM channels WHERE server_id = ? AND deleted_at IS NULL",
            &id
        )
        .await,
        live.len() as i64
    );
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM messages WHERE channel_id = ?",
            &general.id.to_string()
        )
        .await,
        1
    );
}

#[tokio::test]
async fn deleting_a_server_ends_its_calls() {
    let rooms = common::livekit::SharedRooms::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = test_config();
    cfg.livekit_api_url = format!("http://{}", listener.local_addr().unwrap());
    let removed = common::livekit::serve_fake_shared(listener, rooms.clone());
    let w = world_with(spawn_with(cfg).await).await;
    let lounge = channels(&w)
        .await
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    let roomid = lounge.id.to_string();
    let mut people = Vec::new();
    for (u, sid) in [(w.alex.0, "PA_a"), (w.sam.0, "PA_s")] {
        assert!(w.app.voice.join(lounge.id, u, sid));
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

    assert_eq!(delete_server(&w, &w.alex.1, w.server, "Main").await, 204);

    assert!(w.app.voice_members(lounge.id).is_empty());
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM voice_sessions WHERE channel_id = ? AND left_at IS NULL",
            &roomid
        )
        .await,
        0
    );
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/voice/{}/token", lounge.id),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 404, "nobody joins it while it is deleted");
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

#[tokio::test]
async fn the_owners_recently_deleted_lists_their_own_deleted_servers() {
    let w = world().await;
    let other = create_server(&w.app, &w.alex.1, "Other").await;
    let sams = create_server(&w.app, &w.sam.1, "Sam's").await;
    assert!(deleted_list(&w, &w.alex.1).await.is_empty());
    assert_eq!(delete_server(&w, &w.alex.1, w.server, "Main").await, 204);
    assert_eq!(delete_server(&w, &w.sam.1, sams.id, "Sam's").await, 204);

    let list = deleted_list(&w, &w.alex.1).await;
    assert_eq!(list.len(), 1, "{list:?}");
    let d = &list[0];
    assert_eq!(d.server.id, w.server);
    assert_eq!(d.server.name, "Main");
    assert!(d.can_restore);
    let at = chrono::DateTime::parse_from_rfc3339(&d.deleted_at).unwrap();
    let purge = chrono::DateTime::parse_from_rfc3339(&d.purge_at).unwrap();
    assert_eq!(purge - at, chrono::Duration::days(7));
    // sam sees only his own
    let list = deleted_list(&w, &w.sam.1).await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].server.id, sams.id);
    assert!(deleted_list(&w, &w.jo.1).await.is_empty());
    let _ = other;
}

#[tokio::test]
async fn the_owner_restores_and_everyone_gets_it_back() {
    let w = world().await;
    let other = create_server(&w.app, &w.alex.1, "Other").await;
    let general = general(&w.app, &w.alex.1, w.server).await;
    let sent: Message = send(&w.app, &w.alex.1, general.id, "still here @sam").await;
    assert_eq!(delete_server(&w, &w.alex.1, w.server, "Main").await, 204);

    assert_eq!(restore(&w, &w.sam.1, w.server).await.status(), 403);
    assert_eq!(restore(&w, &w.jo.1, w.server).await.status(), 403);
    assert_eq!(
        restore(&w, &w.alex.1, other.id).await.status(),
        404,
        "not deleted"
    );
    assert_eq!(restore(&w, &w.alex.1, ServerId::new()).await.status(), 404);

    let mut jo_rx = w.app.hub.register(w.jo.0, String::new()).rx;
    let mut sam_rx = w.app.hub.register(w.sam.0, String::new()).rx;
    let r = restore(&w, &w.alex.1, w.server).await;
    assert_eq!(r.status(), 200);
    let back: Server = r.json().await.unwrap();
    assert_eq!(back.id, w.server);
    assert_eq!(back.owner_id, Some(w.alex.0));

    match next_event(&mut jo_rx).await {
        Event::ServerRestored { snapshot } => {
            assert_eq!(snapshot.server.id, w.server);
            assert!(snapshot.channels.iter().any(|c| c.id == general.id));
            assert!(snapshot.members.iter().any(|m| m.user.id == w.sam.0));
            assert_eq!(snapshot.layout.server_id, w.server);
            assert!(!snapshot.layout.categories.is_empty());
        }
        other => panic!("unexpected {other:?}"),
    }
    // sam's counts in it come back: one unread mention
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        assert!(tokio::time::Instant::now() < deadline, "no read state");
        if let Event::ReadStateUpdated {
            user_id,
            channel_id,
            mentions,
            ..
        } = next_event(&mut sam_rx).await
            && channel_id == general.id
        {
            assert_eq!(user_id, w.sam.0);
            assert_eq!(mentions, Some(1));
            break;
        }
    }

    let listed: Vec<Server> = get_json(&w.app, &w.sam.1, "/servers").await;
    assert!(listed.iter().any(|s| s.id == w.server));
    let ready = ready_for(&w, w.sam.0).await;
    assert!(ready.channels.iter().any(|c| c.id == general.id));
    let msgs: Vec<Message> = get_json(
        &w.app,
        &w.sam.1,
        &format!("/channels/{}/messages", general.id),
    )
    .await;
    assert!(msgs.iter().any(|m| m.id == sent.id));
    assert!(deleted_list(&w, &w.alex.1).await.is_empty());
    assert_eq!(
        restore(&w, &w.alex.1, w.server).await.status(),
        404,
        "live again"
    );
}

async fn insert_media(app: &TestApp, hash: &str, kind: &str) {
    sqlx::query(
        "INSERT INTO media (hash, kind, content_type, size, width, height, frames, created_at)
         VALUES (?, ?, 'image/webp', 1, 1, 1, 1, ?)",
    )
    .bind(hash)
    .bind(kind)
    .bind(pulse_server::db::now())
    .execute(&app.db)
    .await
    .unwrap();
}

#[tokio::test]
async fn the_sweeper_purges_a_server_after_seven_days_in_batches() {
    let w = world().await;
    let other = create_server(&w.app, &w.alex.1, "Other").await;
    let kept_channel = general(&w.app, &w.alex.1, other.id).await;
    let kept = send(&w.app, &w.alex.1, kept_channel.id, "elsewhere").await;
    let general = general(&w.app, &w.alex.1, w.server).await;
    // Replies across batch boundaries (batches of 2 below).
    let mut last = send(&w.app, &w.alex.1, general.id, "first").await;
    for i in 0..6 {
        let r = post_json(
            &w.app,
            &w.sam.1,
            &format!("/channels/{}/messages", general.id),
            json!({ "content": format!("reply {i} @alex"), "reply_to_id": last.id }),
        )
        .await;
        assert_eq!(r.status(), 200);
        last = r.json().await.unwrap();
    }
    // A channel already in the server's Recently deleted.
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/servers/{}/channels", w.server),
        json!({ "kind": "text", "name": "old" }),
    )
    .await;
    let old: Channel = r.json().await.unwrap();
    send(&w.app, &w.alex.1, old.id, "in the old one").await;
    assert_eq!(
        status(
            w.app
                .http
                .delete(w.app.url(&format!("/channels/{}", old.id))),
            &w.alex.1
        )
        .await,
        204
    );
    // Mutes on the server and a channel, an emoji and a sound.
    for (kind, target) in [
        ("server", w.server.to_string()),
        ("channel", general.id.to_string()),
    ] {
        let r = w
            .app
            .http
            .put(w.app.url("/mutes"))
            .bearer_auth(&w.sam.1)
            .json(&json!({ "target_kind": kind, "target_id": target, "until": null }))
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success(), "mute {kind}");
    }
    let emoji_hash = "a".repeat(64);
    let sound_hash = "b".repeat(64);
    insert_media(&w.app, &emoji_hash, "emoji").await;
    insert_media(&w.app, &sound_hash, "sound").await;
    sqlx::query(
        "INSERT INTO custom_emoji (id, server_id, name, media_hash, animated, created_at)
         VALUES ('emo1', ?, 'wave', ?, 0, ?)",
    )
    .bind(w.server.to_string())
    .bind(&emoji_hash)
    .bind(pulse_server::db::now())
    .execute(&w.app.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sounds (id, server_id, name, media_hash, duration_ms, created_at)
         VALUES ('snd1', ?, 'boop', ?, 500, ?)",
    )
    .bind(w.server.to_string())
    .bind(&sound_hash)
    .bind(pulse_server::db::now())
    .execute(&w.app.db)
    .await
    .unwrap();
    sqlx::query("UPDATE server_members SET join_sound_id = 'snd1' WHERE server_id = ?")
        .bind(w.server.to_string())
        .execute(&w.app.db)
        .await
        .unwrap();

    assert_eq!(delete_server(&w, &w.alex.1, w.server, "Main").await, 204);
    let s = w.app.state();
    let now = chrono::Utc::now();

    // While the server is deleted its own Recently deleted channel waits too (frozen).
    let swept = pulse_server::servers::channels::purge_expired(&s, now + chrono::Duration::days(8))
        .await
        .unwrap();
    assert_eq!(swept, 0, "a deleted server's channels are left to it");

    let swept = pulse_server::servers::deleted::purge_expired_batched(
        &s,
        now + chrono::Duration::days(6),
        2,
    )
    .await
    .unwrap();
    assert_eq!(swept, 0);
    assert_eq!(deleted_list(&w, &w.alex.1).await.len(), 1);
    let swept = pulse_server::servers::deleted::purge_expired_batched(
        &s,
        now + chrono::Duration::days(8),
        2,
    )
    .await
    .unwrap();
    assert_eq!(swept, 1);

    let id = w.server.to_string();
    for table in [
        "servers WHERE id",
        "channels WHERE server_id",
        "server_members WHERE server_id",
        "categories WHERE server_id",
        "custom_emoji WHERE server_id",
        "sounds WHERE server_id",
        "notification_prefs WHERE target_id",
    ] {
        let q = format!("SELECT COUNT(*) FROM {table} = ?");
        assert_eq!(count(&w.app, &q, &id).await, 0, "{table}");
    }
    for ch in [general.id.to_string(), old.id.to_string()] {
        for table in ["messages", "read_states", "search_docs", "voice_sessions"] {
            let q = format!("SELECT COUNT(*) FROM {table} WHERE channel_id = ?");
            assert_eq!(count(&w.app, &q, &ch).await, 0, "{table}");
        }
        assert_eq!(
            count(
                &w.app,
                "SELECT COUNT(*) FROM notification_prefs WHERE target_id = ?",
                &ch
            )
            .await,
            0
        );
    }
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM mentions WHERE user_id = ?",
            &w.alex.0.to_string()
        )
        .await,
        0
    );
    // The other server is untouched.
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM messages WHERE id = ?",
            &kept.id.to_string()
        )
        .await,
        1
    );
    assert!(deleted_list(&w, &w.alex.1).await.is_empty());
    assert_eq!(restore(&w, &w.alex.1, w.server).await.status(), 404);
    // Nothing broke the database's references.
    let broken: Vec<(String, Option<i64>, String, i64)> =
        sqlx::query_as("PRAGMA foreign_key_check")
            .fetch_all(&w.app.db)
            .await
            .unwrap();
    assert!(broken.is_empty(), "{broken:?}");
}

/// Ids come from the clock: one that went backwards across a restart can leave an older message
/// replying to a newer one. The purge still goes through.
#[tokio::test]
async fn a_reply_older_than_what_it_answers_doesnt_stop_the_purge() {
    let w = world().await;
    let general = general(&w.app, &w.alex.1, w.server).await;
    let older = send(&w.app, &w.alex.1, general.id, "older").await;
    for i in 0..3 {
        send(&w.app, &w.alex.1, general.id, &format!("filler {i}")).await;
    }
    let newer = send(&w.app, &w.alex.1, general.id, "newer").await;
    sqlx::query("UPDATE messages SET reply_to_id = ? WHERE id = ?")
        .bind(newer.id.to_string())
        .bind(older.id.to_string())
        .execute(&w.app.db)
        .await
        .unwrap();
    assert_eq!(delete_server(&w, &w.alex.1, w.server, "Main").await, 204);
    let swept = pulse_server::servers::deleted::purge_expired_batched(
        &w.app.state(),
        chrono::Utc::now() + chrono::Duration::days(8),
        2,
    )
    .await
    .unwrap();
    assert_eq!(swept, 1);
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM servers WHERE id = ?",
            &w.server.to_string()
        )
        .await,
        0
    );
}

#[tokio::test]
async fn once_due_it_cant_be_restored() {
    let w = world().await;
    assert_eq!(delete_server(&w, &w.alex.1, w.server, "Main").await, 204);
    let eight_days_ago = pulse_server::db::stamp(chrono::Utc::now() - chrono::Duration::days(8));
    sqlx::query("UPDATE servers SET deleted_at = ? WHERE id = ?")
        .bind(&eight_days_ago)
        .bind(w.server.to_string())
        .execute(&w.app.db)
        .await
        .unwrap();
    let list = deleted_list(&w, &w.alex.1).await;
    assert_eq!(list.len(), 1);
    assert!(!list[0].can_restore, "it's being purged");
    assert_eq!(restore(&w, &w.alex.1, w.server).await.status(), 404);
}

/// A deleted server's emoji and sounds are out of everyone's Frequently used lists, aren't counted
/// as used when written in a message elsewhere, and can't be a sound's emoji elsewhere (#287).
#[tokio::test]
async fn a_deleted_servers_emoji_and_sounds_leave_the_frequent_lists() {
    let w = world().await;
    let (emoji_hash, sound_hash) = ("e".repeat(64), "f".repeat(64));
    insert_media(&w.app, &emoji_hash, "emoji").await;
    insert_media(&w.app, &sound_hash, "sound").await;
    let now = pulse_server::db::now();
    for q in [
        "INSERT INTO custom_emoji (id, server_id, name, media_hash, animated, created_at)
         VALUES ('emo1', ?1, 'wave', ?2, 0, ?4)",
        "INSERT INTO sounds (id, server_id, name, media_hash, duration_ms, created_at)
         VALUES ('snd1', ?1, 'boop', ?3, 500, ?4)",
        "INSERT INTO emoji_usage (user_id, emoji, score, last_used_at) VALUES (?5, 'custom:emo1', 5.0, ?4)",
        "INSERT INTO sound_usage (user_id, sound_id, score, last_used_at) VALUES (?5, 'snd1', 5.0, ?4)",
    ] {
        sqlx::query(q)
            .bind(w.server.to_string())
            .bind(&emoji_hash)
            .bind(&sound_hash)
            .bind(&now)
            .bind(w.sam.0.to_string())
            .execute(&w.app.db)
            .await
            .unwrap();
    }
    let emoji = || pulse_server::emoji::frequent(&w.app.db, w.sam.0, 27);
    let sounds = || pulse_server::sounds::frequent(&w.app.db, w.sam.0, 8);
    let live = || async {
        let mut c = w.app.db.acquire().await.unwrap();
        pulse_server::messages::reactions::live_keys(&mut c, w.sam.0, vec!["custom:emo1".into()])
            .await
            .unwrap()
    };
    assert!(emoji().await.unwrap().contains(&"custom:emo1".to_string()));
    assert_eq!(sounds().await.unwrap(), vec!["snd1".to_string()]);
    assert_eq!(live().await, vec!["custom:emo1".to_string()]);
    // sam's own server, whose new sound would use Main's emoji
    let other = create_server(&w.app, &w.sam.1, "Other").await;
    let sounds_path = format!("/servers/{}/sounds", other.id);
    let sound_with = |token: String| {
        post_json(
            &w.app,
            &w.sam.1,
            &sounds_path,
            json!({ "name": token, "emoji_id": "emo1", "media_hash": "0".repeat(64) }),
        )
    };
    let r = sound_with("live".into()).await;
    let body = r.text().await.unwrap();
    assert!(
        !body.contains("emoji isn't available"),
        "passes the emoji check: {body}"
    );

    assert_eq!(delete_server(&w, &w.alex.1, w.server, "Main").await, 204);
    assert!(!emoji().await.unwrap().contains(&"custom:emo1".to_string()));
    assert!(sounds().await.unwrap().is_empty());
    assert!(live().await.is_empty());
    let r = sound_with("gone".into()).await;
    let status = r.status().as_u16();
    let body = r.text().await.unwrap();
    assert_eq!(status, 400);
    assert!(body.contains("emoji isn't available"), "{body}");
}

/// A write that passed its "is the server there" check just before a delete committed (#287):
/// the delete holds the write lock while the request checks, then commits while the request waits
/// for the lock. The request must answer 404 and write nothing (no member, channel, category,
/// emoji, sound, game or Quote of the Year left in the deleted server, to come back on restore).
#[tokio::test]
async fn a_write_that_raced_the_delete_writes_nothing() {
    let w = world().await;
    let s = w.server;
    let emoji_hash = "c".repeat(64);
    let sound_hash = "d".repeat(64);
    insert_media(&w.app, &emoji_hash, "emoji").await;
    insert_media(&w.app, &sound_hash, "sound").await;
    let cases: Vec<(&str, &str, String, serde_json::Value, &str)> = vec![
        (
            "join",
            &w.jo.1,
            format!("/servers/{s}/join"),
            json!({}),
            "server_members",
        ),
        (
            "channel",
            &w.sam.1,
            format!("/servers/{s}/channels"),
            json!({ "kind": "text", "name": "late" }),
            "channels",
        ),
        (
            "category",
            &w.sam.1,
            format!("/servers/{s}/categories"),
            json!({ "name": "late" }),
            "categories",
        ),
        (
            "emoji",
            &w.sam.1,
            format!("/servers/{s}/emoji"),
            json!({ "name": "late", "media_hash": emoji_hash }),
            "custom_emoji",
        ),
        (
            "sound",
            &w.sam.1,
            format!("/servers/{s}/sounds"),
            json!({ "name": "late", "media_hash": sound_hash }),
            "sounds",
        ),
        (
            "game",
            &w.sam.1,
            format!("/servers/{s}/games"),
            json!({ "name": "late" }),
            "games",
        ),
        (
            "qoty",
            &w.sam.1,
            format!("/servers/{s}/qoty"),
            json!({ "year": 2026, "minutes": null }),
            "qoty_events",
        ),
    ];
    for (what, token, path, body, table) in cases {
        let rows = format!("SELECT COUNT(*) FROM {table} WHERE server_id = ?");
        let before = count(&w.app, &rows, &s.to_string()).await;
        let mut del = pulse_server::db::begin_write(&w.app.db).await.unwrap();
        sqlx::query("UPDATE servers SET deleted_at = ?, deleted_by = ? WHERE id = ?")
            .bind(pulse_server::db::now())
            .bind(w.alex.0.to_string())
            .bind(s.to_string())
            .execute(&mut *del)
            .await
            .unwrap();
        let (r, ()) = tokio::join!(post_json(&w.app, token, &path, body), async {
            // the request checks (the delete isn't committed, so the server is there) and then
            // waits for the write lock; the delete commits under it
            tokio::time::sleep(Duration::from_millis(300)).await;
            del.commit().await.unwrap();
        });
        let status = r.status().as_u16();
        let text = r.text().await.unwrap();
        assert_eq!(status, 404, "{what}: {text}");
        assert_eq!(
            count(&w.app, &rows, &s.to_string()).await,
            before,
            "{what}: nothing written into the deleted server"
        );
        sqlx::query("UPDATE servers SET deleted_at = NULL, deleted_by = NULL WHERE id = ?")
            .bind(s.to_string())
            .execute(&w.app.db)
            .await
            .unwrap();
    }
}
