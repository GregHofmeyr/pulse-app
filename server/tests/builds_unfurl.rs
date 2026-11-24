//! Builds (#616): a build's link fetched for its title, thumbnail and player, and its game guessed from that title,
//! against the fake provider (`testing::fake_provider`).
mod common;

use std::time::Duration;

use common::*;
use futures::SinkExt;
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{Channel, EmbedProvider, Message};
use pulse_server::config::Config;
use pulse_server::limits::{LimitsConfig, Rate};
use pulse_server::testing::fake_provider::{FakeProvider, YOUTUBE_ID};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;

struct Env {
    app: TestApp,
    fake: FakeProvider,
    alex: UserId,
    token: String,
    server: ServerId,
    ch: ChannelId,
}

/// alex with a server and a builds channel, on an app whose unfurler reads the fake provider. With `auto` off, a new
/// build's link isn't fetched on its own (no `unfurl_user` tokens): the test calls `builds::unfurl::fill` itself.
async fn setup(auto: bool) -> Env {
    let fake = FakeProvider::start().await;
    let limits = if auto {
        LimitsConfig::unlimited()
    } else {
        LimitsConfig {
            unfurl_user: Rate::new(0, 0.0),
            ..LimitsConfig::unlimited()
        }
    };
    let app = spawn_with(Config {
        embeds: fake.config(),
        limits,
        ..test_config()
    })
    .await;
    let (alex, token) = register(&app, "alex").await;
    let s = create_server(&app, &token, "Main").await;
    let r = post_json(
        &app,
        &token,
        &format!("/servers/{}/channels", s.id),
        json!({ "kind": "builds", "name": "builds" }),
    )
    .await;
    let ch: Channel = r.json().await.unwrap();
    Env {
        app,
        fake,
        alex,
        token,
        server: s.id,
        ch: ch.id,
    }
}

impl Env {
    async fn build(&self, body: serde_json::Value) -> Message {
        let r = post_json(&self.app, &self.token, &format!("/channels/{}/builds", self.ch), body).await;
        assert_eq!(r.status(), 200);
        r.json().await.unwrap()
    }

    async fn fill(&self, id: MessageId, link: &str) {
        pulse_server::builds::unfurl::fill(&self.app.state(), id, self.alex, link.to_string())
            .await
            .unwrap();
    }

    async fn get(&self, id: MessageId) -> Message {
        get_json(&self.app, &self.token, &format!("/builds/{id}")).await
    }

    async fn game(&self, name: &str) -> pulse_protocol::rest::Game {
        let r = post_json(
            &self.app,
            &self.token,
            &format!("/servers/{}/games", self.server),
            json!({ "name": name }),
        )
        .await;
        r.json().await.unwrap()
    }

    fn titled(&self, title: &str) -> String {
        self.fake.url("site.test", &format!("/titled/{}", title.replace(' ', "%20")))
    }
}

