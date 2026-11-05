//! Notification levels per server and server channel (#195), kept beside mutes.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::rest::{MuteTarget, NotifyLevel, NotifyLevelPref};
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

/// The next `NotifyLevelsChanged` (skips other frames); None when none comes within 3 s.
async fn next_levels(ws: &mut Socket) -> Option<Vec<NotifyLevelPref>> {
    loop {
        match next_frame(ws).await? {
            ServerFrame::Event(Event::NotifyLevelsChanged { levels, .. }) => return Some(levels),
            _ => continue,
        }
    }
}

async fn set_level(app: &TestApp, token: &str, kind: &str, id: &str, level: Option<&str>) -> u16 {
    app.http
        .put(app.url("/notify-levels"))
        .bearer_auth(token)
        .json(&serde_json::json!({"target_kind": kind, "target_id": id, "level": level}))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn mute(app: &TestApp, token: &str, kind: &str, id: &str) -> u16 {
    app.http
        .put(app.url("/mutes"))
        .bearer_auth(token)
        .json(&serde_json::json!({"target_kind": kind, "target_id": id, "until": null}))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn unmute(app: &TestApp, token: &str, kind: &str, id: &str) -> u16 {
    app.http
        .delete(app.url(&format!("/mutes/{kind}/{id}")))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

fn pref(kind: MuteTarget, id: impl ToString, level: NotifyLevel) -> NotifyLevelPref {
    NotifyLevelPref {
        target_kind: kind,
        target_id: id.to_string(),
        level,
    }
}

#[tokio::test]
async fn levels_are_set_read_back_and_private() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let s = create_server(&app, &a, "Main").await;
    let g = general(&app, &a, s.id).await;
    let sid = s.id.to_string();
    let gid = g.id.to_string();

    let (_ws, ready) = hello(&app, &a).await;
    assert!(
        ready.notify_levels.is_empty(),
        "everyone starts on the defaults"
    );

    let (mut ws, _) = hello(&app, &a).await;
    assert_eq!(
        set_level(&app, &a, "server", &sid, Some("mentions")).await,
        204
    );
    assert_eq!(
        next_levels(&mut ws).await.unwrap(),
        vec![pref(MuteTarget::Server, &sid, NotifyLevel::Mentions)]
    );
    assert_eq!(
        set_level(&app, &a, "channel", &gid, Some("nothing")).await,
        204
    );
    let levels = next_levels(&mut ws).await.unwrap();
    assert_eq!(levels.len(), 2);
    assert!(levels.contains(&pref(MuteTarget::Channel, &gid, NotifyLevel::Nothing)));

    let (_ws, ready) = hello(&app, &a).await;
    assert_eq!(ready.notify_levels.len(), 2, "follows alex to every device");
    let (mut ws_b, ready_b) = hello(&app, &b).await;
    assert!(
        ready_b.notify_levels.is_empty(),
        "sam never sees alex's levels"
    );

    // back to the defaults
    assert_eq!(set_level(&app, &a, "channel", &gid, None).await, 204);
    assert_eq!(set_level(&app, &a, "server", &sid, None).await, 204);
    let (_ws, ready) = hello(&app, &a).await;
    assert!(ready.notify_levels.is_empty());
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notification_prefs")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(rows, 0, "a row with nothing left in it goes");
    assert!(
        next_levels(&mut ws_b).await.is_none(),
        "the event is alex's alone"
    );
}

#[tokio::test]
async fn a_servers_all_messages_is_its_default() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let s = create_server(&app, &a, "Main").await;
    let g = general(&app, &a, s.id).await;
    assert_eq!(
        set_level(&app, &a, "server", &s.id.to_string(), Some("all")).await,
        204
    );
    // a channel's All is an override of its server, so it is kept
    assert_eq!(
        set_level(&app, &a, "channel", &g.id.to_string(), Some("all")).await,
        204
    );
    let (_ws, ready) = hello(&app, &a).await;
    assert_eq!(
        ready.notify_levels,
        vec![pref(MuteTarget::Channel, g.id, NotifyLevel::All)]
    );
}

#[tokio::test]
async fn levels_and_mutes_live_side_by_side() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let s = create_server(&app, &a, "Main").await;
    let sid = s.id.to_string();
    assert_eq!(mute(&app, &a, "server", &sid).await, 204);
    assert_eq!(
        set_level(&app, &a, "server", &sid, Some("nothing")).await,
        204
    );
    let (_ws, ready) = hello(&app, &a).await;
    assert_eq!(ready.mutes.len(), 1, "setting a level keeps the mute");
    assert_eq!(ready.notify_levels.len(), 1);

    assert_eq!(unmute(&app, &a, "server", &sid).await, 204);
    let (_ws, ready) = hello(&app, &a).await;
    assert!(ready.mutes.is_empty());
    assert_eq!(
        ready.notify_levels,
        vec![pref(MuteTarget::Server, &sid, NotifyLevel::Nothing)],
        "unmuting keeps the level"
    );

    assert_eq!(mute(&app, &a, "server", &sid).await, 204);
    assert_eq!(set_level(&app, &a, "server", &sid, None).await, 204);
    let (_ws, ready) = hello(&app, &a).await;
    assert_eq!(ready.mutes.len(), 1, "clearing the level keeps the mute");
    assert!(ready.notify_levels.is_empty());
}

