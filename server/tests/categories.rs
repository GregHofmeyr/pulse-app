//! Channel categories (#54): the update that gives existing servers their default categories,
//! and the defaults a new server starts with.
mod common;

use std::borrow::Cow;

use common::*;
use pulse_protocol::rest::{Channel, ChannelKind};
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;

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
