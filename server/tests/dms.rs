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
