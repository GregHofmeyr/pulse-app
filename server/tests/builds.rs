//! Builds (#616): the `builds` channel kind, migration 0082, and posting, reading, editing and searching builds.
mod common;

use std::borrow::Cow;
use std::time::Duration;

use common::*;
use futures::SinkExt;
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::ServerId;
use pulse_protocol::rest::{Channel, ChannelKind, Message};
use serde_json::json;
use sqlx::sqlite::SqliteConnectOptions;
use tokio_tungstenite::tungstenite::Message as Ws;

async fn hello(app: &TestApp, token: &str) -> Socket {
    hello_as(app, token, pulse_protocol::PROTOCOL_VERSION)
        .await
        .0
}

/// A gateway connection whose Hello says `version`, and the Ready it got.
async fn hello_as(
    app: &TestApp,
    token: &str,
    version: u32,
) -> (Socket, pulse_protocol::gateway::Ready) {
    let (mut ws, _) = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap();
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: version,
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    loop {
        match next_frame_beating(&mut ws, Duration::from_secs(3)).await {
            Some(ServerFrame::Ready(r)) => return (ws, r),
            Some(_) => continue,
            None => panic!("expected Ready"),
        }
    }
}

async fn create_builds(app: &TestApp, token: &str, server: ServerId) -> Channel {
    let r = post_json(
        app,
        token,
        &format!("/servers/{server}/channels"),
        json!({ "kind": "builds", "name": "builds" }),
    )
    .await;
    assert_eq!(r.status(), 200, "create builds channel");
    r.json().await.unwrap()
}

/// Waits for the first event `pick` takes, skipping the rest.
async fn wait_for<T>(ws: &mut Socket, what: &str, pick: impl Fn(Event) -> Option<T>) -> T {
    loop {
        match next_frame_beating(ws, Duration::from_secs(3)).await {
            Some(ServerFrame::Event(e)) => {
                if let Some(t) = pick(e) {
                    return t;
                }
            }
            Some(_) => continue,
            None => panic!("expected {what}"),
        }
    }
}

#[tokio::test]
async fn creates_a_builds_channel() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let s = create_server(&app, &alex, "Main").await;
    assert_eq!(join_server(&app, &sam, s.id).await.status(), 200);

    let mut ws = hello(&app, &sam).await;
    let ch = create_builds(&app, &alex, s.id).await;
    assert_eq!(ch.kind, ChannelKind::Builds);
    let mode: String = sqlx::query_scalar("SELECT reaction_mode FROM channels WHERE id = ?")
        .bind(ch.id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(mode, "normal");
    let got = wait_for(&mut ws, "ChannelCreated", |e| match e {
        Event::ChannelCreated { channel } => Some(channel),
        _ => None,
    })
    .await;
    assert_eq!(got, ch);
}

/// A kind the server doesn't know (it reads as `Unknown`) is never made.
#[tokio::test]
async fn an_unknown_kind_is_refused() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let s = create_server(&app, &alex, "Main").await;
    let r = post_json(
        &app,
        &alex,
        &format!("/servers/{}/channels", s.id),
        json!({ "kind": "forum", "name": "forum" }),
    )
    .await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn plain_messages_are_refused_in_a_builds_channel() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let s = create_server(&app, &alex, "Main").await;
    let ch = create_builds(&app, &alex, s.id).await;
    let r = post_json(
        &app,
        &alex,
        &format!("/channels/{}/messages", ch.id),
        json!({ "content": "hi" }),
    )
    .await;
    assert_eq!(r.status(), 400);
    assert!(
        r.text()
            .await
            .unwrap()
            .contains("only builds can be posted here")
    );
}

#[tokio::test]
async fn polls_plans_wheels_and_forwards_are_refused_in_a_builds_channel() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let s = create_server(&app, &alex, "Main").await;
    let ch = create_builds(&app, &alex, s.id).await;
    let text = general(&app, &alex, s.id).await;
    let hi = send(&app, &alex, text.id, "hi").await;

    let later =
        pulse_server::times::rfc3339(jiff::Timestamp::now() + pulse_server::times::days(30));
    let posts = [
        (
            "polls",
            json!({ "question": "Which?", "options": ["a", "b"], "ends_in": "day" }),
        ),
        ("plans", json!({ "title": "Raid", "starts_at": later })),
        ("wheel-spins", json!({ "options": ["a", "b"] })),
        ("scheduled", json!({ "content": "later", "send_at": later })),
    ];
    for (path, body) in posts {
        // the same request is fine in a text channel: the refusal is the builds channel's
        let r = post_json(
            &app,
            &alex,
            &format!("/channels/{}/{path}", text.id),
            body.clone(),
        )
        .await;
        assert!(r.status().is_success(), "{path} in text: {}", r.status());
        let r = post_json(&app, &alex, &format!("/channels/{}/{path}", ch.id), body).await;
        assert_eq!(r.status(), 400, "{path}");
    }
    let r = post_json(
        &app,
        &alex,
        &format!("/messages/{}/forward", hi.id),
        json!({ "channel_id": ch.id }),
    )
    .await;
    assert_eq!(r.status(), 400, "forward");
}

/// Members who join later start at "all read" in a builds channel, as in any chat.
#[tokio::test]
async fn joining_gives_a_read_point_in_builds_channels() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let s = create_server(&app, &alex, "Main").await;
    let ch = create_builds(&app, &alex, s.id).await;
    assert_eq!(join_server(&app, &sam, s.id).await.status(), 200);
    let n: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM read_states WHERE channel_id = ? AND user_id = ?")
            .bind(ch.id.to_string())
            .bind(sam_id.to_string())
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(n, 1);
}

