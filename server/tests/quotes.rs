//! Quotes: the `quotes` channel kind and migration 0008.
mod common;

use std::borrow::Cow;
use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::ChannelId;
use pulse_protocol::rest::{Channel, ChannelKind};
use serde_json::json;
use sqlx::ConnectOptions;
use sqlx::sqlite::SqliteConnectOptions;
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello(app: &TestApp, token: &str) -> Socket {
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
        match next_frame(&mut ws).await {
            Some(ServerFrame::Ready(_)) => return ws,
            Some(_) => continue,
            None => panic!("expected Ready"),
        }
    }
}

async fn next_frame(ws: &mut Socket) -> Option<ServerFrame> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        if let Ws::Text(t) = m {
            return Some(serde_json::from_str(&t).unwrap());
        }
    }
}

async fn create_quotes(
    app: &TestApp,
    token: &str,
    server: pulse_protocol::ids::ServerId,
) -> Channel {
    let r = post_json(
        app,
        token,
        &format!("/servers/{server}/channels"),
        json!({ "kind": "quotes", "name": "quotes" }),
    )
    .await;
    assert_eq!(r.status(), 200, "create quotes channel");
    r.json().await.unwrap()
}

#[tokio::test]
async fn creates_a_quotes_channel() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let s = create_server(&app, &alex, "Main").await;
    let r = post_json(&app, &sam, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);

    let mut ws = hello(&app, &alex).await;
    let ch = create_quotes(&app, &alex, s.id).await;
    assert_eq!(ch.kind, ChannelKind::Quotes);
    assert_eq!(ch.name.as_deref(), Some("quotes"));

    let mode: String = sqlx::query_scalar("SELECT reaction_mode FROM channels WHERE id = ?")
        .bind(ch.id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(mode, "one_per_person");

    for user in [alex_id, sam_id] {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM read_states WHERE channel_id = ? AND user_id = ?",
        )
        .bind(ch.id.to_string())
        .bind(user.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
        assert_eq!(n, 1, "{user} has a read point");
    }

    loop {
        match next_frame(&mut ws).await.expect("ChannelCreated") {
            ServerFrame::Event(Event::ChannelCreated { channel }) => {
                assert_eq!(channel, ch);
                break;
            }
            _ => continue,
        }
    }
}

/// A member who joins after the channel exists starts at "all read" there too.
#[tokio::test]
async fn joining_gives_a_read_point_in_quotes_channels() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let s = create_server(&app, &alex, "Main").await;
    let ch = create_quotes(&app, &alex, s.id).await;
    let r = post_json(&app, &sam, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);
    let n: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM read_states WHERE channel_id = ? AND user_id = ?")
            .bind(ch.id.to_string())
            .bind(sam_id.to_string())
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn quotes_channels_have_no_voice() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let s = create_server(&app, &alex, "Main").await;
    let ch = create_quotes(&app, &alex, s.id).await;
    let r = post_json(&app, &alex, &format!("/voice/{}/token", ch.id), json!({})).await;
    assert_eq!(r.status(), 400);
    // ... and a LiveKit join for one is refused too.
    assert!(
        !pulse_server::voice::routes::may_join(&app.db, alex_id, &ch)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn plain_messages_are_refused_in_a_quotes_channel() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let s = create_server(&app, &alex, "Main").await;
    let ch = create_quotes(&app, &alex, s.id).await;
    let path = format!("/channels/{}/messages", ch.id);

    let r = post_json(&app, &alex, &path, json!({ "content": "hi" })).await;
    assert_eq!(r.status(), 400);
    // Threads arrive later: until then any thread_root_id is refused as well.
    let r = post_json(
        &app,
        &alex,
        &path,
        json!({ "content": "hi", "thread_root_id": pulse_protocol::ids::MessageId::new() }),
    )
    .await;
    assert_eq!(r.status(), 400);
    // Not a member of the server: 403, as for a text channel.
    let r = post_json(&app, &sam, &path, json!({ "content": "hi" })).await;
    assert_eq!(r.status(), 403);
}

