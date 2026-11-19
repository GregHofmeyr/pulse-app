//! Blocking a person (#474), the server's part: the list (yours only, `BlocksChanged`), DMs from them withheld for good
//! (looks sent to them, never reaches you), their typing in that DM, their pings, their direct server invites (#80),
//! and what they see of you: offline and an emptied profile.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{Block, BlockMode, Message, PresenceStatus, Profile, ServerInvite};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello(app: &TestApp, token: &str) -> (Socket, Ready) {
    let mut ws = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap()
        .0;
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    loop {
        if let Some(ServerFrame::Ready(r)) = next_frame(&mut ws).await {
            return (ws, r);
        }
    }
}

async fn next_frame(ws: &mut Socket) -> Option<ServerFrame> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        match m {
            Ws::Text(t) => return Some(serde_json::from_str(&t).unwrap()),
            Ws::Close(_) => return None,
            _ => continue,
        }
    }
}

async fn drain(ws: &mut Socket, ms: u64) -> Vec<Event> {
    let mut got = vec![];
    while let Ok(Some(f)) = tokio::time::timeout(Duration::from_millis(ms), next_frame(ws)).await {
        if let ServerFrame::Event(e) = f {
            got.push(e);
        }
    }
    got
}

async fn block(app: &TestApp, token: &str, who: UserId, mode: &str) -> reqwest::Response {
    app.http
        .put(app.url(&format!("/me/blocks/{who}")))
        .bearer_auth(token)
        .json(&json!({ "mode": mode }))
        .send()
        .await
        .unwrap()
}

async fn unblock(app: &TestApp, token: &str, who: UserId) -> reqwest::Response {
    app.http
        .delete(app.url(&format!("/me/blocks/{who}")))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
}

async fn history(app: &TestApp, token: &str, ch: ChannelId) -> Vec<String> {
    let list: Vec<Message> = get_json(app, token, &format!("/channels/{ch}/messages")).await;
    list.into_iter().map(|m| m.content).collect()
}

fn created(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::MessageCreated { message, .. } => Some(message.content.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn the_list_is_yours_and_says_how_each_shows() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (mut ws, ready) = hello(&app, &alex).await;
    assert!(ready.blocks.is_empty());
    drain(&mut ws, 200).await;

    let r = block(&app, &alex, sam_id, "hide").await;
    assert_eq!(r.status(), 200);
    let list: Vec<Block> = r.json().await.unwrap();
    assert_eq!(
        list,
        vec![Block {
            user_id: sam_id,
            mode: BlockMode::Hide
        }]
    );
    let heard = drain(&mut ws, 300).await;
    assert!(
        heard
            .iter()
            .any(|e| matches!(e, Event::BlocksChanged { list, .. } if list.len() == 1))
    );
    // changing how it shows
    let list: Vec<Block> = block(&app, &alex, sam_id, "line")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(list[0].mode, BlockMode::Line);
    let (_ws, ready) = hello(&app, &alex).await;
    assert_eq!(
        ready.blocks,
        vec![Block {
            user_id: sam_id,
            mode: BlockMode::Line
        }]
    );
    // not yourself, not nobody
    assert_eq!(block(&app, &alex, alex_id, "line").await.status(), 400);
    assert_eq!(
        block(&app, &alex, UserId::new(), "line").await.status(),
        404
    );
    // sam's own Ready knows nothing of it
    let (_ws, ready) = hello(&app, &sam).await;
    assert!(ready.blocks.is_empty());
    let r = unblock(&app, &alex, sam_id).await;
    assert_eq!(r.status(), 200);
    assert!(r.json::<Vec<Block>>().await.unwrap().is_empty());
}

#[tokio::test]
async fn their_dms_look_sent_and_never_arrive_not_even_after_an_unblock() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let dm = create_dm(&app, &sam, &[alex_id]).await.id;
    send(&app, &sam, dm, "before").await;
    assert_eq!(block(&app, &alex, sam_id, "line").await.status(), 200);
    let (mut alex_ws, _) = hello(&app, &alex).await;
    let (mut sam_ws, _) = hello(&app, &sam).await;
    drain(&mut alex_ws, 200).await;
    drain(&mut sam_ws, 200).await;

    // sent, as far as sam can tell
    let m = send(&app, &sam, dm, "while blocked").await;
    assert_eq!(m.content, "while blocked");
    assert_eq!(
        created(&drain(&mut sam_ws, 300).await),
        vec!["while blocked"]
    );
    assert_eq!(
        history(&app, &sam, dm).await,
        vec!["while blocked", "before"]
    );
    // nothing reaches alex: no event, no history, no unread, no fetch around it
    assert!(created(&drain(&mut alex_ws, 300).await).is_empty());
    assert_eq!(history(&app, &alex, dm).await, vec!["before"]);
    let (_ws, ready) = hello(&app, &alex).await;
    let rs = ready
        .read_states
        .iter()
        .find(|r| r.channel_id == dm)
        .unwrap();
    assert_eq!(rs.unread, 1, "only the message from before");
    let r = app
        .http
        .get(app.url(&format!("/channels/{dm}/messages?around={}", m.id)))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    // their typing doesn't reach alex either
    let f = serde_json::to_string(&ClientFrame::Typing { channel_id: dm }).unwrap();
    sam_ws.send(Ws::text(f)).await.unwrap();
    assert!(
        !drain(&mut alex_ws, 300)
            .await
            .iter()
            .any(|e| matches!(e, Event::Typing { .. }))
    );

    // alex can't message someone they've blocked
    let r = post_json(
        &app,
        &alex,
        &format!("/channels/{dm}/messages"),
        json!({ "content": "hi" }),
    )
    .await;
    assert_eq!(r.status(), 403);

    // unblocked: what came while blocked stays away; what comes now arrives
    assert_eq!(unblock(&app, &alex, sam_id).await.status(), 200);
    drain(&mut alex_ws, 200).await;
    send(&app, &sam, dm, "after").await;
    assert_eq!(created(&drain(&mut alex_ws, 300).await), vec!["after"]);
    assert_eq!(history(&app, &alex, dm).await, vec!["after", "before"]);
}

