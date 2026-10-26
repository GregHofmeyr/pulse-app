//! Revoking a session also removes the LiveKit participants it admitted (voice room and stream
//! rooms), and `logout-others` counts only sessions that were still live. LiveKit is a fake
//! RoomService here (Twirp: POST /twirp/livekit.RoomService/<Method>, protobuf bodies) that keeps
//! a participant list, like LiveKit, and checks the admin JWT on every call.
//!
//! Every voice token / stream ticket carries a random grant id (participant attribute
//! `pulse_grant`). A join webhook is accepted only for a grant of a live session that matches
//! the room + identity. Revoking asks LiveKit who is in the rooms the revoked sessions hold
//! grants for and removes exactly the participants carrying one of those grants.

mod common;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
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
type Rooms = Arc<Mutex<Vec<(String, lkp::ParticipantInfo)>>>;

/// The fake LiveKit: its URL, every RemoveParticipant it was asked for (room, identity), and
/// who is connected (room, participant).
#[derive(Clone)]
struct Lk {
    url: String,
    removed: Removed,
    rooms: Rooms,
}

impl Lk {
    /// LiveKit admitting a participant (a newer connection with the same identity replaces the
    /// older one, as LiveKit's duplicate-identity rule does).
    fn connect(&self, room: &str, identity: &str, sid: &str, grant: Option<&str>) {
        let mut rooms = self.rooms.lock().unwrap();
        rooms.retain(|(r, p)| !(r == room && p.identity == identity));
        rooms.push((
            room.to_owned(),
            lkp::ParticipantInfo {
                identity: identity.to_owned(),
                sid: sid.to_owned(),
                state: lkp::participant_info::State::Active as i32,
                attributes: grant
                    .map(|g| [(session::GRANT_ATTRIBUTE.to_owned(), g.to_owned())].into())
                    .unwrap_or_default(),
                ..Default::default()
            },
        ));
    }

    fn connected(&self, room: &str, identity: &str) -> bool {
        self.rooms
            .lock()
            .unwrap()
            .iter()
            .any(|(r, p)| r == room && p.identity == identity)
    }
}

/// The admin check LiveKit makes: protobuf body, and a bearer JWT signed with our key that
/// grants `room_admin` for `room`.
fn authorized(verifier: &TokenVerifier, headers: &HeaderMap, room: &str) -> bool {
    let protobuf = headers
        .get(header::CONTENT_TYPE)
        .is_some_and(|v| v == "application/protobuf");
    let claims = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|jwt| verifier.verify(jwt).ok());
    protobuf && claims.is_some_and(|c| c.video.room_admin && c.video.room == room)
}

fn twirp_not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"code":"not_found","msg":"participant not found"}"#,
    )
        .into_response()
}

