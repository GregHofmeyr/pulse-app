//! Bug reports: friends can file them; nobody can read them over HTTP.
mod common;

use common::*;
use pulse_server::limits::{LimitsConfig, Rate};
use serde_json::json;

fn report(kind: &str, text: &str, log: &str) -> serde_json::Value {
    json!({ "kind": kind, "text": text, "meta": { "app_version": "0.1.0" }, "log": log })
}

#[tokio::test]
async fn a_report_is_stored_and_its_id_returned() {
    let app = spawn().await;
    let (alex, token) = register(&app, "alex").await;
    let r = post_json(
        &app,
        &token,
        "/reports",
        report("bug", "voice cut out", "line1\nline2\n"),
    )
    .await;
    assert_eq!(r.status(), 200);
    let id = r.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (user_id, kind, text, log): (String, String, String, String) =
        sqlx::query_as("SELECT user_id, kind, text, log FROM bug_reports WHERE id = ?")
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        (user_id, kind.as_str(), text.as_str(), log.as_str()),
        (alex.to_string(), "bug", "voice cut out", "line1\nline2\n")
    );
}

#[tokio::test]
async fn bad_reports_are_rejected() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let long = "x".repeat(4001);
    assert_eq!(
        post_json(&app, &token, "/reports", report("bug", "   ", ""))
            .await
            .status(),
        400
    );
    assert_eq!(
        post_json(&app, &token, "/reports", report("bug", &long, ""))
            .await
            .status(),
        400
    );
    assert_eq!(
        post_json(&app, &token, "/reports", report("nope", "x", ""))
            .await
            .status(),
        422
    );
    let big_log = "y".repeat(512 * 1024 + 1);
    assert_eq!(
        post_json(&app, &token, "/reports", report("bug", "x", &big_log))
            .await
            .status(),
        400
    );
    let huge = "z".repeat(1024 * 1024 + 1);
    assert_eq!(
        post_json(&app, &token, "/reports", report("bug", "x", &huge))
            .await
            .status(),
        413
    );
    // automatic reports may be empty
    assert_eq!(
        post_json(&app, &token, "/reports", report("crash", "", ""))
            .await
            .status(),
        200
    );
}

