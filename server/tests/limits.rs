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

/// Open `/gateway` as if from `ip`; `Err(status)` if the upgrade was refused.
async fn connect_from(app: &TestApp, ip: &str) -> Result<Socket, u16> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut req = app.ws_url("/gateway").into_client_request().unwrap();
    req.headers_mut()
        .insert("x-forwarded-for", ip.parse().unwrap());
    match tokio_tungstenite::connect_async(req).await {
        Ok((ws, _)) => Ok(ws),
        Err(tokio_tungstenite::tungstenite::Error::Http(r)) => Err(r.status().as_u16()),
        Err(e) => panic!("connect failed: {e}"),
    }
}

/// Send Hello and return the close code the server answered with (`None` if Ready came instead).
async fn hello_close_code(ws: &mut Socket, token: &str) -> Option<u16> {
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    loop {
        match tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .expect("no answer to Hello")
        {
            Some(Ok(Ws::Close(Some(f)))) => return Some(u16::from(f.code)),
            Some(Ok(Ws::Text(t))) => {
                if let Ok(ServerFrame::Ready(_)) = serde_json::from_str(&t) {
                    return None;
                }
            }
            Some(Ok(_)) => continue,
            other => panic!("expected Ready or a close, got {other:?}"),
        }
    }
}

/// Heartbeat and expect the ack: the socket is still served.
async fn alive(ws: &mut Socket) -> bool {
    let f = serde_json::to_string(&ClientFrame::Heartbeat).unwrap();
    if ws.send(Ws::text(f)).await.is_err() {
        return false;
    }
    while let Ok(Some(Ok(m))) = tokio::time::timeout(Duration::from_secs(3), ws.next()).await {
        match m {
            Ws::Text(t) => {
                if let Ok(ServerFrame::HeartbeatAck) = serde_json::from_str(&t) {
                    return true;
                }
            }
            Ws::Close(_) => return false,
            _ => {}
        }
    }
    false
}

#[tokio::test]
async fn failed_gateway_auths_are_limited_per_ip() {
    let app = spawn_limited(|l| l.gateway_ip = Rate::new(3, 0.0)).await;
    let (_, a) = register(&app, "alex").await;
    for _ in 0..3 {
        let mut ws = connect_from(&app, "10.6.0.1").await.expect("upgrade");
        assert_eq!(hello_close_code(&mut ws, "garbage").await, Some(4001));
    }
    assert_eq!(
        connect_from(&app, "10.6.0.1").await.err(),
        Some(429),
        "refused before the upgrade"
    );
    let mut other = connect_from(&app, "10.6.0.2")
        .await
        .expect("other addresses unaffected");
    assert_eq!(hello_close_code(&mut other, &a).await, None);
}

#[tokio::test]
async fn successful_gateway_auths_do_not_use_up_the_bucket() {
    let app = spawn_limited(|l| l.gateway_ip = Rate::new(2, 0.0)).await;
    let (_, a) = register(&app, "alex").await;
    for i in 0..6 {
        let mut ws = connect_from(&app, "10.7.0.1")
            .await
            .unwrap_or_else(|s| panic!("connect {i} refused with {s}"));
        assert_eq!(hello_close_code(&mut ws, &a).await, None, "connect {i}");
    }
}

#[tokio::test]
async fn ninth_gateway_connection_is_refused_and_the_first_eight_stay_up() {
    let app = spawn_limited(|_| {}).await;
    let (a_id, a) = register(&app, "alex").await;
    let mut socks = vec![];
    for _ in 0..8 {
        socks.push(hello(&app, &a).await);
    }
    let mut ninth = connect_from(&app, "127.0.0.1").await.unwrap();
    assert_eq!(hello_close_code(&mut ninth, &a).await, Some(4006));
    assert_eq!(app.hub.connections_for(a_id), 8);
    for (i, ws) in socks.iter_mut().enumerate() {
        assert!(alive(ws).await, "connection {i} was kicked");
    }
    // another user has their own allowance
    let (_, b) = register(&app, "sam").await;
    let _sam = hello(&app, &b).await;
}

#[tokio::test]
async fn a_gateway_slot_frees_when_a_connection_closes() {
    let app = spawn_limited(|_| {}).await;
    let (a_id, a) = register(&app, "alex").await;
    let mut socks = vec![];
    for _ in 0..8 {
        socks.push(hello(&app, &a).await);
    }
    let mut gone = socks.pop().unwrap();
    gone.close(None).await.unwrap();
    drop(gone);
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while app.hub.connections_for(a_id) > 7 {
        assert!(std::time::Instant::now() < deadline, "slot never freed");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let mut again = connect_from(&app, "127.0.0.1").await.unwrap();
    assert_eq!(hello_close_code(&mut again, &a).await, None);
}

/// Voice tokens and stream tickets share one per-user bucket (each records a grant row). The
/// production allowance covers a hover-prefetch sweep down a long channel list at once.
#[tokio::test]
async fn livekit_tokens_are_limited_per_user() {
    let app = spawn_limited(|_| {}).await;
    let (a_id, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let srv = create_server(&app, &a, "Main").await;
    let chans: Vec<pulse_protocol::rest::Channel> =
        get_json(&app, &a, &format!("/servers/{}/channels", srv.id)).await;
    let lounge = chans
        .iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap()
        .id;
    app.voice.join(lounge, a_id, "PA_a");
    let burst = LimitsConfig::default().livekit_token_user.burst;
    assert!(burst >= 30, "a hover sweep fits: {burst}");
    for i in 0..burst {
        let path = if i % 2 == 0 {
            format!("/voice/{lounge}/token")
        } else {
            format!("/streams/{lounge}/ticket")
        };
        let body = serde_json::json!({ "role": "v" });
        assert_eq!(post_json(&app, &a, &path, body).await.status(), 200, "#{i}");
    }
    let path = format!("/voice/{lounge}/token");
    let over = post_json(&app, &a, &path, serde_json::json!({})).await;
    assert_eq!(over.status(), 429);
    let other = create_server(&app, &b, "Theirs").await;
    let theirs: Vec<pulse_protocol::rest::Channel> =
        get_json(&app, &b, &format!("/servers/{}/channels", other.id)).await;
    let their_lounge = theirs
        .iter()
        .find(|c| c.kind == pulse_protocol::rest::ChannelKind::Voice)
        .unwrap()
        .id;
    let r = post_json(
        &app,
        &b,
        &format!("/voice/{their_lounge}/token"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(r.status(), 200, "another user has their own allowance");
}
