//! Attachments (#57): sending messages with files, their links, deleting them and the sweep.
mod common;

use common::*;
use pulse_protocol::ids::{AttachmentId, ChannelId, MessageId, UserId};
use pulse_protocol::rest::{
    AttachmentKind, AttachmentUpload, AttachmentUrl, Channel, Message, SearchResults,
};
use serde_json::json;

struct World {
    app: TestApp,
    alex: (UserId, String),
    sam: (UserId, String),
    server: pulse_protocol::ids::ServerId,
    text: Channel,
}

async fn world() -> World {
    let app = spawn().await;
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let s = create_server(&app, &alex.1, "Main").await;
    let r = post_json(&app, &sam.1, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);
    let text = general(&app, &alex.1, s.id).await;
    World {
        app,
        alex,
        sam,
        server: s.id,
        text,
    }
}

/// The smallest WebP header the signature check looks at, padded out.
fn webp(len: usize) -> Vec<u8> {
    let mut b = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
    b.resize(len.max(16), 7);
    b
}

/// Start an upload and PUT `bytes` to it, as the client does.
async fn upload(
    app: &TestApp,
    token: &str,
    channel: ChannelId,
    name: &str,
    content_type: &str,
    bytes: &[u8],
) -> AttachmentId {
    let up = start(app, token, channel, name, content_type, bytes.len() as u64).await;
    put(&up, bytes).await;
    up.id
}

async fn start(
    app: &TestApp,
    token: &str,
    channel: ChannelId,
    name: &str,
    content_type: &str,
    size: u64,
) -> AttachmentUpload {
    let r = post_json(
        app,
        token,
        &format!("/channels/{channel}/attachments"),
        json!({ "name": name, "size": size, "content_type": content_type }),
    )
    .await;
    assert_eq!(r.status(), 200, "start {name}");
    r.json().await.unwrap()
}

async fn put(up: &AttachmentUpload, bytes: &[u8]) {
    let r = reqwest::Client::new()
        .put(up.url.clone().unwrap())
        .header("content-type", &up.content_type)
        .body(bytes.to_vec())
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "PUT {}", r.status());
}

async fn send_files(
    app: &TestApp,
    token: &str,
    channel: ChannelId,
    content: &str,
    ids: &[AttachmentId],
) -> reqwest::Response {
    post_json(
        app,
        token,
        &format!("/channels/{channel}/messages"),
        json!({ "content": content, "attachments": ids }),
    )
    .await
}

async fn send_ok(
    app: &TestApp,
    token: &str,
    channel: ChannelId,
    content: &str,
    ids: &[AttachmentId],
) -> Message {
    let r = send_files(app, token, channel, content, ids).await;
    assert_eq!(r.status(), 200, "send with files");
    r.json().await.unwrap()
}

async fn error_of(r: reqwest::Response) -> String {
    let v: serde_json::Value = r.json().await.unwrap();
    v["message"].as_str().unwrap_or_default().to_string()
}

async fn urls(app: &TestApp, token: &str, message: MessageId) -> reqwest::Response {
    app.http
        .get(app.url(&format!("/messages/{message}/attachments/urls")))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
}

async fn keys(app: &TestApp, message: MessageId) -> Vec<String> {
    sqlx::query_scalar("SELECT key FROM attachments WHERE message_id = ? ORDER BY position")
        .bind(message.to_string())
        .fetch_all(&app.db)
        .await
        .unwrap()
}

fn s3(app: &TestApp) -> &pulse_server::testing::fake_s3::FakeS3 {
    app.s3.as_ref().unwrap()
}

#[tokio::test]
async fn sends_a_message_with_three_images_and_a_pdf() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    let mut ids = vec![];
    for i in 0..3 {
        ids.push(
            upload(
                app,
                t,
                ch,
                &format!("shot{i}.webp"),
                "image/webp",
                &webp(100 + i),
            )
            .await,
        );
    }
    ids.push(upload(app, t, ch, "report.pdf", "application/pdf", b"%PDF-1.4").await);
    let m = send_ok(app, t, ch, "look", &ids).await;
    assert_eq!(m.content, "look");
    assert!(!m.caption_is_fallback);
    let names: Vec<&str> = m.attachments.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(
        names,
        ["shot0.webp", "shot1.webp", "shot2.webp", "report.pdf"]
    );
    assert_eq!(m.attachments[0].kind, AttachmentKind::Image);
    assert_eq!(m.attachments[0].size, 100);
    assert_eq!(m.attachments[3].kind, AttachmentKind::File);
    assert_eq!(m.attachments[3].content_type, "application/octet-stream");
    // the list carries them too, in order
    let list: Vec<Message> = get_json(app, &w.sam.1, &format!("/channels/{ch}/messages")).await;
    let got = list.iter().find(|x| x.id == m.id).unwrap();
    assert_eq!(got.attachments, m.attachments);
}

