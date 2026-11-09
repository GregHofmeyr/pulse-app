//! The picker wheel (#158): saved wheels (server and Just me), spins, Spin again, limits and the events.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::ChannelId;
use pulse_protocol::rest::{
    ApiError, Channel, ChannelKind, Message, MessageKind, Wheel, WheelScope, wheel_winner_at,
};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

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

/// The next event `pick` takes (skipping others); None when none comes within 3 s.
async fn next_matching<T>(ws: &mut Socket, pick: impl Fn(Event) -> Option<T>) -> Option<T> {
    loop {
        if let ServerFrame::Event(e) = next_frame(ws).await?
            && let Some(t) = pick(e)
        {
            return Some(t);
        }
    }
}

async fn spun(ws: &mut Socket) -> Option<Message> {
    next_matching(ws, |e| match e {
        Event::MessageCreated { message, .. } if message.wheel.is_some() => Some(message),
        _ => None,
    })
    .await
}

async fn req(
    app: &TestApp,
    method: reqwest::Method,
    token: &str,
    path: &str,
    body: Option<Value>,
) -> reqwest::Response {
    let mut r = app.http.request(method, app.url(path)).bearer_auth(token);
    if let Some(b) = body {
        r = r.json(&b);
    }
    r.send().await.unwrap()
}

async fn create(app: &TestApp, token: &str, body: Value) -> reqwest::Response {
    post_json(app, token, "/wheels", body).await
}

async fn create_ok(app: &TestApp, token: &str, body: Value) -> Wheel {
    let r = create(app, token, body).await;
    assert_eq!(r.status(), 201);
    r.json().await.unwrap()
}

async fn spin(app: &TestApp, token: &str, channel: ChannelId, body: Value) -> reqwest::Response {
    post_json(
        app,
        token,
        &format!("/channels/{channel}/wheel-spins"),
        body,
    )
    .await
}

async fn spin_ok(app: &TestApp, token: &str, channel: ChannelId, body: Value) -> Message {
    let r = spin(app, token, channel, body).await;
    assert_eq!(r.status(), 201);
    r.json().await.unwrap()
}

async fn error(r: reqwest::Response) -> (u16, String) {
    let status = r.status().as_u16();
    let body: ApiError = r.json().await.unwrap();
    (status, body.message)
}

async fn join(app: &TestApp, token: &str, server: pulse_protocol::ids::ServerId) {
    let r = post_json(app, token, &format!("/servers/{server}/join"), json!({})).await;
    assert!(r.status().is_success());
}

async fn new_channel(
    app: &TestApp,
    token: &str,
    server: pulse_protocol::ids::ServerId,
    kind: &str,
) -> Channel {
    let r = post_json(
        app,
        token,
        &format!("/servers/{server}/channels"),
        json!({"kind": kind, "name": format!("a-{kind}")}),
    )
    .await;
    assert!(r.status().is_success(), "create {kind}");
    r.json().await.unwrap()
}

