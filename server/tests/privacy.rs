//! Privacy: outsiders must learn nothing about DMs/groups they are not in.

mod common;

use common::*;
use serde_json::json;

#[tokio::test]
async fn outsider_gets_404_everywhere() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let (_, c) = register(&app, "jo").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    let m = send(&app, &a, dm.id, "secret").await;

    let list = app
        .http
        .get(app.url(&format!("/channels/{}/messages", dm.id)))
        .bearer_auth(&c)
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), 404);
    let post = post_json(
        &app,
        &c,
        &format!("/channels/{}/messages", dm.id),
        json!({"content": "hi"}),
    )
    .await;
    assert_eq!(post.status(), 404);
    let patch = app
        .http
        .patch(app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(&c)
        .json(&json!({"content": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(patch.status(), 404);
    let del = app
        .http
        .delete(app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(&c)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 404);

    let (content, deleted): (String, Option<String>) =
        sqlx::query_as("SELECT content, deleted_at FROM messages WHERE id = ?")
            .bind(m.id.to_string())
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(content, "secret");
    assert!(deleted.is_none());
}

#[tokio::test]
async fn outsider_cannot_reply_into_private_message() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let (_, c) = register(&app, "jo").await;
    let s = create_server(&app, &c, "C's server").await;
    let g = general(&app, &c, s.id).await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    let m = send(&app, &a, dm.id, "secret").await;
    // replying to a private message from a channel C can post in must not confirm it exists
    let r = post_json(
        &app,
        &c,
        &format!("/channels/{}/messages", g.id),
        json!({"content": "re", "reply_to_id": m.id}),
    )
    .await;
    assert_eq!(r.status(), 400);
}