/// A fake RoomService (ListParticipants, GetParticipant, RemoveParticipant) over `Lk::rooms`.
/// Refuses (401, nothing done) anything [`authorized`] rejects.
async fn fake_livekit() -> Lk {
    let lk = Lk {
        url: String::new(),
        removed: Arc::default(),
        rooms: Arc::default(),
    };
    let cfg = test_config();
    let verifier = Arc::new(TokenVerifier::with_api_key(
        &cfg.livekit_key,
        &cfg.livekit_secret,
    ));
    let (v1, v2, v3) = (verifier.clone(), verifier.clone(), verifier);
    let (r1, r2, r3, removed) = (
        lk.rooms.clone(),
        lk.rooms.clone(),
        lk.rooms.clone(),
        lk.removed.clone(),
    );
    let app = axum::Router::new()
        .route(
            "/twirp/livekit.RoomService/ListParticipants",
            post(move |headers: HeaderMap, body: Bytes| {
                let (v, rooms) = (v1.clone(), r1.clone());
                async move {
                    let req = lkp::ListParticipantsRequest::decode(body).unwrap();
                    if !authorized(&v, &headers, &req.room) {
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    let participants = rooms
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|(r, _)| *r == req.room)
                        .map(|(_, p)| p.clone())
                        .collect();
                    lkp::ListParticipantsResponse { participants }
                        .encode_to_vec()
                        .into_response()
                }
            }),
        )
        .route(
            "/twirp/livekit.RoomService/GetParticipant",
            post(move |headers: HeaderMap, body: Bytes| {
                let (v, rooms) = (v2.clone(), r2.clone());
                async move {
                    let req = lkp::RoomParticipantIdentity::decode(body).unwrap();
                    if !authorized(&v, &headers, &req.room) {
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    let found = rooms
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|(r, p)| *r == req.room && p.identity == req.identity)
                        .map(|(_, p)| p.clone());
                    match found {
                        Some(p) => p.encode_to_vec().into_response(),
                        None => twirp_not_found(),
                    }
                }
            }),
        )
        .route(
            "/twirp/livekit.RoomService/RemoveParticipant",
            post(move |headers: HeaderMap, body: Bytes| {
                let (v, rooms, removed) = (v3.clone(), r3.clone(), removed.clone());
                async move {
                    let req = lkp::RoomParticipantIdentity::decode(body).unwrap();
                    if !authorized(&v, &headers, &req.room) {
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    let mut rooms = rooms.lock().unwrap();
                    let before = rooms.len();
                    rooms.retain(|(r, p)| !(*r == req.room && p.identity == req.identity));
                    if rooms.len() == before {
                        return twirp_not_found();
                    }
                    removed.lock().unwrap().push((req.room, req.identity));
                    lkp::RemoveParticipantResponse {}
                        .encode_to_vec()
                        .into_response()
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Lk {
        url: format!("http://{addr}"),
        ..lk
    }
}

/// A RoomService that accepts the connection and never answers.
async fn fake_livekit_hanging() -> String {
    let hang = || async {
        tokio::time::sleep(Duration::from_secs(60)).await;
        Vec::<u8>::new()
    };
    let app = axum::Router::new()
        .route("/twirp/livekit.RoomService/ListParticipants", post(hang))
        .route("/twirp/livekit.RoomService/GetParticipant", post(hang))
        .route("/twirp/livekit.RoomService/RemoveParticipant", post(hang));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn app_with_livekit(url: &str) -> TestApp {
    let mut cfg = test_config();
    cfg.livekit_api_url = url.to_owned();
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

fn calls(lk: &Lk) -> BTreeSet<(String, String)> {
    lk.removed.lock().unwrap().iter().cloned().collect()
}

fn removed_count(lk: &Lk) -> usize {
    lk.removed.lock().unwrap().len()
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

/// LiveKit's signed webhook (`participant_joined` / `participant_left`) for a participant
/// carrying `grant` as its `pulse_grant` attribute (LiveKit copies a token's attributes onto it).
async fn webhook(
    app: &TestApp,
    event: &str,
    room: &str,
    identity: &str,
    grant: Option<&str>,
    sid: &str,
) {
    let attributes = match grant {
        Some(g) => json!({ session::GRANT_ATTRIBUTE: g }),
        None => json!({}),
    };
    let body = json!({"event": event, "id": "EV_test", "room": {"name": room},
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

/// A participant connecting with any token LiveKit accepts: LiveKit admits it (when `lk` is
/// given) and then sends the join webhook.
async fn connect_raw(
    app: &TestApp,
    lk: Option<&Lk>,
    room: &str,
    identity: &str,
    grant: Option<&str>,
    sid: &str,
) {
    if let Some(lk) = lk {
        lk.connect(room, identity, sid, grant);
    }
    webhook(app, "participant_joined", room, identity, grant, sid).await;
}

/// The room and identity a JWT names.
fn names(app: &TestApp, jwt: &str) -> (String, String) {
    let c = TokenVerifier::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .verify(jwt)
        .unwrap();
    (c.video.room, c.sub)
}

/// A participant connecting with `jwt` (room and identity are the ones the token names).
async fn connect(app: &TestApp, lk: Option<&Lk>, jwt: &str, sid: &str) {
    let (room, identity) = names(app, jwt);
    connect_raw(app, lk, &room, &identity, Some(&grant_of(app, jwt)), sid).await;
}

/// Wait (up to 3 s) until LiveKit was asked for `n` removals; webhooks remove in the background.
async fn wait_for(lk: &Lk, n: usize) {
    let until = Instant::now() + Duration::from_secs(3);
    while removed_count(lk) < n && Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

struct Setup {
    app: TestApp,
    lk: Lk,
    alex: UserId,
    sam: UserId,
    a1: String,
    a2: String,
    a3: String,
    /// the JWTs each participant below connected with
    a2_video: String,
    a3_dm: String,
    lounge: Channel,
    dm: Channel,
}

/// alex on three devices (a1 = the caller), sam on one, each connected through LiveKit: a2 in
/// Lounge voice and streaming video, a3 in a DM call with sam, a1 holding the stream-audio
/// identity, sam in the DM call.
async fn setup() -> Setup {
    let lk = fake_livekit().await;
    let app = app_with_livekit(&lk.url).await;
    let (alex, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let a3 = login(&app, "alex").await;
    let (sam, s) = register(&app, "sam").await;
    let lounge = lounge(&app, &a1).await;
    let dm = create_dm(&app, &a1, &[sam]).await;

    let a2_voice = voice_token(&app, &a2, &lounge).await;
    connect(&app, Some(&lk), &a2_voice, "PA_a2").await; // now in Lounge voice: may stream
    let a2_video = stream_ticket(&app, &a2, &lounge, "v").await;
    connect(&app, Some(&lk), &a2_video, "PA_a2_v").await;
    let a1_audio = stream_ticket(&app, &a1, &lounge, "a").await;
    connect(&app, Some(&lk), &a1_audio, "PA_a1_a").await;
    let a3_dm = voice_token(&app, &a3, &dm).await;
    connect(&app, Some(&lk), &a3_dm, "PA_a3").await;
    let sam_dm = voice_token(&app, &s, &dm).await;
    connect(&app, Some(&lk), &sam_dm, "PA_sam").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(removed_count(&lk), 0, "all joins were valid");
    Setup {
        app,
        lk,
        alex,
        sam,
        a1,
        a2,
        a3,
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
        calls(&t.lk),
        set(&[
            (t.lounge.id.to_string(), t.alex.to_string()),
            (format!("{}~s", t.lounge.id), format!("{}#v", t.alex)),
            (t.dm.id.to_string(), t.alex.to_string()),
        ]),
        "a1's own stream audio and sam's DM call are untouched"
    );
    assert_eq!(removed_count(&t.lk), 3, "each removed once");
    assert!(t.lk.connected(&format!("{}~s", t.lounge.id), &format!("{}#a", t.alex)));
    assert!(t.lk.connected(&t.dm.id.to_string(), &t.sam.to_string()));
}

#[tokio::test]
async fn plain_logout_removes_only_that_sessions_participants() {
    let t = setup().await;
    logout(&t.app, &t.a2).await;
    assert_eq!(
        calls(&t.lk),
        set(&[
            (t.lounge.id.to_string(), t.alex.to_string()),
            (format!("{}~s", t.lounge.id), format!("{}#v", t.alex)),
        ])
    );
    logout(&t.app, &t.a3).await;
    assert_eq!(removed_count(&t.lk), 3);
    assert!(
        calls(&t.lk).contains(&(t.dm.id.to_string(), t.alex.to_string())),
        "a3's DM call goes when a3 signs out"
    );
}

/// The join webhook never arrived (lost or late): LiveKit's own participant list still shows
/// who joined with the revoked session's grant, so the revoke removes it anyway.
#[tokio::test]
async fn a_participant_whose_join_webhook_was_lost_is_still_removed() {
    let lk = fake_livekit().await;
    let app = app_with_livekit(&lk.url).await;
    let (alex, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let l = lounge(&app, &a1).await;
    let jwt = voice_token(&app, &a2, &l).await;
    lk.connect(
        &l.id.to_string(),
        &alex.to_string(),
        "PA_a2",
        Some(&grant_of(&app, &jwt)),
    );
    assert_eq!(logout_others(&app, &a1).await, 1);
    assert_eq!(calls(&lk), set(&[(l.id.to_string(), alex.to_string())]));
}

/// The review case: another device merely fetching a token (hover prefetch) for the room the
/// caller is in must not get the caller removed when that device is signed out.
#[tokio::test]
async fn a_prefetch_by_the_revoked_device_does_not_remove_the_caller() {
    let lk = fake_livekit().await;
    let app = app_with_livekit(&lk.url).await;
    let (_, a) = register(&app, "alex").await;
    let b = login(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let mine = voice_token(&app, &a, &l).await;
    connect(&app, Some(&lk), &mine, "PA_a").await;
    voice_token(&app, &b, &l).await; // B hovers Lounge: a token, but never used
    assert_eq!(logout_others(&app, &a).await, 1);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(removed_count(&lk), 0, "{:?}", calls(&lk));
}

/// Device B joined, then the caller's device A joined the same room (LiveKit drops B as a
/// duplicate identity): the participant now in the room is A's, so revoking B removes nothing.
#[tokio::test]
async fn the_grant_a_participant_joined_with_decides_not_the_latest_token() {
    let lk = fake_livekit().await;
    let app = app_with_livekit(&lk.url).await;
    let (_, a) = register(&app, "alex").await;
    let b = login(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let theirs = voice_token(&app, &b, &l).await;
    connect(&app, Some(&lk), &theirs, "PA_b").await;
    let mine = voice_token(&app, &a, &l).await;
    connect(&app, Some(&lk), &mine, "PA_a").await;
    voice_token(&app, &b, &l).await; // and B fetches another, unused
    assert_eq!(logout_others(&app, &a).await, 1);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(removed_count(&lk), 0, "{:?}", calls(&lk));
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
    connect(&app, None, &t2, "PA_2").await;
    let t3 = voice_token(&app, &a3, &l).await;
    connect(&app, None, &t3, "PA_3").await;
    assert_eq!(logout_others(&app, &a1).await, 2);
    assert_eq!(session::authenticate(&app.db, &a2).await.unwrap(), None);
    assert_eq!(session::authenticate(&app.db, &a3).await.unwrap(), None);
    // and the caller's own plain logout still goes through while in a call
    let t1 = voice_token(&app, &a1, &l).await;
    connect(&app, None, &t1, "PA_1").await;
    logout(&app, &a1).await;
    assert_eq!(session::authenticate(&app.db, &a1).await.unwrap(), None);
}

#[tokio::test]
async fn a_hanging_livekit_delays_the_logout_only_briefly() {
    let app = app_with_livekit(&fake_livekit_hanging().await).await;
    let (_, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let l = lounge(&app, &a1).await;
    let voice = voice_token(&app, &a2, &l).await;
    connect(&app, None, &voice, "PA_2").await;
    let video = stream_ticket(&app, &a2, &l, "v").await;
    connect(&app, None, &video, "PA_2v").await;
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
    let lk = fake_livekit().await;
    let app = app_with_livekit(&lk.url).await;
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
    let lk = fake_livekit().await;
    let app = app_with_livekit(&lk.url).await;
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
    t.lk.removed.lock().unwrap().clear();

    connect(&t.app, Some(&t.lk), &t.a3_dm, "PA_back").await;
    connect(&t.app, Some(&t.lk), &t.a2_video, "PA_back_v").await;
    wait_for(&t.lk, 2).await;
    assert_eq!(
        calls(&t.lk),
        set(&[
            (t.dm.id.to_string(), t.alex.to_string()),
            (format!("{}~s", t.lounge.id), format!("{}#v", t.alex)),
        ])
    );
    let _ = (&t.a2, &t.a3);
}

/// The background removal of a refused join checks LiveKit first: if a legitimate device has
/// taken the identity over in the meantime, it is left alone.
#[tokio::test]
async fn a_refused_join_is_not_removed_once_a_valid_device_took_the_identity() {
    let t = setup().await;
    logout_others(&t.app, &t.a1).await;
    t.lk.removed.lock().unwrap().clear();

    // The caller's device is now the one in the DM call (as LiveKit sees it) when the late
    // webhook for the revoked device's rejoin arrives.
    let mine = voice_token(&t.app, &t.a1, &t.dm).await;
    t.lk.connect(
        &t.dm.id.to_string(),
        &t.alex.to_string(),
        "PA_a1",
        Some(&grant_of(&t.app, &mine)),
    );
    webhook(
        &t.app,
        "participant_joined",
        &t.dm.id.to_string(),
        &t.alex.to_string(),
        Some(&grant_of(&t.app, &t.a3_dm)),
        "PA_back",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(removed_count(&t.lk), 0, "{:?}", calls(&t.lk));
    assert!(t.lk.connected(&t.dm.id.to_string(), &t.alex.to_string()));
}

#[tokio::test]
async fn joins_backed_by_a_live_grant_are_left_alone() {
    let t = setup().await;
    logout_others(&t.app, &t.a1).await;
    t.lk.removed.lock().unwrap().clear();

    // The caller joins Lounge voice from this device, and LiveKit later does a full reconnect
    // of it (new participant, refreshed token with the same attributes).
    let mine = voice_token(&t.app, &t.a1, &t.lounge).await;
    connect(&t.app, Some(&t.lk), &mine, "PA_a1").await;
    connect(&t.app, Some(&t.lk), &mine, "PA_a1_again").await;

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(removed_count(&t.lk), 0, "{:?}", calls(&t.lk));
    assert!(t.app.voice_members(t.lounge.id).contains(&t.alex));
    assert!(t.app.voice_members(t.dm.id).contains(&t.sam));
}

#[tokio::test]
async fn a_live_grant_used_for_another_room_or_identity_is_removed() {
    let lk = fake_livekit().await;
    let app = app_with_livekit(&lk.url).await;
    let (alex, a) = register(&app, "alex").await;
    let (sam, _) = register(&app, "sam").await;
    let l = lounge(&app, &a).await;
    let dm = create_dm(&app, &a, &[sam]).await;
    let grant = grant_of(&app, &voice_token(&app, &a, &l).await);
    let (lounge_room, dm_room) = (l.id.to_string(), dm.id.to_string());
    let (alex_id, video, sam_id) = (alex.to_string(), format!("{alex}#v"), sam.to_string());
    let stream_room = format!("{}~s", l.id);
    let cases = [
        (&dm_room, &alex_id),    // another room
        (&stream_room, &video),  // another identity
        (&lounge_room, &sam_id), // another user
    ];
    for (i, (room, identity)) in cases.iter().enumerate() {
        connect_raw(
            &app,
            Some(&lk),
            room,
            identity,
            Some(&grant),
            &format!("PA_{i}"),
        )
        .await;
    }
    wait_for(&lk, 3).await;
    assert_eq!(removed_count(&lk), 3, "{:?}", calls(&lk));
    assert!(!app.voice_members(dm.id).contains(&alex));
    assert!(!app.voice_members(l.id).contains(&sam));
}

#[tokio::test]
async fn a_join_without_a_known_grant_is_removed() {
    // e.g. a token minted before grants existed, or a made-up id
    let lk = fake_livekit().await;
    let app = app_with_livekit(&lk.url).await;
    let (alex, a) = register(&app, "alex").await;
    let (sam, _) = register(&app, "sam").await;
    let l = lounge(&app, &a).await;
    let dm = create_dm(&app, &a, &[sam]).await;
    connect_raw(
        &app,
        Some(&lk),
        &l.id.to_string(),
        &alex.to_string(),
        None,
        "PA_x",
    )
    .await;
    connect_raw(
        &app,
        Some(&lk),
        &dm.id.to_string(),
        &alex.to_string(),
        Some("made-up"),
        "PA_y",
    )
    .await;
    wait_for(&lk, 2).await;
    assert_eq!(removed_count(&lk), 2);
    assert!(!app.voice_members(l.id).contains(&alex));
    let open: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_sessions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(open, 0, "no voice-hours row for a refused join");
}

#[tokio::test]
async fn a_join_after_the_session_expired_is_removed() {
    let lk = fake_livekit().await;
    let app = app_with_livekit(&lk.url).await;
    let (alex, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let jwt = voice_token(&app, &a, &l).await;
    sqlx::query("UPDATE sessions SET expires_at = '2020-01-01T00:00:00.000Z'")
        .execute(&app.db)
        .await
        .unwrap();
    connect(&app, Some(&lk), &jwt, "PA_x").await;
    wait_for(&lk, 1).await;
    assert_eq!(removed_count(&lk), 1);
    assert!(!app.voice_members(l.id).contains(&alex));
}

async fn grant_ids(app: &TestApp) -> BTreeSet<String> {
    sqlx::query_scalar::<_, String>("SELECT id FROM livekit_grants")
        .fetch_all(&app.db)
        .await
        .unwrap()
        .into_iter()
        .collect()
}

async fn age_grant(app: &TestApp, id: &str, minutes: i64) {
    let at = (chrono::Utc::now() - chrono::Duration::minutes(minutes))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    sqlx::query("UPDATE livekit_grants SET issued_at = ? WHERE id = ?")
        .bind(at)
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
}

/// Grants whose participant left more than 15 min ago, and grants of expired sessions, are
/// pruned by any user's next token request. Never-joined grants of a live session are kept,
/// however old (a lost join webhook would make a connected one look never-joined).
#[tokio::test]
async fn unused_and_finished_grants_are_pruned_globally() {
    let lk = fake_livekit().await;
    let app = app_with_livekit(&lk.url).await;
    let (_, a) = register(&app, "alex").await;
    let (_, s) = register(&app, "sam").await;
    let (jo_id, jo) = register(&app, "jo").await;
    let l = lounge(&app, &a).await;
    let jl = lounge(&app, &jo).await;
    let expired = grant_of(&app, &voice_token(&app, &jo, &jl).await);
    sqlx::query("UPDATE sessions SET expires_at = '2026-01-01T00:00:00.000Z' WHERE user_id = ?")
        .bind(jo_id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    let unused_old = grant_of(&app, &voice_token(&app, &a, &l).await);
    let unused_new = grant_of(&app, &voice_token(&app, &a, &l).await);
    let in_call = voice_token(&app, &a, &l).await;
    connect(&app, Some(&lk), &in_call, "PA_call").await;
    let left = stream_ticket(&app, &a, &l, "v").await;
    connect(&app, Some(&lk), &left, "PA_left").await;
    let (room, identity) = names(&app, &left);
    webhook(
        &app,
        "participant_left",
        &room,
        &identity,
        Some(&grant_of(&app, &left)),
        "PA_left",
    )
    .await;
    // a full reconnect's late leave for the old connection doesn't finish the grant
    let (vroom, videntity) = names(&app, &in_call);
    connect(&app, Some(&lk), &in_call, "PA_call2").await;
    webhook(
        &app,
        "participant_left",
        &vroom,
        &videntity,
        Some(&grant_of(&app, &in_call)),
        "PA_call",
    )
    .await;
    let (in_call, left) = (grant_of(&app, &in_call), grant_of(&app, &left));
    let left_at = |id: String| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT left_at FROM livekit_grants WHERE id = ?",
            )
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    assert!(
        left_at(left.clone()).await.is_some(),
        "its participant left"
    );
    assert!(
        left_at(in_call.clone()).await.is_none(),
        "still connected (PA_call2)"
    );
    for g in [&unused_old, &in_call, &left] {
        age_grant(&app, g, 20).await;
    }
    sqlx::query("UPDATE livekit_grants SET left_at = ? WHERE id = ?")
        .bind("2026-01-01T00:00:00.000Z")
        .bind(&left)
        .execute(&app.db)
        .await
        .unwrap();
    age_grant(&app, &unused_new, 5).await;

    // sam's request (another user) prunes alex's finished one and jo's expired one
    let sl = lounge(&app, &s).await;
    let sams = grant_of(&app, &voice_token(&app, &s, &sl).await);
    let got = grant_ids(&app).await;
    assert!(!got.contains(&left) && !got.contains(&expired), "{got:?}");
    assert_eq!(
        got,
        [unused_old, unused_new, in_call, sams]
            .into_iter()
            .collect()
    );
}

/// Never-joined grants are capped per session (the newest are kept) instead of aged out.
#[tokio::test]
async fn never_joined_grants_are_capped_per_session() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (_, s) = register(&app, "sam").await;
    let l = lounge(&app, &a).await;
    let sl = lounge(&app, &s).await;
    let sams = grant_of(&app, &voice_token(&app, &s, &sl).await);
    let cap = session::UNJOINED_GRANTS_KEPT;
    let mut alex = Vec::new();
    for _ in 0..cap + 5 {
        alex.push(grant_of(&app, &voice_token(&app, &a, &l).await));
    }
    let got = grant_ids(&app).await;
    assert_eq!(got.len(), cap + 1, "alex's newest {cap} plus sam's");
    assert!(
        got.contains(&sams),
        "another session's grants aren't touched"
    );
    for (i, g) in alex.iter().enumerate() {
        assert_eq!(got.contains(g), i >= 5, "grant #{i}");
    }
}

/// The join webhook was lost (say, the server was restarting), and much later the session is
/// revoked: the grant is still known, so LiveKit's list still shows who to remove.
#[tokio::test]
async fn a_lost_join_webhook_participant_is_removed_by_a_much_later_revoke() {
    let lk = fake_livekit().await;
    let app = app_with_livekit(&lk.url).await;
    let (alex, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let l = lounge(&app, &a1).await;
    let jwt = voice_token(&app, &a2, &l).await;
    let grant = grant_of(&app, &jwt);
    lk.connect(&l.id.to_string(), &alex.to_string(), "PA_a2", Some(&grant));
    age_grant(&app, &grant, 120).await;
    let (_, s) = register(&app, "sam").await;
    let sl = lounge(&app, &s).await;
    voice_token(&app, &s, &sl).await; // prunes whatever is prunable
    assert_eq!(logout_others(&app, &a1).await, 1);
    assert_eq!(calls(&lk), set(&[(l.id.to_string(), alex.to_string())]));
}

/// A device whose join webhook was lost is still admitted at its next full reconnect, long
/// after its token's own lifetime (LiveKit refreshed it, keeping the grant).
#[tokio::test]
async fn a_lost_join_webhook_device_survives_its_next_full_reconnect() {
    let lk = fake_livekit().await;
    let app = app_with_livekit(&lk.url).await;
    let (alex, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let jwt = voice_token(&app, &a, &l).await;
    let grant = grant_of(&app, &jwt);
    lk.connect(&l.id.to_string(), &alex.to_string(), "PA_1", Some(&grant));
    age_grant(&app, &grant, 120).await;
    let (_, s) = register(&app, "sam").await;
    let sl = lounge(&app, &s).await;
    voice_token(&app, &s, &sl).await; // prunes whatever is prunable
    connect(&app, Some(&lk), &jwt, "PA_2").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(removed_count(&lk), 0, "{:?}", calls(&lk));
    assert!(app.voice_members(l.id).contains(&alex));
}

async fn remove_member(app: &TestApp, token: &str, group: &Channel, user: UserId) {
    let r = app
        .http
        .delete(app.url(&format!("/channels/{}/members/{user}", group.id)))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
}

/// Removing someone from a group ends their part in its call: LiveKit is asked to remove them,
/// they leave the voice state, and the token they joined with is refused if replayed.
#[tokio::test]
async fn removing_a_group_member_ends_their_part_in_its_call() {
    let lk = fake_livekit().await;
    let app = app_with_livekit(&lk.url).await;
    let (_, a) = register(&app, "alex").await;
    let (sam, s) = register(&app, "sam").await;
    let (riley, r) = register(&app, "riley").await;
    let group = create_dm(&app, &a, &[sam, riley]).await;
    let sams = voice_token(&app, &s, &group).await;
    connect(&app, Some(&lk), &sams, "PA_sam").await;
    let rileys = voice_token(&app, &r, &group).await;
    connect(&app, Some(&lk), &rileys, "PA_riley").await;
    assert!(app.voice_members(group.id).contains(&riley));
    assert_eq!(removed_count(&lk), 0);

    remove_member(&app, &a, &group, riley).await;
    assert_eq!(
        calls(&lk),
        set(&[(group.id.to_string(), riley.to_string())])
    );
    assert!(!app.voice_members(group.id).contains(&riley));
    assert!(app.voice_members(group.id).contains(&sam), "sam stays");
    assert!(lk.connected(&group.id.to_string(), &sam.to_string()));

    // LiveKit doesn't revoke the JWT: rejoining with it is removed again.
    connect(&app, Some(&lk), &rileys, "PA_riley_back").await;
    wait_for(&lk, 2).await;
    assert_eq!(removed_count(&lk), 2, "{:?}", calls(&lk));
    assert!(!lk.connected(&group.id.to_string(), &riley.to_string()));
    assert!(!app.voice_members(group.id).contains(&riley));
}

/// A join is checked against current access, not just the grant: someone who lost access to the
/// channel after getting their token is removed.
#[tokio::test]
async fn a_join_by_someone_who_lost_access_is_removed() {
    let lk = fake_livekit().await;
    let app = app_with_livekit(&lk.url).await;
    let (_, a) = register(&app, "alex").await;
    let (sam, _) = register(&app, "sam").await;
    let (riley, r) = register(&app, "riley").await;
    let group = create_dm(&app, &a, &[sam, riley]).await;
    let rileys = voice_token(&app, &r, &group).await;
    // access gone, the grant still on record
    sqlx::query("DELETE FROM channel_members WHERE channel_id = ? AND user_id = ?")
        .bind(group.id.to_string())
        .bind(riley.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    connect(&app, Some(&lk), &rileys, "PA_riley").await;
    wait_for(&lk, 1).await;
    assert_eq!(
        calls(&lk),
        set(&[(group.id.to_string(), riley.to_string())])
    );
    assert!(!app.voice_members(group.id).contains(&riley));
}

/// alex in Lounge voice, with a stream video and a stream audio connection; returns the voice
/// JWT and the video ticket.
async fn streaming(app: &TestApp, lk: &Lk, a: &str, l: &Channel) -> (String, String) {
    let voice = voice_token(app, a, l).await;
    connect(app, Some(lk), &voice, "PA_voice").await;
    let video = stream_ticket(app, a, l, "v").await;
    connect(app, Some(lk), &video, "PA_v").await;
    let audio = stream_ticket(app, a, l, "a").await;
    connect(app, Some(lk), &audio, "PA_a").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(removed_count(lk), 0, "{:?}", calls(lk));
    (voice, video)
}

/// Leaving voice ends stream-room access: both stream identities are removed, and rejoining the
/// stream room with the old ticket is removed again.
#[tokio::test]
async fn leaving_voice_ends_stream_room_access() {
    let lk = fake_livekit().await;
    let app = app_with_livekit(&lk.url).await;
    let (alex, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let (voice, video) = streaming(&app, &lk, &a, &l).await;
    let stream_room = format!("{}~s", l.id);

    webhook(
        &app,
        "participant_left",
        &l.id.to_string(),
        &alex.to_string(),
        Some(&grant_of(&app, &voice)),
        "PA_voice",
    )
    .await;
    wait_for(&lk, 2).await;
    assert_eq!(
        calls(&lk),
        set(&[
            (stream_room.clone(), format!("{alex}#v")),
            (stream_room.clone(), format!("{alex}#a")),
        ])
    );

    // a voice token fetched but not used (a hover prefetch) doesn't count as being in the call
    voice_token(&app, &a, &l).await;
    connect(&app, Some(&lk), &video, "PA_v_back").await;
    wait_for(&lk, 3).await;
    assert_eq!(removed_count(&lk), 3, "{:?}", calls(&lk));
    assert!(!lk.connected(&stream_room, &format!("{alex}#v")));
}

/// A voice full reconnect whose old connection's leave arrives late (after the new join) is not
/// leaving voice: the stream connections stay.
#[tokio::test]
async fn a_voice_reconnect_keeps_the_stream_connections() {
    let lk = fake_livekit().await;
    let app = app_with_livekit(&lk.url).await;
    let (alex, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let (voice, _) = streaming(&app, &lk, &a, &l).await;
    connect(&app, Some(&lk), &voice, "PA_voice2").await;
    webhook(
        &app,
        "participant_left",
        &l.id.to_string(),
        &alex.to_string(),
        Some(&grant_of(&app, &voice)),
        "PA_voice",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(removed_count(&lk), 0, "{:?}", calls(&lk));
    assert!(app.voice_members(l.id).contains(&alex));
}

/// The stream-room join can beat the voice state (a restart before presence is rebuilt from
/// LiveKit): a live, joined voice grant for the channel is enough.
#[tokio::test]
async fn a_stream_join_is_admitted_while_the_voice_grant_is_connected() {
    let lk = fake_livekit().await;
    let app = app_with_livekit(&lk.url).await;
    let (alex, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let (_, video) = streaming(&app, &lk, &a, &l).await;
    app.voice.leave(l.id, alex, ""); // voice state not rebuilt yet
    connect(&app, Some(&lk), &video, "PA_v2").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(removed_count(&lk), 0, "{:?}", calls(&lk));
}

/// Voice tokens carry the microphone only: no data messages.
#[tokio::test]
async fn voice_tokens_cannot_publish_data() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (sam, _) = register(&app, "sam").await;
    let l = lounge(&app, &a).await;
    let dm = create_dm(&app, &a, &[sam]).await;
    for ch in [&l, &dm] {
        let jwt = voice_token(&app, &a, ch).await;
        let claims = TokenVerifier::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
            .verify(&jwt)
            .unwrap();
        assert_eq!(claims.video.can_publish_data, Some(false));
        assert!(
            jwt_payload(&jwt).contains(r#""canPublishData":false"#),
            "{}",
            jwt_payload(&jwt)
        );
    }
}
