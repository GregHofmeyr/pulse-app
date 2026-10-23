//! The media store: emoji uploads, fetches, the disk guard and cleanup.
mod common;

use std::io::Cursor;
use std::time::Duration;

use common::*;
use image::{ImageFormat, Rgba, RgbaImage};
use pulse_protocol::rest::{ApiError, MediaInfo};
use pulse_server::limits::Rate;
use pulse_server::media;

fn png() -> Vec<u8> {
    let img = RgbaImage::from_pixel(64, 32, Rgba([10, 120, 240, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

async fn upload(app: &TestApp, token: &str, body: Vec<u8>) -> reqwest::Response {
    app.http
        .post(app.url("/media/emoji"))
        .bearer_auth(token)
        .header("content-type", "application/octet-stream")
        .body(body)
        .send()
        .await
        .unwrap()
}

async fn upload_ok(app: &TestApp, token: &str) -> MediaInfo {
    let r = upload(app, token, png()).await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

async fn media_rows(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM media")
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn upload_returns_info_and_dedupes() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let a = upload_ok(&app, &token).await;
    let b = upload_ok(&app, &token).await;
    assert_eq!(a.hash, b.hash);
    assert_eq!(a.hash.len(), 64);
    assert_eq!(
        (
            a.content_type.as_str(),
            a.width,
            a.height,
            a.frames,
            a.animated
        ),
        ("image/webp", 128, 128, 1, false)
    );
    assert_eq!(a.still_hash, None);
    assert_eq!(media_rows(&app).await, 1);
    assert!(media::path_for(&app.cfg.media_dir, &a.hash).is_file());
}

#[tokio::test]
async fn fetch_needs_login() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let info = upload_ok(&app, &token).await;
    let url = app.url(&format!("/media/{}", info.hash));
    assert_eq!(app.http.get(&url).send().await.unwrap().status(), 401);
    let r = app.http.get(&url).bearer_auth(&token).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "image/webp");
    assert_eq!(
        r.headers()["cache-control"],
        "private, max-age=31536000, immutable"
    );
    let bytes = r.bytes().await.unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
}

#[tokio::test]
async fn unknown_or_malformed_hash_is_404() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    for path in [
        format!("/media/{}", "ab".repeat(32)),
        "/media/..%2F..%2Fetc%2Fpasswd".into(),
        "/media/../etc/passwd".into(),
        format!("/media/{}", "AB".repeat(32)),
    ] {
        let r = app
            .http
            .get(app.url(&path))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 404, "{path}");
    }
}

#[tokio::test]
async fn bad_upload_is_a_clear_400() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    for body in [b"hello".to_vec(), Vec::new()] {
        let r = upload(&app, &token, body).await;
        assert_eq!(r.status(), 400);
        let e: ApiError = r.json().await.unwrap();
        assert!(
            e.message.contains("PNG, JPEG, GIF or WebP"),
            "{}",
            e.message
        );
    }
    assert_eq!(media_rows(&app).await, 0);
}

#[tokio::test]
async fn upload_needs_login() {
    let app = spawn().await;
    let r = app
        .http
        .post(app.url("/media/emoji"))
        .body(png())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn upload_rate_limited() {
    let mut cfg = test_config();
    cfg.limits.upload_user = Rate::new(1, 0.0);
    let app = spawn_with(cfg).await;
    let (_, token) = register(&app, "alex").await;
    assert_eq!(upload(&app, &token, png()).await.status(), 200);
    assert_eq!(upload(&app, &token, png()).await.status(), 429);
}

#[tokio::test]
async fn disk_guard_refuses() {
    let mut cfg = test_config();
    cfg.media_min_free_bytes = u64::MAX;
    let app = spawn_with(cfg).await;
    let (_, token) = register(&app, "alex").await;
    let r = upload(&app, &token, png()).await;
    assert_eq!(r.status(), 507);
    let e: ApiError = r.json().await.unwrap();
    assert!(
        e.message.contains("server storage is full"),
        "{}",
        e.message
    );
    assert_eq!(media_rows(&app).await, 0);
}

#[tokio::test]
async fn cleanup_removes_unreferenced_after_grace() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let info = upload_ok(&app, &token).await;
    let file = media::path_for(&app.cfg.media_dir, &info.hash);

    let n = media::cleanup(&app.db, &app.cfg.media_dir, Duration::from_secs(3600))
        .await
        .unwrap();
    assert_eq!(n, 0);
    assert_eq!(media_rows(&app).await, 1);
    assert!(file.is_file());

    let n = media::cleanup(&app.db, &app.cfg.media_dir, Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(n, 1);
    assert_eq!(media_rows(&app).await, 0);
    assert!(!file.exists());
}

fn gif(frames: usize) -> Vec<u8> {
    use image::codecs::gif::GifEncoder;
    use image::{Delay, Frame};
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        for i in 0..frames {
            let img = RgbaImage::from_pixel(32, 32, Rgba([(i * 80) as u8, 40, 90, 255]));
            let delay = Delay::from_numer_denom_ms(50, 1);
            enc.encode_frame(Frame::from_parts(img, 0, 0, delay))
                .unwrap();
        }
    }
    out
}

#[tokio::test]
async fn animated_upload_stores_its_still_and_cleanup_follows_the_parent() {
    let app = spawn().await;
    let (alex, token) = register(&app, "alex").await;
    let r = upload(&app, &token, gif(3)).await;
    assert_eq!(r.status(), 200);
    let info: MediaInfo = r.json().await.unwrap();
    assert_eq!((info.frames, info.animated), (3, true));
    let still = info.still_hash.clone().unwrap();
    assert_ne!(still, info.hash);
    let (parent_still, still_frames): (Option<String>, i64) = sqlx::query_as(
        "SELECT p.still_hash, s.frames FROM media p JOIN media s ON s.hash = ? WHERE p.hash = ?",
    )
    .bind(&still)
    .bind(&info.hash)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (parent_still.as_deref(), still_frames),
        (Some(still.as_str()), 1)
    );
    let dir = &app.cfg.media_dir;
    assert!(media::path_for(dir, &still).is_file());

    // In use by an emoji: neither the animation nor its still goes.
    let server = create_server(&app, &token, "home").await;
    sqlx::query(
        "INSERT INTO custom_emoji (id, server_id, name, media_hash, animated, added_by, created_at)
         VALUES ('e1', ?, 'wave', ?, 1, ?, '2026-01-01T00:00:00.000Z')",
    )
    .bind(server.id.to_string())
    .bind(&info.hash)
    .bind(alex.to_string())
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(
        media::cleanup(&app.db, dir, Duration::ZERO).await.unwrap(),
        0
    );
    assert_eq!(media_rows(&app).await, 2);

    // Unused: both go in one pass.
    sqlx::query("DELETE FROM custom_emoji")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        media::cleanup(&app.db, dir, Duration::ZERO).await.unwrap(),
        2
    );
    assert_eq!(media_rows(&app).await, 0);
    assert!(!media::path_for(dir, &still).exists());
    assert!(!media::path_for(dir, &info.hash).exists());
}
