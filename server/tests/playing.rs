//! "Playing" (#380): the game someone's app detects reaches exactly the people they look online to (events and Ready),
//! beside their "Listening to"; a Steam game gets its art from the (fake) Steam CDN, fetched by the server once per
//! game and reused; a non-Steam game is its name only; it goes with its connection; and a client that sends nothing
//! (game activity off) shows nothing. Nothing here reaches the real network.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{ListeningUpdate, Playing, PlayingUpdate};
use pulse_server::config::Config;
use pulse_server::limits::{LimitsConfig, Rate};
use pulse_server::testing::fake_provider::{FakeProvider, STEAM_APP_FULL, STEAM_APP_HEADER_ONLY};
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

/// The next `PlayingChanged` about `who` (skipping everything else); panics after 3 s.
async fn next_playing(ws: &mut Socket, who: UserId) -> Option<Playing> {
    loop {
        match next_frame(ws).await.expect("socket ended") {
            ServerFrame::Event(Event::PlayingChanged { user_id, playing }) if user_id == who => {
                return playing;
            }
            _ => continue,
        }
    }
}

fn about(events: &[Event], who: UserId) -> Vec<Option<Playing>> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::PlayingChanged { user_id, playing } if *user_id == who => Some(playing.clone()),
            _ => None,
        })
        .collect()
}

fn game(name: &str) -> PlayingUpdate {
    PlayingUpdate {
        name: name.into(),
        steam_app_id: None,
        elapsed_secs: None,
        hide_art: false,
    }
}

fn steam(name: &str, app_id: u32) -> PlayingUpdate {
    PlayingUpdate {
        steam_app_id: Some(app_id),
        elapsed_secs: Some(120),
        ..game(name)
    }
}

async fn play(ws: &mut Socket, playing: Option<PlayingUpdate>) {
    let f = serde_json::to_string(&ClientFrame::Playing { playing }).unwrap();
    ws.send(Ws::text(f)).await.unwrap();
}

fn playing_in(r: &Ready, who: UserId) -> Option<Playing> {
    r.people
        .iter()
        .find(|p| p.user.id == who)
        .unwrap()
        .playing
        .clone()
}

/// The fake CDN's requests for Steam art so far.
fn cdn_hits(fake: &FakeProvider) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.starts_with("/steam/"))
        .count()
}

