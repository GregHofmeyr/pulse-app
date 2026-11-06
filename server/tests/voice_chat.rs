//! Text chat in voice channels (#226): a voice channel holds messages like a text channel, for anyone in the server.
mod common;

use std::borrow::Cow;
use std::time::Duration;

use common::*;
use futures::SinkExt;
use pulse_protocol::gateway::{ClientFrame, Ready, ServerFrame};
use pulse_protocol::ids::{ChannelId, ServerId};
use pulse_protocol::rest::{Channel, ChannelKind, Message, SearchResults};
use serde_json::json;
use sqlx::sqlite::SqliteConnectOptions;
use tokio_tungstenite::tungstenite::Message as Ws;

/// alex's server "Main" (sam has joined; jo hasn't), and its "Lounge" voice channel.
struct World {
    app: TestApp,
    alex: String,
    sam: String,
    jo: String,
    server: ServerId,
    lounge: Channel,
}

async fn world() -> World {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let (_, jo) = register(&app, "jo").await;
    let s = create_server(&app, &alex, "Main").await;
    let r = post_json(&app, &sam, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);
    let chans: Vec<Channel> = get_json(&app, &alex, &format!("/servers/{}/channels", s.id)).await;
    let lounge = chans
        .into_iter()
        .find(|c| c.kind == ChannelKind::Voice)
        .expect("a new server has a voice channel");
    World {
        app,
        alex,
        sam,
        jo,
        server: s.id,
        lounge,
    }
}

async fn ready_of(app: &TestApp, token: &str) -> Ready {
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

/// `(unread, mentions)` in `channel` as Ready reports them; None without a read point.
async fn counts(app: &TestApp, token: &str, channel: ChannelId) -> Option<(u32, u32)> {
    ready_of(app, token)
        .await
        .read_states
        .into_iter()
        .find(|r| r.channel_id == channel)
        .map(|r| (r.unread, r.mentions))
}

async fn status(r: reqwest::RequestBuilder, token: &str) -> u16 {
    r.bearer_auth(token).send().await.unwrap().status().as_u16()
}

#[test]
fn voice_has_chat_and_stays_out_of_the_message_only_kinds() {
    assert!(ChannelKind::Voice.has_chat());
    for k in [ChannelKind::Text, ChannelKind::Quotes, ChannelKind::Clips] {
        assert!(k.has_chat() && k.is_server_chat(), "{k:?}");
    }
    // voice tokens are refused for the message-only kinds: voice must stay out of them
    assert!(!ChannelKind::Voice.is_server_chat());
    assert!(!ChannelKind::Dm.has_chat() && !ChannelKind::Group.has_chat());
}

#[tokio::test]
async fn members_send_read_edit_and_delete_in_a_voice_channel() {
    let w = world().await;
    let m1 = send(&w.app, &w.alex, w.lounge.id, "can you hear me?").await;
    let m2 = send(&w.app, &w.sam, w.lounge.id, "loud and clear").await;
    let list: Vec<Message> = get_json(
        &w.app,
        &w.jo,
        &format!("/channels/{}/messages", w.lounge.id),
    )
    .await;
    assert_eq!(
        list.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![m2.id, m1.id],
        "anyone can read it, in the server or not, like a text channel"
    );
    let path = format!("/messages/{}", m1.id);
    assert_eq!(
        status(
            w.app
                .http
                .patch(w.app.url(&path))
                .json(&json!({"content": "hear me?"})),
            &w.alex
        )
        .await,
        200
    );
    assert_eq!(
        status(
            w.app
                .http
                .patch(w.app.url(&format!("/messages/{}", m2.id)))
                .json(&json!({"content": "no"})),
            &w.alex
        )
        .await,
        403,
        "only your own"
    );
    assert_eq!(
        status(w.app.http.delete(w.app.url(&path)), &w.alex).await,
        204
    );
    let list: Vec<Message> = get_json(
        &w.app,
        &w.sam,
        &format!("/channels/{}/messages", w.lounge.id),
    )
    .await;
    assert!(
        list.iter()
            .find(|m| m.id == m1.id)
            .is_none_or(|m| m.deleted)
    );
    // a reply, and a reaction
    let r = post_json(
        &w.app,
        &w.alex,
        &format!("/channels/{}/messages", w.lounge.id),
        json!({"content": "same", "reply_to_id": m2.id}),
    )
    .await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        status(
            w.app.http.put(
                w.app
                    .url(&format!("/messages/{}/reactions/%F0%9F%91%8D", m2.id))
            ),
            &w.alex
        )
        .await,
        204
    );
}

