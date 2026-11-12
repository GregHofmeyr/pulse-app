//! "Listening to…" (#275): what someone's Spotify plays, as their app sends it, reaches exactly the people they look
//! online to (events and Ready), goes with its connection, is rate-limited without losing the latest, and feeds a Jam
//! its sharer put on the stage.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{Channel, ChannelKind, Listening, ListeningUpdate, StageShare};
use pulse_server::config::Config;
use pulse_server::limits::{LimitsConfig, Rate};
use pulse_server::testing::fake_provider::{FakeProvider, SPOTIFY_ID, SPOTIFY_JAM_URL};
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

/// Every event that arrives within `ms` of quiet.
async fn drain(ws: &mut Socket, ms: u64) -> Vec<Event> {
    let mut got = vec![];
    while let Ok(Some(f)) = tokio::time::timeout(Duration::from_millis(ms), next_frame(ws)).await {
        if let ServerFrame::Event(e) = f {
            got.push(e);
        }
    }
    got
}

/// The next `ListeningChanged` about `who` (skipping everything else); panics after 3 s.
async fn next_listening(ws: &mut Socket, who: UserId) -> Option<Listening> {
    loop {
        match next_frame(ws).await.expect("socket ended") {
            ServerFrame::Event(Event::ListeningChanged { user_id, listening })
                if user_id == who =>
            {
                return listening;
            }
            _ => continue,
        }
    }
}

/// The `ListeningChanged` events about `who` among `events`.
fn about(events: &[Event], who: UserId) -> Vec<Option<Listening>> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ListeningChanged { user_id, listening } if *user_id == who => {
                Some(listening.clone())
            }
            _ => None,
        })
        .collect()
}

fn update(title: &str) -> ListeningUpdate {
    ListeningUpdate {
        title: title.into(),
        artist: Some("The Band".into()),
        album: None,
        spotify_id: None,
    }
}

async fn say(ws: &mut Socket, listening: Option<ListeningUpdate>, show: bool) {
    let f = serde_json::to_string(&ClientFrame::Listening { listening, show }).unwrap();
    ws.send(Ws::text(f)).await.unwrap();
}

