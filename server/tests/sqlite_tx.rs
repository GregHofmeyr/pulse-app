//! SQLite discipline: WAL with synchronous=NORMAL, and read-then-write handlers that survive
//! concurrent writers (BEGIN IMMEDIATE) instead of failing with "database is locked" 500s.

mod common;

use std::collections::HashSet;

use common::*;
use pulse_protocol::ids::ChannelId;
use pulse_protocol::rest::Channel;
use serde_json::json;

#[tokio::test]
async fn pool_uses_wal_with_synchronous_normal() {
    let app = spawn().await;
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(mode, "wal");
    let sync: i64 = sqlx::query_scalar("PRAGMA synchronous")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(sync, 1, "synchronous=NORMAL (FULL is 2)");
}

/// Two people double-clicking "message" at once: every call answers 200 with the same DM.
#[tokio::test]
async fn concurrent_create_dm_for_one_pair_all_succeed_with_one_channel() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    for round in 0..5 {
        let (b_id, _) = register(&app, &format!("sam{round}")).await;
        let calls = (0..16).map(|_| post_json(&app, &a, "/dms", json!({ "user_ids": [b_id] })));
        let mut ids = HashSet::new();
        for r in futures::future::join_all(calls).await {
            let status = r.status();
            let body = r.text().await.unwrap();
            assert_eq!(status, 200, "round {round}: {body}");
            let ch: Channel = serde_json::from_str(&body).unwrap();
            ids.insert(ch.id);
        }
        assert_eq!(ids.len(), 1, "round {round}: one DM per pair");
    }
}

#[tokio::test]
async fn concurrent_create_channel_all_succeed_with_distinct_positions() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let s = create_server(&app, &a, "Main").await;
    let path = format!("/servers/{}/channels", s.id);
    let calls = (0..16).map(|i| {
        post_json(
            &app,
            &a,
            &path,
            json!({ "kind": "text", "name": format!("c{i}") }),
        )
    });
    let mut positions = HashSet::new();
    for r in futures::future::join_all(calls).await {
        let status = r.status();
        let body = r.text().await.unwrap();
        assert_eq!(status, 200, "{body}");
        let ch: Channel = serde_json::from_str(&body).unwrap();
        positions.insert(ch.position);
    }
    assert_eq!(positions.len(), 16, "no two channels share a position");
}

/// Make every row write of `table` fail (`op` is INSERT or DELETE), as a full disk would part-way
/// through a handler.
async fn fail_writes(app: &TestApp, op: &str, table: &str) {
    sqlx::query(&format!(
        "CREATE TRIGGER fail_{table} BEFORE {op} ON {table} BEGIN SELECT RAISE(ABORT, 'injected'); END"
    ))
    .execute(&app.db)
    .await
    .unwrap();
}

async fn members(app: &TestApp, channel: ChannelId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM channel_members WHERE channel_id = ?")
        .bind(channel.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn delete(app: &TestApp, token: &str, path: &str) -> u16 {
    app.http
        .delete(app.url(path))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

/// The message and its mentions land together: when storing the mention fails, no message
/// without its ping is left behind.
#[tokio::test]
async fn a_send_whose_mentions_fail_leaves_no_message() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let s = create_server(&app, &a, "Main").await;
    post_json(&app, &b, &format!("/servers/{}/join", s.id), json!({})).await;
    let g = general(&app, &a, s.id).await.id;
    fail_writes(&app, "INSERT", "mentions").await;

    let r = post_json(
        &app,
        &a,
        &format!("/channels/{g}/messages"),
        json!({ "content": "hi @sam" }),
    )
    .await;

    assert_eq!(r.status(), 500);
    let normal: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM messages WHERE channel_id = ? AND kind = 'normal'",
    )
    .bind(g.to_string())
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(normal, 0);
}

/// Removing a member is all or nothing: a failure part-way keeps them in the group.
#[tokio::test]
async fn a_removal_that_fails_part_way_keeps_the_member() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let (c_id, _) = register(&app, "jo").await;
    let g = create_dm(&app, &a, &[b_id, c_id]).await;
    fail_writes(&app, "DELETE", "read_states").await;

    let status = delete(&app, &a, &format!("/channels/{}/members/{c_id}", g.id)).await;

    assert_eq!(status, 500);
    assert_eq!(members(&app, g.id).await, 3);
}

/// Once the removal has committed, a failure in what follows (the announcements) must not turn
/// it into an error: the client would show a failure, and its retry would get a 404.
#[tokio::test]
async fn a_committed_removal_is_a_success_even_if_its_announcement_fails() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let (c_id, _) = register(&app, "jo").await;
    let g = create_dm(&app, &a, &[b_id, c_id]).await;
    fail_writes(&app, "INSERT", "messages").await; // the "alex removed jo" line

    let status = delete(&app, &a, &format!("/channels/{}/members/{c_id}", g.id)).await;

    assert_eq!(status, 204);
    assert_eq!(members(&app, g.id).await, 2);
}