#[tokio::test]
async fn people_outside_the_server_cannot_post_or_react() {
    let w = world().await;
    let m = send(&w.app, &w.alex, w.lounge.id, "hi").await;
    let r = post_json(
        &w.app,
        &w.jo,
        &format!("/channels/{}/messages", w.lounge.id),
        json!({"content": "let me in"}),
    )
    .await;
    assert_eq!(r.status(), 403);
    assert_eq!(
        status(
            w.app.http.put(
                w.app
                    .url(&format!("/messages/{}/reactions/%F0%9F%91%8D", m.id))
            ),
            &w.jo
        )
        .await,
        403
    );
}

#[tokio::test]
async fn unreads_and_mentions_count_for_everyone_in_the_server() {
    let w = world().await;
    // the server's own voice channel (creator), a joiner's, and a voice channel made later
    assert_eq!(counts(&w.app, &w.alex, w.lounge.id).await, Some((0, 0)));
    assert_eq!(counts(&w.app, &w.sam, w.lounge.id).await, Some((0, 0)));
    let r = post_json(
        &w.app,
        &w.alex,
        &format!("/servers/{}/channels", w.server),
        json!({"name": "Den", "kind": "voice"}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let den: Channel = r.json().await.unwrap();
    assert_eq!(counts(&w.app, &w.sam, den.id).await, Some((0, 0)));

    send(&w.app, &w.alex, w.lounge.id, "anyone on?").await;
    let last = send(&w.app, &w.alex, w.lounge.id, "@sam you there").await;
    assert_eq!(counts(&w.app, &w.sam, w.lounge.id).await, Some((2, 1)));
    assert_eq!(
        counts(&w.app, &w.alex, w.lounge.id).await,
        Some((0, 0)),
        "your own don't count"
    );
    let r = post_json(
        &w.app,
        &w.sam,
        &format!("/channels/{}/read", w.lounge.id),
        json!({"message_id": last.id}),
    )
    .await;
    assert_eq!(r.status(), 204);
    assert_eq!(counts(&w.app, &w.sam, w.lounge.id).await, Some((0, 0)));
    // a server joined after messages were said there starts with them read
    let (_, kim) = register(&w.app, "kim").await;
    let r = post_json(
        &w.app,
        &kim,
        &format!("/servers/{}/join", w.server),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 204);
    assert_eq!(counts(&w.app, &kim, w.lounge.id).await, Some((0, 0)));
    // Ready's newest-message pointers stay a conversation list thing (DMs and groups)
    let ready = ready_of(&w.app, &w.sam).await;
    assert!(ready.latest.iter().all(|m| m.channel_id != w.lounge.id));
}

#[tokio::test]
async fn a_voice_channel_takes_a_notification_level() {
    let w = world().await;
    let r = w
        .app
        .http
        .put(w.app.url("/notify-levels"))
        .bearer_auth(&w.sam)
        .json(&json!({"target_kind": "channel", "target_id": w.lounge.id, "level": "mentions"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let ready = ready_of(&w.app, &w.sam).await;
    assert!(
        ready
            .notify_levels
            .iter()
            .any(|l| l.target_id == w.lounge.id.to_string())
    );
}

#[tokio::test]
async fn server_search_finds_voice_chat() {
    let w = world().await;
    let m = send(&w.app, &w.alex, w.lounge.id, "pineapple on the stream").await;
    let g = general(&w.app, &w.alex, w.server).await;
    send(&w.app, &w.alex, g.id, "no fruit here").await;
    let found: SearchResults = get_json(
        &w.app,
        &w.sam,
        &format!("/servers/{}/search?q=pineapple", w.server),
    )
    .await;
    assert_eq!(
        found.hits.iter().map(|h| h.message.id).collect::<Vec<_>>(),
        vec![m.id]
    );
    let only: SearchResults = get_json(
        &w.app,
        &w.sam,
        &format!(
            "/servers/{}/search?q=pineapple&in={}",
            w.server, w.lounge.id
        ),
    )
    .await;
    assert_eq!(only.hits.len(), 1, "in: a voice channel");
}

#[tokio::test]
async fn deleting_a_voice_channel_takes_its_chat_and_a_purge_removes_it() {
    let w = world().await;
    let m = send(&w.app, &w.sam, w.lounge.id, "@alex bye").await;
    let id = w.lounge.id;
    assert_eq!(
        status(
            w.app.http.delete(w.app.url(&format!("/channels/{id}"))),
            &w.alex
        )
        .await,
        204
    );
    assert_eq!(
        counts(&w.app, &w.alex, id).await,
        None,
        "its counts go with it"
    );
    let r = get_json::<SearchResults>(
        &w.app,
        &w.alex,
        &format!("/servers/{}/search?q=bye", w.server),
    )
    .await;
    assert!(r.hits.is_empty());
    // a restore brings it back, chat and counts
    assert_eq!(
        status(
            w.app
                .http
                .post(w.app.url(&format!("/channels/{id}/restore"))),
            &w.alex
        )
        .await,
        200
    );
    assert_eq!(counts(&w.app, &w.alex, id).await, Some((1, 1)));
    assert_eq!(
        status(
            w.app.http.delete(w.app.url(&format!("/channels/{id}"))),
            &w.alex
        )
        .await,
        204
    );
    assert_eq!(
        status(
            w.app
                .http
                .delete(w.app.url(&format!("/channels/{id}/permanent"))),
            &w.alex
        )
        .await,
        204
    );
    for table in ["messages", "read_states", "search_docs"] {
        let n: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM {table} WHERE channel_id = ?"
        ))
        .bind(id.to_string())
        .fetch_one(&w.app.db)
        .await
        .unwrap();
        assert_eq!(n, 0, "{table}");
    }
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mentions WHERE message_id = ?")
        .bind(m.id.to_string())
        .fetch_one(&w.app.db)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

/// The migration gives everyone already in a server a read point in its voice channels, at the newest message there.
#[tokio::test]
async fn migration_starts_existing_members_at_all_read_in_voice_channels() {
    let dir = tempfile::tempdir().unwrap();
    let opts = SqliteConnectOptions::new()
        .filename(dir.path().join("old.db"))
        .create_if_missing(true)
        .foreign_keys(false);
    let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();
    let mut migrator = sqlx::migrate::Migrator::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"),
    )
    .await
    .unwrap();
    let ours = migrator
        .migrations
        .iter()
        .find(|m| m.description.contains("voice chat"))
        .expect("the voice chat migration")
        .version;
    let all = migrator.migrations.clone();
    migrator.migrations = Cow::Owned(all.iter().filter(|m| m.version < ours).cloned().collect());
    migrator.run(&pool).await.unwrap();

    let ts = "2026-11-01T00:00:00.000Z";
    let (alex, sam, server, voice, text, other) = (
        "01J0000000000000000000ALEX",
        "01J00000000000000000000SAM",
        "01J000000000000000000SERVER",
        "01J00000000000000000VOICE0",
        "01J000000000000000000TEXT0",
        "01J00000000000000000OTHERS",
    );
    let line = "01J0000000000000000000LINE";
    for q in [
        format!(
            "INSERT INTO users (id, username, password_hash, created_at) VALUES ('{alex}', 'alex', 'x', '{ts}')"
        ),
        format!(
            "INSERT INTO users (id, username, password_hash, created_at) VALUES ('{sam}', 'sam', 'x', '{ts}')"
        ),
        format!(
            "INSERT INTO servers (id, name, created_by, created_at) VALUES ('{server}', 'Main', '{alex}', '{ts}')"
        ),
        format!(
            "INSERT INTO servers (id, name, created_by, created_at) VALUES ('{other}', 'Other', '{alex}', '{ts}')"
        ),
        format!(
            "INSERT INTO server_members (server_id, user_id, joined_at) VALUES ('{server}', '{alex}', '{ts}')"
        ),
        format!(
            "INSERT INTO server_members (server_id, user_id, joined_at) VALUES ('{server}', '{sam}', '{ts}')"
        ),
        format!(
            "INSERT INTO channels (id, server_id, kind, name, position, created_at) VALUES ('{voice}', '{server}', 'voice', 'Lounge', 0, '{ts}')"
        ),
        format!(
            "INSERT INTO channels (id, server_id, kind, name, position, created_at) VALUES ('{text}', '{other}', 'text', 'general', 0, '{ts}')"
        ),
        format!(
            "INSERT INTO read_states (user_id, channel_id, last_read_message_id) VALUES ('{alex}', '{text}', NULL)"
        ),
        // a line already there (none could be posted, but a point at the newest one is what joining gives)
        format!(
            "INSERT INTO messages (id, channel_id, author_id, content, created_at) VALUES ('{line}', '{voice}', '{alex}', 'hi', '{ts}')"
        ),
    ] {
        sqlx::query(&q).execute(&pool).await.unwrap();
    }
    migrator.migrations = Cow::Owned(all.into_owned());
    migrator.run(&pool).await.unwrap();
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT user_id, channel_id, last_read_message_id FROM read_states ORDER BY user_id, channel_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let mut want = vec![
        (alex.to_string(), text.to_string(), None),
        (alex.to_string(), voice.to_string(), Some(line.to_string())),
        (sam.to_string(), voice.to_string(), Some(line.to_string())),
    ];
    want.sort();
    assert_eq!(rows, want);
}