/// Every column of every channel row, as SQL literals.
async fn channel_rows(pool: &sqlx::SqlitePool) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT quote(id) || ',' || quote(server_id) || ',' || quote(kind) || ',' || quote(name) || ',' ||
                quote(position) || ',' || quote(created_at) || ',' || quote(reaction_mode) || ',' || quote(deleted_at)
                || ',' || quote(deleted_by) || ',' || quote(icon_hash) || ',' || quote(category_id) || ',' ||
                quote(bitrate) || ',' || quote(archived_at)
         FROM channels ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn migrator_upto(
    version: i64,
) -> (
    sqlx::migrate::Migrator,
    Cow<'static, [sqlx::migrate::Migration]>,
) {
    let mut migrator = sqlx::migrate::Migrator::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"),
    )
    .await
    .unwrap();
    let all = migrator.migrations.clone();
    assert!(all.iter().any(|m| m.version == 82), "0082 exists");
    migrator.migrations = Cow::Owned(
        all.iter()
            .filter(|m| m.version <= version)
            .cloned()
            .collect(),
    );
    (migrator, all)
}

#[tokio::test]
async fn migration_0082_keeps_every_channel_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(false);
    let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();
    let (migrator, _) = migrator_upto(81).await;
    migrator.run(&pool).await.unwrap();

    let ts = "2026-10-01T00:00:00.000Z";
    for q in [
        format!("INSERT INTO users (id, username, password_hash, created_at) VALUES ('U1', 'alex', 'x', '{ts}')"),
        format!("INSERT INTO users (id, username, password_hash, created_at) VALUES ('U2', 'sam', 'x', '{ts}')"),
        format!("INSERT INTO servers (id, name, created_by, created_at) VALUES ('S1', 'Main', 'U1', '{ts}')"),
        format!("INSERT INTO server_members (server_id, user_id, joined_at) VALUES ('S1', 'U1', '{ts}')"),
        format!("INSERT INTO categories (id, server_id, name, position, created_at) VALUES ('K1', 'S1', 'Games', 0, '{ts}')"),
        // one of every kind, with every column set where it can be
        format!(
            "INSERT INTO channels (id, server_id, kind, name, position, created_at, reaction_mode, category_id, archived_at)
             VALUES ('C1', 'S1', 'text', 'general', 3, '{ts}', 'one_per_person', 'K1', '{ts}')"
        ),
        format!(
            "INSERT INTO channels (id, server_id, kind, name, position, created_at, bitrate, category_id)
             VALUES ('C2', 'S1', 'voice', 'lounge', 1, '{ts}', 128, 'K1')"
        ),
        format!(
            "INSERT INTO channels (id, server_id, kind, name, position, created_at, deleted_at, deleted_by)
             VALUES ('C3', 'S1', 'quotes', 'quotes', 2, '{ts}', '{ts}', 'U1')"
        ),
        format!("INSERT INTO channels (id, server_id, kind, name, position, created_at) VALUES ('C4', 'S1', 'clips', 'clips', 4, '{ts}')"),
        format!("INSERT INTO channels (id, server_id, kind, name, position, created_at) VALUES ('C5', NULL, 'dm', NULL, 0, '{ts}')"),
        format!(
            "INSERT INTO channels (id, server_id, kind, name, position, created_at, icon_hash)
             VALUES ('C6', NULL, 'group', 'pals', 0, '{ts}', 'abc')"
        ),
        format!("INSERT INTO channel_members (channel_id, user_id, added_at) VALUES ('C5', 'U1', '{ts}')"),
        format!("INSERT INTO channel_members (channel_id, user_id, added_at) VALUES ('C5', 'U2', '{ts}')"),
        format!("INSERT INTO messages (id, channel_id, author_id, content, created_at) VALUES ('M1', 'C1', 'U1', 'hi', '{ts}')"),
        "INSERT INTO read_states (user_id, channel_id, last_read_message_id) VALUES ('U1', 'C1', 'M1')".to_string(),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(&*q))
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{q}: {e}"));
    }
    let before = channel_rows(&pool).await;
    pool.close().await;

    let pool = pulse_server::db::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    assert_eq!(
        channel_rows(&pool).await,
        before,
        "every column of every row"
    );
    for index in [
        "channels_server",
        "channels_deleted",
        "channels_category",
        "builds_browse",
        "builds_game",
    ] {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?",
        )
        .bind(index)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(n, 1, "{index}");
    }
    let broken: Vec<(String, i64, String, i64)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(broken.is_empty(), "{broken:?}");
    // the new kind fits, and the children still point at the rebuilt table
    sqlx::query("INSERT INTO channels (id, server_id, kind, name, position, created_at) VALUES ('C7', 'S1', 'builds', 'builds', 5, ?)")
        .bind(ts)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM channels WHERE id = 'C1'")
        .execute(&pool)
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0, "ON DELETE CASCADE still works");
    // deleting a category still lets go of its channels
    sqlx::query("DELETE FROM categories WHERE id = 'K1'")
        .execute(&pool)
        .await
        .unwrap();
    let cat: Option<String> =
        sqlx::query_scalar("SELECT category_id FROM channels WHERE id = 'C2'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(cat, None, "ON DELETE SET NULL still works");
}

