mod common;

use base64::Engine;
use common::*;
use livekit_api::access_token::{AccessToken, TokenVerifier};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{Channel, VoiceTokenResponse};
use serde_json::json;
use sha2::{Digest, Sha256};

async fn lounge(app: &TestApp, token: &str) -> Channel {
    let s = create_server(app, token, "Main").await;
    let chans: Vec<Channel> = get_json(app, token, &format!("/servers/{}/channels", s.id)).await;
    chans
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap()
}

fn signed(app: &TestApp, body: &str) -> String {
    let sum = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body.as_bytes()));
    AccessToken::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .with_sha256(&sum)
        .to_jwt()
        .unwrap()
}

fn event_body(kind: &str, room: ChannelId, user: UserId) -> String {
    json!({"event": kind, "id": "EV_test", "room": {"name": room.to_string()}, "participant": {"identity": user.to_string()}}).to_string()
}

/// `participant_joined` as LiveKit sends it for a participant whose token carried `grant`.
fn join_body(room: ChannelId, user: UserId, grant: &str) -> String {
    json!({"event": "participant_joined", "id": "EV_test", "room": {"name": room.to_string()},
        "participant": {"identity": user.to_string(), "attributes": {"pulse_grant": grant}}})
    .to_string()
}

async fn grant_of(app: &TestApp, r: reqwest::Response) -> String {
    let t: VoiceTokenResponse = r.json().await.unwrap();
    TokenVerifier::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .verify(&t.token)
        .unwrap()
        .attributes["pulse_grant"]
        .clone()
}

async fn webhook(app: &TestApp, body: &str, auth: &str) -> reqwest::Response {
    app.http
        .post(app.url("/livekit/webhook"))
        .header("Authorization", auth)
        .header("Content-Type", "application/webhook+json")
        .body(body.to_string())
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn token_for_server_voice_channel() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let r = post_json(&app, &a, &format!("/voice/{}/token", l.id), json!({})).await;
    assert_eq!(r.status(), 200);
    let t: VoiceTokenResponse = r.json().await.unwrap();
    assert_eq!(t.url, app.cfg.livekit_url);
    let claims = TokenVerifier::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .verify(&t.token)
        .unwrap();
    assert_eq!(claims.sub, a_id.to_string());
    assert_eq!(claims.video.room, l.id.to_string());
    assert!(claims.video.room_join);
}

#[tokio::test]
async fn token_refused_for_outsider_dm_404() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let (_, c) = register(&app, "jo").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    assert_eq!(
        post_json(&app, &c, &format!("/voice/{}/token", dm.id), json!({}))
            .await
            .status(),
        404
    );
    assert_eq!(
        post_json(&app, &b, &format!("/voice/{}/token", dm.id), json!({}))
            .await
            .status(),
        200
    );
}

#[tokio::test]
async fn token_text_channel_400() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let s = create_server(&app, &a, "Main").await;
    let g = general(&app, &a, s.id).await;
    assert_eq!(
        post_json(&app, &a, &format!("/voice/{}/token", g.id), json!({}))
            .await
            .status(),
        400
    );
}

