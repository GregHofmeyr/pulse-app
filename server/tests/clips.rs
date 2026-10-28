//! Clips: the `clips` channel kind, migration 0009 and the `migrate` subcommand.
mod common;

use std::borrow::Cow;
use std::time::Duration;

use common::*;
use futures::SinkExt;
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::ChannelId;
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{Channel, ChannelKind, ClipUpload, MediaInfo, Message};
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
    // A thread_root_id that is not a live clip here is refused as well.
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
    let (other, voice, dm) = (
        "01J000000000000000000OTHER",
        "01J00000000000000000VOICE0",
        "01J0000000000000000000DM00",
    );
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
            "INSERT INTO users (id, username, password_hash, created_at) VALUES ('{other}', 'sam', 'x', '{ts}')"
        ),
        format!(
            "INSERT INTO channels (id, server_id, kind, name, position, created_at) VALUES ('{voice}', '{server}', 'voice', 'lounge', 2, '{ts}')"
        ),
        format!(
            "INSERT INTO channels (id, server_id, kind, name, position, created_at) VALUES ('{dm}', NULL, 'dm', NULL, 0, '{ts}')"
        ),
        format!(
            "INSERT INTO channel_members (channel_id, user_id, added_at) VALUES ('{dm}', '{user}', '{ts}')"
        ),
        format!(
            "INSERT INTO channel_members (channel_id, user_id, added_at) VALUES ('{dm}', '{other}', '{ts}')"
        ),
        format!(
            "INSERT INTO dm_hidden (user_id, channel_id, hidden_at) VALUES ('{user}', '{dm}', '{ts}')"
        ),
        format!(
            "INSERT INTO voice_sessions (id, user_id, channel_id, joined_at) VALUES ('VS1', '{user}', '{voice}', '{ts}')"
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
    assert_eq!(count("channels").await, 4);
    assert_eq!(count("channel_members").await, 2);
    assert_eq!(count("dm_hidden").await, 1);
    assert_eq!(count("voice_sessions").await, 1);
    // Copied values survive, and the rebuilt table keeps its index.
    let (mode, position): (String, i64) =
        sqlx::query_as("SELECT reaction_mode, position FROM channels WHERE id = ?")
            .bind(quotes)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((mode.as_str(), position), ("one_per_person", 1));
    let index: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'channels_server'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(index, 1);
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
async fn migration_0009_refuses_to_run_with_foreign_keys_on() {
    use sqlx::{ConnectOptions, Connection};
    let dir = tempfile::tempdir().unwrap();
    // 0008 refuses foreign keys too, so bring the database to 8 with them off first.
    let opts = SqliteConnectOptions::new()
        .filename(dir.path().join("old.db"))
        .create_if_missing(true)
        .foreign_keys(false);
    let mut conn = opts.connect().await.unwrap();
    let mut migrator = sqlx::migrate::Migrator::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"),
    )
    .await
    .unwrap();
    let all = migrator.migrations.clone();
    migrator.migrations = Cow::Owned(all.iter().filter(|m| m.version <= 8).cloned().collect());
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
    conn.close().await.unwrap();
    let mut conn = opts.foreign_keys(true).connect().await.unwrap();
    migrator.migrations = all;
    let err = migrator.run(&mut conn).await.unwrap_err().to_string();
    assert!(
        err.contains("CHECK constraint failed"),
        "refused by the guard: {err}"
    );
    let v: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(v, 8, "0009 rolled back");
    for (table, want) in [("channels", 1), ("messages", 1)] {
        let n: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&mut conn)
            .await
            .unwrap();
        assert_eq!(n, want, "{table} unchanged");
    }
}

fn run_migrate(url: &str) -> std::process::Output {
    // Only PULSE_DB_URL is needed: no LiveKit variables.
    std::process::Command::new(env!("CARGO_BIN_EXE_pulse-app-server"))
        .env_clear()
        .env("PULSE_DB_URL", url)
        .env("RUST_LOG", "warn")
        .arg("migrate")
        .output()
        .unwrap()
}

#[tokio::test]
async fn migrate_subcommand_migrates_and_exits() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("m.db");
    let url = format!("sqlite://{}", file.display());
    // `migrate` upgrades a database that exists.
    let pool = sqlx::SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&file)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    pool.close().await;
    let out = run_migrate(&url);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.lines().any(|l| l == "migrated: 9"), "{stdout}");
    assert!(stdout.contains(&url), "names the database: {stdout}");
}

#[tokio::test]
async fn migrate_refuses_a_missing_database() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("nope.db");
    let url = format!("sqlite://{}", file.display());
    let out = run_migrate(&url);
    assert!(!out.status.success(), "{out:?}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains(&url), "{stderr}");
    assert!(!file.exists(), "migrate must not create a database");
}

// ---- Games registry ----

async fn add_game(
    app: &TestApp,
    token: &str,
    server: pulse_protocol::ids::ServerId,
    name: &str,
) -> reqwest::Response {
    post_json(
        app,
        token,
        &format!("/servers/{server}/games"),
        json!({ "name": name }),
    )
    .await
}

async fn add_game_ok(
    app: &TestApp,
    token: &str,
    server: pulse_protocol::ids::ServerId,
    name: &str,
) -> pulse_protocol::rest::Game {
    let r = add_game(app, token, server, name).await;
    assert_eq!(r.status(), 200, "add {name:?}");
    r.json().await.unwrap()
}

async fn ready_of(app: &TestApp, token: &str) -> pulse_protocol::gateway::Ready {
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

async fn game_rows(app: &TestApp, server: pulse_protocol::ids::ServerId) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM games WHERE server_id = ?")
        .bind(server.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn adding_a_game_is_case_and_space_insensitive() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let s = create_server(&app, &alex, "Main").await;
    let r = post_json(&app, &sam, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);
    let mut ws = hello(&app, &sam).await;

    let g = add_game_ok(&app, &alex, s.id, "Overwatch 2").await;
    assert_eq!(g.name, "Overwatch 2");
    assert_eq!(g.server_id, s.id);
    assert_eq!(g.created_by, Some(alex_id));
    let again = add_game_ok(&app, &sam, s.id, " overwatch  2 ").await;
    assert_eq!(again, g, "the existing game comes back as it was stored");
    assert_eq!(game_rows(&app, s.id).await, 1);

    let e = add_game_ok(&app, &alex, s.id, "ÉCLAIR").await;
    let e2 = add_game_ok(&app, &alex, s.id, "éclair").await;
    assert_eq!(e2.id, e.id);
    assert_eq!(e.name, "ÉCLAIR");

    // Exactly one GameAdded per new game, none for the repeats.
    let mut added = vec![];
    while let Some(f) = next_frame_beating(&mut ws, Duration::from_millis(500)).await {
        if let ServerFrame::Event(Event::GameAdded { game }) = f {
            added.push(game);
        }
    }
    assert_eq!(added, vec![g, e]);
}

#[tokio::test]
async fn game_names_are_checked() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let s = create_server(&app, &alex, "Main").await;
    for bad in ["", "   ", &"x".repeat(41)] {
        assert_eq!(
            add_game(&app, &alex, s.id, bad).await.status(),
            400,
            "{bad:?}"
        );
    }
    // Characters, not bytes.
    let ok = "é".repeat(40);
    assert_eq!(add_game_ok(&app, &alex, s.id, &ok).await.name, ok);
    assert_eq!(
        add_game(&app, &alex, s.id, &"é".repeat(41)).await.status(),
        400
    );
    assert_eq!(
        add_game(&app, &alex, s.id, &"x".repeat(40)).await.status(),
        200
    );
    assert_eq!(game_rows(&app, s.id).await, 2);
}

#[tokio::test]
async fn non_members_cant_add_games() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, kim) = register(&app, "kim").await;
    let s = create_server(&app, &alex, "Main").await;
    assert_eq!(
        add_game(&app, &kim, s.id, "Overwatch 2").await.status(),
        403
    );
    assert_eq!(game_rows(&app, s.id).await, 0);
}

