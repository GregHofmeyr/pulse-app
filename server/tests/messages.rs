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

// ---- around / after ----

async fn seeded(n: usize) -> (TestApp, String, pulse_protocol::rest::Channel, Vec<Message>) {
    let (app, a, _, g) = setup().await;
    let mut all = Vec::new();
    for i in 1..=n {
        all.push(send(&app, &a, g.id, &format!("m{i}")).await);
    }
    (app, a, g, all)
}

async fn fetch(
    app: &TestApp,
    token: &str,
    ch: pulse_protocol::ids::ChannelId,
    q: &str,
) -> Vec<Message> {
    get_json(app, token, &format!("/channels/{ch}/messages?{q}")).await
}

fn contents(v: &[Message]) -> Vec<String> {
    v.iter().map(|m| m.content.clone()).collect()
}

fn range(from: usize, to: usize) -> Vec<String> {
    (from..=to).rev().map(|i| format!("m{i}")).collect()
}

#[tokio::test]
async fn around_returns_a_window_centred_on_the_target() {
    let (app, a, g, all) = seeded(60).await;
    let w = fetch(&app, &a, g.id, &format!("around={}&limit=20", all[29].id)).await;
    assert_eq!(contents(&w), range(20, 39));
}

#[tokio::test]
async fn around_near_the_ends_is_short() {
    let (app, a, g, all) = seeded(60).await;
    let w = fetch(&app, &a, g.id, &format!("around={}&limit=20", all[1].id)).await;
    assert_eq!(contents(&w), range(1, 11));
    let w = fetch(&app, &a, g.id, &format!("around={}&limit=20", all[59].id)).await;
    assert_eq!(contents(&w), range(50, 60));
    let w = fetch(&app, &a, g.id, &format!("around={}&limit=1", all[29].id)).await;
    assert_eq!(contents(&w), range(30, 30));
}

#[tokio::test]
async fn after_pages_forward() {
    let (app, a, g, all) = seeded(30).await;
    let w = fetch(&app, &a, g.id, &format!("after={}&limit=5", all[9].id)).await;
    assert_eq!(contents(&w), range(11, 15));
    let w = fetch(&app, &a, g.id, &format!("after={}&limit=5", all[29].id)).await;
    assert!(w.is_empty());
}

#[tokio::test]
async fn around_and_after_page_without_gaps() {
    let (app, a, g, all) = seeded(60).await;
    let mut seen: Vec<_> = fetch(&app, &a, g.id, &format!("around={}&limit=10", all[9].id)).await;
    for _ in 0..20 {
        let newest = seen[0].id;
        let next = fetch(&app, &a, g.id, &format!("after={newest}&limit=7")).await;
        if next.is_empty() {
            break;
        }
        let mut merged = next;
        merged.extend(seen);
        seen = merged;
    }
    assert_eq!(seen.first().unwrap().id, all[59].id);
    let ids: Vec<_> = seen.iter().map(|m| m.id).collect();
    let want: Vec<_> = all[4..].iter().rev().map(|m| m.id).collect();
    assert_eq!(ids, want);
}

#[tokio::test]
async fn around_refuses_a_reply_a_foreign_or_unknown_id() {
    let (app, a, g, all) = seeded(3).await;
    let reply = pulse_protocol::ids::MessageId::new();
    sqlx::query("INSERT INTO messages (id, channel_id, author_id, content, created_at, thread_root_id) VALUES (?, ?, NULL, 'r', '2026-01-01T00:00:00.000Z', ?)")
        .bind(reply.to_string())
        .bind(g.id.to_string())
        .bind(all[0].id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    let other = general_of_new_server(&app, &a).await;
    let foreign = send(&app, &a, other, "elsewhere").await;
    for id in [
        reply.to_string(),
        foreign.id.to_string(),
        pulse_protocol::ids::MessageId::new().to_string(),
    ] {
        for param in ["around", "after"] {
            let status = app
                .http
                .get(app.url(&format!("/channels/{}/messages?{param}={id}", g.id)))
                .bearer_auth(&a)
                .send()
                .await
                .unwrap()
                .status();
            // `after` is only a cursor, so it never needs the id to exist.
            let want = if param == "around" { 404 } else { 200 };
            assert_eq!(status, want, "{param}={id}");
        }
    }
}

async fn general_of_new_server(app: &TestApp, token: &str) -> pulse_protocol::ids::ChannelId {
    let s = create_server(app, token, "Other").await;
    general(app, token, s.id).await.id
}

#[tokio::test]
async fn around_accepts_system_and_deleted_anchors() {
    let (app, a, g, all) = seeded(3).await;
    let sys = pulse_protocol::ids::MessageId::new();
    sqlx::query("INSERT INTO messages (id, channel_id, kind, content, created_at) VALUES (?, ?, 'system', 'card', '2026-01-01T00:00:00.000Z')")
        .bind(sys.to_string())
        .bind(g.id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("UPDATE messages SET deleted_at = '2026-01-02T00:00:00.000Z' WHERE id = ?")
        .bind(all[1].id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    let w = fetch(&app, &a, g.id, &format!("around={sys}&limit=3")).await;
    assert_eq!(w[0].id, sys);
    assert_eq!(w.len(), 2);
    let w = fetch(&app, &a, g.id, &format!("around={}&limit=3", all[1].id)).await;
    assert_eq!(w.len(), 3);
    assert!(w[1].deleted && w[1].content.is_empty());
}

#[tokio::test]
async fn one_cursor_only() {
    let (app, a, g, all) = seeded(2).await;
    let (x, y) = (all[0].id, all[1].id);
    for q in [
        format!("before={x}&after={y}"),
        format!("before={x}&around={y}"),
        format!("after={x}&around={y}"),
    ] {
        let r = app
            .http
            .get(app.url(&format!("/channels/{}/messages?{q}", g.id)))
            .bearer_auth(&a)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 400, "{q}");
    }
}

#[tokio::test]
async fn around_needs_access() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (sam, _) = register(&app, "sam").await;
    let (_, jo) = register(&app, "jo").await;
    let dm = create_dm(&app, &a, &[sam]).await;
    let m = send(&app, &a, dm.id, "psst").await;
    let r = app
        .http
        .get(app.url(&format!("/channels/{}/messages?around={}", dm.id, m.id)))
        .bearer_auth(&jo)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn around_skips_replies() {
    let (app, a, _, g) = setup().await;
    let m1 = send(&app, &a, g.id, "m1").await;
    let m2 = send(&app, &a, g.id, "m2").await;
    for _ in 0..3 {
        sqlx::query("INSERT INTO messages (id, channel_id, author_id, content, created_at, thread_root_id) VALUES (?, ?, NULL, 'reply', '2026-01-01T00:00:00.000Z', ?)")
            .bind(pulse_protocol::ids::MessageId::new().to_string())
            .bind(g.id.to_string())
            .bind(m1.id.to_string())
            .execute(&app.db)
            .await
            .unwrap();
    }
    send(&app, &a, g.id, "m3").await;
    send(&app, &a, g.id, "m4").await;
    let w = fetch(&app, &a, g.id, &format!("around={}&limit=9", m2.id)).await;
    assert_eq!(contents(&w), range(1, 4));
    let w = fetch(&app, &a, g.id, &format!("after={}&limit=9", m2.id)).await;
    assert_eq!(contents(&w), range(3, 4));
}
