//! Builds (#616): the `builds` channel kind, migration 0082, and posting, reading, editing and searching builds.
mod common;

use std::borrow::Cow;
use std::time::Duration;

use common::*;
use futures::SinkExt;
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::ServerId;
use pulse_protocol::rest::{Channel, ChannelKind};
use serde_json::json;
use sqlx::sqlite::SqliteConnectOptions;
use tokio_tungstenite::tungstenite::Message as Ws;

async fn hello(app: &TestApp, token: &str) -> Socket {
    hello_as(app, token, pulse_protocol::PROTOCOL_VERSION).await.0
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

    let later = pulse_server::times::rfc3339(
        jiff::Timestamp::now() + pulse_server::times::days(30),
    );
    let posts = [
        ("polls", json!({ "question": "Which?", "options": ["a", "b"], "ends_in": "day" })),
        ("plans", json!({ "title": "Raid", "starts_at": later })),
        ("wheel-spins", json!({ "options": ["a", "b"] })),
        ("scheduled", json!({ "content": "later", "send_at": later })),
    ];
    for (path, body) in posts {
        // the same request is fine in a text channel: the refusal is the builds channel's
        let r = post_json(&app, &alex, &format!("/channels/{}/{path}", text.id), body.clone()).await;
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

async fn migrator_upto(version: i64) -> (sqlx::migrate::Migrator, Cow<'static, [sqlx::migrate::Migration]>) {
    let mut migrator = sqlx::migrate::Migrator::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"),
    )
    .await
    .unwrap();
    let all = migrator.migrations.clone();
    assert!(all.iter().any(|m| m.version == 82), "0082 exists");
    migrator.migrations = Cow::Owned(all.iter().filter(|m| m.version <= version).cloned().collect());
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
        format!("INSERT INTO read_states (user_id, channel_id, last_read_message_id) VALUES ('U1', 'C1', 'M1')"),
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
    assert_eq!(channel_rows(&pool).await, before, "every column of every row");
    for index in ["channels_server", "channels_deleted", "channels_category", "builds_browse", "builds_game"] {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?")
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
    let cat: Option<String> = sqlx::query_scalar("SELECT category_id FROM channels WHERE id = 'C2'")
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
        format!("INSERT INTO users (id, username, password_hash, created_at) VALUES ('U', 'alex', 'x', '{ts}')"),
        format!("INSERT INTO servers (id, name, created_by, created_at) VALUES ('S', 'Main', 'U', '{ts}')"),
        format!("INSERT INTO channels (id, server_id, kind, name, position, created_at) VALUES ('C', 'S', 'text', 'general', 0, '{ts}')"),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(&*q)).execute(&mut conn).await.unwrap();
    }
    conn.close().await.unwrap();
    let mut conn = opts.foreign_keys(true).connect().await.unwrap();
    migrator.migrations = all;
    let err = migrator.run(&mut conn).await.unwrap_err().to_string();
    assert!(err.contains("CHECK constraint failed"), "refused by the guard: {err}");
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

fn channels_get(app: &TestApp, token: &str, path: &str, version: Option<u32>) -> reqwest::RequestBuilder {
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
    assert!(old.channels.iter().all(|c| c.id != ch.id), "no builds channel");
    assert!(old.read_states.iter().all(|r| r.channel_id != ch.id), "nor its read point");
    assert!(old.channels.iter().any(|c| c.kind == ChannelKind::Text), "the rest is there");
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
        got_new.iter().any(|e| matches!(e, Event::ChannelCreated { channel } if channel.id == ch.id)),
        "a new client hears of it"
    );
    let got_old = events_until(&mut old, "after").await;
    assert!(
        !got_old.iter().any(|e| matches!(e, Event::ChannelCreated { .. })),
        "{got_old:?}"
    );
    assert!(!got_old.iter().any(|e| serde_json::to_string(e).unwrap().contains("\"builds\"")));
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
    assert!(snapshot.layout.channels.iter().all(|p| p.channel_id != ch.id));
    assert!(snapshot.channels.iter().any(|c| c.kind == ChannelKind::Text));
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
    let new: Vec<Channel> = channels_get(&app, &alex, &path, Some(pulse_protocol::PROTOCOL_VERSION))
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
    let new: Vec<pulse_protocol::rest::DeletedChannel> =
        channels_get(&app, &alex, &deleted, Some(pulse_protocol::PROTOCOL_VERSION))
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
    let r = save(Some(pulse_protocol::PROTOCOL_VERSION)).send().await.unwrap();
    assert_eq!(r.status(), 409, "a current client names every channel");
    let r = save(None).send().await.unwrap();
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    let still: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channels WHERE id = ? AND deleted_at IS NULL")
        .bind(ch.id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(still, 1);
}
