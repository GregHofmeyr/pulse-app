//! Link previews on messages: unfurled after a send or an edit (against the fake provider,
//! `testing::fake_provider`), hidden by the sender, rate limited, and never on quotes or clips.
mod common;

use std::time::{Duration, Instant};

use common::*;
use futures::SinkExt;
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{Channel, ClipUpload, EmbedProvider, MediaInfo, Message};
use pulse_server::config::{Config, EmbedConfig};
use pulse_server::limits::{LimitsConfig, Rate};
use pulse_server::testing::fake_provider::{FakeProvider, YOUTUBE_ID};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;

struct Env {
    app: TestApp,
    fake: FakeProvider,
    alex: UserId,
    alex_t: String,
    sam: UserId,
    sam_t: String,
    server: pulse_protocol::ids::ServerId,
    ch: ChannelId,
}

impl Env {
    /// The fake's OpenGraph page; `n` makes a distinct link to the same page.
    fn page(&self, n: u32) -> String {
        self.fake.url("site.test", &format!("/page?n={n}"))
    }

    fn requested(&self, needle: &str) -> bool {
        self.fake.requests().iter().any(|r| r.contains(needle))
    }

    async fn edit(&self, token: &str, id: MessageId, content: &str) -> reqwest::Response {
        self.app
            .http
            .patch(self.app.url(&format!("/messages/{id}")))
            .bearer_auth(token)
            .json(&json!({ "content": content }))
            .send()
            .await
            .unwrap()
    }

    async fn hide(&self, token: &str, id: MessageId, url: Option<&str>) -> reqwest::Response {
        post_json(
            &self.app,
            token,
            &format!("/messages/{id}/embeds/hide"),
            json!({ "url": url }),
        )
        .await
    }

    /// `id` as `token` reads it in `ch`'s list.
    async fn read(&self, token: &str, ch: ChannelId, id: MessageId) -> Message {
        let page: Vec<Message> = get_json(
            &self.app,
            token,
            &format!("/channels/{ch}/messages?around={id}"),
        )
        .await;
        page.into_iter().find(|m| m.id == id).expect("message")
    }

    /// The urls of `id`'s stored previews (hidden ones too), in link order.
    async fn rows(&self, id: MessageId) -> Vec<(String, bool)> {
        sqlx::query_as("SELECT url, hidden FROM link_embeds WHERE message_id = ? ORDER BY position")
            .bind(id.to_string())
            .fetch_all(&self.app.db)
            .await
            .unwrap()
    }
}

const YOUTUBE: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";

async fn setup() -> Env {
    setup_with(LimitsConfig::unlimited()).await
}

/// alex and sam in a server, on an app whose unfurler reads the fake provider.
async fn setup_with(limits: LimitsConfig) -> Env {
    setup_full(limits, |c| c).await
}

/// [`setup_with`], with the fake's `EmbedConfig` changed by `embeds`.
async fn setup_full(limits: LimitsConfig, embeds: impl FnOnce(EmbedConfig) -> EmbedConfig) -> Env {
    let fake = FakeProvider::start().await;
    let app = spawn_with(Config {
        embeds: embeds(fake.config()),
        limits,
        ..test_config()
    })
    .await;
    let (alex, alex_t) = register(&app, "alex").await;
    let (sam, sam_t) = register(&app, "sam").await;
    let server = create_server(&app, &alex_t, "Main").await.id;
    let r = post_json(&app, &sam_t, &format!("/servers/{server}/join"), json!({})).await;
    assert_eq!(r.status(), 204);
    let ch = general(&app, &alex_t, server).await.id;
    Env {
        app,
        fake,
        alex,
        alex_t,
        sam,
        sam_t,
        server,
        ch,
    }
}

/// `id` once it has `n` previews, as `token` reads it in `ch` (polls up to 5 s).
async fn wait_for_embeds(
    env: &Env,
    token: &str,
    ch: ChannelId,
    id: MessageId,
    n: usize,
) -> Message {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let m = env.read(token, ch, id).await;
        if m.embeds.len() == n {
            return m;
        }
        assert!(
            Instant::now() < deadline,
            "{id} has {} previews, waited for {n}: {:?}",
            m.embeds.len(),
            m.embeds
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Wait until `cond` holds (up to 5 s).
async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn urls(m: &Message) -> Vec<&str> {
    m.embeds.iter().map(|e| e.url.as_str()).collect()
}

async fn hello(app: &TestApp, token: &str) -> Socket {
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
            Some(ServerFrame::Ready(_)) => return ws,
            Some(_) => continue,
            None => panic!("expected Ready"),
        }
    }
}

