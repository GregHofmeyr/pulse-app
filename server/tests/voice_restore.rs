//! Voice presence after a restart: rebuilt from LiveKit's room list (a fake RoomService here,
//! speaking LiveKit's Twirp protocol: POST /twirp/livekit.RoomService/<Method>, protobuf bodies).

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use common::*;
use livekit_protocol as lkp;
use prost::Message as _;
use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::Channel;
use pulse_server::voice::restore;
use pulse_server::voice::state::Left;

const BUDGET: Duration = Duration::from_secs(2);
/// 2026-01-01T00:00:00Z
const JAN_1: i64 = 1_767_225_600;

type Rooms = Vec<(String, Vec<lkp::ParticipantInfo>)>;

/// A fake RoomService on 127.0.0.1:<free port>; returns its base URL.
async fn fake_livekit(rooms: Rooms) -> String {
    fake_livekit_logged(rooms).await.0
}

/// [`fake_livekit`], also returning the log of participants it was asked to remove.
async fn fake_livekit_logged(rooms: Rooms) -> (String, Removed) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let removed = serve_fake(listener, rooms);
    (format!("http://{addr}"), removed)
}

/// A RoomService that accepts the connection and never answers (overloaded or mid-restart).
async fn fake_livekit_hanging() -> String {
    let app = axum::Router::new().route(
        "/twirp/livekit.RoomService/ListRooms",
        post(|| async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Vec::<u8>::new()
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// LiveKit removals the fake saw, as (room, identity).
type Removed = Arc<Mutex<Vec<(String, String)>>>;

/// Serve the fake RoomService on an already-bound listener (lets a test bring LiveKit up late).
/// Also answers GetParticipant / RemoveParticipant (restore removes participants whose grant
/// isn't a live session's); returns the removal log.
fn serve_fake(listener: tokio::net::TcpListener, rooms: Rooms) -> Removed {
    let rooms = Arc::new(Mutex::new(rooms));
    let removed = Removed::default();
    let (r1, r2, r3, r4, log) = (
        rooms.clone(),
        rooms.clone(),
        rooms.clone(),
        rooms,
        removed.clone(),
    );
    let app = axum::Router::new()
        .route(
            "/twirp/livekit.RoomService/ListRooms",
            post(move || {
                let rooms = r1.clone();
                async move {
                    lkp::ListRoomsResponse {
                        rooms: rooms
                            .lock()
                            .unwrap()
                            .iter()
                            .map(|(name, _)| lkp::Room {
                                name: name.clone(),
                                ..Default::default()
                            })
                            .collect(),
                    }
                    .encode_to_vec()
                }
            }),
        )
        .route(
            "/twirp/livekit.RoomService/ListParticipants",
            post(move |body: Bytes| {
                let rooms = r2.clone();
                async move {
                    let req = lkp::ListParticipantsRequest::decode(body).unwrap();
                    let participants = rooms
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|(name, _)| *name == req.room)
                        .map(|(_, p)| p.clone())
                        .unwrap_or_default();
                    lkp::ListParticipantsResponse { participants }.encode_to_vec()
                }
            }),
        )
        .route(
            "/twirp/livekit.RoomService/GetParticipant",
            post(move |body: Bytes| {
                let rooms = r3.clone();
                async move {
                    let req = lkp::RoomParticipantIdentity::decode(body).unwrap();
                    let found = rooms
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|(name, _)| *name == req.room)
                        .flat_map(|(_, ps)| ps.iter())
                        .find(|p| p.identity == req.identity)
                        .cloned();
                    match found {
                        Some(p) => p.encode_to_vec().into_response(),
                        None => twirp_not_found(),
                    }
                }
            }),
        )
        .route(
            "/twirp/livekit.RoomService/RemoveParticipant",
            post(move |body: Bytes| {
                let (rooms, log) = (r4.clone(), log.clone());
                async move {
                    let req = lkp::RoomParticipantIdentity::decode(body).unwrap();
                    let mut hit = false;
                    for (name, ps) in rooms.lock().unwrap().iter_mut() {
                        if *name == req.room {
                            let before = ps.len();
                            ps.retain(|p| p.identity != req.identity);
                            hit |= ps.len() != before;
                        }
                    }
                    if !hit {
                        return twirp_not_found();
                    }
                    log.lock().unwrap().push((req.room, req.identity));
                    lkp::RemoveParticipantResponse {}
                        .encode_to_vec()
                        .into_response()
                }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    removed
}

fn twirp_not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"code":"not_found","msg":"participant not found"}"#,
    )
        .into_response()
}

fn participant(user: UserId, sid: &str) -> lkp::ParticipantInfo {
    lkp::ParticipantInfo {
        identity: user.to_string(),
        sid: sid.into(),
        state: lkp::participant_info::State::Active as i32,
        joined_at: JAN_1,
        ..Default::default()
    }
}

/// `p` carrying `grant` as its `pulse_grant` attribute (LiveKit copies it from the token).
fn with_grant(mut p: lkp::ParticipantInfo, grant: &str) -> lkp::ParticipantInfo {
    p.attributes.insert(
        pulse_server::auth::session::GRANT_ATTRIBUTE.into(),
        grant.into(),
    );
    p
}

/// A grant's (joined_sid, left_at).
async fn grant_row(app: &TestApp, id: &str) -> (Option<String>, Option<String>) {
    sqlx::query_as("SELECT joined_sid, left_at FROM livekit_grants WHERE id = ?")
        .bind(id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// Waits (up to 2 s) for the background removals restore starts.
async fn removals(removed: &Removed, n: usize) -> Vec<(String, String)> {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let now = removed.lock().unwrap().clone();
        if now.len() >= n || std::time::Instant::now() > deadline {
            return now;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The app's state, pointed at `livekit` for API calls.
fn state_with(app: &TestApp, livekit: String) -> pulse_server::AppState {
    let mut s = app.state();
    s.cfg = Arc::new(pulse_server::config::Config {
        livekit_api_url: livekit,
        ..(*app.cfg).clone()
    });
    s
}

async fn lounge(app: &TestApp, token: &str) -> Channel {
    let s = create_server(app, token, "Main").await;
    let chans: Vec<Channel> = get_json(app, token, &format!("/servers/{}/channels", s.id)).await;
    chans
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap()
}

async fn open_session(app: &TestApp, id: &str, user: UserId, channel: ChannelId) {
    sqlx::query("INSERT INTO voice_sessions (id, user_id, channel_id, joined_at) VALUES (?, ?, ?, '2026-01-01T00:00:00.000Z')")
        .bind(id)
        .bind(user.to_string())
        .bind(channel.to_string())
        .execute(&app.db)
        .await
        .unwrap();
}

async fn session(app: &TestApp, user: UserId) -> Vec<(String, String, Option<String>)> {
    sqlx::query_as(
        "SELECT id, joined_at, left_at FROM voice_sessions WHERE user_id = ? ORDER BY joined_at",
    )
    .bind(user.to_string())
    .fetch_all(&app.db)
    .await
    .unwrap()
}

#[tokio::test]
async fn restart_mid_call_restores_presence_and_keeps_the_session_open() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    open_session(&app, "VS1", a_id, l.id).await;
    let lk = fake_livekit(vec![(l.id.to_string(), vec![participant(a_id, "PA_1")])]).await;
    let mut rx = app.hub.register(a_id, String::new()).rx;

    let n = restore::sync(&state_with(&app, lk), &pulse_server::db::now(), BUDGET)
        .await
        .unwrap();

    assert_eq!(n, 1);
    assert!(app.voice_members(l.id).contains(&a_id));
    assert_eq!(
        session(&app, a_id).await,
        vec![("VS1".into(), "2026-01-01T00:00:00.000Z".into(), None)],
        "the pre-restart session row stays open"
    );
    match rx.recv().await.unwrap() {
        ServerFrame::Event(Event::VoiceJoined {
            channel_id,
            user_id,
            ..
        }) => {
            assert_eq!((channel_id, user_id), (l.id, a_id));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn left_while_down_is_closed_and_joined_while_down_is_opened() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let l = lounge(&app, &a).await;
    open_session(&app, "VS_B", b_id, l.id).await; // sam left while the server was down
    let lk = fake_livekit(vec![(l.id.to_string(), vec![participant(a_id, "PA_1")])]).await;

    restore::sync(&state_with(&app, lk), &pulse_server::db::now(), BUDGET)
        .await
        .unwrap();

    assert_eq!(
        session(&app, b_id).await,
        vec![(
            "VS_B".into(),
            "2026-01-01T00:00:00.000Z".into(),
            Some("2026-01-01T00:00:00.000Z".into())
        )],
        "closed at zero length: we can't know when they left"
    );
    let a_rows = session(&app, a_id).await;
    assert_eq!(a_rows.len(), 1);
    assert_eq!(
        a_rows[0].1, "2026-01-01T00:00:00.000Z",
        "LiveKit's join time"
    );
    assert_eq!(a_rows[0].2, None);
    assert!(!app.voice_members(l.id).contains(&b_id));
}

#[tokio::test]
async fn foreign_rooms_identities_and_disconnected_participants_are_ignored() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let mut gone = participant(a_id, "PA_old");
    gone.state = lkp::participant_info::State::Disconnected as i32;
    let bot = lkp::ParticipantInfo {
        identity: "egress-bot".into(),
        ..Default::default()
    };
    let lk = fake_livekit(vec![
        ("not-a-channel".into(), vec![participant(a_id, "PA_x")]),
        (l.id.to_string(), vec![gone, bot]),
    ])
    .await;

    let n = restore::sync(&state_with(&app, lk), &pulse_server::db::now(), BUDGET)
        .await
        .unwrap();

    assert_eq!(n, 0);
    assert!(app.voice_members(l.id).is_empty());
}

#[tokio::test]
async fn a_webhook_join_that_beat_the_restore_wins() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    app.voice.join(l.id, a_id, "PA_new");
    let lk = fake_livekit(vec![(l.id.to_string(), vec![participant(a_id, "PA_old")])]).await;

    let n = restore::sync(&state_with(&app, lk), &pulse_server::db::now(), BUDGET)
        .await
        .unwrap();

    assert_eq!(n, 0, "already present: no second VoiceJoined");
    // The current connection is still PA_new, so a `left` for PA_old is stale.
    assert_eq!(app.voice.leave(l.id, a_id, "PA_old"), Left::Stale);
}

#[tokio::test]
async fn livekit_unreachable_is_an_error_and_changes_nothing() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    open_session(&app, "VS1", a_id, l.id).await;

    let started = std::time::Instant::now();
    let r = restore::sync(
        &state_with(&app, "http://127.0.0.1:1".into()),
        &pulse_server::db::now(),
        BUDGET,
    )
    .await;

    assert!(r.is_err());
    assert!(started.elapsed() < BUDGET + Duration::from_secs(1));
    assert!(app.voice_members(l.id).is_empty());
    assert_eq!(
        session(&app, a_id).await[0].2,
        None,
        "rows untouched by a failed sync"
    );
}

/// LiveKit takes the TCP connection but never answers: boot must not wait longer than its
/// budget, and must fall back to closing old sessions.
#[tokio::test]
async fn hanging_livekit_at_boot_is_bounded_and_falls_back() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    open_session(&app, "VS1", a_id, l.id).await;
    let s = state_with(&app, fake_livekit_hanging().await);

    let started = std::time::Instant::now();
    restore::on_startup(&s, &pulse_server::db::now())
        .await
        .unwrap();

    assert!(
        started.elapsed() < Duration::from_millis(3000),
        "boot waited {:?} for a hanging LiveKit",
        started.elapsed()
    );
    assert!(app.voice_members(l.id).is_empty());
    assert_eq!(
        session(&app, a_id).await[0].2.as_deref(),
        Some("2026-01-01T00:00:00.000Z"),
        "fallback closed the old row at zero length"
    );
}

/// Host reboot where LiveKit starts after Pulse: the background retry finds the call later and
/// tells connected clients.
#[tokio::test]
async fn livekit_up_late_is_picked_up_by_the_retry_loop() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    // Reserve a port, release it: nothing listens there until we "start LiveKit" below.
    let addr = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let s = state_with(&app, format!("http://{addr}"));
    let mut rx = app.hub.register(a_id, String::new()).rx;

    tokio::spawn(restore::retry_loop(
        s,
        pulse_server::db::now(),
        Duration::from_millis(100),
        50,
    ));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(app.voice_members(l.id).is_empty(), "nothing to find yet");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    serve_fake(
        listener,
        vec![(l.id.to_string(), vec![participant(a_id, "PA_1")])],
    );

    let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("no VoiceJoined within 2 s of LiveKit coming up")
        .unwrap();
    match frame {
        ServerFrame::Event(Event::VoiceJoined {
            channel_id,
            user_id,
            ..
        }) => {
            assert_eq!((channel_id, user_id), (l.id, a_id));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(app.voice_members(l.id).contains(&a_id));
    assert_eq!(
        session(&app, a_id).await.len(),
        1,
        "a session row was opened"
    );
}

#[tokio::test]
async fn server_boots_and_serves_when_livekit_is_down() {
    // spawn_binary points PULSE_LIVEKIT_API_URL at a closed port and waits for /health.
    let p = spawn_binary().await;
    let r = reqwest::get(p.url("/health")).await.unwrap();
    assert_eq!(r.status(), 200);
}

/// Some LiveKit builds report `joined_at` 0 (unset): that's "now", not 1970.
#[tokio::test]
async fn an_unset_livekit_join_time_is_treated_as_now() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let mut p = participant(a_id, "PA_1");
    p.joined_at = 0;
    let lk = fake_livekit(vec![(l.id.to_string(), vec![p])]).await;
    let before = pulse_server::db::now();

    restore::sync(&state_with(&app, lk), &before, BUDGET)
        .await
        .unwrap();

    let rows = session(&app, a_id).await;
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].1 >= before,
        "joined_at {} should be now-ish (>= {before})",
        rows[0].1
    );
}