struct People {
    /// Keep it bound: it owns the database's temp dir (see listening.rs).
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
async fn a_steam_game_reaches_everyone_with_its_art_beside_listening() {
    let People {
        app,
        fake,
        alex: (a, _, mut alex_ws),
        sam: (_, st, mut sam_ws),
        kim: (k, _, mut kim_ws),
    } = people().await;
    // listening and playing at once: two activities, neither replaces the other
    let f = serde_json::to_string(&ClientFrame::Listening {
        listening: Some(ListeningUpdate {
            title: "A song".into(),
            artist: None,
            album: None,
            spotify_id: None,
        }),
        show: true,
    })
    .unwrap();
    alex_ws.send(Ws::text(f)).await.unwrap();
    play(
        &mut alex_ws,
        Some(steam("  Space\u{0007} Game ", STEAM_APP_FULL)),
    )
    .await;
    let got = next_playing(&mut sam_ws, a).await.expect("playing");
    assert_eq!(got.name, "Space Game", "cleaned");
    assert_eq!(got.steam_app_id, Some(STEAM_APP_FULL));
    let started = chrono::DateTime::parse_from_rfc3339(got.started_at.as_deref().unwrap()).unwrap();
    let ago = chrono::Utc::now()
        .signed_duration_since(started)
        .num_seconds();
    assert!((115..=135).contains(&ago), "started about 120 s ago: {ago}");
    let art = got.art.clone().expect("art from the Steam CDN");
    let (hero, header, logo) = (
        art.hero.clone().unwrap(),
        art.header.clone().unwrap(),
        art.logo.clone().unwrap(),
    );
    assert!(hero.width > hero.height * 2, "the wide hero, kept wide");
    assert_ne!(header.hash, logo.hash);
    assert_eq!(cdn_hits(&fake), 3, "three pieces, each fetched once");
    // the art is stored media anyone signed in can load
    let r = app
        .http
        .get(app.url(&format!("/media/{}", hero.hash)))
        .bearer_auth(&st)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    // alex sees their own; a new device of sam's starts from it, with the listening beside it
    assert_eq!(next_playing(&mut alex_ws, a).await, Some(got.clone()));
    let (_sam2, ready) = hello(&app, &st).await;
    assert_eq!(playing_in(&ready, a), Some(got.clone()));
    let person = ready.people.iter().find(|p| p.user.id == a).unwrap();
    assert_eq!(
        person.listening.as_ref().map(|l| l.title.as_str()),
        Some("A song")
    );

    // the same game again (a reconnect re-sends it) asks the CDN nothing; nor does kim starting it
    play(&mut alex_ws, Some(steam("Space Game", STEAM_APP_FULL))).await;
    assert_eq!(
        next_playing(&mut sam_ws, a).await.unwrap().art,
        Some(art.clone())
    );
    play(&mut kim_ws, Some(steam("Space Game", STEAM_APP_FULL))).await;
    assert_eq!(next_playing(&mut sam_ws, k).await.unwrap().art, Some(art));
    assert_eq!(cdn_hits(&fake), 3);

    // stopped: gone for everyone, and the listening stays
    play(&mut alex_ws, None).await;
    assert_eq!(next_playing(&mut sam_ws, a).await, None);
    let (_sam3, ready) = hello(&app, &st).await;
    assert_eq!(playing_in(&ready, a), None);
    assert!(
        ready
            .people
            .iter()
            .find(|p| p.user.id == a)
            .unwrap()
            .listening
            .is_some()
    );
}

#[tokio::test]
async fn non_steam_and_partial_art() {
    let People {
        app: _app,
        fake,
        alex: (a, _, mut alex_ws),
        sam: (_, _, mut sam_ws),
        ..
    } = people().await;
    // an exe added by hand: its name only (and no play time when that's off), nothing fetched
    play(&mut alex_ws, Some(game("Old Game"))).await;
    let got = next_playing(&mut sam_ws, a).await.unwrap();
    assert_eq!(
        (got.name.as_str(), got.steam_app_id, got.started_at, got.art),
        ("Old Game", None, None, None)
    );
    assert_eq!(cdn_hits(&fake), 0);
    // a game with only a header on the CDN: just that
    play(&mut alex_ws, Some(steam("Hat Game", STEAM_APP_HEADER_ONLY))).await;
    let art = next_playing(&mut sam_ws, a).await.unwrap().art.unwrap();
    assert!(art.header.is_some() && art.hero.is_none() && art.logo.is_none());
    // a game with nothing on the CDN: no art, and remembered as none
    play(&mut alex_ws, Some(steam("Odd Game", 12345))).await;
    assert_eq!(next_playing(&mut sam_ws, a).await.unwrap().art, None);
    let hits = cdn_hits(&fake);
    play(&mut alex_ws, Some(steam("Hat Game", STEAM_APP_HEADER_ONLY))).await;
    next_playing(&mut sam_ws, a).await.unwrap();
    play(&mut alex_ws, Some(steam("Odd Game", 12345))).await;
    next_playing(&mut sam_ws, a).await.unwrap();
    assert_eq!(cdn_hits(&fake), hits, "both from the cache");
}

#[tokio::test]
async fn the_art_budget_is_spent_per_game() {
    let People {
        app: _app,
        fake,
        alex: (a, _, mut alex_ws),
        sam: (_, _, mut sam_ws),
        ..
    } = people_with(LimitsConfig {
        listening_art_user: Rate::new(1, 0.0001),
        ..LimitsConfig::unlimited()
    })
    .await;
    play(&mut alex_ws, Some(steam("Hat Game", STEAM_APP_HEADER_ONLY))).await;
    assert!(next_playing(&mut sam_ws, a).await.unwrap().art.is_some());
    // out of budget: the next game shows without art, and the CDN isn't asked
    let hits = cdn_hits(&fake);
    play(&mut alex_ws, Some(steam("Space Game", STEAM_APP_FULL))).await;
    let got = next_playing(&mut sam_ws, a).await.unwrap();
    assert_eq!((got.name.as_str(), got.art), ("Space Game", None));
    assert_eq!(cdn_hits(&fake), hits);
}

#[tokio::test]
async fn nothing_sent_shows_nothing_and_invisible_hides_it() {
    let People {
        app,
        alex: (a, at, mut alex_ws),
        sam: (_, st, mut sam_ws),
        kim: (_, _, mut kim_ws),
        ..
    } = people().await;
    // game activity off: the app sends no Playing at all, so nobody hears of one
    assert!(about(&drain(&mut sam_ws, 300).await, a).is_empty());
    let (_sam2, ready) = hello(&app, &st).await;
    assert_eq!(playing_in(&ready, a), None);

    play(&mut alex_ws, Some(game("Chess"))).await;
    assert!(next_playing(&mut sam_ws, a).await.is_some());
    next_playing(&mut kim_ws, a).await;
    let r = app
        .http
        .put(app.url("/me/status"))
        .bearer_auth(&at)
        .json(&json!({ "status": "invisible", "dnd_until": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(next_playing(&mut sam_ws, a).await, None);
    assert_eq!(next_playing(&mut kim_ws, a).await, None);
    let (_sam3, ready) = hello(&app, &st).await;
    assert_eq!(playing_in(&ready, a), None);
}

#[tokio::test]
async fn it_goes_with_its_connection() {
    let People {
        app,
        alex: (a, at, mut alex_ws),
        sam: (_, _, mut sam_ws),
        ..
    } = people().await;
    let (_alex2, _) = hello(&app, &at).await;
    drain(&mut sam_ws, 200).await;
    play(&mut alex_ws, Some(game("Chess"))).await;
    assert!(next_playing(&mut sam_ws, a).await.is_some());
    // the playing device closes; alex is still online on the other, playing nothing
    alex_ws.close(None).await.unwrap();
    assert_eq!(next_playing(&mut sam_ws, a).await, None);
}

#[tokio::test]
async fn over_the_limit_the_latest_still_lands() {
    let People {
        app: _app,
        alex: (a, _, mut alex_ws),
        sam: (_, _, mut sam_ws),
        ..
    } = people_with(LimitsConfig {
        playing_user: Rate::new(1, 2.0),
        ..LimitsConfig::unlimited()
    })
    .await;
    for g in ["One", "Two", "Three", "Four"] {
        play(&mut alex_ws, Some(game(g))).await;
    }
    let mut seen: Vec<String> = vec![];
    while seen.last().map(String::as_str) != Some("Four") {
        seen.push(next_playing(&mut sam_ws, a).await.expect("a game").name);
    }
    assert!(
        about(&drain(&mut sam_ws, 1000).await, a).is_empty(),
        "{seen:?}"
    );
    assert!(seen.len() <= 2, "the held-back ones were dropped: {seen:?}");
}

#[tokio::test]
async fn playing_is_never_broadcast() {
    use pulse_server::gateway::audience::{Audience, audience_for};
    let app = spawn().await;
    let e = Event::PlayingChanged {
        user_id: UserId::new(),
        playing: None,
    };
    assert_eq!(
        audience_for(&app.db, &e).await.unwrap(),
        Audience::Users(Default::default())
    );
}
