//! Attachments (#57): starting an upload and cancelling one.
mod common;

use common::*;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{AttachmentUpload, Channel, attachment_limits};
use serde_json::json;

const MB: u64 = 1024 * 1024;

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

async fn channel(w: &World, kind: &str) -> Channel {
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

async fn start(
    app: &TestApp,
    token: &str,
    channel: ChannelId,
    body: serde_json::Value,
) -> reqwest::Response {
    post_json(
        app,
        token,
        &format!("/channels/{channel}/attachments"),
        body,
    )
    .await
}

fn file(name: &str, size: u64, content_type: &str) -> serde_json::Value {
    json!({ "name": name, "size": size, "content_type": content_type })
}

async fn start_ok(app: &TestApp, token: &str, channel: ChannelId, size: u64) -> AttachmentUpload {
    let r = start(app, token, channel, file("notes.txt", size, "text/plain")).await;
    assert_eq!(r.status(), 200, "start upload");
    r.json().await.unwrap()
}

async fn message_of(r: reqwest::Response) -> String {
    let v: serde_json::Value = r.json().await.unwrap();
    v["message"].as_str().unwrap_or_default().to_string()
}

/// (key, name, content_type, kind, channel_id, message_id) of an attachment row.
async fn row(
    app: &TestApp,
    up: &AttachmentUpload,
) -> (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
) {
    sqlx::query_as(
        "SELECT key, name, content_type, kind, channel_id, message_id FROM attachments WHERE id = ?",
    )
    .bind(up.id.to_string())
    .fetch_one(&app.db)
    .await
    .unwrap()
}

async fn rows(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM attachments")
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn starts_an_upload_in_text_voice_dm_and_group() {
    let w = world().await;
    let (kim_id, _) = register(&w.app, "kim").await;
    let voice = channel(&w, "voice").await;
    let dm = create_dm(&w.app, &w.alex.1, &[w.sam.0]).await;
    let group = create_dm(&w.app, &w.alex.1, &[w.sam.0, kim_id]).await;
    for ch in [&w.text, &voice, &dm, &group] {
        let up = start_ok(&w.app, &w.alex.1, ch.id, 1_000).await;
        assert!(up.url.is_some(), "{:?}", ch.kind);
        assert_eq!(up.content_type, "application/octet-stream");
        let (key, name, _, kind, channel, message) = row(&w.app, &up).await;
        assert_eq!(key.len(), 64, "{key}");
        assert_eq!(name, "notes.txt");
        assert_eq!(kind, "file");
        assert_eq!(channel, Some(ch.id.to_string()));
        assert_eq!(message, None, "pending until sent");
    }
    assert_eq!(rows(&w.app).await, 4);
}

#[tokio::test]
async fn refused_in_quotes_and_clips_channels() {
    let w = world().await;
    for kind in ["quotes", "clips"] {
        let ch = channel(&w, kind).await;
        let r = start(&w.app, &w.alex.1, ch.id, file("a.txt", 10, "text/plain")).await;
        assert_eq!(r.status(), 400, "{kind}");
        assert_eq!(message_of(r).await, "Files can't be posted here.");
    }
    assert_eq!(rows(&w.app).await, 0);
}

#[tokio::test]
async fn non_member_is_forbidden() {
    let w = world().await;
    let (_, kim) = register(&w.app, "kim").await;
    let r = start(&w.app, &kim, w.text.id, file("a.txt", 10, "text/plain")).await;
    assert_eq!(r.status(), 403);
    // Someone else's DM looks like no channel at all.
    let dm = create_dm(&w.app, &w.alex.1, &[w.sam.0]).await;
    let r = start(&w.app, &kim, dm.id, file("a.txt", 10, "text/plain")).await;
    assert_eq!(r.status(), 404);
    assert_eq!(rows(&w.app).await, 0);
}

#[tokio::test]
async fn empty_and_over_50mb_are_refused() {
    let w = world().await;
    let r = start(&w.app, &w.alex.1, w.text.id, file("a.txt", 0, "text/plain")).await;
    assert_eq!(r.status(), 400);
    assert_eq!(message_of(r).await, "That file is empty.");
    let max = attachment_limits::MAX_BYTES;
    assert_eq!(max, 50 * MB);
    let r = start(&w.app, &w.alex.1, w.text.id, file("a.bin", max + 1, "")).await;
    assert_eq!(r.status(), 413);
    assert_eq!(message_of(r).await, "Files can be up to 50 MB.");
    assert_eq!(rows(&w.app).await, 0);
    start_ok(&w.app, &w.alex.1, w.text.id, max).await;
}

#[tokio::test]
async fn a_temporary_send_goes_up_in_parts() {
    let w = world().await;
    let mut body = file("world.zip", 300 * MB, "application/zip");
    body["temp_days"] = json!(1);
    let r = start(&w.app, &w.alex.1, w.text.id, body).await;
    assert_eq!(r.status(), 200);
    let up: AttachmentUpload = r.json().await.unwrap();
    assert!(up.url.is_none());
    assert_eq!(up.multipart.unwrap().parts, 19);
    assert_eq!(rows(&w.app).await, 1);
}

#[tokio::test]
async fn names_are_sanitised() {
    let w = world().await;
    let r = start(
        &w.app,
        &w.alex.1,
        w.text.id,
        file("../../a\u{202e}gpj.exe", 10, ""),
    )
    .await;
    assert_eq!(r.status(), 200);
    let up: AttachmentUpload = r.json().await.unwrap();
    assert_eq!(row(&w.app, &up).await.1, "agpj.exe");

    let long = format!("{}.txt", "n".repeat(296));
    let up: AttachmentUpload = start(&w.app, &w.alex.1, w.text.id, file(&long, 10, ""))
        .await
        .json()
        .await
        .unwrap();
    let name = row(&w.app, &up).await.1;
    assert!(name.chars().count() <= 200, "{}", name.chars().count());
    assert!(name.ends_with(".txt"), "{name}");

    for bad in ["", "   ", "dir/", "...", "\u{202e}\u{2066}"] {
        let r = start(&w.app, &w.alex.1, w.text.id, file(bad, 10, "")).await;
        assert_eq!(r.status(), 400, "{bad:?}");
    }
}

#[tokio::test]
async fn non_webp_image_type_is_refused() {
    let w = world().await;
    for t in ["image/png", "image/jpeg", "IMAGE/GIF"] {
        let r = start(&w.app, &w.alex.1, w.text.id, file("a.png", 10, t)).await;
        assert_eq!(r.status(), 400, "{t}");
        assert_eq!(message_of(r).await, "Images must be WebP.");
    }
    assert_eq!(rows(&w.app).await, 0);
    let r = start(
        &w.app,
        &w.alex.1,
        w.text.id,
        file("a.webp", 10, "image/webp"),
    )
    .await;
    assert_eq!(r.status(), 200);
    let up: AttachmentUpload = r.json().await.unwrap();
    assert_eq!(up.content_type, "image/webp");
    let (_, _, content_type, kind, _, _) = row(&w.app, &up).await;
    assert_eq!(
        (content_type.as_str(), kind.as_str()),
        ("image/webp", "image")
    );
}

#[tokio::test]
async fn unknown_types_are_stored_as_octet_stream() {
    let w = world().await;
    for (declared, stored, kind) in [
        ("text/html", "application/octet-stream", "file"),
        (
            "application/x-msdownload",
            "application/octet-stream",
            "file",
        ),
        ("", "application/octet-stream", "file"),
        ("video/mp4", "video/mp4", "video"),
        ("video/quicktime", "video/quicktime", "video"),
        ("audio/ogg; codecs=opus", "audio/ogg", "audio"),
    ] {
        let r = start(&w.app, &w.alex.1, w.text.id, file("f", 10, declared)).await;
        assert_eq!(r.status(), 200, "{declared}");
        let up: AttachmentUpload = r.json().await.unwrap();
        assert_eq!(up.content_type, stored, "{declared}");
        let (_, _, content_type, k, _, _) = row(&w.app, &up).await;
        assert_eq!((content_type.as_str(), k.as_str()), (stored, kind));
    }
}

#[tokio::test]
async fn the_put_url_is_signed_for_one_object_and_one_length() {
    let w = world().await;
    let s3 = w.app.s3.as_ref().unwrap();
    let up = start_ok(&w.app, &w.alex.1, w.text.id, 5).await;
    let key = row(&w.app, &up).await.0;
    let url = reqwest::Url::parse(up.url.as_deref().unwrap()).unwrap();
    assert!(
        url.path().ends_with(&format!("/attachments/{key}")),
        "{}",
        url.path()
    );
    let expires = url
        .query_pairs()
        .find(|(k, _)| k == "X-Amz-Expires")
        .map(|(_, v)| v.to_string());
    assert_eq!(expires.as_deref(), Some("3600"));

    let put = |body: &'static [u8]| {
        w.app
            .http
            .put(url.clone())
            .header("content-type", &up.content_type)
            .body(body)
            .send()
    };
    assert_eq!(put(b"too long").await.unwrap().status(), 403);
    assert!(s3.keys().is_empty());
    assert_eq!(put(b"hello").await.unwrap().status(), 200);
    let (bytes, ct) = s3.object(&format!("attachments/{key}")).unwrap();
    assert_eq!(
        (bytes.as_slice(), ct.as_str()),
        (&b"hello"[..], "application/octet-stream")
    );
}

#[tokio::test]
async fn open_uploads_are_capped_at_50() {
    let w = world().await;
    for _ in 0..50 {
        start_ok(&w.app, &w.alex.1, w.text.id, 10).await;
    }
    let r = start(&w.app, &w.alex.1, w.text.id, file("a.txt", 10, "")).await;
    assert_eq!(r.status(), 429);
    assert_eq!(message_of(r).await, "Too many uploads in progress.");
    // Per person: sam still can.
    start_ok(&w.app, &w.sam.1, w.text.id, 10).await;
    // Sent uploads no longer count.
    let msg = send(&w.app, &w.alex.1, w.text.id, "hi").await;
    sqlx::query(
        "UPDATE attachments SET message_id = ? WHERE id IN
         (SELECT id FROM attachments WHERE user_id = ? LIMIT 1)",
    )
    .bind(msg.id.to_string())
    .bind(w.alex.0.to_string())
    .execute(&w.app.db)
    .await
    .unwrap();
    start_ok(&w.app, &w.alex.1, w.text.id, 10).await;
}

#[tokio::test]
async fn cancel_deletes_row_and_object() {
    let w = world().await;
    let s3 = w.app.s3.as_ref().unwrap();
    let up = start_ok(&w.app, &w.alex.1, w.text.id, 5).await;
    let r = w
        .app
        .http
        .put(up.url.as_deref().unwrap())
        .header("content-type", &up.content_type)
        .body(&b"hello"[..])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let object = format!("attachments/{}", row(&w.app, &up).await.0);
    assert!(s3.object(&object).is_some());

    let url = w.app.url(&format!("/attachment-uploads/{}", up.id));
    let del = |token: &str| w.app.http.delete(&url).bearer_auth(token).send();
    assert_eq!(
        del(&w.sam.1).await.unwrap().status(),
        404,
        "only the owner cancels"
    );
    assert!(s3.object(&object).is_some());
    assert_eq!(del(&w.alex.1).await.unwrap().status(), 204);
    assert_eq!(rows(&w.app).await, 0);
    assert!(s3.object(&object).is_none());
    assert_eq!(del(&w.alex.1).await.unwrap().status(), 404, "already gone");

    // A sent attachment is part of its message now: not cancellable.
    let up = start_ok(&w.app, &w.alex.1, w.text.id, 5).await;
    let msg = send(&w.app, &w.alex.1, w.text.id, "hi").await;
    sqlx::query("UPDATE attachments SET message_id = ? WHERE id = ?")
        .bind(msg.id.to_string())
        .bind(up.id.to_string())
        .execute(&w.app.db)
        .await
        .unwrap();
    let url = w.app.url(&format!("/attachment-uploads/{}", up.id));
    let r = w
        .app
        .http
        .delete(&url)
        .bearer_auth(&w.alex.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    assert_eq!(rows(&w.app).await, 1);
}

#[tokio::test]
async fn without_storage_uploads_are_unavailable() {
    let app = spawn_without_clips().await;
    let (_, alex) = register(&app, "alex").await;
    let s = create_server(&app, &alex, "Main").await;
    let text = general(&app, &alex, s.id).await;
    let r = start(&app, &alex, text.id, file("a.txt", 10, "")).await;
    assert_eq!(r.status(), 503);
    assert_eq!(
        message_of(r).await,
        "File storage isn't set up on this server."
    );
}
