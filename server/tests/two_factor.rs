//! `/auth/2fa/*`: status, enrolment, turning off, regenerating recovery codes.

mod common;

use std::time::{SystemTime, UNIX_EPOCH};

use common::*;
use pulse_protocol::rest::{
    RecoveryCodesResponse, TwoFactorEnabledResponse, TwoFactorSetupResponse, TwoFactorStatus,
};
use pulse_server::auth::{session, totp, two_factor};
use pulse_server::config::Config;
use pulse_server::limits::{LimitsConfig, Rate};
use serde_json::json;
use totp_rs::{Algorithm, Builder, Secret};

const PW: &str = "hunter2hunter2";

/// The code for the step `offset` steps from now. Offsets 0 and +1 are always inside the
/// server's window, however close to a step boundary the test runs.
fn code_at(secret_b32: &str, offset: i64) -> String {
    let secret = Secret::try_from_base32(secret_b32)
        .unwrap()
        .as_bytes()
        .to_vec();
    let t = Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_skew(0)
        .with_step_duration(30)
        .with_secret(secret)
        .build_noncompliant();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    t.generate((now as i64 + offset * 30) as u64).to_string()
}

/// A 6-digit code that is certainly not valid now: never one of the three in the window.
fn wrong_code(secret_b32: &str) -> String {
    let valid: Vec<String> = (-1..=1).map(|o| code_at(secret_b32, o)).collect();
    ["000000", "000001"]
        .into_iter()
        .find(|c| !valid.iter().any(|v| v == c))
        .unwrap()
        .to_string()
}

async fn setup(app: &TestApp, token: &str) -> reqwest::Response {
    post_json(app, token, "/auth/2fa/setup", json!({"password": PW})).await
}

async fn setup_ok(app: &TestApp, token: &str) -> TwoFactorSetupResponse {
    let r = setup(app, token).await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

async fn code_req(app: &TestApp, token: &str, path: &str, code: &str) -> reqwest::Response {
    post_json(app, token, path, json!({ "code": code })).await
}

/// Setup + enable with the current code.
async fn turn_on(app: &TestApp, token: &str) -> (String, TwoFactorEnabledResponse) {
    let s = setup_ok(app, token).await;
    let r = code_req(app, token, "/auth/2fa/enable", &code_at(&s.secret, 0)).await;
    assert_eq!(r.status(), 200);
    (s.secret, r.json().await.unwrap())
}

async fn status(app: &TestApp, token: &str) -> TwoFactorStatus {
    get_json(app, token, "/auth/2fa").await
}

async fn login(app: &TestApp, username: &str) -> String {
    let r: pulse_protocol::rest::SessionResponse = app
        .http
        .post(app.url("/auth/login"))
        .json(&json!({"username": username, "password": PW}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    r.token
}

fn assert_code_format(c: &str) {
    assert_eq!(c.len(), 19, "{c}");
    assert!(
        c.split('-').count() == 4 && c.split('-').all(|g| g.len() == 4),
        "{c}"
    );
}

#[tokio::test]
async fn status_off_and_available() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let st = status(&app, &a).await;
    assert_eq!(
        st,
        TwoFactorStatus {
            enabled: false,
            available: true,
            recovery_codes_left: 0
        }
    );
    let none = app.http.get(app.url("/auth/2fa")).send().await.unwrap();
    assert_eq!(none.status(), 401);
}

#[tokio::test]
async fn setup_with_wrong_password_is_403_and_the_session_survives() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let r = post_json(
        &app,
        &a,
        "/auth/2fa/setup",
        json!({"password": "nope-nope-nope"}),
    )
    .await;
    assert_eq!(r.status(), 403);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["message"], "wrong password");
    let me = app
        .http
        .get(app.url("/me"))
        .bearer_auth(&a)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), 200);
}

