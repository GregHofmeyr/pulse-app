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
    // Two filesystems every Linux box has: tmpfs at /dev/shm and the disk the build is on. The
    // database goes on the one with less room, the media dir on the other, and the threshold
    // sits between their free space.
    const MARGIN: u64 = 256 << 20;
    let shm = std::path::Path::new("/dev/shm");
    let target = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let (s, t) = if shm.is_dir() {
        (free_bytes(shm), free_bytes(target))
    } else {
        (0, 0)
    };
    if s.abs_diff(t) <= 2 * MARGIN {
        let why = format!(
            "needs /dev/shm and the target dir with free space at least 512 MiB apart (got {s} and {t})"
        );
        assert!(std::env::var_os("CI").is_none(), "{why}");
        eprintln!("skipped: {why}");
        return;
    }
    let (db_fs, media_fs) = if s < t { (shm, target) } else { (target, shm) };
    let (low, high) = (s.min(t), s.max(t));
    for path in ["/media/emoji", "/media/quote"] {
        let media = tempfile::tempdir_in(media_fs).unwrap();
        let mut cfg = test_config();
        cfg.media_dir = media.path().join("media");
        cfg.media_min_free_bytes = low + (high - low) / 2;
        let app = spawn_in(cfg, tempfile::tempdir_in(db_fs).unwrap()).await;
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

fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 40, 90]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Jpeg).unwrap();
    out.into_inner()
}

#[tokio::test]
async fn uploads_a_clip_thumbnail() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let r = upload_to(&app, "/media/clip-thumb", &token, jpeg(1280, 720)).await;
    assert_eq!(r.status(), 200);
    let info: MediaInfo = r.json().await.unwrap();
    assert_eq!((info.width, info.height), (1280, 720));
    assert_eq!(info.content_type, "image/webp");
    let (kind, location): (String, String) =
        sqlx::query_as("SELECT kind, location FROM media WHERE hash = ?")
            .bind(&info.hash)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((kind.as_str(), location.as_str()), ("clip_thumb", "disk"));
    assert!(
        media::path_for(&app.cfg.media_dir, &info.hash)
            .unwrap()
            .is_file()
    );
}

/// An aged `r2` row nothing references yet: the clips sweeper owns it, never disk cleanup.
#[tokio::test]
async fn cleanup_never_touches_remote_rows() {
    let app = spawn().await;
    let (alex, _) = register(&app, "alex").await;
    let key = media::new_key();
    let mut conn = app.db.acquire().await.unwrap();
    media::put_remote(
        &mut conn,
        media::MediaKind::Clip,
        &key,
        1234,
        1920,
        1080,
        alex,
    )
    .await
    .unwrap();
    drop(conn);
    sqlx::query("UPDATE media SET created_at = '2020-01-01T00:00:00.000Z'")
        .execute(&app.db)
        .await
        .unwrap();
    let row: (String, String, String, i64, i64) = sqlx::query_as(
        "SELECT kind, location, content_type, frames, size FROM media WHERE hash = ?",
    )
    .bind(&key)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        (row.0.as_str(), row.1.as_str(), row.2.as_str(), row.3, row.4),
        ("clip", "r2", "video/mp4", 0, 1234)
    );
    assert_eq!(
        media::cleanup(&app.db, &app.cfg.media_dir, Duration::ZERO)
            .await
            .unwrap(),
        0
    );
    assert_eq!(media_rows(&app).await, 1);
}

/// A clips channel with one clip (a stored video key) whose thumbnail is `thumb_hash`.
async fn insert_live_clip(
    app: &TestApp,
    token: &str,
    alex: pulse_protocol::ids::UserId,
    thumb_hash: &str,
) {
    let key = media::new_key();
    let mut conn = app.db.acquire().await.unwrap();
    media::put_remote(
        &mut conn,
        media::MediaKind::Clip,
        &key,
        10,
        1920,
        1080,
        alex,
    )
    .await
    .unwrap();
    drop(conn);
    let server = create_server(app, token, "home").await;
    for sql in [
        format!(
            "INSERT INTO channels (id, server_id, kind, name, position, created_at)
             VALUES ('c1', '{}', 'clips', 'clips', 0, '2026-01-01T00:00:00.000Z')",
            server.id
        ),
        format!(
            "INSERT INTO messages (id, channel_id, author_id, content, created_at)
             VALUES ('m1', 'c1', '{alex}', '', '2026-01-01T00:00:00.000Z')"
        ),
        format!(
            "INSERT INTO clips (message_id, video_key, thumb_hash, duration_ms, width, height)
             VALUES ('m1', '{key}', '{thumb_hash}', 1000, 1920, 1080)"
        ),
    ] {
        sqlx::query(&sql).execute(&app.db).await.unwrap();
    }
}

