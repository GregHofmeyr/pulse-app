//! The startup key check, and the owner's `reset-2fa`, against the real binary where it matters.

mod common;

use std::time::Duration;

use common::*;
use pulse_protocol::ids::UserId;
use pulse_server::auth::totp::{self, TotpKey};
use pulse_server::auth::two_factor::check_key;
use pulse_server::db;
use sqlx::SqlitePool;

const KEY_A: &str = "4242424242424242424242424242424242424242424242424242424242424242";
const KEY_B: &str = "abababababababababababababababababababababababababababababababab";
const PW: &str = "hunter2hunter2";

fn key(hex: &str) -> TotpKey {
    TotpKey::parse(&hex[..64]).unwrap()
}

/// A fresh database file with the migrations applied.
async fn fresh() -> (tempfile::TempDir, String, SqlitePool) {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("pulse.db").display());
    let pool = db::connect(&url).await.unwrap();
    (dir, url, pool)
}

/// A user with a `user_totp` row: enabled (encrypted under `k`) or only a pending setup.
async fn add_user(db: &SqlitePool, name: &str, totp: Option<(&TotpKey, bool)>) -> UserId {
    let id = UserId::new();
    sqlx::query("INSERT INTO users (id, username, password_hash, created_at) VALUES (?, ?, ?, ?)")
        .bind(id.to_string())
        .bind(name)
        .bind(pulse_server::auth::password::hash(PW).unwrap())
        .bind(db::now())
        .execute(db)
        .await
        .unwrap();
    if let Some((k, enabled)) = totp {
        let blob = totp::encrypt(k, id, &totp::new_secret());
        sqlx::query(
            "INSERT INTO user_totp (user_id, secret_enc, enabled_at, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(blob)
        .bind(enabled.then(db::now))
        .bind(db::now())
        .execute(db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO recovery_codes (user_id, code_hash) VALUES (?, 'x')")
            .bind(id.to_string())
            .execute(db)
            .await
            .unwrap();
    }
    id
}

#[tokio::test]
async fn check_key_cases() {
    let (_d, _u, db) = fresh().await;
    let (a, b) = (key(KEY_A), key(KEY_B));
    // No users with 2FA: fine with or without a key.
    add_user(&db, "sam", None).await;
    check_key(&db, None).await.unwrap();
    check_key(&db, Some(&a)).await.unwrap();
    // A pending setup is not checked, even with no key or a different one.
    add_user(&db, "pat", Some((&a, false))).await;
    check_key(&db, None).await.unwrap();
    check_key(&db, Some(&b)).await.unwrap();
    // An enabled user.
    add_user(&db, "alex", Some((&a, true))).await;
    let e = format!("{:#}", check_key(&db, None).await.unwrap_err());
    assert!(
        e.contains("1 account uses two-factor authentication but PULSE_TOTP_KEY is not set"),
        "{e}"
    );
    let e = format!("{:#}", check_key(&db, Some(&b)).await.unwrap_err());
    assert!(
        e.contains("PULSE_TOTP_KEY does not match") && e.contains("1 of 1"),
        "{e}"
    );
    check_key(&db, Some(&a)).await.unwrap();
    // One bad secret among good ones is enough to refuse.
    add_user(&db, "kim", Some((&b, true))).await;
    let e = format!("{:#}", check_key(&db, Some(&a)).await.unwrap_err());
    assert!(e.contains("1 of 2"), "{e}");
    let e = format!("{:#}", check_key(&db, None).await.unwrap_err());
    assert!(e.contains("2 accounts use"), "{e}");
}

#[tokio::test]
async fn binary_refuses_to_start_without_the_right_key() {
    let (_d, url, db) = fresh().await;
    add_user(&db, "alex", Some((&key(KEY_A), true))).await;
    db.close().await;
    let t = Duration::from_secs(20);

    let (ok, out) = run_binary(&url, &[], &["serve"], t);
    assert!(!ok, "{out}");
    assert!(out.contains("PULSE_TOTP_KEY is not set"), "{out}");

    let (ok, out) = run_binary(&url, &[("PULSE_TOTP_KEY", &KEY_B[..64])], &["serve"], t);
    assert!(!ok, "{out}");
    assert!(out.contains("PULSE_TOTP_KEY does not match"), "{out}");
    assert!(
        !out.contains(&KEY_B[..64]) && !out.contains(&KEY_A[..64]),
        "key leaked"
    );

    // A malformed key refuses too (Config::from_env), and never echoes it.
    let (ok, out) = run_binary(&url, &[("PULSE_TOTP_KEY", "not-a-key")], &["serve"], t);
    assert!(!ok, "{out}");
    assert!(
        out.contains("PULSE_TOTP_KEY") && !out.contains("not-a-key"),
        "{out}"
    );

    // The right key starts it.
    let p = spawn_binary_on(url, &[("PULSE_TOTP_KEY", &KEY_A[..64])], None).await;
    let r = reqwest::get(p.url("/health")).await.unwrap();
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn binary_without_two_factor_users_starts_without_a_key() {
    let (_d, url, db) = fresh().await;
    add_user(&db, "alex", None).await;
    db.close().await;
    let p = spawn_binary_on(url, &[], None).await;
    assert_eq!(reqwest::get(p.url("/health")).await.unwrap().status(), 200);
}

async fn login(p: &ServerProcess, user: &str) -> reqwest::StatusCode {
    reqwest::Client::new()
        .post(p.url("/auth/login"))
        .json(&serde_json::json!({"username": user, "password": PW}))
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn reset_2fa_turns_it_off() {
    let (_d, url, db) = fresh().await;
    let k = key(KEY_A);
    let alex = add_user(&db, "alex", Some((&k, true))).await;
    let sam = add_user(&db, "sam", Some((&k, true))).await;
    for (u, h) in [(alex, "t-alex"), (sam, "t-sam")] {
        sqlx::query("INSERT INTO mfa_tickets (ticket_hash, user_id, expires_at) VALUES (?, ?, ?)")
            .bind(h)
            .bind(u.to_string())
            .bind("2999-01-01T00:00:00Z")
            .execute(&db)
            .await
            .unwrap();
    }
    let env = [("PULSE_TOTP_KEY", &KEY_A[..64])];
    let p = spawn_binary_on(url.clone(), &env, None).await;
    assert_eq!(login(&p, "alex").await, 403, "2FA is on: ticket step");

    // No key needed for the reset; the name is normalised like login does.
    let (ok, out) = run_binary(&url, &[], &["reset-2fa", " Alex "], Duration::from_secs(20));
    assert!(ok, "{out}");
    assert!(
        out.contains("two-factor turned off for alex: they can sign in with their password and set it up again"),
        "{out}"
    );
    assert_eq!(login(&p, "alex").await, 200, "password alone now works");
    assert_eq!(login(&p, "sam").await, 403, "other users are untouched");
    for t in ["user_totp", "recovery_codes", "mfa_tickets"] {
        let n: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {t} WHERE user_id = ?"))
            .bind(alex.to_string())
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(n, 0, "{t}");
    }
    // An unknown user is an error, not a silent no-op.
    let (ok, out) = run_binary(&url, &[], &["reset-2fa", "nobody"], Duration::from_secs(20));
    assert!(!ok, "{out}");
    assert!(out.contains("nobody"), "{out}");
}