#[tokio::test]
async fn migration_0082_refuses_to_run_with_foreign_keys_on() {
    use sqlx::{ConnectOptions, Connection};
    let dir = tempfile::tempdir().unwrap();
    let opts = SqliteConnectOptions::new()
        .filename(dir.path().join("old.db"))
        .create_if_missing(true)
        .foreign_keys(false);
    let mut conn = opts.connect().await.unwrap();
    let (mut migrator, all) = migrator_upto(81).await;
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
    ] {
        sqlx::query(sqlx::AssertSqlSafe(&*q))
            .execute(&mut conn)
            .await
            .unwrap();
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
    assert_eq!(v, 81, "0082 rolled back");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channels")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

// ---- Clients older than protocol 5 never meet a builds channel (#616) ----

const OLD: u32 = 4;

/// Every event `ws` gets until a `MessageCreated` with `sentinel` as its content.
async fn events_until(ws: &mut Socket, sentinel: &str) -> Vec<Event> {
    let mut out = vec![];
    loop {
        match next_frame_beating(ws, Duration::from_secs(3)).await {
            Some(ServerFrame::Event(Event::MessageCreated { message, .. }))
                if message.content == sentinel =>
            {
                return out;
            }
            Some(ServerFrame::Event(e)) => out.push(e),
            Some(_) => continue,
            None => panic!("expected the sentinel {sentinel:?}"),
        }
    }
}

fn channels_get(
    app: &TestApp,
    token: &str,
    path: &str,
    version: Option<u32>,
) -> reqwest::RequestBuilder {
    let r = app.http.get(app.url(path)).bearer_auth(token);
    match version {
        Some(v) => r.header(pulse_protocol::PROTOCOL_HEADER, v.to_string()),
        None => r,
    }
}

#[tokio::test]
async fn a_version_4_ready_leaves_builds_channels_out() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let s = create_server(&app, &alex, "Main").await;
    let ch = create_builds(&app, &alex, s.id).await;

    let (_, old) = hello_as(&app, &alex, OLD).await;
    assert!(
        old.channels.iter().all(|c| c.id != ch.id),
        "no builds channel"
    );
    assert!(
        old.read_states.iter().all(|r| r.channel_id != ch.id),
        "nor its read point"
    );
    assert!(
        old.channels.iter().any(|c| c.kind == ChannelKind::Text),
        "the rest is there"
    );
    // what a version-4 client reads it as: no kind it can't parse
    assert!(!serde_json::to_string(&old).unwrap().contains("\"builds\""));

    let (_, new) = hello_as(&app, &alex, pulse_protocol::PROTOCOL_VERSION).await;
    assert!(new.channels.iter().any(|c| c.id == ch.id));
}

#[tokio::test]
async fn a_version_4_client_hears_nothing_of_a_builds_channel() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let s = create_server(&app, &alex, "Main").await;
    assert_eq!(join_server(&app, &sam, s.id).await.status(), 200);
    let text = general(&app, &alex, s.id).await;
    let (mut old, _) = hello_as(&app, &sam, OLD).await;
    let mut new = hello(&app, &sam).await;

    let ch = create_builds(&app, &alex, s.id).await;
    send(&app, &alex, text.id, "after").await;
    let got_new = events_until(&mut new, "after").await;
    assert!(
        got_new
            .iter()
            .any(|e| matches!(e, Event::ChannelCreated { channel } if channel.id == ch.id)),
        "a new client hears of it"
    );
    let got_old = events_until(&mut old, "after").await;
    assert!(
        !got_old
            .iter()
            .any(|e| matches!(e, Event::ChannelCreated { .. })),
        "{got_old:?}"
    );
    assert!(
        !got_old
            .iter()
            .any(|e| serde_json::to_string(e).unwrap().contains("\"builds\""))
    );
}

#[tokio::test]
async fn a_version_4_join_leaves_builds_channels_out() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, kim) = register(&app, "kim").await;
    let s = create_server(&app, &alex, "Main").await;
    let ch = create_builds(&app, &alex, s.id).await;
    let (mut old, _) = hello_as(&app, &kim, OLD).await;
    assert_eq!(join_server(&app, &kim, s.id).await.status(), 200);
    let snapshot = wait_for(&mut old, "ServerJoined", |e| match e {
        Event::ServerJoined { snapshot, .. } => Some(snapshot),
        _ => None,
    })
    .await;
    assert!(snapshot.channels.iter().all(|c| c.id != ch.id));
    assert!(
        snapshot
            .layout
            .channels
            .iter()
            .all(|p| p.channel_id != ch.id)
    );
    assert!(
        snapshot
            .channels
            .iter()
            .any(|c| c.kind == ChannelKind::Text)
    );
}

#[tokio::test]
async fn rest_channel_lists_leave_builds_out_for_older_clients() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let s = create_server(&app, &alex, "Main").await;
    let ch = create_builds(&app, &alex, s.id).await;
    let path = format!("/servers/{}/channels", s.id);

    let old: Vec<Channel> = channels_get(&app, &alex, &path, None)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(old.iter().all(|c| c.id != ch.id));
    let new: Vec<Channel> =
        channels_get(&app, &alex, &path, Some(pulse_protocol::PROTOCOL_VERSION))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    assert!(new.iter().any(|c| c.id == ch.id));

    // and in Recently deleted
    let r = app
        .http
        .delete(app.url(&format!("/channels/{}", ch.id)))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    let deleted = format!("/servers/{}/deleted-channels", s.id);
    let old: Vec<pulse_protocol::rest::DeletedChannel> = channels_get(&app, &alex, &deleted, None)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(old.is_empty(), "{old:?}");
    let new: Vec<pulse_protocol::rest::DeletedChannel> = channels_get(
        &app,
        &alex,
        &deleted,
        Some(pulse_protocol::PROTOCOL_VERSION),
    )
    .send()
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(new.len(), 1);
}

/// An older client saves its server's order without the builds channels it never sees: the save lands and the
/// builds channel keeps its place. A current client's save must still name every channel.
#[tokio::test]
async fn an_older_clients_layout_save_needn_t_name_builds_channels() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let s = create_server(&app, &alex, "Main").await;
    let ch = create_builds(&app, &alex, s.id).await;
    let (_, ready) = hello_as(&app, &alex, pulse_protocol::PROTOCOL_VERSION).await;
    let layout = ready.layouts.iter().find(|l| l.server_id == s.id).unwrap();
    let mine: Vec<&Channel> = ready
        .channels
        .iter()
        .filter(|c| c.server_id == Some(s.id) && c.id != ch.id)
        .collect();
    let in_cat = |cat: Option<pulse_protocol::ids::CategoryId>| {
        let mut v: Vec<&&Channel> = mine.iter().filter(|c| c.category_id == cat).collect();
        v.sort_by_key(|c| c.position);
        v.into_iter().map(|c| c.id).collect::<Vec<_>>()
    };
    let body = json!({
        "revision": layout.revision,
        "uncategorised": in_cat(None),
        "categories": layout.categories.iter().map(|k| json!({ "id": k.id, "channels": in_cat(Some(k.id)) })).collect::<Vec<_>>(),
    });
    let save = |version: Option<u32>| {
        let r = app
            .http
            .put(app.url(&format!("/servers/{}/layout", s.id)))
            .bearer_auth(&alex)
            .json(&body);
        match version {
            Some(v) => r.header(pulse_protocol::PROTOCOL_HEADER, v.to_string()),
            None => r,
        }
    };
    let r = save(Some(pulse_protocol::PROTOCOL_VERSION))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409, "a current client names every channel");
    let r = save(None).send().await.unwrap();
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    let still: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM channels WHERE id = ? AND deleted_at IS NULL")
            .bind(ch.id.to_string())
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(still, 1);
}

