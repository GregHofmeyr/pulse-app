//! Quotes: the `quotes` channel kind and migration 0008.
mod common;

use std::borrow::Cow;
use std::time::Duration;

use common::*;
use futures::SinkExt;
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, MessageId};
use pulse_protocol::rest::{Channel, ChannelKind};
use serde_json::json;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection};
use tokio_tungstenite::tungstenite::Message as Ws;

async fn hello(app: &TestApp, token: &str) -> Socket {
    hello_ready(app, token).await.0
}

async fn hello_ready(app: &TestApp, token: &str) -> (Socket, pulse_protocol::gateway::Ready) {
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
            Some(ServerFrame::Ready(r)) => return (ws, r),
            Some(_) => continue,
            None => panic!("expected Ready"),
        }
    }
}

/// The next frame, or `None` after 3 quiet seconds (heartbeating meanwhile).
async fn next_frame(ws: &mut Socket) -> Option<ServerFrame> {
    next_frame_beating(ws, Duration::from_secs(3)).await
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
        format!(
            "INSERT INTO dm_hidden (user_id, channel_id, hidden_at) VALUES ('{user}', '{dm}', '{ts}')"
        ),
        format!(
            "INSERT INTO voice_sessions (id, user_id, channel_id, joined_at) VALUES ('VS', '{user}', '{voice}', '{ts}')"
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
    assert_eq!(count("dm_hidden").await, 1, "the hidden dm stayed hidden");
    assert_eq!(count("voice_sessions").await, 1);
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
    for (child, of) in [("dm_hidden", dm), ("voice_sessions", voice)] {
        sqlx::query("DELETE FROM channels WHERE id = ?")
            .bind(of)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(count(child).await, 0, "{child} still cascades");
    }
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

/// A reactions row whose message is gone, written with foreign keys off (as an old build
/// could have left one).
async fn orphan_reaction(path: &std::path::Path) {
    let mut conn = SqliteConnectOptions::new()
        .filename(path)
        .foreign_keys(false)
        .connect()
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at) VALUES ('U', 'alex', 'x', '2026-10-01T00:00:00.000Z')",
    )
    .execute(&mut conn)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO reactions (message_id, user_id, emoji, created_at) VALUES ('GONE', 'U', '👍', '2026-10-01T00:00:00.000Z')",
    )
    .execute(&mut conn)
    .await
    .unwrap();
    conn.close().await.unwrap();
}

/// M1: an unrelated legacy orphan never stops a boot that applies nothing.
#[tokio::test]
async fn a_legacy_orphan_does_not_stop_a_boot_without_migrations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pulse.db");
    let url = format!("sqlite://{}", path.display());
    pulse_server::db::connect(&url).await.unwrap().close().await;
    orphan_reaction(&path).await;
    let pool = pulse_server::db::connect(&url)
        .await
        .expect("a boot that migrates nothing starts despite an old orphan");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reactions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "nothing is touched");
}

/// M1: after migrating, an orphan outside 0008's guarded tables is only logged; the scoped
/// guard stays the hard stop.
#[tokio::test]
async fn a_legacy_orphan_outside_the_guard_does_not_stop_a_migrating_boot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(false);
    let mut conn = opts.connect().await.unwrap();
    let mut migrator = sqlx::migrate::Migrator::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"),
    )
    .await
    .unwrap();
    migrator.migrations = Cow::Owned(
        migrator
            .migrations
            .iter()
            .filter(|m| m.version <= 7)
            .cloned()
            .collect(),
    );
    migrator.run(&mut conn).await.unwrap();
    conn.close().await.unwrap();
    orphan_reaction(&path).await;

    let pool = pulse_server::db::connect(&format!("sqlite://{}", path.display()))
        .await
        .expect("0008 applies and the server starts");
    let v: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(v, 8);
}

// ---------- posting and reading ----------

use pulse_protocol::ids::{LineId, ServerId, UserId};
use pulse_protocol::rest::{MediaInfo, Message};

struct Fixture {
    app: TestApp,
    alex: UserId,
    alex_t: String,
    sam: UserId,
    sam_t: String,
    server: ServerId,
    ch: Channel,
}

/// alex and sam in one server with a quotes channel.
async fn fixture() -> Fixture {
    let app = spawn().await;
    let (alex, alex_t) = register(&app, "alex").await;
    let (sam, sam_t) = register(&app, "sam").await;
    let s = create_server(&app, &alex_t, "Main").await;
    let r = post_json(&app, &sam_t, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);
    let ch = create_quotes(&app, &alex_t, s.id).await;
    Fixture {
        app,
        alex,
        alex_t,
        sam,
        sam_t,
        server: s.id,
        ch,
    }
}

impl Fixture {
    async fn post(&self, token: &str, body: serde_json::Value) -> reqwest::Response {
        post_json(
            &self.app,
            token,
            &format!("/channels/{}/quotes", self.ch.id),
            body,
        )
        .await
    }
    async fn post_lines(&self, lines: serde_json::Value) -> reqwest::Response {
        self.post(&self.alex_t, json!({ "lines": lines })).await
    }
    async fn status(&self, lines: serde_json::Value) -> u16 {
        self.post_lines(lines).await.status().as_u16()
    }
    fn say(&self, who: UserId, text: &str) -> serde_json::Value {
        json!({ "speaker_id": who, "text": text })
    }
}

