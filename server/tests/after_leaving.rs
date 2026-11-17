//! What someone can still change after they leave a server or are removed from it (security review, post-0.2.12).
//! Its channels stay readable to everyone, but their old messages, polls and plans are no longer theirs to edit;
//! deleting their own message stays allowed (#418).
mod common;

use common::*;
use pulse_protocol::ids::{MessageId, ServerId, UserId};
use pulse_protocol::rest::{Channel, Message};
use serde_json::{Value, json};

/// alex owns "Main"; sam is in it.
struct World {
    app: TestApp,
    alex: (UserId, String),
    sam: (UserId, String),
    server: ServerId,
    general: Channel,
}

async fn world() -> World {
    let app = spawn().await;
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let s = create_server(&app, &alex.1, "Main").await;
    let r = post_json(&app, &sam.1, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);
    let general = general(&app, &alex.1, s.id).await;
    World {
        app,
        alex,
        sam,
        server: s.id,
        general,
    }
}

async fn put_json(app: &TestApp, token: &str, path: &str, body: Value) -> reqwest::Response {
    app.http
        .put(app.url(path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

async fn remove_sam(w: &World) {
    let r = w
        .app
        .http
        .delete(
            w.app
                .url(&format!("/servers/{}/members/{}", w.server, w.sam.0)),
        )
        .bearer_auth(&w.alex.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
}

async fn sams_poll(w: &World) -> MessageId {
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/channels/{}/polls", w.general.id),
        json!({"question": "Pizza?", "options": ["yes", "no"], "ends_in": "day"}),
    )
    .await;
    assert_eq!(r.status(), 201);
    r.json::<Message>().await.unwrap().id
}

async fn sams_plan(w: &World) -> MessageId {
    let starts = (chrono::Utc::now() + chrono::TimeDelta::days(3)).to_rfc3339();
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/channels/{}/plans", w.general.id),
        json!({"title": "Braai", "starts_at": starts}),
    )
    .await;
    assert_eq!(r.status(), 201);
    r.json::<Message>().await.unwrap().id
}

#[tokio::test]
async fn a_removed_member_cant_edit_their_old_message_but_may_delete_it() {
    let w = world().await;
    let m = send(&w.app, &w.sam.1, w.general.id, "hello").await;
    remove_sam(&w).await;
    let path = format!("/messages/{}", m.id);
    let r = w
        .app
        .http
        .patch(w.app.url(&path))
        .bearer_auth(&w.sam.1)
        .json(&json!({"content": "rewritten"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let content: String = sqlx::query_scalar("SELECT content FROM messages WHERE id = ?")
        .bind(m.id.to_string())
        .fetch_one(&w.app.db)
        .await
        .unwrap();
    assert_eq!(content, "hello");
    // taking your words back stays allowed (#418)
    let r = w
        .app
        .http
        .delete(w.app.url(&path))
        .bearer_auth(&w.sam.1)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
}

#[tokio::test]
async fn a_removed_member_cant_edit_or_close_their_poll() {
    let w = world().await;
    let poll = sams_poll(&w).await;
    remove_sam(&w).await;
    let r = put_json(
        &w.app,
        &w.sam.1,
        &format!("/messages/{poll}/poll"),
        json!({"question": "Burgers?", "options": [{"text": "yes", "from": 0}, {"text": "no", "from": 1}]}),
    )
    .await;
    assert_eq!(r.status(), 403, "edit");
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/messages/{poll}/poll/close"),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 403, "close");
    let (question, closed): (String, Option<String>) =
        sqlx::query_as("SELECT question, closed_at FROM polls WHERE message_id = ?")
            .bind(poll.to_string())
            .fetch_one(&w.app.db)
            .await
            .unwrap();
    assert_eq!(question, "Pizza?");
    assert_eq!(closed, None);
}

#[tokio::test]
async fn a_removed_member_cant_edit_or_cancel_their_plan() {
    let w = world().await;
    let plan = sams_plan(&w).await;
    remove_sam(&w).await;
    let starts = (chrono::Utc::now() + chrono::TimeDelta::days(4)).to_rfc3339();
    let r = put_json(
        &w.app,
        &w.sam.1,
        &format!("/messages/{plan}/plan"),
        json!({"title": "Moved", "starts_at": starts}),
    )
    .await;
    assert_eq!(r.status(), 403, "edit");
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/messages/{plan}/plan/cancel"),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 403, "cancel");
    let (title, cancelled): (String, Option<String>) =
        sqlx::query_as("SELECT title, cancelled_at FROM plans WHERE message_id = ?")
            .bind(plan.to_string())
            .fetch_one(&w.app.db)
            .await
            .unwrap();
    assert_eq!(title, "Braai");
    assert_eq!(cancelled, None);
}