// ---- Create, get and list (Task 2) ----

/// The smallest WebP header the signature check looks at, padded out.
fn webp(len: usize) -> Vec<u8> {
    let mut b = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
    b.resize(len.max(16), 7);
    b
}

/// Start an upload in `channel` and PUT `bytes` to it, as the client does.
async fn upload(
    app: &TestApp,
    token: &str,
    channel: pulse_protocol::ids::ChannelId,
    name: &str,
    content_type: &str,
    bytes: &[u8],
) -> pulse_protocol::ids::AttachmentId {
    let r = post_json(
        app,
        token,
        &format!("/channels/{channel}/attachments"),
        json!({ "name": name, "size": bytes.len(), "content_type": content_type }),
    )
    .await;
    assert_eq!(r.status(), 200, "start {name}");
    let up: pulse_protocol::rest::AttachmentUpload = r.json().await.unwrap();
    let r = reqwest::Client::new()
        .put(up.url.clone().unwrap())
        .header("content-type", &up.content_type)
        .body(bytes.to_vec())
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "PUT {}", r.status());
    up.id
}

async fn shot(
    app: &TestApp,
    token: &str,
    channel: pulse_protocol::ids::ChannelId,
) -> pulse_protocol::ids::AttachmentId {
    upload(app, token, channel, "shot.webp", "image/webp", &webp(64)).await
}

async fn post_build(
    app: &TestApp,
    token: &str,
    channel: pulse_protocol::ids::ChannelId,
    body: serde_json::Value,
) -> reqwest::Response {
    post_json(app, token, &format!("/channels/{channel}/builds"), body).await
}

async fn build_ok(
    app: &TestApp,
    token: &str,
    channel: pulse_protocol::ids::ChannelId,
    body: serde_json::Value,
) -> Message {
    let r = post_build(app, token, channel, body).await;
    assert_eq!(r.status(), 200, "create build");
    r.json().await.unwrap()
}

async fn add_game(
    app: &TestApp,
    token: &str,
    server: ServerId,
    name: &str,
) -> pulse_protocol::rest::Game {
    let r = post_json(
        app,
        token,
        &format!("/servers/{server}/games"),
        json!({ "name": name }),
    )
    .await;
    assert!(r.status().is_success(), "add game {}", r.status());
    r.json().await.unwrap()
}

async fn list(
    app: &TestApp,
    token: &str,
    channel: pulse_protocol::ids::ChannelId,
    query: &str,
) -> Vec<Message> {
    get_json(app, token, &format!("/channels/{channel}/builds{query}")).await
}

async fn message_of(r: reqwest::Response) -> String {
    let v: serde_json::Value = r.json().await.unwrap();
    v["message"].as_str().unwrap_or_default().to_string()
}

/// A server with alex (its owner) and sam in it, and a builds channel.
struct World {
    app: TestApp,
    alex: (pulse_protocol::ids::UserId, String),
    sam: (pulse_protocol::ids::UserId, String),
    server: ServerId,
    ch: Channel,
}

async fn world() -> World {
    let app = spawn().await;
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let s = create_server(&app, &alex.1, "Main").await;
    assert_eq!(join_server(&app, &sam.1, s.id).await.status(), 200);
    let ch = create_builds(&app, &alex.1, s.id).await;
    World {
        app,
        alex,
        sam,
        server: s.id,
        ch,
    }
}

#[tokio::test]
async fn creates_a_build_with_a_title_only() {
    let w = world().await;
    let mut ws = hello(&w.app, &w.sam.1).await;
    let m = build_ok(
        &w.app,
        &w.alex.1,
        w.ch.id,
        json!({ "title": "  Raven crit " }),
    )
    .await;
    let b = m.build.as_ref().unwrap();
    assert_eq!(b.title, "Raven crit");
    assert!(b.quick_drop && !b.title_auto);
    assert_eq!(m.content, "Raven crit");
    assert_eq!(m.author_id, Some(w.alex.0));
    let got = wait_for(&mut ws, "MessageCreated", |e| match e {
        Event::MessageCreated { message, .. } => Some(message),
        _ => None,
    })
    .await;
    assert_eq!(got.id, m.id);
    assert_eq!(got.build.unwrap().title, "Raven crit");
    // and it reads back the same
    let back: Message = get_json(&w.app, &w.sam.1, &format!("/builds/{}", m.id)).await;
    assert_eq!(back.build.unwrap().title, "Raven crit");
}

#[tokio::test]
async fn creates_a_build_from_a_link_only() {
    let w = world().await;
    let m = build_ok(
        &w.app,
        &w.alex.1,
        w.ch.id,
        json!({ "link_url": "https://youtu.be/abc12345678" }),
    )
    .await;
    let b = m.build.unwrap();
    assert_eq!(b.title, "youtu.be/abc12345678");
    assert!(b.title_auto && b.quick_drop);
    assert_eq!(b.link.unwrap().url, "https://youtu.be/abc12345678");
}

#[tokio::test]
async fn creates_a_build_from_an_image_only() {
    let w = world().await;
    let a = shot(&w.app, &w.alex.1, w.ch.id).await;
    let m = build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "attachments": [a] })).await;
    assert_eq!(m.build.as_ref().unwrap().title, "Untitled build");
    assert!(m.build.unwrap().title_auto);
    assert_eq!(m.attachments.len(), 1);
    assert_eq!(m.attachments[0].id, a);
}

