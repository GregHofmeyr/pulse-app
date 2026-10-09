//! Rate limits: failed logins, bad invite codes, message sends, typing.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_server::limits::{LimitsConfig, Rate};
use tokio_tungstenite::tungstenite::Message as Ws;

async fn login(app: &TestApp, user: &str, pass: &str, ip: &str) -> u16 {
    app.http
        .post(app.url("/auth/login"))
        .header("x-forwarded-for", ip)
        .json(&serde_json::json!({ "username": user, "password": pass }))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn spawn_limited(f: impl FnOnce(&mut LimitsConfig)) -> TestApp {
    let mut cfg = test_config();
    cfg.limits = LimitsConfig::default();
    f(&mut cfg.limits);
    spawn_with(cfg).await
}

#[tokio::test]
async fn failed_logins_are_limited_per_ip_but_good_ones_are_not() {
    let app = spawn_limited(|_| {}).await;
    register(&app, "alex").await;
    // a LAN party: plenty of successful logins from one address
    for _ in 0..8 {
        assert_eq!(login(&app, "alex", "hunter2hunter2", "10.0.0.1").await, 200);
    }
    for _ in 0..5 {
        assert_eq!(login(&app, "alex", "wrong-password", "10.0.0.2").await, 401);
    }
    assert_eq!(login(&app, "alex", "wrong-password", "10.0.0.2").await, 429);
    assert_eq!(
        login(&app, "alex", "hunter2hunter2", "10.0.0.2").await,
        429,
        "even the right password waits"
    );
    assert_eq!(
        login(&app, "alex", "hunter2hunter2", "10.0.0.3").await,
        200,
        "other addresses unaffected"
    );
}

#[tokio::test]
async fn failed_logins_are_limited_per_username_across_addresses() {
    let app = spawn_limited(|_| {}).await;
    register(&app, "alex").await;
    register(&app, "sam").await;
    for i in 0..10 {
        assert_eq!(
            login(&app, "alex", "wrong-password", &format!("10.1.0.{i}")).await,
            401
        );
    }
    assert_eq!(login(&app, "alex", "wrong-password", "10.1.1.1").await, 429);
    assert_eq!(
        login(&app, "ALEX", "hunter2hunter2", "10.1.1.2").await,
        429,
        "case doesn't dodge it"
    );
    assert_eq!(
        login(&app, "sam", "hunter2hunter2", "10.1.1.1").await,
        200,
        "other accounts unaffected"
    );
}

#[tokio::test]
async fn bad_invite_codes_are_limited_per_ip_but_registrations_are_not() {
    let app = spawn_limited(|_| {}).await;
    for i in 0..6 {
        register(&app, &format!("friend{i}")).await; // all from 127.0.0.1, all fine
    }
    let attempt = |code: &'static str| {
        app.http
            .post(app.url("/auth/register"))
            .header("x-forwarded-for", "10.2.0.1")
            .json(&serde_json::json!({ "invite_code": code, "username": "mallory", "password": "hunter2hunter2" }))
            .send()
    };
    for _ in 0..5 {
        assert_eq!(attempt("nope-nope").await.unwrap().status(), 404);
    }
    assert_eq!(attempt("nope-nope").await.unwrap().status(), 429);
}

#[tokio::test]
async fn sends_are_limited_per_user() {
    let app = spawn_limited(|l| l.send_user = Rate::new(2, 0.0)).await;
    let (_, a) = register(&app, "alex").await;
    let srv = create_server(&app, &a, "Main").await;
    let ch = general(&app, &a, srv.id).await;
    let path = format!("/channels/{}/messages", ch.id);
    let send = || post_json(&app, &a, &path, serde_json::json!({ "content": "hi" }));
    assert_eq!(send().await.status(), 200);
    assert_eq!(send().await.status(), 200);
    assert_eq!(send().await.status(), 429);
}

#[tokio::test]
async fn extra_typing_frames_are_dropped() {
    let app = spawn_limited(|l| l.typing_user = Rate::new(1, 0.0)).await;
    let (b_id, b) = register(&app, "sam").await;
    let (_, a) = register(&app, "alex").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    let mut watcher = hello(&app, &b).await;
    let mut typer = hello(&app, &a).await;
    for _ in 0..3 {
        let f = serde_json::to_string(&ClientFrame::Typing { channel_id: dm.id }).unwrap();
        typer.send(Ws::text(f)).await.unwrap();
    }
    let mut typings = 0;
    while let Ok(Some(Ok(Ws::Text(t)))) =
        tokio::time::timeout(Duration::from_millis(500), watcher.next()).await
    {
        if let Ok(ServerFrame::Event(Event::Typing { .. })) = serde_json::from_str(&t) {
            typings += 1;
        }
    }
    assert_eq!(typings, 1);
}

#[tokio::test]
async fn concurrent_bad_logins_cannot_outrun_the_limit() {
    let app = spawn_limited(|_| {}).await;
    register(&app, "alex").await;
    let attempts = (0..20).map(|_| login(&app, "alex", "wrong-password", "10.3.0.1"));
    let codes = futures::future::join_all(attempts).await;
    let through = codes.iter().filter(|c| **c == 401).count();
    assert!(
        through <= 5,
        "{through} guesses got through at once: {codes:?}"
    );
    assert!(codes.iter().all(|c| *c == 401 || *c == 429), "{codes:?}");
}

#[tokio::test]
async fn oversized_usernames_never_become_limiter_keys() {
    let app = spawn_limited(|_| {}).await;
    let huge = "a".repeat(5000);
    assert_eq!(login(&app, &huge, "wrong-password", "10.4.0.1").await, 401);
    assert_eq!(
        app.limits.login_user.len(),
        0,
        "no key for a name that can't exist"
    );
    assert_eq!(
        app.limits.login_ip.len(),
        1,
        "the address still pays for the attempt"
    );
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Connect and complete the handshake (returns once Ready arrived).
async fn hello(app: &TestApp, token: &str) -> Socket {
    let mut ws = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap()
        .0;
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    while let Some(Ok(m)) = ws.next().await {
        if let Ws::Text(t) = m
            && let Ok(ServerFrame::Ready(_)) = serde_json::from_str(&t)
        {
            return ws;
        }
    }
    panic!("no Ready");
}
