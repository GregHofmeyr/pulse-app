//! Badges (#49) from chat: messages, reactions, custom emoji and clips. Server activity only.
mod common;

use std::io::Cursor;
use std::time::Duration;

use common::*;
use futures::SinkExt;
use image::{ImageFormat, Rgba, RgbaImage};
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{Badge, Channel, ClipUpload, CustomEmoji, MediaInfo, Message, Profile};
use pulse_server::badges::{catalogue::stat, stats};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;

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
    match next_frame_beating(&mut ws, Duration::from_secs(3)).await {
        Some(ServerFrame::Ready(_)) => ws,
        other => panic!("expected Ready, got {other:?}"),
    }
}

/// The keys of the `BadgeEarned` events that arrive before the socket goes quiet.
async fn earned(ws: &mut Socket) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(f) = next_frame_beating(ws, Duration::from_millis(300)).await {
        if let ServerFrame::Event(Event::BadgeEarned { badge, .. }) = f {
            out.push(badge.key);
        }
    }
    out
}

async fn stat_of(app: &TestApp, user: UserId, key: &str) -> i64 {
    let mut c = app.db.acquire().await.unwrap();
    stats::get(&mut c, user, key).await.unwrap()
}

async fn set_stat(app: &TestApp, user: UserId, key: &str, value: i64) {
    let mut c = app.db.acquire().await.unwrap();
    stats::set(&mut c, user, key, value).await.unwrap();
}

async fn badges_of(app: &TestApp, token: &str, user: UserId) -> Vec<Badge> {
    get_json::<Profile>(app, token, &format!("/users/{user}/profile"))
        .await
        .badges
}

async fn holds(app: &TestApp, token: &str, user: UserId, key: &str) -> bool {
    badges_of(app, token, user)
        .await
        .iter()
        .any(|b| b.key == key)
}

async fn join(app: &TestApp, token: &str, server: ServerId) {
    let r = post_json(app, token, &format!("/servers/{server}/join"), json!({})).await;
    assert!(r.status().is_success(), "join {}", r.status());
}

async fn react(app: &TestApp, token: &str, msg: MessageId, emoji: &str, add: bool) {
    let url = app.url(&format!("/messages/{msg}/reactions/{emoji}"));
    let r = if add {
        app.http.put(url)
    } else {
        app.http.delete(url)
    };
    let status = r.bearer_auth(token).send().await.unwrap().status();
    assert_eq!(status, 204, "react {emoji} add={add}");
}

async fn create_channel(app: &TestApp, token: &str, server: ServerId, kind: &str) -> Channel {
    let r = post_json(
        app,
        token,
        &format!("/servers/{server}/channels"),
        json!({ "kind": kind, "name": kind }),
    )
    .await;
    assert_eq!(r.status(), 200, "create {kind} channel");
    r.json().await.unwrap()
}

