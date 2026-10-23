//! Custom emoji: create/rename/delete, Ready, events, media cleanup and the frequent ranking.
mod common;

use std::io::Cursor;
use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use image::{ImageFormat, Rgba, RgbaImage};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{ApiError, CustomEmoji, MediaInfo};
use pulse_server::{emoji, media};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello(app: &TestApp, token: &str) -> (Socket, Ready) {
    let (mut ws, _) = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap();
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    match next_event_or_ready(&mut ws).await {
        Some(ServerFrame::Ready(r)) => (ws, r),
        other => panic!("expected Ready, got {other:?}"),
    }
}

async fn next_event_or_ready(ws: &mut Socket) -> Option<ServerFrame> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        if let Ws::Text(t) = m {
            let f: ServerFrame = serde_json::from_str(&t).unwrap();
            if matches!(
                f,
                ServerFrame::Event(Event::PresenceChanged { .. } | Event::UserCreated { .. })
            ) {
                continue;
            }
            return Some(f);
        }
    }
}

async fn next_event(ws: &mut Socket) -> Event {
    match next_event_or_ready(ws).await {
        Some(ServerFrame::Event(e)) => e,
        other => panic!("expected an event, got {other:?}"),
    }
}

fn png() -> Vec<u8> {
    let img = RgbaImage::from_pixel(64, 32, Rgba([10, 120, 240, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

async fn upload(app: &TestApp, token: &str) -> MediaInfo {
    let r = app
        .http
        .post(app.url("/media/emoji"))
        .bearer_auth(token)
        .body(png())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

async fn create(
    app: &TestApp,
    token: &str,
    server: ServerId,
    name: &str,
    hash: &str,
) -> reqwest::Response {
    post_json(
        app,
        token,
        &format!("/servers/{server}/emoji"),
        json!({ "name": name, "media_hash": hash }),
    )
    .await
}

async fn create_ok(
    app: &TestApp,
    token: &str,
    server: ServerId,
    name: &str,
    hash: &str,
) -> CustomEmoji {
    let r = create(app, token, server, name, hash).await;
    assert_eq!(r.status(), 200, "create {name}");
    r.json().await.unwrap()
}

async fn join(app: &TestApp, token: &str, server: ServerId) {
    let r = post_json(app, token, &format!("/servers/{server}/join"), json!({})).await;
    assert!(r.status().is_success());
}

async fn error_message(r: reqwest::Response) -> String {
    r.json::<ApiError>().await.unwrap().message
}

#[tokio::test]
async fn create_list_in_ready_and_event() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let srv = create_server(&app, &alex, "Main").await;
    join(&app, &sam, srv.id).await;
    let media = upload(&app, &alex).await;

    let (mut ws, _) = hello(&app, &sam).await;
    let e = create_ok(&app, &alex, srv.id, "pog", &media.hash).await;
    assert_eq!(e.name, "pog");
    assert_eq!(e.server_id, srv.id);
    assert_eq!(e.hash.as_deref(), Some(media.hash.as_str()));
    assert!(!e.animated);

    match next_event(&mut ws).await {
        Event::EmojiCreated { emoji } => assert_eq!(emoji, e),
        other => panic!("{other:?}"),
    }
    let (_, ready) = hello(&app, &sam).await;
    let got = ready.custom_emoji.iter().find(|x| x.id == e.id).unwrap();
    assert_eq!(got.hash.as_deref(), Some(media.hash.as_str()));
}

#[tokio::test]
async fn names_validated_and_unique_per_server() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let srv = create_server(&app, &alex, "Main").await;
    let other = create_server(&app, &alex, "Other").await;
    let media = upload(&app, &alex).await;

    for bad in ["a", "has space", &"x".repeat(33), "dash-ed", "caf\u{e9}"] {
        let r = create(&app, &alex, srv.id, bad, &media.hash).await;
        assert_eq!(r.status(), 400, "{bad}");
    }
    create_ok(&app, &alex, srv.id, "pog", &media.hash).await;
    let r = create(&app, &alex, srv.id, "POG", &media.hash).await;
    assert_eq!(r.status(), 409);
    create_ok(&app, &alex, other.id, "pog", &media.hash).await;
}

#[tokio::test]
async fn non_member_cannot_add() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let srv = create_server(&app, &alex, "Main").await;
    let media = upload(&app, &sam).await;
    let r = create(&app, &sam, srv.id, "pog", &media.hash).await;
    assert_eq!(r.status(), 403);
}

#[tokio::test]
async fn cap_100() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let srv = create_server(&app, &alex, "Main").await;
    let media = upload(&app, &alex).await;
    for i in 0..100 {
        sqlx::query(
            "INSERT INTO custom_emoji (id, server_id, name, media_hash, animated, added_by, created_at)
             VALUES (?, ?, ?, ?, 0, ?, '2026-10-01T00:00:00.000Z')",
        )
        .bind(format!("E{i:03}"))
        .bind(srv.id.to_string())
        .bind(format!("e{i:03}"))
        .bind(&media.hash)
        .bind(alex_id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    }
    let r = create(&app, &alex, srv.id, "one_more", &media.hash).await;
    assert_eq!(r.status(), 400);
    assert!(error_message(r).await.contains("this server has 100 emoji"));

    // A deleted one frees a slot.
    let r = app
        .http
        .delete(app.url("/emoji/E000"))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    create_ok(&app, &alex, srv.id, "one_more", &media.hash).await;
}

#[tokio::test]
async fn media_must_exist_and_be_emoji_kind() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let srv = create_server(&app, &alex, "Main").await;
    let media = upload(&app, &alex).await;
    for hash in ["ab".repeat(32), "../etc/passwd".into(), String::new()] {
        let r = create(&app, &alex, srv.id, "pog", &hash).await;
        assert_eq!(r.status(), 400, "{hash}");
    }
    sqlx::query("UPDATE media SET kind = 'avatar' WHERE hash = ?")
        .bind(&media.hash)
        .execute(&app.db)
        .await
        .unwrap();
    let r = create(&app, &alex, srv.id, "pog", &media.hash).await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn rename_and_delete() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let srv = create_server(&app, &alex, "Main").await;
    join(&app, &sam, srv.id).await;
    let media = upload(&app, &alex).await;
    let e = create_ok(&app, &alex, srv.id, "pog", &media.hash).await;
    let (mut ws, _) = hello(&app, &sam).await;

    let r = app
        .http
        .patch(app.url(&format!("/emoji/{}", e.id)))
        .bearer_auth(&alex)
        .json(&json!({ "name": "poggers" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    match next_event(&mut ws).await {
        Event::EmojiUpdated { emoji } => assert_eq!(emoji.name, "poggers"),
        other => panic!("{other:?}"),
    }

    let del = || async {
        app.http
            .delete(app.url(&format!("/emoji/{}", e.id)))
            .bearer_auth(&alex)
            .send()
            .await
            .unwrap()
    };
    assert_eq!(del().await.status(), 200);
    match next_event(&mut ws).await {
        Event::EmojiDeleted { emoji } => {
            assert_eq!((emoji.id.as_str(), emoji.hash), (e.id.as_str(), None));
            assert_eq!(emoji.still_hash, None);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(del().await.status(), 404);
    let r = app
        .http
        .patch(app.url(&format!("/emoji/{}", e.id)))
        .bearer_auth(&alex)
        .json(&json!({ "name": "again" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    let (_, ready) = hello(&app, &sam).await;
    let got = ready.custom_emoji.iter().find(|x| x.id == e.id).unwrap();
    assert_eq!((got.name.as_str(), got.hash.as_deref()), ("poggers", None));

    // The name is free again.
    create_ok(&app, &alex, srv.id, "poggers", &media.hash).await;
}

#[tokio::test]
async fn only_server_members_can_change() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let srv = create_server(&app, &alex, "Main").await;
    let media = upload(&app, &alex).await;
    let e = create_ok(&app, &alex, srv.id, "pog", &media.hash).await;
    let r = app
        .http
        .delete(app.url(&format!("/emoji/{}", e.id)))
        .bearer_auth(&sam)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let r = app
        .http
        .delete(app.url("/emoji/nope"))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn deleted_emoji_media_is_cleaned() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let srv = create_server(&app, &alex, "Main").await;
    let media = upload(&app, &alex).await;
    let e = create_ok(&app, &alex, srv.id, "pog", &media.hash).await;
    let file = media::path_for(&app.cfg.media_dir, &media.hash).unwrap();

    // Live emoji pin their media.
    let n = media::cleanup(&app.db, &app.cfg.media_dir, Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(n, 0);
    assert!(file.is_file());

    let r = app
        .http
        .delete(app.url(&format!("/emoji/{}", e.id)))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let n = media::cleanup(&app.db, &app.cfg.media_dir, Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(n, 1);
    assert!(!file.exists());
}

#[tokio::test]
async fn frequent_ranks_with_decay() {
    let app = spawn().await;
    let (alex, _) = register(&app, "alex").await;
    let day0 = chrono::Utc::now() - chrono::Duration::days(30);
    let day30 = chrono::Utc::now();
    let mut conn = app.db.acquire().await.unwrap();
    let x = vec!["x".to_string()];
    for _ in 0..3 {
        emoji::record_use_at(&mut conn, alex, &x, day0)
            .await
            .unwrap();
    }
    emoji::record_use_at(&mut conn, alex, &["y".to_string()], day30)
        .await
        .unwrap();
    drop(conn);
    // 3 * 0.5^(30/14) is about 0.68, below y's 1.
    assert_eq!(
        emoji::frequent(&app.db, alex, 27).await.unwrap(),
        ["y", "x"]
    );
    assert_eq!(emoji::frequent(&app.db, alex, 1).await.unwrap(), ["y"]);
    let nobody = UserId::new();
    assert!(
        emoji::frequent(&app.db, nobody, 27)
            .await
            .unwrap()
            .is_empty()
    );
}

async fn patch_name(app: &TestApp, token: &str, id: &str, name: &str) -> reqwest::Response {
    app.http
        .patch(app.url(&format!("/emoji/{id}")))
        .bearer_auth(token)
        .json(&json!({ "name": name }))
        .send()
        .await
        .unwrap()
}

async fn delete_emoji(app: &TestApp, token: &str, id: &str) -> reqwest::Response {
    app.http
        .delete(app.url(&format!("/emoji/{id}")))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn names_are_not_trimmed() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let srv = create_server(&app, &alex, "Main").await;
    let media = upload(&app, &alex).await;
    for bad in [" pog ", "pog ", " pog", "pog\n"] {
        let r = create(&app, &alex, srv.id, bad, &media.hash).await;
        assert_eq!(r.status(), 400, "{bad:?}");
    }
    let e = create_ok(&app, &alex, srv.id, "pog", &media.hash).await;
    assert_eq!(patch_name(&app, &alex, &e.id, " kek ").await.status(), 400);
}

#[tokio::test]
async fn rename_validates_and_conflicts() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let srv = create_server(&app, &alex, "Main").await;
    let media = upload(&app, &alex).await;
    let pog = create_ok(&app, &alex, srv.id, "pog", &media.hash).await;
    create_ok(&app, &alex, srv.id, "kek", &media.hash).await;
    for bad in ["a", "has space", "dash-ed", &"x".repeat(33)] {
        assert_eq!(
            patch_name(&app, &alex, &pog.id, bad).await.status(),
            400,
            "{bad}"
        );
    }
    let r = patch_name(&app, &alex, &pog.id, "KEK").await;
    assert_eq!(r.status(), 409);
    // Same name in another case is just a rename of itself.
    assert_eq!(patch_name(&app, &alex, &pog.id, "POG").await.status(), 200);
}

#[tokio::test]
async fn deleted_emoji_is_404_for_everyone() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let srv = create_server(&app, &alex, "Main").await;
    let media = upload(&app, &alex).await;
    let e = create_ok(&app, &alex, srv.id, "pog", &media.hash).await;
    // Live: a non-member is refused.
    assert_eq!(patch_name(&app, &sam, &e.id, "kek").await.status(), 403);
    assert_eq!(delete_emoji(&app, &alex, &e.id).await.status(), 200);
    assert_eq!(patch_name(&app, &sam, &e.id, "kek").await.status(), 404);
    assert_eq!(delete_emoji(&app, &sam, &e.id).await.status(), 404);
    assert_eq!(patch_name(&app, &sam, "nope", "kek").await.status(), 404);
}

#[tokio::test]
async fn media_vanishing_during_create_is_400() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let srv = create_server(&app, &alex, "Main").await;
    let media = upload(&app, &alex).await;
    // Stands in for a cleanup that lands between the media check and the insert.
    sqlx::query(
        "CREATE TRIGGER vanish BEFORE INSERT ON custom_emoji
         BEGIN DELETE FROM media WHERE hash = NEW.media_hash; END",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let r = create(&app, &alex, srv.id, "pog", &media.hash).await;
    assert_eq!(r.status(), 400);
    assert!(error_message(r).await.contains("upload the image again"));
}

#[tokio::test]
async fn usage_decays_on_write() {
    let app = spawn().await;
    let (alex, _) = register(&app, "alex").await;
    let now = chrono::Utc::now();
    let then = now - chrono::Duration::days(7);
    sqlx::query(
        "INSERT INTO emoji_usage (user_id, emoji, score, last_used_at) VALUES (?, 'x', 4.0, ?)",
    )
    .bind(alex.to_string())
    .bind(then.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
    .execute(&app.db)
    .await
    .unwrap();
    let mut conn = app.db.acquire().await.unwrap();
    emoji::record_use_at(&mut conn, alex, &["x".to_string()], now)
        .await
        .unwrap();
    let score: f64 = sqlx::query_scalar("SELECT score FROM emoji_usage WHERE user_id = ?")
        .bind(alex.to_string())
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    // 4·0.5^(7/14) + 1
    let want = 4.0 * 0.5f64.sqrt() + 1.0;
    assert!((score - want).abs() < 1e-6, "{score} vs {want}");
}

fn gif(frames: usize) -> Vec<u8> {
    use image::codecs::gif::GifEncoder;
    use image::{Delay, Frame};
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        for i in 0..frames {
            let img = RgbaImage::from_pixel(32, 32, Rgba([(i * 80) as u8, 40, 90, 255]));
            enc.encode_frame(Frame::from_parts(
                img,
                0,
                0,
                Delay::from_numer_denom_ms(50, 1),
            ))
            .unwrap();
        }
    }
    out
}

/// Ages every media row past the hourly job's one-day grace.
async fn age_media(app: &TestApp) {
    sqlx::query("UPDATE media SET created_at = '2026-01-01T00:00:00.000Z'")
        .execute(&app.db)
        .await
        .unwrap();
}

const DAY: Duration = Duration::from_secs(24 * 3600);

#[tokio::test]
async fn deleted_animated_emoji_media_is_cleaned() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let srv = create_server(&app, &alex, "Main").await;
    let r = app
        .http
        .post(app.url("/media/emoji"))
        .bearer_auth(&alex)
        .body(gif(3))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let media: MediaInfo = r.json().await.unwrap();
    let still = media.still_hash.clone().unwrap();
    let e = create_ok(&app, &alex, srv.id, "dance", &media.hash).await;
    assert!(e.animated);
    age_media(&app).await;
    let dir = &app.cfg.media_dir;
    assert_eq!(media::cleanup(&app.db, dir, DAY).await.unwrap(), 0);

    assert_eq!(delete_emoji(&app, &alex, &e.id).await.status(), 200);
    assert_eq!(media::cleanup(&app.db, dir, DAY).await.unwrap(), 2);
    assert!(!media::path_for(dir, &media.hash).unwrap().exists());
    assert!(!media::path_for(dir, &still).unwrap().exists());
}

#[tokio::test]
async fn shared_media_survives_while_one_emoji_uses_it() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let srv = create_server(&app, &alex, "Main").await;
    let media = upload(&app, &alex).await;
    let a = create_ok(&app, &alex, srv.id, "pog", &media.hash).await;
    let b = create_ok(&app, &alex, srv.id, "kek", &media.hash).await;
    age_media(&app).await;
    let dir = &app.cfg.media_dir;
    let file = media::path_for(dir, &media.hash).unwrap();

    assert_eq!(delete_emoji(&app, &alex, &a.id).await.status(), 200);
    assert_eq!(media::cleanup(&app.db, dir, DAY).await.unwrap(), 0);
    assert!(file.is_file());

    assert_eq!(delete_emoji(&app, &alex, &b.id).await.status(), 200);
    assert_eq!(media::cleanup(&app.db, dir, DAY).await.unwrap(), 1);
    assert!(!file.exists());
}

async fn seed_usage(app: &TestApp, user: UserId, emoji: &str, score: f64) {
    sqlx::query(
        "INSERT INTO emoji_usage (user_id, emoji, score, last_used_at) VALUES (?, ?, ?, ?)",
    )
    .bind(user.to_string())
    .bind(emoji)
    .bind(score)
    .bind(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
    .execute(&app.db)
    .await
    .unwrap();
}

#[tokio::test]
async fn frequent_skips_unusable_and_fills_to_27() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let mine = create_server(&app, &alex, "Main").await;
    let theirs = create_server(&app, &sam, "Other").await;
    let media = upload(&app, &alex).await;
    let live = create_ok(&app, &alex, mine.id, "pog", &media.hash).await;
    let gone = create_ok(&app, &alex, mine.id, "kek", &media.hash).await;
    assert!(
        delete_emoji(&app, &alex, &gone.id)
            .await
            .status()
            .is_success()
    );
    let theirs_media = upload(&app, &sam).await;
    let foreign = create_ok(&app, &sam, theirs.id, "lul", &theirs_media.hash).await;

    // The unusable ones rank highest: deleted, another server's, and one that never existed.
    seed_usage(&app, alex_id, &format!("custom:{}", gone.id), 100.0).await;
    seed_usage(&app, alex_id, &format!("custom:{}", foreign.id), 99.0).await;
    seed_usage(&app, alex_id, "custom:01JAAAAAAAAAAAAAAAAAAAAAAA", 98.0).await;
    let live_key = format!("custom:{}", live.id);
    seed_usage(&app, alex_id, &live_key, 50.0).await;
    let standard: Vec<String> = (0..30)
        .map(|i| char::from_u32(0x1F600 + i).unwrap().to_string())
        .collect();
    for (i, e) in standard.iter().enumerate() {
        seed_usage(&app, alex_id, e, 40.0 - i as f64).await;
    }

    let want: Vec<String> = std::iter::once(live_key.clone())
        .chain(standard[..26].iter().cloned())
        .collect();
    assert_eq!(emoji::frequent(&app.db, alex_id, 27).await.unwrap(), want);
    let (_, ready) = hello(&app, &alex).await;
    assert_eq!(ready.frequent_emoji, want);

    // Fewer usable than 27: all of them, still none of the rest.
    sqlx::query("DELETE FROM emoji_usage WHERE emoji NOT LIKE 'custom:%'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        emoji::frequent(&app.db, alex_id, 27).await.unwrap(),
        [live_key]
    );
}

#[tokio::test]
async fn one_write_counts_each_emoji_once() {
    let app = spawn().await;
    let (alex, _) = register(&app, "alex").await;
    let mut conn = app.db.acquire().await.unwrap();
    let both = ["\u{2764}".to_string(), "\u{2764}\u{FE0F}".to_string()];
    emoji::record_use(&mut conn, alex, &both).await.unwrap();
    let rows: Vec<(String, f64)> =
        sqlx::query_as("SELECT emoji, score FROM emoji_usage WHERE user_id = ?")
            .bind(alex.to_string())
            .fetch_all(&mut *conn)
            .await
            .unwrap();
    assert_eq!(rows, [("\u{2764}".to_string(), 1.0)]);
}