/// A LiveKit identity that parses as a user id but has no `users` row (deleted account, other
/// deployment's token) is skipped; everyone else is still restored.
#[tokio::test]
async fn an_unknown_user_is_skipped_without_failing_the_restore() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let ghost = UserId::new();
    let lk = fake_livekit(vec![(
        l.id.to_string(),
        vec![participant(ghost, "PA_ghost"), participant(a_id, "PA_1")],
    )])
    .await;

    let n = restore::sync(&state_with(&app, lk), &pulse_server::db::now(), BUDGET)
        .await
        .unwrap();

    assert_eq!(n, 1);
    assert!(app.voice_members(l.id).contains(&a_id));
    assert!(!app.voice_members(l.id).contains(&ghost));
    assert_eq!(session(&app, a_id).await.len(), 1);
    assert!(session(&app, ghost).await.is_empty());
}

/// After a restart no join webhook is re-sent, so restore does the join check itself: a
/// participant carrying a `pulse_grant` that isn't a live session's grant for that room +
/// identity (here: its session was revoked while the server was down) is removed from LiveKit,
/// in voice and stream rooms, and not restored. One without the attribute is left alone.
#[tokio::test]
async fn a_restored_participant_without_a_live_grant_is_removed_not_restored() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let l = lounge(&app, &a).await;
    let room = l.id.to_string();
    let stream_room = format!("{room}~s");
    let voice_grant = grant_livekit(&app, &room, &a_id.to_string()).await;
    let video_grant = grant_livekit(&app, &stream_room, &format!("{a_id}#v")).await;
    sqlx::query("DELETE FROM sessions WHERE user_id = ?") // revoked while the server was down
        .bind(a_id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    let mut video = with_grant(participant(a_id, "PV_1"), &video_grant);
    video.identity = format!("{a_id}#v");
    let (lk, removed) = fake_livekit_logged(vec![
        (
            room.clone(),
            vec![
                with_grant(participant(a_id, "PA_1"), &voice_grant),
                participant(b_id, "PB_1"), // pre-grant token: no attribute
            ],
        ),
        (stream_room.clone(), vec![video]),
    ])
    .await;

    let n = restore::sync(&state_with(&app, lk), &pulse_server::db::now(), BUDGET)
        .await
        .unwrap();

    assert_eq!(n, 1, "only sam is restored");
    assert!(!app.voice_members(l.id).contains(&a_id));
    assert!(app.voice_members(l.id).contains(&b_id));
    assert!(
        session(&app, a_id).await.is_empty(),
        "no session row for a refused join"
    );
    let mut got = removals(&removed, 2).await;
    got.sort();
    let mut want = vec![(room, a_id.to_string()), (stream_room, format!("{a_id}#v"))];
    want.sort();
    assert_eq!(got, want, "sam (no attribute) is left alone");
}