#[tokio::test]
async fn server_wheels_are_shared_with_the_server_and_just_me_wheels_are_private() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    let s = create_server(&app, &alex, "Main").await;
    join(&app, &sam, s.id).await;

    let games = create_ok(
        &app,
        &alex,
        json!({"server_id": s.id, "name": " Games tonight ", "options": ["Valheim", " Apex "]}),
    )
    .await;
    assert_eq!(
        (games.scope, games.server_id, games.owner_id),
        (WheelScope::Server, Some(s.id), None)
    );
    assert_eq!(games.name, "Games tonight");
    assert_eq!(games.options, vec!["Valheim", "Apex"]);
    let mine = create_ok(&app, &alex, json!({"name": "Tank", "options": []})).await;
    assert_eq!(mine.scope, WheelScope::Personal);
    assert!(mine.owner_id.is_some() && mine.server_id.is_none());

    // sam is in the server: sees and edits its wheels, never alex's own
    let list: Vec<Wheel> = get_json(&app, &sam, &format!("/servers/{}/wheels", s.id)).await;
    assert_eq!(list, vec![games.clone()]);
    let sams: Vec<Wheel> = get_json(&app, &sam, "/wheels/mine").await;
    assert!(sams.is_empty());
    let alexs: Vec<Wheel> = get_json(&app, &alex, "/wheels/mine").await;
    assert_eq!(alexs, vec![mine.clone()]);
    let r = req(
        &app,
        reqwest::Method::PATCH,
        &sam,
        &format!("/wheels/{}", games.id),
        Some(json!({"options": ["Valheim", "Apex", "Among Us"]})),
    )
    .await;
    assert_eq!(r.status(), 200);
    let edited: Wheel = r.json().await.unwrap();
    assert_eq!(edited.options.len(), 3);
    assert_eq!(edited.name, "Games tonight", "absent fields are kept");
    let r = req(
        &app,
        reqwest::Method::PATCH,
        &sam,
        &format!("/wheels/{}", mine.id),
        Some(json!({"name": "mine now"})),
    )
    .await;
    assert_eq!(
        r.status(),
        404,
        "someone else's Just me wheel doesn't exist for you"
    );
    let r = req(
        &app,
        reqwest::Method::DELETE,
        &sam,
        &format!("/wheels/{}", mine.id),
        None,
    )
    .await;
    assert_eq!(r.status(), 404);

    // kim isn't in the server: no list, no edits, no making one there
    let r = req(
        &app,
        reqwest::Method::GET,
        &kim,
        &format!("/servers/{}/wheels", s.id),
        None,
    )
    .await;
    assert_eq!(r.status(), 403);
    let r = req(
        &app,
        reqwest::Method::PATCH,
        &kim,
        &format!("/wheels/{}", games.id),
        Some(json!({"name": "x"})),
    )
    .await;
    assert_eq!(r.status(), 403);
    let r = create(&app, &kim, json!({"server_id": s.id, "name": "x"})).await;
    assert_eq!(r.status(), 403);

    // deleting
    let r = req(
        &app,
        reqwest::Method::DELETE,
        &sam,
        &format!("/wheels/{}", games.id),
        None,
    )
    .await;
    assert_eq!(r.status(), 204);
    let list: Vec<Wheel> = get_json(&app, &alex, &format!("/servers/{}/wheels", s.id)).await;
    assert!(list.is_empty());
    let r = req(
        &app,
        reqwest::Method::DELETE,
        &alex,
        &format!("/wheels/{}", mine.id),
        None,
    )
    .await;
    assert_eq!(r.status(), 204);
}

#[tokio::test]
async fn wheel_events_reach_the_right_people() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let s = create_server(&app, &alex, "Main").await;
    join(&app, &sam, s.id).await;
    let mut alex_ws = hello(&app, &alex).await;
    let mut sam_ws = hello(&app, &sam).await;
    let saved = |e: Event| match e {
        Event::WheelSaved { wheel } => Some(wheel),
        _ => None,
    };
    let deleted = |e: Event| match e {
        Event::WheelDeleted { wheel_id, .. } => Some(wheel_id),
        _ => None,
    };

    let shared = create_ok(&app, &alex, json!({"server_id": s.id, "name": "Dinner"})).await;
    assert_eq!(
        next_matching(&mut sam_ws, saved).await,
        Some(shared.clone())
    );
    assert_eq!(
        next_matching(&mut alex_ws, saved).await,
        Some(shared.clone())
    );

    let mine = create_ok(&app, &alex, json!({"name": "Tank"})).await;
    assert_eq!(next_matching(&mut alex_ws, saved).await, Some(mine.clone()));
    assert_eq!(
        next_matching(&mut sam_ws, saved).await,
        None,
        "sam never hears of alex's own"
    );

    req(
        &app,
        reqwest::Method::DELETE,
        &alex,
        &format!("/wheels/{}", mine.id),
        None,
    )
    .await;
    assert_eq!(next_matching(&mut alex_ws, deleted).await, Some(mine.id));
    req(
        &app,
        reqwest::Method::DELETE,
        &alex,
        &format!("/wheels/{}", shared.id),
        None,
    )
    .await;
    assert_eq!(next_matching(&mut sam_ws, deleted).await, Some(shared.id));
}