#[tokio::test]
async fn a_build_with_game_and_notes_is_not_a_quick_drop() {
    let w = world().await;
    let g = add_game(&w.app, &w.alex.1, w.server, "The First Descendant").await;
    let m = build_ok(
        &w.app,
        &w.alex.1,
        w.ch.id,
        json!({ "title": "Raven crit", "game_id": g.id, "notes": "- crit rate\n- **reload**" }),
    )
    .await;
    let b = m.build.unwrap();
    assert!(!b.quick_drop && !b.game_guessed);
    assert_eq!(b.game_id, Some(g.id));
    assert_eq!(b.notes.as_deref(), Some("- crit rate\n- **reload**"));
    assert_eq!(
        m.content,
        "Raven crit\nThe First Descendant\n- crit rate\n- **reload**"
    );
    // a game alone ends it too
    let m = build_ok(
        &w.app,
        &w.alex.1,
        w.ch.id,
        json!({ "title": "Bunny", "game_id": g.id }),
    )
    .await;
    assert!(!m.build.unwrap().quick_drop);
}

#[tokio::test]
async fn build_inputs_are_checked() {
    let w = world().await;
    let other = create_server(&w.app, &w.sam.1, "Other").await;
    let elsewhere = add_game(&w.app, &w.sam.1, other.id, "Warframe").await;
    let long = |n: usize| "x".repeat(n);
    let shots: Vec<_> = {
        let mut v = vec![];
        for _ in 0..11 {
            v.push(shot(&w.app, &w.alex.1, w.ch.id).await);
        }
        v
    };
    let pdf = upload(
        &w.app,
        &w.alex.1,
        w.ch.id,
        "a.pdf",
        "application/pdf",
        b"%PDF-1.4 hello",
    )
    .await;
    for body in [
        json!({ "title": long(121) }),
        json!({ "title": "ok", "notes": long(4001) }),
        json!({ "title": "ok", "attachments": shots }),
        json!({ "link_url": "ftp://example.com/x" }),
        json!({ "link_url": format!("https://example.com/{}", long(2030)) }),
        json!({ "title": "ok", "game_id": elsewhere.id }),
        json!({ "title": "ok", "attachments": [pdf] }),
    ] {
        let r = post_build(&w.app, &w.alex.1, w.ch.id, body.clone()).await;
        assert_eq!(r.status(), 400, "{body}");
    }
    let r = post_build(&w.app, &w.alex.1, w.ch.id, json!({ "title": "   " })).await;
    assert_eq!(r.status(), 400);
    assert_eq!(
        message_of(r).await,
        "A build needs a title, a link or a screenshot."
    );
    let r = post_build(&w.app, &w.alex.1, w.ch.id, json!({})).await;
    assert_eq!(r.status(), 400);
    // a builds route on another kind of channel
    let text = general(&w.app, &w.alex.1, w.server).await;
    let r = post_build(&w.app, &w.alex.1, text.id, json!({ "title": "ok" })).await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn non_members_cant_add_or_see_builds() {
    let w = world().await;
    let (_, kim) = register(&w.app, "kim").await;
    let m = build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "title": "Raven crit" })).await;
    // what the clips routes answer someone outside the server
    let clips = post_json(
        &w.app,
        &w.alex.1,
        &format!("/servers/{}/channels", w.server),
        json!({ "kind": "clips", "name": "clips" }),
    )
    .await
    .json::<Channel>()
    .await
    .unwrap();
    let outsider = w
        .app
        .http
        .get(w.app.url(&format!("/channels/{}/clips", clips.id)))
        .bearer_auth(&kim)
        .send()
        .await
        .unwrap()
        .status();
    let r = post_build(&w.app, &kim, w.ch.id, json!({ "title": "mine" })).await;
    assert_eq!(r.status(), outsider, "create");
    for path in [
        format!("/builds/{}", m.id),
        format!("/channels/{}/builds", w.ch.id),
    ] {
        let r = w
            .app
            .http
            .get(w.app.url(&path))
            .bearer_auth(&kim)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), outsider, "{path}");
    }
}

#[tokio::test]
async fn lists_builds_newest_first_by_game_without_archived() {
    let w = world().await;
    let tfd = add_game(&w.app, &w.alex.1, w.server, "The First Descendant").await;
    let wf = add_game(&w.app, &w.alex.1, w.server, "Warframe").await;
    let a = build_ok(
        &w.app,
        &w.alex.1,
        w.ch.id,
        json!({ "title": "A", "game_id": tfd.id }),
    )
    .await;
    let b = build_ok(
        &w.app,
        &w.sam.1,
        w.ch.id,
        json!({ "title": "B", "game_id": wf.id }),
    )
    .await;
    let c = build_ok(
        &w.app,
        &w.alex.1,
        w.ch.id,
        json!({ "title": "C", "game_id": tfd.id }),
    )
    .await;
    // a comment is no build, and isn't listed
    let ids = |v: Vec<Message>| v.into_iter().map(|m| m.id).collect::<Vec<_>>();
    assert_eq!(
        ids(list(&w.app, &w.sam.1, w.ch.id, "").await),
        vec![c.id, b.id, a.id]
    );
    assert_eq!(
        ids(list(&w.app, &w.sam.1, w.ch.id, &format!("?game={}", tfd.id)).await),
        vec![c.id, a.id]
    );
    assert_eq!(
        ids(list(
            &w.app,
            &w.sam.1,
            w.ch.id,
            &format!("?before={}&limit=1", c.id)
        )
        .await),
        vec![b.id]
    );
    // archived ones stay out unless asked
    sqlx::query("UPDATE builds SET archived_at = '2026-01-01T00:00:00.000Z', archived_by = ? WHERE message_id = ?")
        .bind(w.alex.0.to_string())
        .bind(b.id.to_string())
        .execute(&w.app.db)
        .await
        .unwrap();
    assert_eq!(
        ids(list(&w.app, &w.sam.1, w.ch.id, "").await),
        vec![c.id, a.id]
    );
    assert_eq!(
        ids(list(&w.app, &w.sam.1, w.ch.id, "?archived=true").await),
        vec![c.id, b.id, a.id]
    );
}

