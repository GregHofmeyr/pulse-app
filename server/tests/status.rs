//! Status (#45): Online / DND / Invisible and appearing offline to chosen people. Every presence path
//! (Ready, REST, PresenceChanged, reconnects, a second device) must give a viewer exactly what they may
//! see, and nothing else.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{OwnStatus, Person, PresenceStatus, Status};
use pulse_server::gateway::audience::{Audience, audience_for};
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

/// Next event matching `pred` (skips others); panics after 3 s.
async fn wait_for(ws: &mut Socket, pred: impl Fn(&Event) -> bool) -> Event {
    loop {
        match next_frame(ws).await.expect("socket ended") {
            ServerFrame::Event(e) if pred(&e) => return e,
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

/// The presence events about `who` among `events`, as (status, last seen).
fn about(events: &[Event], who: UserId) -> Vec<(PresenceStatus, Option<String>)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::PresenceChanged {
                user_id,
                status,
                last_seen_at,
                online,
            } if *user_id == who => {
                assert_eq!(*online, *status != PresenceStatus::Offline);
                Some((*status, last_seen_at.clone()))
            }
            _ => None,
        })
        .collect()
}

fn person(r: &Ready, who: UserId) -> Person {
    r.people.iter().find(|p| p.user.id == who).unwrap().clone()
}

/// What someone hidden from sees: offline, last seen `at`, `online` false.
fn offline_at(p: &Person, at: &Option<String>) {
    assert_eq!(
        (p.status, p.online, &p.last_seen_at),
        (PresenceStatus::Offline, false, at),
        "{p:?}"
    );
}

