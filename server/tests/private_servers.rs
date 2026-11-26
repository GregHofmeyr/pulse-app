//! Private servers (#80, step 2): someone not in a server gets nothing of it over the gateway (no events, nothing in
//! Ready, not even its members' presence) until they accept an invite, and then `ServerJoined` brings it whole.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::rest::{Channel, Message, ServerInvite};
use pulse_server::gateway::audience::{Audience, audience_for};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello(app: &TestApp, token: &str) -> (Socket, Ready) {
    hello_as(app, token, pulse_protocol::PROTOCOL_VERSION).await
}

/// [`hello`] as a client speaking protocol `version`.
async fn hello_as(app: &TestApp, token: &str, version: u32) -> (Socket, Ready) {
    let mut ws = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap()
        .0;
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: version,
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    loop {
        if let Some((_, ServerFrame::Ready(r))) = next(&mut ws).await {
            return (ws, r);
        }
    }
}

/// The next frame, with its raw text; `None` after 3 s of quiet or a close.
async fn next(ws: &mut Socket) -> Option<(String, ServerFrame)> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        match m {
            Ws::Text(t) => {
                let f: ServerFrame = serde_json::from_str(&t).unwrap();
                return Some((t.to_string(), f));
            }
            Ws::Close(_) => return None,
            _ => continue,
        }
    }
}

/// Every event frame until `fence` picks one (that one included), as raw text and parsed.
async fn until(ws: &mut Socket, fence: impl Fn(&Event) -> bool) -> Vec<(String, Event)> {
    let mut out = vec![];
    loop {
        let (raw, f) = next(ws).await.expect("the fence never came");
        if let ServerFrame::Event(e) = f {
            let done = fence(&e);
            out.push((raw, e));
            if done {
                return out;
            }
        }
    }
}

/// alex owns "Main" (sam is in it); jo is in nothing. All three are connected.
struct World {
    app: TestApp,
    alex: (pulse_protocol::ids::UserId, String),
    sam: (pulse_protocol::ids::UserId, String),
    jo: (pulse_protocol::ids::UserId, String),
    server: pulse_protocol::ids::ServerId,
    general: Channel,
}

async fn world() -> World {
    let app = spawn().await;
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let jo = register(&app, "jo").await;
    let server = create_server(&app, &alex.1, "Main").await.id;
    assert_eq!(join_server(&app, &sam.1, server).await.status(), 200);
    let general = general(&app, &alex.1, server).await;
    World {
        app,
        alex,
        sam,
        jo,
        server,
        general,
    }
}

