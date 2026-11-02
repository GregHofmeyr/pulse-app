//! The message search index (migration 0011): what is in it, and that triggers keep it so.
use std::borrow::Cow;

use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, SqlitePool};

const TS: &str = "2026-10-01T00:00:00.000Z";

/// Ids of the messages the index finds for an FTS5 `expr`, sorted.
async fn hits(db: &SqlitePool, expr: &str) -> Vec<String> {
    let mut ids: Vec<String> = sqlx::query_scalar(
        "SELECT d.message_id FROM message_search s JOIN search_docs d ON d.doc = s.rowid
         WHERE message_search MATCH ?",
    )
    .bind(expr)
    .fetch_all(db)
    .await
    .unwrap();
    ids.sort();
    ids
}

async fn exec(db: &SqlitePool, q: &str) {
    sqlx::query(q).execute(db).await.unwrap();
}

/// A migrated database with alex, server Main, #general (text), #quotes and a DM.
async fn fresh() -> (tempfile::TempDir, SqlitePool) {
    let dir = tempfile::tempdir().unwrap();
    let db = pulse_server::db::connect(&format!("sqlite://{}", dir.path().join("s.db").display()))
        .await
        .unwrap();
    for q in base_rows() {
        exec(&db, &q).await;
    }
    (dir, db)
}

fn base_rows() -> Vec<String> {
    vec![
        format!("INSERT INTO users (id, username, password_hash, created_at) VALUES ('U', 'alex', 'x', '{TS}')"),
        format!("INSERT INTO servers (id, name, created_by, created_at) VALUES ('S', 'Main', 'U', '{TS}')"),
        format!("INSERT INTO channels (id, server_id, kind, name, position, created_at) VALUES ('C', 'S', 'text', 'general', 0, '{TS}')"),
        format!("INSERT INTO channels (id, server_id, kind, name, position, created_at) VALUES ('Q', 'S', 'quotes', 'quotes', 1, '{TS}')"),
    ]
}

fn msg(id: &str, channel: &str, content: &str) -> String {
    format!(
        "INSERT INTO messages (id, channel_id, author_id, content, created_at) VALUES ('{id}', '{channel}', 'U', '{content}', '{TS}')"
    )
}

fn caption_line(id: &str, message: &str, caption: &str) -> String {
    format!(
        "INSERT INTO media (hash, kind, content_type, size, width, height, frames, created_at) VALUES ('H{id}', 'quote', 'image/webp', 1, 1, 1, 1, '{TS}');
         INSERT INTO quote_lines (id, message_id, position, speaker_id, media_hash, caption) VALUES ('{id}', '{message}', 1, 'U', 'H{id}', '{caption}')"
    )
}

#[tokio::test]
async fn insert_is_indexed() {
    let (_d, db) = fresh().await;
    exec(&db, &msg("M1", "C", "pizza tonight")).await;
    assert_eq!(hits(&db, "pizza").await, vec!["M1"]);
}

#[tokio::test]
async fn edit_reindexes() {
    let (_d, db) = fresh().await;
    exec(&db, &msg("M1", "C", "pizza tonight")).await;
    exec(&db, "UPDATE messages SET content = 'tacos tonight', edited_at = '2026-10-02T00:00:00.000Z' WHERE id = 'M1'").await;
    assert!(hits(&db, "pizza").await.is_empty(), "the old words are gone");
    assert_eq!(hits(&db, "tacos").await, vec!["M1"]);
}

#[tokio::test]
async fn soft_delete_unindexes() {
    let (_d, db) = fresh().await;
    exec(&db, &msg("M1", "C", "pizza tonight")).await;
    exec(&db, &format!("UPDATE messages SET content = '', deleted_at = '{TS}' WHERE id = 'M1'")).await;
    assert!(hits(&db, "pizza").await.is_empty());
    // Even with words left behind, a deleted message stays out.
    exec(&db, &msg("M2", "C", "pizza again")).await;
    exec(&db, &format!("UPDATE messages SET deleted_at = '{TS}' WHERE id = 'M2'")).await;
    assert!(hits(&db, "pizza").await.is_empty());
    let docs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM search_docs").fetch_one(&db).await.unwrap();
    assert_eq!(docs, 0);
}

#[tokio::test]
async fn hard_delete_unindexes() {
    let (_d, db) = fresh().await;
    exec(&db, &msg("M1", "C", "pizza tonight")).await;
    exec(&db, "DELETE FROM messages WHERE id = 'M1'").await;
    assert!(hits(&db, "pizza").await.is_empty());
    let docs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM search_docs").fetch_one(&db).await.unwrap();
    assert_eq!(docs, 0);
}

