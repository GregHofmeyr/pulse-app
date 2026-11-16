//! Saved messages (#315): private bookmarks with a note, on any message you can see; told only to their person's
//! devices; one whose message is deleted or out of reach stays listed with nothing of the message.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, ServerFrame};
use pulse_protocol::rest::{MessageBookmark, bookmark_limits};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello(app: &TestApp, token: &str) -> (Socket, Value) {
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
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Ws::Text(t) = m
            && let Ok(ServerFrame::Ready(_)) = serde_json::from_str(&t)
        {
            let v: Value = serde_json::from_str(&t).unwrap();
            return (ws, v["d"].clone());
        }
    }
}

/// The next event `pick` takes (skipping others); None when none comes within `secs`.
async fn next_event(ws: &mut Socket, secs: u64, pick: impl Fn(&Value) -> bool) -> Option<Value> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(secs), ws.next())
            .await
            .ok()??
            .ok()?;
        if let Ws::Text(t) = m {
            let v: Value = serde_json::from_str(&t).unwrap();
            if v["op"] == "Event" && pick(&v["d"]) {
                return Some(v["d"].clone());
            }
        }
    }
}

async fn status(
    app: &TestApp,
    method: reqwest::Method,
    token: &str,
    path: &str,
    body: Value,
) -> reqwest::Response {
    app.http
        .request(method, app.url(path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn a_saved_message_is_private_synced_and_its_note_editable() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let s = create_server(&app, &alex, "Main").await;
    let r = post_json(&app, &sam, &format!("/servers/{}/join", s.id), json!({})).await;
    assert!(r.status().is_success());
    let g = general(&app, &alex, s.id).await;
    let m = send(&app, &sam, g.id, "the address is 12 park lane").await;

    let (mut ws, ready) = hello(&app, &alex).await;
    assert_eq!(ready["bookmarks"], json!([]));
    let (mut sam_ws, _) = hello(&app, &sam).await;

    let r = post_json(
        &app,
        &alex,
        &format!("/messages/{}/bookmark", m.id),
        json!({"note": "  for saturday  "}),
    )
    .await;
    assert_eq!(r.status(), 201);
    let b: MessageBookmark = r.json().await.unwrap();
    assert_eq!(b.note.as_deref(), Some("for saturday"));
    let saved = b.message.clone().unwrap();
    assert_eq!(saved.channel_id, g.id);
    assert_eq!(saved.content, "the address is 12 park lane");
    let ev = next_event(&mut ws, 3, |d| d["t"] == "BookmarkSaved")
        .await
        .unwrap();
    assert_eq!(ev["d"]["bookmark"]["id"], b.id.to_string());
    // sam's devices hear nothing of it, and his list is empty
    assert!(
        next_event(&mut sam_ws, 1, |d| d["t"] == "BookmarkSaved")
            .await
            .is_none()
    );
    let theirs: Vec<MessageBookmark> = get_json(&app, &sam, "/bookmarks").await;
    assert!(theirs.is_empty());

    // saving it again gives back the same one
    let r = post_json(
        &app,
        &alex,
        &format!("/messages/{}/bookmark", m.id),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let again: MessageBookmark = r.json().await.unwrap();
    assert_eq!(again.id, b.id);

    // the note: changed, cleared, too long; sam can't touch it
    let path = format!("/bookmarks/{}", b.id);
    let r = status(
        &app,
        reqwest::Method::PATCH,
        &alex,
        &path,
        json!({"note": "gate code 4321"}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let ev = next_event(&mut ws, 3, |d| d["t"] == "BookmarkSaved")
        .await
        .unwrap();
    assert_eq!(ev["d"]["bookmark"]["note"], "gate code 4321");
    let long = "n".repeat(bookmark_limits::NOTE_MAX_CHARS + 1);
    let r = status(
        &app,
        reqwest::Method::PATCH,
        &alex,
        &path,
        json!({ "note": long }),
    )
    .await;
    assert_eq!(r.status(), 400);
    let r = status(
        &app,
        reqwest::Method::PATCH,
        &sam,
        &path,
        json!({"note": "mine"}),
    )
    .await;
    assert_eq!(r.status(), 404);
    let r = status(
        &app,
        reqwest::Method::PATCH,
        &alex,
        &path,
        json!({"note": "  "}),
    )
    .await;
    let cleared: MessageBookmark = r.json().await.unwrap();
    assert_eq!(cleared.note, None);

    let (_ws2, ready) = hello(&app, &alex).await;
    assert_eq!(ready["bookmarks"][0]["id"], b.id.to_string());

    // removed: only alex can, and her devices hear it
    let r = status(&app, reqwest::Method::DELETE, &sam, &path, json!({})).await;
    assert_eq!(r.status(), 404);
    let r = status(&app, reqwest::Method::DELETE, &alex, &path, json!({})).await;
    assert_eq!(r.status(), 204);
    let ev = next_event(&mut ws, 3, |d| d["t"] == "BookmarkRemoved")
        .await
        .unwrap();
    assert_eq!(ev["d"]["id"], b.id.to_string());
    let mine: Vec<MessageBookmark> = get_json(&app, &alex, "/bookmarks").await;
    assert!(mine.is_empty());
}

#[tokio::test]
async fn a_deleted_or_unreachable_message_shows_as_unavailable_never_its_content() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (kim_id, kim) = register(&app, "kim").await;
    let (_, jo) = register(&app, "jo").await;
    let group = create_dm(&app, &alex, &[sam_id, kim_id]).await;
    let secret = send(&app, &sam, group.id, "the surprise is on friday").await;
    let gone = send(&app, &sam, group.id, "oops wrong chat").await;

    // jo isn't in the group: the message doesn't exist for her
    let r = post_json(
        &app,
        &jo,
        &format!("/messages/{}/bookmark", secret.id),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 404);

    for m in [&secret, &gone] {
        let r = post_json(
            &app,
            &kim,
            &format!("/messages/{}/bookmark", m.id),
            json!({}),
        )
        .await;
        assert_eq!(r.status(), 201);
    }
    // sam deletes one
    let r = app
        .http
        .delete(app.url(&format!("/messages/{}", gone.id)))
        .bearer_auth(&sam)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let list: Vec<MessageBookmark> = get_json(&app, &kim, "/bookmarks").await;
    assert_eq!(list.len(), 2);
    let deleted = list.iter().find(|b| b.message_id == gone.id).unwrap();
    assert!(deleted.message.is_none());
    let live = list.iter().find(|b| b.message_id == secret.id).unwrap();
    assert!(live.message.is_some());
    // a deleted one can't be saved
    let r = post_json(
        &app,
        &alex,
        &format!("/messages/{}/bookmark", gone.id),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 404);

    // kim leaves the group: nothing of it is listed any more
    let r = app
        .http
        .delete(app.url(&format!("/channels/{}/members/{kim_id}", group.id)))
        .bearer_auth(&kim)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "leave: {}", r.status());
    let list: Vec<MessageBookmark> = get_json(&app, &kim, "/bookmarks").await;
    assert_eq!(list.len(), 2);
    assert!(list.iter().all(|b| b.message.is_none()));
    let raw = serde_json::to_string(&list).unwrap();
    assert!(!raw.contains("surprise"), "{raw}");
}
