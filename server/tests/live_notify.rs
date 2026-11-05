//! "Notify me when they go live" (#60): a per-account list of people, stored on the server so it
//! follows you to every device, and sent only to its owner (like the appear-offline list, #45).
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::UserId;
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
        client_version: pulse_protocol::PROTOCOL_VERSION,
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

/// Every event that arrives within `ms` of quiet.
async fn drain(ws: &mut Socket, ms: u64) -> Vec<Event> {
    let mut got = vec![];
    while let Ok(Some(f)) = tokio::time::timeout(Duration::from_millis(ms), next_frame(ws)).await {
        if let ServerFrame::Event(e) = f {
            got.push(e);
        }
    }
    got
}

async fn live_notify(app: &TestApp, token: &str, who: UserId, on: bool) -> u16 {
    let url = app.url(&format!("/me/live-notify/{who}"));
    let req = if on {
        app.http.put(url)
    } else {
        app.http.delete(url)
    };
    req.bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

fn changes(events: &[Event]) -> Vec<(UserId, bool)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::LiveNotifyChanged { target_id, on, .. } => Some((*target_id, *on)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn the_list_is_saved_and_follows_you_to_every_device() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (s_id, _) = register(&app, "sam").await;
    let (k_id, _) = register(&app, "kim").await;
    let (mut one, r) = hello(&app, &a).await;
    assert!(r.live_notify.is_empty());
    let (mut two, _) = hello(&app, &a).await;

    assert_eq!(live_notify(&app, &a, s_id, true).await, 204);
    assert_eq!(live_notify(&app, &a, k_id, true).await, 204);
    // turning it on twice is harmless and keeps the order
    assert_eq!(live_notify(&app, &a, s_id, true).await, 204);
    for ws in [&mut one, &mut two] {
        assert_eq!(
            changes(&drain(ws, 400).await),
            vec![(s_id, true), (k_id, true), (s_id, true)]
        );
    }
    drop(one);
    drop(two);
    let (mut three, r) = hello(&app, &a).await;
    assert_eq!(r.live_notify, vec![s_id, k_id]);

    assert_eq!(live_notify(&app, &a, s_id, false).await, 204);
    assert_eq!(changes(&drain(&mut three, 400).await), vec![(s_id, false)]);
    let (_four, r) = hello(&app, &a).await;
    assert_eq!(r.live_notify, vec![k_id]);
    // turning off someone not on the list is harmless too
    assert_eq!(live_notify(&app, &a, s_id, false).await, 204);
}

#[tokio::test]
async fn the_list_only_ever_reaches_its_owner() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (s_id, s) = register(&app, "sam").await;
    let e = Event::LiveNotifyChanged {
        user_id: a_id,
        target_id: s_id,
        on: true,
    };
    assert_eq!(
        audience_for(&app.db, &e).await.unwrap(),
        Audience::Users([a_id].into_iter().collect())
    );
    let (mut sam_ws, _) = hello(&app, &s).await;
    drain(&mut sam_ws, 200).await;
    live_notify(&app, &a, s_id, true).await;
    let seen = drain(&mut sam_ws, 400).await;
    assert!(changes(&seen).is_empty(), "{seen:?}");
    let (_s2, sr) = hello(&app, &s).await;
    assert!(sr.live_notify.is_empty());
}

#[tokio::test]
async fn bad_requests_are_refused() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    assert_eq!(live_notify(&app, &a, a_id, true).await, 400);
    assert_eq!(live_notify(&app, &a, UserId::new(), true).await, 404);
    let r = app
        .http
        .put(app.url("/me/live-notify/not-an-id"))
        .bearer_auth(&a)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 404);
    let r = app
        .http
        .put(app.url(&format!("/me/live-notify/{a_id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
    let (_ws, r) = hello(&app, &a).await;
    assert!(r.live_notify.is_empty());
}
