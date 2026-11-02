//! Unfurling links into previews against the fake provider (`testing::fake_provider`): oEmbed
//! and OpenGraph per provider, thumbnails stored as `link_thumb` media, the cache, and `store`.
mod common;

use common::*;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{EmbedKind, EmbedProvider, EmbedThumb, Message};
use pulse_server::config::Config;
use pulse_server::embeds::{self, Unfurled};
use pulse_server::limits::{LimitsConfig, Rate};
use pulse_server::media;
use pulse_server::testing::fake_provider::{
    FakeProvider, SPOTIFY_ID, TIKTOK_ID, TIKTOK_URL, YOUTUBE_ID,
};

const OLD: &str = "2020-01-01T00:00:00.000Z";

struct Env {
    app: TestApp,
    fake: FakeProvider,
    alex: UserId,
    token: String,
    ch: ChannelId,
}

impl Env {
    async fn unfurl(&self, url: &str) -> Option<Unfurled> {
        self.app
            .embeds
            .unfurl(&self.app.db, &self.app.cfg.media_dir, self.alex, url)
            .await
    }
}

/// alex with a server and its `#general`, on an app whose unfurler reads the fake provider.
/// Sends don't unfurl on their own here (no `unfurl_user` tokens): these tests call the
/// unfurler and `store` themselves (`tests/embeds.rs` covers the automatic path).
async fn setup() -> Env {
    let fake = FakeProvider::start().await;
    let app = spawn_with(Config {
        embeds: fake.config(),
        limits: LimitsConfig {
            unfurl_user: Rate::new(0, 0.0),
            ..LimitsConfig::unlimited()
        },
        ..test_config()
    })
    .await;
    let (alex, token) = register(&app, "alex").await;
    let server = create_server(&app, &token, "Main").await;
    let ch = general(&app, &token, server.id).await.id;
    Env {
        app,
        fake,
        alex,
        token,
        ch,
    }
}

/// The thumbnail is a `link_thumb` media row with its file on disk, fitted to 480 wide.
async fn assert_stored_thumb(env: &Env, thumb: &EmbedThumb) {
    assert_eq!((thumb.width, thumb.height), (480, 270));
    let kind: String = sqlx::query_scalar("SELECT kind FROM media WHERE hash = ?")
        .bind(&thumb.hash)
        .fetch_one(&env.app.db)
        .await
        .unwrap();
    assert_eq!(kind, "link_thumb");
    let path = media::path_for(&env.app.cfg.media_dir, &thumb.hash).unwrap();
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[8..12], b"WEBP");
}

fn requested(env: &Env, prefix: &str) -> bool {
    env.fake.requests().iter().any(|r| r.starts_with(prefix))
}

#[tokio::test]
async fn youtube_card_from_oembed_with_a_webp_thumbnail() {
    let env = setup().await;
    let u = env
        .unfurl(&format!(
            "https://www.youtube.com/watch?v={YOUTUBE_ID}&t=90&si=tracking"
        ))
        .await
        .unwrap();
    let thumb = u.thumb.clone().unwrap();
    assert_eq!(
        u,
        Unfurled {
            provider: EmbedProvider::Youtube,
            kind: EmbedKind::Player,
            provider_id: Some(YOUTUBE_ID.into()),
            // Markup in a provider's text is kept as text, never interpreted.
            title: Some("A video with a <b>tag</b>".into()),
            description: None,
            site_name: Some("YouTube".into()),
            author: Some("alex".into()),
            thumb: Some(thumb.clone()),
            start: Some(90),
            vertical: false,
        }
    );
    assert_stored_thumb(&env, &thumb).await;
    // oEmbed is asked about the canonical URL: no tracking or start parameters leave.
    assert!(
        requested(
            &env,
            "/oembed/youtube?url=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3DdQw4w9WgXcQ&format=json"
        ),
        "{:?}",
        env.fake.requests()
    );
    assert!(requested(&env, "/thumb.png"));
}

#[tokio::test]
async fn tiktok_short_link_resolves_to_a_vertical_player() {
    let env = setup().await;
    let u = env
        .unfurl(&env.fake.url("www.tiktok.com", "/t/ZTabcdef/"))
        .await
        .unwrap();
    assert_eq!(u.provider, EmbedProvider::Tiktok);
    assert_eq!(u.kind, EmbedKind::Player);
    assert_eq!(u.provider_id.as_deref(), Some(TIKTOK_ID));
    assert!(u.vertical);
    assert_eq!(u.title.as_deref(), Some("A dance"));
    assert_eq!(u.author.as_deref(), Some("sam"));
    assert_eq!(u.site_name.as_deref(), Some("TikTok"));
    assert!(u.thumb.is_some());
    let reqs = env.fake.requests();
    assert!(requested(&env, "/t/ZTabcdef/"), "{reqs:?}");
    // The redirect's target was only read, never requested; oEmbed got the video's URL.
    let want = format!(
        "/oembed/tiktok?url={}&format=json",
        TIKTOK_URL
            .replace(':', "%3A")
            .replace('/', "%2F")
            .replace('@', "%40")
    );
    assert!(requested(&env, &want), "{reqs:?}");
}