#[tokio::test]
async fn webhook_bad_signature_401() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let body = event_body("participant_joined", l.id, a_id);
    // signed for a different body
    let auth = signed(&app, "{}");
    assert_eq!(webhook(&app, &body, &auth).await.status(), 401);
    // signed with the wrong secret
    let sum = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body.as_bytes()));
    let forged =
        AccessToken::with_api_key(&app.cfg.livekit_key, "not-the-secret-not-the-secret-xx")
            .with_sha256(&sum)
            .to_jwt()
            .unwrap();
    assert_eq!(webhook(&app, &body, &forged).await.status(), 401);
    assert_eq!(webhook(&app, &body, "").await.status(), 401);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM voice_sessions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn webhook_join_server_channel_records_session_and_broadcasts() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let mut rx = app.hub.register(a_id, String::new()).rx;
    // a join needs the grant id the token route put in the token's attributes
    let t = post_json(&app, &a, &format!("/voice/{}/token", l.id), json!({})).await;
    assert_eq!(t.status(), 200);
    let body = join_body(l.id, a_id, &grant_of(&app, t).await);
    assert_eq!(
        webhook(&app, &body, &signed(&app, &body)).await.status(),
        200
    );
    let open: i64 = sqlx::query_scalar("SELECT count(*) FROM voice_sessions WHERE left_at IS NULL")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(open, 1);
    match rx.recv().await.unwrap() {
        pulse_protocol::gateway::ServerFrame::Event(Event::VoiceJoined {
            channel_id,
            user_id,
            ..
        }) => {
            assert_eq!((channel_id, user_id), (l.id, a_id));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(app.voice_members(l.id).contains(&a_id));

    let left = event_body("participant_left", l.id, a_id);
    assert_eq!(
        webhook(&app, &left, &signed(&app, &left)).await.status(),
        200
    );
    let open: i64 = sqlx::query_scalar("SELECT count(*) FROM voice_sessions WHERE left_at IS NULL")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(open, 0);
    assert!(!app.voice_members(l.id).contains(&a_id));
}

#[tokio::test]
async fn webhook_join_dm_call_writes_no_voice_session() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    // a join needs the grant id the token route put in the token's attributes
    let t = post_json(&app, &a, &format!("/voice/{}/token", dm.id), json!({})).await;
    assert_eq!(t.status(), 200);
    let body = join_body(dm.id, a_id, &grant_of(&app, t).await);
    assert_eq!(
        webhook(&app, &body, &signed(&app, &body)).await.status(),
        200
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM voice_sessions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(n, 0, "DM calls must never count toward voice stats");
}

// I-6: a participant_left the server never saw join (e.g. after a restart) still closes the open row.
#[tokio::test]
async fn webhook_left_without_memory_closes_open_session() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    sqlx::query("INSERT INTO voice_sessions (id, user_id, channel_id, joined_at) VALUES ('VS1', ?, ?, '2026-01-01T00:00:00.000Z')")
        .bind(a_id.to_string())
        .bind(l.id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    let left = event_body("participant_left", l.id, a_id);
    assert_eq!(
        webhook(&app, &left, &signed(&app, &left)).await.status(),
        200
    );
    let open: i64 = sqlx::query_scalar("SELECT count(*) FROM voice_sessions WHERE left_at IS NULL")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(open, 0);
}

/// A join that applied after its channel's delete took its snapshot of the call (#109) leaves
/// someone in the voice state of a channel that's gone. LiveKit drops them when the delete closes
/// the room; that leave must take them out of the voice state too, although the channel is gone.
#[tokio::test]
async fn a_leave_from_a_deleted_channel_still_clears_its_voice_state() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let r = app
        .http
        .delete(app.url(&format!("/channels/{}", l.id)))
        .bearer_auth(&a)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    // the racing join, applied after the delete's snapshot
    assert!(app.voice.join(l.id, a_id, "PA_a"));

    let left =
        json!({"event": "participant_left", "id": "EV_test", "room": {"name": l.id.to_string()},
        "participant": {"identity": a_id.to_string(), "sid": "PA_a"}})
        .to_string();
    assert_eq!(
        webhook(&app, &left, &signed(&app, &left)).await.status(),
        200
    );
    assert!(!app.voice_members(l.id).contains(&a_id));
}

/// A server whose held voice leaves (#281) wait a short moment.
async fn spawn_quick_reconnect() -> TestApp {
    let mut cfg = test_config();
    cfg.calls.reconnect = std::time::Duration::from_millis(400);
    spawn_with(cfg).await
}

/// `user`'s connection `sid` joining `room` with a fresh token's grant.
async fn join_as(app: &TestApp, token: &str, room: ChannelId, user: UserId, sid: &str) {
    let t = post_json(app, token, &format!("/voice/{room}/token"), json!({})).await;
    assert_eq!(t.status(), 200);
    let body =
        json!({"event": "participant_joined", "id": "EV_test", "room": {"name": room.to_string()},
        "participant": {"identity": user.to_string(), "sid": sid,
            "attributes": {"pulse_grant": grant_of(app, t).await}}})
        .to_string();
    assert_eq!(webhook(app, &body, &signed(app, &body)).await.status(), 200);
}

/// `user`'s connection `sid` leaving `room`, as LiveKit reports it with `reason`.
async fn leave_as(app: &TestApp, room: ChannelId, user: UserId, sid: &str, reason: &str) {
    let body =
        json!({"event": "participant_left", "id": "EV_test", "room": {"name": room.to_string()},
        "participant": {"identity": user.to_string(), "sid": sid, "disconnectReason": reason}})
        .to_string();
    assert_eq!(webhook(app, &body, &signed(app, &body)).await.status(), 200);
}