/// The next `MessageUpdated` for `id` on `ws` (up to 5 s).
async fn next_update(ws: &mut Socket, id: MessageId) -> Message {
    loop {
        match next_frame_beating(ws, Duration::from_secs(5))
            .await
            .expect("a MessageUpdated")
        {
            ServerFrame::Event(Event::MessageUpdated { message }) if message.id == id => {
                return message;
            }
            _ => continue,
        }
    }
}

#[tokio::test]
async fn a_sent_link_gets_a_card_over_the_gateway() {
    let env = setup().await;
    let mut ws = hello(&env.app, &env.sam_t).await;
    let page = env.page(1);
    let sent = send(&env.app, &env.alex_t, env.ch, &format!("look at {page}!")).await;
    assert!(
        sent.embeds.is_empty(),
        "the card follows, it isn't waited for"
    );

    let mut created = false;
    loop {
        match next_frame_beating(&mut ws, Duration::from_secs(5))
            .await
            .expect("MessageUpdated with the card")
        {
            ServerFrame::Event(Event::MessageCreated { message, .. }) if message.id == sent.id => {
                assert!(message.embeds.is_empty());
                created = true;
            }
            ServerFrame::Event(Event::MessageUpdated { message }) if message.id == sent.id => {
                assert!(created, "MessageCreated comes first");
                assert_eq!(urls(&message), vec![page.as_str()]);
                let e = &message.embeds[0];
                assert_eq!(e.provider, EmbedProvider::Generic);
                assert_eq!(e.title.as_deref(), Some("A page & more"));
                assert!(e.thumb.is_some());
                assert_eq!(message.content, format!("look at {page}!"));
                break;
            }
            _ => continue,
        }
    }
}

#[tokio::test]
async fn the_first_three_distinct_links_only() {
    let env = setup().await;
    let (one, two, three) = (env.page(1), env.page(2), env.page(3));
    let content = format!("{one} {one}#again {YOUTUBE} `{three}` {two} {three}");
    let m = send(&env.app, &env.alex_t, env.ch, &content).await;
    let m = wait_for_embeds(&env, &env.alex_t, env.ch, m.id, 3).await;
    assert_eq!(urls(&m), vec![one.as_str(), YOUTUBE, two.as_str()]);
    assert_eq!(m.embeds[1].provider, EmbedProvider::Youtube);
    assert_eq!(m.embeds[1].provider_id.as_deref(), Some(YOUTUBE_ID));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(env.rows(m.id).await.len(), 3);
    assert!(!env.requested("n=3"), "the fourth link isn't fetched");
}

#[tokio::test]
async fn a_private_ip_link_gets_no_card_and_the_message_is_unchanged() {
    let env = setup().await;
    let page = env.page(1);
    let content = format!("http://10.0.0.7/page then {page}");
    let m = send(&env.app, &env.alex_t, env.ch, &content).await;
    let got = wait_for_embeds(&env, &env.alex_t, env.ch, m.id, 1).await;
    assert_eq!(urls(&got), vec![page.as_str()]);
    assert_eq!(got.content, content);
    assert_eq!(got.edited_at, None);
    let rows = env.rows(m.id).await;
    assert_eq!(rows, vec![(page.clone(), false)]);

    // Only a private link: the message stays as it was, with no card.
    let lone = send(&env.app, &env.alex_t, env.ch, "http://192.168.1.20/admin").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let lone = env.read(&env.alex_t, env.ch, lone.id).await;
    assert!(lone.embeds.is_empty());
    assert_eq!(lone.content, "http://192.168.1.20/admin");
    assert!(env.rows(lone.id).await.is_empty());
}

