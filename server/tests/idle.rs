//! Idle (#53): each device says when it has had no input for a while (`ClientFrame::Idle`), and a person looks
//! Idle only while every connected device of theirs says so. Idle only modifies Online: DND and Invisible win.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::PresenceStatus;
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello(app: &TestApp, token: &str) -> (Socket, Ready) {
    let mut ws = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap()
        .0;
    send(
        &mut ws,
        &ClientFrame::Hello {
            token: token.into(),
            client_version: pulse_protocol::PROTOCOL_VERSION,
        },
    )
    .await;
    loop {
        if let Some(ServerFrame::Ready(r)) = next_frame(&mut ws).await {
            return (ws, r);
        }
    }
}

async fn send(ws: &mut Socket, f: &ClientFrame) {
    ws.send(Ws::text(serde_json::to_string(f).unwrap()))
        .await
        .unwrap();
}

async fn idle(ws: &mut Socket, idle: bool) {
    send(ws, &ClientFrame::Idle { idle }).await;
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

/// The presence events about `who` among `events`, as (status, idle).
fn about(events: &[Event], who: UserId) -> Vec<(PresenceStatus, bool)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::PresenceChanged {
                user_id,
                status,
                idle,
                ..
            } if *user_id == who => Some((*status, *idle)),
            _ => None,
        })
        .collect()
}

/// The next presence events about `who` (waits for the first, then gathers what follows within 300 ms).
async fn next_about(ws: &mut Socket, who: UserId) -> Vec<(PresenceStatus, bool)> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let got = about(&drain(ws, 300).await, who);
        if !got.is_empty() || tokio::time::Instant::now() > deadline {
            return got;
        }
    }
}

