mod common;

use pulse_protocol::ids::UserId;
use pulse_server::auth::{password, session};
use sqlx::SqlitePool;

async fn user(db: &SqlitePool) -> UserId {
    let id = UserId::new();
    sqlx::query("INSERT INTO users (id, username, password_hash, created_at) VALUES (?, ?, 'x', '2026-01-01T00:00:00.000Z')")
        .bind(id.to_string())
        .bind(format!("u{}", &id.to_string()[20..]).to_lowercase())
        .execute(db)
        .await
        .unwrap();
    id
}

#[test]
fn hash_then_verify_roundtrip() {
    let h = password::hash("correct horse").unwrap();
    assert!(password::verify("correct horse", &h));
}

#[test]
fn verify_rejects_wrong_password() {
    let h = password::hash("correct horse").unwrap();
    assert!(!password::verify("wrong horse", &h));
    assert!(!password::verify("anything", "not-a-hash"));
}

#[tokio::test]
async fn token_authenticates() {
    let app = common::spawn().await;
    let u = user(&app.db).await;
    let t = session::create(&app.db, u).await.unwrap();
    assert_eq!(session::authenticate(&app.db, &t).await.unwrap(), Some(u));
    assert_eq!(
        session::authenticate(&app.db, "garbage").await.unwrap(),
        None
    );
}

#[tokio::test]
async fn stored_value_is_hash_not_token() {
    let app = common::spawn().await;
    let u = user(&app.db).await;
    let t = session::create(&app.db, u).await.unwrap();
    let stored: String = sqlx::query_scalar("SELECT token_hash FROM sessions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_ne!(stored, t);
    assert!(!stored.contains(&t));
    assert!(t.len() >= 43, "256-bit token, base64url");
}

#[tokio::test]
async fn expired_token_rejected() {
    let app = common::spawn().await;
    let u = user(&app.db).await;
    let t = session::create(&app.db, u).await.unwrap();
    sqlx::query("UPDATE sessions SET expires_at = '2000-01-01T00:00:00.000Z'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(session::authenticate(&app.db, &t).await.unwrap(), None);
}

#[tokio::test]
async fn use_slides_expiry_forward() {
    let app = common::spawn().await;
    let u = user(&app.db).await;
    let t = session::create(&app.db, u).await.unwrap();
    // pretend it was last used long ago and expires soon
    let soon = (chrono::Utc::now() + chrono::Duration::days(1))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    sqlx::query("UPDATE sessions SET expires_at = ?, last_used_at = '2000-01-01T00:00:00.000Z'")
        .bind(&soon)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(session::authenticate(&app.db, &t).await.unwrap(), Some(u));
    let exp: String = sqlx::query_scalar("SELECT expires_at FROM sessions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let exp = chrono::DateTime::parse_from_rfc3339(&exp).unwrap();
    assert!(exp > chrono::Utc::now() + chrono::Duration::days(89));
}

#[tokio::test]
async fn revoked_token_rejected() {
    let app = common::spawn().await;
    let u = user(&app.db).await;
    let t = session::create(&app.db, u).await.unwrap();
    session::revoke(&app.db, &t).await.unwrap();
    assert_eq!(session::authenticate(&app.db, &t).await.unwrap(), None);
}
