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
    let app = spawn_limited(|l| l.login_user = Rate::new(10, 0.0)).await;
    register(&app, "alex").await;
    register(&app, "sam").await;
    for i in 0..10 {
        assert_eq!(
            login(&app, "alex", "wrong-password", &format!("203.0.113.{i}")).await,
            401
        );
    }
    // The username's bucket is empty: addresses that have failed for it are refused.
    for i in 0..10 {
        assert_eq!(
            login(&app, "alex", "wrong-password", &format!("203.0.113.{i}")).await,
            429
        );
    }
    assert_eq!(
        login(&app, "ALEX", "hunter2hunter2", "203.0.113.3").await,
        429,
        "case doesn't dodge it"
    );
    // A fresh address gets one try, and once it has failed it is refused too.
    assert_eq!(
        login(&app, "alex", "wrong-password", "198.51.100.1").await,
        401
    );
    assert_eq!(
        login(&app, "alex", "wrong-password", "198.51.100.1").await,
        429
    );
    assert_eq!(
        login(&app, "sam", "hunter2hunter2", "203.0.113.1").await,
        200,
        "other accounts unaffected"
    );
}

#[tokio::test]
async fn failed_logins_elsewhere_cannot_lock_the_owner_out() {
    let app = spawn_limited(|l| {
        l.login_user = Rate::new(4, 0.0);
        l.login_user_ip = Rate::new(2, 0.0);
    })
    .await;
    register(&app, "alex").await;
    // An attacker drains alex's username bucket from two addresses.
    for ip in ["203.0.113.1", "203.0.113.2"] {
        for _ in 0..2 {
            assert_eq!(login(&app, "alex", "wrong-password", ip).await, 401);
        }
        assert_eq!(
            login(&app, "alex", "wrong-password", ip).await,
            429,
            "this address has used its tries for alex"
        );
    }
    assert_eq!(
        login(&app, "alex", "hunter2hunter2", "203.0.113.1").await,
        429,
        "the attacking address stays refused, even with the right password"
    );
    // The owner, from an address with no failures for alex, still gets in.
    assert_eq!(
        login(&app, "alex", "hunter2hunter2", "198.51.100.4").await,
        200
    );
    assert_eq!(
        login(&app, "alex", "hunter2hunter2", "198.51.100.4").await,
        200
    );
    // A wrong password from it while the username bucket is empty blocks it too.
    assert_eq!(
        login(&app, "alex", "wrong-password", "198.51.100.4").await,
        401
    );
    assert_eq!(
        login(&app, "alex", "hunter2hunter2", "198.51.100.4").await,
        429
    );
}

