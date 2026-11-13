//! Purging a deleted channel for good in short batches (#108): a big channel never holds the write
//! lock for the whole purge, and a purge cut off part-way (a restart) finishes on the next run with
//! nothing left behind.
mod common;

use std::time::{Duration, Instant};

use common::*;
use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{Channel, DeletedChannel, Message};
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
    server: ServerId,
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
        server: s.id,
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

async fn deleted_list(w: &World) -> Vec<DeletedChannel> {
    get_json(
        &w.app,
        &w.alex.1,
        &format!("/servers/{}/deleted-channels", w.server),
    )
    .await
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

/// While a big deleted channel is purged, a send in another channel goes through between two of
/// its batches instead of waiting for the whole purge.
#[tokio::test]
async fn a_send_elsewhere_isnt_held_up_by_the_whole_purge() {
    const N: i64 = 6_000;
    let w = world().await;
    fill(&w.app, w.old.id, w.alex.0, N as usize).await;
    delete_channel(&w, w.old.id).await;
    let s = w.app.state();
    let purge = tokio::spawn(async move {
        pulse_server::servers::channels::purge_expired_batched(
            &s,
            chrono::Utc::now() + chrono::Duration::days(8),
            20,
        )
        .await
        .unwrap()
    });
    let old = w.old.id.to_string();
    let q = "SELECT COUNT(*) FROM messages WHERE channel_id = ?";
    // Wait for the purge to commit its first batch: one transaction for everything never shows a
    // part-purged channel.
    let started = Instant::now();
    let left = loop {
        let left = count(&w.app, q, &old).await;
        if left < N {
            break left;
        }
        assert!(started.elapsed() < Duration::from_secs(30), "purge stuck");
        tokio::time::sleep(Duration::from_millis(1)).await;
    };
    assert!(left > 0, "the purge committed in one go");
    let sent_at = Instant::now();
    send(&w.app, &w.alex.1, w.general.id, "still here").await;
    let took = sent_at.elapsed();
    let after = count(&w.app, q, &old).await;
    assert!(after > 0, "the send waited for the whole purge ({took:?})");
    assert_eq!(purge.await.unwrap(), 1);
    assert_eq!(count(&w.app, q, &old).await, 0);
    assert_eq!(
        count(&w.app, "SELECT COUNT(*) FROM channels WHERE id = ?", &old).await,
        0
    );
    assert_no_orphans(&w.app).await;
}

/// A reply in `channel` by `token`'s user to `to` (mentioning alex).
async fn reply(w: &World, token: &str, channel: ChannelId, to: MessageId, text: &str) -> Message {
    let r = post_json(
        &w.app,
        token,
        &format!("/channels/{channel}/messages"),
        json!({ "content": format!("{text} @alex"), "reply_to_id": to }),
    )
    .await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

/// "Delete now" cut off after two batches (the server stopped): the channel is out of Recently
/// deleted and can't be restored or written to, and the next sweep finishes it with nothing of it
/// left anywhere.
#[tokio::test]
async fn a_purge_cut_off_part_way_finishes_on_the_next_run() {
    let w = world().await;
    let old = w.old.id;
    // Replies across batch boundaries (batches of 3 below), mentions, reactions, a pin, a poll,
    // an inbox read, a thread read and a file.
    let first = send(&w.app, &w.alex.1, old, "findable first").await;
    let mut last = first.clone();
    let mut ids = vec![first.id];
    for i in 0..10 {
        let token = if i % 2 == 0 { &w.sam.1 } else { &w.alex.1 };
        last = reply(&w, token, old, last.id, &format!("findable reply {i}")).await;
        ids.push(last.id);
    }
    let kept = send(&w.app, &w.alex.1, w.general.id, "findable elsewhere").await;
    let r = w
        .app
        .http
        .put(
            w.app
                .url(&format!("/messages/{}/reactions/%F0%9F%91%8D", first.id)),
        )
        .bearer_auth(&w.sam.1)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "react");
    let now = pulse_server::db::now();
    let (alex, sam) = (w.alex.0.to_string(), w.sam.0.to_string());
    for (q, binds) in [
        (
            "INSERT INTO pins (message_id, channel_id, pinned_by, pinned_at) VALUES (?, ?, ?, ?)",
            vec![first.id.to_string(), old.to_string(), alex.clone(), now.clone()],
        ),
        (
            "INSERT INTO polls (message_id, channel_id, question, options, multi, anonymous, created_at)
             VALUES (?, ?, 'which?', '[\"a\",\"b\"]', 0, 0, ?)",
            vec![ids[1].to_string(), old.to_string(), now.clone()],
        ),
        (
            "INSERT INTO inbox_reads (user_id, message_id) VALUES (?, ?)",
            vec![alex.clone(), ids[4].to_string()],
        ),
        (
            "INSERT INTO thread_reads (user_id, root_id, last_seen_id) VALUES (?, ?, ?)",
            vec![sam.clone(), first.id.to_string(), ids[2].to_string()],
        ),
        (
            "UPDATE messages SET thread_root_id = ? WHERE id = ?",
            vec![first.id.to_string(), ids[9].to_string()],
        ),
        (
            "INSERT INTO attachments (id, key, user_id, channel_id, message_id, name, content_type,
                                      kind, size, created_at)
             VALUES ('att1', 'key1', ?, ?, ?, 'a.txt', 'text/plain', 'file', 1, ?)",
            vec![alex.clone(), old.to_string(), ids[3].to_string(), now.clone()],
        ),
    ] {
        let mut query = sqlx::query(q);
        for b in binds {
            query = query.bind(b);
        }
        query.execute(&w.app.db).await.unwrap();
    }
    delete_channel(&w, old).await;
    assert_eq!(deleted_list(&w).await.len(), 1);

    let s = w.app.state();
    pulse_server::servers::channels::purge_partly(&s, old, 3, 2)
        .await
        .unwrap();
    let id = old.to_string();
    let left = count(
        &w.app,
        "SELECT COUNT(*) FROM messages WHERE channel_id = ?",
        &id,
    )
    .await;
    assert_eq!(left, 11 - 6, "two batches of three went");
    assert_eq!(
        count(&w.app, "SELECT COUNT(*) FROM channels WHERE id = ?", &id).await,
        1
    );
    // Half-purged: out of Recently deleted, never coming back, and still refusing writes.
    assert!(deleted_list(&w).await.is_empty(), "it's being purged");
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/channels/{old}/restore"),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 404);
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/channels/{old}/messages"),
        json!({ "content": "sneaking in" }),
    )
    .await;
    assert_eq!(r.status(), 404);
    assert_no_orphans(&w.app).await;

    // The next sweep (no need to wait the 7 days: "Delete now" made it due) finishes it.
    let swept = pulse_server::servers::channels::purge_expired(&s, chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(swept, 1);
    assert_eq!(
        count(&w.app, "SELECT COUNT(*) FROM channels WHERE id = ?", &id).await,
        0
    );
    for table in ["messages", "search_docs", "pins", "polls", "read_states"] {
        let q = format!("SELECT COUNT(*) FROM {table} WHERE channel_id = ?");
        assert_eq!(count(&w.app, &q, &id).await, 0, "{table}");
    }
    assert_no_orphans(&w.app).await;
    // Its file goes too (object and row, after the commit).
    let started = Instant::now();
    while scalar(&w.app, "SELECT COUNT(*) FROM attachments WHERE id = 'att1'").await > 0 {
        assert!(started.elapsed() < Duration::from_secs(5), "file row left");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // The search index lost the channel's messages and nothing else.
    let hits = scalar(
        &w.app,
        "SELECT COUNT(*) FROM message_search WHERE message_search MATCH 'findable'",
    )
    .await;
    assert_eq!(hits, 1);
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM search_docs WHERE message_id = ?",
            &kept.id.to_string()
        )
        .await,
        1
    );
}