#[tokio::test]
async fn attachment_only_message_has_fallback_content() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    let id = upload(app, t, ch, "report.pdf", "application/pdf", b"%PDF").await;
    let m = send_ok(app, t, ch, "  ", &[id]).await;
    // what a 0.2.x client shows (and notifications and search)
    assert_eq!(m.content, "📎 report.pdf");
    assert!(m.caption_is_fallback);
    let ids: Vec<AttachmentId> = futures_ids(app, t, ch, 3).await;
    let m = send_ok(app, t, ch, "", &ids).await;
    assert_eq!(m.content, "📎 3 images");
    // a captioned message's JSON says nothing about a fallback
    let id = upload(app, t, ch, "a.txt", "text/plain", b"hi").await;
    let r = send_files(app, t, ch, "with words", &[id]).await;
    let v: serde_json::Value = r.json().await.unwrap();
    assert!(v.get("caption_is_fallback").is_none(), "{v}");
    // and a message without files has no attachments key at all
    let r = send_files(app, t, ch, "plain", &[]).await;
    let v: serde_json::Value = r.json().await.unwrap();
    assert!(v.get("attachments").is_none(), "{v}");
}

async fn futures_ids(app: &TestApp, t: &str, ch: ChannelId, n: usize) -> Vec<AttachmentId> {
    let mut ids = vec![];
    for i in 0..n {
        ids.push(upload(app, t, ch, &format!("{i}.webp"), "image/webp", &webp(20)).await);
    }
    ids
}

#[tokio::test]
async fn eleven_attachments_are_refused() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    let mut ids = vec![];
    for i in 0..11 {
        ids.push(upload(app, t, ch, &format!("{i}.txt"), "text/plain", b"x").await);
    }
    let r = send_files(app, t, ch, "", &ids).await;
    assert_eq!(r.status(), 400);
    assert_eq!(error_of(r).await, "Up to 10 files per message.");
    // ten is fine
    send_ok(app, t, ch, "", &ids[..10]).await;
}

#[tokio::test]
async fn send_with_missing_object_is_409_and_posts_nothing() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    let up = start(app, t, ch, "world.zip", "application/zip", 10).await;
    let r = send_files(app, t, ch, "", &[up.id]).await;
    assert_eq!(r.status(), 409);
    assert_eq!(error_of(r).await, "The upload didn't arrive. Try again.");
    let list: Vec<Message> = get_json(app, t, &format!("/channels/{ch}/messages")).await;
    assert!(list.is_empty());
    // still pending, for the sweep (or a retry once the PUT lands)
    put(&up, &[1; 10]).await;
    send_ok(app, t, ch, "", &[up.id]).await;
}

#[tokio::test]
async fn size_mismatch_and_fake_webp_are_409() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    let up = start(app, t, ch, "a.txt", "text/plain", 5).await;
    let key: String = sqlx::query_scalar("SELECT key FROM attachments WHERE id = ?")
        .bind(up.id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    s3(app).put_raw(&format!("attachments/{key}"), b"longer than five".to_vec());
    let r = send_files(app, t, ch, "", &[up.id]).await;
    assert_eq!(r.status(), 409);
    assert_eq!(error_of(r).await, "The upload didn't match. Try again.");

    let id = upload(
        app,
        t,
        ch,
        "x.webp",
        "image/webp",
        b"<html>not an image at all",
    )
    .await;
    let r = send_files(app, t, ch, "", &[id]).await;
    assert_eq!(r.status(), 409);
    assert_eq!(
        error_of(r).await,
        "That image didn't upload correctly. Try again."
    );
}