#[tokio::test]
async fn an_outsider_gets_nothing_of_a_server_until_an_invite_brings_it_whole() {
    let w = world().await;
    let (alex_ws, _) = hello(&w.app, &w.alex.1).await;
    let (mut sam_ws, _) = hello(&w.app, &w.sam.1).await;
    let (mut jo_ws, ready) = hello(&w.app, &w.jo.1).await;

    // Ready: none of it, nor its members (who share nothing with jo, #486)
    assert!(ready.servers.is_empty(), "{:?}", ready.servers);
    assert!(ready.members.is_empty());
    assert!(ready.layouts.is_empty());
    assert!(ready.channels.iter().all(|c| c.server_id.is_none()));
    assert!(
        ready
            .people
            .iter()
            .all(|p| p.user.id != w.alex.0 && p.user.id != w.sam.0),
        "{:?}",
        ready.people
    );

    // the members chat, react, type, pin, make a channel and come and go
    let m = send(&w.app, &w.alex.1, w.general.id, "hello in here").await;
    let r = w
        .app
        .http
        .put(
            w.app
                .url(&format!("/messages/{}/reactions/%F0%9F%91%8D", m.id)),
        )
        .bearer_auth(&w.sam.1)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    let typing = serde_json::to_string(&ClientFrame::Typing {
        channel_id: w.general.id,
    })
    .unwrap();
    sam_ws.send(Ws::text(typing)).await.unwrap();
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/servers/{}/channels", w.server),
        json!({ "kind": "text", "name": "secret-plans" }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/servers/{}/leave", w.server),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 204);
    assert_eq!(join_server(&w.app, &w.sam.1, w.server).await.status(), 200);
    // the fence: jo makes a server of their own, which only they hear about
    let mine = create_server(&w.app, &w.jo.1, "Jo's").await.id;
    let seen = until(
        &mut jo_ws,
        |e| matches!(e, Event::ServerCreated { server } if server.id == mine),
    )
    .await;
    let names = [
        w.server.to_string(),
        w.general.id.to_string(),
        m.id.to_string(),
        w.alex.0.to_string(),
        w.sam.0.to_string(),
    ];
    // not even a stranger's presence (#486): they're nobody jo has
    for (raw, e) in &seen {
        assert!(
            !names.iter().any(|n| raw.contains(n.as_str())),
            "an outsider got {e:?}"
        );
    }
    // nobody else heard of jo's server
    assert_eq!(
        audience_for(
            &w.app.db,
            &Event::ServerCreated {
                server: seen
                    .last()
                    .map(|(_, e)| match e {
                        Event::ServerCreated { server } => server.clone(),
                        _ => unreachable!(),
                    })
                    .unwrap()
            }
        )
        .await
        .unwrap(),
        Audience::Users([w.jo.0].into_iter().collect())
    );
    let mut alex_rx = w.app.hub.register(w.alex.0, String::new()).rx;
    drop(sam_ws);

    // an invite link (not a direct invite: that would make them a DM, and share it): jo joins, and the server arrives
    // whole before anything in it
    let inv: ServerInvite = post_json(
        &w.app,
        &w.alex.1,
        &format!("/servers/{}/invites", w.server),
        json!({}),
    )
    .await
    .json()
    .await
    .unwrap();
    let r = post_json(
        &w.app,
        &w.jo.1,
        &format!("/invites/{}/accept", inv.code),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let seen = until(&mut jo_ws, |e| matches!(e, Event::ServerJoined { .. })).await;
    let Some((
        _,
        Event::ServerJoined {
            user_id, snapshot, ..
        },
    )) = seen.last()
    else {
        unreachable!()
    };
    assert_eq!(*user_id, w.jo.0);
    assert_eq!(snapshot.server.id, w.server);
    let mut names: Vec<_> = snapshot
        .channels
        .iter()
        .filter_map(|c| c.name.clone())
        .collect();
    names.sort();
    assert_eq!(names, ["Lounge", "general", "secret-plans"]);
    assert!(snapshot.members.iter().any(|m| m.user.id == w.jo.0));
    assert!(snapshot.members.iter().any(|m| m.user.id == w.alex.0));
    // from now on what happens in it reaches them, and they see its members' presence
    let m2 = send(&w.app, &w.alex.1, w.general.id, "welcome").await;
    let seen = until(
        &mut jo_ws,
        |e| matches!(e, Event::MessageCreated { message, .. } if message.id == m2.id),
    )
    .await;
    assert!(
        seen.iter().any(|(_, e)| matches!(
            e,
            Event::PresenceChanged { user_id, online: true, .. } if *user_id == w.alex.0
        )),
        "alex's presence reaches jo once they share a server: {seen:?}"
    );
    // and alex sees jo come online for them too
    let at = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(ServerFrame::Event(Event::PresenceChanged {
                user_id,
                online: true,
                ..
            })) = alex_rx.recv().await
                && user_id == w.jo.0
            {
                return;
            }
        }
    })
    .await;
    assert!(at.is_ok(), "jo's presence reaches alex");
    drop(alex_ws);

    // a fresh Ready lists it now
    let (_, ready) = hello(&w.app, &w.jo.1).await;
    let mut ids: Vec<_> = ready.servers.iter().map(|s| s.id).collect();
    ids.sort();
    let mut want = vec![w.server, mine];
    want.sort();
    assert_eq!(ids, want);
}

