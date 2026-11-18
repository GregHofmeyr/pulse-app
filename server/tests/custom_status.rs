//! Custom status (#152): text and an emoji beside your name, sanitised, clearing on a server-side timer, with recent
//! ones for picking again. It rides on presence, so only those you look online to ever receive it.
mod common;

use std::io::Cursor;
use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use image::{ImageFormat, Rgba, RgbaImage};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{CustomEmoji, CustomStatus, MediaInfo, MyCustomStatus};
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

/// The custom statuses `who`'s presence events among `events` carried, in order.
fn customs(events: &[Event], who: UserId) -> Vec<Option<CustomStatus>> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::PresenceChanged {
                user_id,
                custom_status,
                ..
            } if *user_id == who => Some(custom_status.clone()),
            _ => None,
        })
        .collect()
}

/// The last own custom status among `events`.
fn own(events: &[Event]) -> Option<MyCustomStatus> {
    events.iter().rev().find_map(|e| match e {
        Event::MyCustomStatusChanged { status, .. } => Some(status.clone()),
        _ => None,
    })
}

fn seen_in(r: &Ready, who: UserId) -> Option<CustomStatus> {
    r.people
        .iter()
        .find(|p| p.user.id == who)
        .unwrap()
        .custom_status
        .clone()
}

fn cs(text: &str, emoji: Option<&str>) -> CustomStatus {
    CustomStatus {
        text: text.into(),
        emoji: emoji.map(Into::into),
    }
}

