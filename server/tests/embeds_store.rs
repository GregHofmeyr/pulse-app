//! Link embed storage (migration 0012): rows go out with their message, go with its delete, and
//! keep their thumbnails through the media cleanup while referenced.
mod common;

use std::time::Duration;

use common::*;
use pulse_protocol::ids::{ChannelId, MessageId};
use pulse_protocol::rest::{EmbedKind, EmbedProvider, EmbedThumb, LinkEmbed, Message};
use pulse_server::media;

const OLD: &str = "2020-01-01T00:00:00.000Z";

/// alex with a server and its `#general`.
async fn setup() -> (TestApp, String, ChannelId) {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let server = create_server(&app, &token, "Main").await;
    let ch = general(&app, &token, server.id).await.id;
    (app, token, ch)
}

/// One `link_embeds` row for `message`, as the unfurler stores it.
async fn insert_embed(
    app: &TestApp,
    message: MessageId,
    position: i64,
    url: &str,
    thumb: Option<&str>,
    hidden: bool,
) {
    sqlx::query(
        "INSERT INTO link_embeds (message_id, position, url, provider, provider_id, kind, title,
                                  description, site_name, author, thumb_hash, thumb_width,
                                  thumb_height, start_secs, vertical, hidden, fetched_at)
         VALUES (?, ?, ?, 'youtube', 'dQw4w9WgXcQ', 'player', 'A video', 'About it', 'YouTube',
                 'alex', ?, 640, 360, 90, 0, ?, ?)",
    )
    .bind(message.to_string())
    .bind(position)
    .bind(url)
    .bind(thumb)
    .bind(hidden)
    .bind(OLD)
    .execute(&app.db)
    .await
    .unwrap();
}

/// A `link_thumb` media row past any grace period, with its file on disk.
async fn insert_thumb(app: &TestApp, hash: &str) {
    sqlx::query(
        "INSERT INTO media (hash, kind, content_type, size, width, height, frames, created_at)
         VALUES (?, 'link_thumb', 'image/webp', 4, 640, 360, 1, ?)",
    )
    .bind(hash)
    .bind(OLD)
    .execute(&app.db)
    .await
    .unwrap();
    let path = media::path_for(&app.cfg.media_dir, hash).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"webp").unwrap();
}

async fn rows(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM link_embeds")
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn delete(app: &TestApp, token: &str, id: MessageId) {
    let r = app
        .http
        .delete(app.url(&format!("/messages/{id}")))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
}

#[tokio::test]
async fn messages_carry_their_embed_rows() {
    let (app, token, ch) = setup().await;
    let a = send(
        &app,
        &token,
        ch,
        "see https://example.com/a and https://example.com/b",
    )
    .await;
    let b = send(&app, &token, ch, "and https://example.com/c").await;
    assert!(a.embeds.is_empty());
    let thumb = "ab".repeat(32);
    // Out of order on purpose: they come back by position.
    insert_embed(&app, a.id, 1, "https://example.com/b", None, false).await;
    insert_embed(&app, a.id, 0, "https://example.com/a", Some(&thumb), false).await;
    insert_embed(&app, a.id, 2, "https://example.com/hidden", None, true).await;
    insert_embed(&app, b.id, 0, "https://example.com/c", None, true).await;

    let page: Vec<Message> = get_json(&app, &token, &format!("/channels/{ch}/messages")).await;
    let got = |id: MessageId| page.iter().find(|m| m.id == id).unwrap().embeds.clone();
    let a_embeds = got(a.id);
    assert_eq!(
        a_embeds.iter().map(|e| e.url.as_str()).collect::<Vec<_>>(),
        ["https://example.com/a", "https://example.com/b"]
    );
    assert_eq!(
        a_embeds[0],
        LinkEmbed {
            url: "https://example.com/a".into(),
            provider: EmbedProvider::Youtube,
            kind: EmbedKind::Player,
            provider_id: Some("dQw4w9WgXcQ".into()),
            title: Some("A video".into()),
            description: Some("About it".into()),
            site_name: Some("YouTube".into()),
            author: Some("alex".into()),
            thumb: Some(EmbedThumb {
                hash: thumb,
                width: 640,
                height: 360,
            }),
            start: Some(90),
            vertical: false,
        }
    );
    assert_eq!(a_embeds[1].thumb, None);
    assert!(got(b.id).is_empty(), "a hidden row never goes out");
}

#[tokio::test]
async fn deleting_a_message_removes_its_embeds() {
    let (app, token, ch) = setup().await;
    let a = send(&app, &token, ch, "https://example.com/a").await;
    let b = send(&app, &token, ch, "https://example.com/b").await;
    insert_embed(&app, a.id, 0, "https://example.com/a", None, false).await;
    insert_embed(&app, b.id, 0, "https://example.com/b", None, false).await;
    delete(&app, &token, a.id).await;
    assert_eq!(rows(&app).await, 1, "only the deleted message's rows go");
    let page: Vec<Message> = get_json(&app, &token, &format!("/channels/{ch}/messages")).await;
    let gone = page.iter().find(|m| m.id == a.id).unwrap();
    assert!(gone.deleted && gone.embeds.is_empty());
}

/// Deleting a quote soft-deletes its thread's replies, and their previews go with them.
#[tokio::test]
async fn deleting_a_quote_removes_its_replies_embeds() {
    let app = spawn().await;
    let (alex, token) = register(&app, "alex").await;
    let server = create_server(&app, &token, "Main").await;
    let r = post_json(
        &app,
        &token,
        &format!("/servers/{}/channels", server.id),
        serde_json::json!({ "kind": "quotes", "name": "quotes" }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let q: pulse_protocol::rest::Channel = r.json().await.unwrap();
    let r = post_json(
        &app,
        &token,
        &format!("/channels/{}/quotes", q.id),
        serde_json::json!({ "lines": [{ "speaker_id": alex, "text": "honest" }] }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let root: Message = r.json().await.unwrap();
    let r = post_json(
        &app,
        &token,
        &format!("/channels/{}/messages", q.id),
        serde_json::json!({ "content": "https://example.com/r", "thread_root_id": root.id }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let reply: Message = r.json().await.unwrap();
    insert_embed(&app, reply.id, 0, "https://example.com/r", None, false).await;
    delete(&app, &token, root.id).await;
    assert_eq!(rows(&app).await, 0);
}

#[tokio::test]
async fn cleanup_keeps_a_referenced_link_thumb() {
    let (app, token, ch) = setup().await;
    let a = send(&app, &token, ch, "https://example.com/a").await;
    let thumb = "cd".repeat(32);
    insert_thumb(&app, &thumb).await;
    insert_embed(&app, a.id, 0, "https://example.com/a", Some(&thumb), false).await;
    let dir = &app.cfg.media_dir;
    assert_eq!(
        media::cleanup(&app.db, dir, Duration::ZERO).await.unwrap(),
        0
    );
    assert!(media::path_for(dir, &thumb).unwrap().is_file());
}

#[tokio::test]
async fn cleanup_drops_a_link_thumb_once_its_message_is_deleted() {
    let (app, token, ch) = setup().await;
    let a = send(&app, &token, ch, "https://example.com/a").await;
    let thumb = "ef".repeat(32);
    insert_thumb(&app, &thumb).await;
    insert_embed(&app, a.id, 0, "https://example.com/a", Some(&thumb), false).await;
    delete(&app, &token, a.id).await;
    let dir = &app.cfg.media_dir;
    assert_eq!(
        media::cleanup(&app.db, dir, Duration::ZERO).await.unwrap(),
        1
    );
    assert!(!media::path_for(dir, &thumb).unwrap().exists());
}
