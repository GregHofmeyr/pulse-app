mod common;

use common::*;
use pulse_protocol::rest::{SessionResponse, User};
use serde_json::json;

async fn post(app: &TestApp, path: &str, body: serde_json::Value) -> reqwest::Response {
    app.http
        .post(app.url(path))
        .json(&body)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn register_with_invite_then_me() {
    let app = spawn().await;
    let code = invite(&app).await;
    let r = post(
        &app,
        "/auth/register",
        json!({"invite_code": code, "username": "alex", "password": "hunter2hunter2"}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let s: SessionResponse = r.json().await.unwrap();
    let me: User = app
        .http
        .get(app.url("/me"))
        .bearer_auth(&s.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me.username, "alex");
    assert_eq!(me.id, s.user.id);
}

#[tokio::test]
async fn register_bad_invite_404() {
    let app = spawn().await;
    let r = post(
        &app,
        "/auth/register",
        json!({"invite_code": "nope", "username": "alex", "password": "hunter2hunter2"}),
    )
    .await;
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn invite_single_use_410() {
    let app = spawn().await;
    let code = invite(&app).await;
    assert_eq!(
        post(
            &app,
            "/auth/register",
            json!({"invite_code": code, "username": "alex", "password": "hunter2hunter2"})
        )
        .await
        .status(),
        200
    );
    assert_eq!(
        post(
            &app,
            "/auth/register",
            json!({"invite_code": code, "username": "sam", "password": "hunter2hunter2"})
        )
        .await
        .status(),
        410
    );
}

#[tokio::test]
async fn concurrent_redemption_creates_one_user() {
    let app = std::sync::Arc::new(spawn().await);
    let code = invite(&app).await;
    let mut tasks = vec![];
    for i in 0..10 {
        let app = app.clone();
        let code = code.clone();
        tasks.push(tokio::spawn(async move {
            post(&app, "/auth/register", json!({"invite_code": code, "username": format!("user{i}"), "password": "hunter2hunter2"}))
                .await
                .status()
                .as_u16()
        }));
    }
    let mut statuses = vec![];
    for t in tasks {
        statuses.push(t.await.unwrap());
    }
    assert_eq!(
        statuses.iter().filter(|s| **s == 200).count(),
        1,
        "{statuses:?}"
    );
    assert!(
        statuses.iter().all(|s| *s == 200 || *s == 410),
        "{statuses:?}"
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn duplicate_username_409() {
    let app = spawn().await;
    register(&app, "alex").await;
    let code = invite(&app).await;
    let r = post(
        &app,
        "/auth/register",
        json!({"invite_code": code, "username": "alex", "password": "hunter2hunter2"}),
    )
    .await;
    assert_eq!(r.status(), 409);
    // the invite was not burned by the failed attempt
    let used: Option<String> = sqlx::query_scalar("SELECT used_by FROM invites WHERE code = ?")
        .bind(&code)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(used.is_none());
}

#[tokio::test]
async fn bad_username_or_short_password_400() {
    let app = spawn().await;
    for (u, p) in [
        ("A", "hunter2hunter2"),
        ("Has Space", "hunter2hunter2"),
        ("alex", "short"),
    ] {
        let code = invite(&app).await;
        let r = post(
            &app,
            "/auth/register",
            json!({"invite_code": code, "username": u, "password": p}),
        )
        .await;
        assert_eq!(r.status(), 400, "{u}/{p}");
    }
}

#[tokio::test]
async fn login_ok_and_wrong_password_401() {
    let app = spawn().await;
    register(&app, "alex").await;
    assert_eq!(
        post(
            &app,
            "/auth/login",
            json!({"username": "alex", "password": "hunter2hunter2"})
        )
        .await
        .status(),
        200
    );
    assert_eq!(
        post(
            &app,
            "/auth/login",
            json!({"username": "alex", "password": "wrongwrong"})
        )
        .await
        .status(),
        401
    );
    assert_eq!(
        post(
            &app,
            "/auth/login",
            json!({"username": "ghost", "password": "wrongwrong"})
        )
        .await
        .status(),
        401
    );
}

#[tokio::test]
async fn logout_revokes_token() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let r = app
        .http
        .post(app.url("/auth/logout"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(
        app.http
            .get(app.url("/me"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
}

#[tokio::test]
async fn me_without_or_with_bad_token_401() {
    let app = spawn().await;
    assert_eq!(
        app.http.get(app.url("/me")).send().await.unwrap().status(),
        401
    );
    assert_eq!(
        app.http
            .get(app.url("/me"))
            .bearer_auth("nope")
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
}

#[tokio::test]
async fn members_can_create_invites() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let r = app
        .http
        .post(app.url("/invites"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: serde_json::Value = r.json().await.unwrap();
    let code = v["code"].as_str().unwrap();
    assert_eq!(
        post(
            &app,
            "/auth/register",
            json!({"invite_code": code, "username": "sam", "password": "hunter2hunter2"})
        )
        .await
        .status(),
        200
    );
}