#[tokio::test]
async fn names_options_and_caps_are_checked() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let s = create_server(&app, &alex, "Main").await;
    let bad = [
        json!({"name": "", "options": []}),
        json!({"name": "n".repeat(41), "options": []}),
        json!({"name": "ok", "options": ["a", " "]}),
        json!({"name": "ok", "options": ["x".repeat(61)]}),
        json!({"name": "ok", "options": vec!["o"; 101]}),
    ];
    for b in bad {
        assert_eq!(create(&app, &alex, b.clone()).await.status(), 400, "{b}");
    }
    let full = create_ok(
        &app,
        &alex,
        json!({"name": "ok", "options": vec!["o"; 100]}),
    )
    .await;
    assert_eq!(full.options.len(), 100);

    for i in 1..50 {
        create_ok(
            &app,
            &alex,
            json!({"server_id": s.id, "name": format!("w{i}")}),
        )
        .await;
        create_ok(&app, &alex, json!({"name": format!("mine {i}")})).await;
    }
    create_ok(&app, &alex, json!({"server_id": s.id, "name": "w50"})).await;
    let (status, msg) =
        error(create(&app, &alex, json!({"server_id": s.id, "name": "w51"})).await).await;
    assert_eq!(status, 400);
    assert!(msg.contains("50 wheels"), "{msg}");
    let (status, _) = error(create(&app, &alex, json!({"name": "mine 51"})).await).await;
    assert_eq!(status, 400, "50 Just me wheels a person");
}

#[tokio::test]
async fn a_spin_lands_where_the_server_says_for_everyone() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let s = create_server(&app, &alex, "Main").await;
    join(&app, &sam, s.id).await;
    let g = general(&app, &alex, s.id).await;
    let mut alex_ws = hello(&app, &alex).await;
    let mut sam_ws = hello(&app, &sam).await;

    let line = spin_ok(
        &app,
        &alex,
        g.id,
        json!({"options": [" pizza ", "sushi", "braai"]}),
    )
    .await;
    assert_eq!(line.kind, MessageKind::System);
    assert_eq!(line.author_id, None);
    let w = line.wheel.clone().unwrap();
    assert_eq!(w.options, vec!["pizza", "sushi", "braai"]);
    assert_eq!(
        (w.spinner_id, w.wheel_id.clone(), w.name.clone()),
        (Some(alex_id), None, None)
    );
    assert!((0.0..360.0).contains(&w.angle));
    assert_eq!(wheel_winner_at(w.angle, 3), w.winner as usize);
    assert_eq!(
        line.content,
        format!(
            "🎡 alex spun a quick wheel → {}",
            w.options[w.winner as usize]
        )
    );

    // both watchers get the identical spin
    let a = spun(&mut alex_ws).await.unwrap();
    let b = spun(&mut sam_ws).await.unwrap();
    assert_eq!(a.id, line.id);
    assert_eq!(a.wheel, line.wheel);
    assert_eq!(b.wheel, line.wheel);

    // and so does someone opening the chat later: history carries it
    let history: Vec<Message> = get_json(&app, &sam, &format!("/channels/{}/messages", g.id)).await;
    let later = history.iter().find(|m| m.id == line.id).unwrap();
    assert_eq!(later.wheel, line.wheel);

    // a saved server wheel spins by id, named
    let games = create_ok(
        &app,
        &sam,
        json!({"server_id": s.id, "name": "Games", "options": ["Valheim", "Apex"]}),
    )
    .await;
    let named = spin_ok(&app, &alex, g.id, json!({"wheel_id": games.id})).await;
    let w = named.wheel.unwrap();
    assert_eq!(
        (w.wheel_id, w.name.as_deref()),
        (Some(games.id), Some("Games"))
    );
    assert!(named.content.starts_with("🎡 alex spun Games → "));
}

