//! Channel categories (#54): the update that gives existing servers their default categories,
//! the defaults a new server starts with, and the category and layout routes (#54, #55).
mod common;

use std::borrow::Cow;
use std::time::Duration;

use common::*;
use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::{CategoryId, ChannelId, ServerId, UserId};
use pulse_protocol::rest::{Category, Channel, ChannelKind, ChannelPlacement, ServerLayout};
use pulse_server::servers::categories::layout_of;
use serde_json::json;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;
use tokio::sync::mpsc;

const ALEX: &str = "01J0000000000000000000USER";
const MAIN: &str = "01J000000000000000000MA1N0";
const SIDE: &str = "01J000000000000000000S1DE0";
const SPARE: &str = "01J000000000000000000SPARE";

/// A database as the release before categories left it, and the migrator that finishes it.
async fn before_categories() -> (tempfile::TempDir, SqlitePool, sqlx::migrate::Migrator) {
    let dir = tempfile::tempdir().unwrap();
    let opts = SqliteConnectOptions::new()
        .filename(dir.path().join("old.db"))
        .create_if_missing(true)
        .foreign_keys(false);
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    let mut migrator = sqlx::migrate::Migrator::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"),
    )
    .await
    .unwrap();
    let all = migrator.migrations.clone();
    let categories = all
        .iter()
        .find(|m| m.description.contains("categories"))
        .expect("the categories migration exists")
        .version;
    migrator.migrations = Cow::Owned(
        all.iter()
            .filter(|m| m.version < categories)
            .cloned()
            .collect(),
    );
    migrator.run(&pool).await.unwrap();
    migrator.migrations = all;
    sql(
        &pool,
        &format!(
            "INSERT INTO users (id, username, password_hash, created_at, display_name) \
             VALUES ('{ALEX}', 'alex', 'x', '2026-10-01T00:00:00.000Z', 'alex')"
        ),
    )
    .await;
    (dir, pool, migrator)
}

async fn sql(pool: &SqlitePool, q: &str) {
    sqlx::query(q).execute(pool).await.unwrap();
}

async fn add_server(pool: &SqlitePool, id: &str, name: &str) {
    sql(
        pool,
        &format!(
            "INSERT INTO servers (id, name, created_by, created_at) \
             VALUES ('{id}', '{name}', '{ALEX}', '2026-10-01T00:00:00.000Z')"
        ),
    )
    .await;
}

/// A channel; `deleted` puts it in Recently deleted.
async fn add_channel(
    pool: &SqlitePool,
    id: &str,
    server: &str,
    kind: &str,
    name: &str,
    position: i64,
    deleted: bool,
) {
    let (at, by) = if deleted {
        ("'2026-11-01T00:00:00.000Z'", format!("'{ALEX}'"))
    } else {
        ("NULL", "NULL".to_string())
    };
    sql(
        pool,
        &format!(
            "INSERT INTO channels (id, server_id, kind, name, position, created_at, deleted_at, deleted_by) \
             VALUES ('{id}', '{server}', '{kind}', '{name}', {position}, '2026-10-01T00:00:00.000Z', {at}, {by})"
        ),
    )
    .await;
}

