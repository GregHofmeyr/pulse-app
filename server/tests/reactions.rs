//! Reactions: toggling, counts, the 20-emoji cap, reaction modes, privacy and usage ranking.
mod common;

use std::io::Cursor;
use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use image::{ImageFormat, Rgba, RgbaImage};
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::{MessageId, ServerId, UserId};
use pulse_protocol::rest::{ApiError, CustomEmoji, MediaInfo, Message, ReactionSummary};
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

// Multi-threaded, so the toggles really race.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_reactions_arrive_in_commit_order() {
    let app = spawn().await;
    let (_, alex, _, _, srv, msg) = setup(&app).await;
    let mut tokens = Vec::new();
    for i in 0..8 {
        let (_, t) = register(&app, &format!("user{i}")).await;
        join(&app, &t, srv).await;
        tokens.push(t);
    }
    let n = tokens.len() as u32;
    let mut ws = hello(&app, &alex).await;
    let app = &app;
    // Several rounds of everyone adding at once, then everyone removing at once: with commit
    // order kept, the counts climb 1..=n and fall back to 0 exactly.
    for _ in 0..5 {
        for add in [true, false] {
            let codes = futures::future::join_all(tokens.iter().map(|t| async move {
                if add {
                    react(app, t, msg.id, "👍").await
                } else {
                    unreact(app, t, msg.id, "👍").await
                }
            }))
            .await;
            assert!(codes.iter().all(|&c| c == 204), "{codes:?}");
            let mut counts = Vec::new();
            for _ in 0..n {
                match next_reaction(&mut ws).await {
                    Event::ReactionAdded { count, .. } | Event::ReactionRemoved { count, .. } => {
                        counts.push(count)
                    }
                    other => panic!("{other:?}"),
                }
            }
            let total: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM reactions WHERE message_id = ?")
                    .bind(msg.id.to_string())
                    .fetch_one(&app.db)
                    .await
                    .unwrap();
            assert_eq!(*counts.last().unwrap() as i64, total);
            let want: Vec<u32> = if add {
                (1..=n).collect()
            } else {
                (0..n).rev().collect()
            };
            assert_eq!(counts, want);
        }
    }
}

async fn reactors_status(app: &TestApp, token: &str, msg: MessageId, emoji: &str) -> u16 {
    app.http
        .get(app.url(&path(msg, emoji)))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test]
async fn fe0f_variants_are_one_reaction() {
    let app = spawn().await;
    let (alex_id, alex, sam_id, sam, _, msg) = setup(&app).await;
    assert_eq!(react(&app, &alex, msg.id, "\u{2764}").await, 204);
    assert_eq!(react(&app, &sam, msg.id, "\u{2764}\u{FE0F}").await, 204);
    assert_eq!(
        reactions_of(&app, &alex, &msg).await,
        [summary("\u{2764}", 2, true)]
    );
    let users: Vec<UserId> = get_json(&app, &alex, &path(msg.id, "\u{2764}\u{FE0F}")).await;
    assert_eq!(users, [alex_id, sam_id]);
    // Removing works with either spelling.
    assert_eq!(unreact(&app, &alex, msg.id, "\u{2764}\u{FE0F}").await, 204);
    assert_eq!(unreact(&app, &sam, msg.id, "\u{2764}").await, 204);
    assert!(reactions_of(&app, &alex, &msg).await.is_empty());

    assert_eq!(react(&app, &alex, msg.id, "👍").await, 204);
    assert_eq!(react(&app, &sam, msg.id, "👍\u{FE0F}").await, 204);
    assert_eq!(
        reactions_of(&app, &alex, &msg).await,
        [summary("👍", 2, true)]
    );

    // Usage keys are canonical too, from reactions and from message text.
    react(&app, &alex, msg.id, "\u{263A}\u{FE0F}").await;
    send(&app, &alex, msg.channel_id, "\u{2764}\u{FE0F} \u{2764}").await;
    let top = emoji::frequent(&app.db, alex_id, 27).await.unwrap();
    assert!(top.iter().all(|k| !k.contains('\u{FE0F}')), "{top:?}");
    assert!(top.contains(&"\u{263A}".to_string()), "{top:?}");
    assert!(top.contains(&"\u{2764}".to_string()), "{top:?}");
}