/// The voice joins and leaves `rx` gets within `wait`.
async fn voice_events(
    rx: &mut tokio::sync::mpsc::Receiver<pulse_protocol::gateway::ServerFrame>,
    wait: std::time::Duration,
) -> Vec<Event> {
    let mut out = Vec::new();
    let until = tokio::time::Instant::now() + wait;
    while let Ok(Some(frame)) = tokio::time::timeout_at(until, rx.recv()).await {
        if let pulse_protocol::gateway::ServerFrame::Event(
            e @ (Event::VoiceJoined { .. } | Event::VoiceLeft { .. }),
        ) = frame
        {
            out.push(e);
        }
    }
    out
}

/// #281: a connection lost to the network that comes back in time is a reconnect nobody else
/// hears: no leave, no join, one session throughout.
#[tokio::test]
async fn a_quick_reconnect_after_a_network_drop_is_silent() {
    let app = spawn_quick_reconnect().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    join_as(&app, &a, l.id, a_id, "PA_1").await;
    // what alex's other device (and everyone in the server) is told
    let mut rx = app.hub.register(a_id, String::new()).rx;

    leave_as(&app, l.id, a_id, "PA_1", "CONNECTION_TIMEOUT").await;
    assert!(
        app.voice_members(l.id).contains(&a_id),
        "still in the call while they may come back"
    );
    join_as(&app, &a, l.id, a_id, "PA_2").await;

    let heard = voice_events(&mut rx, std::time::Duration::from_millis(900)).await;
    assert!(heard.is_empty(), "nothing published: {heard:?}");
    assert!(app.voice_members(l.id).contains(&a_id));
    let open: i64 = sqlx::query_scalar("SELECT count(*) FROM voice_sessions WHERE left_at IS NULL")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let all: i64 = sqlx::query_scalar("SELECT count(*) FROM voice_sessions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!((open, all), (1, 1), "the same session carries on");
    // the replaced connection's late leave is stale, as before
    leave_as(&app, l.id, a_id, "PA_1", "DUPLICATE_IDENTITY").await;
    assert!(app.voice_members(l.id).contains(&a_id));
}

/// #281: not back in time, the held leave goes out then; leaving on purpose goes out at once.
#[tokio::test]
async fn a_network_drop_not_back_in_time_leaves_after_the_wait() {
    let app = spawn_quick_reconnect().await;
    let (a_id, a) = register(&app, "alex").await;
    let (k_id, k) = register(&app, "kim").await;
    let (s_id, _) = register(&app, "sam").await;
    let l = lounge(&app, &a).await;
    join_as(&app, &a, l.id, a_id, "PA_a").await;
    let mut rx = app.hub.register(a_id, String::new()).rx;

    leave_as(&app, l.id, a_id, "PA_a", "SIGNAL_CLOSE").await;
    assert!(app.voice_members(l.id).contains(&a_id));
    let heard = voice_events(&mut rx, std::time::Duration::from_millis(900)).await;
    assert!(
        matches!(heard.as_slice(), [Event::VoiceLeft { channel_id, user_id }] if (*channel_id, *user_id) == (l.id, a_id)),
        "one leave, after the wait: {heard:?}"
    );
    assert!(!app.voice_members(l.id).contains(&a_id));
    let open: i64 = sqlx::query_scalar("SELECT count(*) FROM voice_sessions WHERE left_at IS NULL")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(open, 0);

    // a leave on purpose (here kim leaving a DM call) goes out at once, as before
    let dm = create_dm(&app, &k, &[s_id]).await;
    join_as(&app, &k, dm.id, k_id, "PA_k").await;
    leave_as(&app, dm.id, k_id, "PA_k", "CLIENT_INITIATED").await;
    assert!(
        !app.voice_members(dm.id).contains(&k_id),
        "left on purpose: at once"
    );
}

/// #281: back from a drop in another channel, the old call is left first, so nobody is in two.
#[tokio::test]
async fn a_rejoin_elsewhere_ends_the_held_call_first() {
    let app = spawn_quick_reconnect().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let dm = create_dm(&app, &a, &[register(&app, "jo").await.0]).await;
    join_as(&app, &a, l.id, a_id, "PA_1").await;
    leave_as(&app, l.id, a_id, "PA_1", "CONNECTION_TIMEOUT").await;
    join_as(&app, &a, dm.id, a_id, "PA_2").await;
    assert!(!app.voice_members(l.id).contains(&a_id));
    assert!(app.voice_members(dm.id).contains(&a_id));
}
