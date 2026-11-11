//! Spotify on a voice channel's stage (#234): share, replace, stop, expiry, leaving the call, the chat message
//! (#288), Ready.
mod common;

use std::time::Duration;

use base64::Engine;
use common::*;
use livekit_api::access_token::AccessToken;
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{Channel, ChannelKind, Message, MessageKind, StageShare};
use pulse_server::config::Config;
use pulse_server::testing::fake_provider::{FakeProvider, SPOTIFY_ID, SPOTIFY_JAM_URL};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc::Receiver;

const JAM: &str = "ABC123abc123ABC123abc123";
/// How a share's chat message starts (#288).
const LEAD: &str = "🎵 Shared to the stage: ";

struct World {
    app: TestApp,
    fake: FakeProvider,
    alex: (UserId, String),
    sam: (UserId, String),
    jo: (UserId, String),
    lounge: Channel,
    /// sam's events.
    rx: Receiver<ServerFrame>,
}

/// alex's server (sam joined, jo didn't) with alex and sam in its Lounge call, on an app whose unfurler reads the
/// fake provider.
async fn world_with(cfg: Config) -> World {
    let fake = FakeProvider::start().await;
    let app = spawn_with(Config {
        embeds: fake.config(),
        ..cfg
    })
    .await;
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let jo = register(&app, "jo").await;
    let s = create_server(&app, &alex.1, "Main").await;
    let r = post_json(&app, &sam.1, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);
    let chans: Vec<Channel> = get_json(&app, &alex.1, &format!("/servers/{}/channels", s.id)).await;
    let lounge = chans
        .into_iter()
        .find(|c| c.kind == ChannelKind::Voice)
        .unwrap();
    app.voice.join(lounge.id, alex.0, "PA_alex");
    app.voice.join(lounge.id, sam.0, "PA_sam");
    let rx = app.hub.register(sam.0, String::new()).rx;
    World {
        app,
        fake,
        alex,
        sam,
        jo,
        lounge,
        rx,
    }
}

async fn world() -> World {
    world_with(test_config()).await
}

fn track() -> String {
    format!("https://open.spotify.com/track/{SPOTIFY_ID}?si=abc")
}

async fn share(w: &World, token: &str, url: &str) -> reqwest::Response {
    w.app
        .http
        .put(w.app.url(&format!("/channels/{}/stage-share", w.lounge.id)))
        .bearer_auth(token)
        .json(&json!({ "url": url }))
        .send()
        .await
        .unwrap()
}

async fn stop(w: &World, token: &str, id: &str) -> u16 {
    w.app
        .http
        .delete(
            w.app
                .url(&format!("/channels/{}/stage-share/{id}", w.lounge.id)),
        )
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

/// The next stage event (or share message) on `rx`, skipping everything else.
async fn next(rx: &mut Receiver<ServerFrame>) -> Event {
    loop {
        let f = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("timed out waiting for a stage event")
            .expect("hub closed");
        if let ServerFrame::Event(e) = f {
            match &e {
                Event::StageShared { .. } | Event::StageShareCleared { .. } => return e,
                Event::MessageCreated { message, .. } if message.content.starts_with(LEAD) => {
                    return e;
                }
                _ => {}
            }
        }
    }
}

/// Nothing more from the stage on `rx` for a moment.
async fn quiet(rx: &mut Receiver<ServerFrame>) {
    let wait = tokio::time::timeout(Duration::from_millis(300), next(rx)).await;
    assert!(wait.is_err(), "unexpected {:?}", wait.unwrap());
}

async fn ready_of(app: &TestApp, token: &str) -> Ready {
    use futures::SinkExt;
    use tokio_tungstenite::tungstenite::Message as Ws;
    let (mut ws, _) = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap();
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    loop {
        match next_frame_beating(&mut ws, Duration::from_secs(3)).await {
            Some(ServerFrame::Ready(r)) => return r,
            Some(_) => continue,
            None => panic!("expected Ready"),
        }
    }
}

fn signed(app: &TestApp, body: &str) -> String {
    let sum = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body.as_bytes()));
    AccessToken::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .with_sha256(&sum)
        .to_jwt()
        .unwrap()
}

