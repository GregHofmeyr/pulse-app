//! Leaving a server and its owner removing someone (#371): who may, what goes with the membership (read points,
//! inbox items, reminders, their place in its calls), what stays (their messages), and that a rejoin works.
mod common;

use std::time::Duration;

use common::*;
use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{Channel, InboxUnread, Member, Message, MessageReminder};
use serde_json::json;
use tokio::sync::mpsc;

/// The next event `rx` gets that `want` picks (others skipped); panics after 3 s.
async fn wait_for<T>(
    rx: &mut mpsc::Receiver<ServerFrame>,
    mut want: impl FnMut(Event) -> Option<T>,
) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let f = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("the event arrives")
            .expect("hub open");
        if let ServerFrame::Event(e) = f
            && let Some(t) = want(e)
        {
            return t;
        }
    }
}

/// alex owns "Main"; sam and kim are in it; jo isn't.
struct World {
    app: TestApp,
    alex: (UserId, String),
    sam: (UserId, String),
    kim: (UserId, String),
    jo: (UserId, String),
    server: ServerId,
    general: Channel,
}

async fn world_with(app: TestApp) -> World {
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let kim = register(&app, "kim").await;
    let jo = register(&app, "jo").await;
    let s = create_server(&app, &alex.1, "Main").await;
    for who in [&sam, &kim] {
        assert_eq!(join(&app, &who.1, s.id).await, 204);
    }
    let general = general(&app, &alex.1, s.id).await;
    World {
        app,
        alex,
        sam,
        kim,
        jo,
        server: s.id,
        general,
    }
}

async fn join(app: &TestApp, token: &str, server: ServerId) -> u16 {
    post_json(app, token, &format!("/servers/{server}/join"), json!({}))
        .await
        .status()
        .as_u16()
}

async fn leave(app: &TestApp, token: &str, server: ServerId) -> reqwest::Response {
    post_json(app, token, &format!("/servers/{server}/leave"), json!({})).await
}

