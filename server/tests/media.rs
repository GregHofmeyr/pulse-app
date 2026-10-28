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
    upload_to(app, "/media/emoji", token, body).await
}

async fn upload_to(app: &TestApp, path: &str, token: &str, body: Vec<u8>) -> reqwest::Response {
    app.http
        .post(app.url(path))
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
    assert!(
        media::path_for(&app.cfg.media_dir, &a.hash)
            .unwrap()
            .is_file()
    );
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
    for path in ["/media/emoji", "/media/quote"] {
        let mut cfg = test_config();
        cfg.limits.upload_user = Rate::new(1, 0.0);
        let app = spawn_with(cfg).await;
        let (_, token) = register(&app, "alex").await;
        assert_eq!(
            upload_to(&app, path, &token, png()).await.status(),
            200,
            "{path}"
        );
        assert_eq!(
            upload_to(&app, path, &token, png()).await.status(),
            429,
            "{path}"
        );
    }
}

#[tokio::test]
async fn disk_guard_refuses() {
    for path in ["/media/emoji", "/media/quote"] {
        let mut cfg = test_config();
        cfg.media_min_free_bytes = u64::MAX;
        let app = spawn_with(cfg).await;
        let (_, token) = register(&app, "alex").await;
        let r = upload_to(&app, path, &token, png()).await;
        assert_eq!(r.status(), 507, "{path}");
        let e: ApiError = r.json().await.unwrap();
        assert!(
            e.message.contains("server storage is full"),
            "{path}: {}",
            e.message
        );
        assert_eq!(media_rows(&app).await, 0, "{path}");
    }
}

#[cfg(unix)]
fn free_bytes(dir: &std::path::Path) -> u64 {
    let st = rustix::fs::statvfs(dir).unwrap();
    st.f_bavail.saturating_mul(st.f_frsize)
}

/// An upload grows the database (its `media` rows) as well as the media dir: with the two on
/// different volumes, a full database volume refuses it even while the media volume has room.
#[cfg(unix)]
#[tokio::test]
async fn upload_refused_when_the_database_volume_is_low() {
    // The test database lives under the system temp dir; put the media dir on the volume the
    // build lives on, and set the threshold between the two volumes' free space.
    let media = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let (m, d) = (free_bytes(media.path()), free_bytes(&std::env::temp_dir()));
    const MARGIN: u64 = 1 << 30;
    if m <= d.saturating_add(2 * MARGIN) {
        eprintln!(
            "skipped: the temp dir and the target dir need different volumes, the target's with \
             at least 2 GiB more free space"
        );
        return;
    }
    for path in ["/media/emoji", "/media/quote"] {
        let mut cfg = test_config();
        cfg.media_dir = media.path().join(path.trim_start_matches("/media/"));
        cfg.media_min_free_bytes = d + (m - d) / 2;
        let app = spawn_with(cfg).await;
        let (_, token) = register(&app, "alex").await;
        let r = upload_to(&app, path, &token, png()).await;
        assert_eq!(r.status(), 507, "{path}");
        assert_eq!(media_rows(&app).await, 0, "{path}");
    }
}

#[tokio::test]
async fn cleanup_removes_unreferenced_after_grace() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let info = upload_ok(&app, &token).await;
    let file = media::path_for(&app.cfg.media_dir, &info.hash).unwrap();

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
    assert!(media::path_for(dir, &still).unwrap().is_file());

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
    assert!(!media::path_for(dir, &still).unwrap().exists());
    assert!(!media::path_for(dir, &info.hash).unwrap().exists());
}

#[tokio::test]
async fn reupload_restores_a_missing_file() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let info = upload_ok(&app, &token).await;
    let file = media::path_for(&app.cfg.media_dir, &info.hash).unwrap();
    std::fs::remove_file(&file).unwrap();
    // Row present, file absent: the next upload of the same bytes heals it.
    let again = upload_ok(&app, &token).await;
    assert_eq!(again.hash, info.hash);
    assert!(file.is_file());
    assert_eq!(media_rows(&app).await, 1);
}

#[tokio::test]
async fn oversize_body_is_a_json_413() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let r = upload(&app, &token, vec![0u8; 2 * 1024 * 1024 + 64 * 1024 + 1]).await;
    assert_eq!(r.status(), 413);
    let e: ApiError = r.json().await.unwrap();
    assert_eq!(e.code, "payload_too_large");
    assert!(e.message.contains("2 MB at most"), "{}", e.message);
    assert_eq!(media_rows(&app).await, 0);
}