fn png_bytes() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(64, 32, image::Rgba([10, 120, 240, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn gif_bytes() -> Vec<u8> {
    use image::codecs::gif::GifEncoder;
    use image::{Delay, Frame};
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        for i in 0..3 {
            let img = image::RgbaImage::from_pixel(32, 16, image::Rgba([i * 80, 40, 90, 255]));
            enc.encode_frame(Frame::from_parts(
                img,
                0,
                0,
                Delay::from_numer_denom_ms(50, 1),
            ))
            .unwrap();
        }
    }
    out
}

async fn upload(f: &Fixture, path: &str, body: Vec<u8>) -> MediaInfo {
    let r = f
        .app
        .http
        .post(f.app.url(path))
        .bearer_auth(&f.alex_t)
        .header("content-type", "application/octet-stream")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

#[tokio::test]
async fn posts_a_two_line_quote() {
    let f = fixture().await;
    let r = f
        .post_lines(json!([
            f.say(f.sam, "“this is a quote”"),
            f.say(f.alex, "and this builds on it"),
        ]))
        .await;
    assert_eq!(r.status(), 200);
    let m: Message = r.json().await.unwrap();
    assert_eq!(m.channel_id, f.ch.id);
    assert_eq!(m.author_id, Some(f.alex));
    assert!(m.reactions.is_empty());
    let q = m.quote.clone().unwrap();
    assert_eq!(q.lines.len(), 2);
    assert_eq!(q.lines[0].speaker_id, Some(f.sam));
    assert_eq!(q.lines[0].text.as_deref(), Some("this is a quote"));
    assert_eq!(q.lines[1].speaker_id, Some(f.alex));
    assert_eq!(q.lines[1].text.as_deref(), Some("and this builds on it"));
    let year: i32 = chrono::Utc::now().format("%Y").to_string().parse().unwrap();
    assert_eq!(q.year, year);
    assert_eq!(q.said_on, None);
    assert_eq!(
        (q.reply_count, q.last_reply_id, q.replies_seen),
        (0, None, None)
    );
    assert!(q.wins.is_empty());
    assert!(
        q.lines
            .iter()
            .all(|l| l.awards == 0 && l.awarders.is_empty() && !l.awarded)
    );
    assert_eq!(
        m.content,
        "“this is a quote” — sam\n“and this builds on it” — alex"
    );

    let page: Vec<Message> =
        get_json(&f.app, &f.sam_t, &format!("/channels/{}/messages", f.ch.id)).await;
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].quote, Some(q));
    assert_eq!(page[0].id, m.id);
}

#[tokio::test]
async fn a_plain_name_speaker_is_kept() {
    let f = fixture().await;
    let r = f
        .post_lines(json!([{ "speaker_name": "  A  Friend ", "text": "hello" }]))
        .await;
    assert_eq!(r.status(), 200);
    let m: Message = r.json().await.unwrap();
    let l = &m.quote.unwrap().lines[0];
    assert_eq!(l.speaker_name.as_deref(), Some("A  Friend"));
    assert_eq!(l.speaker_id, None);
    let key: String = sqlx::query_scalar("SELECT speaker_key FROM quote_lines WHERE id = ?")
        .bind(l.id.to_string())
        .fetch_one(&f.app.db)
        .await
        .unwrap();
    assert_eq!(key, "a friend");
}

#[tokio::test]
async fn line_rules() {
    let f = fixture().await;
    let sam = f.say(f.sam, "x");
    assert_eq!(f.status(json!([])).await, 400, "no lines");
    assert_eq!(
        f.status(json!(vec![sam.clone(); 11])).await,
        400,
        "11 lines"
    );
    let png = upload(&f, "/media/quote", png_bytes()).await;
    assert_eq!(
        f.status(json!([{ "speaker_id": f.sam, "text": "x", "media_hash": png.hash }]))
            .await,
        400,
        "text and media"
    );
    assert_eq!(
        f.status(json!([{ "speaker_id": f.sam }])).await,
        400,
        "neither"
    );
    assert_eq!(
        f.status(json!([{ "speaker_id": f.sam, "speaker_name": "Bo", "text": "x" }]))
            .await,
        400,
        "both speakers"
    );
    assert_eq!(f.status(json!([{ "text": "x" }])).await, 400, "no speaker");
    assert_eq!(
        f.status(json!([f.say(f.sam, "“ ”")])).await,
        400,
        "empty after stripping"
    );
    assert_eq!(
        f.status(json!([f.say(f.sam, &"a".repeat(4001))])).await,
        400,
        "4001 chars"
    );
    assert_eq!(
        f.status(json!([{ "speaker_name": "n".repeat(33), "text": "x" }]))
            .await,
        400,
        "33-char name"
    );
    assert_eq!(
        f.status(json!([{ "speaker_name": "  ", "text": "x" }]))
            .await,
        400,
        "blank name"
    );
    assert_eq!(
        f.status(
            json!([{ "speaker_id": f.sam, "media_hash": png.hash, "caption": "c".repeat(201) }])
        )
        .await,
        400,
        "201-char caption"
    );
    assert_eq!(
        f.status(
            json!([{ "speaker_id": f.sam, "media_hash": png.hash, "caption": "c".repeat(200) }])
        )
        .await,
        200,
        "200-char caption"
    );
    assert_eq!(
        f.status(json!([{ "speaker_id": f.sam, "text": "x", "caption": "c" }]))
            .await,
        400,
        "caption without media"
    );
    assert_eq!(
        f.status(json!([{ "speaker_id": f.sam, "media_hash": "0".repeat(64) }]))
            .await,
        400,
        "unknown media"
    );
    let (outsider, _) = register(&f.app, "jo").await;
    assert_eq!(
        f.status(json!([f.say(outsider, "x")])).await,
        400,
        "not a member"
    );
    assert_eq!(f.status(json!(vec![sam; 10])).await, 200, "10 lines");
    assert_eq!(
        f.status(json!([f.say(f.sam, &"a".repeat(4000))])).await,
        200
    );
    assert_eq!(
        f.status(json!([{ "speaker_name": "n".repeat(32), "text": "x" }]))
            .await,
        200
    );
}

#[tokio::test]
async fn image_lines_use_the_media_row() {
    let f = fixture().await;
    let still = upload(&f, "/media/quote", png_bytes()).await;
    let gif = upload(&f, "/media/quote", gif_bytes()).await;
    let r = f
        .post_lines(json!([
            { "speaker_id": f.sam, "media_hash": still.hash, "caption": "  look " },
            { "speaker_name": "Bo", "media_hash": gif.hash },
        ]))
        .await;
    assert_eq!(r.status(), 200);
    let m: Message = r.json().await.unwrap();
    let lines = m.quote.unwrap().lines;
    let media = lines[0].media.clone().unwrap();
    assert_eq!(
        (
            media.hash.as_str(),
            media.still_hash.clone(),
            media.width,
            media.height
        ),
        (still.hash.as_str(), None, still.width, still.height)
    );
    assert_eq!(lines[0].text, None);
    assert_eq!(lines[0].caption.as_deref(), Some("look"));
    let media = lines[1].media.clone().unwrap();
    assert_eq!(media.still_hash, gif.still_hash);
    assert!(media.still_hash.is_some());
    assert_eq!(m.content, "[image]\n[image]");

    // An animation's still is not itself a usable picture.
    let still_of_gif = gif.still_hash.clone().unwrap();
    assert_eq!(
        f.status(json!([{ "speaker_id": f.sam, "media_hash": still_of_gif }]))
            .await,
        400
    );
    // An emoji upload is fine; identical bytes dedupe across kinds.
    let emoji = upload(&f, "/media/emoji", png_bytes()).await;
    let r = f
        .post_lines(json!([{ "speaker_id": f.sam, "media_hash": emoji.hash }]))
        .await;
    assert_eq!(r.status(), 200);
    let m: Message = r.json().await.unwrap();
    let media = m.quote.unwrap().lines[0].media.clone().unwrap();
    assert_eq!(
        (
            media.hash.as_str(),
            media.still_hash,
            media.width,
            media.height
        ),
        (emoji.hash.as_str(), None, emoji.width, emoji.height)
    );
}

#[tokio::test]
async fn today_sets_the_year_within_a_day() {
    let f = fixture().await;
    let utc = chrono::Utc::now().date_naive();
    let posted_on = |m: &Message| {
        let id = m.id.to_string();
        let db = f.app.db.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT posted_on FROM quotes WHERE message_id = ?")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    let cases = [
        (utc.to_string(), utc.to_string()),
        (
            (utc - chrono::Days::new(1)).to_string(),
            (utc - chrono::Days::new(1)).to_string(),
        ),
        (
            (utc + chrono::Days::new(1)).to_string(),
            (utc + chrono::Days::new(1)).to_string(),
        ),
        ((utc + chrono::Days::new(2)).to_string(), utc.to_string()),
        ((utc - chrono::Days::new(2)).to_string(), utc.to_string()),
        ("not-a-date".to_string(), utc.to_string()),
        ("2026-13-45".to_string(), utc.to_string()),
    ];
    for (today, want) in cases {
        let r = f
            .post(
                &f.alex_t,
                json!({ "lines": [f.say(f.sam, "x")], "today": today }),
            )
            .await;
        assert_eq!(r.status(), 200);
        let m: Message = r.json().await.unwrap();
        assert_eq!(posted_on(&m).await, want, "today = {today}");
    }
}

#[tokio::test]
async fn quotes_need_a_quotes_channel_and_membership() {
    let f = fixture().await;
    let body = json!({ "lines": [f.say(f.sam, "x")] });
    let text = general(&f.app, &f.alex_t, f.server).await;
    let r = post_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/quotes", text.id),
        body.clone(),
    )
    .await;
    assert_eq!(r.status(), 400, "text channel");
    let (_, jo) = register(&f.app, "jo").await;
    assert_eq!(f.post(&jo, body.clone()).await.status(), 403, "non-member");
    let dm = create_dm(&f.app, &f.alex_t, &[f.sam]).await;
    let r = post_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/quotes", dm.id),
        body.clone(),
    )
    .await;
    assert!(
        matches!(r.status().as_u16(), 400 | 404),
        "dm: {}",
        r.status()
    );
    let missing = ChannelId::new();
    let r = post_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{missing}/quotes"),
        body,
    )
    .await;
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn quotes_cant_be_edited_as_messages() {
    let f = fixture().await;
    let m: Message = f
        .post_lines(json!([f.say(f.sam, "x")]))
        .await
        .json()
        .await
        .unwrap();
    let r = f
        .app
        .http
        .patch(f.app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(&f.alex_t)
        .json(&json!({ "content": "changed" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    let (content, edited): (String, Option<String>) =
        sqlx::query_as("SELECT content, edited_at FROM messages WHERE id = ?")
            .bind(m.id.to_string())
            .fetch_one(&f.app.db)
            .await
            .unwrap();
    assert_eq!((content.as_str(), edited), (m.content.as_str(), None));
}

#[tokio::test]
async fn a_nonce_makes_posting_idempotent() {
    let f = fixture().await;
    let body = json!({ "lines": [f.say(f.sam, "x")], "nonce": "n-1" });
    let a: Message = f.post(&f.alex_t, body.clone()).await.json().await.unwrap();
    let b: Message = f.post(&f.alex_t, body).await.json().await.unwrap();
    assert_eq!(a.id, b.id);
    assert_eq!(b.quote, a.quote);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM quotes")
        .fetch_one(&f.app.db)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn posting_publishes_and_moves_the_read_point() {
    let f = fixture().await;
    let mut ws = hello(&f.app, &f.sam_t).await;
    let m: Message = f
        .post_lines(json!([f.say(f.sam, "x")]))
        .await
        .json()
        .await
        .unwrap();
    loop {
        match next_frame(&mut ws).await.expect("MessageCreated") {
            ServerFrame::Event(Event::MessageCreated { message, .. }) => {
                assert_eq!(message.id, m.id);
                let q = message.quote.unwrap();
                assert!(q.replies_seen.is_none() && !q.lines[0].awarded);
                break;
            }
            _ => continue,
        }
    }
    let read: Option<String> = sqlx::query_scalar(
        "SELECT last_read_message_id FROM read_states WHERE user_id = ? AND channel_id = ?",
    )
    .bind(f.alex.to_string())
    .bind(f.ch.id.to_string())
    .fetch_one(&f.app.db)
    .await
    .unwrap();
    assert_eq!(read, Some(m.id.to_string()));
}

/// Awards, replies and links read through `attach`, set up by SQL until their routes exist.
#[tokio::test]
async fn reading_counts_awards_replies_and_wins() {
    let f = fixture().await;
    let (jo, _) = register(&f.app, "jo").await;
    let m: Message = f
        .post_lines(json!([
            f.say(f.sam, "one"),
            { "speaker_name": "A Friend", "text": "two" },
        ]))
        .await
        .json()
        .await
        .unwrap();
    let lines = m.quote.clone().unwrap().lines;
    let (l1, l2) = (lines[0].id.to_string(), lines[1].id.to_string());
    let db = &f.app.db;
    let exec = |q: String| async move { sqlx::query(&q).execute(db).await.unwrap() };
    // Oldest first: sam (the speaker, uncounted), alex, jo.
    for (giver, at) in [(f.sam, "01"), (f.alex, "02"), (jo, "03")] {
        exec(format!(
            "INSERT INTO quote_awards (line_id, giver_id, created_at) VALUES ('{l1}', '{giver}', '2026-01-01T00:00:{at}.000Z')"
        ))
        .await;
    }
    exec(format!(
        "INSERT INTO quote_awards (line_id, giver_id, created_at) VALUES ('{l2}', '{jo}', '2026-01-01T00:00:01.000Z')"
    ))
    .await;
    let read = |token: String| {
        let (app, ch, qid) = (&f.app, f.ch.id, m.id);
        async move {
            let page: Vec<Message> =
                get_json(app, &token, &format!("/channels/{ch}/messages")).await;
            page.iter()
                .find(|p| p.id == qid)
                .unwrap()
                .quote
                .clone()
                .unwrap()
        }
    };
    let q = read(f.alex_t.clone()).await;
    assert_eq!(
        q.lines[0].awarders,
        vec![f.alex, jo],
        "own award not counted"
    );
    assert_eq!(q.lines[0].awards, 2);
    assert!(q.lines[0].awarded);
    assert_eq!(q.lines[1].awarders, vec![jo]);
    assert!(!q.lines[1].awarded);
    let q = read(f.sam_t.clone()).await;
    assert!(
        !q.lines[0].awarded,
        "sam's own award is not counted, so not awarded"
    );

    // A link makes jo the effective speaker of "A Friend": jo's award on line 2 drops.
    exec(format!(
        "INSERT INTO speaker_links (server_id, name_key, name, user_id, linked_by, linked_at)
         VALUES ('{}', 'a friend', 'A Friend', '{jo}', '{}', '2026-01-02T00:00:00.000Z')",
        f.server, f.alex
    ))
    .await;
    let q = read(f.alex_t.clone()).await;
    assert_eq!(q.lines[1].awards, 0);
    assert!(q.lines[1].awarders.is_empty());
    // The raw speaker survives.
    assert_eq!(q.lines[1].speaker_name.as_deref(), Some("A Friend"));
    assert_eq!(q.lines[0].awards, 2, "line 1 unaffected");

    // Replies count live ones, and sam has seen up to the first.
    let mid = m.id.to_string();
    let (r1, r2, r3) = (MessageId::new(), MessageId::new(), MessageId::new());
    for (id, deleted) in [(r1, false), (r2, false), (r3, true)] {
        exec(format!(
            "INSERT INTO messages (id, channel_id, author_id, content, created_at, thread_root_id, deleted_at)
             VALUES ('{id}', '{}', '{}', 'r', '2026-01-03T00:00:00.000Z', '{mid}', {})",
            f.ch.id,
            f.sam,
            if deleted { "'2026-01-04T00:00:00.000Z'" } else { "NULL" }
        ))
        .await;
    }
    exec(format!(
        "INSERT INTO thread_reads (user_id, root_id, last_seen_id) VALUES ('{}', '{mid}', '{r1}')",
        f.sam
    ))
    .await;
    let q = read(f.sam_t.clone()).await;
    assert_eq!(
        (q.reply_count, q.last_reply_id, q.replies_seen),
        (2, Some(r2), Some(r1))
    );
    let q = read(f.alex_t.clone()).await;
    assert_eq!((q.reply_count, q.replies_seen), (2, None));

    // Wins come from qoty_winners.
    let e1 = pulse_protocol::ids::QotyEventId::new();
    exec(format!(
        "INSERT INTO qoty_events (id, server_id, year, started_by, started_at, ended_at, end_reason)
         VALUES ('{e1}', '{}', 2025, '{}', '2025-12-01T00:00:00.000Z', '2025-12-02T00:00:00.000Z', 'early')",
        f.server, f.alex
    ))
    .await;
    exec(format!(
        "INSERT INTO qoty_winners (event_id, message_id, votes) VALUES ('{e1}', '{mid}', 3)"
    ))
    .await;
    assert_eq!(read(f.alex_t.clone()).await.wins, vec![2025]);
}

// ---- editing and deleting ----

impl Fixture {
    async fn put(&self, token: &str, id: MessageId, body: serde_json::Value) -> reqwest::Response {
        self.app
            .http
            .put(self.app.url(&format!("/quotes/{id}")))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .unwrap()
    }
    async fn delete(&self, token: &str, id: MessageId) -> reqwest::Response {
        self.app
            .http
            .delete(self.app.url(&format!("/messages/{id}")))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
    }
    async fn award(&self, line: LineId, giver: UserId) {
        sqlx::query("INSERT INTO quote_awards (line_id, giver_id, created_at) VALUES (?, ?, '2026-01-01T00:00:00.000Z')")
            .bind(line.to_string())
            .bind(giver.to_string())
            .execute(&self.app.db)
            .await
            .unwrap();
    }
    async fn awards_on(&self, line: LineId) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM quote_awards WHERE line_id = ?")
            .bind(line.to_string())
            .fetch_one(&self.app.db)
            .await
            .unwrap()
    }
    /// A two-line quote by alex: sam says "one", a plain name says "two"; each line has one award.
    async fn awarded_quote(&self) -> Message {
        let m: Message = self
            .post_lines(json!([
                self.say(self.sam, "one"),
                { "speaker_name": "A Friend", "text": "two" },
            ]))
            .await
            .json()
            .await
            .unwrap();
        for l in &m.quote.as_ref().unwrap().lines {
            self.award(l.id, self.alex).await;
        }
        m
    }
}

/// The edit body that changes nothing about `m`'s lines, as the client would send it back.
fn same_lines(m: &Message) -> Vec<serde_json::Value> {
    m.quote
        .as_ref()
        .unwrap()
        .lines
        .iter()
        .map(|l| {
            json!({
                "id": l.id,
                "speaker_id": l.speaker_id,
                "speaker_name": l.speaker_name,
                "text": l.text,
            })
        })
        .collect()
}

#[tokio::test]
async fn editing_text_keeps_awards() {
    let f = fixture().await;
    let m = f.awarded_quote().await;
    let mut ws = hello(&f.app, &f.sam_t).await;
    let mut lines = same_lines(&m);
    lines[0]["text"] = json!("“one, edited”");
    let r = f.put(&f.alex_t, m.id, json!({ "lines": lines })).await;
    assert_eq!(r.status(), 200);
    let edited: Message = r.json().await.unwrap();
    let q = edited.quote.clone().unwrap();
    assert_eq!(q.lines[0].id, m.quote.as_ref().unwrap().lines[0].id);
    assert_eq!(q.lines[0].text.as_deref(), Some("one, edited"));
    assert_eq!((q.lines[0].awards, q.lines[1].awards), (1, 1));
    assert!(edited.edited_at.is_some());
    assert_eq!(
        edited.content, "“one, edited” — sam\n“two” — A Friend",
        "content is re-rendered"
    );
    for l in &q.lines {
        assert_eq!(f.awards_on(l.id).await, 1);
    }
    loop {
        match next_frame(&mut ws).await.expect("MessageUpdated") {
            ServerFrame::Event(Event::MessageUpdated { message }) => {
                assert_eq!(message.id, m.id);
                assert_eq!(
                    message.quote.unwrap().lines[0].text.as_deref(),
                    Some("one, edited")
                );
                assert!(message.edited_at.is_some());
                break;
            }
            _ => continue,
        }
    }
}

#[tokio::test]
async fn changing_a_speaker_drops_that_lines_awards() {
    let f = fixture().await;
    let m = f.awarded_quote().await;
    let ids: Vec<LineId> = m
        .quote
        .as_ref()
        .unwrap()
        .lines
        .iter()
        .map(|l| l.id)
        .collect();
    let mut lines = same_lines(&m);
    // Line 1: sam -> alex (a different member). Line 2: the same plain name, recased and respaced.
    lines[0]["speaker_id"] = json!(f.alex);
    lines[1]["speaker_name"] = json!("  a   FRIEND ");
    let r = f.put(&f.alex_t, m.id, json!({ "lines": lines })).await;
    assert_eq!(r.status(), 200);
    assert_eq!(f.awards_on(ids[0]).await, 0);
    assert_eq!(
        f.awards_on(ids[1]).await,
        1,
        "a recased name is the same speaker"
    );
    let edited: Message = r.json().await.unwrap();
    assert_eq!(
        edited.quote.as_ref().unwrap().lines[1]
            .speaker_name
            .as_deref(),
        Some("a   FRIEND"),
        "the new spelling is stored"
    );

    // A real rename drops them too.
    let mut lines = same_lines(&edited);
    lines[1]["speaker_name"] = json!("Someone Else");
    assert_eq!(
        f.put(&f.alex_t, m.id, json!({ "lines": lines }))
            .await
            .status(),
        200
    );
    assert_eq!(f.awards_on(ids[1]).await, 0);

    // So does switching between a member and a name.
    f.award(ids[0], f.sam).await;
    let mut lines = same_lines(&edited);
    lines[0]["speaker_id"] = json!(null);
    lines[0]["speaker_name"] = json!("alex");
    assert_eq!(
        f.put(&f.alex_t, m.id, json!({ "lines": lines }))
            .await
            .status(),
        200
    );
    assert_eq!(f.awards_on(ids[0]).await, 0);
}

#[tokio::test]
async fn removing_a_line_takes_its_awards() {
    let f = fixture().await;
    let m = f.awarded_quote().await;
    let ids: Vec<LineId> = m
        .quote
        .as_ref()
        .unwrap()
        .lines
        .iter()
        .map(|l| l.id)
        .collect();
    let mut lines = same_lines(&m);
    lines.remove(0);
    // And a new line (no id) joins at the end.
    lines.push(f.say(f.alex, "three"));
    let r = f.put(&f.alex_t, m.id, json!({ "lines": lines })).await;
    assert_eq!(r.status(), 200);
    let q = r.json::<Message>().await.unwrap().quote.unwrap();
    assert_eq!(q.lines.len(), 2);
    assert_eq!(q.lines[0].id, ids[1]);
    assert_eq!(q.lines[1].text.as_deref(), Some("three"));
    assert!(!ids.contains(&q.lines[1].id));
    assert_eq!(f.awards_on(ids[0]).await, 0);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM quote_lines WHERE id = ?")
        .bind(ids[0].to_string())
        .fetch_one(&f.app.db)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    assert_eq!(q.lines[0].awards, 1);
}

#[tokio::test]
async fn lines_can_be_reordered() {
    let f = fixture().await;
    let m = f.awarded_quote().await;
    let mut lines = same_lines(&m);
    lines.reverse();
    let q = f
        .put(&f.alex_t, m.id, json!({ "lines": lines }))
        .await
        .json::<Message>()
        .await
        .unwrap()
        .quote
        .unwrap();
    assert_eq!(q.lines[0].text.as_deref(), Some("two"));
    assert_eq!(q.lines[1].text.as_deref(), Some("one"));
}

/// A name that is only dots ("." or "..") can't be a speaker: its name_key couldn't be a path
/// segment, so a link to it could never be undone (M14).
#[tokio::test]
async fn dot_only_names_are_refused() {
    let f = fixture().await;
    for name in [".", "..", " ... "] {
        assert_eq!(
            f.status(json!([{ "speaker_name": name, "text": "x" }]))
                .await,
            400,
            "{name:?}"
        );
    }
    assert_eq!(
        f.status(json!([{ "speaker_name": "Dot.", "text": "x" }]))
            .await,
        200,
        "dots beside other characters are fine"
    );
    let m: Message = f
        .post_lines(json!([{ "speaker_name": "A Friend", "text": "x" }]))
        .await
        .json()
        .await
        .unwrap();
    let line = m.quote.as_ref().unwrap().lines[0].id;
    let body = json!({ "lines": [{ "id": line, "speaker_name": "..", "text": "x" }] });
    assert_eq!(f.put(&f.alex_t, m.id, body).await.status(), 400, "edit");
}

#[tokio::test]
async fn only_the_poster_edits() {
    let f = fixture().await;
    let m = f.awarded_quote().await;
    let body = json!({ "lines": same_lines(&m) });
    assert_eq!(f.put(&f.sam_t, m.id, body.clone()).await.status(), 403);
    let (_, jo_t) = register(&f.app, "jo").await;
    assert_eq!(
        f.put(&jo_t, m.id, body.clone()).await.status(),
        403,
        "not the poster"
    );
    // Not a quote at all.
    let msg_id = MessageId::new();
    sqlx::query("INSERT INTO messages (id, channel_id, author_id, content, created_at) VALUES (?, ?, ?, 'x', '2026-01-01T00:00:00.000Z')")
        .bind(msg_id.to_string())
        .bind(f.ch.id.to_string())
        .bind(f.alex.to_string())
        .execute(&f.app.db)
        .await
        .unwrap();
    assert_eq!(f.put(&f.alex_t, msg_id, body.clone()).await.status(), 400);
    // A poster who has left the server can't edit.
    sqlx::query("DELETE FROM server_members WHERE server_id = ? AND user_id = ?")
        .bind(f.server.to_string())
        .bind(f.alex.to_string())
        .execute(&f.app.db)
        .await
        .unwrap();
    assert_eq!(
        f.put(&f.alex_t, m.id, body.clone()).await.status(),
        403,
        "left the server"
    );
    sqlx::query("INSERT INTO server_members (server_id, user_id, joined_at) VALUES (?, ?, '2026-01-01T00:00:00.000Z')")
        .bind(f.server.to_string())
        .bind(f.alex.to_string())
        .execute(&f.app.db)
        .await
        .unwrap();
    // A deleted quote is gone.
    assert_eq!(f.delete(&f.alex_t, m.id).await.status(), 204);
    assert_eq!(f.put(&f.alex_t, m.id, body).await.status(), 404);
}

#[tokio::test]
async fn an_unknown_line_id_is_refused() {
    let f = fixture().await;
    let m = f.awarded_quote().await;
    let other = f.awarded_quote().await;
    let before = m.content.clone();
    for bad in [LineId::new(), other.quote.as_ref().unwrap().lines[0].id] {
        let mut lines = same_lines(&m);
        lines[0]["id"] = json!(bad);
        assert_eq!(
            f.put(&f.alex_t, m.id, json!({ "lines": lines }))
                .await
                .status(),
            400
        );
    }
    // The same id twice is refused too.
    let mut lines = same_lines(&m);
    lines[1]["id"] = lines[0]["id"].clone();
    assert_eq!(
        f.put(&f.alex_t, m.id, json!({ "lines": lines }))
            .await
            .status(),
        400
    );
    // Line rules apply to edits (no lines, a speaker outside the server) and change nothing.
    assert_eq!(
        f.put(&f.alex_t, m.id, json!({ "lines": [] }))
            .await
            .status(),
        400
    );
    let (stranger, _) = register(&f.app, "jo").await;
    let mut lines = same_lines(&m);
    lines[0]["speaker_id"] = json!(stranger);
    assert_eq!(
        f.put(&f.alex_t, m.id, json!({ "lines": lines }))
            .await
            .status(),
        400
    );
    let (content, edited): (String, Option<String>) =
        sqlx::query_as("SELECT content, edited_at FROM messages WHERE id = ?")
            .bind(m.id.to_string())
            .fetch_one(&f.app.db)
            .await
            .unwrap();
    assert_eq!((content, edited), (before, None));
}

#[tokio::test]
async fn deleting_a_quote_clears_reactions_awards_and_replies() {
    let f = fixture().await;
    let m = f.awarded_quote().await;
    let ids: Vec<LineId> = m
        .quote
        .as_ref()
        .unwrap()
        .lines
        .iter()
        .map(|l| l.id)
        .collect();
    let db = &f.app.db;
    let mid = m.id.to_string();
    sqlx::query("INSERT INTO reactions (message_id, user_id, emoji, created_at) VALUES (?, ?, '👍', '2026-01-01T00:00:00.000Z')")
        .bind(&mid)
        .bind(f.sam.to_string())
        .execute(db)
        .await
        .unwrap();
    let (r1, r2) = (MessageId::new(), MessageId::new());
    for id in [r1, r2] {
        sqlx::query("INSERT INTO messages (id, channel_id, author_id, content, created_at, thread_root_id) VALUES (?, ?, ?, 'r', '2026-01-03T00:00:00.000Z', ?)")
            .bind(id.to_string())
            .bind(f.ch.id.to_string())
            .bind(f.sam.to_string())
            .bind(&mid)
            .execute(db)
            .await
            .unwrap();
    }
    // An already-deleted reply gets no event.
    let gone = MessageId::new();
    sqlx::query("INSERT INTO messages (id, channel_id, author_id, content, created_at, thread_root_id, deleted_at) VALUES (?, ?, ?, '', '2026-01-03T00:00:00.000Z', ?, '2026-01-04T00:00:00.000Z')")
        .bind(gone.to_string())
        .bind(f.ch.id.to_string())
        .bind(f.sam.to_string())
        .bind(&mid)
        .execute(db)
        .await
        .unwrap();
    // A QotY option with a vote, and a past win.
    let (e1, e0) = (
        pulse_protocol::ids::QotyEventId::new(),
        pulse_protocol::ids::QotyEventId::new(),
    );
    for q in [
        format!(
            "INSERT INTO qoty_events (id, server_id, year, started_by, started_at) VALUES ('{e1}', '{}', 2026, '{}', '2026-12-01T00:00:00.000Z')",
            f.server, f.alex
        ),
        format!(
            "INSERT INTO qoty_events (id, server_id, year, started_by, started_at, ended_at, end_reason) VALUES ('{e0}', '{}', 2025, '{}', '2025-12-01T00:00:00.000Z', '2025-12-02T00:00:00.000Z', 'early')",
            f.server, f.alex
        ),
        format!(
            "INSERT INTO qoty_options (event_id, message_id, added_by) VALUES ('{e1}', '{mid}', '{}')",
            f.alex
        ),
        format!(
            "INSERT INTO qoty_votes (event_id, user_id, message_id) VALUES ('{e1}', '{}', '{mid}')",
            f.sam
        ),
        format!(
            "INSERT INTO qoty_winners (event_id, message_id, votes) VALUES ('{e0}', '{mid}', 3)"
        ),
        format!("INSERT INTO qoty_options (event_id, message_id) VALUES ('{e0}', '{mid}')"),
        format!(
            "INSERT INTO qoty_votes (event_id, user_id, message_id) VALUES ('{e0}', '{}', '{mid}')",
            f.alex
        ),
    ] {
        sqlx::query(&q).execute(db).await.unwrap();
    }

    let mut ws = hello(&f.app, &f.sam_t).await;
    assert_eq!(f.delete(&f.alex_t, m.id).await.status(), 204);

    let count = |q: String| async move {
        sqlx::query_scalar::<_, i64>(&q)
            .fetch_one(db)
            .await
            .unwrap()
    };
    assert_eq!(
        count(format!(
            "SELECT COUNT(*) FROM reactions WHERE message_id = '{mid}'"
        ))
        .await,
        0
    );
    for l in &ids {
        assert_eq!(f.awards_on(*l).await, 0);
    }
    assert_eq!(
        count(format!(
            "SELECT COUNT(*) FROM messages WHERE thread_root_id = '{mid}' AND deleted_at IS NULL"
        ))
        .await,
        0
    );
    assert_eq!(
        count(format!(
            "SELECT COUNT(*) FROM messages WHERE thread_root_id = '{mid}' AND content = ''"
        ))
        .await,
        3
    );
    assert_eq!(
        count(format!(
            "SELECT COUNT(*) FROM qoty_options WHERE message_id = '{mid}' AND event_id = '{e1}'"
        ))
        .await,
        0,
        "the live event loses it"
    );
    assert_eq!(
        count(format!(
            "SELECT COUNT(*) FROM qoty_votes WHERE event_id = '{e1}'"
        ))
        .await,
        0
    );
    assert_eq!(
        count(format!(
            "SELECT COUNT(*) FROM qoty_options WHERE event_id = '{e0}'"
        ))
        .await,
        1,
        "an ended event keeps its option"
    );
    assert_eq!(
        count(format!(
            "SELECT COUNT(*) FROM qoty_votes WHERE event_id = '{e0}'"
        ))
        .await,
        1,
        "and its votes"
    );
    assert_eq!(
        count(format!(
            "SELECT COUNT(*) FROM qoty_winners WHERE message_id = '{mid}'"
        ))
        .await,
        1,
        "wins stay"
    );

    let mut deleted: Vec<(MessageId, Option<MessageId>)> = vec![];
    while deleted.len() < 3 {
        match next_frame(&mut ws).await.expect("MessageDeleted") {
            ServerFrame::Event(Event::MessageDeleted {
                message_id,
                thread_root_id,
                channel_id,
                ..
            }) => {
                assert_eq!(channel_id, f.ch.id);
                deleted.push((message_id, thread_root_id));
            }
            _ => continue,
        }
    }
    assert_eq!(deleted[0], (m.id, None), "the quote's event comes first");
    let mut replies: Vec<_> = deleted[1..].to_vec();
    replies.sort_by_key(|d| d.0.to_string());
    let mut want = vec![(r1, Some(m.id)), (r2, Some(m.id))];
    want.sort_by_key(|d| d.0.to_string());
    assert_eq!(replies, want);
    // No event for the reply that was already deleted (or anything else).
    let extra = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            if let Some(ServerFrame::Event(Event::MessageDeleted { message_id, .. })) =
                next_frame(&mut ws).await
            {
                return message_id;
            }
        }
    })
    .await;
    assert!(extra.is_err(), "no fourth MessageDeleted: {extra:?}");
    // The quote reads back as deleted, without lines.
    let page: Vec<Message> =
        get_json(&f.app, &f.sam_t, &format!("/channels/{}/messages", f.ch.id)).await;
    let read = page.iter().find(|p| p.id == m.id).unwrap();
    assert!(read.deleted && read.quote.is_none());
}

// ---- replies (threads) ----

impl Fixture {
    async fn quote(&self) -> Message {
        self.post_lines(json!([self.say(self.sam, "x")]))
            .await
            .json()
            .await
            .unwrap()
    }
    async fn reply_raw(&self, token: &str, body: serde_json::Value) -> reqwest::Response {
        post_json(
            &self.app,
            token,
            &format!("/channels/{}/messages", self.ch.id),
            body,
        )
        .await
    }
    async fn reply(&self, token: &str, root: MessageId, text: &str) -> Message {
        let r = self
            .reply_raw(token, json!({ "content": text, "thread_root_id": root }))
            .await;
        assert_eq!(r.status(), 200, "reply");
        r.json().await.unwrap()
    }
    async fn replies(&self, token: &str, root: MessageId, query: &str) -> Vec<Message> {
        get_json(
            &self.app,
            token,
            &format!("/messages/{root}/replies{query}"),
        )
        .await
    }
    async fn seen(&self, token: &str, root: MessageId, reply: MessageId) -> reqwest::Response {
        post_json(
            &self.app,
            token,
            &format!("/messages/{root}/replies/seen"),
            json!({ "message_id": reply }),
        )
        .await
    }
    async fn state(&self, token: &str) -> pulse_protocol::rest::ReadState {
        let (_ws, ready) = hello_ready(&self.app, token).await;
        ready
            .read_states
            .into_iter()
            .find(|s| s.channel_id == self.ch.id)
            .expect("read state")
    }
    async fn react(&self, token: &str, msg: MessageId, emoji: &str) -> u16 {
        self.app
            .http
            .put(self.app.url(&format!("/messages/{msg}/reactions/{emoji}")))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }
}

#[tokio::test]
async fn replies_stay_out_of_the_main_list() {
    let f = fixture().await;
    let q = f.quote().await;
    let r = f.reply(&f.sam_t, q.id, "hello").await;
    assert_eq!(r.thread_root_id, Some(q.id));
    let page: Vec<Message> = get_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/messages", f.ch.id),
    )
    .await;
    assert_eq!(page.iter().map(|m| m.id).collect::<Vec<_>>(), vec![q.id]);
    let list = f.replies(&f.alex_t, q.id, "").await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, r.id);
    assert_eq!(list[0].thread_root_id, Some(q.id));
    assert_eq!(list[0].content, "hello");
}