#[tokio::test]
async fn editing_adds_new_cards_and_drops_removed_ones() {
    let env = setup().await;
    let (one, two) = (env.page(1), env.page(2));
    // Listening from before the send, so the cards' own update is seen go by first.
    let mut ws = hello(&env.app, &env.sam_t).await;
    let m = send(
        &env.app,
        &env.alex_t,
        env.ch,
        &format!("{one} and {YOUTUBE}"),
    )
    .await;
    assert_eq!(next_update(&mut ws, m.id).await.embeds.len(), 2);

    let r = env
        .edit(&env.alex_t, m.id, &format!("{YOUTUBE} and now {two}"))
        .await;
    assert_eq!(r.status(), 200);
    let edited: Message = r.json().await.unwrap();
    // The removed link's card is gone in the edit's answer already; the new one follows.
    assert_eq!(urls(&edited), vec![YOUTUBE]);
    loop {
        match next_frame_beating(&mut ws, Duration::from_secs(5))
            .await
            .expect("the edit's MessageUpdated")
        {
            ServerFrame::Event(Event::MessageUpdated { message }) if message.id == m.id => {
                assert_eq!(urls(&message), vec![YOUTUBE]);
                break;
            }
            _ => continue,
        }
    }

    let got = wait_for_embeds(&env, &env.sam_t, env.ch, m.id, 2).await;
    assert_eq!(urls(&got), vec![YOUTUBE, two.as_str()]);
    assert!(
        env.rows(m.id).await.iter().all(|(u, _)| *u != one),
        "the removed link's row is deleted"
    );

    // Editing every link away leaves no cards.
    let r = env.edit(&env.alex_t, m.id, "no links any more").await;
    let edited: Message = r.json().await.unwrap();
    assert!(edited.embeds.is_empty());
    assert!(env.rows(m.id).await.is_empty());
}

#[tokio::test]
async fn an_edit_during_the_fetch_drops_the_stale_card() {
    let env = setup().await;
    let held = env.fake.url("site.test", "/held");
    let page = env.page(1);
    let m = send(&env.app, &env.alex_t, env.ch, &format!("wait for {held}")).await;
    until("the held fetch", || env.requested("/held")).await;

    let r = env
        .edit(&env.alex_t, m.id, &format!("never mind, {page}"))
        .await;
    assert_eq!(r.status(), 200);
    let got = wait_for_embeds(&env, &env.alex_t, env.ch, m.id, 1).await;
    assert_eq!(urls(&got), vec![page.as_str()]);

    let thumbs = || {
        env.fake
            .requests()
            .iter()
            .filter(|r| r.starts_with("/thumb.png"))
            .count()
    };
    let before = thumbs();
    env.fake.release();
    // The held page answers and its thumbnail is fetched; give its store the chance to (wrongly) land.
    until("the held page's thumbnail", || thumbs() > before).await;
    tokio::time::sleep(Duration::from_millis(400)).await;

    assert_eq!(env.rows(m.id).await, vec![(page.clone(), false)]);
    let got = env.read(&env.alex_t, env.ch, m.id).await;
    assert_eq!(urls(&got), vec![page.as_str()]);
}

