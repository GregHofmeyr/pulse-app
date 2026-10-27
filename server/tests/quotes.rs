//! Quotes: the `quotes` channel kind and migration 0008.
mod common;

use std::borrow::Cow;
use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, MessageId};
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

// ---------- posting and reading ----------

use pulse_protocol::ids::{ServerId, UserId};
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
    let (outsider, _) = register(&f.app, "robin").await;
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
    let (_, robin) = register(&f.app, "robin").await;
    assert_eq!(
        f.post(&robin, body.clone()).await.status(),
        403,
        "non-member"
    );
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
    let (robin, _) = register(&f.app, "robin").await;
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
    // Oldest first: sam (the speaker, uncounted), alex, robin.
    for (giver, at) in [(f.sam, "01"), (f.alex, "02"), (robin, "03")] {
        exec(format!(
            "INSERT INTO quote_awards (line_id, giver_id, created_at) VALUES ('{l1}', '{giver}', '2026-01-01T00:00:{at}.000Z')"
        ))
        .await;
    }
    exec(format!(
        "INSERT INTO quote_awards (line_id, giver_id, created_at) VALUES ('{l2}', '{robin}', '2026-01-01T00:00:01.000Z')"
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
        vec![f.alex, robin],
        "own award not counted"
    );
    assert_eq!(q.lines[0].awards, 2);
    assert!(q.lines[0].awarded);
    assert_eq!(q.lines[1].awarders, vec![robin]);
    assert!(!q.lines[1].awarded);
    let q = read(f.sam_t.clone()).await;
    assert!(
        !q.lines[0].awarded,
        "sam's own award is not counted, so not awarded"
    );

    // A link makes robin the effective speaker of "A Friend": robin's award on line 2 drops.
    exec(format!(
        "INSERT INTO speaker_links (server_id, name_key, name, user_id, linked_by, linked_at)
         VALUES ('{}', 'a friend', 'A Friend', '{robin}', '{}', '2026-01-02T00:00:00.000Z')",
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
    exec(format!(
        "INSERT INTO qoty_events (id, server_id, year, started_by, started_at, ended_at, end_reason)
         VALUES ('e1', '{}', 2025, '{}', '2025-12-01T00:00:00.000Z', '2025-12-02T00:00:00.000Z', 'early')",
        f.server, f.alex
    ))
    .await;
    exec(format!(
        "INSERT INTO qoty_winners (event_id, message_id, votes) VALUES ('e1', '{mid}', 3)"
    ))
    .await;
    assert_eq!(read(f.alex_t.clone()).await.wins, vec![2025]);
}