#[tokio::test]
async fn replies_page_newest_first() {
    let f = fixture().await;
    let q = f.quote().await;
    let mut ids = vec![];
    for i in 0..3 {
        ids.push(f.reply(&f.sam_t, q.id, &format!("r{i}")).await.id);
    }
    let all = f.replies(&f.alex_t, q.id, "").await;
    assert_eq!(
        all.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![ids[2], ids[1], ids[0]]
    );
    let page = f
        .replies(&f.alex_t, q.id, &format!("?before={}&limit=1", ids[2]))
        .await;
    assert_eq!(page.iter().map(|m| m.id).collect::<Vec<_>>(), vec![ids[1]]);
    // Not a quote, or not visible: 404.
    let r = f
        .app
        .http
        .get(f.app.url(&format!("/messages/{}/replies", ids[0])))
        .bearer_auth(&f.alex_t)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let (_, outsider) = register(&f.app, "jo").await;
    let r = f
        .app
        .http
        .get(f.app.url(&format!("/messages/{}/replies", q.id)))
        .bearer_auth(&outsider)
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        200,
        "server channels are readable by any signed-in user, like list"
    );
}

#[tokio::test]
async fn replies_are_not_unread() {
    let f = fixture().await;
    let q = f.quote().await;
    let before = f.state(&f.alex_t).await;
    assert_eq!(before.unread, 0, "alex posted it");
    f.reply(&f.sam_t, q.id, "hi").await;
    let after = f.state(&f.alex_t).await;
    assert_eq!(after.unread, 0);
    assert_eq!(after.last_read_message_id, before.last_read_message_id);
    // sam's own reply doesn't move sam's channel read point past unread quotes.
    let sam = f.state(&f.sam_t).await;
    assert_eq!(sam.unread, 1, "alex's quote is still unread for sam");
}

#[tokio::test]
async fn a_reply_mention_counts_until_the_panel_is_seen() {
    let f = fixture().await;
    let q = f.quote().await;
    let mut alex_ws = hello(&f.app, &f.alex_t).await;
    let mut sam_ws = hello(&f.app, &f.sam_t).await;
    let r = f.reply(&f.sam_t, q.id, "hey @alex look").await;
    assert_eq!(f.state(&f.alex_t).await.mentions, 1);
    assert_eq!(f.seen(&f.alex_t, q.id, r.id).await.status(), 204);
    assert_eq!(f.state(&f.alex_t).await.mentions, 0);
    match loop {
        if let ServerFrame::Event(e @ Event::ThreadSeen { .. }) =
            next_frame(&mut alex_ws).await.expect("ThreadSeen")
        {
            break e;
        }
    } {
        Event::ThreadSeen {
            user_id,
            channel_id,
            root_id,
            last_seen_id,
            mentions_cleared,
            mentions,
        } => assert_eq!(
            (
                user_id,
                channel_id,
                root_id,
                last_seen_id,
                mentions_cleared,
                mentions
            ),
            (f.alex, f.ch.id, q.id, r.id, 1, Some(0))
        ),
        _ => unreachable!(),
    }
    // Only to alex.
    let leaked = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            if let Some(ServerFrame::Event(Event::ThreadSeen { user_id, .. })) =
                next_frame(&mut sam_ws).await
                && user_id == f.alex
            {
                return;
            }
        }
    })
    .await;
    assert!(leaked.is_err(), "sam must not get alex's ThreadSeen");
    // The quote now reads as seen by alex.
    let page: Vec<Message> = get_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/messages", f.ch.id),
    )
    .await;
    assert_eq!(page[0].quote.as_ref().unwrap().replies_seen, Some(r.id));
}