#[tokio::test]
async fn a_build_counts_as_unread_for_others() {
    let w = world().await;
    build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "title": "Raven crit" })).await;
    let (_, ready) = hello_as(&w.app, &w.sam.1, pulse_protocol::PROTOCOL_VERSION).await;
    let st = ready
        .read_states
        .iter()
        .find(|r| r.channel_id == w.ch.id)
        .unwrap();
    assert_eq!(st.unread, 1);
    let (_, ready) = hello_as(&w.app, &w.alex.1, pulse_protocol::PROTOCOL_VERSION).await;
    let st = ready
        .read_states
        .iter()
        .find(|r| r.channel_id == w.ch.id)
        .unwrap();
    assert_eq!(st.unread, 0, "the author has read their own");
}

#[tokio::test]
async fn a_retried_create_gets_the_same_build() {
    let w = world().await;
    let body = json!({ "title": "Raven crit", "nonce": "n-1" });
    let a = build_ok(&w.app, &w.alex.1, w.ch.id, body.clone()).await;
    let b = build_ok(&w.app, &w.alex.1, w.ch.id, body).await;
    assert_eq!(a.id, b.id);
    assert_eq!(list(&w.app, &w.alex.1, w.ch.id, "").await.len(), 1);
}

/// A version-4 client hears nothing of a build, nor its comments' channel (#616).
#[tokio::test]
async fn a_version_4_client_hears_no_build() {
    let w = world().await;
    let text = general(&w.app, &w.alex.1, w.server).await;
    let (mut old, _) = hello_as(&w.app, &w.sam.1, OLD).await;
    build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "title": "Raven crit" })).await;
    send(&w.app, &w.alex.1, text.id, "after").await;
    let got = events_until(&mut old, "after").await;
    let id = w.ch.id.to_string();
    assert!(
        !got.iter()
            .any(|e| serde_json::to_string(e).unwrap().contains(&id)),
        "{got:?}"
    );
}

/// A forward names its original's kind of chat, which a version-4 client can't read: nothing leaves a builds channel
/// by forward yet (#616).
#[tokio::test]
async fn a_build_isnt_forwarded() {
    let w = world().await;
    let text = general(&w.app, &w.alex.1, w.server).await;
    let m = build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "title": "Raven crit" })).await;
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/messages/{}/forward", m.id),
        json!({ "channel_id": text.id }),
    )
    .await;
    assert_eq!(r.status(), 400);
}

// ---- Edit by the author (Task 4) ----

async fn put_build(
    app: &TestApp,
    token: &str,
    id: pulse_protocol::ids::MessageId,
    body: serde_json::Value,
) -> reqwest::Response {
    app.http
        .put(app.url(&format!("/builds/{id}")))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

async fn edit_ok(
    app: &TestApp,
    token: &str,
    id: pulse_protocol::ids::MessageId,
    body: serde_json::Value,
) -> Message {
    let r = put_build(app, token, id, body).await;
    assert_eq!(r.status(), 200, "edit build");
    r.json().await.unwrap()
}

#[tokio::test]
async fn only_the_author_edits_a_build() {
    let w = world().await;
    let m = build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "title": "Raven crit" })).await;
    let r = put_build(&w.app, &w.sam.1, m.id, json!({ "title": "Mine now" })).await;
    assert_eq!(r.status(), 403, "someone else's build, as any message");
    let mut ws = hello(&w.app, &w.sam.1).await;
    let e = edit_ok(
        &w.app,
        &w.alex.1,
        m.id,
        json!({ "title": "Raven crit v2", "notes": "swap the module" }),
    )
    .await;
    let b = e.build.as_ref().unwrap();
    assert_eq!(b.title, "Raven crit v2");
    assert!(b.updated_at.is_some());
    assert_eq!(e.content, "Raven crit v2\nswap the module");
    let got = wait_for(&mut ws, "MessageUpdated", |e| match e {
        Event::MessageUpdated { message } if message.id == m.id => Some(message),
        _ => None,
    })
    .await;
    assert_eq!(got.build.unwrap().title, "Raven crit v2");
    // an outsider doesn't learn it exists
    let (_, kim) = register(&w.app, "kim").await;
    let r = put_build(&w.app, &kim, m.id, json!({ "title": "x" })).await;
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn an_edit_keeps_the_builds_place_in_the_list() {
    let w = world().await;
    let a = build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "title": "A" })).await;
    let b = build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "title": "B" })).await;
    let c = build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "title": "C" })).await;
    edit_ok(&w.app, &w.alex.1, a.id, json!({ "title": "A2" })).await;
    let ids: Vec<_> = list(&w.app, &w.alex.1, w.ch.id, "")
        .await
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(ids, vec![c.id, b.id, a.id]);
}