fn sized_png(w: u32, h: u32) -> Vec<u8> {
    let img = RgbaImage::from_pixel(w, h, Rgba([10, 120, 240, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

async fn media_kind(app: &TestApp, hash: &str) -> String {
    sqlx::query_scalar("SELECT kind FROM media WHERE hash = ?")
        .bind(hash)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn uploads_a_quote_image() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let r = upload_to(&app, "/media/quote", &token, sized_png(3000, 1000)).await;
    assert_eq!(r.status(), 200);
    let info: MediaInfo = r.json().await.unwrap();
    assert_eq!((info.width, info.height, info.animated), (1600, 533, false));
    assert_eq!(info.still_hash, None);
    assert_eq!(media_kind(&app, &info.hash).await, "quote");
    assert!(
        media::path_for(&app.cfg.media_dir, &info.hash)
            .unwrap()
            .is_file()
    );
    // Same rules as emoji: junk is a clear 400, and it needs a login.
    let r = upload_to(&app, "/media/quote", &token, b"hello".to_vec()).await;
    assert_eq!(r.status(), 400);
    let r = app
        .http
        .post(app.url("/media/quote"))
        .body(png())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn uploads_an_animated_quote_image() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let r = upload_to(&app, "/media/quote", &token, gif(3)).await;
    assert_eq!(r.status(), 200);
    let info: MediaInfo = r.json().await.unwrap();
    assert_eq!((info.frames, info.animated), (3, true));
    let still = info.still_hash.clone().unwrap();
    assert_eq!(media_kind(&app, &still).await, "quote");
    assert_eq!(media_kind(&app, &info.hash).await, "quote");
    assert!(
        media::path_for(&app.cfg.media_dir, &still)
            .unwrap()
            .is_file()
    );
}

#[tokio::test]
async fn cleanup_keeps_media_used_by_a_quote_line() {
    let app = spawn().await;
    let (alex, token) = register(&app, "alex").await;
    let r = upload_to(&app, "/media/quote", &token, gif(3)).await;
    let info: MediaInfo = r.json().await.unwrap();
    let dir = &app.cfg.media_dir;
    let server = create_server(&app, &token, "home").await;
    for sql in [
        format!(
            "INSERT INTO channels (id, server_id, kind, name, position, created_at)
             VALUES ('c1', '{}', 'quotes', 'quotes', 0, '2026-01-01T00:00:00.000Z')",
            server.id
        ),
        format!(
            "INSERT INTO messages (id, channel_id, author_id, content, created_at)
             VALUES ('m1', 'c1', '{alex}', '', '2026-01-01T00:00:00.000Z')"
        ),
        format!(
            "INSERT INTO quote_lines (id, message_id, position, speaker_id, media_hash)
             VALUES ('l1', 'm1', 0, '{alex}', '{}')",
            info.hash
        ),
    ] {
        sqlx::query(&sql).execute(&app.db).await.unwrap();
    }
    // In use by a line: neither the animation nor its still goes.
    assert_eq!(
        media::cleanup(&app.db, dir, Duration::ZERO).await.unwrap(),
        0
    );
    assert_eq!(media_rows(&app).await, 2);

    sqlx::query("DELETE FROM quote_lines")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        media::cleanup(&app.db, dir, Duration::ZERO).await.unwrap(),
        2
    );
    assert_eq!(media_rows(&app).await, 0);
}

/// A deleted quote leaves nothing behind (I6): its lines go, so its picture and still are
/// reclaimed and its text is no longer stored.
#[tokio::test]
async fn deleting_a_quote_frees_its_lines_and_pictures() {
    let app = spawn().await;
    let (alex, token) = register(&app, "alex").await;
    let info: MediaInfo = upload_to(&app, "/media/quote", &token, gif(3))
        .await
        .json()
        .await
        .unwrap();
    let server = create_server(&app, &token, "home").await;
    let r = post_json(
        &app,
        &token,
        &format!("/servers/{}/channels", server.id),
        serde_json::json!({ "kind": "quotes", "name": "quotes" }),
    )
    .await;
    let ch: pulse_protocol::rest::Channel = r.json().await.unwrap();
    let r = post_json(
        &app,
        &token,
        &format!("/channels/{}/quotes", ch.id),
        serde_json::json!({ "lines": [
            { "speaker_id": alex, "media_hash": info.hash },
            { "speaker_name": "A Friend", "text": "embarrassing" },
        ] }),
    )
    .await;
    assert_eq!(r.status(), 200, "post quote");
    let q: pulse_protocol::rest::Message = r.json().await.unwrap();
    let dir = &app.cfg.media_dir;
    assert_eq!(
        media::cleanup(&app.db, dir, Duration::ZERO).await.unwrap(),
        0,
        "in use"
    );

    let r = app
        .http
        .delete(app.url(&format!("/messages/{}", q.id)))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let lines: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM quote_lines")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(lines, 0, "the line text is gone");
    assert_eq!(
        media::cleanup(&app.db, dir, Duration::ZERO).await.unwrap(),
        2,
        "the picture and its still are reclaimed"
    );
    assert!(!media::path_for(dir, &info.hash).unwrap().exists());
}

/// Incompressible pixels as a JPEG: under the 2 MB input cap, yet well over the 1 MB a still quote
/// may be once converted.
fn noisy_jpeg(size: u32) -> Vec<u8> {
    let mut x = 0x9E37_79B9u32;
    let img = image::RgbImage::from_fn(size, size, |_, _| {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        image::Rgb([x as u8, (x >> 8) as u8, (x >> 16) as u8])
    });
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Jpeg).unwrap();
    out.into_inner()
}

#[tokio::test]
async fn a_quote_image_too_big_once_converted_is_a_clear_400() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let body = noisy_jpeg(1250);
    assert!(body.len() < media::process::MAX_INPUT, "{}", body.len());
    let r = upload_to(&app, "/media/quote", &token, body).await;
    assert_eq!(r.status(), 400);
    let e: ApiError = r.json().await.unwrap();
    assert!(
        e.message.contains("too big once converted") && !e.message.contains("512"),
        "{}",
        e.message
    );
    assert_eq!(media_rows(&app).await, 0);
}