async fn remove(app: &TestApp, token: &str, server: ServerId, user: UserId) -> u16 {
    app.http
        .delete(app.url(&format!("/servers/{server}/members/{user}")))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn members(w: &World) -> Vec<UserId> {
    get_json::<Vec<Member>>(&w.app, &w.alex.1, &format!("/servers/{}/members", w.server))
        .await
        .into_iter()
        .map(|m| m.user.id)
        .collect()
}

async fn count(app: &TestApp, sql: &str, a: &str, b: &str) -> i64 {
    sqlx::query_scalar(sql)
        .bind(a)
        .bind(b)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn who_may_leave_and_who_may_remove() {
    let w = world_with(spawn().await).await;
    let r = leave(&w.app, &w.alex.1, w.server).await;
    assert_eq!(r.status(), 403, "the owner can't leave");
    let body: serde_json::Value = r.json().await.unwrap();
    assert!(
        body["message"].as_str().unwrap().contains("owner"),
        "{body}"
    );
    assert_eq!(
        leave(&w.app, &w.jo.1, w.server).await.status(),
        404,
        "not in it"
    );
    assert_eq!(
        remove(&w.app, &w.sam.1, w.server, w.kim.0).await,
        403,
        "only the owner removes"
    );
    assert_eq!(remove(&w.app, &w.alex.1, w.server, w.alex.0).await, 400);
    assert_eq!(remove(&w.app, &w.alex.1, w.server, w.jo.0).await, 404);
    assert_eq!(members(&w).await.len(), 3);
}

#[tokio::test]
async fn leaving_takes_their_place_but_keeps_their_messages_and_a_rejoin_works() {
    let w = world_with(spawn().await).await;
    let g = w.general.id;
    let theirs = send(&w.app, &w.sam.1, g, "bye all").await;
    // a mention and a reminder in the server
    send(&w.app, &w.kim.1, g, "@sam see you").await;
    let at = (chrono::Utc::now() + chrono::TimeDelta::minutes(30)).to_rfc3339();
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/messages/{}/reminders", theirs.id),
        json!({"remind_at": at}),
    )
    .await;
    assert_eq!(r.status(), 201);
    let unread: InboxUnread = get_json(&w.app, &w.sam.1, "/inbox/unread").await;
    assert_eq!(unread.unread, 1);
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/messages/{}/bookmark", theirs.id),
        json!({"note": "the goodbye"}),
    )
    .await;
    assert!(r.status().is_success(), "{}", r.status());
    let mut kim_rx = w.app.hub.register(w.kim.0, String::new()).rx;
    let mut sam_rx = w.app.hub.register(w.sam.0, String::new()).rx;

    assert_eq!(leave(&w.app, &w.sam.1, w.server).await.status(), 204);

    // everyone hears it, the leaver too
    let server = w.server;
    let sam_id = w.sam.0;
    for rx in [&mut kim_rx, &mut sam_rx] {
        let by = wait_for(rx, |e| match e {
            Event::MemberLeft {
                server_id,
                user_id,
                removed_by,
            } if server_id == server && user_id == sam_id => Some(removed_by),
            _ => None,
        })
        .await;
        assert_eq!(by, None);
    }
    let gone = wait_for(&mut sam_rx, |e| match e {
        Event::ReminderRemoved { id, .. } => Some(id),
        _ => None,
    })
    .await;
    let mine: Vec<MessageReminder> = get_json(&w.app, &w.sam.1, "/reminders").await;
    assert!(mine.is_empty(), "{gone} still listed");
    assert!(!members(&w).await.contains(&w.sam.0));
    let (uid, sid) = (w.sam.0.to_string(), w.server.to_string());
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM read_states WHERE user_id = ?
             AND channel_id IN (SELECT id FROM channels WHERE server_id = ?)",
            &uid,
            &sid
        )
        .await,
        0
    );
    let unread: InboxUnread = get_json(&w.app, &w.sam.1, "/inbox/unread").await;
    assert_eq!(unread.unread, 0, "its mentions leave their inbox");
    // their messages stay
    let history: Vec<Message> =
        get_json(&w.app, &w.kim.1, &format!("/channels/{g}/messages")).await;
    assert!(history.iter().any(|m| m.id == theirs.id && !m.deleted));
    // they can't post or react there now, and a mention no longer reaches them
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/channels/{g}/messages"),
        json!({"content": "still here?"}),
    )
    .await;
    assert_eq!(r.status(), 403);
    let m = send(&w.app, &w.kim.1, g, "@sam you there?").await;
    assert!(m.mentions.is_empty(), "{:?}", m.mentions);
    let r = w
        .app
        .http
        .put(w.app.url(&format!("/messages/{}/reactions/👍", m.id)))
        .bearer_auth(&w.sam.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);

    // their bookmark of one of its messages reads as unavailable while they're out (#315), and a new one is refused
    let saved: Vec<pulse_protocol::rest::MessageBookmark> =
        get_json(&w.app, &w.sam.1, "/bookmarks").await;
    assert!(saved.iter().all(|b| b.message.is_none()), "{saved:?}");
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/messages/{}/bookmark", theirs.id),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 404);

    // back in by joining, as before, at "all read"
    assert_eq!(join(&w.app, &w.sam.1, w.server).await, 204);
    let saved: Vec<pulse_protocol::rest::MessageBookmark> =
        get_json(&w.app, &w.sam.1, "/bookmarks").await;
    assert!(saved.iter().all(|b| b.message.is_some()), "{saved:?}");
    assert!(members(&w).await.contains(&w.sam.0));
    send(&w.app, &w.sam.1, g, "back").await;
    let m = send(&w.app, &w.kim.1, g, "@sam welcome back").await;
    assert_eq!(m.mentions, vec![w.sam.0]);
}

