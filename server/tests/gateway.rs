mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, MessageId, ServerId};
use pulse_protocol::rest::{Message, MessageKind};
use pulse_server::gateway::audience::{Audience, audience_for};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(app: &TestApp) -> Socket {
    tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap()
        .0
}

async fn hello(app: &TestApp, token: &str) -> (Socket, pulse_protocol::gateway::Ready) {
    let mut ws = connect(app).await;
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    match next_frame(&mut ws).await {
        Some(ServerFrame::Ready(r)) => (ws, r),
        other => panic!("expected Ready, got {other:?}"),
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

async fn next_event(ws: &mut Socket) -> Event {
    loop {
        match next_frame(ws).await.expect("socket ended") {
            ServerFrame::Event(e) => return e,
            _ => continue,
        }
    }
}

fn msg(channel: ChannelId, content: &str) -> Message {
    Message {
        id: MessageId::new(),
        channel_id: channel,
        author_id: None,
        kind: MessageKind::Normal,
        content: content.into(),
        reply_to_id: None,
        created_at: "2026-10-01T00:00:00.000Z".into(),
        edited_at: None,
        deleted: false,
    }
}

// ---------- audience_for: table-driven ----------

#[tokio::test]
async fn audience_table() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let (c_id, _) = register(&app, "jo").await;
    let s = create_server(&app, &a, "Main").await;
    let text = general(&app, &a, s.id).await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    let group = create_dm(&app, &a, &[b_id, c_id]).await;
    let set = |ids: &[pulse_protocol::ids::UserId]| Audience::Users(ids.iter().copied().collect());

    let cases: Vec<(&str, Event, Audience)> = vec![
        (
            "text message",
            Event::MessageCreated {
                message: msg(text.id, "hi"),
                nonce: None,
            },
            Audience::Everyone,
        ),
        (
            "dm message",
            Event::MessageCreated {
                message: msg(dm.id, "hi"),
                nonce: None,
            },
            set(&[a_id, b_id]),
        ),
        (
            "dm typing",
            Event::Typing {
                channel_id: dm.id,
                user_id: a_id,
            },
            set(&[a_id, b_id]),
        ),
        (
            "dm delete",
            Event::MessageDeleted {
                channel_id: dm.id,
                message_id: MessageId::new(),
            },
            set(&[a_id, b_id]),
        ),
        (
            "group message",
            Event::MessageUpdated {
                message: msg(group.id, "hi"),
            },
            set(&[a_id, b_id, c_id]),
        ),
        (
            "dm created",
            Event::ChannelCreated {
                channel: dm.clone(),
            },
            set(&[a_id, b_id]),
        ),
        (
            "dm call",
            Event::VoiceJoined {
                channel_id: dm.id,
                user_id: a_id,
            },
            set(&[a_id, b_id]),
        ),
        (
            "server created",
            Event::ServerCreated { server: s.clone() },
            Audience::Everyone,
        ),
        (
            "unknown channel",
            Event::Typing {
                channel_id: ChannelId::new(),
                user_id: a_id,
            },
            Audience::Users(Default::default()),
        ),
    ];
    for (name, event, want) in cases {
        assert_eq!(audience_for(&app.db, &event).await.unwrap(), want, "{name}");
    }
    let _ = ServerId::new();
}

// ---------- socket protocol ----------

#[tokio::test]
async fn hello_then_ready_contains_my_servers_and_dms() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let (_, c) = register(&app, "jo").await;
    let s = create_server(&app, &a, "Main").await;
    let dm = create_dm(&app, &a, &[b_id]).await;

    let (_ws, ready) = hello(&app, &a).await;
    assert_eq!(ready.me.id, a_id);
    assert_eq!(ready.servers.len(), 1);
    assert!(ready.channels.iter().any(|ch| ch.id == dm.id));
    assert!(
        ready
            .channels
            .iter()
            .filter(|ch| ch.server_id == Some(s.id))
            .count()
            >= 2
    );
    assert!(
        ready
            .dm_members
            .iter()
            .any(|d| d.channel_id == dm.id && d.user_ids.contains(&b_id))
    );

    let (_ws, ready_c) = hello(&app, &c).await;
    assert!(
        !ready_c.channels.iter().any(|ch| ch.id == dm.id),
        "outsider must not see the dm in Ready"
    );
    assert!(ready_c.dm_members.is_empty());
}