/// A sweep that runs while "Delete now" is still going (or two "Delete now"s) purges alongside it:
/// both finish, and nothing is left.
#[tokio::test]
async fn two_purges_of_one_channel_both_finish() {
    let w = world().await;
    fill(&w.app, w.old.id, w.alex.0, 400).await;
    delete_channel(&w, w.old.id).await;
    let s = w.app.state();
    pulse_server::servers::channels::purge_partly(&s, w.old.id, 10, 1)
        .await
        .unwrap();
    let (a, b) = (w.app.state(), w.app.state());
    let now = chrono::Utc::now();
    let (x, y) = tokio::join!(
        pulse_server::servers::channels::purge_expired_batched(&a, now, 7),
        pulse_server::servers::channels::purge_expired_batched(&b, now, 11),
    );
    assert_eq!(
        x.unwrap() + y.unwrap(),
        1,
        "one of them deletes the channel"
    );
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM channels WHERE id = ?",
            &w.old.id.to_string()
        )
        .await,
        0
    );
    assert_no_orphans(&w.app).await;
}

/// Two "Delete now" clicks on one channel (#337): both purges run, and whichever reaches the last
/// transaction second finds the channel gone. That isn't a 404: the channel is gone, as asked.
#[tokio::test]
async fn a_double_delete_now_answers_204_twice() {
    let w = world().await;
    fill(&w.app, w.old.id, w.alex.0, 3_000).await;
    delete_channel(&w, w.old.id).await;
    let url = w.app.url(&format!("/channels/{}/permanent", w.old.id));
    let (a, b) = tokio::join!(
        status(w.app.http.delete(&url), &w.alex.1),
        status(w.app.http.delete(&url), &w.alex.1),
    );
    assert_eq!((a, b), (204, 204));
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM channels WHERE id = ?",
            &w.old.id.to_string()
        )
        .await,
        0
    );
    assert_no_orphans(&w.app).await;
}

/// "Delete now" racing the sweep (#337): the click answers 204 whichever purge deletes the channel.
#[tokio::test]
async fn a_delete_now_racing_the_sweep_answers_204() {
    let w = world().await;
    fill(&w.app, w.old.id, w.alex.0, 3_000).await;
    delete_channel(&w, w.old.id).await;
    let url = w.app.url(&format!("/channels/{}/permanent", w.old.id));
    let s = w.app.state();
    let click = status(w.app.http.delete(&url), &w.alex.1);
    let sweep = async {
        // Let the click make the channel due first, so the sweep takes it too.
        tokio::time::sleep(Duration::from_millis(50)).await;
        pulse_server::servers::channels::purge_expired_batched(&s, chrono::Utc::now(), 300).await
    };
    let (click, swept) = tokio::join!(click, sweep);
    swept.unwrap();
    assert_eq!(click, 204);
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM channels WHERE id = ?",
            &w.old.id.to_string()
        )
        .await,
        0
    );
    assert_no_orphans(&w.app).await;
}
