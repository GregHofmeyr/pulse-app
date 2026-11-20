//! What someone can still change after they leave a server or are removed from it (security review, post-0.2.12).
//! Its channels are hidden from them now (#80: 404, as for a server that isn't there), so their old messages, polls and
//! plans are no longer theirs to edit; deleting their own message stays allowed (#418).
mod common;

use common::*;
use pulse_protocol::ids::{MessageId, ServerId, UserId};
use pulse_protocol::rest::{Channel, InboxThreads, InboxUnread, Message, MessageReminder};
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
    let r = join_server(&app, &sam.1, s.id).await;
    assert_eq!(r.status(), 200);
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
    let starts =
        pulse_server::times::rfc3339(jiff::Timestamp::now() + pulse_server::times::days(3));
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
    assert_eq!(r.status(), 404, "edit");
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/messages/{poll}/poll/close"),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 404, "close");
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
    let starts =
        pulse_server::times::rfc3339(jiff::Timestamp::now() + pulse_server::times::days(4));
    let r = put_json(
        &w.app,
        &w.sam.1,
        &format!("/messages/{plan}/plan"),
        json!({"title": "Moved", "starts_at": starts}),
    )
    .await;
    assert_eq!(r.status(), 404, "edit");
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/messages/{plan}/plan/cancel"),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 404, "cancel");
    let (title, cancelled): (String, Option<String>) =
        sqlx::query_as("SELECT title, cancelled_at FROM plans WHERE message_id = ?")
            .bind(plan.to_string())
            .fetch_one(&w.app.db)
            .await
            .unwrap();
    assert_eq!(title, "Braai");
    assert_eq!(cancelled, None);
}

async fn leave_sam(w: &World) {
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/servers/{}/leave", w.server),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 204);
}

#[tokio::test]
async fn threads_in_a_server_you_left_leave_your_inbox() {
    let w = world().await;
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/servers/{}/channels", w.server),
        json!({ "kind": "quotes", "name": "quotes" }),
    )
    .await;
    let quotes: Channel = r.json().await.unwrap();
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/channels/{}/quotes", quotes.id),
        json!({ "lines": [{ "speaker_id": w.alex.0, "text": "x" }] }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let quote: Message = r.json().await.unwrap();
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/channels/{}/messages", quotes.id),
        json!({ "content": "ha", "thread_root_id": quote.id }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let t: InboxThreads = get_json(&w.app, &w.sam.1, "/inbox/threads").await;
    assert_eq!((t.watching.len(), t.threads.len()), (1, 1));
    let n: InboxUnread = get_json(&w.app, &w.sam.1, "/inbox/unread").await;
    assert_eq!(n.unread, 1);

    leave_sam(&w).await;
    let t: InboxThreads = get_json(&w.app, &w.sam.1, "/inbox/threads").await;
    assert!(t.watching.is_empty() && t.threads.is_empty(), "{t:?}");
    let n: InboxUnread = get_json(&w.app, &w.sam.1, "/inbox/unread").await;
    assert_eq!(n.unread, 0);
}

#[tokio::test]
async fn no_reminders_in_a_server_you_left() {
    let w = world().await;
    let m = send(&w.app, &w.alex.1, w.general.id, "remember me").await;
    leave_sam(&w).await;
    let at =
        pulse_server::times::rfc3339(jiff::Timestamp::now() + jiff::SignedDuration::from_mins(30));
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/messages/{}/reminders", m.id),
        json!({ "remind_at": at }),
    )
    .await;
    assert_eq!(
        r.status(),
        404,
        "a server's messages are for its members to be reminded of"
    );

    // and one left over (made before the leave, kept by a race) isn't listed or counted
    sqlx::query(
        "INSERT INTO message_reminders (id, user_id, message_id, remind_at, created_at, fired_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(pulse_protocol::ids::ReminderId::new().to_string())
    .bind(w.sam.0.to_string())
    .bind(m.id.to_string())
    .bind("2026-01-01T00:00:00.000Z")
    .bind("2026-01-01T00:00:00.000Z")
    .bind("2026-01-01T00:00:00.000Z")
    .execute(&w.app.db)
    .await
    .unwrap();
    let mine: Vec<MessageReminder> = get_json(&w.app, &w.sam.1, "/reminders").await;
    assert!(mine.is_empty(), "{mine:?}");
    let n: InboxUnread = get_json(&w.app, &w.sam.1, "/inbox/unread").await;
    assert_eq!(n.unread, 0);
}