#[tokio::test]
async fn spins_are_refused_where_they_dont_belong() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    let s = create_server(&app, &alex, "Main").await;
    let other = create_server(&app, &alex, "Other").await;
    let g = general(&app, &alex, s.id).await;
    let quotes = new_channel(&app, &alex, s.id, "quotes").await;
    let voice = new_channel(&app, &alex, s.id, "voice").await;
    let two = json!({"options": ["a", "b"]});

    // too few, neither or both
    for b in [
        json!({"options": ["a"]}),
        json!({"options": []}),
        json!({}),
        json!({"options": ["a", "b"], "wheel_id": "x"}),
    ] {
        assert_eq!(
            spin(&app, &alex, g.id, b.clone()).await.status(),
            400,
            "{b}"
        );
    }
    // not a chat for wheels
    let (status, msg) = error(spin(&app, &alex, quotes.id, two.clone()).await).await;
    assert_eq!(status, 400);
    assert!(msg.contains("wheels spin in"), "{msg}");
    // a voice channel's chat is fine
    spin_ok(&app, &alex, voice.id, two.clone()).await;
    // kim isn't in the server
    assert_eq!(spin(&app, &kim, g.id, two.clone()).await.status(), 403);
    // a DM only for its two
    let dm = create_dm(&app, &alex, &[sam_id]).await;
    assert_eq!(dm.kind, ChannelKind::Dm);
    // a spin reopens the DM for whoever had closed it, like a message
    sqlx::query("INSERT INTO dm_hidden (user_id, channel_id, hidden_at) VALUES (?, ?, ?)")
        .bind(sam_id.to_string())
        .bind(dm.id.to_string())
        .bind("2026-11-09T00:00:00.000Z")
        .execute(&app.db)
        .await
        .unwrap();
    spin_ok(&app, &alex, dm.id, two.clone()).await;
    let hidden: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dm_hidden WHERE channel_id = ?")
        .bind(dm.id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(hidden, 0);
    spin_ok(&app, &sam, dm.id, two.clone()).await;
    assert_eq!(spin(&app, &kim, dm.id, two.clone()).await.status(), 404);

    // a server's wheel spins only in that server; someone's Just me wheel only for them
    let elsewhere = create_ok(
        &app,
        &alex,
        json!({"server_id": other.id, "name": "x", "options": ["a", "b"]}),
    )
    .await;
    assert_eq!(
        spin(&app, &alex, g.id, json!({"wheel_id": elsewhere.id}))
            .await
            .status(),
        400
    );
    assert_eq!(
        spin(&app, &alex, dm.id, json!({"wheel_id": elsewhere.id}))
            .await
            .status(),
        400,
        "no server wheels in DMs"
    );
    let mine = create_ok(&app, &alex, json!({"name": "mine", "options": ["a", "b"]})).await;
    spin_ok(&app, &alex, dm.id, json!({"wheel_id": mine.id})).await;
    spin_ok(&app, &alex, g.id, json!({"wheel_id": mine.id})).await;
    assert_eq!(
        spin(&app, &sam, dm.id, json!({"wheel_id": mine.id}))
            .await
            .status(),
        404
    );
    // a saved wheel being filled can't spin yet
    let empty = create_ok(&app, &alex, json!({"name": "empty", "options": ["a"]})).await;
    assert_eq!(
        spin(&app, &alex, g.id, json!({"wheel_id": empty.id}))
            .await
            .status(),
        400
    );
}

#[tokio::test]
async fn spin_again_and_remove_the_winner() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    let s = create_server(&app, &alex, "Main").await;
    join(&app, &sam, s.id).await;
    let g = general(&app, &alex, s.id).await;
    let wheel = create_ok(
        &app,
        &alex,
        json!({"server_id": s.id, "name": "Games", "options": ["a", "b", "c"]}),
    )
    .await;
    let first = spin_ok(&app, &alex, g.id, json!({"wheel_id": wheel.id})).await;
    let fw = first.wheel.clone().unwrap();

    // anyone in the chat may: sam spins the same three again
    let r = again(&app, &sam, first.id, false).await;
    assert_eq!(r.status(), 201);
    let second: Message = r.json().await.unwrap();
    let sw = second.wheel.unwrap();
    assert_eq!(sw.options, fw.options);
    assert_eq!(
        (sw.name.as_deref(), sw.wheel_id.as_ref()),
        (Some("Games"), Some(&wheel.id))
    );
    assert!(second.content.starts_with("🎡 sam spun Games → "));

    // without the winner: two left, then one would be left
    let r = again(&app, &alex, first.id, true).await;
    assert_eq!(r.status(), 201);
    let third: Message = r.json().await.unwrap();
    let tw = third.wheel.unwrap();
    let mut expect = fw.options.clone();
    expect.remove(fw.winner as usize);
    assert_eq!(tw.options, expect);
    let (status, msg) = error(again(&app, &alex, third.id, true).await).await;
    assert_eq!(status, 400);
    assert!(msg.contains("fewer than 2"), "{msg}");

    // the saved wheel is untouched by the chain
    let list: Vec<Wheel> = get_json(&app, &alex, &format!("/servers/{}/wheels", s.id)).await;
    assert_eq!(list[0].options, vec!["a", "b", "c"]);

    // not for outsiders, nor from a line that isn't a spin
    assert_eq!(again(&app, &kim, first.id, false).await.status(), 403);
    let plain = send(&app, &alex, g.id, "hi").await;
    assert_eq!(again(&app, &alex, plain.id, false).await.status(), 404);

    // a deleted line spins no more
    let gone = spin_ok(&app, &alex, g.id, json!({"options": ["a", "b"]})).await;
    sqlx::query("UPDATE messages SET deleted_at = ? WHERE id = ?")
        .bind("2026-11-09T00:00:00.000Z")
        .bind(gone.id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(again(&app, &alex, gone.id, false).await.status(), 404);

    // a deleted saved wheel's spins still spin again, unlinked
    req(
        &app,
        reqwest::Method::DELETE,
        &alex,
        &format!("/wheels/{}", wheel.id),
        None,
    )
    .await;
    let r = again(&app, &alex, first.id, false).await;
    assert_eq!(r.status(), 201);
    let after: Message = r.json().await.unwrap();
    assert_eq!(after.wheel.unwrap().wheel_id, None);
}

