//! Voice presence: Ready.voice, mute/deafen broadcast, membership gate.

mod common;

use std::time::Duration;

use base64::Engine;
use common::*;
use futures::{SinkExt, StreamExt};
use livekit_api::access_token::AccessToken;
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame, VoiceFlags};
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::Channel;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello(app: &TestApp, token: &str) -> (Socket, Ready) {
    let mut ws = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap()
        .0;
    ws.send(Ws::text(
        serde_json::to_string(&ClientFrame::Hello {
            token: token.into(),
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    loop {
        if let ServerFrame::Ready(r) = next(&mut ws).await {
            return (ws, r);
        }
    }
}

async fn next(ws: &mut Socket) -> ServerFrame {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .expect("timeout")
            .unwrap()
            .unwrap();
        if let Ws::Text(t) = m {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

async fn next_event(ws: &mut Socket) -> Event {
    loop {
        if let ServerFrame::Event(e) = next(ws).await {
            return e;
        }
    }
}

fn signed(app: &TestApp, body: &str) -> String {
    let sum = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body.as_bytes()));
    AccessToken::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .with_sha256(&sum)
        .to_jwt()
        .unwrap()
}

async fn joined(app: &TestApp, room: ChannelId, user: UserId) {
    let body = json!({"event": "participant_joined", "id": "EV", "room": {"name": room.to_string()}, "participant": {"identity": user.to_string()}}).to_string();
    let r = app
        .http
        .post(app.url("/livekit/webhook"))
        .header("Authorization", signed(app, &body))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

/// A participant webhook with LiveKit's per-connection `sid` (a rejoin gets a new one).
async fn lk_event(app: &TestApp, event: &str, room: ChannelId, user: UserId, sid: &str) {
    let body = json!({"event": event, "id": "EV", "room": {"name": room.to_string()}, "participant": {"identity": user.to_string(), "sid": sid}}).to_string();
    let r = app
        .http
        .post(app.url("/livekit/webhook"))
        .header("Authorization", signed(app, &body))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

async fn voice_members(app: &TestApp, token: &str, room: ChannelId) -> Vec<UserId> {
    let (_ws, ready) = hello(app, token).await;
    ready
        .voice
        .iter()
        .find(|r| r.channel_id == room)
        .map(|r| r.members.iter().map(|m| m.user_id).collect())
        .unwrap_or_default()
}

/// Rejoining with the same identity makes LiveKit kick the old connection, firing its `left`
/// right next to the new `joined`, in either order. A stale `left` must not remove the new one.
#[tokio::test]
async fn stale_left_from_replaced_connection_keeps_user_in_voice() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (_, l) = lounge(&app, &a).await;
    lk_event(&app, "participant_joined", l.id, a_id, "PA_old").await;
    lk_event(&app, "participant_joined", l.id, a_id, "PA_new").await;
    lk_event(&app, "participant_left", l.id, a_id, "PA_old").await;
    assert_eq!(
        voice_members(&app, &a, l.id).await,
        vec![a_id],
        "still in voice"
    );
    lk_event(&app, "participant_left", l.id, a_id, "PA_new").await;
    assert!(
        voice_members(&app, &a, l.id).await.is_empty(),
        "gone once the live one leaves"
    );
}

async fn lounge(app: &TestApp, token: &str) -> (pulse_protocol::rest::Server, Channel) {
    let s = create_server(app, token, "Main").await;
    let chans: Vec<Channel> = get_json(app, token, &format!("/servers/{}/channels", s.id)).await;
    let l = chans
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    (s, l)
}

#[tokio::test]
async fn token_requires_server_membership_403() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (_, c) = register(&app, "jo").await;
    let (s, l) = lounge(&app, &a).await;
    assert_eq!(
        post_json(&app, &c, &format!("/voice/{}/token", l.id), json!({}))
            .await
            .status(),
        403
    );
    post_json(&app, &c, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(
        post_json(&app, &c, &format!("/voice/{}/token", l.id), json!({}))
            .await
            .status(),
        200
    );
}

#[tokio::test]
async fn ready_includes_voice_rooms() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let (_, l) = lounge(&app, &a).await;
    joined(&app, l.id, a_id).await;
    let (_ws, ready) = hello(&app, &b).await;
    let room = ready
        .voice
        .iter()
        .find(|r| r.channel_id == l.id)
        .expect("lounge in Ready.voice");
    assert_eq!(room.members.len(), 1);
    assert_eq!(room.members[0].user_id, a_id);
    assert_eq!(room.members[0].flags, VoiceFlags::default());
}

#[tokio::test]
async fn voice_state_frame_broadcasts_flags() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let (s, l) = lounge(&app, &a).await;
    let g = general(&app, &a, s.id).await;
    joined(&app, l.id, a_id).await;
    let (mut wa, _) = hello(&app, &a).await;
    let (mut wb, _) = hello(&app, &b).await;
    let flags = VoiceFlags {
        muted: true,
        deafened: false,
    };
    wa.send(Ws::text(
        serde_json::to_string(&ClientFrame::VoiceState { flags }).unwrap(),
    ))
    .await
    .unwrap();
    match next_event(&mut wb).await {
        Event::VoiceStateChanged {
            channel_id,
            user_id,
            flags: f,
        } => {
            assert_eq!((channel_id, user_id, f), (l.id, a_id, flags));
        }
        other => panic!("unexpected {other:?}"),
    }
    let _ = g;
}

#[tokio::test]
async fn voice_state_from_user_not_in_voice_is_ignored() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let (s, _) = lounge(&app, &a).await;
    let g = general(&app, &a, s.id).await;
    let (mut wa, _) = hello(&app, &a).await;
    let (mut wb, _) = hello(&app, &b).await;
    wa.send(Ws::text(
        serde_json::to_string(&ClientFrame::VoiceState {
            flags: VoiceFlags {
                muted: true,
                deafened: true,
            },
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let marker = send(&app, &a, g.id, "marker").await;
    match next_event(&mut wb).await {
        Event::MessageCreated { message, .. } => assert_eq!(message.id, marker.id),
        other => panic!("expected only the marker, got {other:?}"),
    }
}

#[tokio::test]
async fn ready_hides_private_voice_rooms_from_outsiders() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let (_, c) = register(&app, "jo").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    joined(&app, dm.id, a_id).await;
    let (_w, ready_c) = hello(&app, &c).await;
    assert!(
        ready_c.voice.iter().all(|r| r.channel_id != dm.id),
        "outsider sees DM call"
    );
    let (_w, ready_b) = hello(&app, &b).await;
    assert!(
        ready_b.voice.iter().any(|r| r.channel_id == dm.id),
        "member should see DM call"
    );
}

/// I5: the client announces its flags right after joining, but LiveKit's join webhook may land
/// later. The flags must stick.
#[tokio::test]
async fn flags_declared_before_join_webhook_are_kept() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let (_, l) = lounge(&app, &a).await;
    let (mut wa, _) = hello(&app, &a).await;
    wa.send(Ws::text(
        serde_json::to_string(&ClientFrame::VoiceState {
            flags: VoiceFlags {
                muted: true,
                deafened: false,
            },
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    joined(&app, l.id, a_id).await; // webhook arrives after
    let (_w, ready) = hello(&app, &b).await;
    let room = ready.voice.iter().find(|r| r.channel_id == l.id).unwrap();
    assert!(room.members[0].flags.muted, "declared mute was lost");
}

/// The mute-icon race: "I'm muted" lands before LiveKit's join webhook. The VoiceJoined everyone
/// receives must carry the remembered flags, or clients show the joiner as unmuted.
#[tokio::test]
async fn voice_joined_event_carries_declared_flags() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let (_, l) = lounge(&app, &a).await;
    let (mut wa, _) = hello(&app, &a).await;
    let (mut wb, _) = hello(&app, &b).await;
    let muted = VoiceFlags {
        muted: true,
        deafened: false,
    };
    wa.send(Ws::text(
        serde_json::to_string(&ClientFrame::VoiceState { flags: muted }).unwrap(),
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    joined(&app, l.id, a_id).await;
    match next_event(&mut wb).await {
        Event::VoiceJoined {
            channel_id,
            user_id,
            flags,
        } => assert_eq!((channel_id, user_id, flags), (l.id, a_id, muted)),
        other => panic!("unexpected {other:?}"),
    }
}
