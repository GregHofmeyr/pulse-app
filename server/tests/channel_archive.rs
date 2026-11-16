//! Archiving a text channel (#303): the server owner's; read-only while archived (posting, reacting, editing and the
//! rest are refused), still readable and searchable, left out of layout saves; unarchived, it takes messages again.
mod common;

use std::time::Duration;

use common::*;
use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{Channel, Message, SearchResults, ServerLayout};
use serde_json::json;
use tokio::sync::mpsc;

/// The next `ChannelUpdated` `rx` gets; panics after 3 s.
async fn updated(rx: &mut mpsc::Receiver<ServerFrame>) -> Channel {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let f = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("an update arrives")
            .expect("hub open");
        if let ServerFrame::Event(Event::ChannelUpdated { channel }) = f {
            return channel;
        }
    }
}

struct Fx {
    app: TestApp,
    alex: (UserId, String),
    sam: (UserId, String),
    server: ServerId,
    general: Channel,
}

async fn setup() -> Fx {
    let app = spawn().await;
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let s = create_server(&app, &alex.1, "Main").await;
    post_json(&app, &sam.1, &format!("/servers/{}/join", s.id), json!({})).await;
    let general = general(&app, &alex.1, s.id).await;
    Fx {
        app,
        alex,
        sam,
        server: s.id,
        general,
    }
}

async fn set(f: &Fx, token: &str, ch: &Channel, on: bool) -> reqwest::Response {
    let url = f.app.url(&format!("/channels/{}/archive", ch.id));
    let req = if on {
        f.app.http.put(url)
    } else {
        f.app.http.delete(url)
    };
    req.bearer_auth(token).send().await.unwrap()
}

async fn status(r: reqwest::RequestBuilder, token: &str) -> u16 {
    r.bearer_auth(token).send().await.unwrap().status().as_u16()
}

#[tokio::test]
async fn only_the_owner_archives_and_only_text_channels() {
    let f = setup().await;
    let r = set(&f, &f.sam.1, &f.general, true).await;
    assert_eq!(r.status(), 403);
    let chans: Vec<Channel> = get_json(
        &f.app,
        &f.alex.1,
        &format!("/servers/{}/channels", f.server),
    )
    .await;
    let lounge = chans
        .iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    assert_eq!(set(&f, &f.alex.1, lounge, true).await.status(), 400);
    let r = set(&f, &f.alex.1, &f.general, true).await;
    assert_eq!(r.status(), 200);
    let ch: Channel = r.json().await.unwrap();
    assert!(ch.archived_at.is_some());
    // twice is the same; sam can't undo it either
    assert_eq!(set(&f, &f.alex.1, &f.general, true).await.status(), 200);
    assert_eq!(set(&f, &f.sam.1, &f.general, false).await.status(), 403);
}

#[tokio::test]
async fn an_archived_channel_is_read_only_but_readable_and_searchable() {
    let f = setup().await;
    let g = f.general.id;
    let mine = send(&f.app, &f.sam.1, g, "pizza on friday").await;
    let mut rx = f.app.hub.register(f.sam.0, String::new()).rx;
    assert_eq!(set(&f, &f.alex.1, &f.general, true).await.status(), 200);
    let ch = updated(&mut rx).await;
    assert_eq!(ch.id, g);
    assert!(ch.archived_at.is_some(), "everyone hears it archived");

    // posting (the owner too), editing and reacting are refused, in words the client shows
    for who in [&f.sam.1, &f.alex.1] {
        let r = post_json(
            &f.app,
            who,
            &format!("/channels/{g}/messages"),
            json!({"content": "hello?"}),
        )
        .await;
        assert_eq!(r.status(), 403);
        let body: serde_json::Value = r.json().await.unwrap();
        assert!(
            body["message"].as_str().unwrap().contains("archived"),
            "{body}"
        );
    }
    let edit = f
        .app
        .http
        .patch(f.app.url(&format!("/messages/{}", mine.id)))
        .json(&json!({"content": "edited"}));
    assert_eq!(status(edit, &f.sam.1).await, 403);
    let react = f
        .app
        .http
        .put(f.app.url(&format!("/messages/{}/reactions/👍", mine.id)));
    assert_eq!(status(react, &f.alex.1).await, 403);
    let poll = post_json(
        &f.app,
        &f.sam.1,
        &format!("/channels/{g}/polls"),
        json!({"question": "pizza?", "options": ["yes", "no"], "ends_in": "day"}),
    )
    .await;
    assert_eq!(poll.status(), 403);

    // still readable and searchable
    let history: Vec<Message> =
        get_json(&f.app, &f.alex.1, &format!("/channels/{g}/messages")).await;
    assert!(history.iter().any(|m| m.id == mine.id));
    let hits: SearchResults = get_json(
        &f.app,
        &f.alex.1,
        &format!("/servers/{}/search?q=pizza", f.server),
    )
    .await;
    assert!(hits.hits.iter().any(|h| h.message.id == mine.id));

    // unarchived: it takes messages again, at the bottom of its category
    let r = set(&f, &f.alex.1, &f.general, false).await;
    assert_eq!(r.status(), 200);
    let back: Channel = r.json().await.unwrap();
    assert!(back.archived_at.is_none());
    assert!(!updated(&mut rx).await.archived_at.is_some());
    send(&f.app, &f.sam.1, g, "we're back").await;
}

#[tokio::test]
async fn a_layout_save_leaves_archived_channels_out() {
    let f = setup().await;
    let made = post_json(
        &f.app,
        &f.alex.1,
        &format!("/servers/{}/channels", f.server),
        json!({"kind": "text", "name": "old-news"}),
    )
    .await;
    let old: Channel = made.json().await.unwrap();
    assert_eq!(set(&f, &f.alex.1, &old, true).await.status(), 200);
    let base: ServerLayout = {
        let mut c = f.app.db.acquire().await.unwrap();
        pulse_server::servers::categories::layout_of(&mut c, f.server)
            .await
            .unwrap()
    };
    let cats: Vec<serde_json::Value> = base
        .categories
        .iter()
        .map(|cat| {
            json!({
                "id": cat.id,
                "channels": base.channels.iter()
                    .filter(|p| p.category_id == Some(cat.id) && p.channel_id != old.id)
                    .map(|p| p.channel_id).collect::<Vec<_>>(),
            })
        })
        .collect();
    // today's clients don't name it...
    let r = f
        .app
        .http
        .put(f.app.url(&format!("/servers/{}/layout", f.server)))
        .bearer_auth(&f.sam.1)
        .json(&json!({"revision": base.revision, "uncategorised": [], "categories": cats}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    // ...and one from before archiving, that does, is still taken
    let saved: ServerLayout = {
        let mut c = f.app.db.acquire().await.unwrap();
        pulse_server::servers::categories::layout_of(&mut c, f.server)
            .await
            .unwrap()
    };
    let r = f
        .app
        .http
        .put(f.app.url(&format!("/servers/{}/layout", f.server)))
        .bearer_auth(&f.sam.1)
        .json(&json!({"revision": saved.revision, "uncategorised": [old.id], "categories": cats}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
}
