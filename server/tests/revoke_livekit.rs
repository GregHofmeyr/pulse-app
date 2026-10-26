//! Revoking a session also removes the LiveKit participants it admitted (voice room and stream
//! rooms), and `logout-others` counts only sessions that were still live. LiveKit is a fake
//! RoomService here (Twirp: POST /twirp/livekit.RoomService/RemoveParticipant, protobuf bodies).

mod common;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::routing::post;
use base64::Engine;
use common::*;
use livekit_protocol as lkp;
use prost::Message as _;
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{Channel, LogoutOthersResponse, SessionResponse, VoiceTokenResponse};
use pulse_server::auth::session;
use serde_json::json;

type Removed = Arc<Mutex<Vec<(String, String)>>>;

/// A fake RoomService that records every RemoveParticipant; returns (base URL, calls).
async fn fake_livekit() -> (String, Removed) {
    let removed: Removed = Arc::default();
    let r = removed.clone();
    let app = axum::Router::new().route(
        "/twirp/livekit.RoomService/RemoveParticipant",
        post(move |body: Bytes| {
            let r = r.clone();
            async move {
                let req = lkp::RoomParticipantIdentity::decode(body).unwrap();
                r.lock().unwrap().push((req.room, req.identity));
                lkp::RemoveParticipantResponse {}.encode_to_vec()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), removed)
}

/// A RoomService that accepts the connection and never answers.
async fn fake_livekit_hanging() -> String {
    let app = axum::Router::new().route(
        "/twirp/livekit.RoomService/RemoveParticipant",
        post(|| async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Vec::<u8>::new()
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn app_with_livekit(url: String) -> TestApp {
    let mut cfg = test_config();
    cfg.livekit_api_url = url;
    spawn_with(cfg).await
}

async fn login(app: &TestApp, username: &str) -> String {
    let r: SessionResponse = app
        .http
        .post(app.url("/auth/login"))
        .json(&json!({"username": username, "password": "hunter2hunter2"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    r.token
}

async fn lounge(app: &TestApp, token: &str) -> Channel {
    let s = create_server(app, token, "Main").await;
    let chans: Vec<Channel> = get_json(app, token, &format!("/servers/{}/channels", s.id)).await;
    chans
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap()
}

async fn voice_token(app: &TestApp, token: &str, channel: &Channel) -> String {
    let r = post_json(
        app,
        token,
        &format!("/voice/{}/token", channel.id),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 200);
    r.json::<VoiceTokenResponse>().await.unwrap().token
}

async fn stream_ticket(app: &TestApp, token: &str, channel: &Channel, role: &str) -> String {
    let r = post_json(
        app,
        token,
        &format!("/streams/{}/ticket", channel.id),
        json!({ "role": role }),
    )
    .await;
    assert_eq!(r.status(), 200);
    r.json::<VoiceTokenResponse>().await.unwrap().token
}

async fn logout_others(app: &TestApp, token: &str) -> u32 {
    let r = app
        .http
        .post(app.url("/auth/logout-others"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json::<LogoutOthersResponse>().await.unwrap().revoked
}

async fn logout(app: &TestApp, token: &str) {
    let r = app
        .http
        .post(app.url("/auth/logout"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
}

fn calls(removed: &Removed) -> BTreeSet<(String, String)> {
    removed.lock().unwrap().iter().cloned().collect()
}

fn set(items: &[(String, String)]) -> BTreeSet<(String, String)> {
    items.iter().cloned().collect()
}

/// The JWT's payload, decoded (no verification: only checking what it carries).
fn jwt_payload(jwt: &str) -> String {
    let part = jwt.split('.').nth(1).unwrap();
    String::from_utf8(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(part)
            .unwrap(),
    )
    .unwrap()
}

struct Setup {
    app: TestApp,
    removed: Removed,
    alex: UserId,
    a1: String,
    a2: String,
    a3: String,
    lounge: Channel,
    dm: Channel,
}

/// alex on three devices (a1 = the caller), sam on one. a2 is in Lounge voice and streaming
/// video; a3 is in a DM call with sam; a1 holds the stream-audio identity; sam is in the DM call.
async fn setup() -> Setup {
    let (url, removed) = fake_livekit().await;
    let app = app_with_livekit(url).await;
    let (alex, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let a3 = login(&app, "alex").await;
    let (sam, s) = register(&app, "sam").await;
    let lounge = lounge(&app, &a1).await;
    let dm = create_dm(&app, &a1, &[sam]).await;
    app.voice.join(lounge.id, alex, "PA_alex"); // what the LiveKit join webhook does

    voice_token(&app, &a2, &lounge).await;
    stream_ticket(&app, &a2, &lounge, "v").await;
    stream_ticket(&app, &a1, &lounge, "a").await;
    voice_token(&app, &a3, &dm).await;
    voice_token(&app, &s, &dm).await;
    Setup {
        app,
        removed,
        alex,
        a1,
        a2,
        a3,
        lounge,
        dm,
    }
}

#[tokio::test]
async fn logout_others_removes_exactly_the_revoked_sessions_participants() {
    let t = setup().await;
    assert_eq!(logout_others(&t.app, &t.a1).await, 2);
    assert_eq!(
        calls(&t.removed),
        set(&[
            (t.lounge.id.to_string(), t.alex.to_string()),
            (format!("{}~s", t.lounge.id), format!("{}#v", t.alex)),
            (t.dm.id.to_string(), t.alex.to_string()),
        ]),
        "a1's own stream audio and sam's DM call are untouched"
    );
    assert_eq!(t.removed.lock().unwrap().len(), 3, "each removed once");
}

#[tokio::test]
async fn plain_logout_removes_only_that_sessions_participants() {
    let t = setup().await;
    logout(&t.app, &t.a2).await;
    assert_eq!(
        calls(&t.removed),
        set(&[
            (t.lounge.id.to_string(), t.alex.to_string()),
            (format!("{}~s", t.lounge.id), format!("{}#v", t.alex)),
        ])
    );
    // a3 and a1 are still signed in, and their grants survive a later revoke.
    logout(&t.app, &t.a3).await;
    assert_eq!(t.removed.lock().unwrap().len(), 3);
    assert!(
        calls(&t.removed).contains(&(t.dm.id.to_string(), t.alex.to_string())),
        "a3's DM call goes when a3 signs out"
    );
}

#[tokio::test]
async fn the_last_issued_token_owns_the_identity() {
    let t = setup().await;
    // The caller re-joins Lounge voice from this device: that identity is now a1's.
    voice_token(&t.app, &t.a1, &t.lounge).await;
    assert_eq!(logout_others(&t.app, &t.a1).await, 2);
    let got = calls(&t.removed);
    assert!(
        !got.contains(&(t.lounge.id.to_string(), t.alex.to_string())),
        "the caller's own voice must stay: {got:?}"
    );
    assert_eq!(
        got,
        set(&[
            (format!("{}~s", t.lounge.id), format!("{}#v", t.alex)),
            (t.dm.id.to_string(), t.alex.to_string()),
        ])
    );
}

#[tokio::test]
async fn no_livekit_call_when_the_revoked_sessions_hold_no_tokens() {
    let (url, removed) = fake_livekit().await;
    let app = app_with_livekit(url).await;
    let (alex, a1) = register(&app, "alex").await;
    let _a2 = login(&app, "alex").await;
    let l = lounge(&app, &a1).await;
    app.voice.join(l.id, alex, "PA_alex");
    voice_token(&app, &a1, &l).await; // only the caller holds one
    assert_eq!(logout_others(&app, &a1).await, 1);
    assert!(removed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn livekit_down_does_not_fail_the_logout() {
    // test_config's LiveKit API URL refuses connections.
    let app = spawn().await;
    let (alex, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let a3 = login(&app, "alex").await;
    let l = lounge(&app, &a1).await;
    app.voice.join(l.id, alex, "PA_alex");
    voice_token(&app, &a2, &l).await;
    voice_token(&app, &a3, &l).await;
    assert_eq!(logout_others(&app, &a1).await, 2);
    assert_eq!(session::authenticate(&app.db, &a2).await.unwrap(), None);
    assert_eq!(session::authenticate(&app.db, &a3).await.unwrap(), None);
    // and the caller's own plain logout still goes through after a voice token
    voice_token(&app, &a1, &l).await;
    logout(&app, &a1).await;
    assert_eq!(session::authenticate(&app.db, &a1).await.unwrap(), None);
}

#[tokio::test]
async fn a_hanging_livekit_delays_the_logout_only_briefly() {
    let app = app_with_livekit(fake_livekit_hanging().await).await;
    let (alex, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let l = lounge(&app, &a1).await;
    app.voice.join(l.id, alex, "PA_alex");
    voice_token(&app, &a2, &l).await;
    stream_ticket(&app, &a2, &l, "v").await;
    let started = Instant::now();
    assert_eq!(logout_others(&app, &a1).await, 1);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "took {:?}",
        started.elapsed()
    );
    assert_eq!(session::authenticate(&app.db, &a2).await.unwrap(), None);
}

#[tokio::test]
async fn session_hashes_never_reach_livekit_tokens() {
    let (url, _) = fake_livekit().await;
    let app = app_with_livekit(url).await;
    let (alex, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    app.voice.join(l.id, alex, "PA_alex");
    let hash = session::hash_token(&a);
    for jwt in [
        voice_token(&app, &a, &l).await,
        stream_ticket(&app, &a, &l, "v").await,
        stream_ticket(&app, &a, &l, "a").await,
    ] {
        let payload = jwt_payload(&jwt);
        assert!(!payload.contains(&hash), "{payload}");
        assert!(!payload.contains(&a), "{payload}");
    }
}

#[tokio::test]
async fn revoked_counts_only_live_sessions_but_deletes_expired_ones() {
    let app = spawn().await;
    let (alex, a1) = register(&app, "alex").await;
    let _a2 = login(&app, "alex").await;
    for (hash, expires) in [
        ("expired-1", "2020-01-01T00:00:00.000Z"),
        ("expired-2", "2026-01-01T00:00:00.000Z"),
    ] {
        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, created_at, last_used_at, expires_at)
             VALUES (?, ?, '2019-01-01T00:00:00.000Z', '2019-01-01T00:00:00.000Z', ?)",
        )
        .bind(hash)
        .bind(alex.to_string())
        .bind(expires)
        .execute(&app.db)
        .await
        .unwrap();
    }
    assert_eq!(
        logout_others(&app, &a1).await,
        1,
        "only the live one counts"
    );
    let left: Vec<String> = sqlx::query_scalar("SELECT token_hash FROM sessions WHERE user_id = ?")
        .bind(alex.to_string())
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(
        left,
        vec![session::hash_token(&a1)],
        "expired rows are gone too"
    );
}

#[tokio::test]
async fn a_token_for_a_revoked_session_is_refused() {
    let (url, _) = fake_livekit().await;
    let app = app_with_livekit(url).await;
    let (_, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let l = lounge(&app, &a1).await;
    logout_others(&app, &a1).await;
    let r = post_json(&app, &a2, &format!("/voice/{}/token", l.id), json!({})).await;
    assert_eq!(r.status(), 401);
}