#[tokio::test]
async fn junk_skin_tones_rejected() {
    let app = spawn().await;
    let (_, alex, _, _, _, msg) = setup(&app).await;
    for junk in [
        "👍🏽🏽",
        "🏽👍",
        "🏽",
        "👍🏽🏻",
        "😀🏽",
        "👍\u{200D}🏽",
        "🏽\u{200D}👍",
        "👩🏽🏽\u{200D}💻",
        "👍🏽x",
    ] {
        assert_eq!(react(&app, &alex, msg.id, junk).await, 400, "{junk:?}");
    }
    for ok in [
        "👍🏽",
        "👩🏿\u{200D}\u{2764}\u{FE0F}\u{200D}👨🏼",
        "🧑🏻\u{200D}🤝\u{200D}🧑🏿",
        "🏃🏽\u{200D}\u{2640}\u{FE0F}",
    ] {
        assert_eq!(react(&app, &alex, msg.id, ok).await, 204, "{ok:?}");
    }
}

#[tokio::test]
async fn reactors_reject_unknown_keys() {
    let app = spawn().await;
    let (_, alex, _, _, _, msg) = setup(&app).await;
    assert_eq!(reactors_status(&app, &alex, msg.id, "abc").await, 400);
    assert_eq!(reactors_status(&app, &alex, msg.id, "👍🏽🏽").await, 400);
    for junk in ["custom:", "custom:a b", "custom:a%3Ab"] {
        assert_eq!(
            reactors_status(&app, &alex, msg.id, junk).await,
            400,
            "{junk}"
        );
    }
    // A well-formed custom key that isn't on the message lists nobody, as DELETE accepts it.
    let r = app
        .http
        .get(app.url(&path(msg.id, "custom:01JAAAAAAAAAAAAAAAAAAAAAAA")))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Vec<UserId>>().await.unwrap(), vec![]);
    // Taking one off is a no-op rather than an error: see removing_a_gone_custom_reaction_is_a_no_op.
    assert_eq!(
        unreact(&app, &alex, msg.id, "custom:01JAAAAAAAAAAAAAAAAAAAAAAA").await,
        204
    );
    assert_eq!(reactors_status(&app, &alex, msg.id, "👍").await, 200);
}

