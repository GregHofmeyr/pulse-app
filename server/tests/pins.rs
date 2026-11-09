//! Pinned messages (#178): pin, unpin and list, who may, the cap, expiry, and what deletes and edits do.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{
    ApiError, Channel, ChannelKind, Message, MessageKind, Pin, PinDuration, PinnedMessage,
};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// alex's server "Main" (sam has joined; jo hasn't) and its #general.
struct World {
    app: TestApp,
    alex: String,
    sam: String,
    sam_id: UserId,
    jo: String,
    server: ServerId,
    general: ChannelId,
}

async fn world() -> World {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (_, jo) = register(&app, "jo").await;
    let s = create_server(&app, &alex, "Main").await;
    let r = post_json(&app, &sam, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);
    let general = general(&app, &alex, s.id).await.id;
    World {
        app,
        alex,
        sam,
        sam_id,
        jo,
        server: s.id,
        general,
    }
}

async fn pin(app: &TestApp, token: &str, msg: MessageId, duration: &str) -> reqwest::Response {
    app.http
        .put(app.url(&format!("/messages/{msg}/pin")))
        .bearer_auth(token)
        .json(&json!({ "duration": duration }))
        .send()
        .await
        .unwrap()
}

async fn unpin(app: &TestApp, token: &str, msg: MessageId) -> u16 {
    app.http
        .delete(app.url(&format!("/messages/{msg}/pin")))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn pins(app: &TestApp, token: &str, channel: ChannelId) -> Vec<PinnedMessage> {
    get_json(app, token, &format!("/channels/{channel}/pins")).await
}

async fn messages(app: &TestApp, token: &str, channel: ChannelId) -> Vec<Message> {
    get_json(app, token, &format!("/channels/{channel}/messages")).await
}

async fn hello(app: &TestApp, token: &str) -> Socket {
    let (mut ws, _) = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap();
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    loop {
        match next_event(&mut ws, Duration::from_secs(3)).await {
            Some(ServerFrame::Ready(_)) => return ws,
            Some(_) => continue,
            None => panic!("expected Ready"),
        }
    }
}

async fn next_event(ws: &mut Socket, wait: Duration) -> Option<ServerFrame> {
    loop {
        let m = tokio::time::timeout(wait, ws.next()).await.ok()??.ok()?;
        if let Ws::Text(t) = m {
            return Some(serde_json::from_str(&t).unwrap());
        }
    }
}

/// The next pin-related event or message created, skipping the rest; None after a quiet second.
async fn next_pin_event(ws: &mut Socket) -> Option<Event> {
    loop {
        match next_event(ws, Duration::from_secs(1)).await? {
            ServerFrame::Event(
                e @ (Event::MessagePinned { .. }
                | Event::MessageUnpinned { .. }
                | Event::MessageCreated { .. }),
            ) => return Some(e),
            _ => continue,
        }
    }
}

#[tokio::test]
async fn pin_list_and_unpin() {
    let w = world().await;
    let a = send(&w.app, &w.alex, w.general, "plan for tonight").await;
    let b = send(&w.app, &w.sam, w.general, "a link").await;
    let r = pin(&w.app, &w.sam, a.id, "day").await;
    assert_eq!(r.status(), 200);
    let p: Pin = r.json().await.unwrap();
    assert!(p.expires_at.is_some());
    assert_eq!(pin(&w.app, &w.alex, b.id, "forever").await.status(), 200);
    let list = pins(&w.app, &w.jo, w.general).await;
    assert_eq!(
        list.iter().map(|p| p.message.id).collect::<Vec<_>>(),
        vec![b.id, a.id],
        "newest pin first, readable by anyone who can read the channel"
    );
    assert_eq!(list[1].pin.expires_at, p.expires_at);
    assert_eq!(list[0].pin.expires_at, None, "Forever has no end");
    assert_eq!(list[1].message.content, "plan for tonight");
    assert_eq!(
        unpin(&w.app, &w.alex, a.id).await,
        204,
        "anyone in the chat unpins"
    );
    assert_eq!(
        unpin(&w.app, &w.alex, a.id).await,
        404,
        "not pinned any more"
    );
    let list = pins(&w.app, &w.alex, w.general).await;
    assert_eq!(list.len(), 1);
}

#[tokio::test]
async fn the_pin_posts_a_line_and_events_reach_the_chat() {
    let w = world().await;
    let a = send(&w.app, &w.alex, w.general, "plan").await;
    let mut ws = hello(&w.app, &w.sam).await;
    assert_eq!(pin(&w.app, &w.alex, a.id, "week").await.status(), 200);
    match next_pin_event(&mut ws).await {
        Some(Event::MessagePinned { channel_id, pinned }) => {
            assert_eq!(channel_id, w.general);
            assert_eq!(pinned.message.id, a.id);
            assert_eq!(pinned.message.pin.as_ref(), Some(&pinned.pin));
        }
        other => panic!("expected MessagePinned, got {other:?}"),
    }
    let line = match next_pin_event(&mut ws).await {
        Some(Event::MessageCreated { message, .. }) => message,
        other => panic!("expected the system line, got {other:?}"),
    };
    assert_eq!(line.kind, MessageKind::System);
    assert_eq!(line.content, "alex pinned a message for 1 week");
    let pl = line.pin_line.expect("the line carries what was pinned");
    assert_eq!(pl.message_id, a.id);
    assert_eq!(pl.duration, PinDuration::Week);
    // the line is in history like any other system line
    let listed = messages(&w.app, &w.sam, w.general).await;
    assert_eq!(listed[0].id, line.id);
    assert_eq!(
        listed[0].pin_line.as_ref().map(|p| p.message_id),
        Some(a.id)
    );

    // an unpin is an event, with no line
    assert_eq!(unpin(&w.app, &w.sam, a.id).await, 204);
    match next_pin_event(&mut ws).await {
        Some(Event::MessageUnpinned {
            channel_id,
            message_id,
        }) => {
            assert_eq!((channel_id, message_id), (w.general, a.id));
        }
        other => panic!("expected MessageUnpinned, got {other:?}"),
    }
    assert!(
        next_pin_event(&mut ws).await.is_none(),
        "no line for an unpin"
    );

    // Forever reads without a duration
    assert_eq!(pin(&w.app, &w.alex, a.id, "forever").await.status(), 200);
    let listed = messages(&w.app, &w.sam, w.general).await;
    assert_eq!(listed[0].content, "alex pinned a message");
}

#[tokio::test]
async fn permissions() {
    let w = world().await;
    let a = send(&w.app, &w.alex, w.general, "plan").await;
    // jo can read the server's channels but isn't in the server: like sending, refused
    assert_eq!(pin(&w.app, &w.jo, a.id, "hour").await.status(), 403);
    assert_eq!(pin(&w.app, &w.sam, a.id, "hour").await.status(), 200);
    assert_eq!(unpin(&w.app, &w.jo, a.id).await, 403);

    // a DM: only its two people
    let dm: Channel = create_dm(&w.app, &w.alex, &[w.sam_id]).await;
    let d = send(&w.app, &w.alex, dm.id, "just us").await;
    assert_eq!(pin(&w.app, &w.jo, d.id, "hour").await.status(), 404);
    let r = w
        .app
        .http
        .get(w.app.url(&format!("/channels/{}/pins", dm.id)))
        .bearer_auth(&w.jo)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404, "an outsider can't list a DM's pins");
    assert_eq!(pin(&w.app, &w.sam, d.id, "forever").await.status(), 200);
    assert_eq!(pins(&w.app, &w.alex, dm.id).await.len(), 1);
}