#[tokio::test]
async fn hidden_stays_hidden_through_edits_and_for_everyone() {
    let env = setup().await;
    let (one, two) = (env.page(1), env.page(2));
    let mut ws = hello(&env.app, &env.sam_t).await;
    let m = send(&env.app, &env.alex_t, env.ch, &format!("{one} {YOUTUBE}")).await;
    assert_eq!(next_update(&mut ws, m.id).await.embeds.len(), 2);

    // Matched after normalising (the fragment isn't part of the link).
    let r = env
        .hide(&env.alex_t, m.id, Some(&format!("{one}#top")))
        .await;
    assert_eq!(r.status(), 200);
    let hidden: Message = r.json().await.unwrap();
    assert_eq!(urls(&hidden), vec![YOUTUBE]);
    loop {
        match next_frame_beating(&mut ws, Duration::from_secs(5))
            .await
            .expect("the hide's MessageUpdated")
        {
            ServerFrame::Event(Event::MessageUpdated { message }) if message.id == m.id => {
                assert_eq!(urls(&message), vec![YOUTUBE]);
                break;
            }
            _ => continue,
        }
    }
    assert_eq!(
        urls(&env.read(&env.sam_t, env.ch, m.id).await),
        vec![YOUTUBE]
    );

    // An edit that keeps the hidden link (and adds one) doesn't bring it back.
    let r = env
        .edit(&env.alex_t, m.id, &format!("{one} {YOUTUBE} {two}"))
        .await;
    assert_eq!(r.status(), 200);
    let got = wait_for_embeds(&env, &env.sam_t, env.ch, m.id, 2).await;
    assert_eq!(urls(&got), vec![YOUTUBE, two.as_str()]);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(env.rows(m.id).await.contains(&(one.clone(), true)));
    // Nor does one that drops it and puts it back.
    env.edit(&env.alex_t, m.id, &format!("{YOUTUBE} {two}"))
        .await;
    env.edit(&env.alex_t, m.id, &format!("{YOUTUBE} {two} {one}"))
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let got = env.read(&env.sam_t, env.ch, m.id).await;
    assert_eq!(urls(&got), vec![YOUTUBE, two.as_str()]);

    // No url hides them all; a link the message has no preview for is a 404.
    let r = env.hide(&env.alex_t, m.id, Some(&env.page(9))).await;
    assert_eq!(r.status(), 404);
    let r = env.hide(&env.alex_t, m.id, None).await;
    assert_eq!(r.status(), 200);
    assert!(r.json::<Message>().await.unwrap().embeds.is_empty());
    assert!(env.read(&env.sam_t, env.ch, m.id).await.embeds.is_empty());
}

#[tokio::test]
async fn only_the_sender_can_hide() {
    let env = setup().await;
    let page = env.page(1);
    let m = send(&env.app, &env.alex_t, env.ch, &page).await;
    wait_for_embeds(&env, &env.alex_t, env.ch, m.id, 1).await;

    let r = env.hide(&env.sam_t, m.id, Some(&page)).await;
    assert_eq!(r.status(), 403);
    let r = env.hide(&env.alex_t, MessageId::new(), Some(&page)).await;
    assert_eq!(r.status(), 404);
    assert_eq!(env.rows(m.id).await, vec![(page.clone(), false)]);
    assert_eq!(
        urls(&env.read(&env.sam_t, env.ch, m.id).await),
        vec![page.as_str()]
    );

    // In a DM: sam (in it) gets 403, kim (outside it) can't tell the message exists.
    let dm = create_dm(&env.app, &env.alex_t, &[env.sam]).await;
    let m = send(&env.app, &env.alex_t, dm.id, &page).await;
    wait_for_embeds(&env, &env.alex_t, dm.id, m.id, 1).await;
    let (_, kim_t) = register(&env.app, "kim").await;
    assert_eq!(env.hide(&kim_t, m.id, Some(&page)).await.status(), 404);
    assert_eq!(env.hide(&env.sam_t, m.id, Some(&page)).await.status(), 403);
    assert_eq!(env.rows(m.id).await, vec![(page.clone(), false)]);
}

#[tokio::test]
async fn over_the_unfurl_limit_the_message_posts_without_cards() {
    let env = setup_with(LimitsConfig {
        unfurl_user: Rate::new(1, 0.0001),
        ..LimitsConfig::unlimited()
    })
    .await;
    // A message without links spends nothing.
    send(&env.app, &env.alex_t, env.ch, "hello").await;
    let first = send(&env.app, &env.alex_t, env.ch, &env.page(1)).await;
    wait_for_embeds(&env, &env.alex_t, env.ch, first.id, 1).await;

    let r = post_json(
        &env.app,
        &env.alex_t,
        &format!("/channels/{}/messages", env.ch),
        json!({ "content": env.page(2) }),
    )
    .await;
    assert_eq!(r.status(), 200, "the message still posts");
    let second: Message = r.json().await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        env.read(&env.alex_t, env.ch, second.id)
            .await
            .embeds
            .is_empty()
    );
    assert!(env.rows(second.id).await.is_empty());
    assert!(!env.requested("n=2"), "nothing was fetched for it");
}

async fn create_channel(env: &Env, kind: &str) -> Channel {
    let r = post_json(
        &env.app,
        &env.alex_t,
        &format!("/servers/{}/channels", env.server),
        json!({ "kind": kind, "name": kind }),
    )
    .await;
    assert_eq!(r.status(), 200, "create {kind} channel");
    r.json().await.unwrap()
}