#[tokio::test]
async fn spins_are_rate_limited_per_person() {
    let mut cfg = test_config();
    cfg.limits.wheel_spin_user = pulse_server::limits::Rate::new(1, 0.0001);
    let app = spawn_with(cfg).await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let s = create_server(&app, &alex, "Main").await;
    join(&app, &sam, s.id).await;
    let g = general(&app, &alex, s.id).await;
    let two = json!({"options": ["a", "b"]});
    // a refused spin costs nothing
    assert_eq!(
        spin(&app, &alex, g.id, json!({"options": ["a"]}))
            .await
            .status(),
        400
    );
    let first = spin_ok(&app, &alex, g.id, two.clone()).await;
    let (status, msg) = error(spin(&app, &alex, g.id, two.clone()).await).await;
    assert_eq!(status, 429);
    assert!(msg.contains("one spin every few seconds"), "{msg}");
    let r = post_json(
        &app,
        &alex,
        &format!("/messages/{}/spin-again", first.id),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 429, "Spin again counts too");
    spin_ok(&app, &sam, g.id, two).await;
}

#[tokio::test]
async fn deleting_a_chat_takes_its_spins() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (kim_id, kim) = register(&app, "kim").await;
    let s = create_server(&app, &alex, "Main").await;
    let chan = new_channel(&app, &alex, s.id, "text").await;
    let two = json!({"options": ["a", "b"]});
    spin_ok(&app, &alex, chan.id, two.clone()).await;
    let count = |c: ChannelId| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM wheel_spins WHERE channel_id = ?")
                .bind(c.to_string())
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    assert_eq!(count(chan.id).await, 1);
    let r = req(
        &app,
        reqwest::Method::DELETE,
        &alex,
        &format!("/channels/{}", chan.id),
        None,
    )
    .await;
    assert!(r.status().is_success());
    assert_eq!(
        count(chan.id).await,
        1,
        "kept while it's in Recently deleted"
    );
    let r = req(
        &app,
        reqwest::Method::DELETE,
        &alex,
        &format!("/channels/{}/permanent", chan.id),
        None,
    )
    .await;
    assert!(r.status().is_success());
    assert_eq!(count(chan.id).await, 0);

    // a group whose last member leaves goes, spins and all
    let group = create_dm(&app, &alex, &[sam_id, kim_id]).await;
    assert_eq!(group.kind, ChannelKind::Group);
    spin_ok(&app, &sam, group.id, two).await;
    assert_eq!(count(group.id).await, 1);
    for (id, token) in [(alex_id, &alex), (sam_id, &sam), (kim_id, &kim)] {
        let r = req(
            &app,
            reqwest::Method::DELETE,
            token,
            &format!("/channels/{}/members/{id}", group.id),
            None,
        )
        .await;
        assert!(r.status().is_success(), "{}", r.status());
    }
    assert_eq!(count(group.id).await, 0);
}

async fn again(
    app: &TestApp,
    token: &str,
    line: pulse_protocol::ids::MessageId,
    remove: bool,
) -> reqwest::Response {
    post_json(
        app,
        token,
        &format!("/messages/{line}/spin-again"),
        json!({"remove_winner": remove}),
    )
    .await
}
