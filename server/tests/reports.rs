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