#[tokio::test]
async fn setup_returns_a_secret_and_uri_and_enable_checks_the_code() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let s = setup_ok(&app, &a).await;
    assert!(
        s.otpauth_uri
            .starts_with("otpauth://totp/Pulse:alex?secret=")
    );
    assert!(s.otpauth_uri.contains(&s.secret));
    assert_eq!(s.secret.len(), 32);
    // Pending only: not on yet.
    assert!(!status(&app, &a).await.enabled);

    let good = code_at(&s.secret, 0);
    let r = code_req(&app, &a, "/auth/2fa/enable", &wrong_code(&s.secret)).await;
    assert_eq!(r.status(), 403);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["message"], "wrong code");
    // A recovery-code-shaped value is simply wrong here.
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/enable", "abcd-efgh-jkmn-pqrs")
            .await
            .status(),
        403
    );
    assert!(!status(&app, &a).await.enabled);

    let r = code_req(&app, &a, "/auth/2fa/enable", &good).await;
    assert_eq!(r.status(), 200);
    let on: TwoFactorEnabledResponse = r.json().await.unwrap();
    assert_eq!(on.recovery_codes.len(), 10);
    on.recovery_codes.iter().for_each(|c| assert_code_format(c));
    assert_eq!(on.signed_out, 0);
    assert_eq!(
        status(&app, &a).await,
        TwoFactorStatus {
            enabled: true,
            available: true,
            recovery_codes_left: 10
        }
    );
}

#[tokio::test]
async fn setup_again_replaces_the_pending_secret() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let first = setup_ok(&app, &a).await;
    let second = setup_ok(&app, &a).await;
    assert_ne!(first.secret, second.secret);
    // Only the newest secret enables.
    let old = code_at(&first.secret, 0);
    if old != code_at(&second.secret, 0) {
        assert_eq!(
            code_req(&app, &a, "/auth/2fa/enable", &old).await.status(),
            403
        );
    }
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/enable", &code_at(&second.secret, 0))
            .await
            .status(),
        200
    );
}

#[tokio::test]
async fn enabling_signs_out_other_sessions_of_that_user_only() {
    let app = spawn().await;
    let (a_id, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let (s_id, s) = register(&app, "sam").await;

    let (_, on) = turn_on(&app, &a1).await;
    assert_eq!(on.signed_out, 1);
    let me = |t: String| {
        let app = &app;
        async move {
            app.http
                .get(app.url("/me"))
                .bearer_auth(t)
                .send()
                .await
                .unwrap()
                .status()
        }
    };
    assert_eq!(me(a1.clone()).await, 200);
    assert_eq!(me(a2).await, 401);
    assert_eq!(me(s.clone()).await, 200);
    assert_eq!(
        session::authenticate(&app.db, &a1).await.unwrap(),
        Some(a_id)
    );
    assert_eq!(
        session::authenticate(&app.db, &s).await.unwrap(),
        Some(s_id)
    );
}

#[tokio::test]
async fn setup_while_on_and_enable_without_setup_are_409() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/enable", "123456")
            .await
            .status(),
        409
    );
    turn_on(&app, &a).await;
    assert_eq!(setup(&app, &a).await.status(), 409);
    // Enable again with nothing pending is a 409 too.
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/enable", "123456")
            .await
            .status(),
        409
    );
}

#[tokio::test]
async fn secret_and_recovery_codes_are_not_stored_in_the_clear() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (secret, on) = turn_on(&app, &a).await;
    let raw = Secret::try_from_base32(secret).unwrap().as_bytes().to_vec();
    let blob: Vec<u8> = sqlx::query_scalar("SELECT secret_enc FROM user_totp WHERE user_id = ?")
        .bind(a_id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(blob.len(), 12 + 20 + 16);
    assert!(!blob.windows(raw.len()).any(|w| w == raw.as_slice()));
    let hashes: Vec<String> =
        sqlx::query_scalar("SELECT code_hash FROM recovery_codes WHERE user_id = ?")
            .bind(a_id.to_string())
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(hashes.len(), 10);
    for c in &on.recovery_codes {
        assert!(hashes.contains(&totp::hash_recovery(c)));
        assert!(
            !hashes
                .iter()
                .any(|h| h.contains(&totp::normalize_recovery(c)))
        );
    }
}

