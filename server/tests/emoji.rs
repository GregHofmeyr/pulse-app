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
    let file = media::path_for(&app.cfg.media_dir, &media.hash);

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
