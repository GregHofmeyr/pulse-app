//! Attachments (#57): big temporary sends (50 MB to 2 GB, 1 or 7 days) in parts, the 20 GB cap
//! and expiry.
mod common;

use common::*;
use pulse_protocol::ids::{AttachmentId, ChannelId, UserId};
use pulse_protocol::rest::{
    AttachmentUpload, AttachmentUrl, Channel, Message, PartUrl, UploadedPart, attachment_limits,
};
use serde_json::json;

const MB: u64 = 1024 * 1024;
const PART: u64 = attachment_limits::PART_SIZE;
/// Just over the normal limit: a temporary send of four parts.
const BIG: u64 = attachment_limits::MAX_BYTES + 1;

struct World {
    app: TestApp,
    alex: (UserId, String),
    sam: (UserId, String),
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
    World { app, alex, sam, text }
}

async fn start(app: &TestApp, token: &str, ch: ChannelId, size: u64, days: Option<u8>) -> reqwest::Response {
    post_json(
        app,
        token,
        &format!("/channels/{ch}/attachments"),
        json!({ "name": "world.zip", "size": size, "content_type": "application/zip", "temp_days": days }),
    )
    .await
}

async fn start_ok(app: &TestApp, token: &str, ch: ChannelId, size: u64, days: u8) -> AttachmentUpload {
    let r = start(app, token, ch, size, Some(days)).await;
    assert_eq!(r.status(), 200, "start temp");
    r.json().await.unwrap()
}

async fn error_of(r: reqwest::Response) -> String {
    let v: serde_json::Value = r.json().await.unwrap();
    v["message"].as_str().unwrap_or_default().to_string()
}

async fn part_urls(app: &TestApp, token: &str, id: AttachmentId, parts: &[u16]) -> reqwest::Response {
    post_json(app, token, &format!("/attachment-uploads/{id}/parts"), json!({ "parts": parts })).await
}

async fn put_part(u: &PartUrl, byte: u8) {
    let r = reqwest::Client::new().put(&u.url).body(vec![byte; u.size as usize]).send().await.unwrap();
    assert!(r.status().is_success(), "PUT part {}: {}", u.part, r.status());
}

async fn uploaded(app: &TestApp, token: &str, id: AttachmentId) -> Vec<UploadedPart> {
    get_json(app, token, &format!("/attachment-uploads/{id}/parts")).await
}

async fn complete(app: &TestApp, token: &str, id: AttachmentId) -> reqwest::Response {
    post_json(app, token, &format!("/attachment-uploads/{id}/complete"), json!({})).await
}

/// Upload every part of a big file and complete it.
async fn upload_all(app: &TestApp, token: &str, up: &AttachmentUpload) {
    let parts: Vec<u16> = (1..=up.multipart.as_ref().unwrap().parts).collect();
    for batch in parts.chunks(attachment_limits::PARTS_PER_ASK) {
        let urls: Vec<PartUrl> = part_urls(app, token, up.id, batch).await.json().await.unwrap();
        for u in &urls {
            put_part(u, u.part as u8).await;
        }
    }
    assert_eq!(complete(app, token, up.id).await.status(), 204);
}

async fn send_files(app: &TestApp, token: &str, ch: ChannelId, ids: &[AttachmentId]) -> reqwest::Response {
    post_json(app, token, &format!("/channels/{ch}/messages"), json!({ "content": "", "attachments": ids })).await
}

fn s3(app: &TestApp) -> &pulse_server::testing::fake_s3::FakeS3 {
    app.s3.as_ref().unwrap()
}