#[tokio::test]
async fn games_are_in_ready() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, jo) = register(&app, "jo").await;
    let s = create_server(&app, &alex, "Main").await;
    let other = create_server(&app, &jo, "Other").await;
    let b = add_game_ok(&app, &alex, s.id, "Zelda").await;
    let a = add_game_ok(&app, &alex, s.id, "Apex").await;
    let o = add_game_ok(&app, &jo, other.id, "Apex").await;
    assert_ne!(o.id, a.id, "the key is per server");

    let ready = ready_of(&app, &alex).await;
    let names: Vec<_> = ready
        .games
        .iter()
        .filter(|g| g.server_id == s.id)
        .map(|g| g.name.as_str())
        .collect();
    assert_eq!(names, ["Apex", "Zelda"]);
    assert!(ready.games.contains(&b));

    use pulse_server::clips::games;
    assert!(games::in_server(&app.db, s.id, a.id).await.unwrap());
    assert!(!games::in_server(&app.db, s.id, o.id).await.unwrap());
    assert!(
        !games::in_server(&app.db, s.id, pulse_protocol::ids::GameId::new())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn games_are_capped() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let s = create_server(&app, &alex, "Main").await;
    for i in 1..=300 {
        sqlx::query(
            "INSERT INTO games (id, server_id, name, name_key, created_by, created_at)
             VALUES (?, ?, ?, ?, NULL, '2026-01-01T00:00:00Z')",
        )
        .bind(pulse_protocol::ids::GameId::new().to_string())
        .bind(s.id.to_string())
        .bind(format!("Game {i}"))
        .bind(format!("game {i}"))
        .execute(&app.db)
        .await
        .unwrap();
    }
    let r = add_game(&app, &alex, s.id, "One more").await;
    assert_eq!(r.status(), 400);
    let e: pulse_protocol::rest::ApiError = r.json().await.unwrap();
    assert_eq!(e.message, "This server has too many games.");
    // An existing name is still answered at the cap, and nothing is added.
    assert_eq!(
        add_game_ok(&app, &alex, s.id, "GAME 7").await.name,
        "Game 7"
    );
    assert_eq!(game_rows(&app, s.id).await, 300);
}

// ---- Posting clips: tickets, confirm, attach ----

const MAX_BYTES: u64 = 200 * 1024 * 1024;

