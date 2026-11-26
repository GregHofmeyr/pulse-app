//! Blocking a person (#474), the server's part: the list (yours only, `BlocksChanged`), DMs from them withheld for good
//! (looks sent to them, never reaches you), their typing in that DM, their pings, their direct server invites (#80),
//! and what they see of you: offline and an emptied profile.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{
    Block, BlockMode, Member, Message, NameStyle, PresenceStatus, Profile, ServerInvite,
    UpdateProfileRequest,
};
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

/// #652: a blocked viewer never gets the blocker's typing text (they see the plain "is typing"): not in Ready's
/// members or people, the members list, a ProfileUpdated, or the profile. Someone else does, everywhere.
#[tokio::test]
async fn a_blocked_viewer_never_gets_your_typing_text() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    let common = acquaint(&app, &alex, &[&sam, &kim]).await;
    let save = |text: &str| UpdateProfileRequest {
        avatar_hash: None,
        banner_hash: None,
        theme_top: "#8a3cff".into(),
        theme_bottom: "#ff8a2a".into(),
        style: NameStyle::default(),
        pronouns: None,
        bio: None,
        display_name: None,
        glow: None,
        typing_text: Some(text.into()),
        ring_style: None,
        ring_color: None,
    };
    let put = |body: UpdateProfileRequest| {
        app.http
            .put(app.url("/me/profile"))
            .bearer_auth(&alex)
            .json(&body)
            .send()
    };
    assert_eq!(put(save("cooking")).await.unwrap().status(), 200);
    assert_eq!(block(&app, &alex, sam_id, "line").await.status(), 200);
    let text_of = |m: &[Member]| member_user(m, alex_id).typing_text.clone();

    // Ready: members and people
    let (mut sam_ws, ready) = hello(&app, &sam).await;
    let m = &ready
        .members
        .iter()
        .find(|m| m.server_id == common)
        .unwrap()
        .members;
    assert_eq!(text_of(m), None);
    let p = ready.people.iter().find(|p| p.user.id == alex_id).unwrap();
    assert_eq!(p.user.typing_text, None);
    let (mut kim_ws, ready) = hello(&app, &kim).await;
    let m = &ready
        .members
        .iter()
        .find(|m| m.server_id == common)
        .unwrap()
        .members;
    assert_eq!(text_of(m).as_deref(), Some("cooking"));

    // the members list
    let list: Vec<Member> = get_json(&app, &sam, &format!("/servers/{common}/members")).await;
    assert_eq!(text_of(&list), None);
    let list: Vec<Member> = get_json(&app, &kim, &format!("/servers/{common}/members")).await;
    assert_eq!(text_of(&list).as_deref(), Some("cooking"));

    // the profile
    let p: Profile = get_json(&app, &sam, &format!("/users/{alex_id}/profile")).await;
    assert_eq!(p.user.typing_text, None);
    let p: Profile = get_json(&app, &kim, &format!("/users/{alex_id}/profile")).await;
    assert_eq!(p.user.typing_text.as_deref(), Some("cooking"));

    // a ProfileUpdated as alex changes it
    drain(&mut sam_ws, 200).await;
    drain(&mut kim_ws, 200).await;
    assert_eq!(put(save("yapping")).await.unwrap().status(), 200);
    let updated = |events: Vec<Event>| {
        events.into_iter().find_map(|e| match e {
            Event::ProfileUpdated { user, .. } if user.id == alex_id => Some(user),
            _ => None,
        })
    };
    let to_sam = updated(drain(&mut sam_ws, 500).await).expect("sam hears of the change");
    assert_eq!(to_sam.typing_text, None);
    let to_kim = updated(drain(&mut kim_ws, 500).await).expect("kim hears of the change");
    assert_eq!(to_kim.typing_text.as_deref(), Some("yapping"));
}

