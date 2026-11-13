//! Writes racing a channel delete (#109). A write checks its channel, then takes the write lock; a
//! delete that commits in between must not let it land: the channel (or its server) is in Recently
//! deleted, or gone for good (a group whose last member left), so the write answers 404, writes
//! nothing and publishes nothing.
//!
//! The race is held open as in `server_delete.rs` (#287): the test takes the write lock with the
//! delete done but not committed, the request checks (it still sees the channel) and waits for the
//! lock, and the delete commits under it.
mod common;

use std::time::Duration;

use common::*;
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use pulse_protocol::rest::{Channel, ClipUpload, Message};
use serde_json::json;

struct World {
    app: TestApp,
    alex: (UserId, String),
    sam: (UserId, String),
    server: ServerId,
}

async fn world() -> World {
    let app = spawn().await;
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let s = create_server(&app, &alex.1, "Main").await;
    let r = post_json(&app, &sam.1, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);
    World {
        app,
        alex,
        sam,
        server: s.id,
    }
}

async fn create_channel(w: &World, kind: &str) -> Channel {
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/servers/{}/channels", w.server),
        json!({ "kind": kind, "name": kind }),
    )
    .await;
    assert_eq!(r.status(), 200, "create {kind} channel");
    r.json().await.unwrap()
}