async fn set_custom(app: &TestApp, token: &str, body: serde_json::Value) -> u16 {
    app.http
        .put(app.url("/me/custom-status"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn clear_custom(app: &TestApp, token: &str) -> u16 {
    app.http
        .delete(app.url("/me/custom-status"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn put(app: &TestApp, token: &str, path: &str, body: serde_json::Value) -> u16 {
    app.http
        .put(app.url(path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

/// alex, sam and kim, all connected and settled.
async fn three() -> (
    TestApp,
    (UserId, String, Socket),
    (UserId, String, Socket),
    (UserId, String, Socket),
) {
    let app = spawn().await;
    let (s_id, s) = register(&app, "sam").await;
    let (k_id, k) = register(&app, "kim").await;
    let (a_id, a) = register(&app, "alex").await;
    // they share a server: presence reaches only people who share something (#80)
    acquaint(&app, &a, &[&s, &k]).await;
    let (mut sam_ws, _) = hello(&app, &s).await;
    let (mut kim_ws, _) = hello(&app, &k).await;
    let (mut alex_ws, _) = hello(&app, &a).await;
    for ws in [&mut sam_ws, &mut kim_ws, &mut alex_ws] {
        drain(ws, 300).await;
    }
    (
        app,
        (a_id, a, alex_ws),
        (s_id, s, sam_ws),
        (k_id, k, kim_ws),
    )
}

#[tokio::test]
async fn custom_status_reaches_only_those_who_see_you_online() {
    let (app, (a_id, a, mut alex_ws), (_, s, mut sam_ws), (k_id, k, mut kim_ws)) = three().await;
    let gaming = cs("Gaming", Some("🎮"));
    assert_eq!(
        set_custom(&app, &a, json!({ "text": "Gaming", "emoji": "🎮" })).await,
        204
    );
    assert_eq!(
        customs(&drain(&mut sam_ws, 300).await, a_id),
        vec![Some(gaming.clone())]
    );
    let mine = own(&drain(&mut alex_ws, 300).await).expect("own sessions told");
    assert_eq!(mine.current, Some(gaming.clone()));
    assert_eq!(mine.until, None);
    assert_eq!(mine.recent, vec![gaming.clone()]);
    // a fresh Ready agrees, for others and for alex
    let (_s2, r) = hello(&app, &s).await;
    assert_eq!(seen_in(&r, a_id), Some(gaming.clone()));
    let (_a2, r) = hello(&app, &a).await;
    assert_eq!(r.my_custom_status.current, Some(gaming.clone()));
    assert_eq!(seen_in(&r, a_id), Some(gaming.clone()));
    drain(&mut alex_ws, 200).await;
    drain(&mut sam_ws, 200).await;
    drain(&mut kim_ws, 200).await;

    // appear offline to kim: kim loses it, sam keeps it
    assert_eq!(
        put(&app, &a, &format!("/me/appear-offline/{k_id}"), json!({})).await,
        204
    );
    assert_eq!(customs(&drain(&mut kim_ws, 300).await, a_id), vec![None]);
    assert!(customs(&drain(&mut sam_ws, 200).await, a_id).is_empty());
    let (_k2, r) = hello(&app, &k).await;
    assert_eq!(seen_in(&r, a_id), None);
    // and a change never reaches kim
    assert_eq!(set_custom(&app, &a, json!({ "text": "Busy" })).await, 204);
    assert_eq!(
        customs(&drain(&mut sam_ws, 300).await, a_id),
        vec![Some(cs("Busy", None))]
    );
    assert!(customs(&drain(&mut kim_ws, 200).await, a_id).is_empty());

    // Invisible: nobody else sees it; alex still does
    assert_eq!(
        put(&app, &a, "/me/status", json!({ "status": "invisible" })).await,
        204
    );
    assert_eq!(customs(&drain(&mut sam_ws, 300).await, a_id), vec![None]);
    let (_s3, r) = hello(&app, &s).await;
    assert_eq!(seen_in(&r, a_id), None);
    let (_a3, r) = hello(&app, &a).await;
    assert_eq!(seen_in(&r, a_id), Some(cs("Busy", None)));
    assert_eq!(r.my_custom_status.current, Some(cs("Busy", None)));
    // back to Online: sam sees it again
    assert_eq!(
        put(&app, &a, "/me/status", json!({ "status": "online" })).await,
        204
    );
    assert_eq!(
        customs(&drain(&mut sam_ws, 300).await, a_id),
        vec![Some(cs("Busy", None))]
    );

    // offline: none
    drop((alex_ws, _a2, _a3));
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (_s4, r) = hello(&app, &s).await;
    assert_eq!(seen_in(&r, a_id), None);

    // cleared: alex's recent ones stay
    assert_eq!(clear_custom(&app, &a).await, 204);
    let (_a4, r) = hello(&app, &a).await;
    assert_eq!(r.my_custom_status.current, None);
    assert_eq!(r.my_custom_status.recent, vec![cs("Busy", None), gaming]);
}

#[tokio::test]
async fn text_is_sanitised_and_bad_requests_are_refused() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    assert_eq!(
        set_custom(
            &app,
            &a,
            json!({ "text": "  \u{202E}on\u{202C}\n\n a\u{0007} break\t ", "emoji": "❤️" })
        )
        .await,
        204
    );
    let row: (Option<String>, Option<String>) =
        sqlx::query_as("SELECT custom_text, custom_emoji FROM users WHERE id = ?")
            .bind(a_id.to_string())
            .fetch_one(&app.db)
            .await
            .unwrap();
    // FE0F dropped, as a reaction key
    assert_eq!(row, (Some("on a break".into()), Some("❤".into())));
    let soon = |h| pulse_server::db::stamp(chrono::Utc::now() + chrono::Duration::hours(h));
    for (body, why) in [
        (json!({ "text": "x".repeat(129) }), "too long"),
        (json!({ "text": "   " }), "empty"),
        (json!({ "text": "\u{202E}" }), "empty once stripped"),
        (
            json!({ "text": "hi", "emoji": "notanemoji" }),
            "unknown emoji",
        ),
        (
            json!({ "text": "hi", "emoji": "custom:NOPE" }),
            "unknown custom emoji",
        ),
        (json!({ "text": "hi", "until": "soon" }), "not a time"),
        (json!({ "text": "hi", "until": soon(-1) }), "past"),
        (json!({ "text": "hi", "until": soon(24 * 9) }), "too far"),
    ] {
        assert_eq!(set_custom(&app, &a, body).await, 400, "{why}");
    }
    // a week (and a bit, for time zones) is fine; exactly 128 characters is fine
    assert_eq!(
        set_custom(
            &app,
            &a,
            json!({ "text": "é".repeat(128), "until": soon(24 * 7 + 12) })
        )
        .await,
        204
    );
    assert_eq!(
        set_custom(&app, &a, json!({ "text": "", "emoji": "🎮" })).await,
        204
    );
}

#[tokio::test]
async fn custom_emoji_need_their_server() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (_, k) = register(&app, "kim").await;
    let srv = create_server(&app, &a, "alex's").await;
    let e = custom_emoji(&app, &a, srv.id, "pog").await;
    let key = format!("custom:{}", e.id);
    assert_eq!(
        set_custom(&app, &a, json!({ "text": "pog", "emoji": key })).await,
        204
    );
    assert_eq!(
        set_custom(&app, &k, json!({ "text": "pog", "emoji": key })).await,
        403
    );
}

#[tokio::test]
async fn timer_clears_on_read_then_the_sweep_tells_everyone() {
    let (app, (a_id, a, mut alex_ws), (_, s, mut sam_ws), _) = three().await;
    let until = pulse_server::db::stamp(chrono::Utc::now() + chrono::Duration::minutes(30));
    assert_eq!(
        set_custom(
            &app,
            &a,
            json!({ "text": "Lunch", "emoji": "🍕", "until": until })
        )
        .await,
        204
    );
    let mine = own(&drain(&mut alex_ws, 300).await).unwrap();
    assert_eq!(mine.until.as_deref(), Some(until.as_str()));
    drain(&mut sam_ws, 200).await;
    // run out: a fresh Ready already shows none, before any sweep
    sqlx::query("UPDATE users SET custom_until = '2000-01-01T00:00:00.000Z' WHERE id = ?")
        .bind(a_id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    let (_s2, r) = hello(&app, &s).await;
    assert_eq!(seen_in(&r, a_id), None);
    let (_a2, r) = hello(&app, &a).await;
    assert_eq!(r.my_custom_status.current, None);
    assert_eq!(r.my_custom_status.recent, vec![cs("Lunch", Some("🍕"))]);
    // the sweep clears it for good and tells everyone whose view changed
    assert_eq!(
        pulse_server::presence::sweep::sweep_timers(&app.state())
            .await
            .unwrap(),
        1
    );
    assert_eq!(customs(&drain(&mut sam_ws, 300).await, a_id), vec![None]);
    assert_eq!(own(&drain(&mut alex_ws, 300).await).unwrap().current, None);
    let left: Option<String> = sqlx::query_scalar("SELECT custom_text FROM users WHERE id = ?")
        .bind(a_id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(left, None);
    assert_eq!(
        pulse_server::presence::sweep::sweep_timers(&app.state())
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn recent_keeps_the_newest_five_and_moves_a_repick_to_the_front() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    for i in 1..=7 {
        assert_eq!(
            set_custom(&app, &a, json!({ "text": format!("s{i}") })).await,
            204
        );
    }
    let (_ws, r) = hello(&app, &a).await;
    let texts = |r: &Ready| {
        r.my_custom_status
            .recent
            .iter()
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(texts(&r), ["s7", "s6", "s5", "s4", "s3"]);
    assert_eq!(set_custom(&app, &a, json!({ "text": "s4" })).await, 204);
    let (_ws2, r) = hello(&app, &a).await;
    assert_eq!(texts(&r), ["s4", "s7", "s6", "s5", "s3"]);
    // the same text with an emoji is another one
    assert_eq!(
        set_custom(&app, &a, json!({ "text": "s4", "emoji": "🎮" })).await,
        204
    );
    let (_ws3, r) = hello(&app, &a).await;
    assert_eq!(r.my_custom_status.recent[0], cs("s4", Some("🎮")));
    assert_eq!(r.my_custom_status.recent[1], cs("s4", None));
}

#[tokio::test]
async fn the_custom_status_lives_only_in_presence_payloads() {
    let (app, (a_id, a, _alex_ws), (_, s, _), _) = three().await;
    assert_eq!(set_custom(&app, &a, json!({ "text": "Gaming" })).await, 204);
    // the profile REST payload carries none (it's presence, not profile)
    let body = app
        .http
        .get(app.url(&format!("/users/{a_id}/profile")))
        .bearer_auth(&s)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!body.contains("Gaming"), "{body}");
}

fn png() -> Vec<u8> {
    let img = RgbaImage::from_pixel(64, 32, Rgba([10, 120, 240, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

async fn custom_emoji(app: &TestApp, token: &str, server: ServerId, name: &str) -> CustomEmoji {
    let r = app
        .http
        .post(app.url("/media/emoji"))
        .bearer_auth(token)
        .body(png())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let media: MediaInfo = r.json().await.unwrap();
    let r = post_json(
        app,
        token,
        &format!("/servers/{server}/emoji"),
        json!({ "name": name, "media_hash": media.hash }),
    )
    .await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

/// #418: leaving (or being removed from) a server drops its emoji from your custom status and keeps the text; one that
/// was only that emoji goes. Your own sessions and everyone who sees you online hear it; recent ones with its emoji go.
#[tokio::test]
async fn leaving_a_server_drops_its_emoji_from_your_status() {
    let (app, (_, a, _alex_ws), (s_id, s, mut sam_ws), (k_id, k, mut kim_ws)) = three().await;
    let srv = create_server(&app, &a, "alex's").await;
    for t in [&s, &k] {
        assert!(join_server(&app, t, srv.id).await.status().is_success());
    }
    let e = custom_emoji(&app, &a, srv.id, "pog").await;
    let key = format!("custom:{}", e.id);
    assert_eq!(
        set_custom(&app, &s, json!({ "text": "", "emoji": key })).await,
        204
    );
    assert_eq!(
        set_custom(&app, &s, json!({ "text": "Raiding", "emoji": key })).await,
        204
    );
    drain(&mut sam_ws, 300).await;
    drain(&mut kim_ws, 300).await;

    let r = post_json(&app, &s, &format!("/servers/{}/leave", srv.id), json!({})).await;
    assert_eq!(r.status(), 204);
    let mine = own(&drain(&mut sam_ws, 300).await).expect("told their own sessions");
    assert_eq!(mine.current, Some(cs("Raiding", None)));
    assert!(
        mine.recent
            .iter()
            .all(|r| r.emoji.as_deref() != Some(key.as_str())),
        "{:?}",
        mine.recent
    );
    assert_eq!(
        customs(&drain(&mut kim_ws, 300).await, s_id)
            .last()
            .cloned(),
        Some(Some(cs("Raiding", None)))
    );

    // an emoji-only status goes altogether (kim, removed this time)
    assert_eq!(
        set_custom(&app, &k, json!({ "text": "", "emoji": key })).await,
        204
    );
    let gone = app
        .http
        .delete(app.url(&format!("/servers/{}/members/{}", srv.id, k_id)))
        .bearer_auth(&a)
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), 204);
    let (_k2, r) = hello(&app, &k).await;
    assert_eq!(r.my_custom_status.current, None);
}
