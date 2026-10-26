//! `/auth/login` and `/auth/login/2fa`: the ticket step for users with two-factor on.

mod common;

use std::time::{SystemTime, UNIX_EPOCH};

use common::*;
use pulse_protocol::rest::{MfaRequired, SessionResponse, TwoFactorEnabledResponse};
use pulse_server::auth::session;
use pulse_server::config::Config;
use pulse_server::limits::{LimitsConfig, Rate};
use serde_json::json;
use sha2::{Digest, Sha256};
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

/// The current 30 s TOTP step by this machine's clock (the server's clock in these tests).
fn now_step() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        / 30) as i64
}

/// A 6-digit code that cannot be valid when the server checks it. Every 6-digit string is the
/// code for *some* step, so "wrong" only means something at a given time. This one differs from
/// the codes for steps t-2..=t+2 (t = the step when it is made), and dropping it asserts the
/// clock moved at most one step meanwhile. The server checks {s-1, s, s+1} for its own step s,
/// read between the making and the drop, so s is t or t+1 and its whole window lies inside the
/// five excluded steps: it can never collide, and a call slow enough to break that bound fails
/// loudly here instead of flaking. Make one just before use rather than holding it for long.
struct WrongCode {
    code: String,
    step: i64,
}

impl std::ops::Deref for WrongCode {
    type Target = str;
    fn deref(&self) -> &str {
        &self.code
    }
}

impl Drop for WrongCode {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let moved = now_step() - self.step;
            assert!(
                (0..=1).contains(&moved),
                "a wrong code outlived its window ({moved} steps); it may have been valid"
            );
        }
    }
}

fn wrong_code(secret_b32: &str) -> WrongCode {
    let step = now_step();
    let near: Vec<String> = (-2..=2).map(|o| code_at(secret_b32, o)).collect();
    let code = (0..)
        .map(|n| format!("{n:06}"))
        .find(|c| !near.contains(c))
        .unwrap();
    WrongCode { code, step }
}