#[tokio::test]
async fn someone_removed_hears_it_and_then_nothing_more() {
    let w = world().await;
    let (mut sam_ws, _) = hello(&w.app, &w.sam.1).await;
    let r = w
        .app
        .http
        .delete(
            w.app
                .url(&format!("/servers/{}/members/{}", w.server, w.sam.0)),
        )
        .bearer_auth(&w.alex.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    until(&mut sam_ws, |e| {
        matches!(e, Event::MemberLeft { user_id, server_id, .. } if *user_id == w.sam.0 && *server_id == w.server)
    })
    .await;
    let after = send(&w.app, &w.alex.1, w.general.id, "they're gone").await;
    // the fence: a DM to sam
    let dm = create_dm(&w.app, &w.jo.1, &[w.sam.0]).await;
    let note: Message = send(&w.app, &w.jo.1, dm.id, "hi sam").await;
    let seen = until(
        &mut sam_ws,
        |e| matches!(e, Event::MessageCreated { message, .. } if message.id == note.id),
    )
    .await;
    for (raw, e) in &seen {
        assert!(
            !raw.contains(&after.id.to_string()) && !raw.contains(&w.server.to_string()),
            "someone removed got {e:?}"
        );
    }
}

#[tokio::test]
async fn presence_reaches_only_people_who_share_something() {
    let w = world().await;
    let (mut jo_ws, ready) = hello(&w.app, &w.jo.1).await;
    assert!(ready.people.iter().all(|p| p.user.id != w.alex.0));
    // alex comes online: jo, who shares nothing with them, isn't told
    let (_alex_ws, _) = hello(&w.app, &w.alex.1).await;
    // a DM between them: now they know each other, and jo sees alex online
    create_dm(&w.app, &w.alex.1, &[w.jo.0]).await;
    let seen = until(
        &mut jo_ws,
        |e| matches!(e, Event::PresenceChanged { user_id, .. } if *user_id == w.alex.0),
    )
    .await;
    let Some((_, Event::PresenceChanged { online, .. })) = seen.last() else {
        unreachable!()
    };
    assert!(online, "the first word about alex is that they're online");
}

/// alex saves a profile with `bio`.
async fn save_bio(app: &TestApp, token: &str, bio: &str) {
    let r = app
        .http
        .put(app.url("/me/profile"))
        .bearer_auth(token)
        .json(&json!({
            "theme_top": "#8a3cff",
            "theme_bottom": "#ff8a2a",
            "style": { "font": null, "colors": [] },
            "bio": bio,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
}

/// The fence for a socket: a DM `from` opens with `to`, and the message in it.
async fn fence(app: &TestApp, from: &str, to: pulse_protocol::ids::UserId) -> Message {
    let dm = create_dm(app, from, &[to]).await;
    send(app, from, dm.id, "fence").await
}

#[tokio::test]
async fn people_and_their_profiles_reach_only_who_shares_something() {
    let w = world().await;
    let kim = register(&w.app, "kim").await;
    // kim and jo know each other (a DM), and nobody else
    create_dm(&w.app, &kim.1, &[w.jo.0]).await;

    // Ready's people: you and who you share something with (#486)
    let (_, ready) = hello(&w.app, &w.jo.1).await;
    let mut ids: Vec<_> = ready.people.iter().map(|p| p.user.id).collect();
    ids.sort();
    let mut want = vec![w.jo.0, kim.0];
    want.sort();
    assert_eq!(ids, want);
    let (mut sam_ws, ready) = hello(&w.app, &w.sam.1).await;
    let mut ids: Vec<_> = ready.people.iter().map(|p| p.user.id).collect();
    ids.sort();
    let mut want = vec![w.alex.0, w.sam.0];
    want.sort();
    assert_eq!(ids, want);
    let (mut jo_ws, _) = hello(&w.app, &w.jo.1).await;

    // alex's profile change reaches sam (a server in common), not jo
    save_bio(&w.app, &w.alex.1, "first").await;
    let seen = until(
        &mut sam_ws,
        |e| matches!(e, Event::ProfileUpdated { user, .. } if user.id == w.alex.0),
    )
    .await;
    assert!(!seen.is_empty());
    let note = fence(&w.app, &kim.1, w.jo.0).await;
    let seen = until(
        &mut jo_ws,
        |e| matches!(e, Event::MessageCreated { message, .. } if message.id == note.id),
    )
    .await;
    let alex = w.alex.0.to_string();
    for (raw, e) in &seen {
        assert!(!raw.contains(&alex), "jo heard of alex: {e:?}");
    }
    // a new account is told to nobody it shares nothing with
    let new = Event::UserCreated {
        user: pulse_protocol::rest::User {
            id: kim.0,
            username: "kim".into(),
            display_name: "kim".into(),
            avatar_hash: None,
            avatar_still: None,
            style: Default::default(),
            typing_text: None,
            ring_style: None,
            ring_color: None,
        },
    };
    assert_eq!(
        audience_for(&w.app.db, &new).await.unwrap(),
        Audience::Users([kim.0, w.jo.0].into_iter().collect())
    );

    // a DM with alex: jo gets alex's profile, then their presence (#486)
    let (_alex_ws, _) = hello(&w.app, &w.alex.1).await;
    create_dm(&w.app, &w.alex.1, &[w.jo.0]).await;
    let seen = until(
        &mut jo_ws,
        |e| matches!(e, Event::PresenceChanged { user_id, .. } if *user_id == w.alex.0),
    )
    .await;
    let intro = seen
        .iter()
        .position(|(_, e)| matches!(e, Event::UserCreated { user } if user.id == w.alex.0))
        .expect("alex's profile reaches jo once they share a DM");
    assert!(
        intro < seen.len() - 1,
        "the profile comes before the presence"
    );
    save_bio(&w.app, &w.alex.1, "second").await;
    until(
        &mut jo_ws,
        |e| matches!(e, Event::ProfileUpdated { user, .. } if user.id == w.alex.0),
    )
    .await;

    // a group with sam: sam and kim meet the same way
    let group = create_dm(&w.app, &w.sam.1, &[w.jo.0, kim.0]).await;
    until(
        &mut sam_ws,
        |e| matches!(e, Event::UserCreated { user } if user.id == kim.0),
    )
    .await;
    // sam leaves the group: kim's profile changes no longer reach sam
    let r = w
        .app
        .http
        .delete(
            w.app
                .url(&format!("/channels/{}/members/{}", group.id, w.sam.0)),
        )
        .bearer_auth(&w.sam.1)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    save_bio(&w.app, &kim.1, "after").await;
    let note = fence(&w.app, &w.alex.1, w.sam.0).await;
    let seen = until(
        &mut sam_ws,
        |e| matches!(e, Event::MessageCreated { message, .. } if message.id == note.id),
    )
    .await;
    assert!(
        !seen
            .iter()
            .any(|(_, e)| matches!(e, Event::ProfileUpdated { user, .. } if user.id == kim.0)),
        "{seen:?}"
    );
    // and a fresh Ready leaves kim out
    let (_, ready) = hello(&w.app, &w.sam.1).await;
    assert!(ready.people.iter().all(|p| p.user.id != kim.0));
}
