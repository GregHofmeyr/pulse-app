//! Build favourites (#617): each person's own stars on builds, told only to their own sessions.
mod common;

use std::time::Duration;

use common::*;
use futures::SinkExt;
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{BuildsBatchResult, Channel, Message};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;

/// alex owns the server; sam is in it; kim isn't.
struct World {
    app: TestApp,
    alex: (UserId, String),
    sam: (UserId, String),
    kim: String,
    server: ServerId,
    ch: ChannelId,
}

async fn world() -> World {
    let app = spawn().await;
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    let s = create_server(&app, &alex.1, "Main").await;
    assert_eq!(join_server(&app, &sam.1, s.id).await.status(), 200);
    let ch = builds_channel(&app, &alex.1, s.id, "builds").await;
    World {
        app,
        alex,
        sam,
        kim,
        server: s.id,
        ch,
    }
}

async fn builds_channel(app: &TestApp, token: &str, server: ServerId, name: &str) -> ChannelId {
    let r = post_json(
        app,
        token,
        &format!("/servers/{server}/channels"),
        json!({ "kind": "builds", "name": name }),
    )
    .await;
    r.json::<Channel>().await.unwrap().id
}

impl World {
    async fn build_in(&self, ch: ChannelId, token: &str, title: &str) -> Message {
        let r = post_json(
            &self.app,
            token,
            &format!("/channels/{ch}/builds"),
            json!({ "title": title }),
        )
        .await;
        assert_eq!(r.status(), 200);
        r.json().await.unwrap()
    }

    async fn build(&self, token: &str, title: &str) -> Message {
        self.build_in(self.ch, token, title).await
    }

    async fn star(&self, token: &str, id: MessageId, on: bool) -> reqwest::Response {
        let url = self.app.url(&format!("/builds/{id}/favourite"));
        let r = if on {
            self.app.http.put(url)
        } else {
            self.app.http.delete(url)
        };
        r.bearer_auth(token).send().await.unwrap()
    }

    async fn star_many(&self, token: &str, ids: &[MessageId], on: bool) -> reqwest::Response {
        post_json(
            &self.app,
            token,
            &format!("/channels/{}/builds/favourite-many", self.ch),
            json!({ "message_ids": ids, "favourite": on }),
        )
        .await
    }

    async fn starred(&self, token: &str, id: MessageId) -> bool {
        let m: Message = get_json(&self.app, token, &format!("/builds/{id}")).await;
        m.build.unwrap().favourite
    }

    async fn listed(&self, token: &str) -> Vec<(MessageId, bool)> {
        let v: Vec<Message> =
            get_json(&self.app, token, &format!("/channels/{}/builds", self.ch)).await;
        v.into_iter()
            .map(|m| (m.id, m.build.unwrap().favourite))
            .collect()
    }
}