#[tokio::test]
async fn spotify_soundcloud_and_twitch_cards() {
    let env = setup().await;
    let s = env
        .unfurl(&format!(
            "https://open.spotify.com/intl-de/track/{SPOTIFY_ID}?si=x"
        ))
        .await
        .unwrap();
    assert_eq!(s.provider, EmbedProvider::Spotify);
    assert_eq!(s.kind, EmbedKind::Player);
    assert_eq!(
        s.provider_id.as_deref(),
        Some(format!("track:{SPOTIFY_ID}").as_str())
    );
    assert_eq!(s.title.as_deref(), Some("A song"));
    assert_eq!(s.site_name.as_deref(), Some("Spotify"));
    assert!(s.thumb.is_some());
    assert!(requested(
        &env,
        &format!(
            "/oembed/spotify?url=https%3A%2F%2Fopen.spotify.com%2Ftrack%2F{SPOTIFY_ID}&format=json"
        )
    ));

    let c = env
        .unfurl("https://soundcloud.com/alex-music/first-track")
        .await
        .unwrap();
    assert_eq!(c.provider, EmbedProvider::Soundcloud);
    assert_eq!(c.provider_id.as_deref(), Some("alex-music/first-track"));
    assert_eq!(c.title.as_deref(), Some("A track by alex"));
    assert_eq!(c.description.as_deref(), Some("Recorded at home."));
    assert_eq!(c.author.as_deref(), Some("alex"));
    assert_eq!(c.site_name.as_deref(), Some("SoundCloud"));

    let t = env
        .unfurl(&env.fake.url("www.twitch.tv", "/alex/clip/FunnyClip-abc_12"))
        .await
        .unwrap();
    assert_eq!(t.provider, EmbedProvider::Twitch);
    assert_eq!(t.kind, EmbedKind::Player);
    assert_eq!(t.provider_id.as_deref(), Some("FunnyClip-abc_12"));
    assert_eq!(t.title.as_deref(), Some("FunnyClip-abc_12 by alex"));
    assert_eq!(t.site_name.as_deref(), Some("Twitch"));
    assert_stored_thumb(&env, t.thumb.as_ref().unwrap()).await;

    // A player whose oEmbed fails gets no card at all.
    assert_eq!(
        env.unfurl("https://soundcloud.com/alex/missing-track")
            .await,
        None
    );
}

#[tokio::test]
async fn instagram_card_without_oembed_is_a_placeholder_card() {
    let env = setup().await;
    let u = env
        .unfurl("https://www.instagram.com/reel/Cabc_12-x/?igsh=1")
        .await
        .unwrap();
    assert_eq!(
        u,
        Unfurled {
            provider: EmbedProvider::Instagram,
            kind: EmbedKind::Player,
            provider_id: Some("reel:Cabc_12-x".into()),
            title: None,
            description: None,
            site_name: Some("Instagram".into()),
            author: None,
            thumb: None,
            start: None,
            vertical: true,
        }
    );
    assert!(requested(
        &env,
        "/oembed/instagram?url=https%3A%2F%2Fwww.instagram.com%2Freel%2FCabc_12-x%2F"
    ));
    // When the oEmbed answers, its title is the caption.
    let u = env
        .unfurl("https://www.instagram.com/p/Ccaption1/")
        .await
        .unwrap();
    assert_eq!(u.title.as_deref(), Some("A caption"));
    assert_eq!(u.author.as_deref(), Some("alex"));
    assert_eq!(u.site_name.as_deref(), Some("Instagram"));
    assert!(!u.vertical);
}

#[tokio::test]
async fn generic_page_gets_an_og_card() {
    let env = setup().await;
    let u = env
        .unfurl(&env.fake.url("site.test", "/page"))
        .await
        .unwrap();
    let thumb = u.thumb.clone().unwrap();
    assert_eq!(
        u,
        Unfurled {
            provider: EmbedProvider::Generic,
            kind: EmbedKind::Card,
            provider_id: None,
            title: Some("A page & more".into()),
            description: Some("What the page is about.".into()),
            site_name: Some("Example".into()),
            author: None,
            thumb: Some(thumb.clone()),
            start: None,
            vertical: false,
        }
    );
    // The page's relative `og:image` was resolved against the page.
    assert_stored_thumb(&env, &thumb).await;
    assert!(requested(&env, "/thumb.png"));
}

#[tokio::test]
async fn a_page_without_metadata_gets_nothing() {
    let env = setup().await;
    assert_eq!(env.unfurl(&env.fake.url("site.test", "/bare")).await, None);
    assert_eq!(
        env.unfurl(&env.fake.url("site.test", "/no-such-page"))
            .await,
        None
    );
    // A private address is never fetched.
    let before = env.fake.hits();
    assert_eq!(env.unfurl("http://10.0.0.1/page").await, None);
    assert_eq!(env.unfurl("not a link").await, None);
    assert_eq!(env.fake.hits(), before);
}

