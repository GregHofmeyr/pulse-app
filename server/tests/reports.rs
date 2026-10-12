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