/// Ticket, PUT, thumbnail, confirm: one clip captioned `caption`.
async fn post_clip(env: &Env, ch: ChannelId, caption: &str) -> Message {
    let app = &env.app;
    let bytes = vec![7u8; 1_000];
    let r = post_json(
        app,
        &env.alex_t,
        &format!("/channels/{ch}/clips/uploads"),
        json!({ "size": bytes.len(), "duration_ms": 30_000 }),
    )
    .await;
    assert_eq!(r.status(), 200, "ticket");
    let up: ClipUpload = r.json().await.unwrap();
    let r = app
        .http
        .put(&up.url)
        .header("content-type", &up.content_type)
        .body(bytes)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "PUT {}", r.status());
    let r = app
        .http
        .post(app.url("/media/clip-thumb"))
        .bearer_auth(&env.alex_t)
        .header("content-type", "application/octet-stream")
        .body(pulse_server::testing::fake_provider::png(64, 36))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "thumb upload");
    let thumb = r.json::<MediaInfo>().await.unwrap().hash;
    let r = post_json(
        app,
        &env.alex_t,
        &format!("/clip-uploads/{}/confirm", up.id),
        json!({
            "caption": caption, "game_id": null, "duration_ms": 30_000,
            "width": 1920, "height": 1080, "thumb_hash": thumb, "nonce": null,
        }),
    )
    .await;
    assert_eq!(r.status(), 200, "confirm");
    r.json().await.unwrap()
}