#[tokio::test]
async fn reports_need_a_session() {
    let app = spawn().await;
    let r = app
        .http
        .post(app.url("/reports"))
        .json(&report("bug", "x", ""))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn five_reports_an_hour() {
    let mut cfg = test_config();
    cfg.limits = LimitsConfig {
        report_user: Rate::per_hour(5),
        ..LimitsConfig::unlimited()
    };
    let app = spawn_with(cfg).await;
    let (_, token) = register(&app, "alex").await;
    for _ in 0..5 {
        assert_eq!(
            post_json(&app, &token, "/reports", report("bug", "x", ""))
                .await
                .status(),
            200
        );
    }
    assert_eq!(
        post_json(&app, &token, "/reports", report("bug", "x", ""))
            .await
            .status(),
        429
    );
}

#[tokio::test]
async fn reports_cannot_be_read_over_http() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let r = app
        .http
        .get(app.url("/reports"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert!(r.status() == 404 || r.status() == 405, "{}", r.status());
}

use pulse_server::reports::store;

async fn file(
    app: &TestApp,
    token: &str,
    kind: &str,
    text: &str,
    meta: serde_json::Value,
) -> String {
    let body = json!({ "kind": kind, "text": text, "meta": meta, "log": "the log\n" });
    let r = post_json(app, token, "/reports", body).await;
    r.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn list_is_newest_first_with_unseen_marked() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let first = file(&app, &token, "bug", "first one", json!({})).await;
    let second = file(
        &app,
        &token,
        "crash",
        "",
        json!({ "crash": { "message": "index out of bounds" } }),
    )
    .await;
    let list = store::list(&app.db, 20).await.unwrap();
    assert_eq!(
        list.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        [second.as_str(), first.as_str()]
    );
    assert_eq!(list[0].username, "alex");
    assert_eq!(list[0].kind, "crash");
    assert_eq!(
        list[0].headline, "index out of bounds",
        "crashes headline the panic message"
    );
    assert_eq!(list[1].headline, "first one");
    assert!(list.iter().all(|r| !r.seen));
    assert_eq!(list[0].short, store::short(&second));
}

#[tokio::test]
async fn find_by_short_id_and_mark_seen() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let id = file(
        &app,
        &token,
        "bug",
        "voice cut out",
        json!({ "os": "Windows 11" }),
    )
    .await;
    let full = store::find(&app.db, &store::short(&id).to_lowercase())
        .await
        .unwrap();
    assert_eq!(full.listed.id, id);
    assert_eq!(full.text, "voice cut out");
    assert_eq!(full.meta["os"], "Windows 11");
    assert_eq!(full.log, "the log\n");
    store::mark_seen(&app.db, &id).await.unwrap();
    assert!(store::list(&app.db, 20).await.unwrap()[0].seen);
    assert!(matches!(
        store::find(&app.db, "ZZZZZZ").await,
        Err(pulse_server::error::AppError::NotFound)
    ));
}

#[tokio::test]
async fn ambiguous_suffix_lists_the_candidates() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    file(&app, &token, "bug", "a", json!({})).await;
    file(&app, &token, "bug", "b", json!({})).await;
    // two ids that end alike (ULIDs are random at the end, so set them by hand)
    for id in ["01M4QWAAAAAAAAAAAAAAXYZ123", "01M4QWBBBBBBBBBBBBBBXYZ123"] {
        sqlx::query("UPDATE bug_reports SET id = ? WHERE id = (SELECT id FROM bug_reports WHERE id NOT LIKE '%XYZ123' LIMIT 1)")
            .bind(id)
            .execute(&app.db)
            .await
            .unwrap();
    }
    match store::find(&app.db, "xyz123").await {
        Err(pulse_server::error::AppError::Conflict(m)) => {
            assert!(m.contains("AAAAXYZ123") && m.contains("BBBBXYZ123"), "{m}")
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
}

#[tokio::test]
async fn a_very_ambiguous_id_lists_only_a_few() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    for i in 0..15 {
        file(&app, &token, "bug", &format!("r{i}"), json!({})).await;
    }
    sqlx::query(
        "UPDATE bug_reports SET id = substr(id, 1, 10) || printf('%011d', rowid) || 'QQQQQ'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    match store::find(&app.db, "q").await {
        Err(pulse_server::error::AppError::Conflict(m)) => {
            assert!(
                m.contains("15 reports") && m.matches(", ").count() <= 10,
                "{m}"
            )
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
}

#[tokio::test]
async fn short_ids_are_plain_letters_and_digits() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    file(&app, &token, "bug", "a", json!({})).await;
    // "" or a LIKE wildcard would otherwise match every report
    for bad in ["", "%", "_A", "AB CD"] {
        assert!(
            matches!(
                store::find(&app.db, bad).await,
                Err(pulse_server::error::AppError::BadRequest(_))
            ),
            "{bad:?}"
        );
    }
}

// #218: feature requests are their own kind, with no log or diagnostics attached.
#[tokio::test]
async fn an_idea_is_stored_as_its_own_kind_and_listed_as_one() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let body = json!({ "kind": "idea", "text": "Let me pin a clip\nto the top of the wall", "meta": {}, "log": "" });
    let r = post_json(&app, &token, "/reports", body).await;
    assert_eq!(r.status(), 200);
    let id = r.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    file(&app, &token, "bug", "voice cut out", json!({})).await;
    let list = store::list(&app.db, 20).await.unwrap();
    let idea = list.iter().find(|l| l.id == id).unwrap();
    assert_eq!(idea.kind, "idea");
    assert_eq!(idea.headline, "Let me pin a clip");
    let full = store::find(&app.db, &store::short(&id)).await.unwrap();
    assert_eq!((full.log.as_str(), full.meta.clone()), ("", json!({})));
}

#[tokio::test]
async fn an_idea_needs_text_and_carries_no_log_or_diagnostics() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let idea = |text: &str, meta: serde_json::Value, log: &str| json!({ "kind": "idea", "text": text, "meta": meta, "log": log });
    for (body, why) in [
        (idea("   ", json!({}), ""), "empty text"),
        (idea("x", json!({}), "a log line\n"), "a log"),
        (idea("x", json!({ "os": "Windows 11" }), ""), "diagnostics"),
    ] {
        assert_eq!(
            post_json(&app, &token, "/reports", body).await.status(),
            400,
            "{why}"
        );
    }
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bug_reports")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

const OLD_USER: &str = "01J0000000000000000000USER";
const OLD_TS: &str = "2026-11-01T00:00:00.000Z";

async fn insert_report(pool: &sqlx::SqlitePool, id: &str, kind: &str) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO bug_reports (id, user_id, created_at, kind, text, meta, log) VALUES (?, ?, ?, ?, 't', '{\"os\":\"x\"}', 'log\n')",
    )
    .bind(id)
    .bind(OLD_USER)
    .bind(OLD_TS)
    .bind(kind)
    .execute(pool)
    .await
    .map(|_| ())
}