async fn set_status(app: &TestApp, token: &str, status: &str) {
    let r = app
        .http
        .put(app.url("/me/status"))
        .bearer_auth(token)
        .json(&json!({ "status": status, "dnd_until": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
}

fn listening_in(r: &Ready, who: UserId) -> Option<Listening> {
    r.people
        .iter()
        .find(|p| p.user.id == who)
        .unwrap()
        .listening
        .clone()
}

struct People {
    /// Keep it bound (`app: _app`), never left to `..`: it owns the database's temp dir, and once that's gone the
    /// server's next new pool connection fails (SQLITE_CANTOPEN), so presence and activity announcements go missing.
    app: TestApp,
    fake: FakeProvider,
    alex: (UserId, String, Socket),
    sam: (UserId, String, Socket),
    kim: (UserId, String, Socket),
}

async fn people_with(limits: LimitsConfig) -> People {
    let fake = FakeProvider::start().await;
    let app = spawn_with(Config {
        embeds: fake.config(),
        limits,
        ..test_config()
    })
    .await;
    let (a, at) = register(&app, "alex").await;
    let (s, st) = register(&app, "sam").await;
    let (k, kt) = register(&app, "kim").await;
    let (mut sam_ws, _) = hello(&app, &st).await;
    let (mut kim_ws, _) = hello(&app, &kt).await;
    let (mut alex_ws, _) = hello(&app, &at).await;
    for ws in [&mut sam_ws, &mut kim_ws, &mut alex_ws] {
        drain(ws, 200).await;
    }
    People {
        app,
        fake,
        alex: (a, at, alex_ws),
        sam: (s, st, sam_ws),
        kim: (k, kt, kim_ws),
    }
}

async fn people() -> People {
    people_with(LimitsConfig::unlimited()).await
}

#[tokio::test]
async fn what_alex_plays_reaches_everyone_with_its_art_and_their_next_ready() {
    let People {
        app,
        fake,
        alex: (a, at, mut alex_ws),
        sam: (_, st, mut sam_ws),
        ..
    } = people().await;
    say(
        &mut alex_ws,
        Some(ListeningUpdate {
            spotify_id: Some(format!("track:{SPOTIFY_ID}")),
            album: Some("  An\u{0007} album ".into()),
            ..update("A song")
        }),
        true,
    )
    .await;
    let got = next_listening(&mut sam_ws, a).await.expect("listening");
    assert_eq!(got.title, "A song");
    assert_eq!(got.artist.as_deref(), Some("The Band"));
    assert_eq!(got.album.as_deref(), Some("An album"), "cleaned");
    assert_eq!(got.spotify_id, Some(format!("track:{SPOTIFY_ID}")));
    let art = got.art.clone().expect("art from the song's Spotify card");
    assert!(fake.hits() > 0);
    // alex sees their own
    assert_eq!(next_listening(&mut alex_ws, a).await, Some(got.clone()));
    // a new device of sam's starts from it
    let (_sam2, ready) = hello(&app, &st).await;
    assert_eq!(listening_in(&ready, a), Some(got.clone()));
    let (_alex2, mine) = hello(&app, &at).await;
    assert_eq!(listening_in(&mine, a), Some(got.clone()));

    // a pause and resume of the same song keeps its art without asking again
    let hits = fake.hits();
    say(
        &mut alex_ws,
        Some(ListeningUpdate {
            spotify_id: Some(format!("track:{SPOTIFY_ID}")),
            ..update("A song")
        }),
        true,
    )
    .await;
    let again = next_listening(&mut sam_ws, a).await.unwrap();
    assert_eq!(again.art, Some(art));
    assert_eq!(fake.hits(), hits);

    // stopped: gone for everyone
    say(&mut alex_ws, None, true).await;
    assert_eq!(next_listening(&mut sam_ws, a).await, None);
    let (_sam3, ready) = hello(&app, &st).await;
    assert_eq!(listening_in(&ready, a), None);
}

#[tokio::test]
async fn show_off_tells_nobody_and_a_bad_update_is_nothing() {
    let People {
        app: _app,
        alex: (a, _, mut alex_ws),
        sam: (_, _, mut sam_ws),
        ..
    } = people().await;
    say(&mut alex_ws, Some(update("Hidden song")), false).await;
    assert!(about(&drain(&mut sam_ws, 400).await, a).is_empty());
    assert!(about(&drain(&mut alex_ws, 100).await, a).is_empty());
    // shown again: there it is; then a title that's only control characters is nothing playing
    say(&mut alex_ws, Some(update("Song")), true).await;
    assert!(next_listening(&mut sam_ws, a).await.is_some());
    say(&mut alex_ws, Some(update("\u{0000}")), true).await;
    assert_eq!(next_listening(&mut sam_ws, a).await, None);
}

#[tokio::test]
async fn invisible_and_appear_offline_show_no_activity() {
    let People {
        app,
        alex: (a, at, mut alex_ws),
        sam: (s, st, mut sam_ws),
        kim: (_, _, mut kim_ws),
        ..
    } = people().await;
    say(&mut alex_ws, Some(update("Song")), true).await;
    let song = next_listening(&mut sam_ws, a).await;
    assert!(song.is_some());
    next_listening(&mut kim_ws, a).await;

    set_status(&app, &at, "invisible").await;
    assert_eq!(next_listening(&mut sam_ws, a).await, None);
    assert_eq!(next_listening(&mut kim_ws, a).await, None);
    let (_sam2, ready) = hello(&app, &st).await;
    assert_eq!(listening_in(&ready, a), None);
    // still sent while invisible: nobody hears it
    say(&mut alex_ws, Some(update("Another")), true).await;
    assert!(about(&drain(&mut sam_ws, 400).await, a).is_empty());
    // back online: everyone sees what's playing now
    set_status(&app, &at, "online").await;
    assert_eq!(
        next_listening(&mut sam_ws, a).await.map(|l| l.title),
        Some("Another".into())
    );
    next_listening(&mut kim_ws, a).await;

    // appearing offline to sam takes it from sam only
    let r = app
        .http
        .put(app.url(&format!("/me/appear-offline/{s}")))
        .bearer_auth(&at)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(next_listening(&mut sam_ws, a).await, None);
    say(&mut alex_ws, Some(update("Third")), true).await;
    assert_eq!(
        next_listening(&mut kim_ws, a).await.map(|l| l.title),
        Some("Third".into())
    );
    assert!(about(&drain(&mut sam_ws, 300).await, a).is_empty());
}

#[tokio::test]
async fn it_goes_with_its_connection() {
    let People {
        app,
        alex: (a, at, mut alex_ws),
        sam: (_, _, mut sam_ws),
        ..
    } = people().await;
    // a second device of alex's that reads nothing
    let (mut alex2, _) = hello(&app, &at).await;
    drain(&mut sam_ws, 200).await;
    say(&mut alex_ws, Some(update("Song")), true).await;
    assert!(next_listening(&mut sam_ws, a).await.is_some());
    // the reading device closes; alex is still online on the other, listening to nothing
    alex_ws.close(None).await.unwrap();
    assert_eq!(next_listening(&mut sam_ws, a).await, None);
    // the other device reads now; its own closing clears it too, with alex going offline
    say(&mut alex2, Some(update("Other")), true).await;
    assert!(next_listening(&mut sam_ws, a).await.is_some());
    alex2.close(None).await.unwrap();
    assert_eq!(next_listening(&mut sam_ws, a).await, None);
}

#[tokio::test]
async fn over_the_limit_the_latest_still_lands() {
    let People {
        app: _app,
        alex: (a, _, mut alex_ws),
        sam: (_, _, mut sam_ws),
        ..
    } = people_with(LimitsConfig {
        listening_user: Rate::new(1, 2.0),
        ..LimitsConfig::unlimited()
    })
    .await;
    for t in ["One", "Two", "Three", "Four"] {
        say(&mut alex_ws, Some(update(t)), true).await;
    }
    // everything up to the latest (the trailing retry is one refill, 0.5 s, away), then nothing after it
    let mut seen: Vec<String> = vec![];
    while seen.last().map(String::as_str) != Some("Four") {
        let l = next_listening(&mut sam_ws, a).await;
        seen.push(l.expect("something playing").title);
    }
    assert!(
        about(&drain(&mut sam_ws, 1000).await, a).is_empty(),
        "{seen:?}"
    );
    assert_eq!(seen.first().map(String::as_str), Some("One"));
    assert_eq!(seen.last().map(String::as_str), Some("Four"), "{seen:?}");
    assert!(seen.len() <= 2, "the held-back ones were dropped: {seen:?}");
}

/// alex's server with alex and sam in its Lounge call.
async fn lounge(app: &TestApp, alex: &(UserId, String), sam: &(UserId, String)) -> Channel {
    let s = create_server(app, &alex.1, "Main").await;
    let r = post_json(app, &sam.1, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);
    let chans: Vec<Channel> = get_json(app, &alex.1, &format!("/servers/{}/channels", s.id)).await;
    let lounge = chans
        .into_iter()
        .find(|c| c.kind == ChannelKind::Voice)
        .unwrap();
    app.voice.join(lounge.id, alex.0, "PA_alex");
    app.voice.join(lounge.id, sam.0, "PA_sam");
    lounge
}

async fn share(app: &TestApp, token: &str, ch: &Channel, url: &str) -> StageShare {
    let r = app
        .http
        .put(app.url(&format!("/channels/{}/stage-share", ch.id)))
        .bearer_auth(token)
        .json(&json!({ "url": url }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

/// The next `StageNowPlaying` (skipping everything else).
async fn next_now_playing(ws: &mut Socket) -> (String, Option<Listening>) {
    loop {
        match next_frame(ws).await.expect("socket ended") {
            ServerFrame::Event(Event::StageNowPlaying {
                share_id,
                now_playing,
                ..
            }) => return (share_id, now_playing),
            _ => continue,
        }
    }
}

#[tokio::test]
async fn a_jam_on_the_stage_shows_what_its_sharer_plays() {
    let People {
        app,
        alex: (a, at, mut alex_ws),
        sam: (s, st, mut sam_ws),
        ..
    } = people().await;
    let ch = lounge(&app, &(a, at.clone()), &(s, st.clone())).await;
    // reading, but not showing it: the Jam still gets it, nobody gets an activity
    say(&mut alex_ws, Some(update("Before")), false).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let jam = share(&app, &at, &ch, SPOTIFY_JAM_URL).await;
    assert_eq!(jam.now_playing.map(|l| l.title), Some("Before".into()));
    drain(&mut sam_ws, 300).await;

    say(&mut alex_ws, Some(update("Next song")), false).await;
    let (id, now) = next_now_playing(&mut sam_ws).await;
    assert_eq!(id, jam.id);
    assert_eq!(now.map(|l| l.title), Some("Next song".into()));
    // Ready carries it
    let (_sam2, ready) = hello(&app, &st).await;
    assert_eq!(
        ready.stage_shares[0]
            .now_playing
            .as_ref()
            .map(|l| l.title.as_str()),
        Some("Next song")
    );
    // stopped reading: the Jam's line goes
    say(&mut alex_ws, None, false).await;
    assert_eq!(next_now_playing(&mut sam_ws).await.1, None);
    assert!(about(&drain(&mut sam_ws, 300).await, a).is_empty());

    // a song shared to the stage isn't a Jam: no now playing
    say(&mut alex_ws, Some(update("Again")), false).await;
    // (the Jam, still up, shows it)
    assert_eq!(
        next_now_playing(&mut sam_ws).await.1.map(|l| l.title),
        Some("Again".into())
    );
    let song = share(
        &app,
        &at,
        &ch,
        &format!("https://open.spotify.com/track/{SPOTIFY_ID}"),
    )
    .await;
    assert_eq!(song.now_playing, None);
    say(&mut alex_ws, Some(update("Once more")), false).await;
    let got: Vec<_> = drain(&mut sam_ws, 400)
        .await
        .into_iter()
        .filter(|e| matches!(e, Event::StageNowPlaying { .. }))
        .collect();
    assert!(got.is_empty(), "{got:?}");
}

/// Published to an audience by mistake, an activity reaches nobody: it's only ever told per viewer.
#[tokio::test]
async fn an_activity_is_never_broadcast() {
    use pulse_server::gateway::audience::{Audience, audience_for};
    let app = spawn().await;
    let e = Event::ListeningChanged {
        user_id: UserId::new(),
        listening: None,
    };
    assert_eq!(
        audience_for(&app.db, &e).await.unwrap(),
        Audience::Users(Default::default())
    );
}
