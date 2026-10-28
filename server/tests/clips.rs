//! Clips: the `clips` channel kind, migration 0009 and the `migrate` subcommand.
mod common;

use std::borrow::Cow;
use std::time::Duration;

use common::*;
use futures::SinkExt;
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::ChannelId;
use pulse_protocol::rest::{Channel, ChannelKind};
use serde_json::json;
use sqlx::sqlite::SqliteConnectOptions;
use tokio_tungstenite::tungstenite::Message as Ws;

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
        match next_frame_beating(&mut ws, Duration::from_secs(3)).await {
            Some(ServerFrame::Ready(_)) => return ws,
            Some(_) => continue,
            None => panic!("expected Ready"),
        }
    }
}

async fn create_clips(
    app: &TestApp,
    token: &str,
    server: pulse_protocol::ids::ServerId,
) -> Channel {
    let r = post_json(
        app,
        token,
        &format!("/servers/{server}/channels"),
        json!({ "kind": "clips", "name": "clips" }),
    )
    .await;
    assert_eq!(r.status(), 200, "create clips channel");
    r.json().await.unwrap()
}

async fn read_points(app: &TestApp, channel: ChannelId, user: pulse_protocol::ids::UserId) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM read_states WHERE channel_id = ? AND user_id = ?")
        .bind(channel.to_string())
        .bind(user.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn creates_a_clips_channel() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let s = create_server(&app, &alex, "Main").await;
    let r = post_json(&app, &sam, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);

    let mut ws = hello(&app, &alex).await;
    let ch = create_clips(&app, &alex, s.id).await;
    assert_eq!(ch.kind, ChannelKind::Clips);
    assert_eq!(ch.name.as_deref(), Some("clips"));

    let mode: String = sqlx::query_scalar("SELECT reaction_mode FROM channels WHERE id = ?")
        .bind(ch.id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(mode, "normal");
    for user in [alex_id, sam_id] {
        assert_eq!(read_points(&app, ch.id, user).await, 1, "{user}");
    }

    loop {
        match next_frame_beating(&mut ws, Duration::from_secs(3))
            .await
            .expect("ChannelCreated")
        {
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
async fn joining_gives_a_read_point_in_clips_channels() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let s = create_server(&app, &alex, "Main").await;
    let ch = create_clips(&app, &alex, s.id).await;
    let r = post_json(&app, &sam, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);
    assert_eq!(read_points(&app, ch.id, sam_id).await, 1);
}

#[tokio::test]
async fn clips_channels_have_no_voice() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let s = create_server(&app, &alex, "Main").await;
    let ch = create_clips(&app, &alex, s.id).await;
    let r = post_json(&app, &alex, &format!("/voice/{}/token", ch.id), json!({})).await;
    assert_eq!(r.status(), 400);
    assert!(
        !pulse_server::voice::routes::may_join(&app.db, alex_id, &ch)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn plain_messages_are_refused_in_a_clips_channel() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let s = create_server(&app, &alex, "Main").await;
    let ch = create_clips(&app, &alex, s.id).await;
    let path = format!("/channels/{}/messages", ch.id);

    let r = post_json(&app, &alex, &path, json!({ "content": "hi" })).await;
    assert_eq!(r.status(), 400);
    assert!(
        r.text()
            .await
            .unwrap()
            .contains("only clips can be posted here")
    );
    // Replies arrive later: until then any thread_root_id is refused as well.
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
        // 0008 and 0009 refuse to run with foreign keys on; the server migrates with them off.
        .foreign_keys(false);
    let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();

    // A database as the previous release left it: migrations up to 0008 only.
    let mut migrator = sqlx::migrate::Migrator::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"),
    )
    .await
    .unwrap();
    assert!(
        migrator.migrations.iter().any(|m| m.version == 9),
        "0009 exists"
    );
    migrator.migrations = Cow::Owned(
        migrator
            .migrations
            .iter()
            .filter(|m| m.version <= 8)
            .cloned()
            .collect(),
    );
    migrator.run(&pool).await.unwrap();

    let ts = "2026-10-01T00:00:00.000Z";
    let (user, server, text, quotes, message, quote_msg) = (
        "01J0000000000000000000USER",
        "01J000000000000000000SERVER",
        "01J00000000000000000CHANNEL",
        "01J0000000000000000000QUOTE",
        "01J00000000000000000MESSAGE",
        "01J000000000000000000QUOTEM",
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
            "INSERT INTO channels (id, server_id, kind, name, position, created_at) VALUES ('{text}', '{server}', 'text', 'general', 0, '{ts}')"
        ),
        format!(
            "INSERT INTO channels (id, server_id, kind, name, position, created_at, reaction_mode) VALUES ('{quotes}', '{server}', 'quotes', 'quotes', 1, '{ts}', 'one_per_person')"
        ),
        format!(
            "INSERT INTO messages (id, channel_id, author_id, content, created_at) VALUES ('{message}', '{text}', '{user}', 'hi', '{ts}')"
        ),
        format!(
            "INSERT INTO messages (id, channel_id, author_id, content, created_at) VALUES ('{quote_msg}', '{quotes}', '{user}', '', '{ts}')"
        ),
        format!(
            "INSERT INTO quotes (message_id, posted_on, year) VALUES ('{quote_msg}', '2026-10-01', 2026)"
        ),
        format!(
            "INSERT INTO reactions (message_id, user_id, emoji, created_at) VALUES ('{message}', '{user}', '👍', '{ts}')"
        ),
        format!(
            "INSERT INTO read_states (user_id, channel_id, last_read_message_id) VALUES ('{user}', '{text}', '{message}')"
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
    assert_eq!(count("channels").await, 2);
    assert_eq!(count("messages").await, 2);
    assert_eq!(count("reactions").await, 1);
    assert_eq!(count("quotes").await, 1);
    assert_eq!(count("read_states").await, 1);
    assert_eq!(count("server_members").await, 1);
    let broken: Vec<(String, i64, String, i64)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(broken.is_empty(), "{broken:?}");

    sqlx::query("INSERT INTO channels (id, server_id, kind, name, position, created_at) VALUES (?, ?, 'clips', 'clips', 2, ?)")
        .bind(ChannelId::new().to_string())
        .bind(server)
        .bind(ts)
        .execute(&pool)
        .await
        .unwrap();
    // The new tables exist.
    for t in ["games", "clips", "clip_uploads", "clip_view_prefs"] {
        assert_eq!(count(t).await, 0, "{t}");
    }
    // The rebuilt table is still what its children point at: deleting a channel cascades.
    sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(text)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(count("reactions").await, 0, "ON DELETE CASCADE still works");
    assert_eq!(count("read_states").await, 0);
}

#[tokio::test]
async fn migrate_subcommand_migrates_and_exits() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("m.db").display());
    // Only PULSE_DB_URL is needed: no LiveKit variables.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_pulse-app-server"))
        .env_clear()
        .env("PULSE_DB_URL", &url)
        .env("RUST_LOG", "warn")
        .arg("migrate")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.lines().any(|l| l == "migrated: 9"), "{stdout}");
}
