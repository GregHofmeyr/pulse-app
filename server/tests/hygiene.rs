//! Periodic database cleanup, and the free-disk guard on write routes.
mod common;

use chrono::{Duration, Utc};
use common::*;
use pulse_server::hygiene::prune;
use serde_json::json;

fn ts(d: Duration) -> String {
    (Utc::now() + d).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

async fn count(app: &TestApp, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&app.db).await.unwrap()
}

#[tokio::test]
async fn prune_removes_expired_rows_and_keeps_fresh_ones() {
    let app = spawn().await;
    let (alex, _token) = register(&app, "alex").await;
    let (sam, _) = register(&app, "sam").await;
    let (alex, sam) = (alex.to_string(), sam.to_string());

    // Sessions: register made one fresh session each; add an expired one for alex.
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, created_at, last_used_at, expires_at) VALUES ('old', ?, ?, ?, ?)",
    )
    .bind(&alex)
    .bind(ts(Duration::days(-200)))
    .bind(ts(Duration::days(-100)))
    .bind(ts(Duration::days(-10)))
    .execute(&app.db)
    .await
    .unwrap();
    // A LiveKit grant on the expired session goes with it (FK cascade).
    sqlx::query(
        "INSERT INTO livekit_grants (id, token_hash, room, identity, issued_at) VALUES ('g', 'old', 'r', 'alex', ?)",
    )
    .bind(ts(Duration::days(-11)))
    .execute(&app.db)
    .await
    .unwrap();

    // Invites: expired unused, expired used (by sam), fresh unused, fresh used.
    for (code, age_h, used_by) in [
        ("expiredunused", 30, None),
        ("expiredused", 30, Some(&sam)),
        ("freshunused", 1, None),
        ("freshused", 1, Some(&sam)),
    ] {
        sqlx::query(
            "INSERT INTO invites (code, created_by, created_at, used_by, used_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(code)
        .bind(&alex)
        .bind(ts(Duration::hours(-age_h)))
        .bind(used_by)
        .bind(used_by.map(|_| ts(Duration::hours(-age_h))))
        .execute(&app.db)
        .await
        .unwrap();
    }

    // MFA tickets.
    for (hash, exp) in [
        ("told", Duration::minutes(-1)),
        ("tnew", Duration::minutes(4)),
    ] {
        sqlx::query("INSERT INTO mfa_tickets (ticket_hash, user_id, expires_at) VALUES (?, ?, ?)")
            .bind(hash)
            .bind(&alex)
            .bind(ts(exp))
            .execute(&app.db)
            .await
            .unwrap();
    }

    // Reports: 91 days old goes, 89 days old stays.
    for (id, age) in [("rold", 91), ("rnew", 89)] {
        sqlx::query(
            "INSERT INTO bug_reports (id, user_id, created_at, kind, text, meta, log) VALUES (?, ?, ?, 'bug', 't', '{}', '')",
        )
        .bind(id)
        .bind(&alex)
        .bind(ts(Duration::days(-age)))
        .execute(&app.db)
        .await
        .unwrap();
    }

    let n = prune(&app.db, Utc::now()).await.unwrap();
    assert_eq!(
        (n.sessions, n.invites, n.mfa_tickets, n.reports),
        (1, 2, 1, 1)
    );

    assert_eq!(
        count(
            &app,
            "SELECT COUNT(*) FROM sessions WHERE token_hash = 'old'"
        )
        .await,
        0
    );
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM sessions").await,
        2,
        "fresh sessions stay"
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM livekit_grants").await, 0);
    let codes: Vec<String> = sqlx::query_scalar(
        "SELECT code FROM invites WHERE code LIKE 'fresh%' OR code LIKE 'expired%' ORDER BY code",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(codes, ["freshunused", "freshused"]);
    let t: Vec<String> = sqlx::query_scalar("SELECT ticket_hash FROM mfa_tickets")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(t, ["tnew"]);
    let r: Vec<String> = sqlx::query_scalar("SELECT id FROM bug_reports")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(r, ["rnew"]);
    // The users who used or created pruned invites are untouched.
    assert_eq!(count(&app, "SELECT COUNT(*) FROM users").await, 2);

    // A second pass has nothing to do.
    assert_eq!(
        prune(&app.db, Utc::now()).await.unwrap(),
        Default::default()
    );
}

#[tokio::test]
async fn invite_cap_still_counts_after_prune() {
    let app = spawn().await;
    let (alex, token) = register(&app, "alex").await;
    let r = post_json(&app, &token, "/invites", json!({})).await;
    assert_eq!(r.status(), 200);
    prune(&app.db, Utc::now()).await.unwrap();
    let n = count(
        &app,
        &format!("SELECT COUNT(*) FROM invites WHERE created_by = '{alex}' AND used_by IS NULL"),
    )
    .await;
    assert_eq!(n, 1, "a fresh unused invite survives");
}

#[tokio::test]
async fn writes_refused_with_507_when_disk_is_low() {
    let mut cfg = test_config();
    cfg.media_min_free_bytes = u64::MAX;
    let app = spawn_with(cfg).await;
    let (_, token) = register(&app, "alex").await;
    let srv = create_server(&app, &token, "Main").await;
    let ch = general(&app, &token, srv.id).await;

    let r = post_json(
        &app,
        &token,
        &format!("/channels/{}/messages", ch.id),
        json!({ "content": "hi" }),
    )
    .await;
    assert_eq!(r.status(), 507, "message");
    let r = post_json(
        &app,
        &token,
        "/reports",
        json!({ "kind": "bug", "text": "x", "meta": {}, "log": "" }),
    )
    .await;
    assert_eq!(r.status(), 507, "report");
    let r = post_json(
        &app,
        &token,
        &format!("/servers/{}/emoji", srv.id),
        json!({ "name": "wave", "media_hash": "0".repeat(64) }),
    )
    .await;
    assert_eq!(r.status(), 507, "emoji");
    assert_eq!(count(&app, "SELECT COUNT(*) FROM messages").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM bug_reports").await, 0);
}
