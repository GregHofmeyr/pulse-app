//! SQLite discipline: WAL with synchronous=NORMAL, and read-then-write handlers that survive
//! concurrent writers (BEGIN IMMEDIATE) instead of failing with "database is locked" 500s.

mod common;

use std::collections::HashSet;

use common::*;
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