#[tokio::test]
async fn a_youtube_link_fills_title_thumb_and_provider() {
    let env = setup(true).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(env.app.ws_url("/gateway"))
        .await
        .unwrap();
    let hello = serde_json::to_string(&ClientFrame::Hello {
        token: env.token.clone(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(hello)).await.unwrap();
    let link = format!("https://youtu.be/{YOUTUBE_ID}");
    // the automatic path: the create starts the fetch
    let m = env.build(json!({ "link_url": link })).await;
    let updated = loop {
        match next_frame_beating(&mut ws, Duration::from_secs(5)).await {
            Some(ServerFrame::Event(Event::MessageUpdated { message })) if message.id == m.id => {
                break message;
            }
            Some(_) => continue,
            None => panic!("expected the build's MessageUpdated"),
        }
    };
    let b = updated.build.unwrap();
    assert_eq!(b.title, "A video with a <b>tag</b>");
    assert!(!b.title_auto);
    let l = b.link.unwrap();
    assert_eq!(l.provider, EmbedProvider::Youtube);
    assert_eq!(l.provider_id.as_deref(), Some(YOUTUBE_ID));
    assert_eq!(l.title.as_deref(), Some("A video with a <b>tag</b>"));
    assert!(l.thumb.is_some());
    assert_eq!(updated.content, "A video with a <b>tag</b>\nA video with a <b>tag</b>");
}

#[tokio::test]
async fn the_game_is_guessed_from_the_link_title() {
    let env = setup(false).await;
    let tfd = env.game("The First Descendant").await;
    env.game("Warframe").await;
    let link = env.titled("Raven Crit Build - The First Descendant");
    let m = env.build(json!({ "link_url": link })).await;
    env.fill(m.id, &link).await;
    let b = env.get(m.id).await.build.unwrap();
    assert_eq!(b.title, "Raven Crit Build - The First Descendant");
    assert_eq!(b.game_id, Some(tfd.id));
    assert!(b.game_guessed);
    assert!(b.quick_drop, "a guess isn't a detail the author added");
    // a game the author picked is never replaced
    let picked = env.game("Deadlock").await;
    let m = env.build(json!({ "link_url": link, "game_id": picked.id })).await;
    env.fill(m.id, &link).await;
    let b = env.get(m.id).await.build.unwrap();
    assert_eq!((b.game_id, b.game_guessed), (Some(picked.id), false));
}

#[tokio::test]
async fn an_unreachable_link_keeps_the_link_as_title() {
    let env = setup(false).await;
    let link = env.fake.url("site.test", "/missing/planner");
    let m = env.build(json!({ "link_url": link })).await;
    env.fill(m.id, &link).await;
    let b = env.get(m.id).await.build.unwrap();
    assert_eq!(b.title, "site.test/missing/planner");
    assert!(b.title_auto);
    assert_eq!(b.link.unwrap().title, None);
}

#[tokio::test]
async fn unfurl_never_overwrites_a_title_the_author_typed() {
    let env = setup(false).await;
    let link = env.fake.url("site.test", "/held");
    let m = env.build(json!({ "link_url": link })).await;
    let state = env.app.state();
    let (id, alex, l) = (m.id, env.alex, link.clone());
    let fetch = tokio::spawn(async move { pulse_server::builds::unfurl::fill(&state, id, alex, l).await });
    // the fetch is in flight when alex names it
    for _ in 0..100 {
        if env.fake.requests().iter().any(|r| r.starts_with("/held")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // as the author's edit writes it (`PUT /builds/{id}`, Task 4)
    sqlx::query("UPDATE builds SET title = 'Mine', title_auto = 0 WHERE message_id = ?")
        .bind(m.id.to_string())
        .execute(&env.app.db)
        .await
        .unwrap();
    env.fake.release();
    fetch.await.unwrap().unwrap();
    let b = env.get(m.id).await.build.unwrap();
    assert_eq!(b.title, "Mine");
    assert_eq!(b.link.unwrap().title.as_deref(), Some("A page & more"), "the page is still filled in");
}

#[tokio::test]
async fn a_long_page_title_is_cut_to_120() {
    let env = setup(false).await;
    let long = "word ".repeat(40);
    let link = env.titled(long.trim());
    let m = env.build(json!({ "link_url": link })).await;
    env.fill(m.id, &link).await;
    let b = env.get(m.id).await.build.unwrap();
    assert_eq!(b.title.chars().count(), 120);
    assert!(b.title.ends_with('…'));
}

#[tokio::test]
async fn a_builds_link_thumb_survives_media_cleanup() {
    let env = setup(false).await;
    let link = env.titled("Bunny");
    let m = env.build(json!({ "link_url": link })).await;
    env.fill(m.id, &link).await;
    let thumb = env.get(m.id).await.build.unwrap().link.unwrap().thumb.unwrap();
    // old enough for the cleanup to take, were nothing holding it
    sqlx::query("UPDATE media SET created_at = '2020-01-01T00:00:00.000Z' WHERE hash = ?")
        .bind(&thumb.hash)
        .execute(&env.app.db)
        .await
        .unwrap();
    pulse_server::media::cleanup(&env.app.db, &env.app.cfg.media_dir, Duration::ZERO)
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media WHERE hash = ?")
        .bind(&thumb.hash)
        .fetch_one(&env.app.db)
        .await
        .unwrap();
    assert_eq!(left, 1);
}

/// A link changed (or removed) while its old page was being fetched: the old page fills nothing in.
#[tokio::test]
async fn a_stale_fetch_fills_nothing() {
    let env = setup(false).await;
    let link = env.titled("Old page");
    let m = env.build(json!({ "link_url": link })).await;
    sqlx::query("UPDATE builds SET link_url = 'https://example.com/new' WHERE message_id = ?")
        .bind(m.id.to_string())
        .execute(&env.app.db)
        .await
        .unwrap();
    env.fill(m.id, &link).await;
    let b = env.get(m.id).await.build.unwrap();
    assert_eq!(b.link.unwrap().title, None);
    assert!(b.title_auto);
}