async fn set_status(app: &TestApp, token: &str, status: &str) {
    let r = app
        .http
        .put(app.url("/me/status"))
        .bearer_auth(token)
        .json(&serde_json::json!({ "status": status, "dnd_until": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
}

/// alex (one device) and sam, both connected and settled.
struct Two {
    /// Keep it bound: it owns the database's temp dir.
    app: TestApp,
    alex: (UserId, String, Socket),
    sam: (UserId, String, Socket),
}

async fn two() -> Two {
    let app = spawn().await;
    let (a, at) = register(&app, "alex").await;
    let (s, st) = register(&app, "sam").await;
    let (mut sam_ws, _) = hello(&app, &st).await;
    let (mut alex_ws, _) = hello(&app, &at).await;
    for ws in [&mut sam_ws, &mut alex_ws] {
        drain(ws, 300).await;
    }
    Two {
        app,
        alex: (a, at, alex_ws),
        sam: (s, st, sam_ws),
    }
}

#[tokio::test]
async fn the_frame_makes_you_idle_to_everyone_and_yourself_and_input_clears_it() {
    let Two {
        app,
        alex: (a, at, mut alex_ws),
        sam: (_, st, mut sam_ws),
    } = two().await;
    idle(&mut alex_ws, true).await;
    assert_eq!(
        next_about(&mut sam_ws, a).await,
        vec![(PresenceStatus::Online, true)]
    );
    // you see your own Idle
    assert_eq!(
        next_about(&mut alex_ws, a).await,
        vec![(PresenceStatus::Online, true)]
    );
    // a fresh Ready says so too
    let (_sam2, r) = hello(&app, &st).await;
    let p = r.people.iter().find(|p| p.user.id == a).unwrap();
    assert_eq!(
        (p.status, p.online, p.idle),
        (PresenceStatus::Online, true, true)
    );
    // saying it again changes nothing
    idle(&mut alex_ws, true).await;
    assert_eq!(about(&drain(&mut sam_ws, 300).await, a), vec![]);
    idle(&mut alex_ws, false).await;
    assert_eq!(
        next_about(&mut sam_ws, a).await,
        vec![(PresenceStatus::Online, false)]
    );
    let (_alex2, r) = hello(&app, &at).await;
    assert!(!r.people.iter().find(|p| p.user.id == a).unwrap().idle);
}

#[tokio::test]
async fn idle_only_when_every_device_is() {
    let Two {
        app,
        alex: (a, at, mut desk),
        sam: (_, _, mut sam_ws),
    } = two().await;
    let (mut laptop, _) = hello(&app, &at).await;
    drain(&mut sam_ws, 300).await;
    drain(&mut desk, 300).await;
    idle(&mut desk, true).await;
    // the laptop is still in use
    assert_eq!(about(&drain(&mut sam_ws, 400).await, a), vec![]);
    idle(&mut laptop, true).await;
    assert_eq!(
        next_about(&mut sam_ws, a).await,
        vec![(PresenceStatus::Online, true)]
    );
    // either one coming back is enough
    idle(&mut desk, false).await;
    assert_eq!(
        next_about(&mut sam_ws, a).await,
        vec![(PresenceStatus::Online, false)]
    );
    // a new device is in use until it says otherwise
    idle(&mut desk, true).await;
    assert_eq!(
        next_about(&mut sam_ws, a).await,
        vec![(PresenceStatus::Online, true)]
    );
    let (_phone, _) = hello(&app, &at).await;
    assert_eq!(
        next_about(&mut sam_ws, a).await,
        vec![(PresenceStatus::Online, false)]
    );
}

#[tokio::test]
async fn the_device_in_use_leaving_makes_you_idle() {
    let Two {
        app,
        alex: (a, at, mut desk),
        sam: (_, _, mut sam_ws),
    } = two().await;
    let (laptop, _) = hello(&app, &at).await;
    drain(&mut sam_ws, 300).await;
    idle(&mut desk, true).await;
    assert_eq!(about(&drain(&mut sam_ws, 400).await, a), vec![]);
    drop(laptop);
    assert_eq!(
        next_about(&mut sam_ws, a).await,
        vec![(PresenceStatus::Online, true)]
    );
    // and the idle one leaving is plain offline
    drop(desk);
    assert_eq!(
        next_about(&mut sam_ws, a).await,
        vec![(PresenceStatus::Offline, false)]
    );
}

#[tokio::test]
async fn dnd_and_invisible_win_over_idle() {
    let Two {
        app,
        alex: (a, at, mut alex_ws),
        sam: (_, st, mut sam_ws),
    } = two().await;
    set_status(&app, &at, "dnd").await;
    assert_eq!(
        next_about(&mut sam_ws, a).await,
        vec![(PresenceStatus::Dnd, false)]
    );
    drain(&mut alex_ws, 300).await;
    idle(&mut alex_ws, true).await;
    // nothing to tell: DND looks the same idle or not, to sam and to alex
    assert_eq!(about(&drain(&mut sam_ws, 400).await, a), vec![]);
    assert_eq!(about(&drain(&mut alex_ws, 100).await, a), vec![]);
    let (_s2, r) = hello(&app, &st).await;
    let p = r.people.iter().find(|p| p.user.id == a).unwrap();
    assert_eq!((p.status, p.idle), (PresenceStatus::Dnd, false));
    // back to Online while still idle: Idle
    set_status(&app, &at, "online").await;
    assert_eq!(
        next_about(&mut sam_ws, a).await,
        vec![(PresenceStatus::Online, true)]
    );
    // Invisible: offline to others; your own sessions see you Online (drawn Invisible), never Idle
    drain(&mut alex_ws, 300).await;
    set_status(&app, &at, "invisible").await;
    assert_eq!(
        next_about(&mut sam_ws, a).await,
        vec![(PresenceStatus::Offline, false)]
    );
    assert_eq!(
        next_about(&mut alex_ws, a).await,
        vec![(PresenceStatus::Online, false)]
    );
}

/// A client from before #53 never sends the frame, so its person never looks idle, and the field is left out of
/// what an older client is sent.
#[tokio::test]
async fn a_client_that_never_says_stays_active() {
    let Two {
        app,
        alex: (a, _, _alex_ws),
        sam: (_, st, _sam_ws),
    } = two().await;
    let mut ws = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap()
        .0;
    send(
        &mut ws,
        &ClientFrame::Hello {
            token: st.clone(),
            client_version: pulse_protocol::PROTOCOL_VERSION,
        },
    )
    .await;
    // the raw Ready: no `idle` key on anyone
    loop {
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let Ws::Text(t) = m else { continue };
        let v: serde_json::Value = serde_json::from_str(&t).unwrap();
        if v["op"] != "Ready" {
            continue;
        }
        let people = v["d"]["people"].as_array().unwrap();
        let alex = people
            .iter()
            .find(|p| p["user"]["id"] == a.to_string())
            .unwrap();
        assert_eq!(alex["status"], "online");
        assert!(alex.get("idle").is_none(), "{alex}");
        break;
    }
}