#[tokio::test]
async fn foreign_or_wrong_channel_upload_is_refused() {
    let w = world().await;
    let app = &w.app;
    let other = post_json(
        app,
        &w.alex.1,
        &format!("/servers/{}/channels", w.server),
        json!({ "kind": "text", "name": "other" }),
    )
    .await
    .json::<Channel>()
    .await
    .unwrap();
    let mine = upload(app, &w.alex.1, w.text.id, "a.txt", "text/plain", b"x").await;
    // sam can't send alex's upload
    assert_eq!(
        send_files(app, &w.sam.1, w.text.id, "", &[mine])
            .await
            .status(),
        404
    );
    // nor can alex send it somewhere it wasn't started for
    assert_eq!(
        send_files(app, &w.alex.1, other.id, "", &[mine])
            .await
            .status(),
        404
    );
    let n: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM attachments WHERE message_id IS NOT NULL")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn an_attachment_can_only_be_sent_once_but_a_retry_gets_the_message() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    let id = upload(app, t, ch, "a.txt", "text/plain", b"x").await;
    let body = json!({ "content": "", "attachments": [id], "nonce": "n-1" });
    let r = post_json(app, t, &format!("/channels/{ch}/messages"), body.clone()).await;
    assert_eq!(r.status(), 200);
    let first: Message = r.json().await.unwrap();
    // the client's retry after a lost answer: the same message back
    let r = post_json(app, t, &format!("/channels/{ch}/messages"), body).await;
    assert_eq!(r.status(), 200);
    let again: Message = r.json().await.unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(again.attachments, first.attachments);
    // a new send can't reuse it
    assert_eq!(send_files(app, t, ch, "", &[id]).await.status(), 404);
}