#[tokio::test]
async fn temp_send_needs_1_or_7_days_and_the_right_size() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    let r = start(app, t, ch, 60 * MB, Some(3)).await;
    assert_eq!(r.status(), 400);
    assert_eq!(error_of(r).await, "Temporary files expire in 1 or 7 days.");
    // a normal-sized file isn't a temporary send
    assert_eq!(start(app, t, ch, 10 * MB, Some(1)).await.status(), 400);
    // a big one needs its days
    let r = start(app, t, ch, 60 * MB, None).await;
    assert_eq!(r.status(), 413);
    // 2 GB at most
    let r = start(app, t, ch, attachment_limits::TEMP_MAX_BYTES + 1, Some(7)).await;
    assert_eq!(r.status(), 413);
    assert_eq!(error_of(r).await, "Files can be up to 2 GB.");
    let up = start_ok(app, t, ch, attachment_limits::TEMP_MAX_BYTES, 7).await;
    assert!(up.url.is_none());
    assert_eq!(up.multipart.as_ref().unwrap().parts, 128);
    assert_eq!(up.multipart.as_ref().unwrap().part_size, PART);
}

#[tokio::test]
async fn the_20gb_cap_refuses_new_big_sends() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    // 19.9 GB of live temporary files already
    sqlx::query(
        "INSERT INTO attachments (id, key, user_id, channel_id, name, content_type, kind, size, expires_at, created_at)
         VALUES ('01JAAAAAAAAAAAAAAAAAAAAAAA', 'seed', ?, ?, 'big.bin', 'application/octet-stream', 'file', ?, '2099-01-01T00:00:00.000Z', ?)",
    )
    .bind(w.alex.0.to_string())
    .bind(ch.to_string())
    .bind((attachment_limits::TEMP_CAP_BYTES - 100 * MB) as i64)
    .bind(pulse_server::db::now())
    .execute(&app.db)
    .await
    .unwrap();
    let r = start(app, t, ch, 200 * MB, Some(1)).await;
    assert_eq!(r.status(), 409);
    assert_eq!(error_of(r).await, "This server's space for big files is full. Try again in a day or two.");
    // a normal file is fine, and a big one that fits too
    let r = post_json(app, t, &format!("/channels/{ch}/attachments"), json!({ "name": "a.txt", "size": 50 * MB, "content_type": "text/plain" })).await;
    assert_eq!(r.status(), 200);
    start_ok(app, t, ch, 60 * MB, 1).await;
    // an expired (purged) file frees its room
    sqlx::query("UPDATE attachments SET purged_at = ? WHERE key = 'seed'")
        .bind(pulse_server::db::now())
        .execute(&app.db)
        .await
        .unwrap();
    start_ok(app, t, ch, 1900 * MB, 1).await;
}

#[tokio::test]
async fn parts_are_signed_per_part_and_resume_lists_done_parts() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    let size = 3 * PART + 3 * MB;
    let up = start_ok(app, t, ch, size, 1).await;
    assert_eq!(up.multipart.as_ref().unwrap().parts, 4);
    let urls: Vec<PartUrl> = part_urls(app, t, up.id, &[1, 4]).await.json().await.unwrap();
    assert_eq!(urls.iter().map(|u| (u.part, u.size)).collect::<Vec<_>>(), vec![(1, PART), (4, 3 * MB)]);
    // each URL takes exactly its part's length
    let r = reqwest::Client::new().put(&urls[1].url).body(vec![0u8; 6]).send().await.unwrap();
    assert_eq!(r.status(), 403);
    // someone else can't sign parts of my upload; parts out of range are refused
    assert_eq!(part_urls(app, &w.sam.1, up.id, &[1]).await.status(), 404);
    assert_eq!(part_urls(app, t, up.id, &[5]).await.status(), 400);
    put_part(&urls[0], 1).await;
    put_part(&urls[1], 4).await;
    let done = uploaded(app, t, up.id).await;
    assert_eq!(done.iter().map(|p| p.part).collect::<Vec<_>>(), vec![1, 4]);
    // not finished: can't complete, can't send
    assert_eq!(complete(app, t, up.id).await.status(), 409);
    assert_eq!(send_files(app, t, ch, &[up.id]).await.status(), 409);
    let urls: Vec<PartUrl> = part_urls(app, t, up.id, &[2, 3]).await.json().await.unwrap();
    for u in &urls {
        put_part(u, u.part as u8).await;
    }
    assert_eq!(complete(app, t, up.id).await.status(), 204);
    let key: String = sqlx::query_scalar("SELECT key FROM attachments WHERE id = ?")
        .bind(up.id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    let (bytes, _) = s3(app).object(&format!("temp/{key}")).unwrap();
    assert_eq!(bytes.len() as u64, size);
    for n in 0..4u64 {
        assert_eq!(bytes[(n * PART) as usize], n as u8 + 1);
    }
}

