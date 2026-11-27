//! Server rail order (#185): the order you drag the servers into, stored on the server so it follows you to every
//! device, and sent only to you (like the live-notify list, #60).
mod common;

use std::time::Duration;

use common::*;
use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::ServerId;
use pulse_server::gateway::audience::{Audience, audience_for};
use serde_json::json;

/// Every server order change that arrives within `ms` of quiet.
async fn orders(ws: &mut Socket, ms: u64) -> Vec<Vec<ServerId>> {
    let mut got = vec![];
    while let Ok(Some(f)) = tokio::time::timeout(Duration::from_millis(ms), next_frame(ws)).await {
        if let ServerFrame::Event(Event::ServerOrderChanged { server_ids, .. }) = f {
            got.push(server_ids);
        }
    }
    got
}

async fn set_order(app: &TestApp, token: &str, ids: serde_json::Value) -> u16 {
    app.http
        .put(app.url("/me/server-order"))
        .bearer_auth(token)
        .json(&json!({ "server_ids": ids }))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test]
async fn the_order_is_saved_and_follows_you_to_every_device() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let one = create_server(&app, &a, "One").await.id;
    let two = create_server(&app, &a, "Two").await.id;
    let three = create_server(&app, &a, "Three").await.id;
    let (mut ws1, r) = hello(&app, &a).await;
    assert!(r.server_order.is_empty());
    let (mut ws2, _) = hello(&app, &a).await;

    assert_eq!(set_order(&app, &a, json!([three, one, two])).await, 204);
    for ws in [&mut ws1, &mut ws2] {
        assert_eq!(orders(ws, 400).await, vec![vec![three, one, two]]);
    }
    let (_ws3, r) = hello(&app, &a).await;
    assert_eq!(r.server_order, vec![three, one, two]);

    // a new order replaces the old one; a part order is fine (the rest follow it in their usual order)
    assert_eq!(set_order(&app, &a, json!([two])).await, 204);
    assert_eq!(orders(&mut ws1, 400).await, vec![vec![two]]);
    let (_ws4, r) = hello(&app, &a).await;
    assert_eq!(r.server_order, vec![two]);
}

#[tokio::test]
async fn deleted_and_unknown_servers_are_left_out() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let one = create_server(&app, &a, "One").await.id;
    let two = create_server(&app, &a, "Two").await.id;
    assert_eq!(
        set_order(&app, &a, json!([two, ServerId::new(), one])).await,
        204
    );
    let (_ws, r) = hello(&app, &a).await;
    assert_eq!(r.server_order, vec![two, one]);
    // a soft-deleted server leaves the order it's in
    let d = app
        .http
        .delete(app.url(&format!("/servers/{two}")))
        .bearer_auth(&a)
        .json(&json!({ "name": "Two" }))
        .send()
        .await
        .unwrap();
    assert!(d.status().is_success(), "delete server {}", d.status());
    let (_ws, r) = hello(&app, &a).await;
    assert_eq!(r.server_order, vec![one]);
}

#[tokio::test]
async fn the_order_only_ever_reaches_its_owner() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (_, s) = register(&app, "sam").await;
    let one = create_server(&app, &a, "One").await.id;
    let e = Event::ServerOrderChanged {
        user_id: a_id,
        server_ids: vec![one],
    };
    assert_eq!(
        audience_for(&app.db, &e).await.unwrap(),
        Audience::Users([a_id].into_iter().collect())
    );
    let (mut sam_ws, _) = hello(&app, &s).await;
    orders(&mut sam_ws, 200).await;
    assert_eq!(set_order(&app, &a, json!([one])).await, 204);
    assert!(orders(&mut sam_ws, 400).await.is_empty());
    let (_s2, sr) = hello(&app, &s).await;
    assert!(sr.server_order.is_empty());
}

#[tokio::test]
async fn bad_requests_are_refused() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let one = create_server(&app, &a, "One").await.id;
    assert_eq!(set_order(&app, &a, json!([one, one])).await, 400);
    assert_eq!(set_order(&app, &a, json!(["not-an-id"])).await, 422);
    let many: Vec<ServerId> = (0..501).map(|_| ServerId::new()).collect();
    assert_eq!(set_order(&app, &a, json!(many)).await, 400);
    let r = app
        .http
        .put(app.url("/me/server-order"))
        .json(&json!({ "server_ids": [one] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
    let (_ws, r) = hello(&app, &a).await;
    assert!(r.server_order.is_empty());
}
