//! Hide what I'm playing per person (#436): someone on your list never gets your live game (Ready's people, live
//! `PlayingChanged`) and sees you online all the same; everyone else is untouched; hiding takes your game from their
//! clients at once and showing sends it again; your list follows you (Ready and `ActivityHiddenChanged`); "Listening
//! to" isn't touched. Your profile and stats (Steam hours on the card and the board, Steam completion, badges) still
//! show to them as to anyone (#438). Nothing here reaches the real network.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{ListeningUpdate, Playing, PlayingUpdate, Profile, SteamHours};
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

fn playing_about(events: &[Event], who: UserId) -> Vec<Option<Playing>> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::PlayingChanged { user_id, playing } if *user_id == who => Some(playing.clone()),
            _ => None,
        })
        .collect()
}

fn person(r: &Ready, who: UserId) -> &pulse_protocol::rest::Person {
    r.people.iter().find(|p| p.user.id == who).unwrap()
}

async fn play(app: &TestApp, ws: &mut Socket, me: UserId, name: Option<&str>) {
    let f = serde_json::to_string(&ClientFrame::Playing {
        playing: name.map(|n| PlayingUpdate {
            name: n.into(),
            steam_app_id: None,
            elapsed_secs: Some(60),
            hide_art: false,
        }),
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    for _ in 0..200 {
        if app.hub.playing_of(me).map(|p| p.name).as_deref() == name {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the server never held the game");
}

async fn hide(app: &TestApp, token: &str, who: UserId, on: bool) -> reqwest::Response {
    let url = app.url(&format!("/me/activity-hidden/{who}"));
    let req = if on {
        app.http.put(url)
    } else {
        app.http.delete(url)
    };
    req.bearer_auth(token).send().await.unwrap()
}

#[tokio::test]
async fn a_hidden_viewer_never_gets_your_game_and_still_sees_you_online() {
    let app = spawn().await;
    let (alex, alex_t) = register(&app, "alex").await;
    let (sam, sam_t) = register(&app, "sam").await;
    let (_, kim_t) = register(&app, "kim").await;

    let r = hide(&app, &alex_t, sam, true).await;
    assert_eq!(r.status(), 200);
    let list: Vec<UserId> = r.json().await.unwrap();
    assert_eq!(list, vec![sam]);

    let (mut alex_ws, ready) = hello(&app, &alex_t).await;
    assert_eq!(ready.activity_hidden, vec![sam], "your list follows you");
    let (mut sam_ws, _) = hello(&app, &sam_t).await;
    let (mut kim_ws, _) = hello(&app, &kim_t).await;
    for ws in [&mut alex_ws, &mut sam_ws, &mut kim_ws] {
        drain(ws, 200).await;
    }

    // and "Listening to" isn't game activity: sam still gets it
    let f = serde_json::to_string(&ClientFrame::Listening {
        listening: Some(ListeningUpdate {
            title: "A song".into(),
            artist: None,
            album: None,
            spotify_id: None,
            player: None,
        }),
        show: true,
    })
    .unwrap();
    alex_ws.send(Ws::text(f)).await.unwrap();
    play(&app, &mut alex_ws, alex, Some("Space Game")).await;

    let kim_got = drain(&mut kim_ws, 400).await;
    assert_eq!(
        playing_about(&kim_got, alex)
            .last()
            .cloned()
            .flatten()
            .map(|p| p.name),
        Some("Space Game".into())
    );
    let sam_got = drain(&mut sam_ws, 400).await;
    assert!(playing_about(&sam_got, alex).is_empty(), "{sam_got:?}");
    assert!(
        sam_got.iter().any(
            |e| matches!(e, Event::ListeningChanged { user_id, listening: Some(_) } if *user_id == alex)
        ),
        "{sam_got:?}"
    );

    // Ready: online, no game, for sam only; you always see your own
    let (_ws, sam_ready) = hello(&app, &sam_t).await;
    let p = person(&sam_ready, alex);
    assert!(p.online);
    assert_eq!(p.playing, None);
    assert!(p.listening.is_some());
    let (_ws, kim_ready) = hello(&app, &kim_t).await;
    assert!(person(&kim_ready, alex).playing.is_some());
    let (_ws, own) = hello(&app, &alex_t).await;
    assert!(person(&own, alex).playing.is_some());
}

#[tokio::test]
async fn hiding_takes_your_game_away_at_once_and_showing_sends_it_again() {
    let app = spawn().await;
    let (alex, alex_t) = register(&app, "alex").await;
    let (sam, sam_t) = register(&app, "sam").await;
    let (_, kim_t) = register(&app, "kim").await;
    let (mut alex_ws, _) = hello(&app, &alex_t).await;
    let (mut sam_ws, _) = hello(&app, &sam_t).await;
    let (mut kim_ws, _) = hello(&app, &kim_t).await;
    play(&app, &mut alex_ws, alex, Some("Space Game")).await;
    for ws in [&mut alex_ws, &mut sam_ws, &mut kim_ws] {
        drain(ws, 300).await;
    }

    assert_eq!(hide(&app, &alex_t, sam, true).await.status(), 200);
    assert_eq!(
        playing_about(&drain(&mut sam_ws, 400).await, alex),
        vec![None]
    );
    assert!(playing_about(&drain(&mut kim_ws, 300).await, alex).is_empty());
    let own = drain(&mut alex_ws, 300).await;
    assert!(
        own.iter().any(
            |e| matches!(e, Event::ActivityHiddenChanged { user_id, list } if *user_id == alex && *list == vec![sam])
        ),
        "{own:?}"
    );
    // a game change while hidden reaches kim, never sam
    play(&app, &mut alex_ws, alex, Some("Racer")).await;
    assert!(playing_about(&drain(&mut sam_ws, 400).await, alex).is_empty());
    assert_eq!(
        playing_about(&drain(&mut kim_ws, 300).await, alex)
            .last()
            .cloned()
            .flatten()
            .map(|p| p.name),
        Some("Racer".into())
    );

    // shown again: sam gets the game now
    let r = hide(&app, &alex_t, sam, false).await;
    assert_eq!(r.status(), 200);
    assert!(r.json::<Vec<UserId>>().await.unwrap().is_empty());
    assert_eq!(
        playing_about(&drain(&mut sam_ws, 400).await, alex)
            .last()
            .cloned()
            .flatten()
            .map(|p| p.name),
        Some("Racer".into())
    );
    let (_ws, sam_ready) = hello(&app, &sam_t).await;
    assert!(person(&sam_ready, alex).playing.is_some());
}

#[tokio::test]
async fn steam_hours_still_show_to_whoever_you_hide_your_games_from() {
    // #438: only your live game is hidden; hours vanishing from the card or the board would be the clearer tell
    let app = spawn().await;
    let (alex, alex_t) = register(&app, "alex").await;
    let (sam, sam_t) = register(&app, "sam").await;
    let (_, kim_t) = register(&app, "kim").await;
    let server = create_server(&app, &alex_t, "Gamers").await;
    for t in [&sam_t, &kim_t] {
        let r = post_json(&app, t, &format!("/servers/{}/join", server.id), json!({})).await;
        assert!(r.status().is_success());
    }
    sqlx::query(
        "INSERT INTO steam_links (user_id, steam_id, linked_at, backfill, share_hours, recent_minutes, recent_at)
         VALUES (?, '76500000000000001', ?, 'done', 1, 300, ?)",
    )
    .bind(alex.to_string())
    .bind(pulse_server::db::now())
    .bind(pulse_server::db::now())
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(hide(&app, &alex_t, sam, true).await.status(), 200);

    let hours = |t: String| {
        let app = &app;
        async move {
            let p: Profile = get_json(app, &t, &format!("/users/{alex}/profile")).await;
            let b: SteamHours =
                get_json(app, &t, &format!("/servers/{}/steam-hours", server.id)).await;
            (
                p.stats.steam_recent_minutes,
                b.entries.iter().any(|e| e.user_id == alex),
            )
        }
    };
    assert_eq!(hours(kim_t.clone()).await, (Some(300), true));
    assert_eq!(hours(sam_t.clone()).await, (Some(300), true), "hidden from");
    assert_eq!(hours(alex_t.clone()).await, (Some(300), true), "your own");
}

#[tokio::test]
async fn only_real_people_and_never_yourself() {
    let app = spawn().await;
    let (alex, alex_t) = register(&app, "alex").await;
    let (sam, sam_t) = register(&app, "sam").await;
    assert_eq!(hide(&app, &alex_t, sam, true).await.status(), 200);
    assert_eq!(hide(&app, &alex_t, alex, true).await.status(), 400);
    assert_eq!(hide(&app, &alex_t, UserId::new(), true).await.status(), 404);
    let r = app
        .http
        .put(app.url("/me/activity-hidden/not-an-id"))
        .bearer_auth(&alex_t)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    // someone else's list is theirs: sam's Ready doesn't carry alex's
    let (_ws, r) = hello(&app, &sam_t).await;
    assert!(r.activity_hidden.is_empty());
}

#[tokio::test]
async fn steam_stats_and_badges_on_the_card_still_show_to_whoever_you_hide_your_games_from() {
    // #438: the profile card's Steam completion, 100 % games and Steam badges show to everyone, including whoever you
    // hide your games from (owner call: something vanishing is a far more obvious tell than numbers ticking up).
    let app = spawn().await;
    let (alex, alex_t) = register(&app, "alex").await;
    let (sam, sam_t) = register(&app, "sam").await;
    let (_, kim_t) = register(&app, "kim").await;
    sqlx::query(
        "INSERT INTO steam_links (user_id, steam_id, linked_at, backfill, share_achievements, completion_rate,
           perfect_games)
         VALUES (?, '76500000000000001', ?, 'done', 1, 0.8, 3)",
    )
    .bind(alex.to_string())
    .bind(pulse_server::db::now())
    .execute(&app.db)
    .await
    .unwrap();
    for (key, showcase) in [
        ("completionist", 1),
        ("achievement_hunter", 2),
        ("touch_grass", 3),
        ("founder", 4),
    ] {
        sqlx::query(
            "INSERT INTO user_badges (user_id, badge_key, tier, earned_at, showcase) VALUES (?, ?, 1, ?, ?)",
        )
        .bind(alex.to_string())
        .bind(key)
        .bind(pulse_server::db::now())
        .bind(showcase)
        .execute(&app.db)
        .await
        .unwrap();
    }
    assert_eq!(hide(&app, &alex_t, sam, true).await.status(), 200);

    let card = |t: String| {
        let app = &app;
        async move {
            let p: Profile = get_json(app, &t, &format!("/users/{alex}/profile")).await;
            let mut badges: Vec<String> = p.badges.iter().map(|b| b.key.clone()).collect();
            badges.sort();
            (
                p.stats.steam_completion.is_some(),
                p.stats.steam_perfect_games,
                badges,
                p.showcase,
            )
        }
    };
    let all = vec![
        "achievement_hunter".to_string(),
        "completionist".into(),
        "founder".into(),
        "touch_grass".into(),
    ];
    let showcase = vec![
        "completionist".to_string(),
        "achievement_hunter".into(),
        "touch_grass".into(),
        "founder".into(),
    ];
    assert_eq!(
        card(kim_t.clone()).await,
        (true, Some(3), all.clone(), showcase.clone())
    );
    assert_eq!(
        card(sam_t.clone()).await,
        (true, Some(3), all.clone(), showcase.clone()),
        "hidden from"
    );
    assert_eq!(
        card(alex_t.clone()).await,
        (true, Some(3), all, showcase),
        "your own"
    );
}