fn participant(identity: &str, sid: &str) -> livekit_protocol::ParticipantInfo {
    livekit_protocol::ParticipantInfo {
        identity: identity.into(),
        sid: sid.into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn removing_someone_in_a_call_disconnects_them() {
    let rooms = common::livekit::SharedRooms::default();
    let (url, removed, _) = common::livekit::fake_livekit_full(rooms.clone()).await;
    let mut cfg = test_config();
    cfg.livekit_api_url = url;
    let w = world_with(spawn_with(cfg).await).await;
    let chans: Vec<Channel> = get_json(
        &w.app,
        &w.alex.1,
        &format!("/servers/{}/channels", w.server),
    )
    .await;
    let lounge = chans
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    assert!(w.app.voice.join(lounge.id, w.sam.0, "PA_sam"));
    rooms.lock().unwrap().push((
        lounge.id.to_string(),
        vec![participant(&w.sam.0.to_string(), "PA_sam")],
    ));
    sqlx::query(
        "INSERT INTO voice_sessions (id, user_id, channel_id, joined_at) VALUES (?, ?, ?, ?)",
    )
    .bind(pulse_protocol::ids::next_ulid().to_string())
    .bind(w.sam.0.to_string())
    .bind(lounge.id.to_string())
    .bind(pulse_server::db::now())
    .execute(&w.app.db)
    .await
    .unwrap();
    let mut sam_rx = w.app.hub.register(w.sam.0, String::new()).rx;
    let mut kim_rx = w.app.hub.register(w.kim.0, String::new()).rx;

    assert_eq!(remove(&w.app, &w.alex.1, w.server, w.sam.0).await, 204);

    let (alex_id, sam_id, ch) = (w.alex.0, w.sam.0, lounge.id);
    // told who did it, as a voice disconnect tells them
    let by = wait_for(&mut sam_rx, |e| match e {
        Event::VoiceDisconnected {
            channel_id,
            user_id,
            by,
        } if channel_id == ch && user_id == sam_id => Some(by),
        _ => None,
    })
    .await;
    assert_eq!(by, alex_id);
    let removed_by = wait_for(&mut kim_rx, |e| match e {
        Event::MemberLeft {
            user_id,
            removed_by,
            ..
        } if user_id == sam_id => Some(removed_by),
        _ => None,
    })
    .await;
    assert_eq!(removed_by, Some(alex_id));
    wait_for(&mut kim_rx, |e| match e {
        Event::VoiceLeft {
            channel_id,
            user_id,
        } if channel_id == ch && user_id == sam_id => Some(()),
        _ => None,
    })
    .await;
    assert!(w.app.voice_members(lounge.id).is_empty());
    assert_eq!(
        count(
            &w.app,
            "SELECT COUNT(*) FROM voice_sessions WHERE user_id = ? AND channel_id = ? AND left_at IS NULL",
            &w.sam.0.to_string(),
            &lounge.id.to_string()
        )
        .await,
        0
    );
    // out of its LiveKit voice room
    assert!(
        removed
            .lock()
            .unwrap()
            .contains(&(lounge.id.to_string(), w.sam.0.to_string())),
        "{:?}",
        removed.lock().unwrap()
    );
    // and a new voice token is refused
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/voice/{}/token", lounge.id),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 403);
}

async fn transfer(app: &TestApp, token: &str, server: ServerId, to: UserId) -> u16 {
    app.http
        .put(app.url(&format!("/servers/{server}/owner")))
        .bearer_auth(token)
        .json(&json!({ "user_id": to }))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn owner(w: &World) -> Option<UserId> {
    let servers: Vec<pulse_protocol::rest::Server> = get_json(&w.app, &w.kim.1, "/servers").await;
    servers
        .into_iter()
        .find(|s| s.id == w.server)
        .and_then(|s| s.owner_id)
}

#[tokio::test]
async fn the_owner_hands_the_server_over_and_can_then_leave() {
    let w = world_with(spawn().await).await;
    // only the owner, only to another member
    assert_eq!(transfer(&w.app, &w.sam.1, w.server, w.kim.0).await, 403);
    assert_eq!(transfer(&w.app, &w.alex.1, w.server, w.alex.0).await, 400);
    assert_eq!(transfer(&w.app, &w.alex.1, w.server, w.jo.0).await, 404);
    assert_eq!(owner(&w).await, Some(w.alex.0));

    let mut kim_rx = w.app.hub.register(w.kim.0, String::new()).rx;
    assert_eq!(transfer(&w.app, &w.alex.1, w.server, w.sam.0).await, 204);
    let server = w.server;
    let (to, from) = wait_for(&mut kim_rx, |e| match e {
        Event::ServerOwnerChanged {
            server_id,
            owner_id,
            previous_owner_id,
        } if server_id == server => Some((owner_id, previous_owner_id)),
        _ => None,
    })
    .await;
    assert_eq!((to, from), (w.sam.0, w.alex.0));
    assert_eq!(owner(&w).await, Some(w.sam.0));

    // the old owner is a member like any other now: no second hand-over, no removing, and they may leave
    assert_eq!(transfer(&w.app, &w.alex.1, w.server, w.kim.0).await, 403);
    assert_eq!(remove(&w.app, &w.alex.1, w.server, w.kim.0).await, 403);
    assert_eq!(leave(&w.app, &w.sam.1, w.server).await.status(), 403);
    assert_eq!(leave(&w.app, &w.alex.1, w.server).await.status(), 204);
    // the new owner removes people
    assert_eq!(remove(&w.app, &w.sam.1, w.server, w.kim.0).await, 204);
    assert_eq!(members(&w).await, vec![w.sam.0]);
}

#[tokio::test]
async fn a_hand_over_respects_the_server_cap() {
    let w = world_with(spawn().await).await;
    for i in 0..pulse_server::limits::caps::SERVERS_PER_USER {
        create_server(&w.app, &w.sam.1, &format!("Sam's {i}")).await;
    }
    assert_eq!(transfer(&w.app, &w.alex.1, w.server, w.sam.0).await, 400);
    assert_eq!(owner(&w).await, Some(w.alex.0));
}
