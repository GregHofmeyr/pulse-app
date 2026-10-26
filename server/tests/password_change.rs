//! `POST /auth/password`, and the owner's `reset-password` / `revoke-sessions`.

mod common;

use std::io::Write;
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::ClientFrame;
use pulse_protocol::rest::{TwoFactorEnabledResponse, TwoFactorSetupResponse};
use pulse_server::auth::{routes, session};
use pulse_server::config::Config;
use pulse_server::limits::{LimitsConfig, Rate};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use totp_rs::{Algorithm, Builder, Secret};

const PW: &str = "hunter2hunter2";
const NEW_PW: &str = "correct horse battery";

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello(app: &TestApp, token: &str) -> Socket {
    let mut ws = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap()
        .0;
    ws.send(Ws::text(
        serde_json::to_string(&ClientFrame::Hello {
            token: token.into(),
            client_version: pulse_protocol::PROTOCOL_VERSION,
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    loop {
        if let Some(Ok(Ws::Text(t))) = ws.next().await
            && t.contains("\"Ready\"")
        {
            return ws;
        }
    }
}

/// Read until a close frame (or timeout); returns its code.
async fn close_code(ws: &mut Socket, within: Duration) -> Option<CloseCode> {
    tokio::time::timeout(within, async {
        while let Some(m) = ws.next().await {
            match m {
                Ok(Ws::Close(f)) => return f.map(|f| f.code),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

async fn login_status(app: &TestApp, username: &str, password: &str) -> reqwest::Response {
    app.http
        .post(app.url("/auth/login"))
        .json(&json!({"username": username, "password": password}))
        .send()
        .await
        .unwrap()
}

async fn login(app: &TestApp, username: &str) -> String {
    let r = login_status(app, username, PW).await;
    assert_eq!(r.status(), 200);
    r.json::<pulse_protocol::rest::SessionResponse>()
        .await
        .unwrap()
        .token
}

async fn me(app: &TestApp, token: &str) -> reqwest::StatusCode {
    app.http
        .get(app.url("/me"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
}

async fn change(app: &TestApp, token: &str, body: serde_json::Value) -> reqwest::Response {
    post_json(app, token, "/auth/password", body).await
}

/// The code for the step `offset` steps from now (0 and +1 are always inside the server's window).
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

/// Turn 2FA on for the caller; returns the secret.
async fn turn_on(app: &TestApp, token: &str) -> String {
    let r = post_json(app, token, "/auth/2fa/setup", json!({"password": PW})).await;
    assert_eq!(r.status(), 200);
    let s: TwoFactorSetupResponse = r.json().await.unwrap();
    let r = post_json(
        app,
        token,
        "/auth/2fa/enable",
        json!({"code": code_at(&s.secret, 0)}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let _: TwoFactorEnabledResponse = r.json().await.unwrap();
    s.secret
}

#[tokio::test]
async fn changing_the_password_signs_out_the_other_devices_only() {
    let app = spawn().await;
    let (a_id, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let (s_id, s) = register(&app, "sam").await;
    let mut ws_caller = hello(&app, &a1).await;
    let mut ws_other = hello(&app, &a2).await;
    let mut ws_sam = hello(&app, &s).await;

    let r = change(
        &app,
        &a1,
        json!({"current_password": PW, "new_password": NEW_PW}),
    )
    .await;
    assert_eq!(r.status(), 204);

    // The old password is dead, the new one works.
    assert_eq!(login_status(&app, "alex", PW).await.status(), 401);
    assert_eq!(login_status(&app, "alex", NEW_PW).await.status(), 200);
    // The other device is out, on REST and on its socket; this one and sam stay.
    assert_eq!(me(&app, &a2).await, 401);
    assert_eq!(
        close_code(&mut ws_other, Duration::from_secs(2)).await,
        Some(CloseCode::Library(4001))
    );
    assert_eq!(me(&app, &a1).await, 200);
    assert_eq!(
        close_code(&mut ws_caller, Duration::from_millis(300)).await,
        None
    );
    assert_eq!(
        close_code(&mut ws_sam, Duration::from_millis(100)).await,
        None
    );
    assert_eq!(app.hub.connections_for(a_id), 1);
    assert_eq!(app.hub.connections_for(s_id), 1);
    assert_eq!(me(&app, &s).await, 200);
}

#[tokio::test]
async fn a_wrong_current_password_is_403_and_changes_nothing() {
    let app = spawn().await;
    let (_, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let r = change(
        &app,
        &a1,
        json!({"current_password": "nopenopenope", "new_password": NEW_PW}),
    )
    .await;
    assert_eq!(r.status(), 403);
    assert_eq!(login_status(&app, "alex", PW).await.status(), 200);
    assert_eq!(login_status(&app, "alex", NEW_PW).await.status(), 401);
    assert_eq!(me(&app, &a2).await, 200, "nothing was signed out");
}

#[tokio::test]
async fn a_new_password_outside_8_to_256_bytes_is_400() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    for bad in ["short".to_string(), "x".repeat(257)] {
        let r = change(
            &app,
            &a,
            json!({"current_password": PW, "new_password": bad}),
        )
        .await;
        assert_eq!(r.status(), 400, "{}", bad.len());
    }
    assert_eq!(login_status(&app, "alex", PW).await.status(), 200);
    // The edges are fine.
    let r = change(
        &app,
        &a,
        json!({"current_password": PW, "new_password": "x".repeat(256)}),
    )
    .await;
    assert_eq!(r.status(), 204);
}

#[tokio::test]
async fn with_two_factor_on_a_code_is_needed_and_pending_tickets_die() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let secret = turn_on(&app, &a).await;
    // Someone with the password is half-way through signing in elsewhere.
    let r = login_status(&app, "alex", PW).await;
    assert_eq!(r.status(), 403);
    let ticket = r.json::<serde_json::Value>().await.unwrap()["ticket"]
        .as_str()
        .unwrap()
        .to_string();

    // Missing or wrong code: the same answer the 2FA routes give for a wrong code.
    for body in [
        json!({"current_password": PW, "new_password": NEW_PW}),
        json!({"current_password": PW, "new_password": NEW_PW, "code": "not-a-code"}),
    ] {
        let r = change(&app, &a, body).await;
        assert_eq!(r.status(), 403);
        let e: serde_json::Value = r.json().await.unwrap();
        assert_eq!(e["message"], "wrong code", "{e}");
    }
    assert_eq!(login_status(&app, "alex", NEW_PW).await.status(), 401);

    let r = change(
        &app,
        &a,
        json!({"current_password": PW, "new_password": NEW_PW, "code": code_at(&secret, 1)}),
    )
    .await;
    assert_eq!(r.status(), 204);
    let tickets: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mfa_tickets WHERE user_id = ?")
        .bind(a_id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(tickets, 0);
    // The old ticket is gone (410), whatever the code.
    let r = app
        .http
        .post(app.url("/auth/login/2fa"))
        .json(&json!({"ticket": ticket, "code": code_at(&secret, 1)}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 410);
    assert_eq!(me(&app, &a).await, 200);
}

#[tokio::test]
async fn wrong_current_passwords_keep_the_two_factor_spend() {
    let cfg = Config {
        limits: LimitsConfig {
            mfa_ip: Rate::new(10, 0.0),
            mfa_user: Rate::new(2, 0.0),
            ..LimitsConfig::unlimited()
        },
        ..test_config()
    };
    let app = spawn_with(cfg).await;
    let (_, a) = register(&app, "alex").await;
    let wrong = json!({"current_password": "nopenopenope", "new_password": NEW_PW});
    // A bad new password is refunded.
    let short = json!({"current_password": PW, "new_password": "short"});
    for _ in 0..3 {
        assert_eq!(change(&app, &a, short.clone()).await.status(), 400);
    }
    assert_eq!(change(&app, &a, wrong.clone()).await.status(), 403);
    assert_eq!(change(&app, &a, wrong.clone()).await.status(), 403);
    assert_eq!(change(&app, &a, wrong).await.status(), 429);
    let right = json!({"current_password": PW, "new_password": NEW_PW});
    assert_eq!(change(&app, &a, right).await.status(), 429);
}

#[tokio::test]
async fn needs_a_session() {
    let app = spawn().await;
    register(&app, "alex").await;
    let r = change(
        &app,
        "garbage",
        json!({"current_password": PW, "new_password": NEW_PW}),
    )
    .await;
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn reset_password_sets_it_and_revokes_every_session() {
    let app = spawn().await;
    let (_, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let (_, s) = register(&app, "sam").await;
    let (name, revoked) = routes::reset_password(&app.db, " Alex ", NEW_PW)
        .await
        .unwrap();
    assert_eq!((name.as_str(), revoked), ("alex", 2));
    assert_eq!(me(&app, &a1).await, 401);
    assert_eq!(me(&app, &a2).await, 401);
    assert_eq!(me(&app, &s).await, 200, "other users are untouched");
    assert_eq!(login_status(&app, "alex", PW).await.status(), 401);
    assert_eq!(login_status(&app, "alex", NEW_PW).await.status(), 200);
    assert_eq!(login_status(&app, "sam", PW).await.status(), 200);

    // The register rule applies; an unknown user is an error, not a silent no-op.
    for bad in ["short".to_string(), "x".repeat(257)] {
        let e = routes::reset_password(&app.db, "alex", &bad)
            .await
            .unwrap_err();
        assert!(format!("{e:#}").contains("password must be"), "{e:#}");
    }
    let e = routes::reset_password(&app.db, "nobody", NEW_PW)
        .await
        .unwrap_err();
    assert!(format!("{e:#}").contains("nobody"), "{e:#}");
}

#[tokio::test]
async fn revoke_sessions_signs_out_every_device_and_keeps_the_password() {
    let app = spawn().await;
    let (_, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let (_, s) = register(&app, "sam").await;
    let (name, revoked) = routes::revoke_sessions(&app.db, "ALEX").await.unwrap();
    assert_eq!((name.as_str(), revoked), ("alex", 2));
    assert_eq!(me(&app, &a1).await, 401);
    assert_eq!(me(&app, &a2).await, 401);
    assert_eq!(me(&app, &s).await, 200);
    assert_eq!(login_status(&app, "alex", PW).await.status(), 200);
    let e = routes::revoke_sessions(&app.db, "nobody")
        .await
        .unwrap_err();
    assert!(format!("{e:#}").contains("nobody"), "{e:#}");
}

#[test]
fn the_new_password_is_one_line_of_stdin() {
    let read = |s: &str| routes::read_password_line(s.as_bytes());
    assert_eq!(read("pass word 1\n").unwrap(), "pass word 1");
    assert_eq!(read("pass word 1\r\n").unwrap(), "pass word 1");
    assert_eq!(read("pass word 1").unwrap(), "pass word 1");
    assert_eq!(read("first line\nsecond\n").unwrap(), "first line");
    assert!(read("").is_err());
}

/// The binary reads the password from stdin (never argv) and says what it did.
#[tokio::test]
async fn binary_reset_password_and_revoke_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("pulse.db").display());
    let db = pulse_server::db::connect(&url).await.unwrap();
    let alex = pulse_protocol::ids::UserId::new();
    sqlx::query("INSERT INTO users (id, username, password_hash, created_at) VALUES (?, ?, ?, ?)")
        .bind(alex.to_string())
        .bind("alex")
        .bind(pulse_server::auth::password::hash(PW).unwrap())
        .bind(pulse_server::db::now())
        .execute(&db)
        .await
        .unwrap();
    let t = session::create(&db, alex).await.unwrap();

    let mut child = binary(&url, "127.0.0.1:1".parse().unwrap())
        .args(["reset-password", "alex"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{NEW_PW}").unwrap();
    let out = child.wait_with_output().unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("password reset for alex"), "{text}");
    assert!(!text.contains(NEW_PW), "{text}");
    assert_eq!(session::authenticate(&db, &t).await.unwrap(), None);
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(alex.to_string())
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(pulse_server::auth::password::verify(NEW_PW, &hash));

    let t = session::create(&db, alex).await.unwrap();
    let (ok, out) = run_binary(
        &url,
        &[],
        &["revoke-sessions", "alex"],
        Duration::from_secs(20),
    );
    assert!(ok, "{out}");
    assert!(out.contains("signed out 1 session of alex"), "{out}");
    assert_eq!(session::authenticate(&db, &t).await.unwrap(), None);
}