#[tokio::test]
async fn the_cache_serves_a_second_unfurl_without_fetching() {
    let env = setup().await;
    let page = env.fake.url("site.test", "/page");
    let first = env.unfurl(&page).await.unwrap();
    let hits = env.fake.hits();
    // The same link, normalised (its fragment dropped).
    let again = env.unfurl(&format!("{page}#section")).await.unwrap();
    assert_eq!(again, first);
    assert_eq!(env.fake.hits(), hits);
    // A link that gave nothing is remembered too.
    let bare = env.fake.url("site.test", "/bare");
    assert_eq!(env.unfurl(&bare).await, None);
    let hits = env.fake.hits();
    assert_eq!(env.unfurl(&bare).await, None);
    assert_eq!(env.fake.hits(), hits);
}

async fn thumb_row(env: &Env, msg: &Message) -> Option<String> {
    sqlx::query_scalar("SELECT thumb_hash FROM link_embeds WHERE message_id = ?")
        .bind(msg.id.to_string())
        .fetch_one(&env.app.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_cached_thumb_that_was_cleaned_up_is_not_reused() {
    let env = setup().await;
    let page = env.fake.url("site.test", "/page");
    let u = env.unfurl(&page).await.unwrap();
    let hash = u.thumb.as_ref().unwrap().hash.clone();

    // While the media row is there, storing restarts its cleanup clock and keeps it.
    sqlx::query("UPDATE media SET created_at = ? WHERE hash = ?")
        .bind(OLD)
        .bind(&hash)
        .execute(&env.app.db)
        .await
        .unwrap();
    let content = format!("look {page}");
    let m1 = send(&env.app, &env.token, env.ch, &content).await;
    assert!(
        embeds::store(&env.app.db, m1.id, &content, vec![(0, page.clone(), u)])
            .await
            .unwrap()
    );
    assert_eq!(thumb_row(&env, &m1).await.as_deref(), Some(hash.as_str()));
    let created: String = sqlx::query_scalar("SELECT created_at FROM media WHERE hash = ?")
        .bind(&hash)
        .fetch_one(&env.app.db)
        .await
        .unwrap();
    assert!(created.as_str() > OLD, "{created}");

    // The cleanup takes it (say the first message went); the cache still names it.
    sqlx::query("DELETE FROM media WHERE hash = ?")
        .bind(&hash)
        .execute(&env.app.db)
        .await
        .unwrap();
    let hits = env.fake.hits();
    let cached = env.unfurl(&page).await.unwrap();
    assert_eq!(env.fake.hits(), hits);
    assert_eq!(cached.thumb.as_ref().unwrap().hash, hash);
    let m2 = send(&env.app, &env.token, env.ch, &content).await;
    assert!(
        embeds::store(
            &env.app.db,
            m2.id,
            &content,
            vec![(0, page.clone(), cached)]
        )
        .await
        .unwrap()
    );
    // The preview is kept, without the dead image.
    assert_eq!(thumb_row(&env, &m2).await, None);
    let title: Option<String> =
        sqlx::query_scalar("SELECT title FROM link_embeds WHERE message_id = ?")
            .bind(m2.id.to_string())
            .fetch_one(&env.app.db)
            .await
            .unwrap();
    assert_eq!(title.as_deref(), Some("A page & more"));
}

#[tokio::test]
async fn store_writes_nothing_after_the_content_changed() {
    let env = setup().await;
    let page = env.fake.url("site.test", "/page");
    let u = env.unfurl(&page).await.unwrap();
    let content = format!("look {page}");
    let m = send(&env.app, &env.token, env.ch, &content).await;
    let count = || async {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM link_embeds")
            .fetch_one(&env.app.db)
            .await
            .unwrap()
    };

    // Read before an edit: the edit wins.
    let stored = embeds::store(
        &env.app.db,
        m.id,
        "an older text",
        vec![(0, page.clone(), u.clone())],
    )
    .await
    .unwrap();
    assert!(!stored);
    assert_eq!(count().await, 0);

    // Stored, hidden by the sender, then stored again (a re-unfurl): still hidden.
    assert!(
        embeds::store(
            &env.app.db,
            m.id,
            &content,
            vec![(0, page.clone(), u.clone())]
        )
        .await
        .unwrap()
    );
    sqlx::query("UPDATE link_embeds SET hidden = 1")
        .execute(&env.app.db)
        .await
        .unwrap();
    assert!(
        embeds::store(
            &env.app.db,
            m.id,
            &content,
            vec![(0, page.clone(), u.clone())]
        )
        .await
        .unwrap()
    );
    let hidden: bool = sqlx::query_scalar("SELECT hidden FROM link_embeds")
        .fetch_one(&env.app.db)
        .await
        .unwrap();
    assert!(hidden);

    // A deleted message takes nothing.
    let m2 = send(&env.app, &env.token, env.ch, &content).await;
    sqlx::query("UPDATE messages SET deleted_at = ? WHERE id = ?")
        .bind(OLD)
        .bind(m2.id.to_string())
        .execute(&env.app.db)
        .await
        .unwrap();
    assert!(
        !embeds::store(&env.app.db, m2.id, &content, vec![(0, page, u)])
            .await
            .unwrap()
    );
    assert_eq!(count().await, 1);
}