#[tokio::test]
async fn bad_token_closed_4001() {
    let app = spawn().await;
    let mut ws = connect(&app).await;
    ws.send(Ws::text(r#"{"op":"Hello","d":{"token":"garbage"}}"#))
        .await
        .unwrap();
    let close = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    match close {
        Ws::Close(Some(f)) => assert_eq!(f.code, CloseCode::Library(4001)),
        other => panic!("expected close 4001, got {other:?}"),
    }
}

#[tokio::test]
async fn no_hello_closed_after_timeout() {
    let app = spawn().await;
    let mut ws = connect(&app).await;
    let close = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    match close {
        Ws::Close(Some(f)) => assert_eq!(f.code, CloseCode::Library(4001)),
        other => panic!("expected close 4001, got {other:?}"),
    }
}

#[tokio::test]
async fn heartbeat_acked() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (mut ws, _) = hello(&app, &a).await;
    ws.send(Ws::text(r#"{"op":"Heartbeat"}"#)).await.unwrap();
    assert!(matches!(
        next_frame(&mut ws).await,
        Some(ServerFrame::HeartbeatAck)
    ));
}

#[tokio::test]
async fn message_event_reaches_server_members() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let s = create_server(&app, &a, "Main").await;
    let g = general(&app, &a, s.id).await;
    let (mut wb, _) = hello(&app, &b).await;
    let sent = send(&app, &a, g.id, "hello all").await;
    match next_event(&mut wb).await {
        Event::MessageCreated { message, .. } => assert_eq!(message, sent),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn outsider_receives_zero_private_events() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let (_, c) = register(&app, "jo").await;
    let s = create_server(&app, &a, "Main").await;
    let g = general(&app, &a, s.id).await;
    let (mut wa, _) = hello(&app, &a).await;
    let (mut wb, _) = hello(&app, &b).await;
    let (mut wc, _) = hello(&app, &c).await;

    // all the private activity
    let dm = create_dm(&app, &a, &[b_id]).await;
    let m1 = send(&app, &a, dm.id, "secret one").await;
    send(&app, &b, dm.id, "secret two").await;
    wa.send(Ws::text(
        serde_json::to_string(&ClientFrame::Typing { channel_id: dm.id }).unwrap(),
    ))
    .await
    .unwrap();
    app.http
        .patch(app.url(&format!("/messages/{}", m1.id)))
        .bearer_auth(&a)
        .json(&json!({"content": "edited"}))
        .send()
        .await
        .unwrap();
    app.http
        .delete(app.url(&format!("/messages/{}", m1.id)))
        .bearer_auth(&a)
        .send()
        .await
        .unwrap();
    // C typing into the dm must be ignored, not relayed
    wc.send(Ws::text(
        serde_json::to_string(&ClientFrame::Typing { channel_id: dm.id }).unwrap(),
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // public marker
    let marker = send(&app, &a, g.id, "marker").await;

    let mut seen = vec![];
    loop {
        let e = next_event(&mut wc).await;
        let is_marker =
            matches!(&e, Event::MessageCreated { message, .. } if message.id == marker.id);
        seen.push(e);
        if is_marker {
            break;
        }
    }
    assert_eq!(seen.len(), 1, "outsider saw private events: {seen:?}");

    // and B did get the private stream (sanity: the test can see private events at all)
    let mut b_saw_dm = false;
    for _ in 0..10 {
        if let Event::MessageCreated { message, .. } = next_event(&mut wb).await
            && message.channel_id == dm.id
        {
            b_saw_dm = true;
            break;
        }
    }
    assert!(b_saw_dm);
}

#[tokio::test]
async fn slow_client_does_not_block_others() {
    let app = spawn().await;
    let (slow_id, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let s = create_server(&app, &a, "Main").await;
    let g = general(&app, &a, s.id).await;
    let (_slow, _) = hello(&app, &a).await; // never read again
    let (mut fast, _) = hello(&app, &b).await;

    let reader = tokio::spawn(async move {
        loop {
            if let Event::MessageCreated { message, .. } = next_event(&mut fast).await
                && message.content.starts_with("last")
            {
                return;
            }
        }
    });
    let big = "x".repeat(4000);
    for _ in 0..5000 {
        app.hub
            .publish(
                &app.db,
                Event::MessageCreated {
                    message: msg(g.id, &big),
                    nonce: None,
                },
            )
            .await;
    }
    app.hub
        .publish(
            &app.db,
            Event::MessageCreated {
                message: msg(g.id, "last"),
                nonce: None,
            },
        )
        .await;
    tokio::time::timeout(Duration::from_secs(5), reader)
        .await
        .expect("fast client starved")
        .unwrap();
    assert_eq!(
        app.hub.connections_for(slow_id),
        0,
        "slow client should have been dropped"
    );
}