#[tokio::test]
async fn only_servers_and_their_channels_take_a_level() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    let unknown = pulse_protocol::ids::ChannelId::new().to_string();
    assert_eq!(
        set_level(
            &app,
            &a,
            "server",
            "01J00000000000000000000000",
            Some("all")
        )
        .await,
        404
    );
    assert_eq!(
        set_level(&app, &a, "channel", &unknown, Some("all")).await,
        404
    );
    assert_eq!(
        set_level(&app, &a, "channel", &dm.id.to_string(), Some("nothing")).await,
        400,
        "a DM always alerts"
    );
    let s = create_server(&app, &a, "Main").await;
    let r = post_json(
        &app,
        &a,
        &format!("/servers/{}/channels", s.id),
        serde_json::json!({"name": "Lounge", "kind": "voice"}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let v: pulse_protocol::rest::Channel = r.json().await.unwrap();
    assert_eq!(
        set_level(&app, &a, "channel", &v.id.to_string(), Some("all")).await,
        400,
        "a voice channel has no messages to alert"
    );
    // a conversation you're not in is as unknown as one that doesn't exist
    let (_, k) = register(&app, "kim").await;
    assert_eq!(
        set_level(&app, &k, "channel", &dm.id.to_string(), Some("all")).await,
        404
    );
    assert_eq!(
        set_level(&app, &b, "server", "x", Some("loud")).await,
        422,
        "no such level"
    );
}

#[tokio::test]
async fn a_restored_channel_brings_its_level_back() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let s = create_server(&app, &a, "Main").await;
    let r = post_json(
        &app,
        &a,
        &format!("/servers/{}/channels", s.id),
        serde_json::json!({"name": "spam", "kind": "text"}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let ch: pulse_protocol::rest::Channel = r.json().await.unwrap();
    assert_eq!(
        set_level(&app, &a, "channel", &ch.id.to_string(), Some("nothing")).await,
        204
    );
    let del = app
        .http
        .delete(app.url(&format!("/channels/{}", ch.id)))
        .bearer_auth(&a)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 204);
    let (mut ws, _) = hello(&app, &a).await;
    let restore = post_json(
        &app,
        &a,
        &format!("/channels/{}/restore", ch.id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(restore.status(), 200);
    assert_eq!(
        next_levels(&mut ws).await.unwrap(),
        vec![pref(MuteTarget::Channel, ch.id, NotifyLevel::Nothing)]
    );
}