/// A live grant is restored, and its connection is recorded as the grant's (its join webhook
/// may have been lost), so a later revoke or leave matches it.
#[tokio::test]
async fn a_restored_participant_with_a_live_grant_is_kept_and_recorded() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let grant = grant_livekit(&app, &l.id.to_string(), &a_id.to_string()).await;
    let (lk, removed) = fake_livekit_logged(vec![(
        l.id.to_string(),
        vec![with_grant(participant(a_id, "PA_1"), &grant)],
    )])
    .await;

    let n = restore::sync(&state_with(&app, lk), &pulse_server::db::now(), BUDGET)
        .await
        .unwrap();

    assert_eq!(n, 1);
    assert!(app.voice_members(l.id).contains(&a_id));
    assert_eq!(grant_row(&app, &grant).await, (Some("PA_1".into()), None));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(removed.lock().unwrap().is_empty());
}

/// A leave webhook lost while the server was down: grants whose connection LiveKit no longer
/// has are marked left (so they are pruned); a grant whose connection is still there is not.
#[tokio::test]
async fn grants_whose_connection_ended_while_down_are_marked_left() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let room = l.id.to_string();
    let gone = grant_livekit(&app, &room, &a_id.to_string()).await;
    let here = grant_livekit(&app, &room, &a_id.to_string()).await;
    for (id, sid) in [(&gone, "PA_old"), (&here, "PA_1")] {
        sqlx::query("UPDATE livekit_grants SET joined_sid = ? WHERE id = ?")
            .bind(sid)
            .bind(id)
            .execute(&app.db)
            .await
            .unwrap();
    }
    let lk = fake_livekit(vec![(
        room,
        vec![with_grant(participant(a_id, "PA_1"), &here)],
    )])
    .await;

    restore::sync(&state_with(&app, lk), &pulse_server::db::now(), BUDGET)
        .await
        .unwrap();

    let (sid, left) = grant_row(&app, &gone).await;
    assert_eq!(sid.as_deref(), Some("PA_old"));
    assert!(left.is_some(), "the lost leave is made up for");
    assert_eq!(grant_row(&app, &here).await, (Some("PA_1".into()), None));
}
