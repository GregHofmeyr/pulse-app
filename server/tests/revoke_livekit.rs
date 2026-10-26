//! Revoking a session also removes the LiveKit participants it admitted (voice room and stream
//! rooms), and `logout-others` counts only sessions that were still live. LiveKit is a fake
//! RoomService here (Twirp: POST /twirp/livekit.RoomService/RemoveParticipant, protobuf bodies).
//!
//! Every voice token / stream ticket carries a random grant id (participant attribute
//! `pulse_grant`). A join webhook is accepted only for a grant of a live session that matches
//! the room + identity, and records which grant that participant joined with; revoking removes
//! exactly the participants whose joined grant belonged to a revoked session.

mod common;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, header};
use axum::routing::post;
use base64::Engine;
use common::*;
use livekit_api::access_token::{AccessToken, TokenVerifier};
use livekit_protocol as lkp;
use prost::Message as _;
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{Channel, LogoutOthersResponse, SessionResponse, VoiceTokenResponse};
use pulse_server::auth::session;
use serde_json::json;
use sha2::{Digest, Sha256};

type Removed = Arc<Mutex<Vec<(String, String)>>>;

/// A fake RoomService that records every RemoveParticipant; returns (base URL, calls). Like
/// LiveKit it refuses (401, nothing recorded) a request that isn't protobuf or whose bearer JWT
/// isn't signed with the test key or lacks `room_admin` for that room.
async fn fake_livekit() -> (String, Removed) {
    let removed: Removed = Arc::default();
    let r = removed.clone();
    let cfg = test_config();
    let verifier = Arc::new(TokenVerifier::with_api_key(
        &cfg.livekit_key,
        &cfg.livekit_secret,
    ));
    let app = axum::Router::new().route(
        "/twirp/livekit.RoomService/RemoveParticipant",
        post(move |headers: HeaderMap, body: Bytes| {
            let (r, verifier) = (r.clone(), verifier.clone());
            async move {
                let req = lkp::RoomParticipantIdentity::decode(body).unwrap();
                let protobuf = headers
                    .get(header::CONTENT_TYPE)
                    .is_some_and(|v| v == "application/protobuf");
                let claims = headers
                    .get(header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer "))
                    .and_then(|jwt| verifier.verify(jwt).ok());
                let admin = claims.is_some_and(|c| c.video.room_admin && c.video.room == req.room);
                if !(protobuf && admin) {
                    return Err(StatusCode::UNAUTHORIZED);
                }
                r.lock().unwrap().push((req.room, req.identity));
                Ok(lkp::RemoveParticipantResponse {}.encode_to_vec())
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

/// The grant id a LiveKit JWT carries (verified with the test key).
fn grant_of(app: &TestApp, jwt: &str) -> String {
    TokenVerifier::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .verify(jwt)
        .unwrap()
        .attributes
        .get(session::GRANT_ATTRIBUTE)
        .cloned()
        .expect("every LiveKit token carries a grant id")
}

fn signed(app: &TestApp, body: &str) -> String {
    let sum = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body.as_bytes()));
    AccessToken::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .with_sha256(&sum)
        .to_jwt()
        .unwrap()
}

/// LiveKit's signed `participant_joined` webhook for `identity` in `room`, carrying `grant` as
/// the participant's `pulse_grant` attribute (LiveKit copies a token's attributes onto it).
async fn joined_raw(app: &TestApp, room: &str, identity: &str, grant: Option<&str>, sid: &str) {
    let attributes = match grant {
        Some(g) => json!({ session::GRANT_ATTRIBUTE: g }),
        None => json!({}),
    };
    let body = json!({"event": "participant_joined", "id": "EV_join", "room": {"name": room},
        "participant": {"identity": identity, "sid": sid, "attributes": attributes}})
    .to_string();
    let r = app
        .http
        .post(app.url("/livekit/webhook"))
        .header("Authorization", signed(app, &body))
        .header("Content-Type", "application/webhook+json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

/// A participant connecting with `jwt`: the room and identity are the ones the token names.
async fn join_with(app: &TestApp, jwt: &str, sid: &str) {
    let claims = TokenVerifier::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .verify(jwt)
        .unwrap();
    joined_raw(
        app,
        &claims.video.room,
        &claims.sub,
        Some(&grant_of(app, jwt)),
        sid,
    )
    .await;
}

/// Wait (up to 3 s) until `removed` holds `n` calls; the webhook removes in the background.
async fn wait_for(removed: &Removed, n: usize) {
    let until = Instant::now() + Duration::from_secs(3);
    while removed.lock().unwrap().len() < n && Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

struct Setup {
    app: TestApp,
    removed: Removed,
    alex: UserId,
    sam: UserId,
    a1: String,
    a2: String,
    a3: String,
    /// the JWTs each participant below connected with
    a2_voice: String,
    a2_video: String,
    a3_dm: String,
    lounge: Channel,
    dm: Channel,
}

/// alex on three devices (a1 = the caller), sam on one, each connected through LiveKit (join
/// webhooks): a2 in Lounge voice and streaming video, a3 in a DM call with sam, a1 holding the
/// stream-audio identity, sam in the DM call.
async fn setup() -> Setup {
    let (url, removed) = fake_livekit().await;
    let app = app_with_livekit(url).await;
    let (alex, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let a3 = login(&app, "alex").await;
    let (sam, s) = register(&app, "sam").await;
    let lounge = lounge(&app, &a1).await;
    let dm = create_dm(&app, &a1, &[sam]).await;

    let a2_voice = voice_token(&app, &a2, &lounge).await;
    join_with(&app, &a2_voice, "PA_a2").await; // now in Lounge voice: may stream
    let a2_video = stream_ticket(&app, &a2, &lounge, "v").await;
    join_with(&app, &a2_video, "PA_a2_v").await;
    let a1_audio = stream_ticket(&app, &a1, &lounge, "a").await;
    join_with(&app, &a1_audio, "PA_a1_a").await;
    let a3_dm = voice_token(&app, &a3, &dm).await;
    join_with(&app, &a3_dm, "PA_a3").await;
    let sam_dm = voice_token(&app, &s, &dm).await;
    join_with(&app, &sam_dm, "PA_sam").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(removed.lock().unwrap().is_empty(), "all joins were valid");
    Setup {
        app,
        removed,
        alex,
        sam,
        a1,
        a2,
        a3,
        a2_voice,
        a2_video,
        a3_dm,
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
    logout(&t.app, &t.a3).await;
    assert_eq!(t.removed.lock().unwrap().len(), 3);
    assert!(
        calls(&t.removed).contains(&(t.dm.id.to_string(), t.alex.to_string())),
        "a3's DM call goes when a3 signs out"
    );
}

/// The review case: another device merely fetching a token (hover prefetch) for the room the
/// caller is in must not get the caller removed when that device is signed out.
#[tokio::test]
async fn a_prefetch_by_the_revoked_device_does_not_remove_the_caller() {
    let (url, removed) = fake_livekit().await;
    let app = app_with_livekit(url).await;
    let (_, a) = register(&app, "alex").await;
    let b = login(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let mine = voice_token(&app, &a, &l).await;
    join_with(&app, &mine, "PA_a").await;
    voice_token(&app, &b, &l).await; // B hovers Lounge: a token, but never used
    assert_eq!(logout_others(&app, &a).await, 1);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(removed.lock().unwrap().is_empty(), "{:?}", calls(&removed));
}

/// Device B joined, then the caller's device A joined the same room (LiveKit drops B as a
/// duplicate identity): the participant now in the room is A's, so revoking B removes nothing.
#[tokio::test]
async fn the_grant_a_participant_joined_with_decides_not_the_latest_token() {
    let (url, removed) = fake_livekit().await;
    let app = app_with_livekit(url).await;
    let (_, a) = register(&app, "alex").await;
    let b = login(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let theirs = voice_token(&app, &b, &l).await;
    join_with(&app, &theirs, "PA_b").await;
    let mine = voice_token(&app, &a, &l).await;
    join_with(&app, &mine, "PA_a").await;
    voice_token(&app, &b, &l).await; // and B fetches another, unused
    assert_eq!(logout_others(&app, &a).await, 1);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(removed.lock().unwrap().is_empty(), "{:?}", calls(&removed));
}

#[tokio::test]
async fn no_livekit_call_when_the_revoked_sessions_hold_no_participants() {
    let (url, removed) = fake_livekit().await;
    let app = app_with_livekit(url).await;
    let (_, a1) = register(&app, "alex").await;
    let _a2 = login(&app, "alex").await;
    let l = lounge(&app, &a1).await;
    let mine = voice_token(&app, &a1, &l).await;
    join_with(&app, &mine, "PA_a1").await;
    assert_eq!(logout_others(&app, &a1).await, 1);
    assert!(removed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn livekit_down_does_not_fail_the_logout() {
    // test_config's LiveKit API URL refuses connections.
    let app = spawn().await;
    let (_, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let a3 = login(&app, "alex").await;
    let l = lounge(&app, &a1).await;
    let t2 = voice_token(&app, &a2, &l).await;
    join_with(&app, &t2, "PA_2").await;
    let t3 = voice_token(&app, &a3, &l).await;
    join_with(&app, &t3, "PA_3").await;
    assert_eq!(logout_others(&app, &a1).await, 2);
    assert_eq!(session::authenticate(&app.db, &a2).await.unwrap(), None);
    assert_eq!(session::authenticate(&app.db, &a3).await.unwrap(), None);
    // and the caller's own plain logout still goes through while in a call
    let t1 = voice_token(&app, &a1, &l).await;
    join_with(&app, &t1, "PA_1").await;
    logout(&app, &a1).await;
    assert_eq!(session::authenticate(&app.db, &a1).await.unwrap(), None);
}

#[tokio::test]
async fn a_hanging_livekit_delays_the_logout_only_briefly() {
    let app = app_with_livekit(fake_livekit_hanging().await).await;
    let (_, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let l = lounge(&app, &a1).await;
    let voice = voice_token(&app, &a2, &l).await;
    join_with(&app, &voice, "PA_2").await;
    let video = stream_ticket(&app, &a2, &l, "v").await;
    join_with(&app, &video, "PA_2v").await;
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
    let mut grants = BTreeSet::new();
    for jwt in [
        voice_token(&app, &a, &l).await,
        stream_ticket(&app, &a, &l, "v").await,
        stream_ticket(&app, &a, &l, "a").await,
    ] {
        let payload = jwt_payload(&jwt);
        assert!(!payload.contains(&hash), "{payload}");
        assert!(!payload.contains(&a), "{payload}");
        let grant = grant_of(&app, &jwt);
        assert!(!hash.contains(&grant) && !a.contains(&grant));
        assert!(grant.len() >= 22, "128-bit random id: {grant}");
        grants.insert(grant);
    }
    assert_eq!(grants.len(), 3, "a fresh id per token");
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

/// LiveKit doesn't revoke a removed device's JWT: rejoining with it is removed again.
#[tokio::test]
async fn a_revoked_device_rejoining_with_its_old_jwt_is_removed_again() {
    let t = setup().await;
    logout_others(&t.app, &t.a1).await;
    t.removed.lock().unwrap().clear();

    join_with(&t.app, &t.a3_dm, "PA_back").await;
    join_with(&t.app, &t.a2_video, "PA_back_v").await;
    wait_for(&t.removed, 2).await;
    assert_eq!(
        calls(&t.removed),
        set(&[
            (t.dm.id.to_string(), t.alex.to_string()),
            (format!("{}~s", t.lounge.id), format!("{}#v", t.alex)),
        ])
    );
    // (no-presence for refused joins: `a_join_without_a_known_grant_is_removed`)
    let _ = (&t.a2_voice, &t.a2, &t.a3);
}

#[tokio::test]
async fn joins_backed_by_a_live_grant_are_left_alone() {
    let t = setup().await;
    logout_others(&t.app, &t.a1).await;
    t.removed.lock().unwrap().clear();

    // The caller joins Lounge voice from this device, and LiveKit later does a full reconnect
    // of it (new participant, refreshed token with the same attributes).
    let mine = voice_token(&t.app, &t.a1, &t.lounge).await;
    join_with(&t.app, &mine, "PA_a1").await;
    join_with(&t.app, &mine, "PA_a1_again").await;

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        t.removed.lock().unwrap().is_empty(),
        "{:?}",
        calls(&t.removed)
    );
    assert!(t.app.voice_members(t.lounge.id).contains(&t.alex));
    assert!(t.app.voice_members(t.dm.id).contains(&t.sam));
}

#[tokio::test]
async fn a_live_grant_used_for_another_room_or_identity_is_removed() {
    let (url, removed) = fake_livekit().await;
    let app = app_with_livekit(url).await;
    let (alex, a) = register(&app, "alex").await;
    let (sam, _) = register(&app, "sam").await;
    let l = lounge(&app, &a).await;
    let dm = create_dm(&app, &a, &[sam]).await;
    let grant = grant_of(&app, &voice_token(&app, &a, &l).await);
    let (lounge_room, dm_room) = (l.id.to_string(), dm.id.to_string());
    let video = format!("{alex}#v");
    let stream_room = format!("{}~s", l.id);
    let sam_id = sam.to_string();
    let cases: [(&str, &str); 3] = [
        (&dm_room, &alex.to_string()), // another room
        (&stream_room, &video),        // another identity
        (&lounge_room, &sam_id),       // another user
    ];
    for (i, (room, identity)) in cases.iter().enumerate() {
        joined_raw(&app, room, identity, Some(&grant), &format!("PA_{i}")).await;
    }
    wait_for(&removed, 3).await;
    assert_eq!(removed.lock().unwrap().len(), 3, "{:?}", calls(&removed));
    assert!(!app.voice_members(dm.id).contains(&alex));
    assert!(!app.voice_members(l.id).contains(&sam));
}

#[tokio::test]
async fn a_join_without_a_known_grant_is_removed() {
    // e.g. a token minted before grants existed, or a made-up id
    let (url, removed) = fake_livekit().await;
    let app = app_with_livekit(url).await;
    let (alex, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let room = l.id.to_string();
    joined_raw(&app, &room, &alex.to_string(), None, "PA_x").await;
    joined_raw(&app, &room, &alex.to_string(), Some("made-up"), "PA_y").await;
    wait_for(&removed, 2).await;
    assert_eq!(removed.lock().unwrap().len(), 2);
    assert!(!app.voice_members(l.id).contains(&alex));
    let open: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_sessions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(open, 0, "no voice-hours row for a refused join");
}

#[tokio::test]
async fn a_join_after_the_session_expired_is_removed() {
    let (url, removed) = fake_livekit().await;
    let app = app_with_livekit(url).await;
    let (alex, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let jwt = voice_token(&app, &a, &l).await;
    sqlx::query("UPDATE sessions SET expires_at = '2020-01-01T00:00:00.000Z'")
        .execute(&app.db)
        .await
        .unwrap();
    join_with(&app, &jwt, "PA_x").await;
    wait_for(&removed, 1).await;
    assert_eq!(removed.lock().unwrap().len(), 1);
    assert!(!app.voice_members(l.id).contains(&alex));
}