#[tokio::test]
async fn edit_reorders_adds_and_removes_screenshots() {
    let w = world().await;
    let a = shot(&w.app, &w.alex.1, w.ch.id).await;
    let b = shot(&w.app, &w.alex.1, w.ch.id).await;
    let m = build_ok(
        &w.app,
        &w.alex.1,
        w.ch.id,
        json!({ "title": "Shots", "attachments": [a, b] }),
    )
    .await;
    let key: String = sqlx::query_scalar("SELECT key FROM attachments WHERE id = ?")
        .bind(a.to_string())
        .fetch_one(&w.app.db)
        .await
        .unwrap();
    let object = pulse_server::attachments::object_key(&key, false);
    assert!(w.app.s3.as_ref().unwrap().object(&object).is_some());
    let c = shot(&w.app, &w.alex.1, w.ch.id).await;
    let e = edit_ok(
        &w.app,
        &w.alex.1,
        m.id,
        json!({ "title": "Shots", "attachments": [b, c] }),
    )
    .await;
    let order: Vec<_> = e.attachments.iter().map(|x| x.id).collect();
    assert_eq!(order, vec![b, c]);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachments WHERE id = ?")
        .bind(a.to_string())
        .fetch_one(&w.app.db)
        .await
        .unwrap();
    assert_eq!(rows, 0, "a's row is gone");
    assert!(
        w.app.s3.as_ref().unwrap().object(&object).is_none(),
        "and its object"
    );
    // someone else's file, or one from another message, isn't taken
    let theirs = shot(&w.app, &w.sam.1, w.ch.id).await;
    let r = put_build(
        &w.app,
        &w.alex.1,
        m.id,
        json!({ "title": "Shots", "attachments": [b, theirs] }),
    )
    .await;
    assert_eq!(r.status(), 404);
    let r = put_build(
        &w.app,
        &w.alex.1,
        m.id,
        json!({ "title": "Shots", "attachments": [b, b] }),
    )
    .await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn quick_drop_ends_with_a_game_notes_or_mark_sorted_and_never_returns() {
    let w = world().await;
    let g = add_game(&w.app, &w.alex.1, w.server, "Warframe").await;
    let m = build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "title": "Quick" })).await;
    let e = edit_ok(&w.app, &w.alex.1, m.id, json!({ "title": "Quick v2" })).await;
    assert!(
        e.build.unwrap().quick_drop,
        "a new title alone is no detail"
    );
    let e = edit_ok(
        &w.app,
        &w.alex.1,
        m.id,
        json!({ "title": "Quick", "notes": "details" }),
    )
    .await;
    assert!(!e.build.unwrap().quick_drop);
    let e = edit_ok(&w.app, &w.alex.1, m.id, json!({ "title": "Quick" })).await;
    assert!(!e.build.unwrap().quick_drop, "and it never comes back");

    let m = build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "title": "Gamed" })).await;
    let e = edit_ok(
        &w.app,
        &w.alex.1,
        m.id,
        json!({ "title": "Gamed", "game_id": g.id }),
    )
    .await;
    assert!(!e.build.unwrap().quick_drop);

    let m = build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "title": "Sorted" })).await;
    let e = edit_ok(
        &w.app,
        &w.alex.1,
        m.id,
        json!({ "title": "Sorted", "mark_sorted": true }),
    )
    .await;
    assert!(!e.build.unwrap().quick_drop);
}

#[tokio::test]
async fn clearing_a_guessed_game_sticks() {
    let w = world().await;
    let g = add_game(&w.app, &w.alex.1, w.server, "Warframe").await;
    let m = build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "title": "Saryn" })).await;
    sqlx::query("UPDATE builds SET game_id = ?, game_guessed = 1 WHERE message_id = ?")
        .bind(g.id.to_string())
        .bind(m.id.to_string())
        .execute(&w.app.db)
        .await
        .unwrap();
    let e = edit_ok(
        &w.app,
        &w.alex.1,
        m.id,
        json!({ "title": "Saryn", "game_id": null }),
    )
    .await;
    let b = e.build.unwrap();
    assert_eq!((b.game_id, b.game_guessed), (None, false));
    // and picking it is no guess
    let e = edit_ok(
        &w.app,
        &w.alex.1,
        m.id,
        json!({ "title": "Saryn", "game_id": g.id }),
    )
    .await;
    let b = e.build.unwrap();
    assert_eq!((b.game_id, b.game_guessed), (Some(g.id), false));
}

#[tokio::test]
async fn a_changed_link_drops_what_was_fetched_of_the_old_one() {
    let w = world().await;
    let m = build_ok(
        &w.app,
        &w.alex.1,
        w.ch.id,
        json!({ "link_url": "https://example.com/old" }),
    )
    .await;
    sqlx::query(
        "UPDATE builds SET link_title = 'Old page', link_provider = 'generic' WHERE message_id = ?",
    )
    .bind(m.id.to_string())
    .execute(&w.app.db)
    .await
    .unwrap();
    // the same link keeps it
    let e = edit_ok(
        &w.app,
        &w.alex.1,
        m.id,
        json!({ "title": "Mine", "link_url": "https://example.com/old" }),
    )
    .await;
    assert_eq!(
        e.build.unwrap().link.unwrap().title.as_deref(),
        Some("Old page")
    );
    let e = edit_ok(
        &w.app,
        &w.alex.1,
        m.id,
        json!({ "title": "Mine", "link_url": "https://example.com/new" }),
    )
    .await;
    let l = e.build.as_ref().unwrap().link.clone().unwrap();
    assert_eq!((l.url.as_str(), l.title), ("https://example.com/new", None));
    let e = edit_ok(&w.app, &w.alex.1, m.id, json!({ "title": "Mine" })).await;
    assert!(e.build.unwrap().link.is_none());
    // the edit's checks are the create's
    let r = put_build(&w.app, &w.alex.1, m.id, json!({ "title": "" })).await;
    assert_eq!(r.status(), 400);
    let r = put_build(&w.app, &w.alex.1, m.id, json!({ "title": "x".repeat(121) })).await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn builds_refuse_the_message_edit_and_delete_routes() {
    let w = world().await;
    let m = build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "title": "Raven crit" })).await;
    let r = w
        .app
        .http
        .patch(w.app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(&w.alex.1)
        .json(&json!({ "content": "hijack" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    assert_eq!(
        message_of(r).await,
        "builds are edited with their own route"
    );
    let r = w
        .app
        .http
        .delete(w.app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(&w.alex.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    assert_eq!(
        message_of(r).await,
        "builds are deleted with their own route"
    );
}

// ---- Search (Task 6) ----

async fn search_all(app: &TestApp, token: &str, query: &str) -> Vec<Message> {
    get_json(app, token, &format!("/builds/search{query}")).await
}

#[tokio::test]
async fn channel_search_finds_title_notes_game_and_link_title() {
    let w = world().await;
    let g = add_game(&w.app, &w.alex.1, w.server, "Borderlands").await;
    let t = build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "title": "Raven crit" })).await;
    let n = build_ok(
        &w.app,
        &w.alex.1,
        w.ch.id,
        json!({ "title": "Moxxi", "notes": "slag every corrosive target" }),
    )
    .await;
    let gm = build_ok(
        &w.app,
        &w.alex.1,
        w.ch.id,
        json!({ "title": "Zane", "game_id": g.id }),
    )
    .await;
    let l = build_ok(
        &w.app,
        &w.alex.1,
        w.ch.id,
        json!({ "link_url": "https://example.com/p" }),
    )
    .await;
    // as the link's fetched page fills it in
    sqlx::query("UPDATE builds SET link_title = 'Wynncraft archer planner' WHERE message_id = ?")
        .bind(l.id.to_string())
        .execute(&w.app.db)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE messages SET content = 'example.com/p\nWynncraft archer planner' WHERE id = ?",
    )
    .bind(l.id.to_string())
    .execute(&w.app.db)
    .await
    .unwrap();
    for (q, want) in [
        ("raven", t.id),
        ("corrosive", n.id),
        ("borderlands", gm.id),
        ("wynncraft", l.id),
    ] {
        let got: Vec<_> = list(&w.app, &w.sam.1, w.ch.id, &format!("?q={q}"))
            .await
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(got, vec![want], "{q}");
    }
    // a comment that says it isn't a build that does
    post_json(
        &w.app,
        &w.sam.1,
        &format!("/channels/{}/messages", w.ch.id),
        json!({ "content": "raven raven raven", "thread_root_id": n.id }),
    )
    .await;
    let got: Vec<_> = list(&w.app, &w.sam.1, w.ch.id, "?q=raven")
        .await
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(got, vec![t.id]);
}

#[tokio::test]
async fn search_everywhere_lists_current_before_archived() {
    let w = world().await;
    let old = build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "title": "Raven old" })).await;
    let a = build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "title": "Raven one" })).await;
    let b = build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "title": "Raven two" })).await;
    sqlx::query("UPDATE builds SET archived_at = '2026-01-01T00:00:00.000Z', archived_by = ? WHERE message_id = ?")
        .bind(w.alex.0.to_string())
        .bind(b.id.to_string())
        .execute(&w.app.db)
        .await
        .unwrap();
    let got: Vec<_> = search_all(&w.app, &w.sam.1, "?q=raven")
        .await
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(got, vec![a.id, old.id, b.id]);
    let got = search_all(&w.app, &w.sam.1, "?q=raven&limit=1").await;
    assert_eq!(got.len(), 1);
}