async fn count(app: &TestApp, q: &str, id: ChannelId) -> i64 {
    sqlx::query_scalar(q)
        .bind(id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn request(
    app: &TestApp,
    token: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> reqwest::Response {
    let mut r = app.http.request(method, app.url(path)).bearer_auth(token);
    if let Some(b) = body {
        r = r.json(&b);
    }
    r.send().await.unwrap()
}

/// Run `req` while `delete` (SQL on the channel, bound to its id) is held uncommitted, committing
/// it once the request has had time to pass its checks and queue for the write lock.
async fn racing<F: std::future::Future<Output = reqwest::Response>>(
    app: &TestApp,
    delete: &str,
    channel: ChannelId,
    req: F,
) -> (u16, String) {
    let mut del = pulse_server::db::begin_write(&app.db).await.unwrap();
    sqlx::query(delete)
        .bind(pulse_server::db::now())
        .bind(channel.to_string())
        .execute(&mut *del)
        .await
        .unwrap();
    let (r, ()) = tokio::join!(req, async {
        // the request checks (the delete isn't committed, so the channel is there) and then
        // waits for the write lock; the delete commits under it
        tokio::time::sleep(Duration::from_millis(300)).await;
        del.commit().await.unwrap();
    });
    let status = r.status().as_u16();
    (status, r.text().await.unwrap())
}

/// A racing write: what it is, its channel, method, path and body, and the count (bound to the
/// channel's id) it must leave as it was.
type Case = (
    &'static str,
    ChannelId,
    reqwest::Method,
    String,
    Option<serde_json::Value>,
    &'static str,
);

const SOFT_DELETE: &str = "UPDATE channels SET deleted_at = ?1 WHERE id = ?2";

async fn restore(app: &TestApp, channel: ChannelId) {
    sqlx::query("UPDATE channels SET deleted_at = NULL, deleted_by = NULL WHERE id = ?")
        .bind(channel.to_string())
        .execute(&app.db)
        .await
        .unwrap();
}

/// Everything that writes into a channel answers 404 and writes nothing when the channel went to
/// Recently deleted between its check and its write. Before #109 they landed in the deleted
/// channel (to come back on restore, or to break its guards: a quote voted onto a deleted channel's
/// ballot).
#[tokio::test]
async fn a_write_that_raced_a_channel_delete_writes_nothing() {
    let w = world().await;
    let general = general(&w.app, &w.alex.1, w.server).await.id;
    let quotes = create_channel(&w, "quotes").await.id;
    let clips = create_channel(&w, "clips").await.id;
    let m = send(&w.app, &w.alex.1, general, "hello").await;
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/channels/{general}/polls"),
        json!({"question": "Pizza?", "options": ["Yes", "No"], "ends_in": "day"}),
    )
    .await;
    assert!(r.status().is_success(), "poll");
    let poll: Message = r.json().await.unwrap();
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/channels/{quotes}/quotes"),
        json!({ "lines": [{ "speaker_id": w.sam.0, "text": "first" }] }),
    )
    .await;
    assert_eq!(r.status(), 200, "quote");
    let quote: Message = r.json().await.unwrap();

    use reqwest::Method;
    let messages = "SELECT COUNT(*) FROM messages WHERE channel_id = ?";
    let cases: Vec<Case> = vec![
        (
            "send",
            general,
            Method::POST,
            format!("/channels/{general}/messages"),
            Some(json!({ "content": "late" })),
            messages,
        ),
        (
            "reply in a thread",
            quotes,
            Method::POST,
            format!("/channels/{quotes}/messages"),
            Some(json!({ "content": "late", "thread_root_id": quote.id })),
            messages,
        ),
        (
            "edit",
            general,
            Method::PATCH,
            format!("/messages/{}", m.id),
            Some(json!({ "content": "edited" })),
            "SELECT COUNT(*) FROM messages WHERE channel_id = ? AND edited_at IS NOT NULL",
        ),
        (
            "delete a message",
            general,
            Method::DELETE,
            format!("/messages/{}", m.id),
            None,
            "SELECT COUNT(*) FROM messages WHERE channel_id = ? AND deleted_at IS NOT NULL",
        ),
        (
            "reaction",
            general,
            Method::PUT,
            format!("/messages/{}/reactions/%F0%9F%91%8D", m.id),
            None,
            "SELECT COUNT(*) FROM reactions r JOIN messages m ON m.id = r.message_id
             WHERE m.channel_id = ?",
        ),
        (
            "pin",
            general,
            Method::PUT,
            format!("/messages/{}/pin", m.id),
            Some(json!({ "duration": "day" })),
            "SELECT COUNT(*) FROM pins WHERE channel_id = ?",
        ),
        (
            "poll",
            general,
            Method::POST,
            format!("/channels/{general}/polls"),
            Some(json!({"question": "Late?", "options": ["Yes", "No"], "ends_in": "day"})),
            messages,
        ),
        (
            "poll vote",
            general,
            Method::PUT,
            format!("/messages/{}/poll/votes", poll.id),
            Some(json!({ "options": [0] })),
            "SELECT COUNT(*) FROM poll_votes v JOIN polls p ON p.message_id = v.message_id
             WHERE p.channel_id = ?",
        ),
        (
            "poll close",
            general,
            Method::POST,
            format!("/messages/{}/poll/close", poll.id),
            Some(json!({})),
            "SELECT COUNT(*) FROM polls WHERE channel_id = ? AND closed_at IS NOT NULL",
        ),
        (
            "wheel spin",
            general,
            Method::POST,
            format!("/channels/{general}/wheel-spins"),
            Some(json!({ "options": ["Valheim", "Apex"] })),
            messages,
        ),
        (
            "attachment upload",
            general,
            Method::POST,
            format!("/channels/{general}/attachments"),
            Some(json!({ "name": "notes.txt", "size": 10, "content_type": "text/plain" })),
            "SELECT COUNT(*) FROM attachments WHERE channel_id = ?",
        ),
        (
            "quote",
            quotes,
            Method::POST,
            format!("/channels/{quotes}/quotes"),
            Some(json!({ "lines": [{ "speaker_id": w.sam.0, "text": "late" }] })),
            messages,
        ),
        (
            "quote edit",
            quotes,
            Method::PUT,
            format!("/quotes/{}", quote.id),
            Some(json!({ "lines": [{ "speaker_id": w.sam.0, "text": "edited" }] })),
            "SELECT COUNT(*) FROM messages WHERE channel_id = ? AND edited_at IS NOT NULL",
        ),
        (
            "clip upload ticket",
            clips,
            Method::POST,
            format!("/channels/{clips}/clips/uploads"),
            Some(json!({ "size": 1000, "duration_ms": 3000 })),
            "SELECT COUNT(*) FROM clip_uploads WHERE channel_id = ?",
        ),
    ];
    for (what, channel, method, path, body, rows) in cases {
        let before = count(&w.app, rows, channel).await;
        let (status, text) = racing(
            &w.app,
            SOFT_DELETE,
            channel,
            request(&w.app, &w.alex.1, method, &path, body),
        )
        .await;
        assert_eq!(status, 404, "{what}: {text}");
        assert_eq!(
            count(&w.app, rows, channel).await,
            before,
            "{what}: nothing written into the deleted channel"
        );
        restore(&w.app, channel).await;
    }
}

/// A clip confirm whose ticket was taken before the clips channel was deleted (#109): 404, no clip.
#[tokio::test]
async fn a_clip_confirm_that_raced_a_channel_delete_posts_nothing() {
    let w = world().await;
    let clips = create_channel(&w, "clips").await.id;
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/channels/{clips}/clips/uploads"),
        json!({ "size": 1000, "duration_ms": 3000 }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let up: ClipUpload = r.json().await.unwrap();
    let put = w
        .app
        .http
        .put(&up.url)
        .header("content-type", &up.content_type)
        .body(vec![7u8; 1000])
        .send()
        .await
        .unwrap();
    assert!(put.status().is_success());
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(32, 18, image::Rgba([9, 99, 199, 255]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let thumb: pulse_protocol::rest::MediaInfo = w
        .app
        .http
        .post(w.app.url("/media/clip-thumb"))
        .bearer_auth(&w.alex.1)
        .body(png.into_inner())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let body = json!({ "caption": "", "game_id": null, "duration_ms": 3000, "width": 1280,
                       "height": 720, "thumb_hash": thumb.hash, "nonce": "n-1" });
    let (status, text) = racing(
        &w.app,
        SOFT_DELETE,
        clips,
        post_json(
            &w.app,
            &w.alex.1,
            &format!("/clip-uploads/{}/confirm", up.id),
            body,
        ),
    )
    .await;
    assert_eq!(status, 404, "{text}");
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM messages WHERE channel_id = ?",
            clips
        )
        .await,
        0
    );
}

/// A group goes for good with its last member (a hard delete). A send that checked it just before
/// answered 500 (the insert failed the `channel_id` foreign key); now 404 (#109).
#[tokio::test]
async fn a_send_that_raced_a_hard_delete_is_a_404() {
    let w = world().await;
    let kim = register(&w.app, "kim").await;
    let group = create_dm(&w.app, &w.alex.1, &[w.sam.0, kim.0]).await;
    send(&w.app, &w.alex.1, group.id, "hi").await;
    let (status, text) = racing(
        &w.app,
        // `?1` (the time) unused: the row goes, its messages and members with it
        "DELETE FROM channels WHERE id = ?2 AND ?1 IS NOT NULL",
        group.id,
        post_json(
            &w.app,
            &w.alex.1,
            &format!("/channels/{}/messages", group.id),
            json!({ "content": "late" }),
        ),
    )
    .await;
    assert_eq!(status, 404, "{text}");
}