#[tokio::test]
async fn cleanup_keeps_a_live_clips_thumbnail() {
    let app = spawn().await;
    let (alex, token) = register(&app, "alex").await;
    let thumb: MediaInfo = upload_to(&app, "/media/clip-thumb", &token, jpeg(640, 360))
        .await
        .json()
        .await
        .unwrap();
    insert_live_clip(&app, &token, alex, &thumb.hash).await;
    let dir = &app.cfg.media_dir;
    assert_eq!(
        media::cleanup(&app.db, dir, Duration::ZERO).await.unwrap(),
        0
    );
    assert!(media::path_for(dir, &thumb.hash).unwrap().is_file());

    sqlx::query("DELETE FROM clips")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        media::cleanup(&app.db, dir, Duration::ZERO).await.unwrap(),
        1
    );
    assert!(!media::path_for(dir, &thumb.hash).unwrap().exists());
}

#[tokio::test]
async fn cleanup_keeps_the_still_of_a_live_animated_clip_thumbnail() {
    let app = spawn().await;
    let (alex, token) = register(&app, "alex").await;
    let thumb: MediaInfo = upload_to(&app, "/media/clip-thumb", &token, gif(2))
        .await
        .json()
        .await
        .unwrap();
    let still = thumb
        .still_hash
        .clone()
        .expect("an animated thumbnail has a still");
    insert_live_clip(&app, &token, alex, &thumb.hash).await;
    let dir = &app.cfg.media_dir;
    assert_eq!(
        media::cleanup(&app.db, dir, Duration::ZERO).await.unwrap(),
        0
    );
    assert!(media::path_for(dir, &thumb.hash).unwrap().is_file());
    assert!(media::path_for(dir, &still).unwrap().is_file());

    sqlx::query("DELETE FROM clips")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        media::cleanup(&app.db, dir, Duration::ZERO).await.unwrap(),
        2
    );
    assert!(!media::path_for(dir, &thumb.hash).unwrap().exists());
    assert!(!media::path_for(dir, &still).unwrap().exists());
}

#[tokio::test]
async fn put_remote_refuses_a_size_that_does_not_fit() {
    let app = spawn().await;
    let (alex, _token) = register(&app, "alex").await;
    let mut conn = app.db.acquire().await.unwrap();
    let r = media::put_remote(
        &mut conn,
        media::MediaKind::Clip,
        &media::new_key(),
        u64::MAX,
        1920,
        1080,
        alex,
    )
    .await;
    assert!(r.is_err());
    drop(conn);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

// The child process is waited on by blocking, so the in-test fake needs its own thread.
#[tokio::test(flavor = "multi_thread")]
async fn clips_check_reports_ok_against_the_fake() {
    use pulse_server::testing::fake_s3::*;
    let fake = FakeS3::start(FakeStore::Memory).await;
    let dir = tempfile::tempdir().unwrap();
    let db = format!("sqlite://{}", dir.path().join("pulse.db").display());
    let endpoint = fake.endpoint();
    let env = [
        ("PULSE_CLIPS_S3_ENDPOINT", endpoint.as_str()),
        ("PULSE_CLIPS_S3_BUCKET", FAKE_BUCKET),
        ("PULSE_CLIPS_S3_KEY_ID", FAKE_KEY_ID),
        ("PULSE_CLIPS_S3_SECRET", FAKE_SECRET),
    ];
    let (ok, out) = run_binary(&db, &env, &["clips-check"], Duration::from_secs(20));
    assert!(ok, "{out}");
    assert!(out.contains("clips storage ok"), "{out}");
    assert!(!out.contains(FAKE_SECRET), "{out}");
    // Probed and cleaned up.
    assert!(fake.keys().is_empty(), "{:?}", fake.keys());
    // The over-long PUT is refused at the signature check, so it never reaches the log.
    let verbs: Vec<_> = fake.requests().iter().map(|(m, _)| m.to_string()).collect();
    assert_eq!(verbs, ["PUT", "HEAD", "GET", "DELETE"]);

    // A wrong secret names the failing step and its status.
    let bad = [
        env[0],
        env[1],
        env[2],
        ("PULSE_CLIPS_S3_SECRET", "wrong-secret-wrong-secret"),
    ];
    let (ok, out) = run_binary(&db, &bad, &["clips-check"], Duration::from_secs(20));
    assert!(!ok, "{out}");
    assert!(out.contains("PUT") && out.contains("403"), "{out}");
    assert!(!out.contains("wrong-secret"), "{out}");

    // Storage that isn't there: the step is named and no signed URL is printed.
    drop(fake);
    let (ok, out) = run_binary(&db, &env, &["clips-check"], Duration::from_secs(20));
    assert!(!ok, "{out}");
    assert!(out.contains("failed"), "{out}");
    for secret in [FAKE_KEY_ID, "X-Amz-Signature", "X-Amz-Credential"] {
        assert!(!out.contains(secret), "{secret} leaked: {out}");
    }
}

#[tokio::test]
async fn the_harness_gives_clips_storage_unless_asked_not_to() {
    let with = spawn().await;
    assert!(with.cfg.clips.is_some() && with.s3.is_some() && with.state().r2.is_some());
    let without = spawn_without_clips().await;
    assert!(without.cfg.clips.is_none() && without.s3.is_none() && without.state().r2.is_none());
}