#[tokio::test]
async fn search_everywhere_filters_by_game_key_across_servers() {
    let w = world().await;
    let g1 = add_game(&w.app, &w.alex.1, w.server, "The First Descendant").await;
    let other = create_server(&w.app, &w.sam.1, "Other").await;
    let ch2 = create_builds(&w.app, &w.sam.1, other.id).await;
    let g2 = add_game(&w.app, &w.sam.1, other.id, "the  first descendant").await;
    let a = build_ok(
        &w.app,
        &w.alex.1,
        w.ch.id,
        json!({ "title": "Bunny", "game_id": g1.id }),
    )
    .await;
    let b = build_ok(
        &w.app,
        &w.sam.1,
        ch2.id,
        json!({ "title": "Gley", "game_id": g2.id }),
    )
    .await;
    build_ok(&w.app, &w.sam.1, ch2.id, json!({ "title": "No game" })).await;
    let mut got: Vec<_> = search_all(&w.app, &w.sam.1, "?game=the%20first%20descendant")
        .await
        .into_iter()
        .map(|m| m.id)
        .collect();
    got.sort();
    let mut want = vec![a.id, b.id];
    want.sort();
    assert_eq!(got, want);
    let got: Vec<_> = search_all(&w.app, &w.sam.1, "?game=the%20first%20descendant&q=gley")
        .await
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(got, vec![b.id]);
}

#[tokio::test]
async fn search_never_shows_another_servers_builds() {
    let w = world().await;
    build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "title": "Raven crit" })).await;
    let (_, kim) = register(&w.app, "kim").await;
    assert!(search_all(&w.app, &kim, "?q=raven").await.is_empty());
    // nor a channel's to someone outside it
    let r = w
        .app
        .http
        .get(w.app.url(&format!("/channels/{}/builds?q=raven", w.ch.id)))
        .bearer_auth(&kim)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

/// The link's fetched title lands while an edit is on its way: the edit's search text keeps it (it's read in the
/// edit's own write, not before).
#[tokio::test]
async fn an_edit_racing_the_link_fetch_keeps_the_link_title_searchable() {
    let w = world().await;
    let m = build_ok(&w.app, &w.alex.1, w.ch.id, json!({ "link_url": "https://example.com/p" })).await;
    let state = w.app.state();
    // hold publishing order, as a fetch writing its result does
    let order = state.hub.in_order().await;
    let (app_url, token, id) = (w.app.url(&format!("/builds/{}", m.id)), w.alex.1.clone(), m.id);
    let http = w.app.http.clone();
    let edit = tokio::spawn(async move {
        http.put(app_url)
            .bearer_auth(token)
            .json(&json!({ "title": "Mine", "link_url": "https://example.com/p" }))
            .send()
            .await
            .unwrap()
            .status()
    });
    // the edit has read the build and waits for its turn; the fetch's result lands meanwhile
    tokio::time::sleep(Duration::from_millis(300)).await;
    sqlx::query("UPDATE builds SET link_title = 'Wynncraft archer planner' WHERE message_id = ?")
        .bind(id.to_string())
        .execute(&w.app.db)
        .await
        .unwrap();
    drop(order);
    assert_eq!(edit.await.unwrap(), 200);
    let content: String = sqlx::query_scalar("SELECT content FROM messages WHERE id = ?")
        .bind(id.to_string())
        .fetch_one(&w.app.db)
        .await
        .unwrap();
    assert_eq!(content, "Mine\nWynncraft archer planner");
    let got: Vec<_> = list(&w.app, &w.sam.1, w.ch.id, "?q=wynncraft").await.into_iter().map(|m| m.id).collect();
    assert_eq!(got, vec![id]);
}
