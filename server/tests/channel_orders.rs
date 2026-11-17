//! Your own channel list order (#304): per person and server, in Ready and `ChannelOrderChanged`, never anyone
//! else's; it never touches the server's layout (nor its revision); cleaned against the server; members only.
mod common;

use common::*;
use pulse_protocol::rest::{Category, ChannelOrder, ServerLayout};
use serde_json::{Value, json};

async fn call(
    app: &TestApp,
    method: reqwest::Method,
    token: &str,
    path: &str,
    body: Option<Value>,
) -> reqwest::Response {
    let mut r = app.http.request(method, app.url(path)).bearer_auth(token);
    if let Some(b) = body {
        r = r.json(&b);
    }
    r.send().await.unwrap()
}

async fn ready_orders(app: &TestApp, user: pulse_protocol::ids::UserId) -> Vec<ChannelOrder> {
    pulse_server::gateway::socket::build_ready(&app.db, &app.voice, &app.streams, &app.hub, user)
        .await
        .unwrap()
        .channel_orders
}

async fn layout(app: &TestApp, user: pulse_protocol::ids::UserId) -> ServerLayout {
    pulse_server::gateway::socket::build_ready(&app.db, &app.voice, &app.streams, &app.hub, user)
        .await
        .unwrap()
        .layouts
        .remove(0)
}

#[tokio::test]
async fn your_order_is_yours_and_leaves_the_servers_alone() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (_, jo) = register(&app, "jo").await;
    let s = create_server(&app, &alex, "Main").await;
    let r = post_json(&app, &sam, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);
    let general = general(&app, &alex, s.id).await;
    let r = post_json(
        &app,
        &alex,
        &format!("/servers/{}/categories", s.id),
        json!({ "name": "Games" }),
    )
    .await;
    let games: Category = r.json().await.unwrap();
    let r = post_json(
        &app,
        &alex,
        &format!("/servers/{}/channels", s.id),
        json!({ "kind": "text", "name": "memes", "category_id": games.id }),
    )
    .await;
    let memes: pulse_protocol::rest::Channel = r.json().await.unwrap();
    let before = layout(&app, alex_id).await;
    let path = format!("/me/channel-orders/{}", s.id);

    // sam puts #memes on top, above the categories, Games first; nonsense ids are dropped, repeats kept once
    let r = call(
        &app,
        reqwest::Method::PUT,
        &sam,
        &path,
        Some(json!({
            "uncategorised": [memes.id, memes.id, pulse_protocol::ids::ChannelId::new()],
            "categories": [
                { "id": games.id, "channels": [general.id] },
                { "id": pulse_protocol::ids::CategoryId::new(), "channels": [] },
            ],
        })),
    )
    .await;
    assert_eq!(r.status(), 200);
    let kept: ChannelOrder = r.json().await.unwrap();
    assert_eq!(kept.uncategorised, vec![memes.id]);
    assert_eq!(kept.categories.len(), 1);
    assert_eq!(kept.categories[0].id, games.id);
    assert_eq!(kept.categories[0].channels, vec![general.id]);

    // sam's Ready has it; alex's doesn't; the server's layout and revision are as they were
    assert_eq!(ready_orders(&app, sam_id).await, vec![kept]);
    assert!(ready_orders(&app, alex_id).await.is_empty());
    let after = layout(&app, alex_id).await;
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.categories, before.categories);

    // jo isn't in the server: as if it weren't there
    let r = call(
        &app,
        reqwest::Method::PUT,
        &jo,
        &path,
        Some(json!({ "uncategorised": [], "categories": [] })),
    )
    .await;
    assert_eq!(r.status(), 404);

    // reset: back to the server's order
    let r = call(&app, reqwest::Method::DELETE, &sam, &path, None).await;
    assert_eq!(r.status(), 204);
    assert!(ready_orders(&app, sam_id).await.is_empty());
}
