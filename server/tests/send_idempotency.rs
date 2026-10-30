//! The outbox retries a send with the same nonce (e.g. after a proxy 502 that hid a success):
//! the server must post it once and hand back the original.

mod common;

use std::time::Duration;

use common::*;
use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::Message;
use serde_json::json;

struct Setup {
    app: TestApp,
    a: String,
    b_id: UserId,
    b: String,
    g: ChannelId,
}

async fn setup() -> Setup {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let s = create_server(&app, &a, "Main").await;
    post_json(&app, &b, &format!("/servers/{}/join", s.id), json!({})).await;
    let g = general(&app, &a, s.id).await.id;
    Setup { app, a, b_id, b, g }
}

async fn send_nonce(
    app: &TestApp,
    token: &str,
    channel: ChannelId,
    content: &str,
    nonce: &str,
) -> reqwest::Response {
    post_json(
        app,
        token,
        &format!("/channels/{channel}/messages"),
        json!({ "content": content, "nonce": nonce }),
    )
    .await
}

async fn normal_rows(app: &TestApp, channel: ChannelId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM messages WHERE channel_id = ? AND kind = 'normal'")
        .bind(channel.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn retry_with_same_nonce_returns_original_and_posts_once() {
    let t = setup().await;
    let mut rx = t.app.hub.register(t.b_id, String::new()).rx;
    let first: Message = send_nonce(&t.app, &t.a, t.g, "hello", "n-1")
        .await
        .json()
        .await
        .unwrap();
    let r = send_nonce(&t.app, &t.a, t.g, "hello", "n-1").await;
    assert_eq!(r.status(), 200);
    let again: Message = r.json().await.unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(again.content, "hello");
    assert_eq!(normal_rows(&t.app, t.g).await, 1);

    // Others saw it exactly once.
    let mut created = 0;
    while let Ok(Some(f)) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
        if matches!(f, ServerFrame::Event(Event::MessageCreated { .. })) {
            created += 1;
        }
    }
    assert_eq!(created, 1);
}

#[tokio::test]
async fn concurrent_duplicates_post_once() {
    let t = setup().await;
    let (r1, r2) = futures::future::join(
        send_nonce(&t.app, &t.a, t.g, "hi", "n-race"),
        send_nonce(&t.app, &t.a, t.g, "hi", "n-race"),
    )
    .await;
    assert_eq!(r1.status(), 200);
    assert_eq!(r2.status(), 200);
    let (m1, m2): (Message, Message) = (r1.json().await.unwrap(), r2.json().await.unwrap());
    assert_eq!(m1.id, m2.id);
    assert_eq!(normal_rows(&t.app, t.g).await, 1);
}

#[tokio::test]
async fn same_nonce_from_different_authors_are_different_messages() {
    let t = setup().await;
    let m1: Message = send_nonce(&t.app, &t.a, t.g, "x", "same")
        .await
        .json()
        .await
        .unwrap();
    let m2: Message = send_nonce(&t.app, &t.b, t.g, "x", "same")
        .await
        .json()
        .await
        .unwrap();
    assert_ne!(m1.id, m2.id);
    assert_eq!(normal_rows(&t.app, t.g).await, 2);
}

#[tokio::test]
async fn sends_without_nonce_are_never_deduplicated() {
    let t = setup().await;
    send(&t.app, &t.a, t.g, "same text").await;
    send(&t.app, &t.a, t.g, "same text").await;
    assert_eq!(normal_rows(&t.app, t.g).await, 2);
}

/// The send landed but the client saw a 502; the user deleted the message (from another window)
/// before the outbox retried. The retry must not resurrect it as a new message.
#[tokio::test]
async fn retry_after_delete_returns_the_deleted_original() {
    let t = setup().await;
    let first: Message = send_nonce(&t.app, &t.a, t.g, "oops", "n-del")
        .await
        .json()
        .await
        .unwrap();
    let del = t
        .app
        .http
        .delete(t.app.url(&format!("/messages/{}", first.id)))
        .bearer_auth(&t.a)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 204);

    let r = send_nonce(&t.app, &t.a, t.g, "oops", "n-del").await;
    assert_eq!(r.status(), 200);
    let again: Message = r.json().await.unwrap();
    assert_eq!(again.id, first.id);
    assert!(again.deleted);
    assert_eq!(again.content, "", "a deleted message never exposes content");
    assert_eq!(normal_rows(&t.app, t.g).await, 1);
}

#[tokio::test]
async fn oversized_nonce_is_400() {
    let t = setup().await;
    let r = send_nonce(&t.app, &t.a, t.g, "x", &"n".repeat(65)).await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn empty_nonce_is_400() {
    let t = setup().await;
    let r = send_nonce(&t.app, &t.a, t.g, "x", "").await;
    assert_eq!(r.status(), 400);
}

/// A nonce names one send, so a retry gets the original whatever it carries: the outbox never
/// changes a queued message, and an edit goes through its own route.
#[tokio::test]
async fn same_nonce_with_other_content_returns_the_original() {
    let t = setup().await;
    let first: Message = send_nonce(&t.app, &t.a, t.g, "hello", "n-text")
        .await
        .json()
        .await
        .unwrap();
    let again: Message = send_nonce(&t.app, &t.a, t.g, "goodbye", "n-text")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(again.content, "hello");
    assert_eq!(normal_rows(&t.app, t.g).await, 1);
}

/// A nonce reused in another channel isn't a retry of that send: 409, and the other channel's
/// message isn't handed back as if it had been posted here.
#[tokio::test]
async fn same_nonce_in_another_channel_is_409() {
    let t = setup().await;
    let r = send_nonce(&t.app, &t.a, t.g, "x", "n-chan").await;
    assert_eq!(r.status(), 200);
    let dm = create_dm(&t.app, &t.a, &[t.b_id]).await.id;

    let r = send_nonce(&t.app, &t.a, dm, "x", "n-chan").await;

    assert_eq!(r.status(), 409);
    assert_eq!(normal_rows(&t.app, dm).await, 0);
}
