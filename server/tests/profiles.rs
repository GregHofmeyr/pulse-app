//! Profiles: migration 0010 and the name style carried inside `User`.
mod common;

use std::borrow::Cow;
use std::time::Duration;

use common::*;
use futures::SinkExt;
use pulse_protocol::gateway::{ClientFrame, Ready, ServerFrame};
use pulse_protocol::rest::{NameStyle, User};
use sqlx::sqlite::SqliteConnectOptions;
use tokio_tungstenite::tungstenite::Message as Ws;

async fn ready_of(app: &TestApp, token: &str) -> Ready {
    let (mut ws, _) = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap();
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    loop {
        match next_frame_beating(&mut ws, Duration::from_secs(3)).await {
            Some(ServerFrame::Ready(r)) => return r,
            Some(_) => continue,
            None => panic!("expected Ready"),
        }
    }
}

fn styled(u: &User, id: pulse_protocol::ids::UserId) {
    assert_eq!(u.id, id);
    assert_eq!(
        u.style,
        NameStyle {
            font: Some("pacifico".into()),
            colors: vec!["#ffc4a3".into(), "#ff7fae".into()],
        }
    );
    assert_eq!(u.avatar_hash.as_deref(), Some("avatar-full"));
    assert_eq!(u.avatar_still.as_deref(), Some("avatar-still"));
}

#[tokio::test]
async fn ready_carries_name_styles() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let s = create_server(&app, &alex, "Main").await;
    let r = post_json(
        &app,
        &sam,
        &format!("/servers/{}/join", s.id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(r.status(), 204);

    for q in [
        "INSERT INTO media (hash, kind, content_type, size, width, height, frames, still_hash, created_at) \
         VALUES ('avatar-still', 'avatar', 'image/webp', 1, 256, 256, 1, NULL, '2026-10-01T00:00:00.000Z')",
        "INSERT INTO media (hash, kind, content_type, size, width, height, frames, still_hash, created_at) \
         VALUES ('avatar-full', 'avatar', 'image/webp', 1, 256, 256, 4, 'avatar-still', '2026-10-01T00:00:00.000Z')",
    ] {
        sqlx::query(q).execute(&app.db).await.unwrap();
    }
    sqlx::query(
        "UPDATE users SET name_font = 'pacifico', name_color = '#ffc4a3,#ff7fae', avatar_hash = 'avatar-full' WHERE id = ?",
    )
    .bind(alex_id.to_string())
    .execute(&app.db)
    .await
    .unwrap();

    let ready = ready_of(&app, &sam).await;
    styled(
        &ready
            .people
            .iter()
            .find(|p| p.user.id == alex_id)
            .unwrap()
            .user,
        alex_id,
    );
    let m = ready
        .members
        .iter()
        .flat_map(|sm| sm.members.iter())
        .find(|m| m.user.id == alex_id)
        .unwrap();
    styled(&m.user, alex_id);

    let me: User = get_json(&app, &alex, "/me").await;
    styled(&me, alex_id);
}

#[tokio::test]
async fn migration_0010_adds_defaults_and_drops_images() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(false);
    let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();

    // A database as the previous release left it: migrations up to 0009 only.
    let mut migrator = sqlx::migrate::Migrator::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"),
    )
    .await
    .unwrap();
    assert!(
        migrator.migrations.iter().any(|m| m.version == 10),
        "0010 exists"
    );
    migrator.migrations = Cow::Owned(
        migrator
            .migrations
            .iter()
            .filter(|m| m.version <= 9)
            .cloned()
            .collect(),
    );
    migrator.run(&pool).await.unwrap();

    let ts = "2026-10-01T00:00:00.000Z";
    let user = "01J0000000000000000000USER";
    sqlx::query(&format!(
        "INSERT INTO users (id, username, password_hash, created_at) VALUES ('{user}', 'alex', 'x', '{ts}')"
    ))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(&format!(
        "INSERT INTO images (hash, bytes, created_at) VALUES ('h', x'00', '{ts}')"
    ))
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    // Upgrade the way the server does at startup.
    let pool = pulse_server::db::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    type Row = (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let row: Row = sqlx::query_as(
        "SELECT theme_top, theme_bottom, banner_hash, name_font, name_color, pronouns, bio FROM users WHERE id = ?",
    )
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        row,
        (
            "#3a3d46".into(),
            "#16171b".into(),
            None,
            None,
            None,
            None,
            None
        )
    );
    let images: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name = 'images'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(images, 0);
    let broken: Vec<(String, i64, String, i64)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(broken.is_empty(), "{broken:?}");
}