#[tokio::test]
async fn migration_0027_keeps_every_report_and_allows_ideas() {
    use sqlx::sqlite::SqliteConnectOptions;
    use std::borrow::Cow;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(false);
    let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();

    // A database as the previous release left it: migrations up to 0026 only.
    let mut migrator = sqlx::migrate::Migrator::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"),
    )
    .await
    .unwrap();
    assert!(
        migrator.migrations.iter().any(|m| m.version == 27),
        "0027 exists"
    );
    migrator.migrations = Cow::Owned(
        migrator
            .migrations
            .iter()
            .filter(|m| m.version <= 26)
            .cloned()
            .collect(),
    );
    migrator.run(&pool).await.unwrap();

    sqlx::query(&format!(
        "INSERT INTO users (id, username, password_hash, created_at) VALUES ('{OLD_USER}', 'alex', 'x', '{OLD_TS}')"
    ))
    .execute(&pool)
    .await
    .unwrap();
    for (id, kind) in [
        ("01J00000000000000000BUG001", "bug"),
        ("01J00000000000000000CRASH1", "crash"),
        ("01J00000000000000000EXIT01", "unexpected-exit"),
    ] {
        insert_report(&pool, id, kind).await.unwrap();
    }
    sqlx::query("UPDATE bug_reports SET seen_at = ? WHERE kind = 'bug'")
        .bind(OLD_TS)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        insert_report(&pool, "01J0000000000000000IDEA00", "idea")
            .await
            .is_err(),
        "the old CHECK refuses an idea"
    );
    pool.close().await;

    // Upgrade the way the server does at startup.
    let pool = pulse_server::db::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    type Row = (
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, user_id, created_at, kind, meta, log, seen_at FROM bug_reports ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        rows.iter().map(|r| r.3.as_str()).collect::<Vec<_>>(),
        ["bug", "crash", "unexpected-exit"]
    );
    assert!(
        rows.iter()
            .all(|r| r.1 == OLD_USER && r.2 == OLD_TS && r.4 == "{\"os\":\"x\"}" && r.5 == "log\n")
    );
    assert_eq!(rows[0].6.as_deref(), Some(OLD_TS), "seen survives");
    insert_report(&pool, "01J0000000000000000IDEA00", "idea")
        .await
        .unwrap();
    assert!(
        insert_report(&pool, "01J0000000000000000NOPE00", "nope")
            .await
            .is_err(),
        "unknown kinds are still refused"
    );
    let idx: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'bug_reports_created'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(idx, 1, "the listing index is back");
    let ok: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(ok, "ok");
    let broken: Vec<(String, Option<i64>, String, i64)> =
        sqlx::query_as("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(broken.is_empty(), "{broken:?}");
    sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(OLD_USER)
        .execute(&pool)
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bug_reports")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0, "reports still go with their user");
}