/// #651: a blocked viewer never gets the blocker's speaking ring (they see the plain green one): not in Ready's
/// members or people, the members list, a ProfileUpdated, or the profile. Someone else does, everywhere.
#[tokio::test]
async fn a_blocked_viewer_never_gets_your_speaking_ring() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    let common = acquaint(&app, &alex, &[&sam, &kim]).await;
    let save = |style: &str, color: &str| UpdateProfileRequest {
        avatar_hash: None,
        banner_hash: None,
        theme_top: "#8a3cff".into(),
        theme_bottom: "#ff8a2a".into(),
        style: NameStyle::default(),
        pronouns: None,
        bio: None,
        display_name: None,
        glow: None,
        typing_text: None,
        ring_style: Some(style.into()),
        ring_color: Some(color.into()),
    };
    let put = |body: UpdateProfileRequest| {
        app.http
            .put(app.url("/me/profile"))
            .bearer_auth(&alex)
            .json(&body)
            .send()
    };
    assert_eq!(put(save("orbit", "gold")).await.unwrap().status(), 200);
    assert_eq!(block(&app, &alex, sam_id, "line").await.status(), 200);
    let ring = |u: &pulse_protocol::rest::User| (u.ring_style.clone(), u.ring_color.clone());
    let none = (None, None);
    let orbit_gold = (Some("orbit".to_owned()), Some("gold".to_owned()));

    // Ready: members and people
    let (mut sam_ws, ready) = hello(&app, &sam).await;
    let m = &ready
        .members
        .iter()
        .find(|m| m.server_id == common)
        .unwrap()
        .members;
    assert_eq!(ring(member_user(m, alex_id)), none);
    let p = ready.people.iter().find(|p| p.user.id == alex_id).unwrap();
    assert_eq!(ring(&p.user), none);
    let (mut kim_ws, ready) = hello(&app, &kim).await;
    let m = &ready
        .members
        .iter()
        .find(|m| m.server_id == common)
        .unwrap()
        .members;
    assert_eq!(ring(member_user(m, alex_id)), orbit_gold);
    let p = ready.people.iter().find(|p| p.user.id == alex_id).unwrap();
    assert_eq!(ring(&p.user), orbit_gold);

    // the members list
    let list: Vec<Member> = get_json(&app, &sam, &format!("/servers/{common}/members")).await;
    assert_eq!(ring(member_user(&list, alex_id)), none);
    let list: Vec<Member> = get_json(&app, &kim, &format!("/servers/{common}/members")).await;
    assert_eq!(ring(member_user(&list, alex_id)), orbit_gold);

    // the profile
    let p: Profile = get_json(&app, &sam, &format!("/users/{alex_id}/profile")).await;
    assert_eq!(ring(&p.user), none);
    let p: Profile = get_json(&app, &kim, &format!("/users/{alex_id}/profile")).await;
    assert_eq!(ring(&p.user), orbit_gold);

    // a ProfileUpdated as alex changes it
    drain(&mut sam_ws, 200).await;
    drain(&mut kim_ws, 200).await;
    assert_eq!(put(save("bars", "purple")).await.unwrap().status(), 200);
    let updated = |events: Vec<Event>| {
        events.into_iter().find_map(|e| match e {
            Event::ProfileUpdated { user, .. } if user.id == alex_id => Some(user),
            _ => None,
        })
    };
    let to_sam = updated(drain(&mut sam_ws, 500).await).expect("sam hears of the change");
    assert_eq!(ring(&to_sam), none);
    let to_kim = updated(drain(&mut kim_ws, 500).await).expect("kim hears of the change");
    assert_eq!(
        ring(&to_kim),
        (Some("bars".to_owned()), Some("purple".to_owned()))
    );
}

/// The `User` of `who` among `members`.
fn member_user(members: &[Member], who: UserId) -> &pulse_protocol::rest::User {
    &members.iter().find(|m| m.user.id == who).unwrap().user
}