async fn set_status(app: &TestApp, token: &str, status: &str, until: Option<&str>) -> u16 {
    app.http
        .put(app.url("/me/status"))
        .bearer_auth(token)
        .json(&serde_json::json!({ "status": status, "dnd_until": until }))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn appear_offline(app: &TestApp, token: &str, to: UserId, hide: bool) -> u16 {
    let url = app.url(&format!("/me/appear-offline/{to}"));
    let req = if hide {
        app.http.put(url)
    } else {
        app.http.delete(url)
    };
    req.bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn db_str(app: &TestApp, sql: &str, id: UserId) -> Option<String> {
    sqlx::query_scalar(sql)
        .bind(id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// alex and sam and kim, sam and kim connected and settled, alex connected and seen online by both.
struct Three {
    app: TestApp,
    alex: (UserId, String),
    sam: (UserId, String, Socket),
    kim: (UserId, String, Socket),
    alex_ws: Socket,
}

async fn three() -> Three {
    let app = spawn().await;
    let alex = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (kim_id, kim) = register(&app, "kim").await;
    let (mut sam_ws, _) = hello(&app, &sam).await;
    let (mut kim_ws, _) = hello(&app, &kim).await;
    let (alex_ws, _) = hello(&app, &alex.1).await;
    for ws in [&mut sam_ws, &mut kim_ws] {
        wait_for(ws, |e| matches!(e, Event::PresenceChanged { user_id, online: true, .. } if *user_id == alex.0)).await;
        drain(ws, 200).await;
    }
    Three {
        app,
        alex,
        sam: (sam_id, sam, sam_ws),
        kim: (kim_id, kim, kim_ws),
        alex_ws,
    }
}

#[tokio::test]
async fn invisible_hides_from_ready_and_events_but_not_from_messages() {
    let Three {
        app,
        alex: (a_id, a),
        sam: (s_id, s, mut sam_ws),
        mut alex_ws,
        ..
    } = three().await;
    assert_eq!(set_status(&app, &a, "invisible", None).await, 204);
    let since = db_str(&app, "SELECT invisible_at FROM users WHERE id = ?", a_id).await;
    assert!(since.is_some());
    let seen = about(&drain(&mut sam_ws, 400).await, a_id);
    assert_eq!(seen, vec![(PresenceStatus::Offline, since.clone())]);
    // alex's own sessions hear their status; their own presence stays true
    let own = wait_for(&mut alex_ws, |e| matches!(e, Event::StatusChanged { .. })).await;
    assert!(matches!(
        own,
        Event::StatusChanged {
            status: OwnStatus {
                status: Status::Invisible,
                ..
            },
            ..
        }
    ));
    // a fresh Ready (sam's second device) agrees
    let (_sam2, ready) = hello(&app, &s).await;
    offline_at(&person(&ready, a_id), &since);
    // ...and alex's own Ready knows the truth
    let (_alex2, mine) = hello(&app, &a).await;
    assert_eq!(mine.status.status, Status::Invisible);
    assert!(person(&mine, a_id).online);
    // decisive actions still show: a message and typing reach sam
    let dm = create_dm(&app, &a, &[s_id]).await;
    send(&app, &a, dm.id, "hi").await;
    wait_for(
        &mut sam_ws,
        |e| matches!(e, Event::MessageCreated { message, .. } if message.author_id == Some(a_id)),
    )
    .await;
    let typing = serde_json::to_string(&ClientFrame::Typing { channel_id: dm.id }).unwrap();
    alex_ws.send(Ws::text(typing)).await.unwrap();
    wait_for(
        &mut sam_ws,
        |e| matches!(e, Event::Typing { user_id, .. } if *user_id == a_id),
    )
    .await;
    // and still no presence about alex
    assert!(about(&drain(&mut sam_ws, 300).await, a_id).is_empty());
}

#[tokio::test]
async fn appear_offline_hides_from_one_viewer_only() {
    let Three {
        app,
        alex: (a_id, a),
        sam: (s_id, _, mut sam_ws),
        kim: (_, k, mut kim_ws),
        alex_ws: _alex_ws,
        ..
    } = three().await;
    assert_eq!(appear_offline(&app, &a, s_id, true).await, 204);
    let at: Option<String> = sqlx::query_scalar("SELECT hidden_at FROM presence_hidden")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(
        about(&drain(&mut sam_ws, 400).await, a_id),
        vec![(PresenceStatus::Offline, at.clone())]
    );
    let kim_saw = drain(&mut kim_ws, 200).await;
    assert!(about(&kim_saw, a_id).is_empty(), "kim: {kim_saw:?}");
    // DND shows to kim, never to sam
    assert_eq!(set_status(&app, &a, "dnd", None).await, 204);
    assert_eq!(
        about(&drain(&mut kim_ws, 400).await, a_id),
        vec![(PresenceStatus::Dnd, None)]
    );
    assert!(about(&drain(&mut sam_ws, 200).await, a_id).is_empty());
    let (_k2, kr) = hello(&app, &k).await;
    assert_eq!(person(&kr, a_id).status, PresenceStatus::Dnd);
}

#[tokio::test]
async fn second_device_and_reconnects_while_invisible_are_silent() {
    let Three {
        app,
        alex: (a_id, a),
        sam: (_, s, mut sam_ws),
        alex_ws,
        ..
    } = three().await;
    assert_eq!(set_status(&app, &a, "invisible", None).await, 204);
    let since = db_str(&app, "SELECT invisible_at FROM users WHERE id = ?", a_id).await;
    drain(&mut sam_ws, 300).await;
    let (second, _) = hello(&app, &a).await;
    drop(second);
    drop(alex_ws);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        db_str(&app, "SELECT last_seen_at FROM users WHERE id = ?", a_id).await > since,
        "the real last seen moves on"
    );
    let (_back, _) = hello(&app, &a).await;
    assert!(about(&drain(&mut sam_ws, 500).await, a_id).is_empty());
    let (_s2, r) = hello(&app, &s).await;
    offline_at(&person(&r, a_id), &since);
}

#[tokio::test]
async fn reconnect_while_hidden_never_leaks() {
    let Three {
        app,
        alex: (a_id, a),
        sam: (s_id, s, mut sam_ws),
        kim: (_, _, mut kim_ws),
        alex_ws,
    } = three().await;
    appear_offline(&app, &a, s_id, true).await;
    let at = drain(&mut sam_ws, 300).await;
    let frozen = about(&at, a_id)[0].1.clone();
    drop(alex_ws);
    // kim sees alex leave, with the real time
    let left = about(&drain(&mut kim_ws, 400).await, a_id);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].0, PresenceStatus::Offline);
    for _ in 0..2 {
        let (ws, _) = hello(&app, &a).await;
        drop(ws);
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    assert!(about(&drain(&mut sam_ws, 400).await, a_id).is_empty());
    // sam reconnecting gets the same frozen picture
    drop(sam_ws);
    let (_s2, r) = hello(&app, &s).await;
    offline_at(&person(&r, a_id), &frozen);
}

#[tokio::test]
async fn last_seen_freezes_then_visible_again_follows_the_normal_rules() {
    let Three {
        app,
        alex: (a_id, a),
        sam: (_, _, mut sam_ws),
        mut alex_ws,
        ..
    } = three().await;
    set_status(&app, &a, "invisible", None).await;
    drain(&mut sam_ws, 300).await;
    // back to Online while connected: sam sees online
    tokio::time::sleep(Duration::from_millis(20)).await;
    set_status(&app, &a, "online", None).await;
    assert_eq!(
        about(&drain(&mut sam_ws, 400).await, a_id),
        vec![(PresenceStatus::Online, None)]
    );
    assert_eq!(
        db_str(&app, "SELECT invisible_at FROM users WHERE id = ?", a_id).await,
        None
    );
    // closing now shows the real disconnect
    alex_ws.close(None).await.unwrap();
    let left = about(&drain(&mut sam_ws, 600).await, a_id);
    let real = db_str(&app, "SELECT last_seen_at FROM users WHERE id = ?", a_id).await;
    assert_eq!(left, vec![(PresenceStatus::Offline, real)]);
}

#[tokio::test]
async fn invisible_and_hidden_stack_earliest_wins() {
    let Three {
        app,
        alex: (a_id, a),
        sam: (s_id, s, mut sam_ws),
        kim: (_, _, mut kim_ws),
        alex_ws: _alex_ws,
        ..
    } = three().await;
    appear_offline(&app, &a, s_id, true).await;
    let t1 = about(&drain(&mut sam_ws, 300).await, a_id)[0].1.clone();
    tokio::time::sleep(Duration::from_millis(20)).await;
    set_status(&app, &a, "invisible", None).await;
    assert!(
        about(&drain(&mut sam_ws, 300).await, a_id).is_empty(),
        "sam keeps t1"
    );
    let t2 = about(&drain(&mut kim_ws, 300).await, a_id)[0].1.clone();
    assert!(t2 > t1);
    // visible again: still hidden from sam
    set_status(&app, &a, "online", None).await;
    assert!(about(&drain(&mut sam_ws, 300).await, a_id).is_empty());
    assert_eq!(
        about(&drain(&mut kim_ws, 300).await, a_id),
        vec![(PresenceStatus::Online, None)]
    );
    let (_s2, r) = hello(&app, &s).await;
    offline_at(&person(&r, a_id), &t1);
    // unhiding announces online to sam
    assert_eq!(appear_offline(&app, &a, s_id, false).await, 204);
    assert_eq!(
        about(&drain(&mut sam_ws, 300).await, a_id),
        vec![(PresenceStatus::Online, None)]
    );
}

#[tokio::test]
async fn hide_while_invisible_then_visible_keeps_freeze() {
    let Three {
        app,
        alex: (a_id, a),
        sam: (s_id, _, mut sam_ws),
        alex_ws: _alex_ws,
        ..
    } = three().await;
    set_status(&app, &a, "invisible", None).await;
    let t = about(&drain(&mut sam_ws, 300).await, a_id)[0].1.clone();
    tokio::time::sleep(Duration::from_millis(20)).await;
    appear_offline(&app, &a, s_id, true).await;
    set_status(&app, &a, "online", None).await;
    assert!(
        about(&drain(&mut sam_ws, 400).await, a_id).is_empty(),
        "sam's last seen must not jump to when alex hid"
    );
    let at: Option<String> = sqlx::query_scalar("SELECT hidden_at FROM presence_hidden")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(at, t);
}

#[tokio::test]
async fn unhide_while_invisible_stays_frozen() {
    let Three {
        app,
        alex: (a_id, a),
        sam: (s_id, s, mut sam_ws),
        mut alex_ws,
        ..
    } = three().await;
    appear_offline(&app, &a, s_id, true).await;
    let t1 = about(&drain(&mut sam_ws, 300).await, a_id)[0].1.clone();
    tokio::time::sleep(Duration::from_millis(20)).await;
    set_status(&app, &a, "invisible", None).await;
    appear_offline(&app, &a, s_id, false).await;
    assert!(about(&drain(&mut sam_ws, 300).await, a_id).is_empty());
    let (_s2, r) = hello(&app, &s).await;
    offline_at(&person(&r, a_id), &t1);
    // off alex's list
    wait_for(&mut alex_ws, |e| {
        matches!(e, Event::AppearOfflineChanged { hidden: false, .. })
    })
    .await;
    let (_a2, mine) = hello(&app, &a).await;
    assert!(mine.appear_offline_to.is_empty());
    // visible again: sam sees alex
    set_status(&app, &a, "online", None).await;
    assert_eq!(
        about(&drain(&mut sam_ws, 300).await, a_id),
        vec![(PresenceStatus::Online, None)]
    );
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM presence_hidden")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn dnd_shows_as_dnd_and_its_timer_expires() {
    let Three {
        app,
        alex: (a_id, a),
        sam: (_, s, mut sam_ws),
        mut alex_ws,
        ..
    } = three().await;
    let until = pulse_server::db::stamp(chrono::Utc::now() + chrono::Duration::hours(1));
    assert_eq!(set_status(&app, &a, "dnd", Some(&until)).await, 204);
    assert_eq!(
        about(&drain(&mut sam_ws, 300).await, a_id),
        vec![(PresenceStatus::Dnd, None)]
    );
    let own = wait_for(&mut alex_ws, |e| matches!(e, Event::StatusChanged { .. })).await;
    assert!(
        matches!(own, Event::StatusChanged { status: OwnStatus { status: Status::Dnd, dnd_until: Some(ref u) }, .. } if *u == until)
    );
    // the timer runs out: a fresh Ready already says Online, before any sweep
    sqlx::query("UPDATE users SET dnd_until = '2000-01-01T00:00:00.000Z' WHERE id = ?")
        .bind(a_id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    let (_s2, r) = hello(&app, &s).await;
    assert_eq!(person(&r, a_id).status, PresenceStatus::Online);
    let (_a2, mine) = hello(&app, &a).await;
    assert_eq!(mine.status, OwnStatus::default());
    // the sweep tells everyone
    assert_eq!(
        pulse_server::presence::sweep::sweep_dnd(&app.state())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        about(&drain(&mut sam_ws, 300).await, a_id),
        vec![(PresenceStatus::Online, None)]
    );
    let own = wait_for(&mut alex_ws, |e| matches!(e, Event::StatusChanged { .. })).await;
    assert!(matches!(
        own,
        Event::StatusChanged {
            status: OwnStatus {
                status: Status::Online,
                dnd_until: None
            },
            ..
        }
    ));
    assert_eq!(
        pulse_server::presence::sweep::sweep_dnd(&app.state())
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn hidden_list_and_status_only_reach_their_owner() {
    let Three {
        app,
        alex: (a_id, a),
        sam: (s_id, s, mut sam_ws),
        kim: (k_id, _, _),
        mut alex_ws,
    } = three().await;
    let hidden = Event::AppearOfflineChanged {
        user_id: a_id,
        target_id: s_id,
        hidden: true,
    };
    let only_alex = Audience::Users([a_id].into_iter().collect());
    assert_eq!(audience_for(&app.db, &hidden).await.unwrap(), only_alex);
    let status = Event::StatusChanged {
        user_id: a_id,
        status: OwnStatus::default(),
    };
    assert_eq!(audience_for(&app.db, &status).await.unwrap(), only_alex);

    appear_offline(&app, &a, s_id, true).await;
    appear_offline(&app, &a, k_id, true).await;
    set_status(&app, &a, "invisible", None).await;
    let (_s2, sr) = hello(&app, &s).await;
    assert!(sr.appear_offline_to.is_empty());
    assert_eq!(sr.status, OwnStatus::default());
    for e in drain(&mut sam_ws, 400).await {
        let text = serde_json::to_string(&e).unwrap();
        assert!(
            !matches!(
                e,
                Event::AppearOfflineChanged { .. } | Event::StatusChanged { .. }
            ),
            "{text}"
        );
        assert!(
            !text.contains("invisible") && !text.contains("\"dnd\""),
            "{text}"
        );
    }
    let (_a2, ar) = hello(&app, &a).await;
    assert_eq!(ar.appear_offline_to, vec![s_id, k_id]);
    wait_for(&mut alex_ws, |e| matches!(e, Event::AppearOfflineChanged { target_id, hidden: true, .. } if *target_id == k_id)).await;
}

#[tokio::test]
async fn own_sessions_get_status_and_dnd_end_on_every_device() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (mut one, _) = hello(&app, &a).await;
    let (mut two, _) = hello(&app, &a).await;
    let until = pulse_server::db::stamp(chrono::Utc::now() + chrono::Duration::hours(8));
    assert_eq!(set_status(&app, &a, "dnd", Some(&until)).await, 204);
    for ws in [&mut one, &mut two] {
        wait_for(ws, |e| matches!(e, Event::StatusChanged { status: OwnStatus { status: Status::Dnd, dnd_until: Some(u) }, .. } if *u == until)).await;
    }
    // it's on the account: every device closed and a new one opened still knows it
    drop(one);
    drop(two);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (_three, r) = hello(&app, &a).await;
    assert_eq!(r.status.status, Status::Dnd);
    assert_eq!(r.status.dnd_until.as_deref(), Some(until.as_str()));
}

#[tokio::test]
async fn bad_requests_are_refused() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let past = pulse_server::db::stamp(chrono::Utc::now() - chrono::Duration::minutes(1));
    let far = pulse_server::db::stamp(chrono::Utc::now() + chrono::Duration::days(8));
    let soon = pulse_server::db::stamp(chrono::Utc::now() + chrono::Duration::hours(1));
    assert_eq!(set_status(&app, &a, "dnd", Some(&past)).await, 400);
    assert_eq!(set_status(&app, &a, "dnd", Some(&far)).await, 400);
    assert_eq!(set_status(&app, &a, "dnd", Some("tomorrow")).await, 400);
    assert_eq!(set_status(&app, &a, "online", Some(&soon)).await, 400);
    assert_eq!(set_status(&app, &a, "invisible", Some(&soon)).await, 400);
    assert_eq!(set_status(&app, &a, "away", None).await, 422);
    assert_eq!(appear_offline(&app, &a, a_id, true).await, 400);
    assert_eq!(appear_offline(&app, &a, UserId::new(), true).await, 404);
    // nothing changed
    assert_eq!(
        db_str(&app, "SELECT presence FROM users WHERE id = ?", a_id).await,
        Some("online".into())
    );
}

#[tokio::test]
async fn never_connected_and_invisible_stays_never_seen() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (_, s) = register(&app, "sam").await;
    let (mut sam_ws, _) = hello(&app, &s).await;
    drain(&mut sam_ws, 200).await;
    set_status(&app, &a, "invisible", None).await;
    let (ws, _) = hello(&app, &a).await;
    drop(ws);
    assert!(about(&drain(&mut sam_ws, 500).await, a_id).is_empty());
    let (_s2, r) = hello(&app, &s).await;
    offline_at(&person(&r, a_id), &None);
}

#[tokio::test]
async fn rest_profile_payloads_carry_no_presence() {
    let Three {
        app,
        alex: (a_id, a),
        sam: (s_id, s, _),
        alex_ws: _alex_ws,
        ..
    } = three().await;
    appear_offline(&app, &a, s_id, true).await;
    set_status(&app, &a, "dnd", None).await;
    for path in [format!("/users/{a_id}/profile"), "/me".to_string()] {
        let v: serde_json::Value = get_json(&app, &s, &path).await;
        let text = v.to_string();
        for key in [
            "online",
            "last_seen",
            "presence",
            "dnd",
            "invisible",
            "hidden",
        ] {
            assert!(!text.contains(key), "{path} mentions {key}: {text}");
        }
    }
}