#[tokio::test]
async fn seen_only_moves_forward_and_validates() {
    let f = fixture().await;
    let q = f.quote().await;
    let other = f.quote().await;
    let (r1, r2) = (
        f.reply(&f.sam_t, q.id, "one @alex").await,
        f.reply(&f.sam_t, q.id, "two @alex").await,
    );
    let foreign = f.reply(&f.sam_t, other.id, "elsewhere").await;
    assert_eq!(f.state(&f.alex_t).await.mentions, 2);
    assert_eq!(f.seen(&f.alex_t, q.id, r2.id).await.status(), 204);
    assert_eq!(f.state(&f.alex_t).await.mentions, 0);
    // Backwards: a no-op (204, no event, nothing moves).
    let mut ws = hello(&f.app, &f.alex_t).await;
    assert_eq!(f.seen(&f.alex_t, q.id, r1.id).await.status(), 204);
    let evt = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            if let Some(ServerFrame::Event(Event::ThreadSeen { .. })) = next_frame(&mut ws).await {
                return;
            }
        }
    })
    .await;
    assert!(evt.is_err(), "a backwards seen publishes nothing");
    let seen: String = sqlx::query_scalar(
        "SELECT last_seen_id FROM thread_reads WHERE user_id = ? AND root_id = ?",
    )
    .bind(f.alex.to_string())
    .bind(q.id.to_string())
    .fetch_one(&f.app.db)
    .await
    .unwrap();
    assert_eq!(seen, r2.id.to_string());
    // A message of another thread, the root itself, or an unknown id: 400.
    for bad in [foreign.id, q.id, MessageId::new()] {
        assert_eq!(f.seen(&f.alex_t, q.id, bad).await.status(), 400, "{bad}");
    }
    // A root that isn't a quote: 404.
    assert_eq!(f.seen(&f.alex_t, r1.id, r2.id).await.status(), 404);
    // Unknown root: 404.
    assert_eq!(
        f.seen(&f.alex_t, MessageId::new(), r2.id).await.status(),
        404
    );
}

#[tokio::test]
async fn a_reply_needs_a_live_quote_in_this_channel() {
    let f = fixture().await;
    let q = f.quote().await;
    // A plain message.
    let plain = MessageId::new();
    sqlx::query("INSERT INTO messages (id, channel_id, author_id, content, created_at) VALUES (?, ?, ?, 'p', '2026-01-01T00:00:00.000Z')")
        .bind(plain.to_string())
        .bind(f.ch.id.to_string())
        .bind(f.sam.to_string())
        .execute(&f.app.db)
        .await
        .unwrap();
    // A system card.
    let system = MessageId::new();
    sqlx::query("INSERT INTO messages (id, channel_id, kind, content, created_at) VALUES (?, ?, 'system', 'card', '2026-01-01T00:00:00.000Z')")
        .bind(system.to_string())
        .bind(f.ch.id.to_string())
        .execute(&f.app.db)
        .await
        .unwrap();
    // A deleted quote.
    let dead = f.quote().await;
    assert_eq!(f.delete(&f.alex_t, dead.id).await.status(), 204);
    // A quote in another channel.
    let other = create_quotes(&f.app, &f.alex_t, f.server).await;
    let r = post_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/quotes", other.id),
        json!({ "lines": [f.say(f.sam, "y")] }),
    )
    .await;
    let elsewhere: Message = r.json().await.unwrap();
    for root in [plain, system, dead.id, elsewhere.id, MessageId::new()] {
        let r = f
            .reply_raw(&f.sam_t, json!({ "content": "x", "thread_root_id": root }))
            .await;
        assert_eq!(r.status(), 400, "{root}");
    }
    // A plain message without a thread is still refused here.
    assert_eq!(
        f.reply_raw(&f.sam_t, json!({ "content": "x" }))
            .await
            .status(),
        400
    );
    // The same rules as any message: content length, and a live quote works.
    assert_eq!(
        f.reply_raw(&f.sam_t, json!({ "content": " ", "thread_root_id": q.id }))
            .await
            .status(),
        400
    );
    f.reply(&f.sam_t, q.id, "fine").await;
}

#[tokio::test]
async fn a_reply_is_idempotent_by_nonce() {
    let f = fixture().await;
    let q = f.quote().await;
    let body = json!({ "content": "x", "thread_root_id": q.id, "nonce": "n-1" });
    let a: Message = f
        .reply_raw(&f.sam_t, body.clone())
        .await
        .json()
        .await
        .unwrap();
    let b: Message = f.reply_raw(&f.sam_t, body).await.json().await.unwrap();
    assert_eq!(a.id, b.id);
    assert_eq!(b.thread_root_id, Some(q.id));
    assert_eq!(f.replies(&f.sam_t, q.id, "").await.len(), 1);
}

#[tokio::test]
async fn reply_to_id_stays_in_the_thread() {
    let f = fixture().await;
    let (a, b) = (f.quote().await, f.quote().await);
    let in_a = f.reply(&f.sam_t, a.id, "a1").await;
    let in_b = f.reply(&f.sam_t, b.id, "b1").await;
    let bad = f
        .reply_raw(
            &f.alex_t,
            json!({ "content": "x", "thread_root_id": a.id, "reply_to_id": in_b.id }),
        )
        .await;
    assert_eq!(bad.status(), 400);
    // A reply to a reply in the same thread is fine, and so is one to the quote itself.
    for target in [in_a.id, a.id] {
        let ok = f
            .reply_raw(
                &f.alex_t,
                json!({ "content": "x", "thread_root_id": a.id, "reply_to_id": target }),
            )
            .await;
        assert_eq!(ok.status(), 200, "{target}");
        let m: Message = ok.json().await.unwrap();
        assert_eq!(
            (m.reply_to_id, m.thread_root_id),
            (Some(target), Some(a.id))
        );
    }
}

#[tokio::test]
async fn replies_get_normal_reactions() {
    let f = fixture().await;
    let q = f.quote().await;
    let r = f.reply(&f.sam_t, q.id, "hi").await;
    assert_eq!(f.react(&f.alex_t, r.id, "👍").await, 204);
    assert_eq!(f.react(&f.alex_t, r.id, "🎉").await, 204);
    let list = f.replies(&f.alex_t, q.id, "").await;
    let mut got: Vec<_> = list[0].reactions.iter().map(|x| x.emoji.clone()).collect();
    got.sort();
    assert_eq!(got.len(), 2, "both stay: {got:?}");
    // A quote is still one per person.
    assert_eq!(f.react(&f.alex_t, q.id, "👍").await, 204);
    assert_eq!(f.react(&f.alex_t, q.id, "🎉").await, 204);
    let page: Vec<Message> = get_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/messages", f.ch.id),
    )
    .await;
    assert_eq!(page[0].reactions.len(), 1);
    assert_eq!(page[0].reactions[0].emoji, "🎉");
}

#[tokio::test]
async fn quote_reply_counts() {
    let f = fixture().await;
    let q = f.quote().await;
    f.reply(&f.sam_t, q.id, "1").await;
    let r2 = f.reply(&f.alex_t, q.id, "2").await;
    let r3 = f.reply(&f.sam_t, q.id, "3").await;
    let read = |token: String| {
        let (app, ch) = (&f.app, f.ch.id);
        async move {
            let page: Vec<Message> =
                get_json(app, &token, &format!("/channels/{ch}/messages")).await;
            page.into_iter().next().unwrap().quote.unwrap()
        }
    };
    let quote = read(f.alex_t.clone()).await;
    assert_eq!((quote.reply_count, quote.last_reply_id), (3, Some(r3.id)));
    assert_eq!(f.delete(&f.sam_t, r3.id).await.status(), 204);
    let quote = read(f.alex_t.clone()).await;
    assert_eq!((quote.reply_count, quote.last_reply_id), (2, Some(r2.id)));
    // Posting a reply marks your place in the thread, deleted or not.
    assert_eq!(quote.replies_seen, Some(r2.id));
    assert_eq!(read(f.sam_t.clone()).await.replies_seen, Some(r3.id));
}

#[tokio::test]
async fn replies_never_change_a_score() {
    let f = fixture().await;
    let q = f.quote().await;
    let r = f.reply(&f.sam_t, q.id, "hi").await;
    assert_eq!(f.react(&f.alex_t, r.id, "🔥").await, 204);
    let page: Vec<Message> = get_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/messages", f.ch.id),
    )
    .await;
    assert!(page[0].reactions.is_empty());
}

#[tokio::test]
async fn reply_events_carry_the_root() {
    let f = fixture().await;
    let q = f.quote().await;
    let mut ws = hello(&f.app, &f.alex_t).await;
    let r = f.reply(&f.sam_t, q.id, "hi").await;
    let edit = f
        .app
        .http
        .patch(f.app.url(&format!("/messages/{}", r.id)))
        .bearer_auth(&f.sam_t)
        .json(&json!({ "content": "hi!" }))
        .send()
        .await
        .unwrap();
    assert_eq!(edit.status(), 200);
    let edited: Message = edit.json().await.unwrap();
    assert_eq!(edited.thread_root_id, Some(q.id));
    assert_eq!(f.delete(&f.sam_t, r.id).await.status(), 204);
    let mut seen = vec![];
    while seen.len() < 3 {
        match next_frame(&mut ws).await.expect("reply events") {
            ServerFrame::Event(Event::MessageCreated { message, .. }) if message.id == r.id => {
                seen.push(("created", message.thread_root_id));
            }
            ServerFrame::Event(Event::MessageUpdated { message }) if message.id == r.id => {
                seen.push(("updated", message.thread_root_id));
            }
            ServerFrame::Event(Event::MessageDeleted {
                message_id,
                thread_root_id,
                ..
            }) if message_id == r.id => seen.push(("deleted", thread_root_id)),
            _ => {}
        }
    }
    assert_eq!(
        seen,
        vec![
            ("created", Some(q.id)),
            ("updated", Some(q.id)),
            ("deleted", Some(q.id))
        ]
    );
}

#[tokio::test]
async fn replying_covers_earlier_mentions() {
    let f = fixture().await;
    let q = f.quote().await;
    f.reply(&f.sam_t, q.id, "hey @alex").await;
    assert_eq!(f.state(&f.alex_t).await.mentions, 1);
    let mut ws = hello(&f.app, &f.alex_t).await;
    let mine = f.reply(&f.alex_t, q.id, "here").await;
    assert_eq!(f.state(&f.alex_t).await.mentions, 0);
    loop {
        if let ServerFrame::Event(Event::ThreadSeen {
            last_seen_id,
            mentions_cleared,
            ..
        }) = next_frame(&mut ws).await.expect("ThreadSeen")
        {
            assert_eq!((last_seen_id, mentions_cleared), (mine.id, 1));
            break;
        }
    }
}

#[tokio::test]
async fn a_main_list_post_does_not_clear_a_reply_mention() {
    let f = fixture().await;
    let q = f.quote().await;
    f.reply(&f.sam_t, q.id, "hey @alex").await;
    assert_eq!(f.state(&f.alex_t).await.mentions, 1);
    // alex posts a newer quote: his channel read point passes the reply's id.
    let newer = f.quote().await;
    let st = f.state(&f.alex_t).await;
    assert_eq!(st.last_read_message_id, Some(newer.id));
    assert_eq!(st.mentions, 1, "only opening the thread clears it");
}

#[tokio::test]
async fn replies_under_a_deleted_quote_count_for_nothing() {
    let f = fixture().await;
    let q = f.quote().await;
    assert_eq!(f.delete(&f.alex_t, q.id).await.status(), 204);
    // A live reply with a mention under the deleted root (as a racing send could have left).
    let id = MessageId::new();
    sqlx::query("INSERT INTO messages (id, channel_id, author_id, content, created_at, thread_root_id) VALUES (?, ?, ?, 'r', '2026-01-03T00:00:00.000Z', ?)")
        .bind(id.to_string())
        .bind(f.ch.id.to_string())
        .bind(f.sam.to_string())
        .bind(q.id.to_string())
        .execute(&f.app.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO mentions (message_id, user_id) VALUES (?, ?)")
        .bind(id.to_string())
        .bind(f.alex.to_string())
        .execute(&f.app.db)
        .await
        .unwrap();
    assert_eq!(f.state(&f.alex_t).await.mentions, 0);
}

#[tokio::test]
async fn deleting_a_quote_races_replies_without_orphans() {
    let f = fixture().await;
    let q = f.quote().await;
    let sends = futures::future::join_all((0..8).map(|i| {
        let (f, q) = (&f, &q);
        async move {
            f.reply_raw(
                &f.sam_t,
                json!({ "content": format!("r{i} @alex"), "thread_root_id": q.id }),
            )
            .await
            .status()
            .as_u16()
        }
    }));
    let (codes, del) = tokio::join!(sends, f.delete(&f.alex_t, q.id));
    assert_eq!(del.status(), 204);
    assert!(codes.iter().all(|c| *c == 200 || *c == 400), "{codes:?}");
    let live: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE thread_root_id = ? AND deleted_at IS NULL",
    )
    .bind(q.id.to_string())
    .fetch_one(&f.app.db)
    .await
    .unwrap();
    assert_eq!(live, 0, "no live reply under a deleted quote");
    assert_eq!(f.state(&f.alex_t).await.mentions, 0);
}

#[tokio::test]
async fn seen_needs_server_membership() {
    let f = fixture().await;
    let q = f.quote().await;
    let r = f.reply(&f.sam_t, q.id, "hi").await;
    sqlx::query("DELETE FROM server_members WHERE server_id = ? AND user_id = ?")
        .bind(f.server.to_string())
        .bind(f.sam.to_string())
        .execute(&f.app.db)
        .await
        .unwrap();
    assert_eq!(f.seen(&f.sam_t, q.id, r.id).await.status(), 403);
}

#[tokio::test]
async fn a_reply_with_an_emoji_does_not_deadlock_ordered_writes() {
    let f = fixture().await;
    let q = f.quote().await;
    let r = tokio::time::timeout(
        Duration::from_secs(5),
        f.reply_raw(
            &f.sam_t,
            json!({ "content": "ha 👍", "thread_root_id": q.id }),
        ),
    )
    .await
    .expect("a reply with an emoji must not hang");
    assert_eq!(r.status(), 200);
    let reply: Message = r.json().await.unwrap();
    // Ordered writes still work afterwards.
    let react = tokio::time::timeout(Duration::from_secs(5), f.react(&f.alex_t, reply.id, "🎉"))
        .await
        .expect("a reaction after it must not hang");
    assert_eq!(react, 204);
}