#[tokio::test]
async fn a_refused_login_spends_nothing() {
    let app = spawn_limited(|l| {
        l.login_user = Rate::new(1, 0.0);
        l.login_user_ip = Rate::new(2, 0.0);
    })
    .await;
    register(&app, "alex").await;
    assert_eq!(
        login(&app, "alex", "wrong-password", "203.0.113.1").await,
        401
    );
    // Username bucket empty, this address not full: refused, many times over...
    for _ in 0..5 {
        assert_eq!(
            login(&app, "alex", "wrong-password", "203.0.113.1").await,
            429
        );
    }
    // ...and none of those refusals spent the address's last (username, IP) token or the
    // per-IP one, so once the username bucket has a token again this address can try.
    app.limits.login_user.refund("alex");
    assert_eq!(
        login(&app, "alex", "hunter2hunter2", "203.0.113.1").await,
        200
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
        app.limits.login_user_ip.len(),
        0,
        "nor a (name, address) key"
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

/// Everything unlimited except what `f` sets (so one limit is tested at a time).
async fn spawn_only(f: impl FnOnce(&mut LimitsConfig)) -> TestApp {
    let mut cfg = test_config();
    cfg.limits = LimitsConfig::unlimited();
    f(&mut cfg.limits);
    spawn_with(cfg).await
}

/// A small bucket that doesn't refill during the test.
fn tiny(burst: u32) -> Rate {
    Rate::new(burst, 0.0)
}

async fn status(r: reqwest::RequestBuilder, token: &str) -> u16 {
    r.bearer_auth(token).send().await.unwrap().status().as_u16()
}

#[tokio::test]
async fn creating_servers_is_limited_per_user() {
    let app = spawn_only(|l| l.create_user = tiny(2)).await;
    let (_, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let body = || serde_json::json!({ "name": "Main" });
    assert_eq!(post_json(&app, &a, "/servers", body()).await.status(), 200);
    assert_eq!(post_json(&app, &a, "/servers", body()).await.status(), 200);
    assert_eq!(post_json(&app, &a, "/servers", body()).await.status(), 429);
    assert_eq!(
        post_json(&app, &b, "/servers", body()).await.status(),
        200,
        "another user has their own allowance"
    );
}

#[tokio::test]
async fn creating_channels_shares_the_create_bucket() {
    let app = spawn_only(|l| l.create_user = tiny(3)).await;
    let (_, a) = register(&app, "alex").await;
    let srv = create_server(&app, &a, "Main").await; // 1
    let path = format!("/servers/{}/channels", srv.id);
    let body = || serde_json::json!({ "name": "more", "kind": "text" });
    assert_eq!(post_json(&app, &a, &path, body()).await.status(), 200); // 2
    assert_eq!(post_json(&app, &a, &path, body()).await.status(), 200); // 3
    assert_eq!(post_json(&app, &a, &path, body()).await.status(), 429);
}

#[tokio::test]
async fn new_dms_and_groups_are_limited_but_reopening_a_dm_is_not() {
    let app = spawn_only(|l| l.create_user = tiny(2)).await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let (c_id, _) = register(&app, "riley").await;
    let dm = |ids: Vec<pulse_protocol::ids::UserId>| {
        post_json(&app, &a, "/dms", serde_json::json!({ "user_ids": ids }))
    };
    assert_eq!(dm(vec![b_id]).await.status(), 200, "new 1:1 DM");
    assert_eq!(dm(vec![b_id, c_id]).await.status(), 200, "new group");
    assert_eq!(dm(vec![b_id, c_id]).await.status(), 429, "another group");
    assert_eq!(
        dm(vec![b_id]).await.status(),
        200,
        "the existing DM is free"
    );
    assert_eq!(dm(vec![c_id]).await.status(), 429, "a new 1:1 DM is not");
}

#[tokio::test]
async fn adding_group_members_is_limited_per_user() {
    let app = spawn_only(|l| l.create_user = tiny(2)).await;
    let (a_id, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let (c_id, _) = register(&app, "riley").await;
    let g = create_dm(&app, &b, &[a_id, c_id]).await; // sam's allowance, not alex's
    let members = format!("/channels/{}/members", g.id);
    let kick = format!("/channels/{}/members/{c_id}", g.id);
    let add = || {
        post_json(
            &app,
            &a,
            &members,
            serde_json::json!({ "user_ids": [c_id] }),
        )
    };
    for i in 0..2 {
        assert_eq!(
            status(app.http.delete(app.url(&kick)), &a).await,
            204,
            "#{i}"
        );
        assert_eq!(add().await.status(), 204, "#{i}");
    }
    assert_eq!(status(app.http.delete(app.url(&kick)), &a).await, 204);
    assert_eq!(add().await.status(), 429);
}

#[tokio::test]
async fn invites_are_limited_per_user() {
    let app = spawn_only(|l| l.invite_user = tiny(2)).await;
    let (_, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let mint = || app.http.post(app.url("/invites"));
    assert_eq!(status(mint(), &a).await, 200);
    assert_eq!(status(mint(), &a).await, 200);
    assert_eq!(status(mint(), &a).await, 429);
    assert_eq!(
        status(mint(), &b).await,
        200,
        "another user has their own allowance"
    );
}

fn png() -> Vec<u8> {
    use image::{ImageFormat, Rgba, RgbaImage};
    let img = RgbaImage::from_pixel(64, 32, Rgba([10, 120, 240, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

async fn upload_emoji(app: &TestApp, token: &str) -> String {
    let r = app
        .http
        .post(app.url("/media/emoji"))
        .bearer_auth(token)
        .body(png())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json::<pulse_protocol::rest::MediaInfo>()
        .await
        .unwrap()
        .hash
}

#[tokio::test]
async fn emoji_create_rename_and_delete_share_a_limit() {
    let app = spawn_only(|l| l.emoji_user = tiny(2)).await;
    let (_, a) = register(&app, "alex").await;
    let srv = create_server(&app, &a, "Main").await;
    let hash = upload_emoji(&app, &a).await;
    let path = format!("/servers/{}/emoji", srv.id);
    let create = |name: &str| {
        post_json(
            &app,
            &a,
            &path,
            serde_json::json!({ "name": name, "media_hash": hash }),
        )
    };
    let r = create("pog").await;
    assert_eq!(r.status(), 200);
    let e: pulse_protocol::rest::CustomEmoji = r.json().await.unwrap();
    let rename = app
        .http
        .patch(app.url(&format!("/emoji/{}", e.id)))
        .json(&serde_json::json!({ "name": "poggers" }));
    assert_eq!(status(rename, &a).await, 200);
    assert_eq!(create("kek").await.status(), 429);
    let del = app.http.delete(app.url(&format!("/emoji/{}", e.id)));
    assert_eq!(status(del, &a).await, 429);
}

#[tokio::test]
async fn edits_deletes_and_reactions_share_a_limit() {
    let app = spawn_only(|l| l.mutate_user = tiny(3)).await;
    let (_, a) = register(&app, "alex").await;
    let srv = create_server(&app, &a, "Main").await;
    let ch = general(&app, &a, srv.id).await;
    let msg = send(&app, &a, ch.id, "hello").await;
    let edit = app
        .http
        .patch(app.url(&format!("/messages/{}", msg.id)))
        .json(&serde_json::json!({ "content": "hello!" }));
    assert_eq!(status(edit, &a).await, 200);
    let reaction = app.url(&format!("/messages/{}/reactions/👍", msg.id));
    assert_eq!(status(app.http.put(&reaction), &a).await, 204);
    assert_eq!(status(app.http.delete(&reaction), &a).await, 204);
    assert_eq!(status(app.http.put(&reaction), &a).await, 429);
    let del = app.http.delete(app.url(&format!("/messages/{}", msg.id)));
    assert_eq!(status(del, &a).await, 429);
    let edit = app
        .http
        .patch(app.url(&format!("/messages/{}", msg.id)))
        .json(&serde_json::json!({ "content": "again" }));
    assert_eq!(status(edit, &a).await, 429);
}

#[tokio::test]
async fn going_live_and_watching_are_limited_but_stop_and_unwatch_are_not() {
    let app = spawn_only(|l| l.mutate_user = tiny(3)).await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let srv = create_server(&app, &a, "Main").await;
    let joined = post_json(
        &app,
        &b,
        &format!("/servers/{}/join", srv.id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(joined.status(), 204);
    let chans: Vec<pulse_protocol::rest::Channel> =
        get_json(&app, &a, &format!("/servers/{}/channels", srv.id)).await;
    let lounge = chans
        .iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap()
        .id;
    app.voice.join(lounge, a_id, "PA_a");
    app.voice.join(lounge, b_id, "PA_b");
    let live = format!("/streams/{lounge}/live");
    let settings = || serde_json::json!({"settings": {"height": 1080, "fps": 60, "audio": true}});
    for _ in 0..3 {
        assert_eq!(post_json(&app, &a, &live, settings()).await.status(), 200);
    }
    assert_eq!(post_json(&app, &a, &live, settings()).await.status(), 429);
    // sam's watch draws on sam's own bucket
    let watch = format!("/streams/{lounge}/watch");
    let unwatch = format!("/streams/{lounge}/unwatch");
    let who = || serde_json::json!({ "streamer": a_id });
    for _ in 0..3 {
        assert_eq!(post_json(&app, &b, &watch, who()).await.status(), 204);
    }
    assert_eq!(post_json(&app, &b, &watch, who()).await.status(), 429);
    // Teardown never spends or waits: both buckets are empty, yet these still go through.
    for _ in 0..3 {
        assert_eq!(post_json(&app, &b, &unwatch, who()).await.status(), 204);
    }
    assert_eq!(status(app.http.delete(app.url(&live)), &a).await, 204);
}

/// Count the `VoiceStateChanged` events `ws` receives until it has been quiet for 500 ms.
async fn count_voice_states(ws: &mut Socket) -> usize {
    let mut n = 0;
    while let Ok(Some(Ok(m))) = tokio::time::timeout(Duration::from_millis(500), ws.next()).await {
        if let Ws::Text(t) = m
            && let Ok(ServerFrame::Event(Event::VoiceStateChanged { .. })) =
                serde_json::from_str(&t)
        {
            n += 1;
        }
    }
    n
}

fn voice_frame(muted: bool) -> Ws {
    use pulse_protocol::gateway::VoiceFlags;
    let flags = VoiceFlags {
        muted,
        deafened: false,
    };
    Ws::text(serde_json::to_string(&ClientFrame::VoiceState { flags }).unwrap())
}

#[tokio::test]
async fn extra_voice_state_frames_are_dropped() {
    // The default burst with no refill, so the bound is exact however slow the machine is.
    let rate = Rate::new(LimitsConfig::default().voice_state_user.burst, 0.0);
    let app = spawn_only(|l| l.voice_state_user = rate).await;
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
    let mut watcher = hello(&app, &b).await;
    let mut talker = hello(&app, &a).await;
    for i in 0..50 {
        talker.send(voice_frame(i % 2 == 0)).await.unwrap();
    }
    let seen = count_voice_states(&mut watcher).await;
    assert_eq!(
        seen, rate.burst as usize,
        "the burst gets through, the rest is dropped: {seen} VoiceStateChanged for 50 frames"
    );
    assert!(alive(&mut talker).await, "the sender is not disconnected");
    assert!(alive(&mut watcher).await, "the watcher is not disconnected");
}

#[tokio::test]
async fn unchanged_voice_flags_publish_nothing() {
    let app = spawn_only(|_| {}).await;
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
    let mut watcher = hello(&app, &b).await;
    let mut talker = hello(&app, &a).await;
    talker.send(voice_frame(false)).await.unwrap(); // same as the join default
    for _ in 0..5 {
        talker.send(voice_frame(true)).await.unwrap();
    }
    assert_eq!(count_voice_states(&mut watcher).await, 1, "only the change");
}

/// Frames over the limit still count: the flags are stored, and one trailing publish carries the
/// latest ones once the bucket allows, so peers never keep a stale mute icon.
#[tokio::test]
async fn limited_voice_state_frames_still_land_in_a_trailing_publish() {
    use pulse_protocol::gateway::VoiceFlags;
    let app = spawn_only(|l| l.voice_state_user = Rate::new(2, 4.0)).await;
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
    let mut watcher = hello(&app, &b).await;
    let mut talker = hello(&app, &a).await;
    // 21 alternating frames: true, false, ..., true. The last one is over the limit.
    let start = std::time::Instant::now();
    for i in 0..21 {
        talker.send(voice_frame(i % 2 == 0)).await.unwrap();
    }
    let mut seen = 0;
    let mut last = None;
    let mut last_at = start;
    while let Ok(Some(Ok(m))) =
        tokio::time::timeout(Duration::from_millis(1000), watcher.next()).await
    {
        if let Ws::Text(t) = m
            && let Ok(ServerFrame::Event(Event::VoiceStateChanged { flags, .. })) =
                serde_json::from_str(&t)
        {
            seen += 1;
            last = Some(flags);
            last_at = std::time::Instant::now();
        }
    }
    // Every publish (direct or trailing) spends a token, and the bucket was full at `start` at
    // most: so by the last one received, at most burst + refill * elapsed went out. Derived from
    // the measured time, the bound holds on a slow machine too.
    let elapsed = last_at.duration_since(start).as_secs_f64();
    let bound = (2.0 + 4.0 * elapsed).floor() as usize;
    let want = VoiceFlags {
        muted: true,
        deafened: false,
    };
    assert_eq!(last, Some(want), "peers see the final flags");
    assert!(
        seen <= bound,
        "{seen} publishes for 21 frames in {elapsed:.3}s (at most {bound})"
    );
    let room = app
        .voice
        .rooms()
        .into_iter()
        .find(|r| r.0 == lounge)
        .unwrap();
    assert_eq!(room.1, vec![(a_id, want)], "stored flags are the latest");
    assert!(alive(&mut talker).await);
}

/// A user out of creates is refused a new 1:1 DM before any lookup or write lock.
#[tokio::test]
async fn a_limited_new_dm_is_refused_before_any_lookup() {
    let app = spawn_only(|l| l.create_user = tiny(1)).await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let dm = |ids: serde_json::Value| {
        post_json(&app, &a, "/dms", serde_json::json!({ "user_ids": ids }))
    };
    assert_eq!(dm(serde_json::json!([b_id])).await.status(), 200);
    let nobody = pulse_protocol::ids::UserId::new();
    assert_eq!(
        dm(serde_json::json!([nobody])).await.status(),
        429,
        "refused up front, not after looking the user up"
    );
    assert_eq!(
        dm(serde_json::json!([b_id])).await.status(),
        200,
        "reopening is still free"
    );
}

#[tokio::test]
async fn renaming_a_group_is_limited_per_user() {
    let app = spawn_only(|l| l.mutate_user = tiny(3)).await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let (c_id, _) = register(&app, "riley").await;
    let g = create_dm(&app, &a, &[b_id, c_id]).await;
    let rename = |name: &str| {
        app.http
            .patch(app.url(&format!("/channels/{}", g.id)))
            .json(&serde_json::json!({ "name": name }))
    };
    let messages = || async {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages WHERE channel_id = ?")
            .bind(g.id.to_string())
            .fetch_one(&app.db)
            .await
            .unwrap()
    };
    let before = messages().await;
    for i in 0..3 {
        assert_eq!(status(rename(&format!("squad {i}")), &a).await, 200);
    }
    assert_eq!(status(rename("one too many"), &a).await, 429);
    // Refused renames add no system message.
    assert_eq!(messages().await, before + 3);
    // sam has a bucket of his own.
    assert_eq!(status(rename("sam's turn"), &b).await, 200);
}

#[tokio::test]
async fn marking_replies_seen_is_limited() {
    let app = spawn_only(|l| l.mutate_user = tiny(2)).await;
    let (_, a) = register(&app, "alex").await;
    let ch = quotes_channel(&app, &a).await;
    let q = post_quote(&app, &a, &ch).await;
    let r: pulse_protocol::rest::Message = post_json(
        &app,
        &a,
        &format!("/channels/{}/messages", ch.id),
        serde_json::json!({ "content": "r", "thread_root_id": q.id }),
    )
    .await
    .json()
    .await
    .unwrap();
    let seen = || async {
        post_json(
            &app,
            &a,
            &format!("/messages/{}/replies/seen", q.id),
            serde_json::json!({ "message_id": r.id }),
        )
        .await
    };
    assert_eq!(seen().await.status(), 204);
    assert_eq!(seen().await.status(), 204);
    assert_eq!(seen().await.status(), 429);
}

#[tokio::test]
async fn speaker_links_share_the_mutate_limit() {
    let app = spawn_only(|l| l.mutate_user = tiny(2)).await;
    let (_, a) = register(&app, "alex").await;
    let srv = create_server(&app, &a, "Main").await;
    let url = app.url(&format!("/servers/{}/speaker-links", srv.id));
    let body = serde_json::json!({ "name": "x", "user_id": pulse_protocol::ids::UserId::new() });
    assert_eq!(status(app.http.put(&url).json(&body), &a).await, 400);
    assert_eq!(status(app.http.delete(format!("{url}/x")), &a).await, 404);
    assert_eq!(status(app.http.put(&url).json(&body), &a).await, 429);
    assert_eq!(status(app.http.delete(format!("{url}/x")), &a).await, 429);
}

#[tokio::test]
async fn qoty_routes_share_the_mutate_limit() {
    let app = spawn_only(|l| l.mutate_user = tiny(5)).await;
    let (_, a) = register(&app, "alex").await;
    let srv = create_server(&app, &a, "Main").await;
    let e = pulse_protocol::ids::QotyEventId::new();
    let pick = serde_json::json!({ "message_id": pulse_protocol::ids::MessageId::new() });
    let vote = app.url(&format!("/qoty/{e}/vote"));
    let start = app.url(&format!("/servers/{}/qoty", srv.id));
    let year = serde_json::json!({ "year": 1999, "minutes": null });
    assert_eq!(status(app.http.put(&vote).json(&pick), &a).await, 404);
    assert_eq!(status(app.http.delete(&vote), &a).await, 404);
    let options = app.url(&format!("/qoty/{e}/options"));
    assert_eq!(status(app.http.post(&options).json(&pick), &a).await, 404);
    let end = app.url(&format!("/qoty/{e}/end"));
    assert_eq!(status(app.http.post(&end), &a).await, 404);
    assert_eq!(status(app.http.post(&start).json(&year), &a).await, 400);
    let reopen = app.url(&format!("/qoty/{e}/reopen"));
    let minutes = serde_json::json!({ "minutes": null });
    assert_eq!(status(app.http.post(&reopen).json(&minutes), &a).await, 429);
    assert_eq!(status(app.http.post(&start).json(&year), &a).await, 429);
    assert_eq!(status(app.http.put(&vote).json(&pick), &a).await, 429);
}

/// A quotes channel in a new server of `token`'s.
async fn quotes_channel(app: &TestApp, token: &str) -> pulse_protocol::rest::Channel {
    let srv = create_server(app, token, "Main").await;
    let r = post_json(
        app,
        token,
        &format!("/servers/{}/channels", srv.id),
        serde_json::json!({ "kind": "quotes", "name": "quotes" }),
    )
    .await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

/// Post a one-line quote (a plain-name speaker) to `ch`.
async fn post_quote_raw(
    app: &TestApp,
    token: &str,
    ch: &pulse_protocol::rest::Channel,
) -> reqwest::Response {
    post_json(
        app,
        token,
        &format!("/channels/{}/quotes", ch.id),
        serde_json::json!({ "lines": [{ "speaker_name": "A Friend", "text": "hi" }] }),
    )
    .await
}

async fn post_quote(
    app: &TestApp,
    token: &str,
    ch: &pulse_protocol::rest::Channel,
) -> pulse_protocol::rest::Message {
    let r = post_quote_raw(app, token, ch).await;
    assert_eq!(r.status(), 200, "post quote");
    r.json().await.unwrap()
}

#[tokio::test]
async fn posting_quotes_shares_the_send_limit() {
    let app = spawn_only(|l| l.send_user = tiny(2)).await;
    let (_, a) = register(&app, "alex").await;
    let ch = quotes_channel(&app, &a).await;
    let post = || post_quote_raw(&app, &a, &ch);
    assert_eq!(post().await.status(), 200);
    assert_eq!(post().await.status(), 200);
    assert_eq!(post().await.status(), 429);
}

#[tokio::test]
async fn quote_edits_and_awards_share_the_mutate_limit() {
    let app = spawn_only(|l| l.mutate_user = tiny(3)).await;
    let (_, a) = register(&app, "alex").await;
    let ch = quotes_channel(&app, &a).await;
    let q = post_quote(&app, &a, &ch).await;
    let line = q.quote.unwrap().lines[0].id;
    let edit = || {
        app.http
            .put(app.url(&format!("/quotes/{}", q.id)))
            .json(&serde_json::json!({
                "lines": [{ "id": line, "speaker_name": "A Friend", "text": "hi!" }]
            }))
    };
    let award = app.url(&format!("/quote-lines/{line}/award"));
    assert_eq!(status(edit(), &a).await, 200);
    assert_eq!(status(app.http.put(&award), &a).await, 204);
    assert_eq!(status(app.http.delete(&award), &a).await, 204);
    assert_eq!(status(edit(), &a).await, 429);
    assert_eq!(status(app.http.put(&award), &a).await, 429);
    assert_eq!(status(app.http.delete(&award), &a).await, 429);
}

#[tokio::test]
async fn clip_view_changes_share_the_mutate_limit() {
    let app = spawn_only(|l| l.mutate_user = tiny(2)).await;
    let (_, a) = register(&app, "alex").await;
    let srv = create_server(&app, &a, "Main").await;
    let ch: pulse_protocol::rest::Channel = post_json(
        &app,
        &a,
        &format!("/servers/{}/channels", srv.id),
        serde_json::json!({ "kind": "clips", "name": "clips" }),
    )
    .await
    .json()
    .await
    .unwrap();
    let put = || {
        app.http
            .put(app.url(&format!("/channels/{}/clip-view", ch.id)))
            .json(&serde_json::json!({ "view": "list" }))
    };
    assert_eq!(status(put(), &a).await, 204);
    assert_eq!(status(put(), &a).await, 204);
    assert_eq!(status(put(), &a).await, 429);
}
