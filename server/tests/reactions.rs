//! Reactions: toggling, counts, the 20-emoji cap, reaction modes, privacy and usage ranking.
mod common;

use std::io::Cursor;
use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use image::{ImageFormat, Rgba, RgbaImage};
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::{MessageId, ServerId, UserId};
use pulse_protocol::rest::{CustomEmoji, MediaInfo, Message, ReactionSummary};
use pulse_server::emoji;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

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
    match next_frame(&mut ws, Duration::from_secs(3)).await {
        Some(ServerFrame::Ready(_)) => ws,
        other => panic!("expected Ready, got {other:?}"),
    }
}

/// The next frame that isn't presence noise, or None on timeout.
async fn next_frame(ws: &mut Socket, wait: Duration) -> Option<ServerFrame> {
    loop {
        let m = tokio::time::timeout(wait, ws.next()).await.ok()??.ok()?;
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

/// The next reaction event, skipping the rest (e.g. the MessageCreated of the setup).
async fn next_reaction(ws: &mut Socket) -> Event {
    loop {
        match next_frame(ws, Duration::from_secs(3)).await {
            Some(ServerFrame::Event(
                e @ (Event::ReactionAdded { .. } | Event::ReactionRemoved { .. }),
            )) => return e,
            Some(_) => continue,
            None => panic!("no reaction event"),
        }
    }
}

fn path(msg: MessageId, emoji: &str) -> String {
    format!("/messages/{msg}/reactions/{emoji}")
}

async fn react(app: &TestApp, token: &str, msg: MessageId, emoji: &str) -> u16 {
    app.http
        .put(app.url(&path(msg, emoji)))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn unreact(app: &TestApp, token: &str, msg: MessageId, emoji: &str) -> u16 {
    app.http
        .delete(app.url(&path(msg, emoji)))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn reactions_of(app: &TestApp, token: &str, msg: &Message) -> Vec<ReactionSummary> {
    let page: Vec<Message> = get_json(
        app,
        token,
        &format!("/channels/{}/messages", msg.channel_id),
    )
    .await;
    page.into_iter().find(|m| m.id == msg.id).unwrap().reactions
}

fn summary(emoji: &str, count: u32, me: bool) -> ReactionSummary {
    ReactionSummary {
        emoji: emoji.into(),
        count,
        me,
    }
}

async fn join(app: &TestApp, token: &str, server: ServerId) {
    let r = post_json(app, token, &format!("/servers/{server}/join"), json!({})).await;
    assert!(r.status().is_success());
}

/// A server with alex (owner) and sam, and a message from alex in #general.
async fn setup(app: &TestApp) -> (UserId, String, UserId, String, ServerId, Message) {
    let (alex_id, alex) = register(app, "alex").await;
    let (sam_id, sam) = register(app, "sam").await;
    let srv = create_server(app, &alex, "Main").await;
    join(app, &sam, srv.id).await;
    let g = general(app, &alex, srv.id).await;
    let msg = send(app, &alex, g.id, "hello").await;
    (alex_id, alex, sam_id, sam, srv.id, msg)
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

#[tokio::test]
async fn add_remove_and_counts() {
    let app = spawn().await;
    let (_, alex, _, sam, _, msg) = setup(&app).await;
    assert_eq!(react(&app, &alex, msg.id, "👍").await, 204);
    assert_eq!(react(&app, &sam, msg.id, "👍").await, 204);
    assert_eq!(
        reactions_of(&app, &alex, &msg).await,
        [summary("👍", 2, true)]
    );
    assert_eq!(unreact(&app, &sam, msg.id, "👍").await, 204);
    assert_eq!(
        reactions_of(&app, &sam, &msg).await,
        [summary("👍", 1, false)]
    );
    // Idempotent both ways.
    assert_eq!(react(&app, &alex, msg.id, "👍").await, 204);
    assert_eq!(unreact(&app, &sam, msg.id, "👍").await, 204);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reactions WHERE message_id = ?")
        .bind(msg.id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn events_carry_absolute_count() {
    let app = spawn().await;
    let (alex_id, alex, sam_id, sam, _, msg) = setup(&app).await;
    let mut ws = hello(&app, &alex).await;
    react(&app, &alex, msg.id, "👍").await;
    react(&app, &sam, msg.id, "👍").await;
    unreact(&app, &alex, msg.id, "👍").await;
    let added = |user_id, count| Event::ReactionAdded {
        channel_id: msg.channel_id,
        message_id: msg.id,
        emoji: "👍".into(),
        user_id,
        count,
    };
    assert_eq!(next_reaction(&mut ws).await, added(alex_id, 1));
    assert_eq!(next_reaction(&mut ws).await, added(sam_id, 2));
    assert_eq!(
        next_reaction(&mut ws).await,
        Event::ReactionRemoved {
            channel_id: msg.channel_id,
            message_id: msg.id,
            emoji: "👍".into(),
            user_id: alex_id,
            count: 1,
        }
    );
}

#[tokio::test]
async fn dm_reactions_only_reach_members() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    let dm = create_dm(&app, &alex, &[sam_id]).await;
    let msg = send(&app, &alex, dm.id, "secret").await;
    let mut member = hello(&app, &sam).await;
    let mut outsider = hello(&app, &kim).await;
    assert_eq!(react(&app, &alex, msg.id, "👍").await, 204);
    assert!(matches!(
        next_reaction(&mut member).await,
        Event::ReactionAdded { count: 1, .. }
    ));
    assert!(
        next_frame(&mut outsider, Duration::from_millis(300))
            .await
            .is_none(),
        "an outsider heard a DM reaction"
    );
}

#[tokio::test]
async fn cannot_react_where_you_cannot_see() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, _) = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    let dm = create_dm(&app, &alex, &[sam_id]).await;
    let msg = send(&app, &alex, dm.id, "secret").await;
    assert_eq!(react(&app, &kim, msg.id, "👍").await, 404);
    assert_eq!(unreact(&app, &kim, msg.id, "👍").await, 404);
    let r = app
        .http
        .get(app.url(&path(msg.id, "👍")))
        .bearer_auth(&kim)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    // Same answer as a message that never existed.
    assert_eq!(react(&app, &kim, MessageId::new(), "👍").await, 404);

    // Server channels are visible to all, but only members take part.
    let srv = create_server(&app, &alex, "Main").await;
    let g = general(&app, &alex, srv.id).await;
    let public = send(&app, &alex, g.id, "hi").await;
    assert_eq!(react(&app, &kim, public.id, "👍").await, 403);
}

#[tokio::test]
async fn deleted_message_is_404() {
    let app = spawn().await;
    let (_, alex, _, sam, _, msg) = setup(&app).await;
    react(&app, &sam, msg.id, "👍").await;
    let r = app
        .http
        .delete(app.url(&format!("/messages/{}", msg.id)))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(react(&app, &sam, msg.id, "😂").await, 404);
    assert_eq!(unreact(&app, &sam, msg.id, "👍").await, 404);
    assert!(reactions_of(&app, &sam, &msg).await.is_empty());
}

const TWENTY: [&str; 20] = [
    "😀", "😃", "😄", "😁", "😆", "😅", "🤣", "😂", "🙂", "🙃", "😉", "😊", "😇", "🥰", "😍", "🤩",
    "😘", "😗", "😚", "😙",
];

#[tokio::test]
async fn cap_20_distinct() {
    let app = spawn().await;
    let (_, alex, _, sam, _, msg) = setup(&app).await;
    for e in TWENTY {
        assert_eq!(react(&app, &alex, msg.id, e).await, 204, "{e}");
    }
    assert_eq!(react(&app, &alex, msg.id, "😋").await, 400);
    assert_eq!(react(&app, &sam, msg.id, "😋").await, 400);
    // Joining an emoji that's already there doesn't add a new one.
    assert_eq!(react(&app, &sam, msg.id, "😀").await, 204);
    assert_eq!(reactions_of(&app, &sam, &msg).await.len(), 20);
}

#[tokio::test]
async fn unknown_emoji_rejected() {
    let app = spawn().await;
    let (_, alex, _, _, srv, msg) = setup(&app).await;
    assert_eq!(react(&app, &alex, msg.id, "abc").await, 400);
    assert_eq!(
        react(&app, &alex, msg.id, "custom:01JAAAAAAAAAAAAAAAAAAAAAAA").await,
        400
    );
    // Skin tones are kept as given.
    assert_eq!(react(&app, &alex, msg.id, "👍🏽").await, 204);

    let e = custom_emoji(&app, &alex, srv, "pog").await;
    let key = format!("custom:{}", e.id);
    assert_eq!(react(&app, &alex, msg.id, &key).await, 204);
    let r = app
        .http
        .delete(app.url(&format!("/emoji/{}", e.id)))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    let (_, kim) = register(&app, "kim").await;
    join(&app, &kim, srv).await;
    assert_eq!(react(&app, &kim, msg.id, &key).await, 400);
    assert_eq!(unreact(&app, &alex, msg.id, &key).await, 204);
    assert_eq!(
        reactions_of(&app, &alex, &msg).await,
        [summary("👍🏽", 1, true)]
    );
}

#[tokio::test]
async fn one_per_person_mode_replaces() {
    let app = spawn().await;
    let (alex_id, alex, _, sam, _, msg) = setup(&app).await;
    sqlx::query("UPDATE channels SET reaction_mode = 'one_per_person' WHERE id = ?")
        .bind(msg.channel_id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    react(&app, &sam, msg.id, "👍").await;
    let mut ws = hello(&app, &sam).await;
    assert_eq!(react(&app, &alex, msg.id, "👍").await, 204);
    assert!(matches!(
        next_reaction(&mut ws).await,
        Event::ReactionAdded { count: 2, .. }
    ));
    assert_eq!(react(&app, &alex, msg.id, "😂").await, 204);
    assert_eq!(
        next_reaction(&mut ws).await,
        Event::ReactionRemoved {
            channel_id: msg.channel_id,
            message_id: msg.id,
            emoji: "👍".into(),
            user_id: alex_id,
            count: 1,
        }
    );
    assert_eq!(
        next_reaction(&mut ws).await,
        Event::ReactionAdded {
            channel_id: msg.channel_id,
            message_id: msg.id,
            emoji: "😂".into(),
            user_id: alex_id,
            count: 1,
        }
    );
    assert_eq!(
        reactions_of(&app, &alex, &msg).await,
        [summary("👍", 1, false), summary("😂", 1, true)]
    );
}

#[tokio::test]
async fn reactors_list() {
    let app = spawn().await;
    let (alex_id, alex, sam_id, sam, _, msg) = setup(&app).await;
    react(&app, &sam, msg.id, "👍").await;
    react(&app, &alex, msg.id, "👍").await;
    let users: Vec<UserId> = get_json(&app, &alex, &path(msg.id, "👍")).await;
    assert_eq!(users, [sam_id, alex_id]);
    let none: Vec<UserId> = get_json(&app, &alex, &path(msg.id, "😂")).await;
    assert!(none.is_empty());
}

#[tokio::test]
async fn reacting_records_usage() {
    let app = spawn().await;
    let (alex_id, alex, _, _, _, msg) = setup(&app).await;
    react(&app, &alex, msg.id, "🎉").await;
    let top = emoji::frequent(&app.db, alex_id, 27).await.unwrap();
    assert!(top.contains(&"🎉".to_string()), "{top:?}");
}

#[tokio::test]
async fn list_and_load_carry_summaries() {
    let app = spawn().await;
    let (_, alex, _, sam, _, msg) = setup(&app).await;
    react(&app, &sam, msg.id, "😂").await;
    react(&app, &alex, msg.id, "👍").await;
    react(&app, &sam, msg.id, "👍").await;
    assert_eq!(
        reactions_of(&app, &alex, &msg).await,
        [summary("😂", 1, false), summary("👍", 2, true)]
    );
    assert_eq!(
        reactions_of(&app, &sam, &msg).await,
        [summary("😂", 1, true), summary("👍", 2, true)]
    );
    // Events (built by `load`) carry the counts, with `me` left to the client.
    let mut ws = hello(&app, &sam).await;
    let r = app
        .http
        .patch(app.url(&format!("/messages/{}", msg.id)))
        .bearer_auth(&alex)
        .json(&json!({"content": "edited"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    match next_frame(&mut ws, Duration::from_secs(3)).await {
        Some(ServerFrame::Event(Event::MessageUpdated { message })) => assert_eq!(
            message.reactions,
            [summary("😂", 1, false), summary("👍", 2, false)]
        ),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn send_records_usage() {
    let app = spawn().await;
    let (alex_id, alex, _, _, srv, msg) = setup(&app).await;
    let e = custom_emoji(&app, &alex, srv, "pog").await;
    send(
        &app,
        &alex,
        msg.channel_id,
        &format!(
            "nice <:pog:{}> 👍🏽 <a:gone:01JAAAAAAAAAAAAAAAAAAAAAAA> ok",
            e.id
        ),
    )
    .await;
    let mut top = emoji::frequent(&app.db, alex_id, 27).await.unwrap();
    top.sort();
    let mut want = vec![format!("custom:{}", e.id), "👍🏽".to_string()];
    want.sort();
    assert_eq!(top, want);
}
