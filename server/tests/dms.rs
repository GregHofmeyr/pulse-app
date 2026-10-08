//! DMs, groups, unread, mentions, mutes, presence.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_server::gateway::audience::{Audience, audience_for};
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello(app: &TestApp, token: &str) -> (Socket, Ready) {
    let mut ws = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap()
        .0;
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    loop {
        if let Some(ServerFrame::Ready(r)) = next_frame(&mut ws).await {
            return (ws, r);
        }
    }
}

async fn next_frame(ws: &mut Socket) -> Option<ServerFrame> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        match m {
            Ws::Text(t) => return Some(serde_json::from_str(&t).unwrap()),
            Ws::Close(_) => return None,
            _ => continue,
        }
    }
}

/// Next event matching `pred` (skips others); panics after 3 s.
async fn wait_for(ws: &mut Socket, pred: impl Fn(&Event) -> bool) -> Event {
    loop {
        match next_frame(ws).await.expect("socket ended") {
            ServerFrame::Event(e) if pred(&e) => return e,
            _ => continue,
        }
    }
}

#[tokio::test]
async fn owner_only_events_reach_only_their_owner() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    let e = Event::ReadStateUpdated {
        user_id: a_id,
        channel_id: dm.id,
        last_read_message_id: None,
    };
    assert_eq!(
        audience_for(&app.db, &e).await.unwrap(),
        Audience::Users([a_id].into_iter().collect()),
        "sam must never learn what alex read"
    );
}

#[tokio::test]
async fn last_seen_only_when_last_connection_closes() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let (mut watcher, _) = hello(&app, &b).await;
    let (ws1, _) = hello(&app, &a).await;
    let (ws2, _) = hello(&app, &a).await;
    // (sam's own "online" arrives first; wait for alex's)
    wait_for(
        &mut watcher,
        |e| matches!(e, Event::PresenceChanged { user_id, online: true, .. } if *user_id == a_id),
    )
    .await;
    drop(ws1);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let seen: Option<String> = sqlx::query_scalar("SELECT last_seen_at FROM users WHERE id = ?")
        .bind(a_id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(seen.is_none(), "still connected on another device");
    drop(ws2);
    let ev = wait_for(
        &mut watcher,
        |e| matches!(e, Event::PresenceChanged { user_id, online: false, .. } if *user_id == a_id),
    )
    .await;
    assert!(matches!(
        ev,
        Event::PresenceChanged {
            last_seen_at: Some(_),
            ..
        }
    ));
}

#[tokio::test]
async fn ready_lists_everyone_with_presence() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let (_ws, ready) = hello(&app, &a).await;
    let me = ready.people.iter().find(|p| p.user.id == a_id).unwrap();
    let sam = ready.people.iter().find(|p| p.user.id == b_id).unwrap();
    assert!(me.online && !sam.online);
}

async fn read_state(
    r: &Ready,
    ch: pulse_protocol::ids::ChannelId,
) -> pulse_protocol::rest::ReadState {
    r.read_states
        .iter()
        .find(|s| s.channel_id == ch)
        .cloned()
        .expect("read state present")
}

#[tokio::test]
async fn unread_counts_others_messages_since_read_point() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    send(&app, &a, dm.id, "one").await;
    send(&app, &a, dm.id, "two").await;
    send(&app, &b, dm.id, "mine").await; // sam's own message: never unread for sam, and it marks sam read
    let (_ws, ready) = hello(&app, &b).await;
    assert_eq!(
        read_state(&ready, dm.id).await.unread,
        0,
        "sending marks your own read point"
    );
    send(&app, &a, dm.id, "three").await;
    let (_ws2, ready) = hello(&app, &b).await;
    assert_eq!(read_state(&ready, dm.id).await.unread, 1);
    let _ = a_id;
}

#[tokio::test]
async fn joining_a_server_starts_all_read() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let s = create_server(&app, &a, "Main").await;
    let g = general(&app, &a, s.id).await;
    for i in 0..5 {
        send(&app, &a, g.id, &format!("old {i}")).await;
    }
    assert_eq!(
        post_json(
            &app,
            &b,
            &format!("/servers/{}/join", s.id),
            serde_json::json!({})
        )
        .await
        .status(),
        204
    );
    let (_ws, ready) = hello(&app, &b).await;
    assert_eq!(
        read_state(&ready, g.id).await.unread,
        0,
        "no wall of old unreads"
    );
}

#[tokio::test]
async fn mark_read_rejects_foreign_message_and_never_moves_back() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    let s = create_server(&app, &a, "Main").await;
    let g = general(&app, &a, s.id).await;
    let other = send(&app, &a, g.id, "elsewhere").await;
    let m1 = send(&app, &a, dm.id, "1").await;
    let m2 = send(&app, &a, dm.id, "2").await;
    let path = format!("/channels/{}/read", dm.id);
    assert_eq!(
        post_json(&app, &b, &path, serde_json::json!({"message_id": other.id}))
            .await
            .status(),
        400
    );
    assert_eq!(
        post_json(&app, &b, &path, serde_json::json!({"message_id": m2.id}))
            .await
            .status(),
        204
    );
    assert_eq!(
        post_json(&app, &b, &path, serde_json::json!({"message_id": m1.id}))
            .await
            .status(),
        204
    );
    let (_ws, ready) = hello(&app, &b).await;
    let st = read_state(&ready, dm.id).await;
    assert_eq!(st.last_read_message_id, Some(m2.id), "monotonic");
    assert_eq!(st.unread, 0);
}

#[tokio::test]
async fn mark_read_syncs_own_sessions_only() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    let m = send(&app, &a, dm.id, "hi").await;
    let (mut sam_other_device, _) = hello(&app, &b).await;
    let (mut alex, _) = hello(&app, &a).await;
    post_json(
        &app,
        &b,
        &format!("/channels/{}/read", dm.id),
        serde_json::json!({"message_id": m.id}),
    )
    .await;
    let e = wait_for(&mut sam_other_device, |e| {
        matches!(e, Event::ReadStateUpdated { .. })
    })
    .await;
    assert!(matches!(e, Event::ReadStateUpdated { user_id, .. } if user_id == b_id));
    // alex must not get it: send a marker and check it's the next thing alex sees
    send(&app, &a, dm.id, "marker").await;
    let next = wait_for(&mut alex, |e| !matches!(e, Event::PresenceChanged { .. })).await;
    assert!(
        matches!(next, Event::MessageCreated { .. }),
        "alex saw {next:?}"
    );
    let _ = a_id;
}

#[tokio::test]
async fn ready_has_latest_message_per_conversation() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    send(&app, &a, dm.id, "first").await;
    let last = send(&app, &a, dm.id, "last").await;
    let (_ws, ready) = hello(&app, &b).await;
    assert_eq!(
        ready
            .latest
            .iter()
            .find(|m| m.channel_id == dm.id)
            .map(|m| m.id),
        Some(last.id)
    );
}
