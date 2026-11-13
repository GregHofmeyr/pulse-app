//! How long purging a big deleted channel (#108) keeps other writers waiting. Ignored by default
//! (it takes a while to fill); run with
//! `cargo test -p pulse-server --test purge_perf -- --ignored --nocapture`.
mod common;

use std::time::{Duration, Instant};

use common::*;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::Channel;
use serde_json::json;

async fn count(app: &TestApp, q: &str, id: &str) -> i64 {
    sqlx::query_scalar(q)
        .bind(id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn scalar(app: &TestApp, q: &str) -> i64 {
    sqlx::query_scalar(q).fetch_one(&app.db).await.unwrap()
}

async fn status(r: reqwest::RequestBuilder, token: &str) -> u16 {
    r.bearer_auth(token).send().await.unwrap().status().as_u16()
}

/// alex's server "Main" (sam has joined), with a text channel "old" beside its general.
struct World {
    app: TestApp,
    alex: (UserId, String),
    sam: (UserId, String),
    general: Channel,
    old: Channel,
}

async fn world() -> World {
    let app = spawn().await;
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let s = create_server(&app, &alex.1, "Main").await;
    let r = post_json(&app, &sam.1, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);
    let general = general(&app, &alex.1, s.id).await;
    let r = post_json(
        &app,
        &alex.1,
        &format!("/servers/{}/channels", s.id),
        json!({ "kind": "text", "name": "old" }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let old = r.json().await.unwrap();
    World {
        app,
        alex,
        sam,
        general,
        old,
    }
}

/// `n` messages straight into `channel` (the search triggers index them), in one transaction.
async fn fill(app: &TestApp, channel: ChannelId, author: UserId, n: usize) {
    let mut tx = app.db.begin().await.unwrap();
    for i in 0..n {
        sqlx::query(
            "INSERT INTO messages (id, channel_id, author_id, content, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(MessageId::new().to_string())
        .bind(channel.to_string())
        .bind(author.to_string())
        .bind(format!("filler number {i} with some searchable words"))
        .bind(pulse_server::db::now())
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
}

async fn delete_channel(w: &World, id: ChannelId) {
    let r = w.app.http.delete(w.app.url(&format!("/channels/{id}")));
    assert_eq!(status(r, &w.sam.1).await, 204);
}

/// Nothing anywhere points at a message, a search entry or a channel that is gone.
async fn assert_no_orphans(app: &TestApp) {
    for (table, col) in [
        ("search_docs", "message_id"),
        ("reactions", "message_id"),
        ("mentions", "message_id"),
        ("pins", "message_id"),
        ("polls", "message_id"),
        ("inbox_reads", "message_id"),
        ("thread_reads", "root_id"),
        ("quote_lines", "message_id"),
        ("quotes", "message_id"),
        ("link_embeds", "message_id"),
        ("messages", "reply_to_id"),
        ("messages", "thread_root_id"),
    ] {
        let q = format!(
            "SELECT COUNT(*) FROM {table} WHERE {col} IS NOT NULL
             AND {col} NOT IN (SELECT id FROM messages)"
        );
        assert_eq!(scalar(app, &q).await, 0, "{table}.{col}");
    }
    for table in ["pins", "polls", "read_states", "search_docs", "messages"] {
        let q = format!(
            "SELECT COUNT(*) FROM {table} WHERE channel_id NOT IN (SELECT id FROM channels)"
        );
        assert_eq!(scalar(app, &q).await, 0, "{table}.channel_id");
    }
    let stray_fts = scalar(
        app,
        "SELECT COUNT(*) FROM message_search WHERE rowid NOT IN (SELECT doc FROM search_docs)",
    )
    .await;
    assert_eq!(stray_fts, 0, "message_search rows without a doc");
    let broken: Vec<(String, Option<i64>, String, i64)> =
        sqlx::query_as("PRAGMA foreign_key_check")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert!(broken.is_empty(), "{broken:?}");
}

/// Seeds ~50k messages in one channel and purges it with "Delete now" while another writer keeps
/// sending elsewhere and a probe keeps taking the write lock: prints the longest wait of each.
#[tokio::test]
#[ignore = "fills 50k messages; run by hand (see the doc)"]
async fn purge_perf_50k() {
    const N: usize = 50_000;
    let _ = tracing_subscriber::fmt()
        .with_env_filter("pulse_server::servers::purge=info")
        .with_test_writer()
        .try_init();
    let w = world().await;
    // Some history elsewhere, as a real server has.
    fill(&w.app, w.general.id, w.alex.0, 10_000).await;
    let started = Instant::now();
    fill(&w.app, w.old.id, w.alex.0, N).await;
    println!("filled {N} messages in {:?}", started.elapsed());
    delete_channel(&w, w.old.id).await;

    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // A probe: the longest a tiny write transaction waits for the lock.
    let probe = {
        let db = w.app.db.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            let (mut worst, mut n, mut timed_out) = (Duration::ZERO, 0, 0);
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let t = Instant::now();
                // Past the 5 s busy timeout the lock attempt fails ("database is locked").
                match pulse_server::db::begin_write(&db).await {
                    Ok(mut tx) => {
                        sqlx::query("SELECT 1").execute(&mut *tx).await.unwrap();
                        tx.commit().await.unwrap();
                    }
                    Err(_) => timed_out += 1,
                }
                worst = worst.max(t.elapsed());
                n += 1;
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            (worst, n, timed_out)
        })
    };
    // A sender: the longest a send in another channel takes.
    let sender = {
        let app_url = w.app.url(&format!("/channels/{}/messages", w.general.id));
        let http = w.app.http.clone();
        let token = w.alex.1.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            let (mut worst, mut n, mut failed) = (Duration::ZERO, 0, 0);
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let t = Instant::now();
                let r = http
                    .post(&app_url)
                    .bearer_auth(&token)
                    .json(&json!({ "content": format!("during the purge {n}") }))
                    .send()
                    .await
                    .unwrap();
                if r.status() != 200 {
                    failed += 1;
                }
                worst = worst.max(t.elapsed());
                n += 1;
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            (worst, n, failed)
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let t = Instant::now();
    let r = w
        .app
        .http
        .delete(w.app.url(&format!("/channels/{}/permanent", w.old.id)))
        .bearer_auth(&w.alex.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let answered = t.elapsed();
    // A big channel goes on purging after the answer: wait for it to be gone.
    let old = w.old.id.to_string();
    while count(&w.app, "SELECT COUNT(*) FROM channels WHERE id = ?", &old).await > 0 {
        assert!(t.elapsed() < Duration::from_secs(600), "purge stuck");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let purge_took = t.elapsed();
    println!("\"Delete now\" answered after {answered:?}");
    tokio::time::sleep(Duration::from_millis(100)).await;
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let (probe_worst, probes, timed_out) = probe.await.unwrap();
    let (send_worst, sends, failed) = sender.await.unwrap();
    println!("purge of {N} messages took {purge_took:?}");
    println!(
        "longest write-lock wait (probe, {probes} tries, {timed_out} past the busy timeout): {probe_worst:?}"
    );
    println!("slowest send elsewhere ({sends} sends, {failed} failed): {send_worst:?}");
    assert_eq!(
        (timed_out, failed),
        (0, 0),
        "writers gave up waiting for the purge"
    );
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM messages WHERE channel_id = ?",
            &w.old.id.to_string()
        )
        .await,
        0
    );
    assert_no_orphans(&w.app).await;
}