#[tokio::test]
async fn deleting_a_reply_publishes_under_the_order_lock() {
    let f = fixture().await;
    let q = f.quote().await;
    let r = f.reply(&f.sam_t, q.id, "hey @alex").await;
    let mut ws = hello(&f.app, &f.alex_t).await;
    // While the lock is held, the delete must wait for it (it publishes under it)...
    let guard = f.app.hub.in_order().await;
    let (http, url, token) = (
        f.app.http.clone(),
        f.app.url(&format!("/messages/{}", r.id)),
        f.sam_t.clone(),
    );
    let del = tokio::spawn(async move {
        http.delete(url)
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status()
    });
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !del.is_finished(),
        "the delete should be waiting on in_order"
    );
    drop(guard);
    assert_eq!(del.await.unwrap(), 204);
    // Its event arrives.
    loop {
        if let ServerFrame::Event(Event::MessageDeleted {
            message_id,
            thread_root_id,
            ..
        }) = next_frame(&mut ws).await.expect("MessageDeleted")
        {
            assert_eq!((message_id, thread_root_id), (r.id, Some(q.id)));
            break;
        }
    }
}

// ---- one reaction per person, and awards ----

impl Fixture {
    async fn award_req(&self, token: &str, line: LineId, give: bool) -> reqwest::Response {
        let url = self.app.url(&format!("/quote-lines/{line}/award"));
        let r = if give {
            self.app.http.put(url)
        } else {
            self.app.http.delete(url)
        };
        r.bearer_auth(token).send().await.unwrap()
    }
    async fn award_status(&self, token: &str, line: LineId, give: bool) -> u16 {
        self.award_req(token, line, give).await.status().as_u16()
    }
    async fn unreact(&self, token: &str, msg: MessageId, emoji: &str) -> u16 {
        self.app
            .http
            .delete(self.app.url(&format!("/messages/{msg}/reactions/{emoji}")))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }
    fn line_of(&self, m: &Message, i: usize) -> LineId {
        m.quote.as_ref().unwrap().lines[i].id
    }
    async fn line_on_page(
        &self,
        token: &str,
        m: &Message,
        i: usize,
    ) -> pulse_protocol::rest::QuoteLine {
        let page: Vec<Message> = get_json(
            &self.app,
            token,
            &format!("/channels/{}/messages", self.ch.id),
        )
        .await;
        page.into_iter()
            .find(|p| p.id == m.id)
            .unwrap()
            .quote
            .unwrap()
            .lines
            .remove(i)
    }
}

type AwardEvent = (LineId, UserId, bool, u32, Vec<UserId>);

/// The `QuoteAwardChanged` events the socket delivers until it goes quiet, as
/// (line, who, given, count, awarders).
async fn award_events(mut ws: Socket) -> Vec<AwardEvent> {
    let mut out = Vec::new();
    while let Some(fr) = next_frame(&mut ws).await {
        if let ServerFrame::Event(Event::QuoteAwardChanged {
            line_id,
            user_id,
            given,
            count,
            awarders,
            ..
        }) = fr
        {
            out.push((line_id, user_id, given, count, awarders));
        }
    }
    out
}

/// What the socket says for a while about reactions, as short strings like `+😂1` and `-😂0`.
async fn reaction_events(ws: &mut Socket) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(f) = next_frame(ws).await {
        match f {
            ServerFrame::Event(Event::ReactionAdded { emoji, count, .. }) => {
                out.push(format!("+{emoji}{count}"))
            }
            ServerFrame::Event(Event::ReactionRemoved { emoji, count, .. }) => {
                out.push(format!("-{emoji}{count}"))
            }
            _ => {}
        }
    }
    out
}

#[tokio::test]
async fn one_reaction_per_quote() {
    let f = fixture().await;
    let q = f.quote().await;
    let mut ws = hello(&f.app, &f.sam_t).await;
    assert_eq!(f.react(&f.alex_t, q.id, "😂").await, 204);
    assert_eq!(f.react(&f.alex_t, q.id, "🔥").await, 204);
    assert_eq!(f.unreact(&f.alex_t, q.id, "🔥").await, 204);
    assert_eq!(f.unreact(&f.alex_t, q.id, "🔥").await, 204, "idempotent");
    assert_eq!(
        reaction_events(&mut ws).await,
        vec!["+😂1", "-😂0", "+🔥1", "-🔥0"]
    );
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reactions WHERE message_id = ?")
        .bind(q.id.to_string())
        .fetch_one(&f.app.db)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn awards_follow_the_rules() {
    let f = fixture().await;
    let q = f.quote().await; // alex posts, sam is the speaker
    let line = f.line_of(&q, 0);
    let alex_ws = hello(&f.app, &f.alex_t).await;

    assert_eq!(f.award_status(&f.alex_t, line, true).await, 204);
    // sam (the speaker) can't award their own line.
    let r = f.award_req(&f.sam_t, line, true).await;
    assert_eq!(r.status(), 403);
    let body: serde_json::Value = r.json().await.unwrap();
    assert!(
        body.to_string().contains("You can't award your own line."),
        "{body}"
    );
    // Awarding again changes nothing.
    assert_eq!(f.award_status(&f.alex_t, line, true).await, 204);
    let l = f.line_on_page(&f.alex_t, &q, 0).await;
    assert_eq!(
        (l.awards, l.awarders.clone(), l.awarded),
        (1, vec![f.alex], true)
    );

    assert_eq!(f.award_status(&f.alex_t, line, false).await, 204);
    assert_eq!(
        f.award_status(&f.alex_t, line, false).await,
        204,
        "idempotent"
    );
    let l = f.line_on_page(&f.alex_t, &q, 0).await;
    assert_eq!((l.awards, l.awarders, l.awarded), (0, vec![], false));
    assert_eq!(
        award_events(alex_ws).await,
        vec![
            (line, f.alex, true, 1, vec![f.alex]),
            (line, f.alex, false, 0, vec![]),
        ]
    );

    // Two awarders, oldest first, absolute in the event.
    let (jo, jo_t) = register(&f.app, "jo").await;
    let r = post_json(
        &f.app,
        &jo_t,
        &format!("/servers/{}/join", f.server),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 204);
    let ws = hello(&f.app, &f.sam_t).await;
    assert_eq!(f.award_status(&jo_t, line, true).await, 204);
    assert_eq!(f.award_status(&f.alex_t, line, true).await, 204);
    assert_eq!(
        award_events(ws).await.last().cloned(),
        Some((line, f.alex, true, 2, vec![jo, f.alex]))
    );
}

#[tokio::test]
async fn award_refusals() {
    let f = fixture().await;
    let q = f.quote().await;
    let line = f.line_of(&q, 0);
    // Not a member of the server.
    let (_, out_t) = register(&f.app, "outsider").await;
    assert_eq!(f.award_status(&out_t, line, true).await, 403);
    // Unknown line.
    assert_eq!(f.award_status(&f.alex_t, LineId::new(), true).await, 404);
    // Deleted quote.
    assert_eq!(f.delete(&f.alex_t, q.id).await.status(), 204);
    assert_eq!(f.award_status(&f.sam_t, line, true).await, 404);
    assert_eq!(f.award_status(&f.sam_t, line, false).await, 404);
}

#[tokio::test]
async fn a_linked_name_is_its_members_own_line() {
    let f = fixture().await;
    let m: Message = f
        .post_lines(json!([{ "speaker_name": "Sammy", "text": "hi" }]))
        .await
        .json()
        .await
        .unwrap();
    let line = f.line_of(&m, 0);
    sqlx::query(
        "INSERT INTO speaker_links (server_id, name_key, name, user_id, linked_by, linked_at)
         VALUES (?, 'sammy', 'Sammy', ?, ?, '2026-01-02T00:00:00.000Z')",
    )
    .bind(f.server.to_string())
    .bind(f.sam.to_string())
    .bind(f.alex.to_string())
    .execute(&f.app.db)
    .await
    .unwrap();
    assert_eq!(f.award_status(&f.sam_t, line, true).await, 403);
    assert_eq!(f.award_status(&f.alex_t, line, true).await, 204);
}

#[tokio::test]
async fn a_plain_name_line_can_be_awarded() {
    let f = fixture().await;
    let m: Message = f
        .post_lines(json!([
            { "speaker_name": "A Friend", "text": "one" },
            { "speaker_name": "a  FRIEND", "text": "two" },
            f.say(f.sam, "three"),
        ]))
        .await
        .json()
        .await
        .unwrap();
    let (l1, l2, l3) = (f.line_of(&m, 0), f.line_of(&m, 1), f.line_of(&m, 2));
    for l in [l1, l2] {
        assert_eq!(f.award_status(&f.sam_t, l, true).await, 204);
    }
    assert_eq!(f.award_status(&f.alex_t, l1, true).await, 204);
    assert_eq!(f.award_status(&f.alex_t, l3, true).await, 204);
    assert_eq!(f.awards_on(l1).await, 2);
    let totals = pulse_server::quotes::award_totals(&f.app.db, f.server)
        .await
        .unwrap();
    assert_eq!(
        totals,
        vec![
            pulse_protocol::rest::SpeakerCount {
                user_id: None,
                name: Some("a  FRIEND".into()),
                count: 3
            },
            pulse_protocol::rest::SpeakerCount {
                user_id: Some(f.sam),
                name: None,
                count: 1
            },
        ],
        "plain names count under the most recent spelling"
    );
    // A link moves the name's awards to the member and drops the member's own.
    sqlx::query(
        "INSERT INTO speaker_links (server_id, name_key, name, user_id, linked_by, linked_at)
         VALUES (?, 'a friend', 'A Friend', ?, ?, '2026-01-02T00:00:00.000Z')",
    )
    .bind(f.server.to_string())
    .bind(f.sam.to_string())
    .bind(f.alex.to_string())
    .execute(&f.app.db)
    .await
    .unwrap();
    let totals = pulse_server::quotes::award_totals(&f.app.db, f.server)
        .await
        .unwrap();
    assert_eq!(
        totals,
        vec![pulse_protocol::rest::SpeakerCount {
            user_id: Some(f.sam),
            name: None,
            count: 2
        }]
    );
    // Deleted quotes count for nothing.
    assert_eq!(f.delete(&f.alex_t, m.id).await.status(), 204);
    assert!(
        pulse_server::quotes::award_totals(&f.app.db, f.server)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn concurrent_awards_end_on_the_true_count() {
    let f = fixture().await;
    let q = f.quote().await; // sam is the speaker
    let line = f.line_of(&q, 0);
    let mut tokens = vec![f.alex_t.clone()];
    for n in 0..4 {
        let (_, t) = register(&f.app, &format!("user{n}")).await;
        let r = post_json(
            &f.app,
            &t,
            &format!("/servers/{}/join", f.server),
            json!({}),
        )
        .await;
        assert_eq!(r.status(), 204);
        tokens.push(t);
    }
    let mut ws = hello(&f.app, &f.sam_t).await;
    let calls = tokens.iter().map(|t| f.award_status(t, line, true));
    for s in futures::future::join_all(calls).await {
        assert_eq!(s, 204);
    }
    let mut counts = Vec::new();
    while let Some(fr) = next_frame(&mut ws).await {
        if let ServerFrame::Event(Event::QuoteAwardChanged {
            count, awarders, ..
        }) = fr
        {
            assert_eq!(awarders.len() as u32, count);
            counts.push(count);
        }
    }
    assert_eq!(counts, vec![1, 2, 3, 4, 5], "published in commit order");
    assert_eq!(f.awards_on(line).await, 5);
}

#[tokio::test]
async fn everyone_sees_awards_live() {
    let f = fixture().await;
    let (_, jo_t) = register(&f.app, "jo").await;
    let r = post_json(
        &f.app,
        &jo_t,
        &format!("/servers/{}/join", f.server),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 204);
    let q = f.quote().await;
    let line = f.line_of(&q, 0);
    let mut ws = hello(&f.app, &jo_t).await;
    assert_eq!(f.award_status(&f.alex_t, line, true).await, 204);
    loop {
        if let ServerFrame::Event(Event::QuoteAwardChanged {
            message_id,
            line_id,
            user_id,
            given,
            count,
            awarders,
            ..
        }) = next_frame(&mut ws).await.expect("QuoteAwardChanged")
        {
            assert_eq!(
                (message_id, line_id, user_id, given, count, awarders),
                (q.id, line, f.alex, true, 1, vec![f.alex])
            );
            break;
        }
    }
}

#[tokio::test]
async fn awarding_publishes_under_the_order_lock() {
    let f = fixture().await;
    let q = f.quote().await;
    let line = f.line_of(&q, 0);
    let guard = f.app.hub.in_order().await;
    let (http, url, token) = (
        f.app.http.clone(),
        f.app.url(&format!("/quote-lines/{line}/award")),
        f.alex_t.clone(),
    );
    let put = tokio::spawn(async move {
        http.put(url)
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status()
    });
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!put.is_finished(), "the award should wait on in_order");
    drop(guard);
    assert_eq!(put.await.unwrap(), 204);
}

#[tokio::test]
async fn taking_back_an_award_the_speaker_left_removes_it() {
    let f = fixture().await;
    let m: Message = f
        .post_lines(json!([{ "speaker_name": "Sammy", "text": "hi" }]))
        .await
        .json()
        .await
        .unwrap();
    let line = f.line_of(&m, 0);
    // sam awards the plain-name line, then a link makes sam its speaker: the award is uncounted.
    assert_eq!(f.award_status(&f.sam_t, line, true).await, 204);
    sqlx::query(
        "INSERT INTO speaker_links (server_id, name_key, name, user_id, linked_by, linked_at)
         VALUES (?, 'sammy', 'Sammy', ?, ?, '2026-01-02T00:00:00.000Z')",
    )
    .bind(f.server.to_string())
    .bind(f.sam.to_string())
    .bind(f.alex.to_string())
    .execute(&f.app.db)
    .await
    .unwrap();
    let mut ws = hello(&f.app, &f.alex_t).await;
    assert_eq!(f.award_status(&f.sam_t, line, false).await, 204);
    assert_eq!(f.awards_on(line).await, 0, "the take-back must commit");
    while let Some(fr) = next_frame(&mut ws).await {
        assert!(
            !matches!(fr, ServerFrame::Event(Event::QuoteAwardChanged { .. })),
            "an uncounted change publishes nothing"
        );
    }
}

/// M7: a server id that isn't one is "not found", not "not a member".
#[tokio::test]
async fn an_unknown_server_is_404_for_board_search_and_links() {
    let f = fixture().await;
    f.friend_quote().await;
    let unknown = ServerId::new();
    let status = |r: reqwest::Response| r.status().as_u16();
    let call = |m: reqwest::Method, path: String, body: Option<serde_json::Value>| {
        let mut r = f
            .app
            .http
            .request(m, f.app.url(&path))
            .bearer_auth(&f.alex_t);
        if let Some(b) = body {
            r = r.json(&b);
        }
        r.send()
    };
    assert_eq!(
        status(
            call(
                reqwest::Method::GET,
                format!("/servers/{unknown}/quotes/leaderboard"),
                None
            )
            .await
            .unwrap()
        ),
        404
    );
    assert_eq!(
        status(
            call(
                reqwest::Method::GET,
                format!("/servers/{unknown}/quotes/search?q=x"),
                None
            )
            .await
            .unwrap()
        ),
        404
    );
    assert_eq!(
        status(
            call(
                reqwest::Method::PUT,
                format!("/servers/{unknown}/speaker-links"),
                Some(json!({ "name": "a friend", "user_id": f.sam }))
            )
            .await
            .unwrap()
        ),
        404
    );
    assert_eq!(
        status(
            call(
                reqwest::Method::DELETE,
                format!("/servers/{unknown}/speaker-links/a%20friend"),
                None
            )
            .await
            .unwrap()
        ),
        404
    );
    // A real server you aren't in stays 403.
    let (_, out_t) = register(&f.app, "outsider").await;
    assert_eq!(f.link(&out_t, "a friend", f.alex).await, 403);
    assert_eq!(f.unlink(&out_t, "a friend").await, 403);
}

// ---- speaker links ----

impl Fixture {
    async fn link_req(&self, token: &str, name: &str, user: UserId) -> reqwest::Response {
        self.app
            .http
            .put(
                self.app
                    .url(&format!("/servers/{}/speaker-links", self.server)),
            )
            .bearer_auth(token)
            .json(&json!({ "name": name, "user_id": user }))
            .send()
            .await
            .unwrap()
    }
    async fn link(&self, token: &str, name: &str, user: UserId) -> u16 {
        self.link_req(token, name, user).await.status().as_u16()
    }
    async fn unlink(&self, token: &str, key: &str) -> u16 {
        self.app
            .http
            .delete(
                self.app
                    .url(&format!("/servers/{}/speaker-links/{key}", self.server)),
            )
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }
    /// A quote by alex with plain-name lines "A Friend" and "a  friend".
    async fn friend_quote(&self) -> Message {
        self.post_lines(json!([
            { "speaker_name": "A Friend", "text": "one" },
            { "speaker_name": "a  friend", "text": "two" },
        ]))
        .await
        .json()
        .await
        .unwrap()
    }
}

/// The `SpeakerLinksChanged` lists the socket delivers until it goes quiet.
async fn link_events(ws: &mut Socket) -> Vec<Vec<pulse_protocol::rest::SpeakerLink>> {
    let mut out = Vec::new();
    while let Some(fr) = next_frame(ws).await {
        if let ServerFrame::Event(Event::SpeakerLinksChanged { links, .. }) = fr {
            out.push(links);
        }
    }
    out
}

#[tokio::test]
async fn linking_counts_lines_for_the_member() {
    let f = fixture().await;
    let m = f.friend_quote().await;
    for i in 0..2 {
        assert_eq!(f.award_status(&f.alex_t, f.line_of(&m, i), true).await, 204);
    }
    assert_eq!(f.link(&f.alex_t, " A  Friend ", f.sam).await, 204);
    let totals = pulse_server::quotes::award_totals(&f.app.db, f.server)
        .await
        .unwrap();
    assert_eq!(
        totals,
        vec![pulse_protocol::rest::SpeakerCount {
            user_id: Some(f.sam),
            name: None,
            count: 2
        }]
    );
}

#[tokio::test]
async fn undo_returns_lines_to_the_name() {
    let f = fixture().await;
    let m = f.friend_quote().await;
    let line = f.line_of(&m, 0);
    assert_eq!(f.award_status(&f.alex_t, line, true).await, 204);
    assert_eq!(f.link(&f.alex_t, "a friend", f.sam).await, 204);
    assert_eq!(f.link(&f.alex_t, "A FRIEND", f.alex).await, 204);
    assert_eq!(f.unlink(&f.sam_t, "a friend").await, 204);
    let l = f.line_on_page(&f.alex_t, &m, 0).await;
    assert_eq!(l.speaker_name.as_deref(), Some("A Friend"));
    assert_eq!(l.speaker_id, None);
    let totals = pulse_server::quotes::award_totals(&f.app.db, f.server)
        .await
        .unwrap();
    assert_eq!(totals[0].name.as_deref(), Some("A Friend"));
    assert_eq!(totals[0].count, 1);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM speaker_links")
        .fetch_one(&f.app.db)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn anyone_can_repoint() {
    let f = fixture().await;
    f.friend_quote().await;
    assert_eq!(f.link(&f.alex_t, "a friend", f.sam).await, 204);
    let mut ws = hello(&f.app, &f.alex_t).await;
    // Same target again: nothing changes, nothing is published.
    assert_eq!(f.link(&f.sam_t, "A Friend", f.sam).await, 204);
    assert!(link_events(&mut ws).await.is_empty());
    // sam moves it to alex.
    assert_eq!(f.link(&f.sam_t, "A Friend", f.alex).await, 204);
    let events = link_events(&mut ws).await;
    assert_eq!(events.len(), 1);
    let [l] = events[0].as_slice() else {
        panic!("one link")
    };
    assert_eq!(
        (l.server_id, l.name_key.as_str(), l.name.as_str()),
        (f.server, "a friend", "A Friend")
    );
    assert_eq!((l.user_id, l.linked_by), (f.alex, f.sam));
}

#[tokio::test]
async fn self_awards_via_a_link_are_not_counted() {
    let f = fixture().await;
    let m = f
        .post_lines(json!([
            { "speaker_name": "A Friend", "text": "one" },
            { "speaker_name": "Other", "text": "two" },
        ]))
        .await
        .json::<Message>()
        .await
        .unwrap();
    let line = f.line_of(&m, 0);
    assert_eq!(f.award_status(&f.sam_t, line, true).await, 204);
    assert_eq!(f.link(&f.alex_t, "a friend", f.sam).await, 204);
    let l = f.line_on_page(&f.sam_t, &m, 0).await;
    assert_eq!((l.awards, l.awarders.len(), l.awarded), (0, 0, false));
    // The line is sam's own now, so sam can't award it again or another by the same name.
    assert_eq!(f.award_status(&f.sam_t, line, true).await, 403);
    let m2 = f
        .post_lines(json!([{ "speaker_name": "a friend", "text": "three" }]))
        .await
        .json::<Message>()
        .await
        .unwrap();
    assert_eq!(f.award_status(&f.sam_t, f.line_of(&m2, 0), true).await, 403);
    assert_eq!(f.unlink(&f.alex_t, "a friend").await, 204);
    let l = f.line_on_page(&f.sam_t, &m, 0).await;
    assert_eq!((l.awards, l.awarders), (1, vec![f.sam]));
}

#[tokio::test]
async fn link_rules() {
    let f = fixture().await;
    f.friend_quote().await;
    let (outsider, out_t) = register(&f.app, "outsider").await;
    // Not a member of the server.
    assert_eq!(f.link(&out_t, "a friend", f.sam).await, 403);
    assert_eq!(f.unlink(&out_t, "a friend").await, 403);
    // The user must be a member.
    assert_eq!(f.link(&f.alex_t, "a friend", outsider).await, 400);
    // The name must be on a live line in this server, and not empty.
    assert_eq!(f.link(&f.alex_t, "nobody", f.sam).await, 400);
    assert_eq!(f.link(&f.alex_t, "   ", f.sam).await, 400);
    assert_eq!(f.link(&f.alex_t, &"a".repeat(33), f.sam).await, 400);
    // A deleted quote's names don't count.
    let gone = f
        .post_lines(json!([{ "speaker_name": "Ghost", "text": "boo" }]))
        .await
        .json::<Message>()
        .await
        .unwrap();
    assert_eq!(f.delete(&f.alex_t, gone.id).await.status(), 204);
    assert_eq!(f.link(&f.alex_t, "ghost", f.sam).await, 400);
    // Unlinking what isn't linked.
    assert_eq!(f.unlink(&f.alex_t, "a friend").await, 404);
    // The name is kept as the request spelled it, trimmed.
    assert_eq!(f.link(&f.alex_t, "  A   FRIEND ", f.sam).await, 204);
    let name: String = sqlx::query_scalar("SELECT name FROM speaker_links")
        .fetch_one(&f.app.db)
        .await
        .unwrap();
    assert_eq!(name, "A   FRIEND");
    // Unauthenticated.
    let r = f
        .app
        .http
        .put(f.app.url(&format!("/servers/{}/speaker-links", f.server)))
        .json(&json!({ "name": "a friend", "user_id": f.sam }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn links_are_in_ready_and_events() {
    let f = fixture().await;
    f.friend_quote().await;
    let mut ws = hello(&f.app, &f.sam_t).await;
    assert_eq!(f.link(&f.alex_t, "a friend", f.sam).await, 204);
    let events = link_events(&mut ws).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].len(), 1);
    let (_ws, ready) = hello_ready(&f.app, &f.sam_t).await;
    assert_eq!(ready.speaker_links, events[0]);
    assert_eq!(f.unlink(&f.alex_t, "a friend").await, 204);
    let (_ws, ready) = hello_ready(&f.app, &f.sam_t).await;
    assert!(ready.speaker_links.is_empty());
}

#[tokio::test]
async fn linking_publishes_under_the_order_lock() {
    let f = fixture().await;
    f.friend_quote().await;
    let guard = f.app.hub.in_order().await;
    let (http, url, token, sam) = (
        f.app.http.clone(),
        f.app.url(&format!("/servers/{}/speaker-links", f.server)),
        f.alex_t.clone(),
        f.sam,
    );
    let put = tokio::spawn(async move {
        http.put(url)
            .bearer_auth(token)
            .json(&json!({ "name": "a friend", "user_id": sam }))
            .send()
            .await
            .unwrap()
            .status()
    });
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!put.is_finished(), "linking should wait on in_order");
    drop(guard);
    assert_eq!(put.await.unwrap(), 204);
}

/// `MessageUpdated` quotes the socket delivers until it goes quiet.
async fn updated_quotes(ws: &mut Socket) -> Vec<Message> {
    let mut out = Vec::new();
    while let Some(fr) = next_frame(ws).await {
        if let ServerFrame::Event(Event::MessageUpdated { message }) = fr {
            out.push(message);
        }
    }
    out
}

#[tokio::test]
async fn link_changes_republish_quotes_whose_counts_moved() {
    let f = fixture().await;
    let m = f
        .post_lines(json!([{ "speaker_name": "A Friend", "text": "one" }]))
        .await
        .json::<Message>()
        .await
        .unwrap();
    let other = f
        .post_lines(json!([{ "speaker_name": "Other", "text": "two" }]))
        .await
        .json::<Message>()
        .await
        .unwrap();
    assert_eq!(f.award_status(&f.sam_t, f.line_of(&m, 0), true).await, 204);
    assert_eq!(
        f.award_status(&f.sam_t, f.line_of(&other, 0), true).await,
        204
    );
    let mut ws = hello(&f.app, &f.alex_t).await;

    assert_eq!(f.link(&f.alex_t, "a friend", f.sam).await, 204);
    let ev = updated_quotes(&mut ws).await;
    assert_eq!(ev.len(), 1, "only the quote whose count moved");
    assert_eq!(ev[0].id, m.id);
    assert_eq!(ev[0].quote.as_ref().unwrap().lines[0].awards, 0);

    assert_eq!(f.unlink(&f.alex_t, "a friend").await, 204);
    let ev = updated_quotes(&mut ws).await;
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].id, m.id);
    assert_eq!(ev[0].quote.as_ref().unwrap().lines[0].awards, 1);

    // Re-pointing from sam to alex moves it back (alex has no award there) and then forth again.
    assert_eq!(f.link(&f.alex_t, "a friend", f.sam).await, 204);
    assert_eq!(updated_quotes(&mut ws).await.len(), 1);
    assert_eq!(f.link(&f.alex_t, "a friend", f.alex).await, 204);
    let ev = updated_quotes(&mut ws).await;
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].quote.as_ref().unwrap().lines[0].awards, 1);
}