#[tokio::test]
async fn a_new_dm_from_them_never_shows_up() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    acquaint(&app, &alex, &[&sam]).await;
    assert_eq!(block(&app, &alex, sam_id, "hide").await.status(), 200);
    let (mut alex_ws, _) = hello(&app, &alex).await;
    drain(&mut alex_ws, 200).await;
    let dm = create_dm(&app, &sam, &[alex_id]).await.id;
    send(&app, &sam, dm, "hello?").await;
    let heard = drain(&mut alex_ws, 400).await;
    assert!(
        !heard
            .iter()
            .any(|e| matches!(e, Event::ChannelCreated { channel } if channel.id == dm))
    );
    assert!(created(&heard).is_empty());
}

#[tokio::test]
async fn their_pings_dont_count_and_their_direct_invites_skip_you() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let shared = acquaint(&app, &sam, &[&alex]).await;
    let chat = general(&app, &sam, shared).await.id;
    assert_eq!(block(&app, &alex, sam_id, "line").await.status(), 200);

    // a mention from sam in a shared server: readable, but no ping
    send(&app, &sam, chat, "hey @alex").await;
    let (_ws, ready) = hello(&app, &alex).await;
    let rs = ready
        .read_states
        .iter()
        .find(|r| r.channel_id == chat)
        .unwrap();
    assert_eq!(rs.mentions, 0);
    assert!(
        history(&app, &alex, chat)
            .await
            .contains(&"hey @alex".to_string())
    );

    // a direct invite from sam to another server: made and looks sent, and never reaches alex (#80)
    let other = create_server(&app, &sam, "Other").await.id;
    let (mut alex_ws, _) = hello(&app, &alex).await;
    drain(&mut alex_ws, 200).await;
    let r = post_json(
        &app,
        &sam,
        &format!("/servers/{other}/invites"),
        json!({ "for_user": alex_id }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let inv: ServerInvite = r.json().await.unwrap();
    assert_eq!(inv.for_user, Some(alex_id));
    let heard = drain(&mut alex_ws, 400).await;
    assert!(created(&heard).is_empty());
    assert!(
        !heard
            .iter()
            .any(|e| matches!(e, Event::ChannelCreated { .. }))
    );
}

#[tokio::test]
async fn they_see_you_offline_with_an_emptied_profile() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    acquaint(&app, &alex, &[&sam, &kim]).await;
    sqlx::query(
        "UPDATE users SET bio = 'hi there', pronouns = 'they/them', avatar_hash = ? WHERE id = ?",
    )
    .bind("ab".repeat(32))
    .bind(alex_id.to_string())
    .execute(&app.db)
    .await
    .unwrap();
    let (_alex_ws, _) = hello(&app, &alex).await;
    assert_eq!(block(&app, &alex, sam_id, "line").await.status(), 200);

    let p: Profile = get_json(&app, &sam, &format!("/users/{alex_id}/profile")).await;
    assert_eq!(p.user.avatar_hash, None);
    assert_eq!(p.card.bio, None);
    assert_eq!(p.card.pronouns, None);
    assert!(p.badges.is_empty());
    let p: Profile = get_json(&app, &kim, &format!("/users/{alex_id}/profile")).await;
    assert_eq!(p.card.bio.as_deref(), Some("hi there"));

    // sam's Ready: alex offline, never seen, no avatar; kim's: online, as alex is
    let (_ws, ready) = hello(&app, &sam).await;
    let a = ready.people.iter().find(|p| p.user.id == alex_id).unwrap();
    assert_eq!(a.status, PresenceStatus::Offline);
    assert_eq!(a.last_seen_at, None);
    assert_eq!(a.user.avatar_hash, None);
    let (_ws, ready) = hello(&app, &kim).await;
    let a = ready.people.iter().find(|p| p.user.id == alex_id).unwrap();
    assert_eq!(a.status, PresenceStatus::Online);
    assert!(a.user.avatar_hash.is_some());
}