fn png() -> Vec<u8> {
    let img = RgbaImage::from_pixel(64, 32, Rgba([10, 120, 240, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

async fn custom_emoji(app: &TestApp, token: &str, server: ServerId, name: &str) -> CustomEmoji {
    let r = app
        .http
        .post(app.url("/media/emoji"))
        .bearer_auth(token)
        .body(png())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let media: MediaInfo = r.json().await.unwrap();
    let r = post_json(
        app,
        token,
        &format!("/servers/{server}/emoji"),
        json!({ "name": name, "media_hash": media.hash }),
    )
    .await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

/// Ticket, PUT, thumbnail, confirm: one posted clip (as in `clips.rs`).
async fn post_clip(app: &TestApp, token: &str, channel: ChannelId) -> Message {
    let bytes = vec![5u8; 10];
    let r = post_json(
        app,
        token,
        &format!("/channels/{channel}/clips/uploads"),
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
        .bearer_auth(token)
        .header("content-type", "application/octet-stream")
        .body(png())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "thumb upload");
    let thumb = r.json::<MediaInfo>().await.unwrap().hash;
    let r = post_json(
        app,
        token,
        &format!("/clip-uploads/{}/confirm", up.id),
        json!({
            "caption": "nice one",
            "game_id": null,
            "duration_ms": 30_000,
            "width": 1920,
            "height": 1080,
            "thumb_hash": thumb,
            "nonce": null,
        }),
    )
    .await;
    assert_eq!(r.status(), 200, "confirm");
    r.json().await.unwrap()
}

/// alex's server "Main" with sam in it; alex's and sam's ids and tokens.
struct World {
    app: TestApp,
    alex: (UserId, String),
    sam: (UserId, String),
    server: ServerId,
    general: ChannelId,
}

async fn world() -> World {
    let app = spawn().await;
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let s = create_server(&app, &alex.1, "Main").await;
    join(&app, &sam.1, s.id).await;
    let general = general(&app, &alex.1, s.id).await.id;
    World {
        app,
        alex,
        sam,
        server: s.id,
        general,
    }
}

#[tokio::test]
async fn server_messages_count_dm_and_group_never_do() {
    let w = world().await;
    let kim = register(&w.app, "kim").await;
    let dm = create_dm(&w.app, &w.alex.1, &[w.sam.0]).await;
    let group = create_dm(&w.app, &w.alex.1, &[w.sam.0, kim.0]).await;
    for i in 0..3 {
        send(&w.app, &w.alex.1, w.general, &format!("hi {i}")).await;
    }
    for i in 0..5 {
        send(&w.app, &w.alex.1, dm.id, &format!("dm {i}")).await;
        send(&w.app, &w.alex.1, group.id, &format!("group {i}")).await;
    }
    assert_eq!(stat_of(&w.app, w.alex.0, stat::MESSAGES).await, 3);
    assert_eq!(stat_of(&w.app, w.sam.0, stat::MESSAGES).await, 0);
}

#[tokio::test]
async fn reactions_received_count_and_toggling_cannot_farm() {
    let w = world().await;
    let msg = send(&w.app, &w.alex.1, w.general, "react to me").await;
    react(&w.app, &w.sam.1, msg.id, "👍", true).await;
    react(&w.app, &w.sam.1, msg.id, "👍", false).await;
    react(&w.app, &w.sam.1, msg.id, "👍", true).await;
    // A repeat of an add or a remove changes nothing.
    react(&w.app, &w.sam.1, msg.id, "👍", true).await;
    assert_eq!(stat_of(&w.app, w.alex.0, stat::REACTIONS_RECEIVED).await, 1);

    // Your own reactions don't count.
    react(&w.app, &w.alex.1, msg.id, "🎉", true).await;
    assert_eq!(stat_of(&w.app, w.alex.0, stat::REACTIONS_RECEIVED).await, 1);

    // Nor do reactions in a DM.
    let dm = create_dm(&w.app, &w.alex.1, &[w.sam.0]).await;
    let private = send(&w.app, &w.alex.1, dm.id, "psst").await;
    react(&w.app, &w.sam.1, private.id, "👍", true).await;
    assert_eq!(stat_of(&w.app, w.alex.0, stat::REACTIONS_RECEIVED).await, 1);
    assert_eq!(stat_of(&w.app, w.sam.0, stat::REACTIONS_RECEIVED).await, 0);
}

#[tokio::test]
async fn one_per_person_swaps_cannot_farm() {
    let w = world().await;
    let ch = create_channel(&w.app, &w.alex.1, w.server, "text").await;
    sqlx::query("UPDATE channels SET reaction_mode = 'one_per_person' WHERE id = ?")
        .bind(ch.id.to_string())
        .execute(&w.app.db)
        .await
        .unwrap();
    let msg = send(&w.app, &w.alex.1, ch.id, "pick one").await;
    // Each add replaces sam's previous reaction: one reaction from sam at any time.
    for emoji in ["👍", "😂", "🎉", "👍"] {
        react(&w.app, &w.sam.1, msg.id, emoji, true).await;
    }
    assert_eq!(stat_of(&w.app, w.alex.0, stat::REACTIONS_RECEIVED).await, 1);
}

#[tokio::test]
async fn counters_survive_channel_purge() {
    let w = world().await;
    let old = create_channel(&w.app, &w.alex.1, w.server, "text").await;
    let clips = create_channel(&w.app, &w.alex.1, w.server, "clips").await;
    set_stat(&w.app, w.alex.0, stat::MESSAGES, 98).await;
    let msg = send(&w.app, &w.alex.1, old.id, "one").await;
    send(&w.app, &w.alex.1, old.id, "two").await;
    react(&w.app, &w.sam.1, msg.id, "👍", true).await;
    post_clip(&w.app, &w.alex.1, clips.id).await;
    assert!(holds(&w.app, &w.alex.1, w.alex.0, "the_yapper").await);

    for ch in [old.id, clips.id] {
        let r = w
            .app
            .http
            .delete(w.app.url(&format!("/channels/{ch}")))
            .bearer_auth(&w.alex.1)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 204, "delete");
        let r = w
            .app
            .http
            .delete(w.app.url(&format!("/channels/{ch}/permanent")))
            .bearer_auth(&w.alex.1)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 204, "purge");
    }
    let gone: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE channel_id IN (?, ?)")
        .bind(old.id.to_string())
        .bind(clips.id.to_string())
        .fetch_one(&w.app.db)
        .await
        .unwrap();
    assert_eq!(gone, 0, "purged");
    assert_eq!(stat_of(&w.app, w.alex.0, stat::MESSAGES).await, 100);
    assert_eq!(stat_of(&w.app, w.alex.0, stat::REACTIONS_RECEIVED).await, 1);
    assert_eq!(stat_of(&w.app, w.alex.0, stat::CLIPS).await, 1);
    assert!(holds(&w.app, &w.alex.1, w.alex.0, "the_yapper").await);
}

#[tokio::test]
async fn yapper_unlocks_from_the_hundredth_message() {
    let w = world().await;
    let mut ws = hello(&w.app, &w.alex.1).await;
    earned(&mut ws).await;
    set_stat(&w.app, w.alex.0, stat::MESSAGES, 98).await;
    send(&w.app, &w.alex.1, w.general, "ninety-nine").await;
    assert!(earned(&mut ws).await.is_empty());
    send(&w.app, &w.alex.1, w.general, "a hundred").await;
    assert_eq!(earned(&mut ws).await, vec!["the_yapper".to_string()]);
}

#[tokio::test]
async fn emoji_smith_counts_emoji_others_used_ten_times() {
    let w = world().await;
    let mut emoji = Vec::new();
    for i in 0..5 {
        emoji.push(custom_emoji(&w.app, &w.alex.1, w.server, &format!("e{i}")).await);
    }
    let tokens: String = emoji
        .iter()
        .map(|e| format!("<:{}:{}> ", e.name, e.id))
        .collect();
    let uses = |e: &CustomEmoji| format!("{}{}", stat::EMOJI_USES_PREFIX, e.id);

    // alex's own uses don't count.
    for _ in 0..10 {
        send(&w.app, &w.alex.1, w.general, &tokens).await;
    }
    assert_eq!(stat_of(&w.app, w.alex.0, &uses(&emoji[0])).await, 0);

    // Nor do sam's in a DM.
    let dm = create_dm(&w.app, &w.sam.1, &[w.alex.0]).await;
    send(&w.app, &w.sam.1, dm.id, &tokens).await;
    assert_eq!(stat_of(&w.app, w.alex.0, &uses(&emoji[0])).await, 0);

    // Twice in one message is one use; a reaction taken off is a use taken back.
    let msg = send(&w.app, &w.alex.1, w.general, "react here").await;
    let key = |e: &CustomEmoji| format!("custom:{}", e.id);
    send(&w.app, &w.sam.1, w.general, &format!("{tokens}{tokens}")).await;
    react(&w.app, &w.sam.1, msg.id, &key(&emoji[0]), true).await;
    react(&w.app, &w.sam.1, msg.id, &key(&emoji[0]), false).await;
    assert_eq!(stat_of(&w.app, w.alex.0, &uses(&emoji[0])).await, 1);

    // Eight more messages and a reaction each: ten uses of every emoji.
    for _ in 0..8 {
        send(&w.app, &w.sam.1, w.general, &tokens).await;
    }
    for e in &emoji[..4] {
        react(&w.app, &w.sam.1, msg.id, &key(e), true).await;
    }
    assert_eq!(stat_of(&w.app, w.alex.0, stat::EMOJI_SMITH).await, 4);
    assert!(!holds(&w.app, &w.alex.1, w.alex.0, "emoji_smith").await);
    react(&w.app, &w.sam.1, msg.id, &key(&emoji[4]), true).await;
    assert_eq!(stat_of(&w.app, w.alex.0, &uses(&emoji[4])).await, 10);
    assert_eq!(stat_of(&w.app, w.alex.0, stat::EMOJI_SMITH).await, 5);
    assert!(holds(&w.app, &w.alex.1, w.alex.0, "emoji_smith").await);

    // Dropping below ten takes it off the count (not the badge), so toggling around ten nets
    // nothing; going past ten doesn't count the emoji twice.
    react(&w.app, &w.sam.1, msg.id, &key(&emoji[4]), false).await;
    assert_eq!(stat_of(&w.app, w.alex.0, stat::EMOJI_SMITH).await, 4);
    assert!(holds(&w.app, &w.alex.1, w.alex.0, "emoji_smith").await);
    react(&w.app, &w.sam.1, msg.id, &key(&emoji[4]), true).await;
    send(&w.app, &w.sam.1, w.general, &tokens).await;
    assert_eq!(stat_of(&w.app, w.alex.0, stat::EMOJI_SMITH).await, 5);
}

#[tokio::test]
async fn highlight_reel_at_five_reactors() {
    let w = world().await;
    let clips = create_channel(&w.app, &w.alex.1, w.server, "clips").await;
    let clip = post_clip(&w.app, &w.alex.1, clips.id).await;
    let mut others = vec![w.sam.1.clone()];
    for name in ["kim", "jo", "riley", "kai"] {
        let (_, t) = register(&w.app, name).await;
        join(&w.app, &t, w.server).await;
        others.push(t);
    }
    // alex's own reaction and a second emoji from the same person don't make a reactor.
    react(&w.app, &w.alex.1, clip.id, "🔥", true).await;
    for t in &others[..4] {
        react(&w.app, t, clip.id, "🔥", true).await;
    }
    react(&w.app, &others[0], clip.id, "😂", true).await;
    assert!(!holds(&w.app, &w.alex.1, w.alex.0, "highlight_reel").await);
    react(&w.app, &others[4], clip.id, "🔥", true).await;
    assert!(holds(&w.app, &w.alex.1, w.alex.0, "highlight_reel").await);

    // Five reactions on a plain message are no highlight.
    let plain = send(&w.app, &w.sam.1, w.general, "not a clip").await;
    for t in [&w.alex.1, &others[1], &others[2], &others[3], &others[4]] {
        react(&w.app, t, plain.id, "🔥", true).await;
    }
    assert_eq!(stat_of(&w.app, w.sam.0, stat::HIGHLIGHT_REEL).await, 0);
}

#[tokio::test]
async fn clips_count() {
    let w = world().await;
    let clips = create_channel(&w.app, &w.alex.1, w.server, "clips").await;
    post_clip(&w.app, &w.alex.1, clips.id).await;
    post_clip(&w.app, &w.alex.1, clips.id).await;
    assert_eq!(stat_of(&w.app, w.alex.0, stat::CLIPS).await, 2);
    // A clip isn't a chat message.
    assert_eq!(stat_of(&w.app, w.alex.0, stat::MESSAGES).await, 0);
}