/// A link that committed answers 204 and still announces itself, even if one of the quotes it
/// republishes can't be read back: that quote is skipped (and logged), the others still go out.
#[tokio::test]
async fn a_committed_link_succeeds_when_a_quote_cant_be_read_back() {
    let f = fixture().await;
    let mut posted = Vec::new();
    for text in ["one", "two"] {
        let m: Message = f
            .post_lines(json!([
                { "speaker_name": "A Friend", "text": text },
                { "speaker_name": "Other", "text": "spare" },
            ]))
            .await
            .json()
            .await
            .unwrap();
        assert_eq!(f.award_status(&f.sam_t, f.line_of(&m, 0), true).await, 204);
        posted.push(m);
    }
    // In the link's own transaction, make the first quote unreadable (a line id that isn't one).
    sqlx::query(&format!(
        "CREATE TRIGGER spoil AFTER INSERT ON speaker_links BEGIN
           UPDATE quote_lines SET id = 'not-a-line-id' WHERE id = '{}';
         END",
        f.line_of(&posted[0], 1)
    ))
    .execute(&f.app.db)
    .await
    .unwrap();
    let mut ws = hello(&f.app, &f.alex_t).await;

    assert_eq!(f.link(&f.alex_t, "a friend", f.sam).await, 204);
    let mut links = 0;
    let mut updated = Vec::new();
    while let Some(fr) = next_frame(&mut ws).await {
        match fr {
            ServerFrame::Event(Event::SpeakerLinksChanged { .. }) => links += 1,
            ServerFrame::Event(Event::MessageUpdated { message }) => updated.push(message.id),
            _ => {}
        }
    }
    assert_eq!(links, 1);
    assert_eq!(
        updated,
        vec![posted[1].id],
        "the readable quote still goes out"
    );
    let users: Vec<String> = sqlx::query_scalar("SELECT user_id FROM speaker_links")
        .fetch_all(&f.app.db)
        .await
        .unwrap();
    assert_eq!(users, vec![f.sam.to_string()]);
}

#[tokio::test]
async fn a_refused_link_changes_and_publishes_nothing() {
    let f = fixture().await;
    f.friend_quote().await;
    let (outsider, out_t) = register(&f.app, "outsider").await;
    assert_eq!(f.link(&f.alex_t, "a friend", f.sam).await, 204);
    let mut ws = hello(&f.app, &f.alex_t).await;
    assert_eq!(f.link(&f.alex_t, "a friend", outsider).await, 400);
    assert_eq!(f.link(&f.alex_t, "nobody", f.alex).await, 400);
    assert_eq!(f.link(&out_t, "a friend", f.alex).await, 403);
    assert_eq!(f.unlink(&out_t, "a friend").await, 403);
    assert_eq!(f.unlink(&f.alex_t, "nobody").await, 404);
    let users: Vec<String> = sqlx::query_scalar("SELECT user_id FROM speaker_links")
        .fetch_all(&f.app.db)
        .await
        .unwrap();
    assert_eq!(users, vec![f.sam.to_string()]);
    assert!(link_events(&mut ws).await.is_empty());
}

#[tokio::test]
async fn unlinking_one_leaves_the_other_in_the_event() {
    let f = fixture().await;
    f.post_lines(json!([
        { "speaker_name": "A Friend", "text": "one" },
        { "speaker_name": "Other", "text": "two" },
    ]))
    .await;
    assert_eq!(f.link(&f.alex_t, "a friend", f.sam).await, 204);
    assert_eq!(f.link(&f.alex_t, "other", f.alex).await, 204);
    let mut ws = hello(&f.app, &f.alex_t).await;
    assert_eq!(f.unlink(&f.sam_t, "a friend").await, 204);
    let events = link_events(&mut ws).await;
    assert_eq!(events.len(), 1);
    let keys: Vec<&str> = events[0].iter().map(|l| l.name_key.as_str()).collect();
    assert_eq!(keys, vec!["other"]);
}

