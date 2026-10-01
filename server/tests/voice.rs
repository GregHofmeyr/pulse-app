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
    let body = event_body("participant_joined", l.id, a_id);
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
    let body = event_body("participant_joined", dm.id, a_id);
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