#[tokio::test]
async fn member_payloads_show_you_emptied_to_them() {
    // #508: the `User` inside members (Ready, the members list, joins, changes, a joined server's snapshot) is emptied
    // for the person you've blocked, as everywhere else
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    let common = acquaint(&app, &alex, &[&sam, &kim]).await;
    sqlx::query("UPDATE users SET avatar_hash = ? WHERE id = ?")
        .bind("ab".repeat(32))
        .bind(alex_id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(block(&app, &alex, sam_id, "hide").await.status(), 200);

    // Ready's members and the members list: emptied for sam, as it is for kim
    let (mut sam_ws, ready) = hello(&app, &sam).await;
    let m = &ready
        .members
        .iter()
        .find(|m| m.server_id == common)
        .unwrap()
        .members;
    assert_eq!(member_user(m, alex_id).avatar_hash, None);
    let (mut kim_ws, ready) = hello(&app, &kim).await;
    let m = &ready
        .members
        .iter()
        .find(|m| m.server_id == common)
        .unwrap()
        .members;
    assert!(member_user(m, alex_id).avatar_hash.is_some());
    let list: Vec<Member> = get_json(&app, &sam, &format!("/servers/{common}/members")).await;
    assert_eq!(member_user(&list, alex_id).avatar_hash, None);
    let list: Vec<Member> = get_json(&app, &kim, &format!("/servers/{common}/members")).await;
    assert!(member_user(&list, alex_id).avatar_hash.is_some());
    drain(&mut sam_ws, 200).await;
    drain(&mut kim_ws, 200).await;

    // a change: alex's nickname
    let r = app
        .http
        .put(app.url(&format!("/servers/{common}/members/{alex_id}/nickname")))
        .bearer_auth(&alex)
        .json(&json!({ "nickname": "Al" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let updated = |events: Vec<Event>| {
        events.into_iter().find_map(|e| match e {
            Event::MemberUpdated { member, .. } if member.user.id == alex_id => Some(member),
            _ => None,
        })
    };
    let to_sam = updated(drain(&mut sam_ws, 300).await).unwrap();
    assert_eq!(to_sam.nickname.as_deref(), Some("Al"));
    assert_eq!(to_sam.user.avatar_hash, None);
    assert!(
        updated(drain(&mut kim_ws, 300).await)
            .unwrap()
            .user
            .avatar_hash
            .is_some()
    );

    // a join: alex joins kim's server with sam in it
    let kims = create_server(&app, &kim, "Kim's").await.id;
    assert_eq!(join_server(&app, &sam, kims).await.status(), 200);
    drain(&mut sam_ws, 200).await;
    assert_eq!(join_server(&app, &alex, kims).await.status(), 200);
    let joined = drain(&mut sam_ws, 300)
        .await
        .into_iter()
        .find_map(|e| match e {
            Event::MemberJoined { member, .. } if member.user.id == alex_id => Some(member),
            _ => None,
        });
    assert_eq!(joined.unwrap().user.avatar_hash, None);

    // a joined server's snapshot: sam joins one alex is already in
    let other = create_server(&app, &kim, "Other").await.id;
    assert_eq!(join_server(&app, &alex, other).await.status(), 200);
    drain(&mut sam_ws, 200).await;
    assert_eq!(join_server(&app, &sam, other).await.status(), 200);
    let snapshot = drain(&mut sam_ws, 300)
        .await
        .into_iter()
        .find_map(|e| match e {
            Event::ServerJoined { snapshot, .. } => Some(snapshot),
            _ => None,
        });
    assert_eq!(
        member_user(&snapshot.unwrap().members, alex_id).avatar_hash,
        None
    );
}

#[tokio::test]
async fn reactions_say_who_reacted() {
    // #508: so a client can leave the people it hides out of the count
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (kim_id, kim) = register(&app, "kim").await;
    let s = acquaint(&app, &alex, &[&sam, &kim]).await;
    let chat = general(&app, &alex, s).await.id;
    let m = send(&app, &alex, chat, "hello").await;
    for t in [&sam, &kim] {
        let r = app
            .http
            .put(app.url(&format!("/messages/{}/reactions/👍", m.id)))
            .bearer_auth(t)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 204);
    }
    let list: Vec<Message> = get_json(&app, &alex, &format!("/channels/{chat}/messages")).await;
    let r = &list.iter().find(|x| x.id == m.id).unwrap().reactions[0];
    assert_eq!(r.count, 2);
    let mut users = r.users.clone().unwrap();
    users.sort();
    let mut want = vec![sam_id, kim_id];
    want.sort();
    assert_eq!(users, want);
}

#[tokio::test]
async fn nothing_of_theirs_reopens_a_dm_you_closed() {
    // #508: their call, poll, plan, spin or message in your DM stays withheld and leaves it closed; yours are refused
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let dm = create_dm(&app, &sam, &[alex_id]).await.id;
    send(&app, &sam, dm, "before").await;
    let r = post_json(&app, &alex, &format!("/channels/{dm}/close"), json!({})).await;
    assert!(r.status().is_success(), "{}", r.status());
    assert_eq!(block(&app, &alex, sam_id, "line").await.status(), 200);
    let (mut alex_ws, _) = hello(&app, &alex).await;
    drain(&mut alex_ws, 200).await;

    let starts =
        pulse_server::times::rfc3339(jiff::Timestamp::now() + pulse_server::times::days(3));
    let posts = [
        (format!("/calls/{dm}"), json!({})),
        (
            format!("/channels/{dm}/polls"),
            json!({"question": "Pizza?", "options": ["yes", "no"], "ends_in": "day"}),
        ),
        (
            format!("/channels/{dm}/plans"),
            json!({"title": "Braai", "starts_at": starts}),
        ),
        (
            format!("/channels/{dm}/wheel-spins"),
            json!({"options": ["a", "b"]}),
        ),
        (
            format!("/channels/{dm}/messages"),
            json!({"content": "hello?"}),
        ),
    ];
    for (path, body) in &posts {
        let r = post_json(&app, &sam, path, body.clone()).await;
        assert!(r.status().is_success(), "{path}: {}", r.status());
    }
    let heard = drain(&mut alex_ws, 400).await;
    assert!(created(&heard).is_empty(), "{:?}", created(&heard));
    assert!(!heard.iter().any(|e| matches!(
        e,
        Event::ConversationVisibility { hidden: false, .. } | Event::CallStarted { .. }
    )));
    let (_ws, ready) = hello(&app, &alex).await;
    assert!(ready.hidden.contains(&dm), "still closed for alex");
    assert_eq!(history(&app, &alex, dm).await, vec!["before"]);
    // sam sees all of it as posted
    assert_eq!(history(&app, &sam, dm).await.len(), 6);

    // alex can't post a poll, plan or spin to someone they've blocked, as with a message
    for (path, body) in &posts[1..] {
        let r = post_json(&app, &alex, path, body.clone()).await;
        assert_eq!(r.status(), 403, "{path}");
    }
}
