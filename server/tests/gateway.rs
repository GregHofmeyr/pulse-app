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
        client_version: pulse_protocol::PROTOCOL_VERSION,
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
            Ws::Text(t) => {
                let f: ServerFrame = serde_json::from_str(&t).unwrap();
                // Presence/new-user events are background noise for these tests.
                if matches!(
                    f,
                    ServerFrame::Event(Event::PresenceChanged { .. } | Event::UserCreated { .. })
                ) {
                    continue;
                }
                return Some(f);
            }
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
        mentions: vec![],
        reactions: vec![],
        thread_root_id: None,
        quote: None,
        qoty_card: None,
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
                author_id: Some(a_id),
                mentions: vec![],
                thread_root_id: None,
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
                flags: Default::default(),
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
    ws.send(Ws::text(
        r#"{"op":"Hello","d":{"token":"garbage","client_version":2}}"#,
    ))
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

#[tokio::test]
async fn outdated_client_is_told_to_update() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    // a client from before versioning (no client_version at all), and the last build before
    // quotes channels (version 1, which can't parse their kind)
    for d in [
        json!({ "token": token }),
        json!({ "token": token, "client_version": 1 }),
    ] {
        let mut ws = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
            .await
            .unwrap()
            .0;
        let hello = json!({ "op": "Hello", "d": d }).to_string();
        ws.send(Ws::text(hello)).await.unwrap();
        let code = loop {
            match tokio::time::timeout(Duration::from_secs(3), ws.next())
                .await
                .unwrap()
            {
                Some(Ok(Ws::Close(Some(f)))) => break u16::from(f.code),
                Some(Ok(_)) => continue,
                other => panic!("expected a close frame, got {other:?}"),
            }
        };
        assert_eq!(code, 4005, "{d}");
    }
}

/// Ready loads every server's channels and members, and every conversation's members, in a
/// few bulk queries regrouped in Rust: each server and conversation must get exactly its own.
#[tokio::test]
async fn ready_groups_channels_and_members_per_server_and_conversation() {
    use pulse_protocol::rest::{Channel, Member};
    use pulse_server::servers::routes::{all_servers, dms_of, server_channels, server_members};

    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (riley_id, riley) = register(&app, "riley").await;

    let s1 = create_server(&app, &alex, "One").await;
    let s2 = create_server(&app, &sam, "Two").await;
    let s3 = create_server(&app, &riley, "Three").await; // only riley
    for (tok, srv) in [(&sam, s1.id), (&riley, s1.id), (&alex, s2.id)] {
        let r = post_json(&app, tok, &format!("/servers/{srv}/join"), json!({})).await;
        assert!(r.status().is_success());
    }
    nickname_and_channels(&app, &alex, s1.id, &["b", "a", "c"]).await;
    nickname_and_channels(&app, &sam, s2.id, &["z"]).await;

    let dm = create_dm(&app, &alex, &[sam_id]).await;
    let g3 = create_dm(&app, &alex, &[sam_id, riley_id]).await;
    let g2 = create_dm(&app, &sam, &[alex_id, riley_id]).await;
    let r = app
        .http
        .delete(app.url(&format!("/channels/{}/members/{riley_id}", g2.id)))
        .bearer_auth(&riley)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let not_mine = create_dm(&app, &sam, &[riley_id]).await;

    let ready = pulse_server::gateway::socket::build_ready(
        &app.db,
        &app.voice,
        &app.streams,
        &app.hub,
        alex_id,
    )
    .await
    .unwrap();

    // Same order and content as the per-server / per-conversation queries.
    let servers = all_servers(&app.db).await.unwrap();
    assert_eq!(ready.servers, servers);
    let mut want_channels: Vec<Channel> = vec![];
    let mut want_members: Vec<(ServerId, Vec<Member>)> = vec![];
    for srv in &servers {
        want_channels.extend(server_channels(&app.db, srv.id).await.unwrap());
        want_members.push((srv.id, server_members(&app.db, srv.id).await.unwrap()));
    }
    want_channels.extend(dms_of(&app.db, alex_id).await.unwrap());
    assert_eq!(ready.channels, want_channels);
    let got_members: Vec<(ServerId, Vec<Member>)> = ready
        .members
        .iter()
        .map(|m| (m.server_id, m.members.clone()))
        .collect();
    assert_eq!(got_members, want_members);

    // Spot-check the regrouping against what was actually set up.
    let names = |srv: ServerId| -> Vec<String> {
        let mut n: Vec<String> = ready
            .channels
            .iter()
            .filter(|c| c.server_id == Some(srv))
            .filter_map(|c| c.name.clone())
            .collect();
        n.sort();
        n
    };
    assert_eq!(names(s1.id), ["Lounge", "a", "b", "c", "general"]);
    assert_eq!(names(s2.id), ["Lounge", "general", "z"]);
    assert_eq!(names(s3.id), ["Lounge", "general"]);
    let member_ids = |srv: ServerId| -> Vec<pulse_protocol::ids::UserId> {
        let mut v: Vec<_> = ready
            .members
            .iter()
            .find(|m| m.server_id == srv)
            .unwrap()
            .members
            .iter()
            .map(|m| m.user.id)
            .collect();
        v.sort();
        v
    };
    let sorted = |mut v: Vec<pulse_protocol::ids::UserId>| {
        v.sort();
        v
    };
    assert_eq!(member_ids(s1.id), sorted(vec![alex_id, sam_id, riley_id]));
    assert_eq!(member_ids(s2.id), sorted(vec![alex_id, sam_id]));
    assert_eq!(member_ids(s3.id), vec![riley_id]);
    let nick = ready
        .members
        .iter()
        .find(|m| m.server_id == s1.id)
        .unwrap()
        .members
        .iter()
        .find(|m| m.user.id == alex_id)
        .unwrap()
        .nickname
        .clone();
    let want_nick: Option<String> = sqlx::query_scalar(
        "SELECT nickname FROM server_members WHERE server_id = ? AND user_id = ?",
    )
    .bind(s1.id.to_string())
    .bind(alex_id.to_string())
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(nick, want_nick);

    let mut dms: Vec<_> = ready
        .dm_members
        .iter()
        .map(|d| (d.channel_id, d.user_ids.clone()))
        .collect();
    dms.sort();
    let mut want_dms = vec![
        (dm.id, sorted(vec![alex_id, sam_id])),
        (g3.id, sorted(vec![alex_id, sam_id, riley_id])),
        (g2.id, sorted(vec![alex_id, sam_id])),
    ];
    want_dms.sort();
    assert_eq!(dms, want_dms);
    assert!(ready.channels.iter().all(|c| c.id != not_mine.id));
    let private: Vec<_> = ready
        .channels
        .iter()
        .filter(|c| c.server_id.is_none())
        .map(|c| c.id)
        .collect();
    assert_eq!(private.len(), 3);
}

/// Give `token`'s user a nickname in `srv` (straight in the DB) and add text channels.
async fn nickname_and_channels(app: &TestApp, token: &str, srv: ServerId, names: &[&str]) {
    let me: pulse_protocol::rest::User = get_json(app, token, "/me").await;
    sqlx::query("UPDATE server_members SET nickname = ? WHERE server_id = ? AND user_id = ?")
        .bind(format!("nick-{}", me.username))
        .bind(srv.to_string())
        .bind(me.id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    for n in names {
        let r = post_json(
            app,
            token,
            &format!("/servers/{srv}/channels"),
            json!({ "name": n, "kind": "text" }),
        )
        .await;
        assert_eq!(r.status(), 200);
    }
}