#[tokio::test]
async fn voice_channels_chats_take_pins() {
    let w = world().await;
    let chans: Vec<Channel> =
        get_json(&w.app, &w.alex, &format!("/servers/{}/channels", w.server)).await;
    let lounge = chans.iter().find(|c| c.kind == ChannelKind::Voice).unwrap();
    let m = send(&w.app, &w.alex, lounge.id, "can you hear me?").await;
    assert_eq!(pin(&w.app, &w.sam, m.id, "hour").await.status(), 200);
    assert_eq!(pins(&w.app, &w.alex, lounge.id).await.len(), 1);
}

#[tokio::test]
async fn quotes_and_clips_channels_refuse_pins() {
    let w = world().await;
    let r = post_json(
        &w.app,
        &w.alex,
        &format!("/servers/{}/channels", w.server),
        json!({"kind": "quotes", "name": "quotes"}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let quotes: Channel = r.json().await.unwrap();
    // a quote is a message of the quotes channel
    let r = post_json(
        &w.app,
        &w.alex,
        &format!("/channels/{}/quotes", quotes.id),
        json!({"lines": [{"speaker_name": "kim", "text": "a joke"}], "today": null, "nonce": null}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let q: Message = r.json().await.unwrap();
    assert_eq!(pin(&w.app, &w.alex, q.id, "hour").await.status(), 400);
    let r = w
        .app
        .http
        .get(w.app.url(&format!("/channels/{}/pins", quotes.id)))
        .bearer_auth(&w.alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400, "a quotes channel has no pins");
}

#[tokio::test]
async fn system_lines_deleted_messages_and_bad_durations_are_refused() {
    let w = world().await;
    let a = send(&w.app, &w.alex, w.general, "plan").await;
    assert_eq!(pin(&w.app, &w.alex, a.id, "day").await.status(), 200);
    let line = messages(&w.app, &w.alex, w.general).await[0].clone();
    assert_eq!(line.kind, MessageKind::System);
    assert_eq!(pin(&w.app, &w.alex, line.id, "day").await.status(), 400);
    let b = send(&w.app, &w.alex, w.general, "oops").await;
    let r = w
        .app
        .http
        .delete(w.app.url(&format!("/messages/{}", b.id)))
        .bearer_auth(&w.alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(pin(&w.app, &w.alex, b.id, "day").await.status(), 404);
    let c = send(&w.app, &w.alex, w.general, "c").await;
    assert_eq!(pin(&w.app, &w.alex, c.id, "year").await.status(), 422);
    assert_eq!(
        pin(&w.app, &w.alex, MessageId::new(), "day").await.status(),
        404
    );
}

#[tokio::test]
async fn pinning_twice_is_a_conflict() {
    let w = world().await;
    let a = send(&w.app, &w.alex, w.general, "plan").await;
    assert_eq!(pin(&w.app, &w.alex, a.id, "day").await.status(), 200);
    assert_eq!(pin(&w.app, &w.sam, a.id, "week").await.status(), 409);
    let lines = messages(&w.app, &w.alex, w.general)
        .await
        .into_iter()
        .filter(|m| m.kind == MessageKind::System)
        .count();
    assert_eq!(lines, 1, "the refused pin posts no line");
}

#[tokio::test]
async fn the_cap_is_50() {
    let w = world().await;
    let mut ids = Vec::new();
    for i in 0..51 {
        ids.push(send(&w.app, &w.alex, w.general, &format!("m{i}")).await.id);
    }
    for id in &ids[..50] {
        // past the write limiter: pins are written straight in, as the route would
        sqlx::query(
            "INSERT INTO pins (message_id, channel_id, pinned_by, pinned_at, expires_at)
             SELECT id, channel_id, author_id, ?, NULL FROM messages WHERE id = ?",
        )
        .bind(pulse_server::db::now())
        .bind(id.to_string())
        .execute(&w.app.db)
        .await
        .unwrap();
    }
    let r = pin(&w.app, &w.alex, ids[50], "hour").await;
    assert_eq!(r.status(), 409);
    let e: ApiError = r.json().await.unwrap();
    assert!(e.message.contains("Unpin something first"), "{}", e.message);
    // an expired pin doesn't count
    sqlx::query("UPDATE pins SET expires_at = '2000-01-01T00:00:00.000Z' WHERE message_id = ?")
        .bind(ids[0].to_string())
        .execute(&w.app.db)
        .await
        .unwrap();
    assert_eq!(pin(&w.app, &w.alex, ids[50], "hour").await.status(), 200);
}

#[tokio::test]
async fn a_timed_pin_expires_and_the_event_fires() {
    let w = world().await;
    let a = send(&w.app, &w.alex, w.general, "plan").await;
    let b = send(&w.app, &w.alex, w.general, "keep").await;
    assert_eq!(pin(&w.app, &w.alex, a.id, "hour").await.status(), 200);
    assert_eq!(pin(&w.app, &w.alex, b.id, "forever").await.status(), 200);
    let mut ws = hello(&w.app, &w.sam).await;
    let soon = pulse_server::db::stamp(chrono::Utc::now() + chrono::Duration::minutes(30));
    assert_eq!(
        pulse_server::pins::expire_overdue(&w.app.state(), &soon)
            .await
            .unwrap(),
        0
    );
    let later = pulse_server::db::stamp(chrono::Utc::now() + chrono::Duration::hours(2));
    assert_eq!(
        pulse_server::pins::expire_overdue(&w.app.state(), &later)
            .await
            .unwrap(),
        1
    );
    match next_pin_event(&mut ws).await {
        Some(Event::MessageUnpinned { message_id, .. }) => assert_eq!(message_id, a.id),
        other => panic!("expected MessageUnpinned, got {other:?}"),
    }
    assert!(
        next_pin_event(&mut ws).await.is_none(),
        "expiry adds no line"
    );
    let list = pins(&w.app, &w.alex, w.general).await;
    assert_eq!(
        list.iter().map(|p| p.message.id).collect::<Vec<_>>(),
        vec![b.id]
    );
}

#[tokio::test]
async fn an_expired_pin_is_gone_before_the_sweep() {
    let w = world().await;
    let a = send(&w.app, &w.alex, w.general, "plan").await;
    assert_eq!(pin(&w.app, &w.alex, a.id, "hour").await.status(), 200);
    sqlx::query("UPDATE pins SET expires_at = '2000-01-01T00:00:00.000Z'")
        .execute(&w.app.db)
        .await
        .unwrap();
    assert!(pins(&w.app, &w.alex, w.general).await.is_empty());
    let listed = messages(&w.app, &w.alex, w.general).await;
    let m = listed.iter().find(|m| m.id == a.id).unwrap();
    assert_eq!(m.pin, None, "the message no longer shows it pinned");
    assert_eq!(
        pin(&w.app, &w.alex, a.id, "day").await.status(),
        200,
        "and it can be pinned again"
    );
}

#[tokio::test]
async fn deleting_the_message_drops_its_pin() {
    let w = world().await;
    let a = send(&w.app, &w.alex, w.general, "plan").await;
    assert_eq!(pin(&w.app, &w.sam, a.id, "forever").await.status(), 200);
    let mut ws = hello(&w.app, &w.sam).await;
    let r = w
        .app
        .http
        .delete(w.app.url(&format!("/messages/{}", a.id)))
        .bearer_auth(&w.alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    match next_pin_event(&mut ws).await {
        Some(Event::MessageUnpinned { message_id, .. }) => assert_eq!(message_id, a.id),
        other => panic!("expected MessageUnpinned, got {other:?}"),
    }
    assert!(pins(&w.app, &w.alex, w.general).await.is_empty());
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pins")
        .fetch_one(&w.app.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn editing_keeps_the_pin_and_messages_carry_it() {
    let w = world().await;
    let a = send(&w.app, &w.alex, w.general, "plan").await;
    assert_eq!(pin(&w.app, &w.sam, a.id, "day").await.status(), 200);
    let r = w
        .app
        .http
        .patch(w.app.url(&format!("/messages/{}", a.id)))
        .bearer_auth(&w.alex)
        .json(&json!({"content": "new plan"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let edited: Message = r.json().await.unwrap();
    assert!(edited.pin.is_some(), "the edit's copy still shows the pin");
    let list = pins(&w.app, &w.alex, w.general).await;
    assert_eq!(list[0].message.content, "new plan");
    let listed = messages(&w.app, &w.jo, w.general).await;
    let m = listed.iter().find(|m| m.id == a.id).unwrap();
    let p = m.pin.as_ref().expect("a listed message carries its pin");
    assert!(p.expires_at.is_some());
    let unpinned = listed
        .iter()
        .find(|m| m.id != a.id && m.kind == MessageKind::Normal);
    assert!(unpinned.is_none_or(|m| m.pin.is_none()));
}
