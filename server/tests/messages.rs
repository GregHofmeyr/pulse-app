mod common;

use common::*;
use pulse_protocol::rest::Message;
use serde_json::json;

async fn setup() -> (TestApp, String, String, pulse_protocol::rest::Channel) {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let s = create_server(&app, &a, "Main").await;
    post_json(&app, &b, &format!("/servers/{}/join", s.id), json!({})).await;
    let g = general(&app, &a, s.id).await;
    (app, a, b, g)
}

#[tokio::test]
async fn send_then_list_newest_first() {
    let (app, a, b, g) = setup().await;
    let m1 = send(&app, &a, g.id, "first").await;
    let m2 = send(&app, &b, g.id, "second").await;
    let list: Vec<Message> = get_json(&app, &a, &format!("/channels/{}/messages", g.id)).await;
    assert_eq!(
        list.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![m2.id, m1.id]
    );
}

#[tokio::test]
async fn pagination_before_cursor() {
    let (app, a, _, g) = setup().await;
    for i in 0..120 {
        send(&app, &a, g.id, &format!("m{i}")).await;
    }
    let p1: Vec<Message> = get_json(&app, &a, &format!("/channels/{}/messages", g.id)).await;
    assert_eq!(p1.len(), 50);
    assert_eq!(p1[0].content, "m119");
    let p2: Vec<Message> = get_json(
        &app,
        &a,
        &format!("/channels/{}/messages?before={}", g.id, p1[49].id),
    )
    .await;
    assert_eq!(p2.len(), 50);
    assert_eq!(p2[0].content, "m69");
    let p3: Vec<Message> = get_json(
        &app,
        &a,
        &format!("/channels/{}/messages?before={}&limit=100", g.id, p2[49].id),
    )
    .await;
    assert_eq!(p3.len(), 20);
    assert_eq!(p3[19].content, "m0");
}

#[tokio::test]
async fn edit_own_sets_edited_at() {
    let (app, a, _, g) = setup().await;
    let m = send(&app, &a, g.id, "tpyo").await;
    let r = app
        .http
        .patch(app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(&a)
        .json(&json!({"content": "typo"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let e: Message = r.json().await.unwrap();
    assert_eq!(e.content, "typo");
    assert!(e.edited_at.is_some());
}

#[tokio::test]
async fn edit_or_delete_others_403() {
    let (app, a, b, g) = setup().await;
    let m = send(&app, &a, g.id, "mine").await;
    let r = app
        .http
        .patch(app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(&b)
        .json(&json!({"content": "hijack"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let r = app
        .http
        .delete(app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(&b)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
}

#[tokio::test]
async fn delete_own_soft_deletes() {
    let (app, a, _, g) = setup().await;
    let m = send(&app, &a, g.id, "oops").await;
    let r = app
        .http
        .delete(app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(&a)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let list: Vec<Message> = get_json(&app, &a, &format!("/channels/{}/messages", g.id)).await;
    assert_eq!(list.len(), 1);
    assert!(list[0].deleted);
    assert_eq!(list[0].content, "");
}

#[tokio::test]
async fn edit_or_delete_deleted_404() {
    let (app, a, _, g) = setup().await;
    let m = send(&app, &a, g.id, "oops").await;
    app.http
        .delete(app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(&a)
        .send()
        .await
        .unwrap();
    let r = app
        .http
        .patch(app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(&a)
        .json(&json!({"content": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let r = app
        .http
        .delete(app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(&a)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn reply_must_be_same_channel_400() {
    let (app, a, _, g) = setup().await;
    let s2 = create_server(&app, &a, "Other").await;
    let g2 = general(&app, &a, s2.id).await;
    let elsewhere = send(&app, &a, g2.id, "over here").await;
    let r = post_json(
        &app,
        &a,
        &format!("/channels/{}/messages", g.id),
        json!({"content": "re", "reply_to_id": elsewhere.id}),
    )
    .await;
    assert_eq!(r.status(), 400);
    let here = send(&app, &a, g.id, "here").await;
    let r = post_json(
        &app,
        &a,
        &format!("/channels/{}/messages", g.id),
        json!({"content": "re", "reply_to_id": here.id}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let m: Message = r.json().await.unwrap();
    assert_eq!(m.reply_to_id, Some(here.id));
}

#[tokio::test]
async fn empty_or_too_long_400() {
    let (app, a, _, g) = setup().await;
    let path = format!("/channels/{}/messages", g.id);
    assert_eq!(
        post_json(&app, &a, &path, json!({"content": "   "}))
            .await
            .status(),
        400
    );
    assert_eq!(
        post_json(&app, &a, &path, json!({"content": "x".repeat(4001)}))
            .await
            .status(),
        400
    );
}

#[tokio::test]
async fn cannot_post_in_voice_channel_400() {
    let (app, a, _, g) = setup().await;
    let chans: Vec<pulse_protocol::rest::Channel> = get_json(
        &app,
        &a,
        &format!("/servers/{}/channels", g.server_id.unwrap()),
    )
    .await;
    let lounge = chans
        .iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    assert_eq!(
        post_json(
            &app,
            &a,
            &format!("/channels/{}/messages", lounge.id),
            json!({"content": "hi"})
        )
        .await
        .status(),
        400
    );
}

#[tokio::test]
async fn non_member_cannot_post_in_server_403_but_can_read() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (_, c) = register(&app, "jo").await;
    let s = create_server(&app, &a, "Main").await;
    let g = general(&app, &a, s.id).await;
    send(&app, &a, g.id, "hello").await;
    assert_eq!(
        post_json(
            &app,
            &c,
            &format!("/channels/{}/messages", g.id),
            json!({"content": "hi"})
        )
        .await
        .status(),
        403
    );
    let list: Vec<Message> = get_json(&app, &c, &format!("/channels/{}/messages", g.id)).await;
    assert_eq!(list.len(), 1);
}