#[tokio::test]
async fn unknown_reaction_mode_acts_normal() {
    let app = spawn().await;
    let (_, alex, _, _, _, msg) = setup(&app).await;
    sqlx::query("UPDATE channels SET reaction_mode = 'bogus' WHERE id = ?")
        .bind(msg.channel_id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(react(&app, &alex, msg.id, "👍").await, 204);
    assert_eq!(react(&app, &alex, msg.id, "😂").await, 204);
    assert_eq!(
        reactions_of(&app, &alex, &msg).await,
        [summary("👍", 1, true), summary("😂", 1, true)]
    );
}

#[tokio::test]
async fn custom_reactions_need_membership_of_the_emoji_server() {
    let app = spawn().await;
    let (alex_id, alex, sam_id, sam, srv, _) = setup(&app).await;
    let (kim_id, kim) = register(&app, "kim").await;
    let e = custom_emoji(&app, &alex, srv, "pog").await;
    let key = format!("custom:{}", e.id);

    // In a DM, and in kim's own server: kim isn't in the emoji's server.
    let dm = create_dm(&app, &alex, &[kim_id, sam_id]).await;
    let dm_msg = send(&app, &alex, dm.id, "hi").await;
    assert_eq!(react(&app, &kim, dm_msg.id, &key).await, 403);
    // The refusal reads as a reason, not a bare "forbidden".
    let r = app
        .http
        .put(app.url(&path(dm_msg.id, &key)))
        .bearer_auth(&kim)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let err: ApiError = r.json().await.unwrap();
    assert_eq!(err.code, "forbidden");
    assert!(err.message.contains("servers you're in"), "{}", err.message);
    let other = create_server(&app, &kim, "Other").await;
    let og = general(&app, &kim, other.id).await;
    let other_msg = send(&app, &kim, og.id, "hi").await;
    assert_eq!(react(&app, &kim, other_msg.id, &key).await, 403);
    // A member may use it anywhere they can react.
    assert_eq!(react(&app, &sam, dm_msg.id, &key).await, 204);
    assert_eq!(react(&app, &alex, dm_msg.id, &key).await, 204);

    // Leaving the server never strands your reaction.
    sqlx::query("DELETE FROM server_members WHERE server_id = ? AND user_id = ?")
        .bind(srv.to_string())
        .bind(sam_id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(react(&app, &sam, dm_msg.id, &key).await, 403);
    let users: Vec<UserId> = get_json(&app, &sam, &path(dm_msg.id, &key)).await;
    assert_eq!(users, [sam_id, alex_id]);
    assert_eq!(unreact(&app, &sam, dm_msg.id, &key).await, 204);

    // Sending another server's token: the text stays, but it isn't counted as kim's usage.
    let text = format!("look <:pog:{}>", e.id);
    let sent = send(&app, &kim, dm.id, &text).await;
    assert_eq!(sent.content, text);
    let top = emoji::frequent(&app.db, kim_id, 27).await.unwrap();
    assert!(top.is_empty(), "{top:?}");
}

#[tokio::test]
async fn orphaned_custom_reactions_can_be_listed_and_removed() {
    let app = spawn().await;
    let (alex_id, alex, sam_id, sam, srv, _) = setup(&app).await;
    let dm = create_dm(&app, &alex, &[sam_id]).await;
    let msg = send(&app, &alex, dm.id, "hi").await;
    let a = custom_emoji(&app, &alex, srv, "pog").await;
    let b = custom_emoji(&app, &alex, srv, "kek").await;
    let (ka, kb) = (format!("custom:{}", a.id), format!("custom:{}", b.id));
    assert_eq!(react(&app, &alex, msg.id, &ka).await, 204);
    assert_eq!(react(&app, &sam, msg.id, &kb).await, 204);

    // The emoji row is gone.
    sqlx::query("DELETE FROM custom_emoji WHERE id = ?")
        .bind(&a.id)
        .execute(&app.db)
        .await
        .unwrap();
    let users: Vec<UserId> = get_json(&app, &sam, &path(msg.id, &ka)).await;
    assert_eq!(users, [alex_id]);
    assert_eq!(react(&app, &sam, msg.id, &ka).await, 400);
    assert_eq!(unreact(&app, &alex, msg.id, &ka).await, 204);

    // The server row is gone (servers can't be deleted yet; this is what one would leave).
    let mut conn = app.db.acquire().await.unwrap();
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *conn)
        .await
        .unwrap();
    sqlx::query("DELETE FROM servers WHERE id = ?")
        .bind(srv.to_string())
        .execute(&mut *conn)
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *conn)
        .await
        .unwrap();
    drop(conn);
    let users: Vec<UserId> = get_json(&app, &alex, &path(msg.id, &kb)).await;
    assert_eq!(users, [sam_id]);
    assert_eq!(unreact(&app, &sam, msg.id, &kb).await, 204);
    assert!(reactions_of(&app, &alex, &msg).await.is_empty());
}

#[tokio::test]
async fn send_usage_is_capped() {
    let app = spawn().await;
    let (alex_id, alex, _, _, srv, msg) = setup(&app).await;
    let e = custom_emoji(&app, &alex, srv, "pog").await;
    let mut text = String::new();
    for i in 0..60 {
        let id = format!("E{i:03}");
        sqlx::query(
            "INSERT INTO custom_emoji (id, server_id, name, media_hash, animated, added_by, created_at)
             VALUES (?, ?, ?, ?, 0, ?, '2026-10-01T00:00:00.000Z')",
        )
        .bind(&id)
        .bind(srv.to_string())
        .bind(format!("e{i:03}"))
        .bind(e.hash.as_deref().unwrap())
        .bind(alex_id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
        text.push_str(&format!("<:e{i:03}:{id}>"));
    }
    send(&app, &alex, msg.channel_id, &text).await;
    let used: Vec<String> =
        sqlx::query_scalar("SELECT emoji FROM emoji_usage WHERE user_id = ? ORDER BY emoji")
            .bind(alex_id.to_string())
            .fetch_all(&app.db)
            .await
            .unwrap();
    let want: Vec<String> = (0..50).map(|i| format!("custom:E{i:03}")).collect();
    assert_eq!(used, want);
}

// Multi-threaded, so the adds really race for the last slot.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_adds_fill_only_the_last_slot() {
    let app = spawn().await;
    let (_, alex, _, sam, _, msg) = setup(&app).await;
    for e in &TWENTY[..19] {
        assert_eq!(react(&app, &alex, msg.id, e).await, 204, "{e}");
    }
    let racers = ["😋", "😛", "😜", "🤪", "😝", "🤑", "🤗", "🤭", "🤫", "🤔"];
    let app = &app;
    let codes = futures::future::join_all(racers.iter().enumerate().map(|(i, e)| {
        let token = if i % 2 == 0 { &alex } else { &sam };
        async move { react(app, token, msg.id, e).await }
    }))
    .await;
    assert_eq!(codes.iter().filter(|&&c| c == 204).count(), 1, "{codes:?}");
    assert_eq!(codes.iter().filter(|&&c| c == 400).count(), 9, "{codes:?}");
    let distinct: i64 =
        sqlx::query_scalar("SELECT COUNT(DISTINCT emoji) FROM reactions WHERE message_id = ?")
            .bind(msg.id.to_string())
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(distinct, 20);
}