/// A decodable image no other call produces (the media store dedups by content hash and keeps
/// the first uploader, so a shared image would belong to whoever uploaded it first).
fn unique_png() -> Vec<u8> {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let px = image::Rgba([
        (n * 53 % 251) as u8,
        (n * 97 % 241) as u8,
        (n * 29 % 239) as u8,
        255,
    ]);
    let mut img = image::RgbaImage::from_pixel(64, 36, px);
    // Mix the counter into the pixels so two calls never share a colour at all.
    img.put_pixel(n % 64, n / 64 % 36, image::Rgba([n as u8, 1, 2, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

async fn upload_thumb(app: &TestApp, token: &str) -> String {
    let r = app
        .http
        .post(app.url("/media/clip-thumb"))
        .bearer_auth(token)
        .header("content-type", "application/octet-stream")
        .body(unique_png())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "thumb upload");
    r.json::<MediaInfo>().await.unwrap().hash
}

async fn ticket(
    app: &TestApp,
    token: &str,
    channel: ChannelId,
    size: u64,
    duration_ms: u32,
) -> reqwest::Response {
    post_json(
        app,
        token,
        &format!("/channels/{channel}/clips/uploads"),
        json!({ "size": size, "duration_ms": duration_ms }),
    )
    .await
}

async fn ticket_ok(app: &TestApp, token: &str, channel: ChannelId, size: u64) -> ClipUpload {
    let r = ticket(app, token, channel, size, 30_000).await;
    assert_eq!(r.status(), 200, "ticket");
    r.json().await.unwrap()
}

async fn put_object(app: &TestApp, up: &ClipUpload, bytes: Vec<u8>) {
    let r = app
        .http
        .put(&up.url)
        .header("content-type", &up.content_type)
        .body(bytes)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "PUT {}", r.status());
}

async fn ticket_key(app: &TestApp, up: &ClipUpload) -> String {
    sqlx::query_scalar("SELECT key FROM clip_uploads WHERE id = ?")
        .bind(up.id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn tickets(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM clip_uploads")
        .fetch_one(&app.db)
        .await
        .unwrap()
}

fn confirm_body(thumb: &str, nonce: Option<&str>) -> serde_json::Value {
    json!({
        "caption": "  nice one  ",
        "game_id": null,
        "duration_ms": 30_000,
        "width": 1920,
        "height": 1080,
        "thumb_hash": thumb,
        "nonce": nonce,
    })
}

async fn confirm(
    app: &TestApp,
    token: &str,
    up: &ClipUpload,
    body: serde_json::Value,
) -> reqwest::Response {
    post_json(
        app,
        token,
        &format!("/clip-uploads/{}/confirm", up.id),
        body,
    )
    .await
}

/// Ticket, PUT, thumbnail, confirm: one posted clip.
async fn upload_clip(app: &TestApp, token: &str, channel: ChannelId, bytes: Vec<u8>) -> Message {
    let up = ticket_ok(app, token, channel, bytes.len() as u64).await;
    put_object(app, &up, bytes).await;
    let thumb = upload_thumb(app, token).await;
    let r = confirm(app, token, &up, confirm_body(&thumb, None)).await;
    assert_eq!(r.status(), 200, "confirm");
    r.json().await.unwrap()
}

struct World {
    app: TestApp,
    alex: (UserId, String),
    sam: (UserId, String),
    server: ServerId,
    channel: Channel,
}

async fn world() -> World {
    let app = spawn().await;
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let s = create_server(&app, &alex.1, "Main").await;
    let r = post_json(&app, &sam.1, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);
    let channel = create_clips(&app, &alex.1, s.id).await;
    World {
        app,
        alex,
        sam,
        server: s.id,
        channel,
    }
}

async fn error_message(r: reqwest::Response) -> String {
    r.json::<pulse_protocol::rest::ApiError>()
        .await
        .unwrap()
        .message
}

#[tokio::test]
async fn posts_a_clip() {
    let w = world().await;
    let mut ws = hello(&w.app, &w.sam.1).await;
    let bytes = vec![7u8; 1_000];
    let up = ticket_ok(&w.app, &w.alex.1, w.channel.id, 1_000).await;
    assert_eq!(up.content_type, "video/mp4");
    put_object(&w.app, &up, bytes.clone()).await;
    let key = ticket_key(&w.app, &up).await;
    let (stored, _) = w
        .app
        .s3
        .as_ref()
        .unwrap()
        .object(&format!("clips/{key}.mp4"))
        .expect("object");
    assert_eq!(stored, bytes);

    let thumb = upload_thumb(&w.app, &w.alex.1).await;
    let r = confirm(&w.app, &w.alex.1, &up, confirm_body(&thumb, Some("n-1"))).await;
    assert_eq!(r.status(), 200);
    let m: Message = r.json().await.unwrap();
    assert_eq!(m.content, "nice one");
    assert_eq!(m.author_id, Some(w.alex.0));
    let clip = m.clip.clone().expect("clip");
    assert_eq!(
        (
            clip.size,
            clip.duration_ms,
            clip.width,
            clip.height,
            clip.thumb_hash.as_str()
        ),
        (1_000, 30_000, 1920, 1080, thumb.as_str())
    );
    assert_eq!(
        (clip.reply_count, clip.last_reply_id, clip.replies_seen),
        (0, None, None)
    );
    assert_eq!(tickets(&w.app).await, 0, "the ticket is spent");
    assert_eq!(read_points(&w.app, w.channel.id, w.alex.0).await, 1);
    let (kind, location, size): (String, String, i64) =
        sqlx::query_as("SELECT kind, location, size FROM media WHERE hash = ?")
            .bind(&key)
            .fetch_one(&w.app.db)
            .await
            .unwrap();
    assert_eq!(
        (kind.as_str(), location.as_str(), size),
        ("clip", "r2", 1_000)
    );

    let list: Vec<Message> = get_json(
        &w.app,
        &w.sam.1,
        &format!("/channels/{}/messages", w.channel.id),
    )
    .await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].clip, m.clip);

    loop {
        match next_frame_beating(&mut ws, Duration::from_secs(3))
            .await
            .expect("MessageCreated")
        {
            ServerFrame::Event(Event::MessageCreated { message, nonce }) => {
                assert_eq!(message.id, m.id);
                assert_eq!(message.clip, m.clip);
                assert_eq!(nonce.as_deref(), Some("n-1"));
                break;
            }
            _ => continue,
        }
    }
}

#[tokio::test]
async fn tickets_check_the_rules() {
    let w = world().await;
    let text = general(&w.app, &w.alex.1, w.server).await;
    let r = ticket(&w.app, &w.alex.1, text.id, 1_000, 1_000).await;
    assert_eq!(r.status(), 400, "a text channel");
    let outsider = register(&w.app, "kim").await;
    let r = ticket(&w.app, &outsider.1, w.channel.id, 1_000, 1_000).await;
    assert_eq!(r.status(), 403, "not a member");
    let r = ticket(&w.app, &w.alex.1, w.channel.id, 0, 1_000).await;
    assert_eq!(r.status(), 400, "empty");
    let r = ticket(&w.app, &w.alex.1, w.channel.id, MAX_BYTES + 1, 1_000).await;
    assert_eq!(r.status(), 413);
    assert_eq!(error_message(r).await, "Clips can be up to 200 MB.");
    let r = ticket(&w.app, &w.alex.1, w.channel.id, MAX_BYTES, 1_000).await;
    assert_eq!(r.status(), 200, "exactly the cap");
    let r = ticket(&w.app, &w.alex.1, w.channel.id, 1_000, 90_001).await;
    assert_eq!(r.status(), 400);
    assert_eq!(error_message(r).await, "Clips can be up to 1:30.");
    let r = ticket(&w.app, &w.alex.1, w.channel.id, 1_000, 0).await;
    assert_eq!(r.status(), 400, "no duration");
    let r = ticket(&w.app, &w.alex.1, w.channel.id, 1_000, 90_000).await;
    assert_eq!(r.status(), 200, "exactly 1:30");

    let bare = spawn_without_clips().await;
    let (_, token) = register(&bare, "alex").await;
    let s = create_server(&bare, &token, "Main").await;
    let ch = create_clips(&bare, &token, s.id).await;
    let r = ticket(&bare, &token, ch.id, 1_000, 1_000).await;
    assert_eq!(r.status(), 503);
    assert_eq!(
        error_message(r).await,
        "Clips storage isn't set up on this server."
    );
}

#[tokio::test]
async fn confirm_checks_the_object() {
    let w = world().await;
    let s3 = w.app.s3.as_ref().unwrap();
    let thumb = upload_thumb(&w.app, &w.alex.1).await;

    // Nothing arrived yet.
    let up = ticket_ok(&w.app, &w.alex.1, w.channel.id, 1_000).await;
    let r = confirm(&w.app, &w.alex.1, &up, confirm_body(&thumb, None)).await;
    assert_eq!(r.status(), 409);
    assert_eq!(
        error_message(r).await,
        "The upload didn't arrive. Try again."
    );

    // The wrong size: the object and the ticket both go.
    let key = ticket_key(&w.app, &up).await;
    let object = format!("clips/{key}.mp4");
    s3.put_raw(&object, vec![1u8; 999]);
    let r = confirm(&w.app, &w.alex.1, &up, confirm_body(&thumb, None)).await;
    assert_eq!(r.status(), 400);
    assert!(s3.object(&object).is_none());
    assert_eq!(tickets(&w.app).await, 0);

    // A good upload; every bad confirm leaves the ticket for a corrected try.
    let up = ticket_ok(&w.app, &w.alex.1, w.channel.id, 1_000).await;
    put_object(&w.app, &up, vec![2u8; 1_000]).await;
    let r = confirm(&w.app, &w.sam.1, &up, confirm_body(&thumb, None)).await;
    assert_eq!(r.status(), 404, "someone else's ticket");

    let sams_thumb = upload_thumb(&w.app, &w.sam.1).await;
    let other = create_server(&w.app, &w.sam.1, "Other").await;
    let foreign_game = add_game_ok(&w.app, &w.sam.1, other.id, "Elsewhere").await;
    let bad = |f: &dyn Fn(&mut serde_json::Value)| {
        let mut b = confirm_body(&thumb, None);
        f(&mut b);
        b
    };
    let cases = [
        ("duration", bad(&|b| b["duration_ms"] = json!(90_501))),
        ("width", bad(&|b| b["width"] = json!(8))),
        ("height", bad(&|b| b["height"] = json!(7681))),
        ("thumb", bad(&|b| b["thumb_hash"] = json!(sams_thumb))),
        ("no such thumb", bad(&|b| b["thumb_hash"] = json!("nothex"))),
        ("game", bad(&|b| b["game_id"] = json!(foreign_game.id))),
        ("caption", bad(&|b| b["caption"] = json!("x".repeat(301)))),
    ];
    for (what, body) in cases {
        let r = confirm(&w.app, &w.alex.1, &up, body).await;
        assert_eq!(r.status(), 400, "{what}");
    }
    assert_eq!(tickets(&w.app).await, 1);
    let mut ok = confirm_body(&thumb, None);
    ok["duration_ms"] = json!(90_500);
    ok["caption"] = json!("x".repeat(300));
    let r = confirm(&w.app, &w.alex.1, &up, ok).await;
    assert_eq!(r.status(), 200, "the limits themselves are fine");
}

#[tokio::test]
async fn confirm_rechecks_membership() {
    let w = world().await;
    let up = ticket_ok(&w.app, &w.sam.1, w.channel.id, 1_000).await;
    put_object(&w.app, &up, vec![3u8; 1_000]).await;
    let thumb = upload_thumb(&w.app, &w.sam.1).await;
    sqlx::query("DELETE FROM server_members WHERE user_id = ?")
        .bind(w.sam.0.to_string())
        .execute(&w.app.db)
        .await
        .unwrap();
    let r = confirm(&w.app, &w.sam.1, &up, confirm_body(&thumb, None)).await;
    assert_eq!(r.status(), 403);
}

#[tokio::test]
async fn the_twenty_first_clip_in_an_hour_waits() {
    let w = world().await;
    let mut ups = vec![];
    for _ in 0..20 {
        ups.push(ticket_ok(&w.app, &w.alex.1, w.channel.id, 1_000).await);
    }
    let r = ticket(&w.app, &w.alex.1, w.channel.id, 1_000, 1_000).await;
    assert_eq!(r.status(), 429);
    assert_eq!(
        error_message(r).await,
        "That's 20 clips this hour. Try again later."
    );
    // Another person has their own allowance.
    assert_eq!(
        ticket(&w.app, &w.sam.1, w.channel.id, 1_000, 1_000)
            .await
            .status(),
        200
    );

    let r = w
        .app
        .http
        .delete(w.app.url(&format!("/clip-uploads/{}", ups[0].id)))
        .bearer_auth(&w.alex.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(
        ticket(&w.app, &w.alex.1, w.channel.id, 1_000, 1_000)
            .await
            .status(),
        200
    );
    assert_eq!(
        ticket(&w.app, &w.alex.1, w.channel.id, 1_000, 1_000)
            .await
            .status(),
        429
    );

    let old = pulse_server::db::stamp(chrono::Utc::now() - chrono::Duration::minutes(61));
    sqlx::query("UPDATE clip_uploads SET created_at = ?")
        .bind(old)
        .execute(&w.app.db)
        .await
        .unwrap();
    assert_eq!(
        ticket(&w.app, &w.alex.1, w.channel.id, 1_000, 1_000)
            .await
            .status(),
        200
    );
}

/// Posted clips count too, deleted or not, and so do they when their tickets are gone.
#[tokio::test]
async fn posted_clips_count_toward_the_hour() {
    let w = world().await;
    for _ in 0..19 {
        upload_clip(&w.app, &w.alex.1, w.channel.id, vec![5u8; 10]).await;
    }
    let last = upload_clip(&w.app, &w.alex.1, w.channel.id, vec![5u8; 10]).await;
    assert_eq!(
        ticket(&w.app, &w.alex.1, w.channel.id, 10, 1_000)
            .await
            .status(),
        429
    );
    // Deleting the clip (rows and all) doesn't free the slot.
    sqlx::query("UPDATE messages SET deleted_at = ?, content = '' WHERE id = ?")
        .bind(pulse_server::db::now())
        .bind(last.id.to_string())
        .execute(&w.app.db)
        .await
        .unwrap();
    sqlx::query("DELETE FROM clips WHERE message_id = ?")
        .bind(last.id.to_string())
        .execute(&w.app.db)
        .await
        .unwrap();
    assert_eq!(
        ticket(&w.app, &w.alex.1, w.channel.id, 10, 1_000)
            .await
            .status(),
        429
    );
}

#[tokio::test]
async fn a_repeated_confirm_returns_the_same_clip() {
    let w = world().await;
    let mut ws = hello(&w.app, &w.sam.1).await;
    let up = ticket_ok(&w.app, &w.alex.1, w.channel.id, 1_000).await;
    put_object(&w.app, &up, vec![4u8; 1_000]).await;
    let thumb = upload_thumb(&w.app, &w.alex.1).await;
    let first: Message = confirm(&w.app, &w.alex.1, &up, confirm_body(&thumb, Some("same")))
        .await
        .json()
        .await
        .unwrap();
    // The ticket is gone by now; the nonce still answers.
    let r = confirm(&w.app, &w.alex.1, &up, confirm_body(&thumb, Some("same"))).await;
    assert_eq!(r.status(), 200);
    let second: Message = r.json().await.unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(second.clip, first.clip);

    let mut created = 0;
    while let Some(f) = next_frame_beating(&mut ws, Duration::from_millis(600)).await {
        if matches!(f, ServerFrame::Event(Event::MessageCreated { .. })) {
            created += 1;
        }
    }
    assert_eq!(created, 1);
}

#[tokio::test]
async fn cancel_deletes_ticket_and_object() {
    let w = world().await;
    let s3 = w.app.s3.as_ref().unwrap();
    let up = ticket_ok(&w.app, &w.alex.1, w.channel.id, 1_000).await;
    put_object(&w.app, &up, vec![9u8; 1_000]).await;
    let object = format!("clips/{}.mp4", ticket_key(&w.app, &up).await);
    assert!(s3.object(&object).is_some());

    let url = w.app.url(&format!("/clip-uploads/{}", up.id));
    let r = w
        .app
        .http
        .delete(&url)
        .bearer_auth(&w.sam.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404, "only the owner cancels");
    assert!(s3.object(&object).is_some());
    let r = w
        .app
        .http
        .delete(&url)
        .bearer_auth(&w.alex.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(tickets(&w.app).await, 0);
    assert!(s3.object(&object).is_none());
}

#[tokio::test]
async fn a_nonce_of_a_plain_message_is_not_a_repeated_confirm() {
    let w = world().await;
    let text = general(&w.app, &w.alex.1, w.server).await;
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/channels/{}/messages", text.id),
        json!({ "content": "hi", "nonce": "taken" }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let up = ticket_ok(&w.app, &w.alex.1, w.channel.id, 1_000).await;
    put_object(&w.app, &up, vec![6u8; 1_000]).await;
    let thumb = upload_thumb(&w.app, &w.alex.1).await;
    let r = confirm(&w.app, &w.alex.1, &up, confirm_body(&thumb, Some("taken"))).await;
    assert_eq!(r.status(), 409);
    assert_eq!(tickets(&w.app).await, 1, "the ticket is untouched");
}

#[tokio::test]
async fn confirm_needs_a_clips_channel() {
    let w = world().await;
    let text = general(&w.app, &w.alex.1, w.server).await;
    let up = ticket_ok(&w.app, &w.alex.1, w.channel.id, 1_000).await;
    put_object(&w.app, &up, vec![8u8; 1_000]).await;
    sqlx::query("UPDATE clip_uploads SET channel_id = ?")
        .bind(text.id.to_string())
        .execute(&w.app.db)
        .await
        .unwrap();
    let thumb = upload_thumb(&w.app, &w.alex.1).await;
    let r = confirm(&w.app, &w.alex.1, &up, confirm_body(&thumb, None)).await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn two_confirms_at_once_post_one_clip() {
    let w = world().await;
    let mut ws = hello(&w.app, &w.sam.1).await;
    let up = ticket_ok(&w.app, &w.alex.1, w.channel.id, 1_000).await;
    put_object(&w.app, &up, vec![1u8; 1_000]).await;
    let thumb = upload_thumb(&w.app, &w.alex.1).await;
    let (a, b) = tokio::join!(
        confirm(&w.app, &w.alex.1, &up, confirm_body(&thumb, Some("race"))),
        confirm(&w.app, &w.alex.1, &up, confirm_body(&thumb, Some("race"))),
    );
    assert_eq!((a.status().as_u16(), b.status().as_u16()), (200, 200));
    let (a, b): (Message, Message) = (a.json().await.unwrap(), b.json().await.unwrap());
    assert_eq!(a.id, b.id);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM clips")
        .fetch_one(&w.app.db)
        .await
        .unwrap();
    assert_eq!(rows, 1);
    let mut created = 0;
    while let Some(f) = next_frame_beating(&mut ws, Duration::from_millis(600)).await {
        if matches!(f, ServerFrame::Event(Event::MessageCreated { .. })) {
            created += 1;
        }
    }
    assert_eq!(created, 1);
}

// ---- Editing, deleting, sweeping ----

async fn put_clip(
    app: &TestApp,
    token: &str,
    id: pulse_protocol::ids::MessageId,
    body: serde_json::Value,
) -> reqwest::Response {
    app.http
        .put(app.url(&format!("/clips/{id}")))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

async fn delete_message(
    app: &TestApp,
    token: &str,
    id: pulse_protocol::ids::MessageId,
) -> reqwest::Response {
    app.http
        .delete(app.url(&format!("/messages/{id}")))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
}

async fn text(app: &TestApp, sql: &str, bind: &str) -> String {
    sqlx::query_scalar(sql)
        .bind(bind)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn count(app: &TestApp, sql: &str, bind: &str) -> i64 {
    sqlx::query_scalar(sql)
        .bind(bind)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn only_the_poster_edits() {
    let w = world().await;
    let clip = upload_clip(&w.app, &w.alex.1, w.channel.id, vec![1u8; 10]).await;
    let game = add_game_ok(&w.app, &w.alex.1, w.server, "Rocket League").await;
    let mut ws = hello(&w.app, &w.sam.1).await;

    let body = json!({ "caption": "  new words ", "game_id": game.id });
    assert_eq!(
        put_clip(&w.app, &w.sam.1, clip.id, body.clone())
            .await
            .status(),
        403
    );

    let r = put_clip(&w.app, &w.alex.1, clip.id, body).await;
    assert_eq!(r.status(), 200);
    let m: Message = r.json().await.unwrap();
    assert_eq!(m.content, "new words");
    assert!(m.edited_at.is_some());
    let c = m.clip.clone().expect("clip");
    assert_eq!(c.game_id, Some(game.id));
    assert_eq!(c.thumb_hash, clip.clip.as_ref().unwrap().thumb_hash);

    loop {
        match next_frame_beating(&mut ws, Duration::from_secs(3))
            .await
            .expect("MessageUpdated")
        {
            ServerFrame::Event(Event::MessageUpdated { message }) => {
                assert_eq!(message.id, clip.id);
                assert_eq!(message.content, "new words");
                assert_eq!(message.clip, m.clip);
                break;
            }
            _ => continue,
        }
    }

    // The same checks as confirm: caption length and a game from this server; a game can be cleared.
    let long = "x".repeat(301);
    assert_eq!(
        put_clip(
            &w.app,
            &w.alex.1,
            clip.id,
            json!({"caption": long, "game_id": null})
        )
        .await
        .status(),
        400
    );
    let other = create_server(&w.app, &w.alex.1, "Other").await;
    let foreign = add_game_ok(&w.app, &w.alex.1, other.id, "Elsewhere").await;
    assert_eq!(
        put_clip(
            &w.app,
            &w.alex.1,
            clip.id,
            json!({"caption": "ok", "game_id": foreign.id})
        )
        .await
        .status(),
        400
    );
    let r = put_clip(
        &w.app,
        &w.alex.1,
        clip.id,
        json!({"caption": "ok", "game_id": null}),
    )
    .await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<Message>().await.unwrap().clip.unwrap().game_id,
        None
    );

    // Not a clip, and a deleted clip, are 404s.
    let text_channel = general(&w.app, &w.alex.1, w.server).await;
    let plain: Message = post_json(
        &w.app,
        &w.alex.1,
        &format!("/channels/{}/messages", text_channel.id),
        json!({"content": "hi"}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(
        put_clip(
            &w.app,
            &w.alex.1,
            plain.id,
            json!({"caption": "x", "game_id": null})
        )
        .await
        .status(),
        404
    );
    assert_eq!(
        delete_message(&w.app, &w.alex.1, clip.id).await.status(),
        204
    );
    assert_eq!(
        put_clip(
            &w.app,
            &w.alex.1,
            clip.id,
            json!({"caption": "x", "game_id": null})
        )
        .await
        .status(),
        404
    );
}

#[tokio::test]
async fn clips_cant_be_edited_as_messages() {
    let w = world().await;
    let clip = upload_clip(&w.app, &w.alex.1, w.channel.id, vec![1u8; 10]).await;
    let r = w
        .app
        .http
        .patch(w.app.url(&format!("/messages/{}", clip.id)))
        .bearer_auth(&w.alex.1)
        .json(&json!({ "content": "sneaky" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    assert_eq!(
        error_message(r).await,
        "clips are edited with their own route"
    );
}

#[tokio::test]
async fn deleting_a_clip_removes_object_reactions_and_replies() {
    let w = world().await;
    let s3 = w.app.s3.as_ref().unwrap();
    let clip = upload_clip(&w.app, &w.alex.1, w.channel.id, vec![3u8; 10]).await;
    let id = clip.id.to_string();
    let key = text(
        &w.app,
        "SELECT video_key FROM clips WHERE message_id = ?",
        &id,
    )
    .await;
    let object = format!("clips/{key}.mp4");
    assert!(s3.object(&object).is_some());
    let reply = pulse_protocol::ids::MessageId::new();
    sqlx::query(
        "INSERT INTO reactions (message_id, user_id, emoji, created_at) VALUES (?, ?, 'x', ?)",
    )
    .bind(&id)
    .bind(w.sam.0.to_string())
    .bind(pulse_server::db::now())
    .execute(&w.app.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO messages (id, channel_id, author_id, kind, content, created_at, thread_root_id)
         VALUES (?, ?, ?, 'normal', 'a reply', ?, ?)",
    )
    .bind(reply.to_string())
    .bind(w.channel.id.to_string())
    .bind(w.sam.0.to_string())
    .bind(pulse_server::db::now())
    .bind(&id)
    .execute(&w.app.db)
    .await
    .unwrap();
    let mut ws = hello(&w.app, &w.sam.1).await;

    assert_eq!(
        delete_message(&w.app, &w.sam.1, clip.id).await.status(),
        403
    );
    assert!(s3.object(&object).is_some());
    assert_eq!(
        delete_message(&w.app, &w.alex.1, clip.id).await.status(),
        204
    );

    assert!(s3.object(&object).is_none(), "the object is gone");
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM reactions WHERE message_id = ?",
            &id
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM clips WHERE message_id = ?",
            &id
        )
        .await,
        0
    );
    assert_eq!(
        count(&w.app, "SELECT COUNT(*) FROM media WHERE hash = ?", &key).await,
        0
    );
    let (content, deleted): (String, Option<String>) =
        sqlx::query_as("SELECT content, deleted_at FROM messages WHERE id = ?")
            .bind(reply.to_string())
            .fetch_one(&w.app.db)
            .await
            .unwrap();
    assert_eq!(content, "");
    assert!(deleted.is_some());

    let (mut got_clip, mut got_reply) = (false, false);
    while let Some(f) = next_frame_beating(&mut ws, Duration::from_millis(600)).await {
        if let ServerFrame::Event(Event::MessageDeleted {
            message_id,
            thread_root_id,
            ..
        }) = f
        {
            if message_id == clip.id && thread_root_id.is_none() {
                got_clip = true;
            }
            if message_id == reply && thread_root_id == Some(clip.id) {
                got_reply = true;
            }
        }
    }
    assert!(got_clip && got_reply);

    let list: Vec<Message> = get_json(
        &w.app,
        &w.sam.1,
        &format!("/channels/{}/messages", w.channel.id),
    )
    .await;
    assert!(
        list.iter().all(|m| m.clip.is_none()),
        "a deleted clip is a bare tombstone"
    );
}

#[tokio::test]
async fn a_failed_object_delete_is_retried_by_the_sweeper() {
    let w = world().await;
    let s3 = w.app.s3.as_ref().unwrap();
    let clip = upload_clip(&w.app, &w.alex.1, w.channel.id, vec![3u8; 10]).await;
    let key = text(
        &w.app,
        "SELECT video_key FROM clips WHERE message_id = ?",
        &clip.id.to_string(),
    )
    .await;
    let object = format!("clips/{key}.mp4");

    s3.set_down(true);
    assert_eq!(
        delete_message(&w.app, &w.alex.1, clip.id).await.status(),
        204
    );
    let down = pulse_server::clips::sweep::run(
        &w.app.state(),
        chrono::Utc::now() + chrono::Duration::hours(2),
    )
    .await
    .unwrap();
    assert_eq!((down.tickets, down.orphans), (0, 0), "storage is down");
    s3.set_down(false);
    assert!(s3.object(&object).is_some(), "the delete failed");
    assert_eq!(
        count(&w.app, "SELECT COUNT(*) FROM media WHERE hash = ?", &key).await,
        1
    );

    let report = pulse_server::clips::sweep::run(
        &w.app.state(),
        chrono::Utc::now() + chrono::Duration::hours(2),
    )
    .await
    .unwrap();
    assert_eq!((report.tickets, report.orphans), (0, 1));
    assert!(s3.object(&object).is_none());
    assert_eq!(
        count(&w.app, "SELECT COUNT(*) FROM media WHERE hash = ?", &key).await,
        0
    );
}

async fn age_ticket(app: &TestApp, up: &ClipUpload, hours: i64) {
    sqlx::query("UPDATE clip_uploads SET created_at = ? WHERE id = ?")
        .bind(pulse_server::db::stamp(
            chrono::Utc::now() - chrono::Duration::hours(hours),
        ))
        .bind(up.id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn expired_tickets_and_orphans_are_swept() {
    let w = world().await;
    let s3 = w.app.s3.as_ref().unwrap();
    let old = ticket_ok(&w.app, &w.alex.1, w.channel.id, 10).await;
    put_object(&w.app, &old, vec![1u8; 10]).await;
    let old_object = format!("clips/{}.mp4", ticket_key(&w.app, &old).await);
    age_ticket(&w.app, &old, 26).await;
    let fresh = ticket_ok(&w.app, &w.alex.1, w.channel.id, 10).await;
    put_object(&w.app, &fresh, vec![2u8; 10]).await;
    let fresh_object = format!("clips/{}.mp4", ticket_key(&w.app, &fresh).await);

    // A posted clip is never an orphan.
    let kept = upload_clip(&w.app, &w.alex.1, w.channel.id, vec![5u8; 10]).await;
    let kept_key = text(
        &w.app,
        "SELECT video_key FROM clips WHERE message_id = ?",
        &kept.id.to_string(),
    )
    .await;
    // Real orphans: an object and a media row no clip references. One is under an hour old.
    let stray = "a".repeat(32);
    let recent = "b".repeat(32);
    for (k, age) in [(&stray, 2), (&recent, 0)] {
        s3.put_raw(&format!("clips/{k}.mp4"), vec![9u8; 4]);
        sqlx::query(
            "INSERT INTO media (hash, kind, location, content_type, size, width, height, frames, uploaded_by, created_at)
             VALUES (?, 'clip', 'r2', 'video/mp4', 4, 16, 16, 1, ?, ?)",
        )
        .bind(k)
        .bind(w.alex.0.to_string())
        .bind(pulse_server::db::stamp(
            chrono::Utc::now() - chrono::Duration::hours(age),
        ))
        .execute(&w.app.db)
        .await
        .unwrap();
    }

    let report = pulse_server::clips::sweep::run(&w.app.state(), chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!((report.tickets, report.orphans), (1, 1));
    assert!(s3.object(&old_object).is_none());
    assert_eq!(tickets(&w.app).await, 1);
    assert!(s3.object(&fresh_object).is_some());
    assert!(s3.object(&format!("clips/{stray}.mp4")).is_none());
    assert_eq!(
        count(&w.app, "SELECT COUNT(*) FROM media WHERE hash = ?", &stray).await,
        0
    );
    assert!(
        s3.object(&format!("clips/{recent}.mp4")).is_some(),
        "under an hour old"
    );
    assert!(s3.object(&format!("clips/{kept_key}.mp4")).is_some());
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM media WHERE hash = ?",
            &kept_key
        )
        .await,
        1
    );

    // A failed delete keeps the rows for next time.
    s3.set_down(true);
    let later = chrono::Utc::now() + chrono::Duration::hours(30);
    let report = pulse_server::clips::sweep::run(&w.app.state(), later)
        .await
        .unwrap();
    assert_eq!((report.tickets, report.orphans), (0, 0));
    assert_eq!(tickets(&w.app).await, 1);
    s3.set_down(false);
    let report = pulse_server::clips::sweep::run(&w.app.state(), later)
        .await
        .unwrap();
    // The fresh ticket, and the recent stray (both now old enough).
    assert_eq!((report.tickets, report.orphans), (1, 1));
    assert_eq!(tickets(&w.app).await, 0);
}

#[tokio::test]
async fn a_ticket_for_a_deleted_channel_is_swept() {
    let w = world().await;
    let s3 = w.app.s3.as_ref().unwrap();
    let up = ticket_ok(&w.app, &w.alex.1, w.channel.id, 10).await;
    put_object(&w.app, &up, vec![1u8; 10]).await;
    let object = format!("clips/{}.mp4", ticket_key(&w.app, &up).await);
    let thumb = upload_thumb(&w.app, &w.alex.1).await;

    sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(w.channel.id.to_string())
        .execute(&w.app.db)
        .await
        .unwrap();
    let channel: Option<String> =
        sqlx::query_scalar("SELECT channel_id FROM clip_uploads WHERE id = ?")
            .bind(up.id.to_string())
            .fetch_one(&w.app.db)
            .await
            .unwrap();
    assert_eq!(channel, None, "the ticket outlives its channel");
    assert_eq!(
        confirm(&w.app, &w.alex.1, &up, confirm_body(&thumb, None))
            .await
            .status(),
        404
    );

    age_ticket(&w.app, &up, 26).await;
    let report = pulse_server::clips::sweep::run(&w.app.state(), chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(report.tickets, 1);
    assert!(s3.object(&object).is_none());
    assert_eq!(tickets(&w.app).await, 0);
}

/// The hourly count keeps counting a clip after it is deleted through the route (rows gone).
#[tokio::test]
async fn a_deleted_clip_still_counts_toward_the_hour() {
    let w = world().await;
    for _ in 0..19 {
        upload_clip(&w.app, &w.alex.1, w.channel.id, vec![5u8; 10]).await;
    }
    let last = upload_clip(&w.app, &w.alex.1, w.channel.id, vec![5u8; 10]).await;
    assert_eq!(
        delete_message(&w.app, &w.alex.1, last.id).await.status(),
        204
    );
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM clips WHERE message_id = ?",
            &last.id.to_string()
        )
        .await,
        0
    );
    assert_eq!(
        ticket(&w.app, &w.alex.1, w.channel.id, 10, 1_000)
            .await
            .status(),
        429
    );
}

#[tokio::test]
async fn confirm_refuses_an_expired_ticket() {
    let w = world().await;
    let s3 = w.app.s3.as_ref().unwrap();
    let up = ticket_ok(&w.app, &w.alex.1, w.channel.id, 10).await;
    put_object(&w.app, &up, vec![1u8; 10]).await;
    let object = format!("clips/{}.mp4", ticket_key(&w.app, &up).await);
    let thumb = upload_thumb(&w.app, &w.alex.1).await;
    age_ticket(&w.app, &up, 25).await;
    let r = confirm(&w.app, &w.alex.1, &up, confirm_body(&thumb, None)).await;
    assert_eq!(r.status(), 409);
    assert_eq!(error_message(r).await, "That upload expired. Try again.");
    let posted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM clips")
        .fetch_one(&w.app.db)
        .await
        .unwrap();
    assert_eq!(posted, 0);
    assert!(s3.object(&object).is_some(), "left for the sweeper");
}

/// An expired ticket whose key a posted clip uses loses its row but never the object.
#[tokio::test]
async fn a_sweep_never_removes_a_posted_clips_object() {
    let w = world().await;
    let s3 = w.app.s3.as_ref().unwrap();
    let clip = upload_clip(&w.app, &w.alex.1, w.channel.id, vec![5u8; 10]).await;
    let key = text(
        &w.app,
        "SELECT video_key FROM clips WHERE message_id = ?",
        &clip.id.to_string(),
    )
    .await;
    sqlx::query(
        "INSERT INTO clip_uploads (id, key, user_id, channel_id, size, duration_ms, created_at)
         VALUES (?, ?, ?, ?, 10, 1000, ?)",
    )
    .bind(pulse_protocol::ids::ClipUploadId::new().to_string())
    .bind(&key)
    .bind(w.alex.0.to_string())
    .bind(w.channel.id.to_string())
    .bind(pulse_server::db::stamp(
        chrono::Utc::now() - chrono::Duration::hours(30),
    ))
    .execute(&w.app.db)
    .await
    .unwrap();
    let report = pulse_server::clips::sweep::run(&w.app.state(), chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(report.orphans, 0);
    assert!(s3.object(&format!("clips/{key}.mp4")).is_some());
    assert_eq!(tickets(&w.app).await, 0);
}

/// Slow storage during a sweep must not block other writers, and a failed delete leaves the
/// ticket for the next run, unchanged.
#[tokio::test]
async fn a_sweep_never_holds_the_write_lock_across_storage_calls() {
    let w = world().await;
    let s3 = w.app.s3.as_ref().unwrap();
    let up = ticket_ok(&w.app, &w.alex.1, w.channel.id, 10).await;
    put_object(&w.app, &up, vec![1u8; 10]).await;
    age_ticket(&w.app, &up, 26).await;
    let before: (String, i64, String) =
        sqlx::query_as("SELECT key, size, created_at FROM clip_uploads WHERE id = ?")
            .bind(up.id.to_string())
            .fetch_one(&w.app.db)
            .await
            .unwrap();

    s3.set_delay(Duration::from_millis(3000));
    s3.set_down(true);
    let state = w.app.state();
    let sweep = tokio::spawn(async move {
        pulse_server::clips::sweep::run(&state, chrono::Utc::now())
            .await
            .unwrap()
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    // The sweep is waiting on storage now; a write must go through at once.
    let t = std::time::Instant::now();
    let general = general(&w.app, &w.alex.1, w.server).await;
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/channels/{}/messages", general.id),
        json!({"content": "during the sweep"}),
    )
    .await;
    assert_eq!(r.status(), 200);
    assert!(
        t.elapsed() < Duration::from_millis(500),
        "write was held up"
    );
    let report = sweep.await.unwrap();
    assert_eq!((report.tickets, report.orphans), (0, 0));

    let after: (String, i64, String) =
        sqlx::query_as("SELECT key, size, created_at FROM clip_uploads WHERE id = ?")
            .bind(up.id.to_string())
            .fetch_one(&w.app.db)
            .await
            .unwrap();
    assert_eq!(after, before, "a failed delete leaves the ticket as it was");

    s3.set_delay(Duration::ZERO);
    s3.set_down(false);
    let report = pulse_server::clips::sweep::run(&w.app.state(), chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(report.tickets, 1);
    assert_eq!(tickets(&w.app).await, 0);
}

/// Expired but inside the grace period: left alone, object included.
#[tokio::test]
async fn a_ticket_inside_the_grace_period_is_left_alone() {
    let w = world().await;
    let s3 = w.app.s3.as_ref().unwrap();
    let up = ticket_ok(&w.app, &w.alex.1, w.channel.id, 10).await;
    put_object(&w.app, &up, vec![1u8; 10]).await;
    let object = format!("clips/{}.mp4", ticket_key(&w.app, &up).await);
    age_ticket(&w.app, &up, 24).await;
    let report = pulse_server::clips::sweep::run(&w.app.state(), chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(report.tickets, 0);
    assert_eq!(tickets(&w.app).await, 1);
    assert!(s3.object(&object).is_some());
}

// ---- Replies ----

async fn clip_reply(
    w: &World,
    token: &str,
    channel: ChannelId,
    root: pulse_protocol::ids::MessageId,
    text: &str,
) -> reqwest::Response {
    post_json(
        &w.app,
        token,
        &format!("/channels/{channel}/messages"),
        json!({ "content": text, "thread_root_id": root }),
    )
    .await
}

async fn read_state_of(w: &World, token: &str) -> pulse_protocol::rest::ReadState {
    ready_of(&w.app, token)
        .await
        .read_states
        .into_iter()
        .find(|s| s.channel_id == w.channel.id)
        .expect("read state")
}

#[tokio::test]
async fn replies_to_a_clip() {
    let w = world().await;
    let clip = upload_clip(&w.app, &w.alex.1, w.channel.id, vec![3u8; 100]).await;
    let before = read_state_of(&w, &w.alex.1).await;
    let r = clip_reply(&w, &w.sam.1, w.channel.id, clip.id, "nice one").await;
    assert_eq!(r.status(), 200);
    let reply: Message = r.json().await.unwrap();
    assert_eq!(reply.thread_root_id, Some(clip.id));

    let list: Vec<Message> = get_json(
        &w.app,
        &w.alex.1,
        &format!("/channels/{}/messages", w.channel.id),
    )
    .await;
    assert_eq!(list.len(), 1, "the reply stays out of the main list");
    assert_eq!(list[0].id, clip.id);
    let c = list[0].clip.as_ref().unwrap();
    assert_eq!(c.reply_count, 1);
    assert_eq!(c.last_reply_id, Some(reply.id));
    assert_eq!(c.replies_seen, None);

    let replies: Vec<Message> =
        get_json(&w.app, &w.alex.1, &format!("/messages/{}/replies", clip.id)).await;
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].id, reply.id);

    let after = read_state_of(&w, &w.alex.1).await;
    assert_eq!(after.unread, before.unread);
    // sam's own view carries sam's seen point (a reply marks your place).
    let mine: Vec<Message> = get_json(
        &w.app,
        &w.sam.1,
        &format!("/channels/{}/messages", w.channel.id),
    )
    .await;
    assert_eq!(mine[0].clip.as_ref().unwrap().replies_seen, Some(reply.id));
}

#[tokio::test]
async fn a_reply_needs_a_live_clip_in_this_channel() {
    let w = world().await;
    let clip = upload_clip(&w.app, &w.alex.1, w.channel.id, vec![4u8; 100]).await;
    let other = create_clips(&w.app, &w.alex.1, w.server).await;
    let r = clip_reply(&w, &w.sam.1, other.id, clip.id, "wrong channel").await;
    assert_eq!(r.status(), 400);
    assert_eq!(
        delete_message(&w.app, &w.alex.1, clip.id).await.status(),
        204
    );
    let r = clip_reply(&w, &w.sam.1, w.channel.id, clip.id, "too late").await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn a_reply_mention_counts_until_seen() {
    let w = world().await;
    let clip = upload_clip(&w.app, &w.alex.1, w.channel.id, vec![5u8; 100]).await;
    let r: Message = clip_reply(&w, &w.sam.1, w.channel.id, clip.id, "look @alex")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(read_state_of(&w, &w.alex.1).await.mentions, 1);
    let seen = post_json(
        &w.app,
        &w.alex.1,
        &format!("/messages/{}/replies/seen", clip.id),
        json!({ "message_id": r.id }),
    )
    .await;
    assert_eq!(seen.status(), 204);
    assert_eq!(read_state_of(&w, &w.alex.1).await.mentions, 0);
    let list: Vec<Message> = get_json(
        &w.app,
        &w.alex.1,
        &format!("/channels/{}/messages", w.channel.id),
    )
    .await;
    assert_eq!(list[0].clip.as_ref().unwrap().replies_seen, Some(r.id));
}

#[tokio::test]
async fn replies_get_normal_reactions() {
    let w = world().await;
    let clip = upload_clip(&w.app, &w.alex.1, w.channel.id, vec![6u8; 100]).await;
    let r: Message = clip_reply(&w, &w.sam.1, w.channel.id, clip.id, "hi")
        .await
        .json()
        .await
        .unwrap();
    let put = w
        .app
        .http
        .put(w.app.url(&format!("/messages/{}/reactions/👍", r.id)))
        .bearer_auth(&w.alex.1)
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), 204);
    let replies: Vec<Message> =
        get_json(&w.app, &w.alex.1, &format!("/messages/{}/replies", clip.id)).await;
    assert_eq!(replies[0].reactions.len(), 1);
    assert_eq!(replies[0].reactions[0].count, 1);
}

// ---- Playback URLs, feeds, facets, view preference, storage totals ----

async fn get_status(app: &TestApp, token: &str, path: &str) -> reqwest::Response {
    app.http
        .get(app.url(path))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
}

async fn play_url(
    app: &TestApp,
    token: &str,
    id: pulse_protocol::ids::MessageId,
) -> reqwest::Response {
    get_status(app, token, &format!("/clips/{id}/url")).await
}

#[tokio::test]
async fn play_urls_are_fresh_and_member_only() {
    let w = world().await;
    let bytes: Vec<u8> = (0..100u8).collect();
    let clip = upload_clip(&w.app, &w.alex.1, w.channel.id, bytes.clone()).await;

    let mut last = String::new();
    for _ in 0..2 {
        let r = play_url(&w.app, &w.sam.1, clip.id).await;
        assert_eq!(r.status(), 200);
        let u: pulse_protocol::rest::ClipUrl = r.json().await.unwrap();
        assert!(u.expires_at >= last, "{} < {last}", u.expires_at);
        last = u.expires_at.clone();
        let got = w
            .app
            .http
            .get(&u.url)
            .header("range", "bytes=0-9")
            .send()
            .await
            .unwrap();
        assert_eq!(got.status(), 206);
        assert_eq!(got.bytes().await.unwrap().to_vec(), bytes[..10].to_vec());
    }

    // Someone who is not in the server: 403.
    let kim = register(&w.app, "kim").await;
    assert_eq!(play_url(&w.app, &kim.1, clip.id).await.status(), 403);
    // Not a clip at all: 404.
    assert_eq!(
        play_url(&w.app, &w.sam.1, pulse_protocol::ids::MessageId::new())
            .await
            .status(),
        404
    );

    // A clip from years back still plays.
    sqlx::query("UPDATE messages SET created_at = '2022-01-01T00:00:00.000Z' WHERE id = ?")
        .bind(clip.id.to_string())
        .execute(&w.app.db)
        .await
        .unwrap();
    let u: pulse_protocol::rest::ClipUrl = play_url(&w.app, &w.sam.1, clip.id)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        w.app
            .http
            .get(&u.url)
            .header("range", "bytes=0-9")
            .send()
            .await
            .unwrap()
            .status(),
        206
    );

    // A deleted clip: 404.
    assert_eq!(
        delete_message(&w.app, &w.alex.1, clip.id).await.status(),
        204
    );
    assert_eq!(play_url(&w.app, &w.sam.1, clip.id).await.status(), 404);
}

#[tokio::test]
async fn play_urls_without_storage_are_503() {
    let bare = spawn_without_clips().await;
    let alex = register(&bare, "alex").await;
    let s = create_server(&bare, &alex.1, "Main").await;
    let ch = create_clips(&bare, &alex.1, s.id).await;
    let r = play_url(&bare, &alex.1, pulse_protocol::ids::MessageId::new()).await;
    assert_eq!(r.status(), 503);
    // Feeds and facets read only the database, so they work.
    let feed: Vec<Message> = get_json(&bare, &alex.1, &format!("/channels/{}/clips", ch.id)).await;
    assert!(feed.is_empty());
}

fn ids(v: &[Message]) -> Vec<pulse_protocol::ids::MessageId> {
    v.iter().map(|m| m.id).collect()
}

#[tokio::test]
async fn feeds_page_and_filter() {
    let w = world().await;
    let doom = add_game_ok(&w.app, &w.alex.1, w.server, "Doom").await;
    let halo = add_game_ok(&w.app, &w.alex.1, w.server, "Halo").await;
    // 18 from alex, 17 from sam, oldest first.
    let mut posted = Vec::new();
    for i in 0..35 {
        let (token, uid) = if i % 2 == 0 {
            (&w.alex.1, w.alex.0)
        } else {
            (&w.sam.1, w.sam.0)
        };
        let m = upload_clip(&w.app, token, w.channel.id, vec![i as u8; 10]).await;
        let game = if i % 3 == 0 { doom.id } else { halo.id };
        sqlx::query("UPDATE clips SET game_id = ? WHERE message_id = ?")
            .bind(game.to_string())
            .bind(m.id.to_string())
            .execute(&w.app.db)
            .await
            .unwrap();
        posted.push((m.id, uid, game));
    }
    let newest_first: Vec<_> = posted.iter().rev().map(|p| p.0).collect();
    let path = format!("/channels/{}/clips", w.channel.id);

    let page1: Vec<Message> = get_json(&w.app, &w.sam.1, &path).await;
    assert_eq!(ids(&page1), newest_first[..30].to_vec());
    assert!(page1.iter().all(|m| m.clip.is_some()));
    let page2: Vec<Message> =
        get_json(&w.app, &w.sam.1, &format!("{path}?before={}", page1[29].id)).await;
    assert_eq!(ids(&page2), newest_first[30..].to_vec());
    let small: Vec<Message> = get_json(&w.app, &w.sam.1, &format!("{path}?limit=3")).await;
    assert_eq!(small.len(), 3);
    let big: Vec<Message> = get_json(&w.app, &w.sam.1, &format!("{path}?limit=99")).await;
    assert_eq!(big.len(), 30, "capped at a page");

    let by_game: Vec<Message> =
        get_json(&w.app, &w.sam.1, &format!("{path}?game={}", doom.id)).await;
    let want: Vec<_> = posted
        .iter()
        .rev()
        .filter(|p| p.2 == doom.id)
        .map(|p| p.0)
        .collect();
    assert_eq!(ids(&by_game), want);
    let by_author: Vec<Message> =
        get_json(&w.app, &w.sam.1, &format!("{path}?author={}", w.sam.0)).await;
    assert_eq!(by_author.len(), 17);
    assert!(by_author.iter().all(|m| m.author_id == Some(w.sam.0)));
    let both: Vec<Message> = get_json(
        &w.app,
        &w.sam.1,
        &format!("{path}?author={}&game={}", w.sam.0, doom.id),
    )
    .await;
    let want: Vec<_> = posted
        .iter()
        .rev()
        .filter(|p| p.1 == w.sam.0 && p.2 == doom.id)
        .map(|p| p.0)
        .collect();
    assert_eq!(ids(&both), want);

    // Replies and deleted clips are absent.
    let newest = posted.last().unwrap().0;
    assert_eq!(
        clip_reply(&w, &w.sam.1, w.channel.id, newest, "gg")
            .await
            .status(),
        200
    );
    assert_eq!(
        delete_message(&w.app, &w.sam.1, posted[33].0)
            .await
            .status(),
        204
    );
    let again: Vec<Message> = get_json(&w.app, &w.sam.1, &path).await;
    assert_eq!(again.len(), 30);
    assert!(again.iter().all(|m| m.thread_root_id.is_none()));
    assert!(!ids(&again).contains(&posted[33].0));
    assert_eq!(again[0].clip.as_ref().unwrap().reply_count, 1);
}

#[tokio::test]
async fn facets_count_live_clips() {
    let w = world().await;
    let doom = add_game_ok(&w.app, &w.alex.1, w.server, "Doom").await;
    let halo = add_game_ok(&w.app, &w.alex.1, w.server, "Halo").await;
    let apex = add_game_ok(&w.app, &w.alex.1, w.server, "Apex").await;
    // (author token, author id, game)
    let plan = [
        (&w.alex.1, Some(doom.id)),
        (&w.alex.1, Some(halo.id)),
        (&w.sam.1, Some(halo.id)),
        (&w.sam.1, Some(halo.id)),
        (&w.sam.1, Some(apex.id)),
        (&w.sam.1, Some(apex.id)),
        (&w.sam.1, None),
        (&w.alex.1, None),
    ];
    let mut clips = Vec::new();
    for (token, game) in plan {
        let m = upload_clip(&w.app, token, w.channel.id, vec![1u8; 10]).await;
        sqlx::query("UPDATE clips SET game_id = ? WHERE message_id = ?")
            .bind(game.map(|g| g.to_string()))
            .bind(m.id.to_string())
            .execute(&w.app.db)
            .await
            .unwrap();
        clips.push(m.id);
    }
    // A reply and a deleted clip add nothing.
    clip_reply(&w, &w.sam.1, w.channel.id, clips[0], "hi").await;
    assert_eq!(
        delete_message(&w.app, &w.sam.1, clips[2]).await.status(),
        204
    );

    let f: pulse_protocol::rest::ClipFacets = get_json(
        &w.app,
        &w.alex.1,
        &format!("/channels/{}/clips/facets", w.channel.id),
    )
    .await;
    let games: Vec<_> = f.games.iter().map(|g| (g.game_id, g.count)).collect();
    // Count desc, then name: Apex 2, Halo 2 (one of three was deleted), Doom 1.
    assert_eq!(games, vec![(apex.id, 2), (halo.id, 2), (doom.id, 1)]);
    let authors: Vec<_> = f.authors.iter().map(|a| (a.user_id, a.count)).collect();
    assert_eq!(authors, vec![(w.sam.0, 4), (w.alex.0, 3)]);
}

#[tokio::test]
async fn view_preference_is_per_person_and_channel() {
    let w = world().await;
    let mut alex_ws = hello(&w.app, &w.alex.1).await;
    let mut sam_ws = hello(&w.app, &w.sam.1).await;
    let path = format!("/channels/{}/clip-view", w.channel.id);
    let put = |token: &str, body: serde_json::Value, path: String| {
        let r = w
            .app
            .http
            .put(w.app.url(&path))
            .bearer_auth(token)
            .json(&body);
        async move { r.send().await.unwrap() }
    };
    assert_eq!(
        put(&w.alex.1, json!({ "view": "list" }), path.clone())
            .await
            .status(),
        204
    );
    // Again, to change it, and an upsert not a duplicate.
    assert_eq!(
        put(&w.alex.1, json!({ "view": "wall" }), path.clone())
            .await
            .status(),
        204
    );
    assert_eq!(
        put(&w.alex.1, json!({ "view": "list" }), path.clone())
            .await
            .status(),
        204
    );

    let r = ready_of(&w.app, &w.alex.1).await;
    assert_eq!(r.clip_views.len(), 1);
    assert_eq!(r.clip_views[0].channel_id, w.channel.id);
    assert_eq!(r.clip_views[0].view, pulse_protocol::rest::ClipView::List);
    assert!(ready_of(&w.app, &w.sam.1).await.clip_views.is_empty());

    let mut seen = false;
    while let Some(f) = next_frame_beating(&mut alex_ws, Duration::from_millis(600)).await {
        if let ServerFrame::Event(Event::ClipViewChanged {
            user_id,
            channel_id,
            view,
        }) = f
        {
            assert_eq!((user_id, channel_id), (w.alex.0, w.channel.id));
            seen = view == pulse_protocol::rest::ClipView::List || seen;
        }
    }
    assert!(seen, "alex's session hears about it");
    while let Some(f) = next_frame_beating(&mut sam_ws, Duration::from_millis(600)).await {
        assert!(
            !matches!(f, ServerFrame::Event(Event::ClipViewChanged { .. })),
            "sam must not hear it"
        );
    }

    // Not a clips channel: 400. Not a member: 403.
    let general = general(&w.app, &w.alex.1, w.server).await;
    let r = put(
        &w.alex.1,
        json!({ "view": "list" }),
        format!("/channels/{}/clip-view", general.id),
    )
    .await;
    assert_eq!(r.status(), 400);
    let kim = register(&w.app, "kim").await;
    assert_eq!(
        put(&kim.1, json!({ "view": "list" }), path).await.status(),
        403
    );
}

#[tokio::test]
async fn storage_counts_this_server_only() {
    let w = world().await;
    for n in [100usize, 250] {
        upload_clip(&w.app, &w.alex.1, w.channel.id, vec![3u8; n]).await;
    }
    let gone = upload_clip(&w.app, &w.alex.1, w.channel.id, vec![3u8; 4_000]).await;
    assert_eq!(
        delete_message(&w.app, &w.alex.1, gone.id).await.status(),
        204
    );
    // A second server with its own clip.
    let other = create_server(&w.app, &w.alex.1, "Other").await;
    let och = create_clips(&w.app, &w.alex.1, other.id).await;
    upload_clip(&w.app, &w.alex.1, och.id, vec![3u8; 1_000_000]).await;

    let thumb_bytes = |server: ServerId| {
        let db = w.app.db.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(SUM(t.size), 0) FROM clips c
                 JOIN messages m ON m.id = c.message_id
                 JOIN channels ch ON ch.id = m.channel_id
                 JOIN media t ON t.hash = c.thumb_hash
                 WHERE ch.server_id = ? AND m.deleted_at IS NULL",
            )
            .bind(server.to_string())
            .fetch_one(&db)
            .await
            .unwrap() as u64
        }
    };
    let a: pulse_protocol::rest::ClipStorage = get_json(
        &w.app,
        &w.sam.1,
        &format!("/servers/{}/clips/storage", w.server),
    )
    .await;
    assert_eq!(a.count, 2);
    let thumbs = thumb_bytes(w.server).await;
    assert!(thumbs > 0);
    assert_eq!(a.bytes, 350 + thumbs);
    assert!((a.est_monthly_usd - a.bytes as f64 / 1e9 * 0.015).abs() < 1e-15);

    let b: pulse_protocol::rest::ClipStorage = get_json(
        &w.app,
        &w.alex.1,
        &format!("/servers/{}/clips/storage", other.id),
    )
    .await;
    assert_eq!(b.count, 1);
    assert_eq!(b.bytes, 1_000_000 + thumb_bytes(other.id).await);

    // Members only.
    let kim = register(&w.app, "kim").await;
    let r = get_status(
        &w.app,
        &kim.1,
        &format!("/servers/{}/clips/storage", w.server),
    )
    .await;
    assert_eq!(r.status(), 403);
}

#[tokio::test]
async fn storage_without_clips_storage_is_503() {
    let bare = spawn_without_clips().await;
    let alex = register(&bare, "alex").await;
    let s = create_server(&bare, &alex.1, "Main").await;
    let r = get_status(&bare, &alex.1, &format!("/servers/{}/clips/storage", s.id)).await;
    assert_eq!(r.status(), 503);
    assert_eq!(
        error_message(r).await,
        "Clips storage isn't set up on this server."
    );
}

#[test]
fn the_storage_report_prices_past_the_free_tier() {
    use pulse_server::clips::{ServerUsage, format_storage_report};
    let line = |name: &str, count: u32, gb: f64| ServerUsage {
        name: name.into(),
        count,
        bytes: (gb * 1e9) as u64,
    };
    let small = format_storage_report(&[line("Main", 3, 1.5), line("Other", 1, 0.25)]);
    assert_eq!(
        small,
        "Main  3 clips  1.50 GB\nOther  1 clips  0.25 GB\n\
         total 4 clips  1.75 GB  ~$0.00/month (after R2's 10 GB free tier; the backups bucket shares it)"
    );
    let big = format_storage_report(&[line("Main", 9, 20.0)]);
    assert!(
        big.ends_with("total 9 clips  20.00 GB  ~$0.15/month (after R2's 10 GB free tier; the backups bucket shares it)"),
        "{big}"
    );
}