#[tokio::test]
async fn migration_keeps_every_channel_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();

    // A database as the previous release left it: the first seven migrations only.
    let mut migrator = sqlx::migrate::Migrator::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"),
    )
    .await
    .unwrap();
    assert!(
        migrator.migrations.iter().any(|m| m.version == 8),
        "0008 exists"
    );
    migrator.migrations = Cow::Owned(
        migrator
            .migrations
            .iter()
            .filter(|m| m.version <= 7)
            .cloned()
            .collect(),
    );
    migrator.run(&pool).await.unwrap();

    let ts = "2026-10-01T00:00:00.000Z";
    let dm = "01J000000000000000000000DM";
    let (user, server, channel, voice, message) = (
        "01J0000000000000000000USER",
        "01J000000000000000000SERVER",
        "01J00000000000000000CHANNEL",
        "01J0000000000000000000VOICE",
        "01J00000000000000000MESSAGE",
    );
    for q in [
        format!(
            "INSERT INTO users (id, username, password_hash, created_at) VALUES ('{user}', 'alex', 'x', '{ts}')"
        ),
        format!(
            "INSERT INTO servers (id, name, created_by, created_at) VALUES ('{server}', 'Main', '{user}', '{ts}')"
        ),
        format!(
            "INSERT INTO server_members (server_id, user_id, joined_at) VALUES ('{server}', '{user}', '{ts}')"
        ),
        format!(
            "INSERT INTO channels (id, server_id, kind, name, position, created_at) VALUES ('{channel}', '{server}', 'text', 'general', 0, '{ts}')"
        ),
        format!(
            "INSERT INTO channels (id, server_id, kind, name, position, created_at, reaction_mode) VALUES ('{voice}', '{server}', 'voice', 'Lounge', 0, '{ts}', 'one_per_person')"
        ),
        format!(
            "INSERT INTO messages (id, channel_id, author_id, content, created_at) VALUES ('{message}', '{channel}', '{user}', 'hi', '{ts}')"
        ),
        format!(
            "INSERT INTO reactions (message_id, user_id, emoji, created_at) VALUES ('{message}', '{user}', '👍', '{ts}')"
        ),
        format!(
            "INSERT INTO read_states (user_id, channel_id, last_read_message_id) VALUES ('{user}', '{channel}', '{message}')"
        ),
        format!(
            "INSERT INTO channels (id, server_id, kind, name, position, created_at) VALUES ('{dm}', NULL, 'dm', NULL, 0, '{ts}')"
        ),
        format!(
            "INSERT INTO channel_members (channel_id, user_id, added_at) VALUES ('{dm}', '{user}', '{ts}')"
        ),
    ] {
        sqlx::query(&q)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{q}: {e}"));
    }
    pool.close().await;

    // Upgrade the way the server does at startup.
    let pool = pulse_server::db::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();

    let count = |t: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {t}"))
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(count("channels").await, 3);
    assert_eq!(count("channel_members").await, 1, "the dm kept its member");
    assert_eq!(count("messages").await, 1);
    assert_eq!(count("reactions").await, 1);
    assert_eq!(count("read_states").await, 1);
    assert_eq!(count("server_members").await, 1);
    let modes: Vec<(String, String)> =
        sqlx::query_as("SELECT id, reaction_mode FROM channels ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        modes,
        vec![
            (dm.into(), "normal".into()),
            (voice.into(), "one_per_person".into()),
            (channel.into(), "normal".into())
        ]
    );
    let broken: Vec<(String, i64, String, i64)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(broken.is_empty(), "{broken:?}");
    let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(fk, 1, "the server's own connections have foreign keys on");

    sqlx::query("INSERT INTO channels (id, server_id, kind, name, position, created_at) VALUES (?, ?, 'quotes', 'quotes', 0, ?)")
        .bind(ChannelId::new().to_string())
        .bind(server)
        .bind(ts)
        .execute(&pool)
        .await
        .unwrap();
    // The rebuilt table is still what its children point at: deleting a channel cascades.
    sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(channel)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(count("messages").await, 0, "ON DELETE CASCADE still works");
    assert_eq!(count("read_states").await, 0);
}

/// Run on a connection with foreign keys on, the table rebuild would cascade-delete every
/// message: the migration refuses, and nothing changes.
#[tokio::test]
async fn migration_refuses_to_run_with_foreign_keys_on() {
    let dir = tempfile::tempdir().unwrap();
    let opts = SqliteConnectOptions::new()
        .filename(dir.path().join("old.db"))
        .create_if_missing(true)
        .foreign_keys(true);
    let mut conn = opts.connect().await.unwrap();
    let mut migrator = sqlx::migrate::Migrator::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"),
    )
    .await
    .unwrap();
    let all = migrator.migrations.clone();
    migrator.migrations = Cow::Owned(all.iter().filter(|m| m.version <= 7).cloned().collect());
    migrator.run(&mut conn).await.unwrap();
    let ts = "2026-10-01T00:00:00.000Z";
    for q in [
        format!(
            "INSERT INTO users (id, username, password_hash, created_at) VALUES ('U', 'alex', 'x', '{ts}')"
        ),
        format!(
            "INSERT INTO servers (id, name, created_by, created_at) VALUES ('S', 'Main', 'U', '{ts}')"
        ),
        format!(
            "INSERT INTO channels (id, server_id, kind, name, position, created_at) VALUES ('C', 'S', 'text', 'general', 0, '{ts}')"
        ),
        format!(
            "INSERT INTO messages (id, channel_id, author_id, content, created_at) VALUES ('M', 'C', 'U', 'hi', '{ts}')"
        ),
    ] {
        sqlx::query(&q).execute(&mut conn).await.unwrap();
    }
    migrator.migrations = all;
    let err = migrator.run(&mut conn).await.unwrap_err().to_string();
    assert!(
        err.contains("CHECK constraint failed"),
        "refused by the guard: {err}"
    );
    let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(kept, 1, "nothing was lost");
    let v: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(v, 7, "0008 rolled back");
}

/// An in-memory database lives only while a connection is open: `connect` must hand back the
/// migrated one, not a fresh empty one.
#[tokio::test]
async fn connect_works_for_an_in_memory_database() {
    let pool = pulse_server::db::connect("sqlite::memory:").await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channels")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}
