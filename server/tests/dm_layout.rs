//! Your conversation list's pins and categories (#305): per person, on every device (Ready and `DmLayoutChanged`),
//! never anyone else's; only your own conversations; deleting a category puts its conversations back.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::rest::DmLayout;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello(app: &TestApp, token: &str) -> (Socket, DmLayout) {
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
            return (ws, r.dm_layout);
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

/// The next `DmLayoutChanged`'s layout, or None within 3 s.
async fn next_layout(ws: &mut Socket) -> Option<DmLayout> {
    loop {
        if let ServerFrame::Event(Event::DmLayoutChanged { layout, .. }) = next_frame(ws).await? {
            return Some(layout);
        }
    }
}

async fn call(
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

async fn new_category(app: &TestApp, token: &str, name: &str) -> DmLayout {
    let r = post_json(app, token, "/me/dm-categories", json!({ "name": name })).await;
    assert_eq!(r.status(), 201);
    r.json().await.unwrap()
}

async fn place(
    app: &TestApp,
    token: &str,
    channel: impl std::fmt::Display,
    body: Value,
) -> reqwest::Response {
    call(
        app,
        reqwest::Method::PUT,
        token,
        &format!("/me/dm-placements/{channel}"),
        body,
    )
    .await
}

#[tokio::test]
async fn pins_and_categories_follow_you_to_every_device_and_no_one_else() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (kim_id, _) = register(&app, "kim").await;
    let dm = create_dm(&app, &alex, &[sam_id]).await;
    let group = create_dm(&app, &alex, &[sam_id, kim_id]).await;
    let (mut other_device, empty) = hello(&app, &alex).await;
    assert_eq!(empty, DmLayout::default());
    let (mut sams, _) = hello(&app, &sam).await;

    let l = new_category(&app, &alex, "  Gaming ").await;
    assert_eq!(l.categories.len(), 1);
    assert_eq!(l.categories[0].name, "Gaming");
    let gaming = l.categories[0].id;
    assert_eq!(next_layout(&mut other_device).await.unwrap(), l);

    let r = place(&app, &alex, dm.id, json!({ "pinned": true })).await;
    assert_eq!(r.status(), 200);
    let r = place(&app, &alex, group.id, json!({ "category_id": gaming })).await;
    let l: DmLayout = r.json().await.unwrap();
    let mut placed: Vec<_> = l
        .placements
        .iter()
        .map(|p| (p.channel_id, p.pinned, p.category_id))
        .collect();
    placed.sort();
    let mut want = vec![(dm.id, true, None), (group.id, false, Some(gaming))];
    want.sort();
    assert_eq!(placed, want);

    // fold it, rename it
    let r = call(
        &app,
        reqwest::Method::PATCH,
        &alex,
        &format!("/me/dm-categories/{gaming}"),
        json!({ "collapsed": true, "name": "Games" }),
    )
    .await;
    let l: DmLayout = r.json().await.unwrap();
    assert!(l.categories[0].collapsed);
    assert_eq!(l.categories[0].name, "Games");

    // a fresh connection gets it all in Ready
    let (_, ready) = hello(&app, &alex).await;
    assert_eq!(ready, l);
    // sam's list is his own: none of alex's, and no event about it
    let (_, sams_ready) = hello(&app, &sam).await;
    assert_eq!(sams_ready, DmLayout::default());
    loop {
        match next_frame(&mut sams).await {
            None => break,
            Some(ServerFrame::Event(Event::DmLayoutChanged { .. })) => {
                panic!("sam was told alex's layout")
            }
            Some(_) => {}
        }
    }
    // sam can't touch alex's category
    let r = place(&app, &sam, dm.id, json!({ "category_id": gaming })).await;
    assert_eq!(r.status(), 404);
    let r = call(
        &app,
        reqwest::Method::DELETE,
        &sam,
        &format!("/me/dm-categories/{gaming}"),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn deleting_a_category_puts_its_conversations_back_and_unpinning_clears_a_placement() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, _) = register(&app, "sam").await;
    let (kim_id, _) = register(&app, "kim").await;
    let a = create_dm(&app, &alex, &[sam_id]).await;
    let b = create_dm(&app, &alex, &[kim_id]).await;
    let l = new_category(&app, &alex, "Friends").await;
    let c = l.categories[0].id;
    place(&app, &alex, a.id, json!({ "category_id": c })).await;
    place(
        &app,
        &alex,
        b.id,
        json!({ "category_id": c, "pinned": true }),
    )
    .await;
    let r = call(
        &app,
        reqwest::Method::DELETE,
        &alex,
        &format!("/me/dm-categories/{c}"),
        json!({}),
    )
    .await;
    let l: DmLayout = r.json().await.unwrap();
    assert!(l.categories.is_empty());
    // the pinned one stays pinned; the other has no placement left
    assert_eq!(l.placements.len(), 1);
    assert_eq!(
        (
            l.placements[0].channel_id,
            l.placements[0].pinned,
            l.placements[0].category_id
        ),
        (b.id, true, None)
    );
    let r = place(&app, &alex, b.id, json!({ "pinned": false })).await;
    let l: DmLayout = r.json().await.unwrap();
    assert!(l.placements.is_empty());
    assert_eq!(count(&app, "SELECT COUNT(*) FROM dm_placements").await, 0);
}

#[tokio::test]
async fn only_your_own_conversations_and_sensible_names() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (kim_id, kim) = register(&app, "kim").await;
    let s = create_server(&app, &alex, "Main").await;
    let g = general(&app, &alex, s.id).await;
    let theirs = create_dm(&app, &sam, &[kim_id]).await;
    // a server channel, or someone else's DM: refused (the DM looks like it isn't there)
    assert_eq!(
        place(&app, &alex, g.id, json!({ "pinned": true }))
            .await
            .status(),
        400
    );
    assert_eq!(
        place(&app, &alex, theirs.id, json!({ "pinned": true }))
            .await
            .status(),
        404
    );
    // names
    for bad in ["", "   ", &"x".repeat(33), "a\nb"] {
        let r = post_json(&app, &alex, "/me/dm-categories", json!({ "name": bad })).await;
        assert_eq!(r.status(), 400, "{bad:?}");
    }
    // two categories you can't tell apart: refused (renaming one to its own name is fine)
    let l = new_category(&app, &alex, "Games").await;
    let r = post_json(
        &app,
        &alex,
        "/me/dm-categories",
        json!({ "name": "games " }),
    )
    .await;
    assert_eq!(r.status(), 409);
    let id = l.categories[0].id;
    let r = call(
        &app,
        reqwest::Method::PATCH,
        &alex,
        &format!("/me/dm-categories/{id}"),
        json!({ "name": "GAMES" }),
    )
    .await;
    assert_eq!(r.status(), 200);
    // a group you leave drops out of your layout
    let dm = create_dm(&app, &alex, &[sam_id]).await;
    let grp = create_dm(&app, &alex, &[sam_id, kim_id]).await;
    place(&app, &kim, grp.id, json!({ "pinned": true })).await;
    place(&app, &alex, dm.id, json!({ "pinned": true })).await;
    let r = call(
        &app,
        reqwest::Method::DELETE,
        &kim,
        &format!("/channels/{}/members/{kim_id}", grp.id),
        json!({}),
    )
    .await;
    assert!(r.status().is_success(), "{}", r.status());
    let (_, kims) = hello(&app, &kim).await;
    assert!(kims.placements.is_empty());
}