#[tokio::test]
async fn regenerate_needs_a_totp_code_and_replaces_all_codes() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    // Off: 409.
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/recovery-codes", "123456")
            .await
            .status(),
        409
    );
    let (secret, on) = turn_on(&app, &a).await;
    // A recovery code isn't accepted here.
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/recovery-codes", &on.recovery_codes[0])
            .await
            .status(),
        403
    );
    let r = code_req(&app, &a, "/auth/2fa/recovery-codes", &code_at(&secret, 1)).await;
    assert_eq!(r.status(), 200);
    let fresh: RecoveryCodesResponse = r.json().await.unwrap();
    assert_eq!(fresh.recovery_codes.len(), 10);
    fresh
        .recovery_codes
        .iter()
        .for_each(|c| assert_code_format(c));
    assert!(
        fresh
            .recovery_codes
            .iter()
            .all(|c| !on.recovery_codes.contains(c))
    );
    assert_eq!(status(&app, &a).await.recovery_codes_left, 10);
    // An old code no longer works, a new one does.
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/disable", &on.recovery_codes[0])
            .await
            .status(),
        403
    );
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/disable", &fresh.recovery_codes[3])
            .await
            .status(),
        204
    );
}

#[tokio::test]
async fn disable_with_a_recovery_code_clears_everything() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (_, on) = turn_on(&app, &a).await;
    // A ticket left over from a login is removed too.
    sqlx::query("INSERT INTO mfa_tickets (ticket_hash, user_id, expires_at) VALUES ('t', ?, '2999-01-01T00:00:00.000Z')")
        .bind(a_id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    // Wrong value first.
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/disable", "zzzz-zzzz-zzzz-zzzz")
            .await
            .status(),
        403
    );
    assert!(status(&app, &a).await.enabled);

    let code = on.recovery_codes[0].to_uppercase().replace('-', " ");
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/disable", &code)
            .await
            .status(),
        204
    );
    assert_eq!(
        status(&app, &a).await,
        TwoFactorStatus {
            enabled: false,
            available: true,
            recovery_codes_left: 0
        }
    );
    for t in ["user_totp", "recovery_codes", "mfa_tickets"] {
        let n: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {t}"))
            .fetch_one(&app.db)
            .await
            .unwrap();
        assert_eq!(n, 0, "{t}");
    }
    // Off now: 409, and the old code can't come back by turning 2FA on again.
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/disable", &on.recovery_codes[0])
            .await
            .status(),
        409
    );
    turn_on(&app, &a).await;
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/disable", &on.recovery_codes[0])
            .await
            .status(),
        403
    );
}

#[tokio::test]
async fn a_recovery_code_is_consumed_atomically() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (_, on) = turn_on(&app, &a).await;
    let mut conn = app.db.acquire().await.unwrap();
    assert!(
        two_factor::consume_recovery(&mut conn, a_id, &on.recovery_codes[0])
            .await
            .unwrap()
    );
    assert!(
        !two_factor::consume_recovery(&mut conn, a_id, &on.recovery_codes[0])
            .await
            .unwrap()
    );
    assert!(
        !two_factor::consume_recovery(&mut conn, a_id, "nnnn-nnnn-nnnn-nnnn")
            .await
            .unwrap()
    );
    drop(conn);
    assert_eq!(status(&app, &a).await.recovery_codes_left, 9);
}

#[tokio::test]
async fn disable_with_totp_works() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (secret, _) = turn_on(&app, &a).await;
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/disable", "12345")
            .await
            .status(),
        403
    );
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/disable", &code_at(&secret, 1))
            .await
            .status(),
        204
    );
    assert!(!status(&app, &a).await.enabled);
}

#[tokio::test]
async fn a_totp_code_works_once() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let s = setup_ok(&app, &a).await;
    let code = code_at(&s.secret, 0);
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/enable", &code).await.status(),
        200
    );
    // The same code again (same step) is refused, however it is used.
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/recovery-codes", &code)
            .await
            .status(),
        403
    );
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/disable", &code)
            .await
            .status(),
        403
    );
    assert!(status(&app, &a).await.enabled);
}