#[tokio::test]
async fn system_lines_excluded() {
    let (_d, db) = fresh().await;
    exec(&db, &format!("INSERT INTO messages (id, channel_id, kind, content, created_at) VALUES ('M1', 'C', 'system', 'Quote of the Year pizza', '{TS}')")).await;
    assert!(hits(&db, "pizza").await.is_empty());
}

#[tokio::test]
async fn quote_captions_indexed_and_follow_edits() {
    let (_d, db) = fresh().await;
    exec(&db, &msg("M1", "Q", "[image]")).await;
    for q in caption_line("L1", "M1", "the famous goat").split(';') {
        exec(&db, q).await;
    }
    assert_eq!(hits(&db, "goat").await, vec!["M1"]);
    exec(&db, "UPDATE quote_lines SET caption = 'a sheep' WHERE id = 'L1'").await;
    assert!(hits(&db, "goat").await.is_empty());
    assert_eq!(hits(&db, "sheep").await, vec!["M1"]);
    // The content is still there alongside the caption.
    assert_eq!(hits(&db, "image").await, vec!["M1"]);
    exec(&db, "DELETE FROM quote_lines WHERE id = 'L1'").await;
    assert!(hits(&db, "sheep").await.is_empty());
}

#[tokio::test]
async fn deleted_quote_unindexed() {
    let (_d, db) = fresh().await;
    exec(&db, &msg("M1", "Q", "“honest” — alex")).await;
    for q in caption_line("L1", "M1", "goat").split(';') {
        exec(&db, q).await;
    }
    // A quote's delete: the soft delete, then its lines go (quotes::on_delete).
    exec(&db, &format!("UPDATE messages SET content = '', deleted_at = '{TS}' WHERE id = 'M1'")).await;
    exec(&db, "DELETE FROM quote_lines WHERE message_id = 'M1'").await;
    assert!(hits(&db, "honest").await.is_empty());
    assert!(hits(&db, "goat").await.is_empty());
}

#[tokio::test]
async fn thread_replies_indexed() {
    let (_d, db) = fresh().await;
    exec(&db, &msg("M1", "Q", "“honest” — alex")).await;
    exec(&db, &format!("INSERT INTO messages (id, channel_id, author_id, content, created_at, thread_root_id) VALUES ('R1', 'Q', 'U', 'so true', '{TS}', 'M1')")).await;
    assert_eq!(hits(&db, "true").await, vec!["R1"]);
}

#[tokio::test]
async fn accent_and_prefix_match() {
    let (_d, db) = fresh().await;
    exec(&db, &msg("M1", "C", "Café clips from Zoë")).await;
    assert_eq!(hits(&db, "cafe").await, vec!["M1"]);
    assert_eq!(hits(&db, "zoe").await, vec!["M1"]);
    assert_eq!(hits(&db, "\"clip\"*").await, vec!["M1"]);
    assert_eq!(hits(&db, "\"cl\"*").await, vec!["M1"], "two-letter prefix");
    assert_eq!(hits(&db, "\"caf\"*").await, vec!["M1"], "three-letter prefix");
}

/// The live database is migrated once: 0011 builds the index from what is already there.
#[tokio::test]
async fn migration_backfills_existing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let mut conn = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(false)
        .connect()
        .await
        .unwrap();
    let mut migrator = sqlx::migrate::Migrator::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"),
    )
    .await
    .unwrap();
    let all = migrator.migrations.clone();
    migrator.migrations = Cow::Owned(all.iter().filter(|m| m.version <= 10).cloned().collect());
    migrator.run(&mut conn).await.unwrap();
    let mut rows = base_rows();
    rows.push(msg("M1", "C", "pizza tonight"));
    // A delete from before search: its words were cleared, but don't trust that.
    rows.push(format!("INSERT INTO messages (id, channel_id, author_id, content, created_at, deleted_at) VALUES ('M2', 'C', 'U', 'pizza leftovers', '{TS}', '{TS}')"));
    rows.push(format!("INSERT INTO messages (id, channel_id, kind, content, created_at) VALUES ('M3', 'C', 'system', 'pizza card', '{TS}')"));
    rows.push(msg("M4", "Q", "[image]"));
    rows.extend(caption_line("L1", "M4", "pizza goat").split(';').map(String::from));
    for q in rows {
        sqlx::query(&q).execute(&mut conn).await.unwrap();
    }
    conn.close().await.unwrap();

    let db = pulse_server::db::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    let v: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(v, 11);
    assert_eq!(hits(&db, "pizza").await, vec!["M1", "M4"]);
    assert_eq!(hits(&db, "goat").await, vec!["M4"]);
    // And the triggers run from here on.
    exec(&db, &msg("M5", "C", "more pizza")).await;
    assert_eq!(hits(&db, "pizza").await, vec!["M1", "M4", "M5"]);
}