async fn left(app: &TestApp, room: ChannelId, user: UserId, sid: &str) {
    let body =
        json!({"event": "participant_left", "id": "EV_test", "room": {"name": room.to_string()},
        "participant": {"identity": user.to_string(), "sid": sid}})
        .to_string();
    let r = app
        .http
        .post(app.url("/livekit/webhook"))
        .header("Authorization", signed(app, &body))
        .header("Content-Type", "application/webhook+json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn sharing_a_song_puts_its_card_up_and_posts_the_link_in_chat() {
    let mut w = world().await;
    let r = share(&w, &w.alex.1, &format!("  {}  ", track())).await;
    assert_eq!(r.status(), 200);
    let s: StageShare = r.json().await.unwrap();
    assert_eq!(s.channel_id, w.lounge.id);
    assert_eq!(s.shared_by, w.alex.0);
    assert_eq!(s.embed.url, track());
    assert_eq!(
        s.embed.provider_id.as_deref(),
        Some(&*format!("track:{SPOTIFY_ID}"))
    );
    assert_eq!(s.embed.title.as_deref(), Some("A song"));
    let thumb = s.embed.thumb.clone().expect("the art");
    // 3 h after it was shared
    let (at, until) = (
        chrono::DateTime::parse_from_rfc3339(&s.shared_at).unwrap(),
        chrono::DateTime::parse_from_rfc3339(&s.expires_at).unwrap(),
    );
    assert_eq!(until - at, chrono::Duration::hours(3));

    match next(&mut w.rx).await {
        Event::StageShared { share } => assert_eq!(share, s),
        e => panic!("unexpected {e:?}"),
    }
    // a normal message from the sharer with the link (#288): it unfurls as the Spotify card for everyone, 0.2.x too
    let posted = match next(&mut w.rx).await {
        Event::MessageCreated { message, .. } => message,
        e => panic!("unexpected {e:?}"),
    };
    assert_eq!(posted.channel_id, w.lounge.id);
    assert_eq!(posted.author_id, Some(w.alex.0));
    assert_eq!(posted.kind, MessageKind::Normal);
    assert_eq!(posted.content, format!("{LEAD}{}", track()));
    assert!(posted.mentions.is_empty());
    assert!(posted.stage_share_line.is_none());
    // its preview follows as for any message
    let card = loop {
        let f = tokio::time::timeout(Duration::from_secs(3), w.rx.recv())
            .await
            .expect("timed out waiting for the preview")
            .expect("hub closed");
        if let ServerFrame::Event(Event::MessageUpdated { message }) = f
            && message.id == posted.id
            && !message.embeds.is_empty()
        {
            break message.embeds[0].clone();
        }
    };
    assert_eq!(
        card.provider_id.as_deref(),
        Some(&*format!("track:{SPOTIFY_ID}"))
    );
    // it reads back from the chat, as alex's
    let msgs: Vec<Message> = get_json(
        &w.app,
        &w.sam.1,
        &format!("/channels/{}/messages", w.lounge.id),
    )
    .await;
    assert!(
        msgs.iter()
            .any(|m| m.id == posted.id && m.author_id == Some(w.alex.0))
    );
    // sending it moved alex's read point, as a send does
    let mine = ready_of(&w.app, &w.alex.1).await;
    let rs = mine
        .read_states
        .iter()
        .find(|r| r.channel_id == w.lounge.id)
        .expect("alex's read state");
    assert_eq!(rs.last_read_message_id, Some(posted.id));

    // the art outlives the share: its cleanup clock was restarted just now
    let created: String = sqlx::query_scalar("SELECT created_at FROM media WHERE hash = ?")
        .bind(&thumb.hash)
        .fetch_one(&w.app.db)
        .await
        .unwrap();
    let age =
        chrono::Utc::now().fixed_offset() - chrono::DateTime::parse_from_rfc3339(&created).unwrap();
    assert!(age < chrono::Duration::seconds(30));

    // everyone joining later sees it (jo isn't in the server but the channel is public)
    let ready = ready_of(&w.app, &w.jo.1).await;
    assert_eq!(ready.stage_shares, vec![s.clone()]);

    // sharing the same thing again (another copy of the link) changes nothing
    let other_copy = format!("https://open.spotify.com/track/{SPOTIFY_ID}?si=xyz");
    let again: StageShare = share(&w, &w.alex.1, &other_copy)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(again.id, s.id);
    quiet(&mut w.rx).await;
}

#[tokio::test]
async fn jams_by_app_uri_and_short_link() {
    let mut w = world().await;
    let s: StageShare = share(&w, &w.alex.1, &format!("spotify:socialsession:{JAM}"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(s.embed.url, SPOTIFY_JAM_URL);
    assert_eq!(s.embed.provider_id.as_deref(), Some(&*format!("jam:{JAM}")));
    next(&mut w.rx).await;
    match next(&mut w.rx).await {
        Event::MessageCreated { message, .. } => {
            assert_eq!(message.content, format!("{LEAD}{SPOTIFY_JAM_URL}"))
        }
        e => panic!("unexpected {e:?}"),
    }
    let short = w.fake.url("spotify.link", "/JamCode1");
    let r = share(&w, &w.sam.1, &short).await;
    assert_eq!(r.status(), 200);
    let s2: StageShare = r.json().await.unwrap();
    assert_eq!(
        s2.embed.provider_id.as_deref(),
        Some(&*format!("jam:{JAM}"))
    );
}

#[tokio::test]
async fn a_new_share_replaces_the_one_up() {
    let mut w = world().await;
    let first: StageShare = share(&w, &w.alex.1, &track()).await.json().await.unwrap();
    next(&mut w.rx).await;
    next(&mut w.rx).await;
    let second: StageShare = share(&w, &w.sam.1, &format!("spotify:socialsession:{JAM}"))
        .await
        .json()
        .await
        .unwrap();
    match next(&mut w.rx).await {
        Event::StageShareCleared {
            channel_id,
            share_id,
        } => assert_eq!((channel_id, share_id), (w.lounge.id, first.id.clone())),
        e => panic!("unexpected {e:?}"),
    }
    assert!(matches!(next(&mut w.rx).await, Event::StageShared { share } if share.id == second.id));
    assert_eq!(w.app.voice.stage.of(w.lounge.id), vec![second]);
    // stopping the replaced one: it isn't up
    assert_eq!(stop(&w, &w.alex.1, &first.id).await, 404);
}

#[tokio::test]
async fn only_spotify_links_from_people_in_the_call() {
    let w = world().await;
    for bad in [
        "https://example.com/track/x",
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
        "not a link",
        "",
    ] {
        assert_eq!(share(&w, &w.alex.1, bad).await.status(), 400, "{bad}");
    }
    // nothing was fetched for them
    assert!(w.fake.requests().is_empty(), "{:?}", w.fake.requests());
    // a Spotify link whose card can't be read
    let missing = "https://open.spotify.com/track/missing0000000000000000";
    assert_eq!(share(&w, &w.alex.1, missing).await.status(), 400);

    // jo isn't in the server; kim is in it but not in the call
    assert_eq!(share(&w, &w.jo.1, &track()).await.status(), 403);
    let kim = register(&w.app, "kim").await;
    let srv = w.lounge.server_id.unwrap();
    post_json(&w.app, &kim.1, &format!("/servers/{srv}/join"), json!({})).await;
    assert_eq!(share(&w, &kim.1, &track()).await.status(), 403);

    // a text channel has no stage
    let chans: Vec<Channel> =
        get_json(&w.app, &w.alex.1, &format!("/servers/{srv}/channels")).await;
    let text = chans.iter().find(|c| c.kind == ChannelKind::Text).unwrap();
    let r = w
        .app
        .http
        .put(w.app.url(&format!("/channels/{}/stage-share", text.id)))
        .bearer_auth(&w.alex.1)
        .json(&json!({ "url": track() }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    assert!(w.app.voice.stage.of(w.lounge.id).is_empty());
}

#[tokio::test]
async fn anyone_in_the_call_may_stop_it() {
    let mut w = world().await;
    let s: StageShare = share(&w, &w.alex.1, &track()).await.json().await.unwrap();
    next(&mut w.rx).await;
    next(&mut w.rx).await;
    // not in the call
    let kim = register(&w.app, "kim").await;
    post_json(
        &w.app,
        &kim.1,
        &format!("/servers/{}/join", w.lounge.server_id.unwrap()),
        json!({}),
    )
    .await;
    assert_eq!(stop(&w, &kim.1, &s.id).await, 403);
    assert_eq!(stop(&w, &w.sam.1, "no-such-share").await, 404);
    assert_eq!(stop(&w, &w.sam.1, &s.id).await, 204);
    match next(&mut w.rx).await {
        Event::StageShareCleared { share_id, .. } => assert_eq!(share_id, s.id),
        e => panic!("unexpected {e:?}"),
    }
    // a stop adds no line
    quiet(&mut w.rx).await;
    assert!(ready_of(&w.app, &w.sam.1).await.stage_shares.is_empty());
}

#[tokio::test]
async fn it_runs_out() {
    let mut w = world_with(Config {
        stage_share_ttl: Duration::from_millis(400),
        ..test_config()
    })
    .await;
    let s: StageShare = share(&w, &w.alex.1, &track()).await.json().await.unwrap();
    next(&mut w.rx).await;
    next(&mut w.rx).await;
    match next(&mut w.rx).await {
        Event::StageShareCleared { share_id, .. } => assert_eq!(share_id, s.id),
        e => panic!("unexpected {e:?}"),
    }
    assert!(w.app.voice.stage.of(w.lounge.id).is_empty());
}

#[tokio::test]
async fn the_sharer_leaving_the_call_takes_it_down() {
    let mut w = world().await;
    let s: StageShare = share(&w, &w.alex.1, &track()).await.json().await.unwrap();
    next(&mut w.rx).await;
    next(&mut w.rx).await;
    // someone else leaving changes nothing
    left(&w.app, w.lounge.id, w.sam.0, "PA_sam").await;
    quiet(&mut w.rx).await;
    assert_eq!(w.app.voice.stage.of(w.lounge.id).len(), 1);
    // a late leave for another channel (alex moved here from there) changes nothing either
    let other = ChannelId::new();
    left(&w.app, other, w.alex.0, "PA_old").await;
    assert_eq!(w.app.voice.stage.of(w.lounge.id).len(), 1);
    // a stale leave (alex's old connection, after a reconnect) changes nothing
    left(&w.app, w.lounge.id, w.alex.0, "PA_alex_old").await;
    quiet(&mut w.rx).await;
    assert_eq!(w.app.voice.stage.of(w.lounge.id).len(), 1);
    // the sharer leaving does
    left(&w.app, w.lounge.id, w.alex.0, "PA_alex").await;
    match next(&mut w.rx).await {
        Event::StageShareCleared { share_id, .. } => assert_eq!(share_id, s.id),
        e => panic!("unexpected {e:?}"),
    }
    assert!(w.app.voice.stage.of(w.lounge.id).is_empty());
}

#[tokio::test]
async fn deleting_the_channel_drops_it() {
    let w = world().await;
    assert_eq!(share(&w, &w.alex.1, &track()).await.status(), 200);
    let r = w
        .app
        .http
        .delete(w.app.url(&format!("/channels/{}", w.lounge.id)))
        .bearer_auth(&w.alex.1)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    assert!(w.app.voice.stage.of(w.lounge.id).is_empty());
    assert!(ready_of(&w.app, &w.sam.1).await.stage_shares.is_empty());
}