// ---- leaderboard and search ----

use pulse_protocol::rest::Leaderboard;

impl Fixture {
    async fn board_raw(&self, token: &str, query: &str) -> reqwest::Response {
        self.app
            .http
            .get(self.app.url(&format!(
                "/servers/{}/quotes/leaderboard{query}",
                self.server
            )))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
    }
    async fn board(&self, query: &str) -> Leaderboard {
        let r = self.board_raw(&self.alex_t, query).await;
        assert_eq!(r.status(), 200);
        r.json().await.unwrap()
    }
    async fn search_raw(&self, token: &str, query: &str) -> reqwest::Response {
        self.app
            .http
            .get(
                self.app
                    .url(&format!("/servers/{}/quotes/search?{query}", self.server)),
            )
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
    }
    /// Message ids found by `q`, newest first.
    async fn found(&self, q: &str) -> Vec<MessageId> {
        let r = self
            .search_raw(
                &self.alex_t,
                &format!("q={}", q.replace('%', "%25").replace('\\', "%5C")),
            )
            .await;
        assert_eq!(r.status(), 200, "search {q}");
        r.json::<Vec<Message>>()
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect()
    }
    async fn sql(&self, q: String) {
        sqlx::query(&q).execute(&self.app.db).await.unwrap();
    }
    async fn say_one(&self, text: &str) -> Message {
        self.post_lines(json!([self.say(self.sam, text)]))
            .await
            .json()
            .await
            .unwrap()
    }
    /// A third member.
    async fn kim(&self) -> (UserId, String) {
        let (kim, t) = register(&self.app, "kim").await;
        let r = post_json(
            &self.app,
            &t,
            &format!("/servers/{}/join", self.server),
            json!({}),
        )
        .await;
        assert_eq!(r.status(), 204);
        (kim, t)
    }
}

fn ids(v: &[Message]) -> Vec<MessageId> {
    v.iter().map(|m| m.id).collect()
}

#[tokio::test]
async fn leaderboard_orders_by_people_not_rows() {
    let f = fixture().await;
    let (_, kim_t) = f.kim().await;
    let (q1, q2, q3, _q4) = (
        f.say_one("one").await,
        f.say_one("two").await,
        f.say_one("three").await,
        f.say_one("four").await,
    );
    // q1: three people. q2 and q3: one person each, q2 with two emoji rows. q4: nobody.
    for t in [&f.alex_t, &f.sam_t, &kim_t] {
        assert_eq!(f.react(t, q1.id, "😂").await, 204);
    }
    assert_eq!(f.react(&f.sam_t, q2.id, "😂").await, 204);
    f.sql(format!(
        "INSERT INTO reactions (message_id, user_id, emoji, created_at) VALUES ('{}', '{}', '🔥', '2026-01-01T00:00:00.000Z')",
        q2.id, f.sam
    ))
    .await;
    assert_eq!(f.react(&f.alex_t, q3.id, "😂").await, 204);
    // q3 was said in an earlier year.
    f.sql(format!(
        "UPDATE quotes SET year = 2020 WHERE message_id = '{}'",
        q3.id
    ))
    .await;

    let b = f.board("").await;
    let this_year = q1.quote.as_ref().unwrap().year;
    assert_eq!(
        ids(&b.top_all_time),
        vec![q1.id, q3.id, q2.id],
        "score, then newest; q4 has none"
    );
    assert_eq!(ids(&b.top_year), vec![q1.id, q2.id]);
    let b = f.board("?year=2020").await;
    assert_eq!(ids(&b.top_year), vec![q3.id]);
    assert_eq!(ids(&b.top_all_time), vec![q1.id, q3.id, q2.id]);
    // The messages are as the caller sees them.
    assert!(b.top_all_time[0].quote.is_some());
    let b = f.board(&format!("?year={this_year}")).await;
    assert_eq!(ids(&b.top_year), vec![q1.id, q2.id]);
}

#[tokio::test]
async fn leaderboard_and_search_are_for_members_with_sane_input() {
    let f = fixture().await;
    let (_, outsider) = register(&f.app, "zed").await;
    assert_eq!(f.board_raw(&outsider, "").await.status(), 403);
    assert_eq!(f.search_raw(&outsider, "q=x").await.status(), 403);
    assert_eq!(f.board_raw(&f.alex_t, "?year=1999").await.status(), 400);
    assert_eq!(f.board_raw(&f.alex_t, "?year=100000").await.status(), 400);
    assert_eq!(f.board_raw(&f.alex_t, "?year=2024").await.status(), 200);
    for q in ["", "q=", "q=%20%20", "limit=5"] {
        assert_eq!(f.search_raw(&f.alex_t, q).await.status(), 400, "{q:?}");
    }
    let long = "a".repeat(101);
    assert_eq!(
        f.search_raw(&f.alex_t, &format!("q={long}")).await.status(),
        400
    );
    let ok = "a".repeat(100);
    assert_eq!(
        f.search_raw(&f.alex_t, &format!("q={ok}")).await.status(),
        200
    );
}

#[tokio::test]
async fn replies_and_deleted_quotes_dont_count() {
    let f = fixture().await;
    let keep = f.say_one("keep").await;
    let gone = f.say_one("gone").await;
    let other = f.say_one("other").await;
    assert_eq!(f.react(&f.sam_t, other.id, "😂").await, 204);
    // keep has no reactions of its own, only on a reply.
    let reply = MessageId::new();
    f.sql(format!(
        "INSERT INTO messages (id, channel_id, author_id, content, created_at, thread_root_id)
         VALUES ('{reply}', '{}', '{}', 'r', '2026-01-03T00:00:00.000Z', '{}')",
        f.ch.id, f.sam, keep.id
    ))
    .await;
    for u in [f.alex, f.sam] {
        f.sql(format!(
            "INSERT INTO reactions (message_id, user_id, emoji, created_at) VALUES ('{reply}', '{u}', '😂', '2026-01-01T00:00:00.000Z')"
        ))
        .await;
    }
    // gone is deleted with its reactions and award rows left in place.
    assert_eq!(f.react(&f.alex_t, gone.id, "😂").await, 204);
    assert_eq!(f.react(&f.sam_t, gone.id, "😂").await, 204);
    f.sql(format!(
        "UPDATE messages SET deleted_at = '2026-01-02T00:00:00.000Z' WHERE id = '{}'",
        gone.id
    ))
    .await;
    let line = f.line_of(&gone, 0);
    f.award(line, f.alex).await;

    let b = f.board("").await;
    assert_eq!(
        ids(&b.top_all_time),
        vec![other.id],
        "no replies, no deleted, no zero scores"
    );
    assert_eq!(ids(&b.top_year), vec![other.id]);
    // sam spoke all three lines; the deleted quote's line is out of both counts.
    let sam = |c: &pulse_protocol::rest::SpeakerCount| (c.user_id, c.count);
    assert_eq!(
        b.most_quoted.iter().map(sam).collect::<Vec<_>>(),
        vec![(Some(f.sam), 2)]
    );
    assert!(b.most_awarded.is_empty());
}

#[tokio::test]
async fn most_quoted_merges_linked_names() {
    let f = fixture().await;
    f.friend_quote().await;
    f.say_one("mine").await;
    f.post_lines(json!([{ "speaker_name": "Zed", "text": "z" }]))
        .await;
    let name = |c: &pulse_protocol::rest::SpeakerCount| (c.user_id, c.name.clone(), c.count);
    let b = f.board("").await;
    // The two spellings are one person; the latest spelling shows; ties by name.
    assert_eq!(
        b.most_quoted.iter().map(name).collect::<Vec<_>>(),
        vec![
            (None, Some("a  friend".into()), 2),
            (Some(f.sam), None, 1),
            (None, Some("Zed".into()), 1),
        ]
    );
    assert_eq!(f.link(&f.alex_t, "a friend", f.sam).await, 204);
    let b = f.board("").await;
    assert_eq!(
        b.most_quoted.iter().map(name).collect::<Vec<_>>(),
        vec![(Some(f.sam), None, 3), (None, Some("Zed".into()), 1)]
    );
}

#[tokio::test]
async fn most_awarded_is_award_totals_and_lists_are_capped() {
    let f = fixture().await;
    // Twelve plain names, one line each; eleven quotes with one reaction each.
    let mut quotes = Vec::new();
    for chunk in (0..12).collect::<Vec<_>>().chunks(6) {
        let lines: Vec<_> = chunk
            .iter()
            .map(|i| json!({ "speaker_name": format!("N{i:02}"), "text": "t" }))
            .collect();
        let m: Message = f.post_lines(json!(lines)).await.json().await.unwrap();
        for l in 0..chunk.len() {
            f.award(f.line_of(&m, l), f.sam).await;
        }
    }
    for i in 0..11 {
        let m = f.say_one(&format!("q{i}")).await;
        assert_eq!(f.react(&f.sam_t, m.id, "😂").await, 204);
        quotes.push(m.id);
    }
    let b = f.board("").await;
    assert_eq!((b.most_quoted.len(), b.most_awarded.len()), (10, 10));
    assert_eq!(b.top_all_time.len(), 10);
    assert_eq!(b.top_year.len(), 10);
    assert_eq!(
        ids(&b.top_all_time),
        quotes.iter().rev().take(10).copied().collect::<Vec<_>>()
    );
    let totals = pulse_server::quotes::award_totals(&f.app.db, f.server)
        .await
        .unwrap();
    assert_eq!(b.most_awarded, totals[..10]);
}

/// Seed a finished Quote of the Year for `year` with `winners` as (message, votes, voters).
async fn seed_event(f: &Fixture, year: i32, winners: &[(MessageId, &[UserId])]) {
    let event = pulse_protocol::ids::QotyEventId::new();
    f.sql(format!(
        "INSERT INTO qoty_events (id, server_id, year, started_by, started_at, ended_at, end_reason)
         VALUES ('{event}', '{}', {year}, '{}', '2026-01-01T00:00:00.000Z', '2026-01-02T00:00:00.000Z', 'early')",
        f.server, f.alex
    ))
    .await;
    for (m, voters) in winners {
        f.sql(format!(
            "INSERT INTO qoty_options (event_id, message_id) VALUES ('{event}', '{m}')"
        ))
        .await;
        f.sql(format!(
            "INSERT INTO qoty_winners (event_id, message_id, votes) VALUES ('{event}', '{m}', {})",
            voters.len()
        ))
        .await;
        for v in *voters {
            f.sql(format!("INSERT INTO qoty_votes (event_id, user_id, message_id) VALUES ('{event}', '{v}', '{m}')")).await;
        }
    }
}

#[tokio::test]
async fn winners_come_newest_year_first_with_voters() {
    let f = fixture().await;
    let (kim, _) = f.kim().await;
    let (a, b, c) = (
        f.say_one("a").await,
        f.say_one("b").await,
        f.say_one("c").await,
    );
    seed_event(&f, 2024, &[(a.id, &[f.sam, f.alex, kim])]).await;
    // A tie: both win. Voters in the order they voted.
    seed_event(&f, 2025, &[(b.id, &[kim]), (c.id, &[f.sam])]).await;
    // Deleted through the API: the quote goes, but the ended event keeps its votes.
    assert_eq!(f.delete(&f.alex_t, a.id).await.status(), 204);

    let board = f.board("").await;
    let w: Vec<_> = board
        .winners
        .iter()
        .map(|w| (w.year, w.message.id, w.votes, w.voters.clone()))
        .collect();
    assert_eq!(
        w,
        vec![
            (2025, c.id, 1, vec![f.sam]),
            (2025, b.id, 1, vec![kim]),
            (2024, a.id, 3, vec![f.sam, f.alex, kim]),
        ]
    );
    assert!(
        board.winners[0]
            .message
            .quote
            .as_ref()
            .unwrap()
            .wins
            .contains(&2025)
    );
    assert!(
        board.winners[2].message.deleted,
        "a deleted winner still shows, as deleted"
    );
    assert!(board.top_all_time.is_empty());
}

