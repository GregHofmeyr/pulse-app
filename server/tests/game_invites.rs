//! Invite to game (#436), against the fake Steam: an invite is a card in a DM or group with the game you play; where to
//! join is read from Steam at send time (with "Share Join" off and your game hidden from the recipient too) and handed
//! only to the chat's members on request, never inside a message (events, history, forwards, search); a lobby that
//! shows up later makes it joinable; leaving the game or the lobby ends it, live and on reload. Nothing here reaches
//! the real network.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{GameInvite, Message, PlayingUpdate, SteamJoin};
use pulse_server::testing::fake_steam::FakeSteam;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const ALEX_STEAM: &str = "76500000000000001";
const SPACE: u32 = 4_000_001;
/// An obviously fake lobby (account 1).
const LOBBY: &str = "109775240917155841";

async fn hello(app: &TestApp, token: &str) -> Socket {
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
        if let Some(ServerFrame::Ready(_)) = next_frame(&mut ws).await {
            return ws;
        }
    }
}

/// The next frame, as parsed, with its raw text.
async fn next_raw(ws: &mut Socket) -> Option<(ServerFrame, String)> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        match m {
            Ws::Text(t) => return Some((serde_json::from_str(&t).unwrap(), t.to_string())),
            Ws::Close(_) => return None,
            _ => continue,
        }
    }
}

async fn next_frame(ws: &mut Socket) -> Option<ServerFrame> {
    next_raw(ws).await.map(|(f, _)| f)
}

/// Every event for `ms`, with the raw text of everything received.
async fn drain(ws: &mut Socket, ms: u64) -> (Vec<Event>, String) {
    let mut got = vec![];
    let mut raw = String::new();
    while let Ok(Some((f, t))) = tokio::time::timeout(Duration::from_millis(ms), next_raw(ws)).await
    {
        raw.push_str(&t);
        if let ServerFrame::Event(e) = f {
            got.push(e);
        }
    }
    (got, raw)
}

fn invite_changes(events: &[Event], id: MessageId) -> Vec<GameInvite> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::GameInviteChanged {
                message_id, invite, ..
            } if *message_id == id => Some((**invite).clone()),
            _ => None,
        })
        .collect()
}