/// Turn 2FA on for the registered user; returns the secret and recovery codes. Spends step 0.
async fn turn_on(app: &TestApp, token: &str) -> (String, Vec<String>) {
    let r = post_json(app, token, "/auth/2fa/setup", json!({"password": PW})).await;
    assert_eq!(r.status(), 200);
    let s: pulse_protocol::rest::TwoFactorSetupResponse = r.json().await.unwrap();
    let r = post_json(
        app,
        token,
        "/auth/2fa/enable",
        json!({"code": code_at(&s.secret, 0)}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let on: TwoFactorEnabledResponse = r.json().await.unwrap();
    (s.secret, on.recovery_codes)
}

async fn login_raw(app: &TestApp, user: &str, password: &str) -> reqwest::Response {
    app.http
        .post(app.url("/auth/login"))
        .json(&json!({"username": user, "password": password}))
        .send()
        .await
        .unwrap()
}

/// Password step for a 2FA user: the ticket.
async fn ticket(app: &TestApp, user: &str) -> String {
    let r = login_raw(app, user, PW).await;
    assert_eq!(r.status(), 403);
    let body: MfaRequired = r.json().await.unwrap();
    assert_eq!(body.code, "mfa_required");
    body.ticket
}

async fn second_step(app: &TestApp, ticket: &str, code: &str) -> reqwest::Response {
    app.http
        .post(app.url("/auth/login/2fa"))
        .json(&json!({"ticket": ticket, "code": code}))
        .send()
        .await
        .unwrap()
}

async fn count(app: &TestApp, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// A registered alex with 2FA on (one live session from registering, which `turn_on` keeps).
async fn alex_with_2fa(app: &TestApp) -> (pulse_protocol::ids::UserId, String, Vec<String>) {
    let (id, tok) = register(app, "alex").await;
    let (secret, codes) = turn_on(app, &tok).await;
    (id, secret, codes)
}

#[tokio::test]
async fn login_without_2fa_is_unchanged() {
    let app = spawn().await;
    register(&app, "alex").await;
    let r = login_raw(&app, "alex", PW).await;
    assert_eq!(r.status(), 200);
    let s: SessionResponse = r.json().await.unwrap();
    assert_eq!(s.user.username, "alex");
    assert_eq!(count(&app, "mfa_tickets").await, 0);
}

#[tokio::test]
async fn password_step_with_2fa_gives_a_ticket_and_no_session() {
    let app = spawn().await;
    alex_with_2fa(&app).await;
    let sessions = count(&app, "sessions").await;
    let r = login_raw(&app, "alex", PW).await;
    assert_eq!(r.status(), 403);
    let body: MfaRequired = r.json().await.unwrap();
    assert_eq!(body.code, "mfa_required");
    assert_eq!(
        body.message,
        "Enter the 6-digit code from your authenticator app. If Pulse doesn't ask for one, update it."
    );
    assert_eq!(body.ticket.len(), 43); // 256 bits, base64url, no padding
    assert_eq!(count(&app, "sessions").await, sessions);
    // Only the SHA-256 is stored.
    let stored: Vec<String> = sqlx::query_scalar("SELECT ticket_hash FROM mfa_tickets")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(
        stored,
        vec![hex::encode(Sha256::digest(body.ticket.as_bytes()))]
    );
}

#[tokio::test]
async fn wrong_password_for_a_2fa_user_is_401_with_no_ticket() {
    let app = spawn().await;
    alex_with_2fa(&app).await;
    assert_eq!(
        login_raw(&app, "alex", "nope-nope-nope").await.status(),
        401
    );
    assert_eq!(count(&app, "mfa_tickets").await, 0);
}

#[tokio::test]
async fn the_right_totp_code_signs_in_once() {
    let app = spawn().await;
    let (id, secret, _) = alex_with_2fa(&app).await;
    let t = ticket(&app, "alex").await;
    let r = second_step(&app, &t, &code_at(&secret, 1)).await;
    assert_eq!(r.status(), 200);
    let s: SessionResponse = r.json().await.unwrap();
    assert_eq!(s.user.id, id);
    assert_eq!(s.user.username, "alex");
    let me = app
        .http
        .get(app.url("/me"))
        .bearer_auth(&s.token)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), 200);
    assert_eq!(
        session::authenticate(&app.db, &s.token).await.unwrap(),
        Some(id)
    );
    // Tickets are single-use.
    assert_eq!(count(&app, "mfa_tickets").await, 0);
    assert_eq!(
        second_step(&app, &t, &code_at(&secret, 1)).await.status(),
        410
    );
}

#[tokio::test]
async fn a_spent_totp_code_is_refused_on_a_fresh_ticket() {
    let app = spawn().await;
    let (_, secret, _) = alex_with_2fa(&app).await;
    let code = code_at(&secret, 1);
    let t1 = ticket(&app, "alex").await;
    assert_eq!(second_step(&app, &t1, &code).await.status(), 200);
    let t2 = ticket(&app, "alex").await;
    assert_eq!(second_step(&app, &t2, &code).await.status(), 401);
    // The code the enrolment used is spent too.
    assert_eq!(
        second_step(&app, &t2, &code_at(&secret, 0)).await.status(),
        401
    );
}

#[tokio::test]
async fn a_recovery_code_signs_in_once() {
    let app = spawn().await;
    let (_, _, codes) = alex_with_2fa(&app).await;
    let t1 = ticket(&app, "alex").await;
    // Typed loosely: upper case, spaces for dashes.
    let loose = codes[0].to_uppercase().replace('-', " ");
    assert_eq!(second_step(&app, &t1, &loose).await.status(), 200);
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recovery_codes")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(left, 9);
    let t2 = ticket(&app, "alex").await;
    assert_eq!(second_step(&app, &t2, &codes[0]).await.status(), 401);
    assert_eq!(second_step(&app, &t2, &codes[1]).await.status(), 200);
}

#[tokio::test]
async fn unknown_and_expired_tickets_are_410() {
    let app = spawn().await;
    let (_, secret, _) = alex_with_2fa(&app).await;
    assert_eq!(
        second_step(&app, "not-a-ticket", &code_at(&secret, 1))
            .await
            .status(),
        410
    );
    let t = ticket(&app, "alex").await;
    sqlx::query("UPDATE mfa_tickets SET expires_at = '2000-01-01T00:00:00.000Z'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        second_step(&app, &t, &code_at(&secret, 1)).await.status(),
        410
    );
    // The next password step prunes the dead ticket.
    ticket(&app, "alex").await;
    assert_eq!(count(&app, "mfa_tickets").await, 1);
}

#[tokio::test]
async fn five_wrong_codes_burn_the_ticket() {
    let app = spawn().await;
    let (_, secret, _) = alex_with_2fa(&app).await;
    let t = ticket(&app, "alex").await;
    for _ in 0..5 {
        assert_eq!(
            second_step(&app, &t, &wrong_code(&secret)).await.status(),
            401
        );
    }
    assert_eq!(count(&app, "mfa_tickets").await, 0);
    assert_eq!(
        second_step(&app, &t, &code_at(&secret, 1)).await.status(),
        410
    );
}

#[tokio::test]
async fn a_wrong_code_does_not_spend_the_ticket_or_the_code() {
    let app = spawn().await;
    let (_, secret, _) = alex_with_2fa(&app).await;
    let t = ticket(&app, "alex").await;
    assert_eq!(
        second_step(&app, &t, &wrong_code(&secret)).await.status(),
        401
    );
    assert_eq!(second_step(&app, &t, "garbage").await.status(), 401);
    assert_eq!(
        second_step(&app, &t, &code_at(&secret, 1)).await.status(),
        200
    );
}

#[tokio::test]
async fn racing_requests_on_one_ticket_have_one_winner() {
    let app = spawn().await;
    let (_, secret, _) = alex_with_2fa(&app).await;
    let t = ticket(&app, "alex").await;
    let code = code_at(&secret, 1);
    let rs = futures::future::join_all((0..4).map(|_| second_step(&app, &t, &code))).await;
    let statuses: Vec<_> = rs.iter().map(|r| r.status().as_u16()).collect();
    assert_eq!(
        statuses.iter().filter(|s| **s == 200).count(),
        1,
        "{statuses:?}"
    );
    assert!(
        statuses.iter().all(|s| [200, 401, 410].contains(s)),
        "{statuses:?}"
    );
}

#[tokio::test]
async fn a_ticket_only_works_for_its_own_user() {
    let app = spawn().await;
    let (_, secret, _) = alex_with_2fa(&app).await;
    let (_, sam) = register(&app, "sam").await;
    let (sam_secret, _) = turn_on(&app, &sam).await;
    let t = ticket(&app, "alex").await;
    // Sam's current code against alex's ticket: wrong, unless it happens to equal one of alex's
    // codes near now (as in `WrongCode`, steps t-2..=t+2 cover the server's window a step later).
    let theirs = code_at(&sam_secret, 1);
    if (-2..=2).all(|o| theirs != code_at(&secret, o)) {
        assert_eq!(second_step(&app, &t, &theirs).await.status(), 401);
    }
}

fn limited_app(ip: Rate, user: Rate) -> Config {
    Config {
        limits: LimitsConfig {
            mfa_ip: ip,
            mfa_user: user,
            ..LimitsConfig::unlimited()
        },
        ..test_config()
    }
}

#[tokio::test]
async fn the_user_limit_stops_guessing_even_with_the_right_code() {
    let app = spawn_with(limited_app(Rate::new(100, 0.0), Rate::new(2, 0.0))).await;
    let (_, secret, _) = alex_with_2fa(&app).await;
    let t = ticket(&app, "alex").await;
    assert_eq!(
        second_step(&app, &t, &wrong_code(&secret)).await.status(),
        401
    );
    assert_eq!(
        second_step(&app, &t, &wrong_code(&secret)).await.status(),
        401
    );
    assert_eq!(
        second_step(&app, &t, &code_at(&secret, 1)).await.status(),
        429
    );
}

#[tokio::test]
async fn the_ip_limit_counts_unknown_tickets_and_success_refunds() {
    let app = spawn_with(limited_app(Rate::new(2, 0.0), Rate::new(100, 0.0))).await;
    let (_, _, codes) = alex_with_2fa(&app).await;
    // Successes cost nothing: three in a row on a two-token bucket.
    for c in &codes[..3] {
        let t = ticket(&app, "alex").await;
        assert_eq!(second_step(&app, &t, c).await.status(), 200);
    }
    // Guessing tickets does cost, and spends no user bucket.
    assert_eq!(second_step(&app, "guess-1", "123456").await.status(), 410);
    assert_eq!(second_step(&app, "guess-2", "123456").await.status(), 410);
    assert_eq!(second_step(&app, "guess-3", "123456").await.status(), 429);
    let t = ticket(&app, "alex").await;
    assert_eq!(second_step(&app, &t, &codes[3]).await.status(), 429);
}

#[tokio::test]
async fn the_password_step_refunds_its_login_limits() {
    let app = spawn_with(Config {
        limits: LimitsConfig {
            login_ip: Rate::new(1, 0.0),
            login_user: Rate::new(1, 0.0),
            ..LimitsConfig::unlimited()
        },
        ..test_config()
    })
    .await;
    alex_with_2fa(&app).await;
    // Not a failed password, so it costs nothing: more tickets than the bucket holds.
    for _ in 0..3 {
        ticket(&app, "alex").await;
    }
    assert_eq!(
        login_raw(&app, "alex", "nope-nope-nope").await.status(),
        401
    );
    assert_eq!(
        login_raw(&app, "alex", "nope-nope-nope").await.status(),
        429
    );
}

#[tokio::test]
async fn concurrent_wrong_codes_cannot_outrun_the_failure_limit() {
    let app = spawn().await;
    let (_, secret, _) = alex_with_2fa(&app).await;
    let t = ticket(&app, "alex").await;
    for _ in 0..4 {
        assert_eq!(
            second_step(&app, &t, &wrong_code(&secret)).await.status(),
            401
        );
    }
    // One code for the concurrent burst; its drop after the burst checks the bound.
    let wrong = wrong_code(&secret);
    let rs = futures::future::join_all((0..6).map(|_| second_step(&app, &t, &wrong))).await;
    let statuses: Vec<_> = rs.iter().map(|r| r.status().as_u16()).collect();
    assert_eq!(
        statuses.iter().filter(|s| **s == 401).count(),
        1,
        "{statuses:?}"
    );
    assert!(
        statuses.iter().all(|s| [401, 410].contains(s)),
        "{statuses:?}"
    );
    assert_eq!(count(&app, "mfa_tickets").await, 0);
    assert_eq!(
        second_step(&app, &t, &code_at(&secret, 1)).await.status(),
        410
    );
}

#[tokio::test]
async fn unknown_tickets_spend_no_user_bucket() {
    let app = spawn_with(limited_app(Rate::new(100, 0.0), Rate::new(1, 0.0))).await;
    let (_, secret, _) = alex_with_2fa(&app).await;
    for i in 0..4 {
        assert_eq!(
            second_step(&app, &format!("guess-{i}"), "123456")
                .await
                .status(),
            410
        );
    }
    let t = ticket(&app, "alex").await;
    assert_eq!(
        second_step(&app, &t, &code_at(&secret, 1)).await.status(),
        200
    );
}

#[tokio::test]
async fn the_session_step_rechecks_2fa_so_enabling_it_mid_login_wins() {
    let app = spawn().await;
    // alex's password was checked while 2FA was still off; then enable commits before the
    // login issues its session. The session step must see 2FA on and give no session.
    let (id, _, _) = alex_with_2fa(&app).await;
    let sessions = count(&app, "sessions").await;
    let out = session::create_unless_2fa(&app.db, id).await.unwrap();
    assert!(
        matches!(out, session::PasswordLogin::TwoFactor(_)),
        "a session was issued past 2FA: {out:?}"
    );
    assert_eq!(
        count(&app, "sessions").await,
        sessions,
        "no session row added"
    );
    assert_eq!(
        count(&app, "mfa_tickets").await,
        1,
        "a login ticket instead"
    );
}

#[tokio::test]
async fn the_session_step_without_2fa_issues_a_session() {
    let app = spawn().await;
    let (id, _) = register(&app, "alex").await;
    let out = session::create_unless_2fa(&app.db, id).await.unwrap();
    let session::PasswordLogin::Session(token) = out else {
        panic!("expected a session: {out:?}");
    };
    assert_eq!(
        session::authenticate(&app.db, &token).await.unwrap(),
        Some(id)
    );
    assert_eq!(count(&app, "mfa_tickets").await, 0);
}