#[tokio::test]
async fn quotes_and_clips_get_no_previews() {
    let env = setup().await;
    let quotes = create_channel(&env, "quotes").await;
    let r = post_json(
        &env.app,
        &env.alex_t,
        &format!("/channels/{}/quotes", quotes.id),
        json!({ "lines": [{ "speaker_id": env.alex, "text": format!("read {}", env.page(1)) }] }),
    )
    .await;
    assert_eq!(r.status(), 200, "post quote");
    let quote: Message = r.json().await.unwrap();

    let clips = create_channel(&env, "clips").await;
    let clip = post_clip(&env, clips.id, &format!("see {}", env.page(2))).await;

    // A reply in the quote's thread is a plain message: it gets its preview.
    let r = post_json(
        &env.app,
        &env.alex_t,
        &format!("/channels/{}/messages", quotes.id),
        json!({ "content": format!("so true {}", env.page(3)), "thread_root_id": quote.id }),
    )
    .await;
    assert_eq!(r.status(), 200, "thread reply");
    let reply: Message = r.json().await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while env.rows(reply.id).await.is_empty() {
        assert!(Instant::now() < deadline, "the thread reply got no preview");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(env.rows(quote.id).await.is_empty());
    assert!(env.rows(clip.id).await.is_empty());
    assert!(!env.requested("n=1") && !env.requested("n=2"));
}

/// How many times the fake served its thumbnail.
fn thumb_hits(env: &Env) -> usize {
    env.fake
        .requests()
        .iter()
        .filter(|r| r.starts_with("/thumb.png"))
        .count()
}

/// Edits that keep the same links have nothing to fetch, so they spend nothing: a later edit
/// that adds a link still gets its preview under a small limit.
#[tokio::test]
async fn edits_that_keep_the_links_spend_no_unfurl_budget() {
    let env = setup_with(LimitsConfig {
        unfurl_user: Rate::new(2, 0.0001),
        ..LimitsConfig::unlimited()
    })
    .await;
    let (one, two) = (env.page(1), env.page(2));
    let m = send(&env.app, &env.alex_t, env.ch, &format!("see {one}")).await;
    wait_for_embeds(&env, &env.alex_t, env.ch, m.id, 1).await;
    for text in ["sea", "see,", "see:", "look at"] {
        let r = env.edit(&env.alex_t, m.id, &format!("{text} {one}")).await;
        assert_eq!(r.status(), 200);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let r = env
        .edit(&env.alex_t, m.id, &format!("look at {one} and {two}"))
        .await;
    assert_eq!(r.status(), 200);
    let got = wait_for_embeds(&env, &env.alex_t, env.ch, m.id, 2).await;
    assert_eq!(urls(&got), vec![one.as_str(), two.as_str()]);
}

/// A link whose preview takes too long is given up on, and its slot goes to the next one long
/// before the fetch itself would time out.
#[tokio::test]
async fn a_preview_past_its_deadline_frees_its_slot() {
    // Long enough for a whole preview (thumbnail processing included, in a debug build), far
    // shorter than one fetch may take.
    let env = setup_full(LimitsConfig::unlimited(), |c| EmbedConfig {
        link_deadline: Duration::from_millis(1500),
        timeout: Duration::from_secs(30),
        max_concurrent: 1,
        ..c
    })
    .await;
    let held = env.fake.url("site.test", "/held");
    let stuck = send(&env.app, &env.alex_t, env.ch, &held).await;
    until("the held fetch", || env.requested("/held")).await;
    let t = Instant::now();
    send(&env.app, &env.sam_t, env.ch, &env.page(1)).await;
    // The slot is free once the next link's fetch starts. Not "once its card is stored": that
    // card has the same short deadline, which thumbnail work on a busy CI runner can overrun.
    until("the next link's fetch", || env.requested("/page?n=1")).await;
    assert!(
        t.elapsed() < Duration::from_secs(4),
        "waited {:?} for the slot",
        t.elapsed()
    );
    assert!(
        t.elapsed() >= Duration::from_millis(1000),
        "the next fetch started at {:?}, before the stuck one's deadline",
        t.elapsed()
    );
    assert!(env.rows(stuck.id).await.is_empty());
}

/// One user's previews are fetched for at most two messages at once; a third message gets
/// none, and nobody else is held up.
#[tokio::test]
async fn one_user_has_at_most_two_unfurls_in_flight() {
    let env = setup().await;
    let held = |n: u32| env.fake.url("site.test", &format!("/held?n={n}"));
    let a = send(&env.app, &env.alex_t, env.ch, &held(1)).await;
    let b = send(&env.app, &env.alex_t, env.ch, &held(2)).await;
    until("both held fetches", || {
        env.requested("/held?n=1") && env.requested("/held?n=2")
    })
    .await;
    let dropped = send(&env.app, &env.alex_t, env.ch, &env.page(3)).await;
    // sam isn't affected.
    let other = send(&env.app, &env.sam_t, env.ch, &env.page(4)).await;
    wait_for_embeds(&env, &env.sam_t, env.ch, other.id, 1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!env.requested("n=3"), "the third message isn't fetched");

    env.fake.release();
    wait_for_embeds(&env, &env.alex_t, env.ch, a.id, 1).await;
    wait_for_embeds(&env, &env.alex_t, env.ch, b.id, 1).await;
    assert!(env.rows(dropped.id).await.is_empty());
    // Done, alex's next message gets its preview again.
    let next = send(&env.app, &env.alex_t, env.ch, &env.page(5)).await;
    wait_for_embeds(&env, &env.alex_t, env.ch, next.id, 1).await;
}

/// When every slot is busy, only so many links wait for one; past that a link gets no preview
/// (and isn't remembered as having none: sent again later, it gets one).
#[tokio::test]
async fn the_wait_for_a_slot_is_bounded() {
    let env = setup_full(LimitsConfig::unlimited(), |c| EmbedConfig {
        max_concurrent: 1,
        max_waiting: 1,
        ..c
    })
    .await;
    let held = |n: u32| env.fake.url("site.test", &format!("/held?n={n}"));
    let a = send(&env.app, &env.alex_t, env.ch, &held(1)).await;
    until("the held fetch", || env.requested("/held?n=1")).await;
    let b = send(&env.app, &env.sam_t, env.ch, &held(2)).await;
    // b is waiting for the slot: nothing to see from outside but time.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let dropped = send(&env.app, &env.alex_t, env.ch, &env.page(3)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    env.fake.release();
    wait_for_embeds(&env, &env.alex_t, env.ch, a.id, 1).await;
    wait_for_embeds(&env, &env.sam_t, env.ch, b.id, 1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(env.rows(dropped.id).await.is_empty());
    assert!(!env.requested("n=3"), "the dropped link isn't fetched");

    let again = send(&env.app, &env.alex_t, env.ch, &env.page(3)).await;
    wait_for_embeds(&env, &env.alex_t, env.ch, again.id, 1).await;
}

/// Thumbnails cost disk: over the per-user limit, previews still come, without one.
#[tokio::test]
async fn over_the_thumbnail_limit_a_card_comes_without_one() {
    let env = setup_with(LimitsConfig {
        link_thumb_user: Rate::new(1, 0.0001),
        ..LimitsConfig::unlimited()
    })
    .await;
    let first = send(&env.app, &env.alex_t, env.ch, &env.page(1)).await;
    let got = wait_for_embeds(&env, &env.alex_t, env.ch, first.id, 1).await;
    assert!(got.embeds[0].thumb.is_some());
    assert_eq!(thumb_hits(&env), 1);

    let second = send(&env.app, &env.alex_t, env.ch, &env.page(2)).await;
    let got = wait_for_embeds(&env, &env.alex_t, env.ch, second.id, 1).await;
    let e = &got.embeds[0];
    assert_eq!(e.title.as_deref(), Some("A page & more"));
    assert!(e.thumb.is_none(), "{e:?}");
    assert_eq!(thumb_hits(&env), 1, "the thumbnail isn't fetched");

    // sam has his own allowance.
    let his = send(&env.app, &env.sam_t, env.ch, &env.page(3)).await;
    let got = wait_for_embeds(&env, &env.sam_t, env.ch, his.id, 1).await;
    assert!(got.embeds[0].thumb.is_some());
}

/// A message deleted while its previews are fetched gets none: no rows, and no update with
/// previews goes out for it.
#[tokio::test]
async fn a_delete_during_the_fetch_stores_nothing() {
    let env = setup().await;
    let held = env.fake.url("site.test", "/held");
    let m = send(&env.app, &env.alex_t, env.ch, &format!("wait for {held}")).await;
    until("the held fetch", || env.requested("/held")).await;
    let mut ws = hello(&env.app, &env.sam_t).await;
    let r = env
        .app
        .http
        .delete(env.app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(&env.alex_t)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);

    let before = thumb_hits(&env);
    env.fake.release();
    until("the held page's thumbnail", || thumb_hits(&env) > before).await;
    let deadline = Instant::now() + Duration::from_millis(800);
    while Instant::now() < deadline {
        if let Some(ServerFrame::Event(Event::MessageUpdated { message })) =
            next_frame_beating(&mut ws, Duration::from_millis(200)).await
        {
            assert!(
                message.id != m.id || message.embeds.is_empty(),
                "previews published for a deleted message: {message:?}"
            );
        }
    }
    assert!(env.rows(m.id).await.is_empty());
}

/// Hiding a preview while another link of the same message is being fetched: the fetch's store
/// lands and the hidden one stays hidden.
#[tokio::test]
async fn a_hide_during_a_fetch_survives_its_store() {
    let env = setup().await;
    let page = env.page(1);
    let held = env.fake.url("site.test", "/held");
    let m = send(&env.app, &env.alex_t, env.ch, &page).await;
    wait_for_embeds(&env, &env.alex_t, env.ch, m.id, 1).await;
    let r = env
        .edit(&env.alex_t, m.id, &format!("{page} and {held}"))
        .await;
    assert_eq!(r.status(), 200);
    until("the held fetch", || env.requested("/held")).await;

    let r = env.hide(&env.alex_t, m.id, Some(&page)).await;
    assert_eq!(r.status(), 200);
    let mut ws = hello(&env.app, &env.sam_t).await;
    env.fake.release();
    loop {
        match next_frame_beating(&mut ws, Duration::from_secs(5))
            .await
            .expect("the store's MessageUpdated")
        {
            ServerFrame::Event(Event::MessageUpdated { message }) if message.id == m.id => {
                assert_eq!(urls(&message), vec![held.as_str()]);
                break;
            }
            _ => continue,
        }
    }
    assert_eq!(
        env.rows(m.id).await,
        vec![(page.clone(), true), (held.clone(), false)]
    );
    assert_eq!(
        urls(&env.read(&env.sam_t, env.ch, m.id).await),
        vec![held.as_str()]
    );
}