/// A server's categories: (id, name, position, default_kind), in order.
async fn categories_of(
    pool: &SqlitePool,
    server: &str,
) -> Vec<(String, String, i64, Option<String>)> {
    sqlx::query_as(
        "SELECT id, name, position, default_kind FROM categories WHERE server_id = ? \
         ORDER BY position",
    )
    .bind(server)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// A channel's (category name, position).
async fn placement(pool: &SqlitePool, channel: &str) -> (Option<String>, i64) {
    sqlx::query_as(
        "SELECT cat.name, c.position FROM channels c \
         LEFT JOIN categories cat ON cat.id = c.category_id WHERE c.id = ?",
    )
    .bind(channel)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn migration_0023_puts_every_channel_in_its_default_category_in_order() {
    let (_dir, pool, migrator) = before_categories().await;
    add_server(&pool, MAIN, "Main").await;
    add_server(&pool, SIDE, "Side").await;
    // Ids are made in insertion order: `news` is made before `memes` but sits below it, so the
    // new order must follow the old positions, not the ids.
    add_channel(
        &pool,
        "01J0000000000000000000C001",
        MAIN,
        "text",
        "general",
        0,
        false,
    )
    .await;
    add_channel(
        &pool,
        "01J0000000000000000000C002",
        MAIN,
        "text",
        "news",
        2,
        false,
    )
    .await;
    add_channel(
        &pool,
        "01J0000000000000000000C003",
        MAIN,
        "text",
        "memes",
        1,
        false,
    )
    .await;
    add_channel(
        &pool,
        "01J0000000000000000000C004",
        MAIN,
        "quotes",
        "quotes",
        0,
        false,
    )
    .await;
    add_channel(
        &pool,
        "01J0000000000000000000C005",
        MAIN,
        "clips",
        "clips",
        0,
        false,
    )
    .await;
    add_channel(
        &pool,
        "01J0000000000000000000C006",
        MAIN,
        "voice",
        "Lounge",
        0,
        false,
    )
    .await;
    add_channel(
        &pool,
        "01J0000000000000000000C007",
        MAIN,
        "voice",
        "Games",
        1,
        false,
    )
    .await;
    // Deleted, and its id is the smallest of Main's text channels: it must not become the
    // category's id, and keeps its old position.
    add_channel(
        &pool,
        "01J0000000000000000000C000",
        MAIN,
        "text",
        "old",
        7,
        true,
    )
    .await;
    add_channel(
        &pool,
        "01J0000000000000000000C008",
        SIDE,
        "text",
        "chat",
        3,
        false,
    )
    .await;
    add_channel(
        &pool,
        "01J0000000000000000000C009",
        SIDE,
        "voice",
        "Hangout",
        5,
        false,
    )
    .await;
    let count = || async {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM channels")
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    let before = count().await;

    migrator.run(&pool).await.unwrap();

    assert_eq!(count().await, before, "no channel is lost");
    let main = categories_of(&pool, MAIN).await;
    let shape: Vec<_> = main
        .iter()
        .map(|(_, n, p, k)| (n.as_str(), *p, k.as_deref()))
        .collect();
    assert_eq!(
        shape,
        vec![
            ("Text Channels", 0, Some("text")),
            ("Quotes", 1, Some("quotes")),
            ("Clips", 2, Some("clips")),
            ("Voice Channels", 3, Some("voice")),
        ]
    );
    assert_eq!(main[0].0, "01J0000000000000000000C001");
    let side = categories_of(&pool, SIDE).await;
    let shape: Vec<_> = side
        .iter()
        .map(|(_, n, p, k)| (n.as_str(), *p, k.as_deref()))
        .collect();
    assert_eq!(
        shape,
        vec![
            ("Text Channels", 0, Some("text")),
            ("Voice Channels", 1, Some("voice")),
        ]
    );

    let text = Some("Text Channels".to_string());
    let voice = Some("Voice Channels".to_string());
    assert_eq!(
        placement(&pool, "01J0000000000000000000C001").await,
        (text.clone(), 0)
    );
    assert_eq!(
        placement(&pool, "01J0000000000000000000C003").await,
        (text.clone(), 1)
    );
    assert_eq!(
        placement(&pool, "01J0000000000000000000C002").await,
        (text.clone(), 2)
    );
    assert_eq!(
        placement(&pool, "01J0000000000000000000C004").await,
        (Some("Quotes".into()), 0)
    );
    assert_eq!(
        placement(&pool, "01J0000000000000000000C005").await,
        (Some("Clips".into()), 0)
    );
    assert_eq!(
        placement(&pool, "01J0000000000000000000C006").await,
        (voice.clone(), 0)
    );
    assert_eq!(
        placement(&pool, "01J0000000000000000000C007").await,
        (voice.clone(), 1)
    );
    assert_eq!(
        placement(&pool, "01J0000000000000000000C000").await,
        (text.clone(), 7)
    );
    assert_eq!(
        placement(&pool, "01J0000000000000000000C008").await,
        (text, 0)
    );
    assert_eq!(
        placement(&pool, "01J0000000000000000000C009").await,
        (voice, 0)
    );
    // Each channel is in its own server's category.
    let crossed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM channels c JOIN categories cat ON cat.id = c.category_id \
         WHERE cat.server_id <> c.server_id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(crossed, 0);

    let revisions: Vec<i64> = sqlx::query_scalar("SELECT layout_revision FROM servers")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(revisions, vec![0, 0]);
    let ok: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(ok, "ok");
    let fk = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(fk.is_empty());
}

#[tokio::test]
async fn a_kind_without_channels_gets_no_category() {
    let (_dir, pool, migrator) = before_categories().await;
    add_server(&pool, SIDE, "Side").await;
    add_server(&pool, SPARE, "Spare").await;
    add_channel(
        &pool,
        "01J0000000000000000000C008",
        SIDE,
        "text",
        "chat",
        0,
        false,
    )
    .await;
    add_channel(
        &pool,
        "01J0000000000000000000C009",
        SIDE,
        "voice",
        "Hangout",
        0,
        false,
    )
    .await;
    add_channel(
        &pool,
        "01J0000000000000000000C010",
        SPARE,
        "text",
        "general",
        0,
        false,
    )
    .await;
    // Spare's only quotes channel is in Recently deleted.
    add_channel(
        &pool,
        "01J0000000000000000000C011",
        SPARE,
        "quotes",
        "quotes",
        0,
        true,
    )
    .await;

    migrator.run(&pool).await.unwrap();

    let kinds = |rows: Vec<(String, String, i64, Option<String>)>| {
        rows.into_iter()
            .map(|(_, _, _, k)| k.unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        kinds(categories_of(&pool, SIDE).await),
        vec!["text", "voice"]
    );
    assert_eq!(kinds(categories_of(&pool, SPARE).await), vec!["text"]);
    assert_eq!(
        placement(&pool, "01J0000000000000000000C011").await,
        (None, 0)
    );
    let fk = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(fk.is_empty());
}

#[tokio::test]
async fn a_new_server_starts_with_text_and_voice_categories() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let s = create_server(&app, &alex, "Main").await;
    let chans: Vec<Channel> = get_json(&app, &alex, &format!("/servers/{}/channels", s.id)).await;
    let rows: Vec<(String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT id, name, position, default_kind FROM categories WHERE server_id = ? \
         ORDER BY position",
    )
    .bind(s.id.to_string())
    .fetch_all(&app.db)
    .await
    .unwrap();
    let shape: Vec<_> = rows
        .iter()
        .map(|(_, n, p, k)| (n.as_str(), *p, k.as_deref()))
        .collect();
    assert_eq!(
        shape,
        vec![
            ("Text Channels", 0, Some("text")),
            ("Voice Channels", 1, Some("voice")),
        ]
    );
    let in_cat = |name: &str| rows.iter().find(|r| r.1 == name).unwrap().0.clone();
    let general = chans
        .iter()
        .find(|c| c.name.as_deref() == Some("general"))
        .unwrap();
    assert_eq!(general.kind, ChannelKind::Text);
    assert_eq!(
        general.category_id.map(|c| c.to_string()),
        Some(in_cat("Text Channels"))
    );
    assert_eq!(general.position, 0);
    let lounge = chans
        .iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    assert_eq!(
        lounge.category_id.map(|c| c.to_string()),
        Some(in_cat("Voice Channels"))
    );
    assert_eq!(lounge.position, 0);
    // The JSON carries it too.
    let raw: serde_json::Value =
        get_json(&app, &alex, &format!("/servers/{}/channels", s.id)).await;
    assert!(
        raw.as_array()
            .unwrap()
            .iter()
            .all(|c| c["category_id"].is_string())
    );
}

// ---------- category and layout routes (#54, #55) ----------

/// What a refused layout save says (the client shows it as the order snaps back).
const STALE: &str = "The order changed just now, so it's back to how it is for everyone.";

/// An authed request with a JSON body (or none) on `method`.
async fn send_json(
    app: &TestApp,
    method: reqwest::Method,
    token: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> reqwest::Response {
    let mut r = app.http.request(method, app.url(path)).bearer_auth(token);
    if let Some(b) = body {
        r = r.json(&b);
    }
    r.send().await.unwrap()
}

async fn patch(
    app: &TestApp,
    token: &str,
    path: &str,
    body: serde_json::Value,
) -> reqwest::Response {
    send_json(app, reqwest::Method::PATCH, token, path, Some(body)).await
}

async fn put(app: &TestApp, token: &str, path: &str, body: serde_json::Value) -> reqwest::Response {
    send_json(app, reqwest::Method::PUT, token, path, Some(body)).await
}

async fn delete(app: &TestApp, token: &str, path: &str) -> u16 {
    send_json(app, reqwest::Method::DELETE, token, path, None)
        .await
        .status()
        .as_u16()
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

/// The next event, which must be a `LayoutChanged`.
async fn next_layout(rx: &mut mpsc::Receiver<ServerFrame>) -> ServerLayout {
    match next_event(rx).await {
        Event::LayoutChanged { layout } => layout,
        other => panic!("expected LayoutChanged, got {other:?}"),
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

impl World {
    /// The server's layout as the database has it now.
    async fn layout(&self, server: ServerId) -> ServerLayout {
        let mut c = self.app.db.acquire().await.unwrap();
        layout_of(&mut c, server).await.unwrap()
    }

    async fn new_category(&self, token: &str, server: ServerId, name: &str) -> reqwest::Response {
        post_json(
            &self.app,
            token,
            &format!("/servers/{server}/categories"),
            json!({ "name": name }),
        )
        .await
    }

    async fn category(&self, name: &str) -> Category {
        let r = self.new_category(&self.alex.1, self.server, name).await;
        assert_eq!(r.status(), 200, "create category {name}");
        r.json().await.unwrap()
    }

    async fn channel_in(
        &self,
        server: ServerId,
        kind: &str,
        name: &str,
        category: Option<CategoryId>,
    ) -> reqwest::Response {
        post_json(
            &self.app,
            &self.alex.1,
            &format!("/servers/{server}/channels"),
            json!({ "kind": kind, "name": name, "category_id": category }),
        )
        .await
    }

    async fn channel(&self, name: &str, category: Option<CategoryId>) -> Channel {
        let r = self.channel_in(self.server, "text", name, category).await;
        assert_eq!(r.status(), 200, "create channel {name}");
        r.json().await.unwrap()
    }

    async fn ready_layouts(&self, user: UserId) -> Vec<ServerLayout> {
        pulse_server::gateway::socket::build_ready(
            &self.app.db,
            &self.app.voice,
            &self.app.streams,
            &self.app.hub,
            user,
        )
        .await
        .unwrap()
        .layouts
    }

    async fn revision(&self) -> i64 {
        self.ready_layouts(self.alex.0)
            .await
            .into_iter()
            .find(|l| l.server_id == self.server)
            .expect("the server's layout is in Ready")
            .revision
    }

    async fn save(&self, token: &str, body: serde_json::Value) -> reqwest::Response {
        put(
            &self.app,
            token,
            &format!("/servers/{}/layout", self.server),
            body,
        )
        .await
    }
}

/// The ids of `layout`'s channels in `category`, in order.
fn channels_in(layout: &ServerLayout, category: Option<CategoryId>) -> Vec<ChannelId> {
    let mut placed: Vec<&ChannelPlacement> = layout
        .channels
        .iter()
        .filter(|p| p.category_id == category)
        .collect();
    placed.sort_by_key(|p| p.position);
    placed.iter().map(|p| p.channel_id).collect()
}

/// `layout` as a save request: the same order, based on its revision.
fn request_for(layout: &ServerLayout) -> serde_json::Value {
    json!({
        "revision": layout.revision,
        "uncategorised": channels_in(layout, None),
        "categories": layout.categories.iter().map(|c| json!({
            "id": c.id,
            "channels": channels_in(layout, Some(c.id)),
        })).collect::<Vec<_>>(),
    })
}

fn names(layout: &ServerLayout) -> Vec<&str> {
    layout.categories.iter().map(|c| c.name.as_str()).collect()
}

#[tokio::test]
async fn members_create_rename_and_delete_categories_and_everyone_hears() {
    let w = world().await;
    let before = w.revision().await;
    let mut rx = w.app.hub.register(w.jo.0, String::new()).rx;
    let r = w.new_category(&w.sam.1, w.server, "  Gaming ").await;
    assert_eq!(r.status(), 200);
    let gaming: Category = r.json().await.unwrap();
    assert_eq!(gaming.name, "Gaming");
    assert_eq!(gaming.server_id, w.server);
    assert_eq!(gaming.position, 2, "after the two defaults");
    assert_eq!(gaming.default_kind, None);
    let heard = next_layout(&mut rx).await;
    assert_eq!(heard.server_id, w.server);
    assert_eq!(heard.categories.last(), Some(&gaming));
    assert!(heard.revision > before);

    let path = format!("/categories/{}", gaming.id);
    let r = patch(&w.app, &w.sam.1, &path, json!({ "name": "Games" })).await;
    assert_eq!(r.status(), 200);
    let renamed: Category = r.json().await.unwrap();
    assert_eq!((renamed.id, renamed.name.as_str()), (gaming.id, "Games"));
    let heard2 = next_layout(&mut rx).await;
    assert_eq!(heard2.categories.last().unwrap().name, "Games");
    assert!(heard2.revision > heard.revision);
    for bad in [json!(""), json!("   "), json!("x".repeat(65))] {
        assert_eq!(
            patch(&w.app, &w.sam.1, &path, json!({ "name": bad }))
                .await
                .status(),
            400
        );
    }

    // jo isn't in the server: no create, rename or delete.
    assert_eq!(
        w.new_category(&w.jo.1, w.server, "Mine").await.status(),
        403
    );
    assert_eq!(
        patch(&w.app, &w.jo.1, &path, json!({ "name": "Mine" }))
            .await
            .status(),
        403
    );
    assert_eq!(delete(&w.app, &w.jo.1, &path).await, 403);
    // No such server or category.
    let nowhere = format!("/categories/{}", CategoryId::new());
    assert_eq!(delete(&w.app, &w.sam.1, &nowhere).await, 404);
    assert_eq!(
        patch(&w.app, &w.sam.1, &nowhere, json!({ "name": "x" }))
            .await
            .status(),
        404
    );
    assert_eq!(
        w.new_category(&w.sam.1, ServerId::new(), "x")
            .await
            .status(),
        404
    );

    assert_eq!(delete(&w.app, &w.sam.1, &path).await, 204);
    let heard3 = next_layout(&mut rx).await;
    assert_eq!(names(&heard3), vec!["Text Channels", "Voice Channels"]);
    assert!(heard3.revision > heard2.revision);
    assert_eq!(heard3.revision, w.revision().await);
}

#[tokio::test]
async fn deleting_a_category_moves_its_channels_to_the_top_area_in_order() {
    let w = world().await;
    let gaming = w.category("Gaming").await;
    let x = w.channel("x", None).await;
    let a = w.channel("a", Some(gaming.id)).await;
    let b = w.channel("b", Some(gaming.id)).await;
    let before = w.layout(w.server).await;
    let mut rx = w.app.hub.register(w.jo.0, String::new()).rx;
    assert_eq!(
        delete(&w.app, &w.sam.1, &format!("/categories/{}", gaming.id)).await,
        204
    );
    // Only the layout changes: no channel goes.
    let heard = next_layout(&mut rx).await;
    let after = w.layout(w.server).await;
    assert_eq!(heard, after);
    assert_eq!(channels_in(&after, None), vec![x.id, a.id, b.id]);
    assert_eq!(after.channels.len(), before.channels.len());
    let listed: Vec<Channel> = get_json(
        &w.app,
        &w.alex.1,
        &format!("/servers/{}/channels", w.server),
    )
    .await;
    let a_now = listed.iter().find(|c| c.id == a.id).unwrap();
    assert_eq!((a_now.category_id, a_now.position), (None, 1));
}

#[tokio::test]
async fn a_layout_saves_and_reaches_everyone() {
    let w = world().await;
    let gaming = w.category("Gaming").await;
    let base = w.layout(w.server).await;
    let text = base.categories[0].id;
    let voice = base.categories[1].id;
    let general = channels_in(&base, Some(text))[0];
    let lounge = channels_in(&base, Some(voice))[0];
    let mut rx = w.app.hub.register(w.jo.0, String::new()).rx;
    let r = w
        .save(
            &w.sam.1,
            json!({
                "revision": base.revision,
                "uncategorised": [],
                "categories": [
                    { "id": gaming.id, "channels": [general] },
                    { "id": text, "channels": [] },
                    { "id": voice, "channels": [lounge] },
                ],
            }),
        )
        .await;
    assert_eq!(r.status(), 200);
    let saved: ServerLayout = r.json().await.unwrap();
    assert_eq!(
        names(&saved),
        vec!["Gaming", "Text Channels", "Voice Channels"]
    );
    assert_eq!(
        saved
            .categories
            .iter()
            .map(|c| c.position)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(saved.revision, base.revision + 1);
    assert_eq!(channels_in(&saved, Some(gaming.id)), vec![general]);
    assert!(channels_in(&saved, Some(text)).is_empty());
    assert_eq!(saved, w.layout(w.server).await);
    let listed: Vec<Channel> =
        get_json(&w.app, &w.jo.1, &format!("/servers/{}/channels", w.server)).await;
    let g = listed.iter().find(|c| c.id == general).unwrap();
    assert_eq!((g.category_id, g.position), (Some(gaming.id), 0));
    assert_eq!(next_layout(&mut rx).await, saved);
}

#[tokio::test]
async fn a_stale_layout_is_refused() {
    let w = world().await;
    let gaming = w.category("Gaming").await;
    let a = w.channel("a", None).await;
    let b = w.channel("b", None).await;
    let base = w.layout(w.server).await;
    let text = base.categories[0].id;
    let voice = base.categories[1].id;
    let general = channels_in(&base, Some(text))[0];
    let lounge = channels_in(&base, Some(voice))[0];
    let order = |up: Vec<ChannelId>| {
        json!({
            "revision": base.revision,
            "uncategorised": up,
            "categories": [
                { "id": text, "channels": [general] },
                { "id": voice, "channels": [lounge] },
                { "id": gaming.id, "channels": [] },
            ],
        })
    };
    // sam and alex both reorder from the same layout: sam's lands first.
    assert_eq!(
        w.save(&w.sam.1, order(vec![b.id, a.id])).await.status(),
        200
    );
    let r = w.save(&w.alex.1, order(vec![a.id, b.id])).await;
    assert_eq!(r.status(), 409);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["message"], STALE);
    let now = w.layout(w.server).await;
    assert_eq!(channels_in(&now, None), vec![b.id, a.id]);
    assert_eq!(now.revision, base.revision + 1);
}

#[tokio::test]
async fn an_incomplete_or_foreign_layout_is_refused() {
    let w = world().await;
    let gaming = w.category("Gaming").await;
    let old = w.channel("old", Some(gaming.id)).await;
    assert_eq!(
        delete(&w.app, &w.alex.1, &format!("/channels/{}", old.id)).await,
        204
    );
    let side = create_server(&w.app, &w.alex.1, "Side").await;
    let foreign = w.layout(side.id).await.categories[0].id;
    let base = w.layout(w.server).await;
    let text = base.categories[0].id;
    let voice = base.categories[1].id;
    let general = channels_in(&base, Some(text))[0];
    let lounge = channels_in(&base, Some(voice))[0];
    let rev = base.revision;
    let cases = [
        (
            "missing a channel",
            json!({ "revision": rev, "uncategorised": [], "categories": [
                { "id": text, "channels": [general] },
                { "id": voice, "channels": [] },
                { "id": gaming.id, "channels": [] },
            ]}),
        ),
        (
            "a channel twice",
            json!({ "revision": rev, "uncategorised": [general], "categories": [
                { "id": text, "channels": [general] },
                { "id": voice, "channels": [lounge] },
                { "id": gaming.id, "channels": [] },
            ]}),
        ),
        (
            "a soft-deleted channel",
            json!({ "revision": rev, "uncategorised": [], "categories": [
                { "id": text, "channels": [general] },
                { "id": voice, "channels": [lounge] },
                { "id": gaming.id, "channels": [old.id] },
            ]}),
        ),
        (
            "another server's category",
            json!({ "revision": rev, "uncategorised": [], "categories": [
                { "id": text, "channels": [general] },
                { "id": voice, "channels": [lounge] },
                { "id": gaming.id, "channels": [] },
                { "id": foreign, "channels": [] },
            ]}),
        ),
        (
            "a category twice",
            json!({ "revision": rev, "uncategorised": [], "categories": [
                { "id": text, "channels": [general] },
                { "id": voice, "channels": [lounge] },
                { "id": gaming.id, "channels": [] },
                { "id": gaming.id, "channels": [] },
            ]}),
        ),
        (
            "a missing category",
            json!({ "revision": rev, "uncategorised": [lounge], "categories": [
                { "id": text, "channels": [general] },
                { "id": gaming.id, "channels": [] },
            ]}),
        ),
    ];
    for (why, body) in cases {
        let r = w.save(&w.sam.1, body).await;
        assert_eq!(r.status(), 409, "{why}");
        let msg: serde_json::Value = r.json().await.unwrap();
        assert_eq!(msg["message"], STALE, "{why}");
        assert_eq!(w.layout(w.server).await, base, "{why}: nothing changed");
    }
    // The same order, complete, is fine (so the refusals above were about what they named).
    assert_eq!(w.save(&w.sam.1, request_for(&base)).await.status(), 200);
}

#[tokio::test]
async fn non_members_cant_save_a_layout() {
    let w = world().await;
    let base = w.layout(w.server).await;
    assert_eq!(w.save(&w.jo.1, request_for(&base)).await.status(), 403);
    assert_eq!(w.layout(w.server).await, base);
    let r = put(
        &w.app,
        &w.jo.1,
        &format!("/servers/{}/layout", ServerId::new()),
        request_for(&base),
    )
    .await;
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn channels_are_created_inside_a_category() {
    let w = world().await;
    let gaming = w.category("Gaming").await;
    let mut rx = w.app.hub.register(w.jo.0, String::new()).rx;
    let a = w.channel("a", Some(gaming.id)).await;
    let b = w.channel("b", Some(gaming.id)).await;
    assert_eq!((a.category_id, a.position), (Some(gaming.id), 0));
    assert_eq!((b.category_id, b.position), (Some(gaming.id), 1));
    for ch in [&a, &b] {
        match next_event(&mut rx).await {
            Event::ChannelCreated { channel } => assert_eq!(&channel, ch),
            other => panic!("expected ChannelCreated, got {other:?}"),
        }
        let layout = next_layout(&mut rx).await;
        assert_eq!(channels_in(&layout, Some(gaming.id)).last(), Some(&ch.id));
    }
    // A voice channel goes in the same category: categories hold every kind.
    let r = w
        .channel_in(w.server, "voice", "Games", Some(gaming.id))
        .await;
    assert_eq!(r.status(), 200);
    let v: Channel = r.json().await.unwrap();
    assert_eq!((v.category_id, v.position), (Some(gaming.id), 2));

    let side = create_server(&w.app, &w.alex.1, "Side").await;
    let foreign = w.layout(side.id).await.categories[0].id;
    for wrong in [foreign, CategoryId::new()] {
        let r = w.channel_in(w.server, "text", "lost", Some(wrong)).await;
        assert_eq!(r.status(), 400);
    }

    let x = w.channel("x", None).await;
    let y = w.channel("y", None).await;
    assert_eq!((x.category_id, x.position), (None, 0));
    assert_eq!((y.category_id, y.position), (None, 1));
    assert_eq!(
        channels_in(&w.layout(w.server).await, None),
        vec![x.id, y.id]
    );
}

#[tokio::test]
async fn create_delete_and_restore_bump_the_revision() {
    let w = world().await;
    let r0 = w.revision().await;
    let ch = w.channel("a", None).await;
    assert_eq!(w.revision().await, r0 + 1);
    let mut rx = w.app.hub.register(w.jo.0, String::new()).rx;
    assert_eq!(
        delete(&w.app, &w.alex.1, &format!("/channels/{}", ch.id)).await,
        204
    );
    assert_eq!(w.revision().await, r0 + 2);
    assert!(matches!(
        next_event(&mut rx).await,
        Event::ChannelDeleted { .. }
    ));
    let heard = next_layout(&mut rx).await;
    assert_eq!(heard.revision, r0 + 2);
    assert!(!heard.channels.iter().any(|p| p.channel_id == ch.id));
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/channels/{}/restore", ch.id),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 200);
    assert_eq!(w.revision().await, r0 + 3);
    assert!(matches!(
        next_event(&mut rx).await,
        Event::ChannelRestored { .. }
    ));
    let heard = next_layout(&mut rx).await;
    assert_eq!(heard.revision, r0 + 3);
    assert!(heard.channels.iter().any(|p| p.channel_id == ch.id));
}

async fn restore(w: &World, id: ChannelId) -> Channel {
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/channels/{id}/restore"),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

#[tokio::test]
async fn a_channel_restored_into_a_deleted_category_lands_at_the_bottom_of_the_top_area() {
    let w = world().await;
    let gaming = w.category("Gaming").await;
    let a = w.channel("a", Some(gaming.id)).await;
    let b = w.channel("b", Some(gaming.id)).await;
    let x = w.channel("x", None).await;
    assert_eq!(
        delete(&w.app, &w.alex.1, &format!("/channels/{}", a.id)).await,
        204
    );
    assert_eq!(
        delete(&w.app, &w.alex.1, &format!("/categories/{}", gaming.id)).await,
        204
    );
    let back = restore(&w, a.id).await;
    assert_eq!((back.category_id, back.position), (None, 2));
    assert_eq!(
        channels_in(&w.layout(w.server).await, None),
        vec![x.id, b.id, a.id]
    );

    // One restored into a category that is still there goes to its bottom.
    let text = w.layout(w.server).await.categories[0].id;
    let t1 = w.channel("t1", Some(text)).await;
    assert_eq!(
        delete(&w.app, &w.alex.1, &format!("/channels/{}", t1.id)).await,
        204
    );
    let t2 = w.channel("t2", Some(text)).await;
    let back = restore(&w, t1.id).await;
    assert_eq!((back.category_id, back.position), (Some(text), 2));
    let in_text = channels_in(&w.layout(w.server).await, Some(text));
    assert_eq!(in_text[1..], [t2.id, t1.id]);
}

#[tokio::test]
async fn the_category_cap_holds() {
    let w = world().await;
    // The two defaults count.
    for i in 2..50 {
        w.category(&format!("c{i}")).await;
    }
    let r = w.new_category(&w.sam.1, w.server, "one too many").await;
    assert_eq!(r.status(), 400);
    assert_eq!(w.layout(w.server).await.categories.len(), 50);
}

#[tokio::test]
async fn layout_changes_are_rate_limited() {
    let mut cfg = test_config();
    cfg.limits.layout_user = pulse_server::limits::Rate::new(2, 0.0001);
    let w = world_with(spawn_with(cfg).await).await;
    let base = w.layout(w.server).await;
    assert_eq!(w.new_category(&w.sam.1, w.server, "a").await.status(), 200);
    assert_eq!(w.new_category(&w.sam.1, w.server, "b").await.status(), 200);
    assert_eq!(w.new_category(&w.sam.1, w.server, "c").await.status(), 429);
    assert_eq!(w.save(&w.sam.1, request_for(&base)).await.status(), 429);
    // Per person: alex still has his.
    assert_eq!(w.new_category(&w.alex.1, w.server, "c").await.status(), 200);
}

#[tokio::test]
async fn ready_carries_every_servers_layout() {
    let w = world().await;
    let gaming = w.category("Gaming").await;
    let side = create_server(&w.app, &w.alex.1, "Side").await;
    // jo is in neither and still gets both: like the channels, layouts are public.
    let layouts = w.ready_layouts(w.jo.0).await;
    assert_eq!(layouts.len(), 2);
    let main = layouts.iter().find(|l| l.server_id == w.server).unwrap();
    assert_eq!(
        names(main),
        vec!["Text Channels", "Voice Channels", "Gaming"]
    );
    assert_eq!(main.categories[2], gaming);
    assert_eq!(main.revision, w.layout(w.server).await.revision);
    assert!(
        main.channels.is_empty(),
        "the channels carry their own placement"
    );
    let side_layout = layouts.iter().find(|l| l.server_id == side.id).unwrap();
    assert_eq!(names(side_layout), vec!["Text Channels", "Voice Channels"]);
    assert_eq!(side_layout.revision, 0);
    assert_eq!(
        side_layout.categories[0].default_kind,
        Some(ChannelKind::Text)
    );
}