#[tokio::test]
async fn racing_requests_with_one_code_have_one_winner() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (secret, _) = turn_on(&app, &a).await;
    let code = code_at(&secret, 1);
    let rs = futures::future::join_all(
        (0..4).map(|_| code_req(&app, &a, "/auth/2fa/recovery-codes", &code)),
    )
    .await;
    let ok = rs.iter().filter(|r| r.status() == 200).count();
    assert_eq!(
        ok,
        1,
        "{:?}",
        rs.iter().map(|r| r.status()).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn wrong_codes_are_rate_limited_and_a_refused_attempt_costs_nothing() {
    let cfg = Config {
        limits: LimitsConfig {
            mfa_ip: Rate::new(3, 0.0),
            mfa_user: Rate::new(2, 0.0),
            ..LimitsConfig::unlimited()
        },
        ..test_config()
    };
    let app = spawn_with(cfg).await;
    let (_, a) = register(&app, "alex").await;
    let (_, s) = register(&app, "sam").await;
    // Successes are refunded, so setup is free.
    let s_wrong = wrong_code(&setup_ok(&app, &s).await.secret);
    let a_wrong = wrong_code(&setup_ok(&app, &a).await.secret);
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/enable", &a_wrong)
            .await
            .status(),
        403
    );
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/enable", &a_wrong)
            .await
            .status(),
        403
    );
    assert_eq!(
        code_req(&app, &a, "/auth/2fa/enable", &a_wrong)
            .await
            .status(),
        429
    );
    // The refused attempt gave its address token back: sam still gets one try from it.
    assert_eq!(
        code_req(&app, &s, "/auth/2fa/enable", &s_wrong)
            .await
            .status(),
        403
    );
    assert_eq!(
        code_req(&app, &s, "/auth/2fa/enable", &s_wrong)
            .await
            .status(),
        429
    );
}

#[tokio::test]
async fn enable_with_an_unreadable_pending_secret_is_409_start_again() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let s = setup_ok(&app, &a).await;
    // As if the key changed between setup and enable: the pending secret no longer decrypts.
    sqlx::query("UPDATE user_totp SET secret_enc = ? WHERE user_id = ?")
        .bind(vec![7u8; 48])
        .bind(a_id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    let r = code_req(&app, &a, "/auth/2fa/enable", &code_at(&s.secret, 0)).await;
    assert_eq!(r.status(), 409);
    let body: serde_json::Value = r.json().await.unwrap();
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("start setup again"),
        "{body}"
    );
    assert!(!status(&app, &a).await.enabled);
    // Starting again works.
    let s = setup_ok(&app, &a).await;
    let r = code_req(&app, &a, "/auth/2fa/enable", &code_at(&s.secret, 0)).await;
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn enabling_and_signing_out_others_commit_together() {
    let app = spawn().await;
    let (_, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let s = setup_ok(&app, &a1).await;
    // Make the sign-out fail: if enabling had already committed, 2FA would be on with the
    // recovery codes never shown and the other device still signed in.
    sqlx::query(
        "CREATE TRIGGER fail_revoke BEFORE DELETE ON sessions BEGIN SELECT RAISE(ABORT, 'boom'); END",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let r = code_req(&app, &a1, "/auth/2fa/enable", &code_at(&s.secret, 0)).await;
    assert_eq!(r.status(), 500);
    assert!(!status(&app, &a1).await.enabled, "nothing committed");
    let me = app
        .http
        .get(app.url("/me"))
        .bearer_auth(&a2)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), 200);
    sqlx::query("DROP TRIGGER fail_revoke")
        .execute(&app.db)
        .await
        .unwrap();
    // The code's step was rolled back with the rest, so the next step's code enables.
    let r = code_req(&app, &a1, "/auth/2fa/enable", &code_at(&s.secret, 1)).await;
    assert_eq!(r.status(), 200);
    let on: TwoFactorEnabledResponse = r.json().await.unwrap();
    assert_eq!(on.signed_out, 1);
}

#[tokio::test]
async fn without_a_key_it_is_unavailable() {
    let app = spawn_with(Config {
        totp_key: None,
        ..test_config()
    })
    .await;
    let (_, a) = register(&app, "alex").await;
    assert_eq!(
        status(&app, &a).await,
        TwoFactorStatus {
            enabled: false,
            available: false,
            recovery_codes_left: 0
        }
    );
    assert_eq!(setup(&app, &a).await.status(), 403);
}