#[tokio::test]
async fn editing_can_clear_the_caption() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    let id = upload(app, t, ch, "report.pdf", "application/pdf", b"%PDF").await;
    let m = send_ok(app, t, ch, "draft numbers", &[id]).await;
    let r = app
        .http
        .patch(app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(t)
        .json(&json!({ "content": "" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let e: Message = r.json().await.unwrap();
    assert_eq!(e.content, "📎 report.pdf");
    assert!(e.caption_is_fallback);
    assert_eq!(e.attachments.len(), 1);
    // a plain message still needs text
    let p = send(app, t, ch, "plain").await;
    let r = app
        .http
        .patch(app.url(&format!("/messages/{}", p.id)))
        .bearer_auth(t)
        .json(&json!({ "content": "" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn urls_are_signed_per_object_and_short_lived() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    let a = upload(app, t, ch, "a.webp", "image/webp", &webp(30)).await;
    let b = upload(app, t, ch, "b.txt", "text/plain", b"hello").await;
    let m = send_ok(app, t, ch, "", &[a, b]).await;
    let r = urls(app, &w.sam.1, m.id).await;
    assert_eq!(r.status(), 200);
    let got: Vec<AttachmentUrl> = r.json().await.unwrap();
    assert_eq!(got.iter().map(|u| u.id).collect::<Vec<_>>(), vec![a, b]);
    let k = keys(app, m.id).await;
    for (u, key) in got.iter().zip(&k) {
        let url = reqwest::Url::parse(u.url.as_deref().unwrap()).unwrap();
        assert!(
            url.path().ends_with(&format!("/attachments/{key}")),
            "{}",
            url.path()
        );
        let ttl = url
            .query_pairs()
            .find(|(k, _)| k == "X-Amz-Expires")
            .unwrap()
            .1
            .to_string();
        assert_eq!(ttl, "3600");
    }
    let body = reqwest::get(got[1].url.clone().unwrap())
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(&body[..], b"hello");
}

#[tokio::test]
async fn only_people_in_the_chat_get_links() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    let a = upload(app, t, ch, "a.txt", "text/plain", b"x").await;
    let m = send_ok(app, t, ch, "", &[a]).await;
    // a server channel is visible to all, so a non-member is refused (as clip links are)
    let (_, kim) = register(app, "kim").await;
    assert_eq!(urls(app, &kim, m.id).await.status(), 403);
    // a DM's files are a 404 to anyone outside it: they can't learn it exists
    let dm = create_dm(app, t, &[w.sam.0]).await;
    let b = upload(app, t, dm.id, "b.txt", "text/plain", b"y").await;
    let d = send_ok(app, t, dm.id, "", &[b]).await;
    assert_eq!(urls(app, &w.sam.1, d.id).await.status(), 200);
    assert_eq!(urls(app, &kim, d.id).await.status(), 404);
}

#[tokio::test]
async fn deleting_a_message_deletes_its_objects() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    let a = upload(app, t, ch, "a.txt", "text/plain", b"x").await;
    let b = upload(app, t, ch, "b.webp", "image/webp", &webp(20)).await;
    let m = send_ok(app, t, ch, "bye", &[a, b]).await;
    let k = keys(app, m.id).await;
    let r = app
        .http
        .delete(app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(t)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    for key in &k {
        assert!(
            s3(app).object(&format!("attachments/{key}")).is_none(),
            "{key} still stored"
        );
    }
    assert!(keys(app, m.id).await.is_empty());
    assert_eq!(urls(app, t, m.id).await.status(), 404);
}

#[tokio::test]
async fn purging_a_channel_deletes_its_attachments() {
    let w = world().await;
    let app = &w.app;
    let t = &w.alex.1;
    let ch = post_json(
        app,
        t,
        &format!("/servers/{}/channels", w.server),
        json!({ "kind": "text", "name": "temp" }),
    )
    .await
    .json::<Channel>()
    .await
    .unwrap();
    let sent = upload(app, t, ch.id, "a.txt", "text/plain", b"x").await;
    send_ok(app, t, ch.id, "", &[sent]).await;
    upload(app, t, ch.id, "pending.txt", "text/plain", b"y").await;
    assert_eq!(s3(app).keys().len(), 2);
    let r = app
        .http
        .delete(app.url(&format!("/channels/{}", ch.id)))
        .bearer_auth(t)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "delete {}", r.status());
    let r = app
        .http
        .delete(app.url(&format!("/channels/{}/permanent", ch.id)))
        .bearer_auth(t)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    // the objects go after the commit, on a task
    for _ in 0..50 {
        if s3(app).keys().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(s3(app).keys().is_empty(), "{:?}", s3(app).keys());
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachments")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

async fn age(app: &TestApp, hours: i64) {
    sqlx::query("UPDATE attachments SET created_at = ?")
        .bind(pulse_server::db::stamp(
            chrono::Utc::now() - chrono::Duration::hours(hours),
        ))
        .execute(&app.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn sweep_removes_unattached_uploads_after_24h() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    upload(app, t, ch, "a.txt", "text/plain", b"x").await;
    let kept = upload(app, t, ch, "b.txt", "text/plain", b"y").await;
    send_ok(app, t, ch, "", &[kept]).await;
    age(app, 23).await;
    let r = pulse_server::attachments::sweep::run(&app.state(), chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(r.pending, 0);
    age(app, 26).await;
    let r = pulse_server::attachments::sweep::run(&app.state(), chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(r.pending, 1);
    // the sent one stays, object and row
    assert_eq!(s3(app).keys().len(), 1);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachments")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn sweep_retries_a_failed_delete() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    let a = upload(app, t, ch, "a.txt", "text/plain", b"x").await;
    let m = send_ok(app, t, ch, "", &[a]).await;
    s3(app).set_down(true);
    let r = app
        .http
        .delete(app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(t)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204, "the delete lands even with storage down");
    let r = pulse_server::attachments::sweep::run(&app.state(), chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(r.deleted, 0);
    assert_eq!(keys(app, m.id).await.len(), 1, "kept for the next run");
    s3(app).set_down(false);
    let r = pulse_server::attachments::sweep::run(&app.state(), chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(r.deleted, 1);
    assert!(s3(app).keys().is_empty());
}

#[tokio::test]
async fn has_media_and_has_file_filter_search() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    let img = upload(app, t, ch, "cat.webp", "image/webp", &webp(20)).await;
    let media = send_ok(app, t, ch, "a cat", &[img]).await;
    let doc = upload(app, t, ch, "cat facts.pdf", "application/pdf", b"%PDF").await;
    let file = send_ok(app, t, ch, "", &[doc]).await;
    send(app, t, ch, "just a cat").await;
    let ids = |r: SearchResults| r.hits.iter().map(|h| h.message.id).collect::<Vec<_>>();
    let r: SearchResults =
        get_json(app, t, &format!("/servers/{}/search?has=media", w.server)).await;
    assert_eq!(ids(r), vec![media.id]);
    let r: SearchResults =
        get_json(app, t, &format!("/servers/{}/search?has=file", w.server)).await;
    assert_eq!(ids(r), vec![file.id]);
    // the fallback text is searchable: a file is found by its name
    let r: SearchResults = get_json(app, t, &format!("/servers/{}/search?q=facts", w.server)).await;
    let r = ids(r);
    assert_eq!(r, vec![file.id]);
}