async fn hello_as(app: &TestApp, token: &str, version: u32) -> Socket {
    let (mut ws, _) = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap();
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: version,
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

async fn hello(app: &TestApp, token: &str) -> Socket {
    hello_as(app, token, pulse_protocol::PROTOCOL_VERSION).await
}

/// Every event up to the message `sentinel` (sent after what's being checked).
async fn events_until(ws: &mut Socket, sentinel: &str) -> Vec<Event> {
    let mut out = vec![];
    loop {
        match next_frame_beating(ws, Duration::from_secs(3)).await {
            Some(ServerFrame::Event(Event::MessageCreated { message, .. }))
                if message.content == sentinel =>
            {
                return out;
            }
            Some(ServerFrame::Event(e)) => out.push(e),
            Some(_) => continue,
            None => panic!("expected the sentinel {sentinel:?}"),
        }
    }
}

fn starred_events(events: &[Event]) -> Vec<(Vec<MessageId>, bool)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::BuildFavouriteChanged {
                message_ids,
                favourite,
                ..
            } => Some((message_ids.clone(), *favourite)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_member_stars_a_build_and_an_outsider_cant() {
    let w = world().await;
    let m = w.build(&w.alex.1, "Raven crit").await;
    assert_eq!(w.star(&w.kim, m.id, true).await.status(), 404);
    assert!(!w.starred(&w.sam.1, m.id).await);
    assert_eq!(w.star(&w.sam.1, m.id, true).await.status(), 204);
    assert!(w.starred(&w.sam.1, m.id).await);
    // again is fine
    assert_eq!(w.star(&w.sam.1, m.id, true).await.status(), 204);
    assert_eq!(w.listed(&w.sam.1).await, vec![(m.id, true)]);
    assert_eq!(w.star(&w.sam.1, m.id, false).await.status(), 204);
    assert!(!w.starred(&w.sam.1, m.id).await);
    // an archived build can be starred too; a deleted one can't
    let r = w
        .app
        .http
        .put(w.app.url(&format!("/builds/{}/archived", m.id)))
        .bearer_auth(&w.alex.1)
        .json(&json!({ "archived": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(w.star(&w.sam.1, m.id, true).await.status(), 204);
    let r = w
        .app
        .http
        .delete(w.app.url(&format!("/builds/{}", m.id)))
        .bearer_auth(&w.alex.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(w.star(&w.sam.1, m.id, true).await.status(), 404);
}

/// Favourites are personal: alex's star isn't sam's, and only alex's own sessions hear of it.
#[tokio::test]
async fn favourites_are_personal() {
    let w = world().await;
    let text = general(&w.app, &w.alex.1, w.server).await;
    let a = w.build(&w.sam.1, "A").await;
    let b = w.build(&w.sam.1, "B").await;
    let mut alex_other = hello(&w.app, &w.alex.1).await;
    let mut sams = hello(&w.app, &w.sam.1).await;
    assert_eq!(w.star(&w.alex.1, a.id, true).await.status(), 204);
    assert!(w.starred(&w.alex.1, a.id).await);
    assert!(
        !w.starred(&w.sam.1, a.id).await,
        "sam doesn't see alex's star"
    );
    assert_eq!(w.listed(&w.alex.1).await, vec![(b.id, false), (a.id, true)]);
    assert_eq!(w.listed(&w.sam.1).await, vec![(b.id, false), (a.id, false)]);
    send(&w.app, &w.alex.1, text.id, "after").await;
    assert_eq!(
        starred_events(&events_until(&mut alex_other, "after").await),
        vec![(vec![a.id], true)]
    );
    assert!(starred_events(&events_until(&mut sams, "after").await).is_empty());
    // a build's events carry no one's star
    let r = w
        .app
        .http
        .put(w.app.url(&format!("/builds/{}", a.id)))
        .bearer_auth(&w.sam.1)
        .json(&json!({ "title": "A2" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    loop {
        match next_frame_beating(&mut alex_other, Duration::from_secs(3)).await {
            Some(ServerFrame::Event(Event::MessageUpdated { message })) if message.id == a.id => {
                assert!(!message.build.unwrap().favourite);
                break;
            }
            Some(_) => continue,
            None => panic!("expected MessageUpdated"),
        }
    }
}

#[tokio::test]
async fn a_star_goes_with_its_build() {
    let w = world().await;
    let m = w.build(&w.sam.1, "A").await;
    let keep = w.build(&w.sam.1, "B").await;
    for id in [m.id, keep.id] {
        assert_eq!(w.star(&w.alex.1, id, true).await.status(), 204);
    }
    assert_eq!(w.star(&w.sam.1, m.id, true).await.status(), 204);
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/channels/{}/builds/delete-many", w.ch),
        json!({ "message_ids": [m.id] }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let rows = count(
        &w.app,
        &format!(
            "SELECT COUNT(*) FROM build_favourites WHERE message_id = '{}'",
            m.id
        ),
    )
    .await;
    assert_eq!(rows, 0, "deleting the build drops everyone's star");
    assert_eq!(
        count(&w.app, "SELECT COUNT(*) FROM build_favourites").await,
        1
    );
    // and a purged message takes it too
    sqlx::query("DELETE FROM messages WHERE id = ?")
        .bind(keep.id.to_string())
        .execute(&w.app.db)
        .await
        .unwrap();
    assert_eq!(
        count(&w.app, "SELECT COUNT(*) FROM build_favourites").await,
        0
    );
}

#[tokio::test]
async fn favourite_many_stars_the_channels_builds_up_to_100() {
    let w = world().await;
    let a = w.build(&w.alex.1, "A").await;
    let b = w.build(&w.sam.1, "B").await;
    let other = builds_channel(&w.app, &w.alex.1, w.server, "more").await;
    let x = w.build_in(other, &w.alex.1, "Elsewhere").await;
    assert_eq!(w.star_many(&w.kim, &[a.id], true).await.status(), 404);
    let mut sams = hello(&w.app, &w.sam.1).await;
    // anyone's builds (a star is yours, not the build's); another channel's or an unknown id is skipped
    let r = w
        .star_many(&w.sam.1, &[a.id, b.id, x.id, MessageId::new()], true)
        .await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<BuildsBatchResult>().await.unwrap(),
        BuildsBatchResult {
            done: 2,
            skipped: 2
        }
    );
    assert!(!w.starred(&w.sam.1, x.id).await);
    let got = loop {
        match next_frame_beating(&mut sams, Duration::from_secs(3)).await {
            Some(ServerFrame::Event(Event::BuildFavouriteChanged {
                message_ids,
                favourite,
                ..
            })) => break (message_ids, favourite),
            Some(_) => continue,
            None => panic!("expected BuildFavouriteChanged"),
        }
    };
    let mut ids = got.0;
    ids.sort();
    let mut want = vec![a.id, b.id];
    want.sort();
    assert_eq!((ids, got.1), (want, true), "one event for the batch");
    assert_eq!(w.listed(&w.sam.1).await, vec![(b.id, true), (a.id, true)]);
    let r = w.star_many(&w.sam.1, &[a.id], false).await;
    assert_eq!(
        r.json::<BuildsBatchResult>().await.unwrap(),
        BuildsBatchResult {
            done: 1,
            skipped: 0
        }
    );
    assert_eq!(w.listed(&w.sam.1).await, vec![(b.id, true), (a.id, false)]);
    let many: Vec<MessageId> = (0..101).map(|_| MessageId::new()).collect();
    assert_eq!(w.star_many(&w.sam.1, &many, true).await.status(), 400);
}

/// A version-4 client hears nothing of a star, its own included (#616).
#[tokio::test]
async fn a_version_4_client_never_hears_a_star() {
    let w = world().await;
    let text = general(&w.app, &w.alex.1, w.server).await;
    let a = w.build(&w.alex.1, "A").await;
    let mut old = hello_as(&w.app, &w.sam.1, 4).await;
    assert_eq!(w.star(&w.sam.1, a.id, true).await.status(), 204);
    assert_eq!(w.star_many(&w.sam.1, &[a.id], false).await.status(), 200);
    send(&w.app, &w.alex.1, text.id, "after").await;
    let got = events_until(&mut old, "after").await;
    let id = w.ch.to_string();
    assert!(
        !got.iter()
            .any(|e| serde_json::to_string(e).unwrap().contains(&id)),
        "{got:?}"
    );
}

/// A pinned build goes out as everyone sees it: the pinner's star (and seen point) stay theirs.
#[tokio::test]
async fn a_pin_carries_no_ones_star() {
    let w = world().await;
    let a = w.build(&w.sam.1, "A").await;
    assert_eq!(w.star(&w.alex.1, a.id, true).await.status(), 204);
    let mut sams = hello(&w.app, &w.sam.1).await;
    let r = w
        .app
        .http
        .put(w.app.url(&format!("/messages/{}/pin", a.id)))
        .bearer_auth(&w.alex.1)
        .json(&json!({ "duration": "forever" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    loop {
        match next_frame_beating(&mut sams, Duration::from_secs(3)).await {
            Some(ServerFrame::Event(Event::MessagePinned { pinned, .. })) => {
                let b = pinned.message.build.unwrap();
                assert!(!b.favourite, "alex's star went out to sam");
                assert_eq!(b.replies_seen, None);
                break;
            }
            Some(_) => continue,
            None => panic!("expected MessagePinned"),
        }
    }
    // and the pins list is each caller's own view
    let pins: Vec<pulse_protocol::rest::PinnedMessage> =
        get_json(&w.app, &w.alex.1, &format!("/channels/{}/pins", w.ch)).await;
    assert!(pins[0].message.build.as_ref().unwrap().favourite);
    let pins: Vec<pulse_protocol::rest::PinnedMessage> =
        get_json(&w.app, &w.sam.1, &format!("/channels/{}/pins", w.ch)).await;
    assert!(!pins[0].message.build.as_ref().unwrap().favourite);
}

/// The copy a pin falls back to when its read-back fails is made neutral the same way.
#[tokio::test]
async fn a_message_made_neutral_keeps_no_ones_view() {
    let w = world().await;
    let a = w.build(&w.sam.1, "A").await;
    assert_eq!(w.star(&w.alex.1, a.id, true).await.status(), 204);
    let mut m: Message = get_json(&w.app, &w.alex.1, &format!("/builds/{}", a.id)).await;
    assert!(m.build.as_ref().unwrap().favourite);
    m.build.as_mut().unwrap().replies_seen = Some(MessageId::new());
    pulse_server::messages::neutral(&mut m);
    let b = m.build.unwrap();
    assert!(!b.favourite);
    assert_eq!(b.replies_seen, None);
}

/// A star that changes nothing (again, or on a build that's gone) tells no one.
#[tokio::test]
async fn a_star_that_changes_nothing_sends_no_event() {
    let w = world().await;
    let text = general(&w.app, &w.alex.1, w.server).await;
    let a = w.build(&w.alex.1, "A").await;
    let mut sams = hello(&w.app, &w.sam.1).await;
    assert_eq!(w.star(&w.sam.1, a.id, true).await.status(), 204);
    assert_eq!(w.star(&w.sam.1, a.id, true).await.status(), 204);
    assert_eq!(w.star(&w.alex.1, a.id, false).await.status(), 204);
    send(&w.app, &w.alex.1, text.id, "after").await;
    assert_eq!(
        starred_events(&events_until(&mut sams, "after").await),
        vec![(vec![a.id], true)]
    );
}

/// `favourite=true` lists only the caller's own starred builds, with the other filters.
#[tokio::test]
async fn the_favourite_filter_lists_your_stars() {
    let w = world().await;
    let a = w.build(&w.alex.1, "Raven crit").await;
    let b = w.build(&w.alex.1, "Ember tank").await;
    let c = w.build(&w.alex.1, "Raven dodge").await;
    for id in [a.id, b.id] {
        assert_eq!(w.star(&w.sam.1, id, true).await.status(), 204);
    }
    assert_eq!(w.star(&w.alex.1, c.id, true).await.status(), 204);
    let ids = |v: Vec<Message>| v.into_iter().map(|m| m.id).collect::<Vec<_>>();
    let base = format!("/channels/{}/builds?favourite=true", w.ch);
    assert_eq!(
        ids(get_json(&w.app, &w.sam.1, &base).await),
        vec![b.id, a.id]
    );
    assert_eq!(ids(get_json(&w.app, &w.alex.1, &base).await), vec![c.id]);
    assert_eq!(
        ids(get_json(&w.app, &w.sam.1, &format!("{base}&q=raven")).await),
        vec![a.id]
    );
    // archived ones only with archived=true, as without the filter
    let r = w
        .app
        .http
        .put(w.app.url(&format!("/builds/{}/archived", a.id)))
        .bearer_auth(&w.alex.1)
        .json(&json!({ "archived": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(ids(get_json(&w.app, &w.sam.1, &base).await), vec![b.id]);
    assert_eq!(
        ids(get_json(&w.app, &w.sam.1, &format!("{base}&archived=true")).await),
        vec![b.id, a.id]
    );
    // false is the plain list
    assert_eq!(
        ids(get_json(
            &w.app,
            &w.sam.1,
            &format!("/channels/{}/builds?favourite=false", w.ch)
        )
        .await),
        vec![c.id, b.id]
    );
}