async fn play(app: &TestApp, ws: &mut Socket, me: UserId, on: bool) {
    let f = serde_json::to_string(&ClientFrame::Playing {
        playing: on.then(|| PlayingUpdate {
            name: "Space Game".into(),
            steam_app_id: Some(SPACE),
            elapsed_secs: Some(60),
            hide_art: false,
        }),
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    for _ in 0..200 {
        if app.hub.playing_of(me).is_some() == on {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the server never held the game");
}

async fn join_tick(app: &TestApp) {
    pulse_server::steam::join::tick(&app.state()).await;
}

async fn invite(app: &TestApp, token: &str, ch: ChannelId) -> reqwest::Response {
    post_json(
        app,
        token,
        &format!("/channels/{ch}/game-invite"),
        json!({}),
    )
    .await
}

async fn join(app: &TestApp, token: &str, id: MessageId) -> reqwest::Response {
    post_json(
        app,
        token,
        &format!("/messages/{id}/game-invite/join"),
        json!({}),
    )
    .await
}

async fn history(app: &TestApp, token: &str, ch: ChannelId) -> (Vec<Message>, String) {
    let r = app
        .http
        .get(app.url(&format!("/channels/{ch}/messages")))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let raw = r.text().await.unwrap();
    (serde_json::from_str(&raw).unwrap(), raw)
}

struct World {
    app: TestApp,
    fake: FakeSteam,
    alex: (UserId, String),
    sam: (UserId, String),
    kim: (UserId, String),
}

/// alex is linked to Steam with "Share Join" off; sam and kim aren't linked.
async fn world() -> World {
    let fake = FakeSteam::start().await;
    let mut cfg = test_config();
    fake.configure(&mut cfg);
    let app = spawn_with(cfg).await;
    fake.player(ALEX_STEAM, "alex on steam");
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let kim = register(&app, "kim").await;
    sqlx::query(
        "INSERT INTO steam_links (user_id, steam_id, linked_at, backfill, share_join) VALUES (?, ?, ?, 'done', 0)",
    )
    .bind(alex.0.to_string())
    .bind(ALEX_STEAM)
    .bind(pulse_server::db::now())
    .execute(&app.db)
    .await
    .unwrap();
    World {
        app,
        fake,
        alex,
        sam,
        kim,
    }
}

fn lobby() -> SteamJoin {
    SteamJoin::Lobby {
        app_id: SPACE,
        lobby_id: LOBBY.into(),
        steam_id: ALEX_STEAM.into(),
    }
}

#[tokio::test]
async fn an_invite_hands_where_to_join_only_to_the_chat_and_never_in_a_message() {
    let World {
        app,
        fake,
        alex: (alex, alex_t),
        sam: (sam, sam_t),
        kim: (kim, kim_t),
    } = world().await;
    let dm = create_dm(&app, &alex_t, &[sam]).await;
    let mut alex_ws = hello(&app, &alex_t).await;
    let mut sam_ws = hello(&app, &sam_t).await;

    // not in a game: nothing to invite to
    assert_eq!(invite(&app, &alex_t, dm.id).await.status(), 409);

    // alex hides his games from sam, and doesn't share Join: an invite is consent for this chat all the same
    let r = app
        .http
        .put(app.url(&format!("/me/activity-hidden/{sam}")))
        .bearer_auth(&alex_t)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    fake.in_game(ALEX_STEAM, Some((SPACE, Some(LOBBY), None)));
    play(&app, &mut alex_ws, alex, true).await;
    drain(&mut sam_ws, 300).await;

    let r = invite(&app, &alex_t, dm.id).await;
    assert_eq!(r.status(), 201);
    let raw = r.text().await.unwrap();
    assert!(!raw.contains(LOBBY), "{raw}");
    let msg: Message = serde_json::from_str(&raw).unwrap();
    let card = msg.game_invite.clone().expect("a card");
    assert_eq!(card.name, "Space Game");
    assert_eq!(card.steam_app_id, Some(SPACE));
    assert!(card.joinable && !card.ended);
    assert_eq!(msg.author_id, Some(alex));
    assert_eq!(msg.content, "🎮 Invite to play Space Game");

    // sam gets the card live, with nothing to join in it, and no game in his view of alex
    let (events, raw) = drain(&mut sam_ws, 400).await;
    assert!(!raw.contains(LOBBY), "{raw}");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::MessageCreated { message, .. }
            if message.id == msg.id && message.game_invite.as_ref().is_some_and(|c| c.joinable))),
        "{events:?}"
    );
    assert!(!events.iter().any(
        |e| matches!(e, Event::PlayingChanged { user_id, playing: Some(_) } if *user_id == alex)
    ));
    let (page, raw) = history(&app, &sam_t, dm.id).await;
    assert!(!raw.contains(LOBBY), "{raw}");
    assert!(
        page.iter()
            .any(|m| m.id == msg.id && m.game_invite.is_some())
    );

    // where to join: the chat's members get it, nobody else
    let r = join(&app, &sam_t, msg.id).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<SteamJoin>().await.unwrap(), lobby());
    assert_eq!(join(&app, &kim_t, msg.id).await.status(), 404);

    // the card can't be edited into something else
    let r = app
        .http
        .patch(app.url(&format!("/messages/{}", msg.id)))
        .bearer_auth(&alex_t)
        .json(&json!({ "content": "something else" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);

    // a forward carries the words only: no card, nothing to join, anywhere it goes
    let server = create_server(&app, &sam_t, "Gamers").await;
    let r = join_server(&app, &kim_t, server.id).await;
    assert!(r.status().is_success());
    let general = general(&app, &sam_t, server.id).await;
    let r = post_json(
        &app,
        &sam_t,
        &format!("/messages/{}/forward", msg.id),
        json!({ "channel_id": general.id, "comment": "" }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let raw = r.text().await.unwrap();
    assert!(!raw.contains(LOBBY), "{raw}");
    let fwd: Message = serde_json::from_str(&raw).unwrap();
    assert!(fwd.game_invite.is_none());
    let (page, raw) = history(&app, &kim_t, general.id).await;
    assert!(!raw.contains(LOBBY), "{raw}");
    assert!(page.iter().all(|m| m.game_invite.is_none()));
    assert_eq!(join(&app, &kim_t, fwd.id).await.status(), 404);
    assert_eq!(join(&app, &kim_t, msg.id).await.status(), 404);
    let _ = kim;
}

#[tokio::test]
async fn leaving_the_game_ends_the_invite_live_and_on_reload() {
    let World {
        app,
        fake,
        alex: (alex, alex_t),
        sam: (sam, sam_t),
        ..
    } = world().await;
    let dm = create_dm(&app, &alex_t, &[sam]).await;
    let mut alex_ws = hello(&app, &alex_t).await;
    let mut sam_ws = hello(&app, &sam_t).await;
    fake.in_game(ALEX_STEAM, Some((SPACE, Some(LOBBY), None)));
    play(&app, &mut alex_ws, alex, true).await;
    let msg: Message = invite(&app, &alex_t, dm.id).await.json().await.unwrap();
    drain(&mut sam_ws, 300).await;

    play(&app, &mut alex_ws, alex, false).await;
    let (events, raw) = drain(&mut sam_ws, 400).await;
    assert!(!raw.contains(LOBBY));
    let changes = invite_changes(&events, msg.id);
    assert_eq!(changes.len(), 1, "{events:?}");
    assert!(changes[0].ended && !changes[0].joinable);
    assert_eq!(join(&app, &sam_t, msg.id).await.status(), 409);
    let (page, _) = history(&app, &sam_t, dm.id).await;
    let card = page
        .iter()
        .find(|m| m.id == msg.id)
        .and_then(|m| m.game_invite.clone())
        .unwrap();
    assert!(card.ended && !card.joinable);

    // playing it again doesn't bring an ended invite back
    play(&app, &mut alex_ws, alex, true).await;
    join_tick(&app).await;
    assert_eq!(join(&app, &sam_t, msg.id).await.status(), 409);
}

#[tokio::test]
async fn a_lobby_that_shows_up_later_makes_it_joinable_and_leaving_the_lobby_ends_it() {
    let World {
        app,
        fake,
        alex: (alex, alex_t),
        sam: (sam, sam_t),
        kim: (kim, kim_t),
    } = world().await;
    // a group works the same way
    let group = create_dm(&app, &alex_t, &[sam, kim]).await;
    let mut alex_ws = hello(&app, &alex_t).await;
    let mut kim_ws = hello(&app, &kim_t).await;
    fake.in_game(ALEX_STEAM, Some((SPACE, None, None)));
    play(&app, &mut alex_ws, alex, true).await;
    let msg: Message = invite(&app, &alex_t, group.id).await.json().await.unwrap();
    let card = msg.game_invite.unwrap();
    assert!(!card.joinable && !card.ended);
    assert_eq!(join(&app, &sam_t, msg.id).await.status(), 409);
    drain(&mut kim_ws, 300).await;

    // the Join round finds a lobby though alex doesn't share Join
    fake.in_game(ALEX_STEAM, Some((SPACE, Some(LOBBY), None)));
    join_tick(&app).await;
    let (events, raw) = drain(&mut kim_ws, 400).await;
    assert!(!raw.contains(LOBBY), "{raw}");
    let changes = invite_changes(&events, msg.id);
    assert!(changes.last().is_some_and(|c| c.joinable), "{events:?}");
    assert_eq!(
        join(&app, &kim_t, msg.id)
            .await
            .json::<SteamJoin>()
            .await
            .unwrap(),
        lobby()
    );
    // and his Playing still shows no Join to anyone: Share Join is off
    assert_eq!(app.hub.playing_of(alex).and_then(|p| p.join), None);

    // out of the lobby, still in the game: the invite ends
    fake.in_game(ALEX_STEAM, Some((SPACE, None, None)));
    join_tick(&app).await;
    let (events, _) = drain(&mut kim_ws, 400).await;
    assert!(
        invite_changes(&events, msg.id)
            .last()
            .is_some_and(|c| c.ended),
        "{events:?}"
    );
    assert_eq!(join(&app, &sam_t, msg.id).await.status(), 409);
}

#[tokio::test]
async fn invites_go_in_dms_and_groups_only() {
    let World {
        app,
        alex: (alex, alex_t),
        ..
    } = world().await;
    let mut alex_ws = hello(&app, &alex_t).await;
    play(&app, &mut alex_ws, alex, true).await;
    let server = create_server(&app, &alex_t, "Gamers").await;
    let general = general(&app, &alex_t, server.id).await;
    assert_eq!(invite(&app, &alex_t, general.id).await.status(), 400);
}

#[tokio::test]
async fn after_a_restart_the_same_game_soon_reopens_the_invite() {
    // #438: a restart ends every open invite, but its sender reporting the same game within the grace opens it again;
    // once the grace is over, it stays ended.
    let World {
        app,
        fake,
        alex: (alex, alex_t),
        sam: (sam, sam_t),
        ..
    } = world().await;
    let dm = create_dm(&app, &alex_t, &[sam]).await;
    let mut alex_ws = hello(&app, &alex_t).await;
    let mut sam_ws = hello(&app, &sam_t).await;
    fake.in_game(ALEX_STEAM, Some((SPACE, Some(LOBBY), None)));
    play(&app, &mut alex_ws, alex, true).await;
    let msg: Message = invite(&app, &alex_t, dm.id).await.json().await.unwrap();
    let late: Message = invite(&app, &alex_t, dm.id).await.json().await.unwrap();

    // the restart (the hub forgetting alex's game is the gateway's part; the invites' is this)
    assert_eq!(
        pulse_server::game_invites::end_all_on_startup(&app.db)
            .await
            .unwrap(),
        2
    );
    assert_eq!(join(&app, &sam_t, msg.id).await.status(), 409);
    // one of them is past its grace
    sqlx::query("UPDATE game_invites SET restart_grace_until = ? WHERE message_id = ?")
        .bind(pulse_server::db::stamp(
            jiff::Timestamp::now() - jiff::SignedDuration::from_mins(1),
        ))
        .bind(late.id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    play(&app, &mut alex_ws, alex, false).await;
    drain(&mut sam_ws, 300).await;

    // alex's client reports the game again
    play(&app, &mut alex_ws, alex, true).await;
    let (events, raw) = drain(&mut sam_ws, 500).await;
    assert!(!raw.contains(LOBBY), "{raw}");
    let changes = invite_changes(&events, msg.id);
    assert!(
        changes.last().is_some_and(|c| !c.ended && c.joinable),
        "{events:?}"
    );
    assert_eq!(
        join(&app, &sam_t, msg.id)
            .await
            .json::<SteamJoin>()
            .await
            .unwrap(),
        lobby()
    );
    assert!(invite_changes(&events, late.id).is_empty());
    assert_eq!(join(&app, &sam_t, late.id).await.status(), 409);

    // and it ends as any invite does when they leave the game
    play(&app, &mut alex_ws, alex, false).await;
    let (events, _) = drain(&mut sam_ws, 400).await;
    assert!(
        invite_changes(&events, msg.id)
            .last()
            .is_some_and(|c| c.ended)
    );
}

/// Another obviously fake lobby (account 2).
const OTHER_LOBBY: &str = "109775240917155842";

async fn leave_group(
    app: &TestApp,
    token: &str,
    ch: ChannelId,
    who: UserId,
) -> reqwest::StatusCode {
    app.http
        .delete(app.url(&format!("/channels/{ch}/members/{who}")))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn leaving_the_group_ends_the_senders_invites_there() {
    // #438: an invite doesn't outlive its sender's membership: leaving (or being removed from) the group ends it live,
    // and the Join round no longer reads a lobby into it.
    let World {
        app,
        fake,
        alex: (alex, alex_t),
        sam: (sam, sam_t),
        kim: (kim, kim_t),
    } = world().await;
    let group = create_dm(&app, &alex_t, &[sam, kim]).await;
    let mut alex_ws = hello(&app, &alex_t).await;
    let mut sam_ws = hello(&app, &sam_t).await;
    fake.in_game(ALEX_STEAM, Some((SPACE, Some(LOBBY), None)));
    play(&app, &mut alex_ws, alex, true).await;
    let msg: Message = invite(&app, &alex_t, group.id).await.json().await.unwrap();
    assert_eq!(join(&app, &sam_t, msg.id).await.status(), 200);
    drain(&mut sam_ws, 300).await;

    assert_eq!(leave_group(&app, &alex_t, group.id, alex).await, 204);
    let (events, raw) = drain(&mut sam_ws, 400).await;
    assert!(!raw.contains(LOBBY), "{raw}");
    assert!(
        invite_changes(&events, msg.id)
            .last()
            .is_some_and(|c| c.ended && !c.joinable),
        "{events:?}"
    );
    assert_eq!(join(&app, &sam_t, msg.id).await.status(), 409);
    // still playing, still in the lobby: the Join round doesn't bring it back
    join_tick(&app).await;
    assert_eq!(join(&app, &kim_t, msg.id).await.status(), 409);

    // removed by someone else: the same
    let group = create_dm(&app, &alex_t, &[sam, kim]).await;
    let msg: Message = invite(&app, &alex_t, group.id).await.json().await.unwrap();
    drain(&mut sam_ws, 300).await;
    assert_eq!(leave_group(&app, &sam_t, group.id, alex).await, 204);
    let (events, _) = drain(&mut sam_ws, 400).await;
    assert!(
        invite_changes(&events, msg.id)
            .last()
            .is_some_and(|c| c.ended),
        "{events:?}"
    );
    join_tick(&app).await;
    assert_eq!(join(&app, &sam_t, msg.id).await.status(), 409);
}

#[tokio::test]
async fn an_invite_whose_sender_is_no_longer_a_member_hands_out_nothing() {
    // #438, defence in depth: whatever left the sender's membership behind without ending the invite, join, the Join
    // round and a reopen after a restart all need the sender still in the chat.
    let World {
        app,
        fake,
        alex: (alex, alex_t),
        sam: (sam, sam_t),
        kim: (kim, _),
    } = world().await;
    let group = create_dm(&app, &alex_t, &[sam, kim]).await;
    let mut alex_ws = hello(&app, &alex_t).await;
    fake.in_game(ALEX_STEAM, Some((SPACE, None, None)));
    play(&app, &mut alex_ws, alex, true).await;
    let msg: Message = invite(&app, &alex_t, group.id).await.json().await.unwrap();
    let gone = || async {
        sqlx::query("DELETE FROM channel_members WHERE channel_id = ? AND user_id = ?")
            .bind(group.id.to_string())
            .bind(alex.to_string())
            .execute(&app.db)
            .await
            .unwrap();
    };
    let back = || async {
        sqlx::query("INSERT INTO channel_members (channel_id, user_id, added_at) VALUES (?, ?, ?)")
            .bind(group.id.to_string())
            .bind(alex.to_string())
            .bind(pulse_server::db::now())
            .execute(&app.db)
            .await
            .unwrap();
    };
    gone().await;
    // a lobby shows up: not for an invite whose sender left
    fake.in_game(ALEX_STEAM, Some((SPACE, Some(LOBBY), None)));
    join_tick(&app).await;
    let set: Option<String> =
        sqlx::query_scalar("SELECT join_info FROM game_invites WHERE message_id = ?")
            .bind(msg.id.to_string())
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(set, None);
    // a join_info already there isn't handed out either
    sqlx::query("UPDATE game_invites SET join_info = ? WHERE message_id = ?")
        .bind(serde_json::to_string(&lobby()).unwrap())
        .bind(msg.id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(join(&app, &sam_t, msg.id).await.status(), 409);

    // nor does a restart's grace reopen it
    back().await;
    pulse_server::game_invites::end_all_on_startup(&app.db)
        .await
        .unwrap();
    gone().await;
    play(&app, &mut alex_ws, alex, false).await;
    play(&app, &mut alex_ws, alex, true).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let ended: Option<String> =
        sqlx::query_scalar("SELECT ended_at FROM game_invites WHERE message_id = ?")
            .bind(msg.id.to_string())
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(ended.is_some());
}

#[tokio::test]
async fn an_invite_doesnt_follow_its_sender_to_another_lobby() {
    // #438: once an invite offers a lobby, a different lobby ends it rather than moving it.
    let World {
        app,
        fake,
        alex: (alex, alex_t),
        sam: (sam, sam_t),
        ..
    } = world().await;
    let dm = create_dm(&app, &alex_t, &[sam]).await;
    let mut alex_ws = hello(&app, &alex_t).await;
    let mut sam_ws = hello(&app, &sam_t).await;
    fake.in_game(ALEX_STEAM, Some((SPACE, Some(LOBBY), None)));
    play(&app, &mut alex_ws, alex, true).await;
    let msg: Message = invite(&app, &alex_t, dm.id).await.json().await.unwrap();
    drain(&mut sam_ws, 300).await;

    fake.in_game(ALEX_STEAM, Some((SPACE, Some(OTHER_LOBBY), None)));
    join_tick(&app).await;
    let (events, raw) = drain(&mut sam_ws, 400).await;
    assert!(!raw.contains(OTHER_LOBBY), "{raw}");
    assert!(
        invite_changes(&events, msg.id)
            .last()
            .is_some_and(|c| c.ended),
        "{events:?}"
    );
    assert_eq!(join(&app, &sam_t, msg.id).await.status(), 409);
}