#[tokio::test]
async fn complete_then_send_posts_with_expiry_counted_from_sending() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    let up = start_ok(app, t, ch, BIG, 1).await;
    // it took an hour to upload
    sqlx::query("UPDATE attachments SET created_at = ?, expires_at = ?")
        .bind(pulse_server::db::stamp(chrono::Utc::now() - chrono::Duration::hours(1)))
        .bind(pulse_server::db::stamp(chrono::Utc::now() + chrono::Duration::hours(23)))
        .execute(&app.db)
        .await
        .unwrap();
    upload_all(app, t, &up).await;
    let r = send_files(app, t, ch, &[up.id]).await;
    assert_eq!(r.status(), 200);
    let m: Message = r.json().await.unwrap();
    let exp = chrono::DateTime::parse_from_rfc3339(m.attachments[0].expires_at.as_deref().unwrap()).unwrap();
    let left = exp.with_timezone(&chrono::Utc) - chrono::Utc::now();
    assert!(left > chrono::Duration::hours(23) + chrono::Duration::minutes(55), "{left}");
    assert!(left <= chrono::Duration::hours(24), "{left}");
    assert!(!m.attachments[0].expired);
    assert_eq!(m.content, "📎 world.zip");
    // sam gets a link
    let urls: Vec<AttachmentUrl> = get_json(app, &w.sam.1, &format!("/messages/{}/attachments/urls", m.id)).await;
    assert!(urls[0].url.as_deref().unwrap().contains("/temp/"));
}

#[tokio::test]
async fn expired_temp_is_purged_and_shows_expired() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    let up = start_ok(app, t, ch, BIG, 1).await;
    upload_all(app, t, &up).await;
    let m: Message = send_files(app, t, ch, &[up.id]).await.json().await.unwrap();
    assert_eq!(s3(app).keys().len(), 1);
    let r = pulse_server::attachments::sweep::run(&app.state(), chrono::Utc::now() + chrono::Duration::days(2))
        .await
        .unwrap();
    assert_eq!(r.expired, 1);
    assert!(s3(app).keys().is_empty());
    let list: Vec<Message> = get_json(app, &w.sam.1, &format!("/channels/{ch}/messages")).await;
    let got = list.iter().find(|x| x.id == m.id).unwrap();
    assert!(got.attachments[0].expired);
    assert_eq!(got.attachments[0].name, "world.zip");
    let urls: Vec<AttachmentUrl> = get_json(app, &w.sam.1, &format!("/messages/{}/attachments/urls", m.id)).await;
    assert!(urls[0].url.is_none());
    // its room is free again
    let live: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(size), 0) FROM attachments WHERE expires_at IS NOT NULL AND purged_at IS NULL")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(live, 0);
}

#[tokio::test]
async fn sweep_and_cancel_abort_unfinished_multipart_uploads() {
    let w = world().await;
    let (app, t, ch) = (&w.app, &w.alex.1, w.text.id);
    let a = start_ok(app, t, ch, BIG, 1).await;
    let b = start_ok(app, t, ch, BIG, 1).await;
    assert_eq!(s3(app).open_uploads(), 2);
    let r = app.http.delete(app.url(&format!("/attachment-uploads/{}", a.id))).bearer_auth(t).send().await.unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(s3(app).open_uploads(), 1);
    sqlx::query("UPDATE attachments SET created_at = ?")
        .bind(pulse_server::db::stamp(chrono::Utc::now() - chrono::Duration::hours(26)))
        .execute(&app.db)
        .await
        .unwrap();
    let r = pulse_server::attachments::sweep::run(&app.state(), chrono::Utc::now()).await.unwrap();
    assert_eq!(r.pending, 1);
    assert_eq!(s3(app).open_uploads(), 0);
    let _ = b;
}