#[tokio::test]
async fn search_finds_text_and_speakers() {
    let f = fixture().await;
    let still = upload(&f, "/media/quote", png_bytes()).await;
    let hello = f.say_one("Hello World").await;
    let pic: Message = f
        .post_lines(
            json!([{ "speaker_name": "Bo", "media_hash": still.hash, "caption": "Sunset Glow" }]),
        )
        .await
        .json()
        .await
        .unwrap();
    let friend = f.friend_quote().await;
    let pct = f.say_one("only 50% off").await;
    let near = f.say_one("only 500 off").await;
    let under = f.say_one("a_b").await;
    let other = f.say_one("axb").await;
    let slash = f.say_one(r"c:\dir").await;
    let twice: Message = f
        .post_lines(json!([f.say(f.alex, "echo"), f.say(f.alex, "echo echo")]))
        .await
        .json()
        .await
        .unwrap();

    assert_eq!(
        f.found("wORLD").await,
        vec![hello.id],
        "case-insensitive, substring"
    );
    assert_eq!(f.found("sunset").await, vec![pic.id], "caption");
    assert_eq!(
        f.found("friend").await,
        vec![friend.id],
        "plain name; two lines, one hit"
    );
    assert_eq!(f.found("50%").await, vec![pct.id], "% is literal");
    assert_eq!(f.found("%").await, vec![pct.id]);
    assert_eq!(f.found("a_b").await, vec![under.id], "_ is literal");
    assert_eq!(f.found("_").await, vec![under.id]);
    assert_eq!(f.found(r"\").await, vec![slash.id]);
    assert_eq!(
        f.found(r"c:\dir").await,
        vec![slash.id],
        "backslash is literal"
    );
    assert_eq!(
        f.found("echo").await,
        vec![twice.id],
        "deduplicated per quote"
    );
    // Member usernames: sam's quotes by id, alex's own.
    let by_sam = f.found("sam").await;
    assert_eq!(
        by_sam,
        vec![slash.id, other.id, under.id, near.id, pct.id, hello.id],
        "newest first"
    );
    assert_eq!(f.found("  alex ").await, vec![twice.id], "trimmed");
    // A linked name is its member's.
    assert_eq!(f.link(&f.alex_t, "bo", f.sam).await, 204);
    assert!(!by_sam.contains(&pic.id));
    assert!(f.found("sam").await.contains(&pic.id));
    // Deleted quotes are not found.
    f.sql(format!(
        "UPDATE messages SET deleted_at = '2026-01-05T00:00:00.000Z' WHERE id = '{}'",
        hello.id
    ))
    .await;
    assert!(f.found("world").await.is_empty());
}

#[tokio::test]
async fn search_limit_defaults_to_and_stops_at_twenty() {
    let f = fixture().await;
    for _ in 0..21 {
        f.say_one("same").await;
    }
    assert_eq!(f.found("same").await.len(), 20);
    for (limit, n) in [(3, 3), (500, 20), (0, 1)] {
        let r = f
            .search_raw(&f.alex_t, &format!("q=same&limit={limit}"))
            .await;
        assert_eq!(r.status(), 200);
        assert_eq!(
            r.json::<Vec<Message>>().await.unwrap().len(),
            n,
            "limit {limit}"
        );
    }
}

#[tokio::test]
async fn search_folds_case_beyond_ascii() {
    let f = fixture().await;
    let m = f.say_one("L'école est finie").await;
    assert_eq!(f.found("ÉCOLE").await, vec![m.id]);
    assert_eq!(f.found("école").await, vec![m.id]);
}

#[tokio::test]
async fn default_year_is_the_current_utc_year() {
    let f = fixture().await;
    let (now, old) = (f.say_one("now").await, f.say_one("old").await);
    for m in [&now, &old] {
        assert_eq!(f.react(&f.sam_t, m.id, "😂").await, 204);
    }
    let year: i64 = sqlx::query_scalar("SELECT CAST(strftime('%Y', 'now') AS INTEGER)")
        .fetch_one(&f.app.db)
        .await
        .unwrap();
    f.sql(format!(
        "UPDATE quotes SET year = {year} WHERE message_id = '{}'",
        now.id
    ))
    .await;
    f.sql(format!(
        "UPDATE quotes SET year = {} WHERE message_id = '{}'",
        year - 1,
        old.id
    ))
    .await;
    assert_eq!(ids(&f.board("").await.top_year), vec![now.id]);
    assert_eq!(
        ids(&f.board(&format!("?year={}", year - 1)).await.top_year),
        vec![old.id]
    );
}

#[tokio::test]
async fn another_servers_quotes_stay_out() {
    let f = fixture().await;
    let ours = f.say_one("shared words").await;
    assert_eq!(f.react(&f.sam_t, ours.id, "😂").await, 204);
    let other = create_server(&f.app, &f.alex_t, "Other").await;
    let ch = create_quotes(&f.app, &f.alex_t, other.id).await;
    let theirs: Message = post_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/quotes", ch.id),
        json!({ "lines": [{ "speaker_name": "Elsewhere", "text": "shared words" }] }),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(f.react(&f.alex_t, theirs.id, "😂").await, 204);
    assert_eq!(f.react(&f.sam_t, theirs.id, "😂").await, 403);
    f.award(theirs.quote.as_ref().unwrap().lines[0].id, f.sam)
        .await;

    let b = f.board("").await;
    assert_eq!(ids(&b.top_all_time), vec![ours.id]);
    assert_eq!(ids(&b.top_year), vec![ours.id]);
    assert_eq!(
        b.most_quoted.iter().map(|c| c.count).collect::<Vec<_>>(),
        vec![1]
    );
    assert!(b.most_quoted[0].name.is_none());
    assert!(b.most_awarded.is_empty());
    assert_eq!(f.found("shared").await, vec![ours.id]);
    assert!(f.found("elsewhere").await.is_empty());
}

#[tokio::test]
async fn read_state_and_thread_seen_carry_the_absolute_mention_count() {
    let f = fixture().await;
    let (a, b) = (f.quote().await, f.quote().await);
    let ra = f.reply(&f.sam_t, a.id, "one @alex").await;
    f.reply(&f.sam_t, b.id, "two @alex").await;
    let mut alex_ws = hello(&f.app, &f.alex_t).await;
    // Alex posts a quote: the read point moves, and the two reply mentions stay counted.
    f.quote().await;
    let e = loop {
        if let ServerFrame::Event(e @ Event::ReadStateUpdated { .. }) =
            next_frame(&mut alex_ws).await.expect("ReadStateUpdated")
        {
            break e;
        }
    };
    assert!(
        matches!(
            e,
            Event::ReadStateUpdated {
                mentions: Some(2),
                ..
            }
        ),
        "{e:?}"
    );
    // Seeing one thread leaves the other's mention.
    assert_eq!(f.seen(&f.alex_t, a.id, ra.id).await.status(), 204);
    let e = loop {
        if let ServerFrame::Event(e @ Event::ThreadSeen { .. }) =
            next_frame(&mut alex_ws).await.expect("ThreadSeen")
        {
            break e;
        }
    };
    assert!(
        matches!(
            e,
            Event::ThreadSeen {
                mentions_cleared: 1,
                mentions: Some(1),
                ..
            }
        ),
        "{e:?}"
    );
    // Replying in the other thread covers its mention; the event counts it as gone.
    f.reply(&f.alex_t, b.id, "ok").await;
    let e = loop {
        if let ServerFrame::Event(e @ Event::ThreadSeen { .. }) =
            next_frame(&mut alex_ws).await.expect("ThreadSeen")
        {
            break e;
        }
    };
    assert!(
        matches!(
            e,
            Event::ThreadSeen {
                mentions: Some(0),
                ..
            }
        ),
        "{e:?}"
    );
}

#[tokio::test]
async fn ready_lists_the_seen_point_of_threads_with_a_counted_mention() {
    let f = fixture().await;
    let (a, b, c) = (f.quote().await, f.quote().await, f.quote().await);
    let ra1 = f.reply(&f.sam_t, a.id, "one @alex").await;
    let ra2 = f.reply(&f.sam_t, a.id, "two @alex").await;
    f.reply(&f.sam_t, b.id, "two @alex").await;
    f.reply(&f.sam_t, c.id, "no mention").await;
    let (_ws, ready) = hello_ready(&f.app, &f.alex_t).await;
    // Thread b was never seen; c has no mention at all.
    let mut got: Vec<_> = ready
        .thread_seen
        .iter()
        .map(|p| (p.root_id, p.last_seen_id))
        .collect();
    got.sort();
    let mut want = vec![(a.id, None), (b.id, None)];
    want.sort();
    assert_eq!(got, want);
    // A partial seen point is listed with its id; a full one drops the thread.
    assert_eq!(f.seen(&f.alex_t, a.id, ra1.id).await.status(), 204);
    let (_ws, ready) = hello_ready(&f.app, &f.alex_t).await;
    assert!(
        ready
            .thread_seen
            .iter()
            .any(|p| p.root_id == a.id && p.last_seen_id == Some(ra1.id))
    );
    assert_eq!(f.seen(&f.alex_t, a.id, ra2.id).await.status(), 204);
    let (_ws, ready) = hello_ready(&f.app, &f.alex_t).await;
    assert_eq!(
        ready
            .thread_seen
            .iter()
            .map(|p| p.root_id)
            .collect::<Vec<_>>(),
        vec![b.id]
    );
}

#[tokio::test]
async fn a_channel_count_ignores_other_channels_reply_mentions() {
    let f = fixture().await;
    let q = f.quote().await;
    f.reply(&f.sam_t, q.id, "hey @alex").await;
    // A second quotes channel with its own reply mention must not count here.
    let (_ch2, _root2) = other_channel_reply_mention(&f).await;
    let mut ws = hello(&f.app, &f.alex_t).await;
    f.quote().await; // moves alex's point in this channel
    let e = loop {
        if let ServerFrame::Event(e @ Event::ReadStateUpdated { .. }) =
            next_frame(&mut ws).await.expect("ReadStateUpdated")
        {
            break e;
        }
    };
    assert!(
        matches!(
            e,
            Event::ReadStateUpdated {
                mentions: Some(1),
                unread: Some(0),
                ..
            }
        ),
        "{e:?}"
    );
}

/// A second quotes channel in the fixture's server with a quote and a reply that mentions alex.
async fn other_channel_reply_mention(f: &Fixture) -> (Channel, Message) {
    let ch2 = create_quotes(&f.app, &f.alex_t, f.server).await;
    let q: Message = post_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/quotes", ch2.id),
        json!({ "lines": [f.say(f.sam, "x")] }),
    )
    .await
    .json()
    .await
    .unwrap();
    let r = post_json(
        &f.app,
        &f.sam_t,
        &format!("/channels/{}/messages", ch2.id),
        json!({ "content": "hey @alex", "thread_root_id": q.id }),
    )
    .await;
    assert_eq!(r.status(), 200);
    (ch2, q)
}

#[tokio::test]
async fn ready_does_not_list_threads_of_channels_without_a_read_state() {
    let f = fixture().await;
    let q = f.quote().await;
    f.reply(&f.sam_t, q.id, "hey @alex").await;
    let (ch2, root2) = other_channel_reply_mention(&f).await;
    let (_ws, ready) = hello_ready(&f.app, &f.alex_t).await;
    let mut roots: Vec<_> = ready.thread_seen.iter().map(|p| p.root_id).collect();
    roots.sort();
    let mut want = vec![q.id, root2.id];
    want.sort();
    assert_eq!(roots, want);
    // No read state for the channel: its mentions aren't counted, so its thread isn't listed.
    sqlx::query("DELETE FROM read_states WHERE user_id = ? AND channel_id = ?")
        .bind(f.alex.to_string())
        .bind(ch2.id.to_string())
        .execute(&f.app.db)
        .await
        .unwrap();
    let (_ws, ready) = hello_ready(&f.app, &f.alex_t).await;
    let roots: Vec<_> = ready.thread_seen.iter().map(|p| p.root_id).collect();
    assert_eq!(roots, vec![q.id]);
}

// ---- a committed change never answers 500 because a count couldn't be read afterwards (M5) ----

impl Fixture {
    /// From now on, any write to a read point (or a first thread seen point) leaves a read point
    /// that can't be parsed back, in the same transaction: the change commits, every later count
    /// read fails.
    async fn spoil_read_points(&self) {
        for t in [
            "CREATE TRIGGER spoil_rs_ins AFTER INSERT ON read_states BEGIN
               UPDATE read_states SET last_read_message_id = 'not-an-id' WHERE rowid = NEW.rowid;
             END",
            "CREATE TRIGGER spoil_rs_upd AFTER UPDATE OF last_read_message_id ON read_states BEGIN
               UPDATE read_states SET last_read_message_id = 'not-an-id' WHERE rowid = NEW.rowid;
             END",
            "CREATE TRIGGER spoil_tr AFTER INSERT ON thread_reads BEGIN
               UPDATE read_states SET last_read_message_id = 'not-an-id' WHERE user_id = NEW.user_id;
             END",
        ] {
            sqlx::query(t).execute(&self.app.db).await.unwrap();
        }
    }
    async fn rows(&self, sql: &str) -> i64 {
        sqlx::query_scalar(sql)
            .fetch_one(&self.app.db)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn a_posted_quote_answers_200_when_its_counts_cant_be_read() {
    let f = fixture().await;
    f.spoil_read_points().await;
    let r = f.post_lines(json!([f.say(f.sam, "landed")])).await;
    assert_eq!(r.status(), 200);
    let m: Message = r.json().await.unwrap();
    assert_eq!(m.quote.unwrap().lines[0].text.as_deref(), Some("landed"));
    assert_eq!(f.rows("SELECT COUNT(*) FROM quotes").await, 1);
}

#[tokio::test]
async fn a_reply_answers_200_when_its_counts_cant_be_read() {
    let f = fixture().await;
    let q = f.quote().await;
    f.spoil_read_points().await;
    let r = f
        .reply_raw(
            &f.sam_t,
            json!({ "content": "landed", "thread_root_id": q.id }),
        )
        .await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        f.rows("SELECT COUNT(*) FROM messages WHERE thread_root_id IS NOT NULL")
            .await,
        1
    );
}

#[tokio::test]
async fn a_plain_message_answers_200_when_its_counts_cant_be_read() {
    let f = fixture().await;
    let g = general(&f.app, &f.alex_t, f.server).await;
    f.spoil_read_points().await;
    let r = post_json(
        &f.app,
        &f.sam_t,
        &format!("/channels/{}/messages", g.id),
        json!({ "content": "landed" }),
    )
    .await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        f.rows("SELECT COUNT(*) FROM messages WHERE content = 'landed'")
            .await,
        1
    );
}

#[tokio::test]
async fn replies_seen_answers_204_when_its_counts_cant_be_read() {
    let f = fixture().await;
    let q = f.quote().await;
    let reply = f.reply(&f.sam_t, q.id, "hey").await;
    f.spoil_read_points().await;
    assert_eq!(f.seen(&f.alex_t, q.id, reply.id).await.status(), 204);
    assert_eq!(
        f.rows("SELECT COUNT(*) FROM thread_reads WHERE root_id IN (SELECT id FROM messages WHERE thread_root_id IS NULL)")
            .await,
        2,
        "sam's (from replying) and alex's"
    );
}

#[tokio::test]
async fn mark_read_answers_204_when_its_counts_cant_be_read() {
    let f = fixture().await;
    let g = general(&f.app, &f.alex_t, f.server).await;
    let m = send(&f.app, &f.sam_t, g.id, "hi").await;
    f.spoil_read_points().await;
    let r = post_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/read", g.id),
        json!({ "message_id": m.id }),
    )
    .await;
    assert_eq!(r.status(), 204);
}

/// M8: a reply isn't a place in the channel's main list, so it can't be its read point.
#[tokio::test]
async fn a_reply_cant_be_the_channel_read_point() {
    let f = fixture().await;
    let q = f.quote().await;
    let reply = f.reply(&f.sam_t, q.id, "newer than the quote").await;
    let path = format!("/channels/{}/read", f.ch.id);
    let read = |id: MessageId| post_json(&f.app, &f.alex_t, &path, json!({ "message_id": id }));
    assert_eq!(read(reply.id).await.status(), 400);
    assert_eq!(read(q.id).await.status(), 204);
}

// ---- the disk guard (M9, M25) ----

/// The test config with the disk guard tripped: every write that grows the database is refused.
fn disk_full() -> pulse_server::config::Config {
    let mut cfg = test_config();
    cfg.media_min_free_bytes = u64::MAX;
    cfg
}

/// M9: an edit can add lines and text, so a full disk refuses it and nothing changes.
#[tokio::test]
async fn editing_a_quote_is_refused_when_disk_is_low() {
    let f = fixture().await;
    let m = f.quote().await;
    let full = spawn_copy(&f.app, disk_full()).await;
    let mut lines = same_lines(&m);
    lines.push(json!({ "speaker_id": f.alex, "text": "x".repeat(4000) }));
    let r = full
        .http
        .put(full.url(&format!("/quotes/{}", m.id)))
        .bearer_auth(&f.alex_t)
        .json(&json!({ "lines": lines }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 507);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM quote_lines")
        .fetch_one(&full.db)
        .await
        .unwrap();
    assert_eq!(n, 1, "not edited");
}

/// M25: posting a quote and replying to one grow the database: both refused on a full disk.
#[tokio::test]
async fn posting_a_quote_or_a_reply_is_refused_when_disk_is_low() {
    let f = fixture().await;
    let q = f.quote().await;
    let full = spawn_copy(&f.app, disk_full()).await;
    let r = post_json(
        &full,
        &f.alex_t,
        &format!("/channels/{}/quotes", f.ch.id),
        json!({ "lines": [f.say(f.sam, "y")] }),
    )
    .await;
    assert_eq!(r.status(), 507, "quote");
    let r = post_json(
        &full,
        &f.sam_t,
        &format!("/channels/{}/messages", f.ch.id),
        json!({ "content": "ha", "thread_root_id": q.id }),
    )
    .await;
    assert_eq!(r.status(), 507, "reply");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&full.db)
        .await
        .unwrap();
    assert_eq!(n, 1, "only the first quote");
}