#[tokio::test]
async fn removing_a_gone_custom_reaction_is_a_no_op() {
    let app = spawn().await;
    let (_, alex, _, _, srv, msg) = setup(&app).await;
    let e = custom_emoji(&app, &alex, srv, "pog").await;
    let key = format!("custom:{}", e.id);
    assert_eq!(react(&app, &alex, msg.id, &key).await, 204);
    sqlx::query("DELETE FROM custom_emoji WHERE id = ?")
        .bind(&e.id)
        .execute(&app.db)
        .await
        .unwrap();
    // The first removal takes it off; the second (a double click) finds nothing and still succeeds.
    assert_eq!(unreact(&app, &alex, msg.id, &key).await, 204);
    assert_eq!(unreact(&app, &alex, msg.id, &key).await, 204);
    assert!(reactions_of(&app, &alex, &msg).await.is_empty());
    // Malformed keys are still refused.
    for junk in ["custom:", "custom:a b", "custom:a%3Ab", "abc"] {
        assert_eq!(unreact(&app, &alex, msg.id, junk).await, 400, "{junk}");
    }
}

/// The next FrequentEmoji event, skipping everything else.
async fn next_frequent(ws: &mut Socket) -> (UserId, Vec<String>) {
    loop {
        match next_frame(ws, Duration::from_secs(3)).await {
            Some(ServerFrame::Event(Event::FrequentEmoji {
                user_id,
                frequent_emoji,
            })) => return (user_id, frequent_emoji),
            Some(_) => continue,
            None => panic!("no FrequentEmoji event"),
        }
    }
}

/// Every frame until the socket goes quiet.
async fn drain(ws: &mut Socket) -> Vec<ServerFrame> {
    let mut out = Vec::new();
    while let Some(f) = next_frame(ws, Duration::from_millis(500)).await {
        out.push(f);
    }
    out
}

fn has_frequent(frames: &[ServerFrame]) -> bool {
    frames
        .iter()
        .any(|f| matches!(f, ServerFrame::Event(Event::FrequentEmoji { .. })))
}

#[tokio::test]
async fn frequent_updates_reach_only_your_own_sessions() {
    let app = spawn().await;
    let (alex_id, alex, sam_id, sam, _, msg) = setup(&app).await;
    let mut alex1 = hello(&app, &alex).await;
    let mut alex2 = hello(&app, &alex).await;
    let mut sam1 = hello(&app, &sam).await;

    assert_eq!(react(&app, &alex, msg.id, "🎉").await, 204);
    for ws in [&mut alex1, &mut alex2] {
        assert_eq!(next_frequent(ws).await, (alex_id, vec!["🎉".to_string()]));
    }
    send(&app, &alex, msg.channel_id, "yay 😂").await;
    let want = emoji::frequent(&app.db, alex_id, 27).await.unwrap();
    assert_eq!(want.len(), 2);
    for ws in [&mut alex1, &mut alex2] {
        assert_eq!(next_frequent(ws).await, (alex_id, want.clone()));
    }
    assert!(!has_frequent(&drain(&mut sam1).await));

    // Writes that record no usage publish nothing.
    assert_eq!(unreact(&app, &alex, msg.id, "🎉").await, 204);
    send(&app, &alex, msg.channel_id, "plain").await;
    // Sam's own usage goes to sam alone.
    assert_eq!(react(&app, &sam, msg.id, "👍").await, 204);
    assert_eq!(
        next_frequent(&mut sam1).await,
        (sam_id, vec!["👍".to_string()])
    );
    for ws in [&mut alex1, &mut alex2] {
        assert!(!has_frequent(&drain(ws).await));
    }
}
