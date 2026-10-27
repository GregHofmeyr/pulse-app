mod common;

use std::time::Duration;

use base64::Engine;
use common::*;
use image::codecs::webp::WebPEncoder;
use image::{ExtendedColorType, ImageFormat, Rgba, RgbaImage};
use livekit_api::access_token::{AccessToken, TokenVerifier};
use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{
    Channel, StreamEndReason, StreamInfo, StreamSettings, StreamStatus, VoiceTokenResponse,
};
use pulse_server::limits::{LimitsConfig, Rate};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc::Receiver;

async fn lounge(app: &TestApp, token: &str) -> Channel {
    let s = create_server(app, token, "Main").await;
    let chans: Vec<Channel> = get_json(app, token, &format!("/servers/{}/channels", s.id)).await;
    chans
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap()
}

fn settings() -> serde_json::Value {
    json!({"settings": {"height": 1080, "fps": 60, "audio": true}})
}

async fn go_live(
    app: &TestApp,
    tok: &str,
    ch: ChannelId,
    body: serde_json::Value,
) -> reqwest::Response {
    post_json(app, tok, &format!("/streams/{ch}/live"), body).await
}

async fn stop(app: &TestApp, tok: &str, ch: ChannelId) -> reqwest::Response {
    app.http
        .delete(app.url(&format!("/streams/{ch}/live")))
        .bearer_auth(tok)
        .send()
        .await
        .unwrap()
}

fn signed(app: &TestApp, body: &str) -> String {
    let sum = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body.as_bytes()));
    AccessToken::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .with_sha256(&sum)
        .to_jwt()
        .unwrap()
}

async fn webhook_left(app: &TestApp, room: ChannelId, user: UserId) {
    let body =
        json!({"event": "participant_left", "id": "EV_test", "room": {"name": room.to_string()},
        "participant": {"identity": user.to_string(), "sid": "PA_test"}})
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

/// Next stream event on a hub registration, skipping presence and voice noise.
async fn next_stream_event(rx: &mut Receiver<ServerFrame>) -> Event {
    loop {
        let f = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("timed out waiting for a stream event")
            .expect("hub closed");
        if let ServerFrame::Event(
            e @ (Event::StreamStarted { .. }
            | Event::StreamUpdated { .. }
            | Event::StreamEnded { .. }
            | Event::ViewersChanged { .. }),
        ) = f
        {
            return e;
        }
    }
}

/// Two users in the same voice channel; `b` has a hub registration to watch events on.
async fn pair(
    app: &TestApp,
) -> (
    Channel,
    (UserId, String),
    (UserId, String),
    Receiver<ServerFrame>,
) {
    let (a_id, a) = register(app, "alex").await;
    let (b_id, b) = register(app, "sam").await;
    let l = lounge(app, &a).await;
    let joined = post_json(
        app,
        &b,
        &format!("/servers/{}/join", l.server_id.unwrap()),
        json!({}),
    )
    .await;
    assert_eq!(joined.status(), 204);
    app.voice.join(l.id, a_id, "PA_test");
    app.voice.join(l.id, b_id, "PA_test");
    let rx = app.hub.register(b_id, String::new()).rx;
    (l, (a_id, a), (b_id, b), rx)
}

#[tokio::test]
async fn tickets_name_only_the_stream_room() {
    let app = spawn().await;
    let (uid, tok) = register(&app, "alex").await;
    let l = lounge(&app, &tok).await;
    app.voice.join(l.id, uid, "PA_test"); // what the LiveKit join webhook does
    for (role, sources) in [("v", "screen_share"), ("a", "screen_share_audio")] {
        let res = post_json(
            &app,
            &tok,
            &format!("/streams/{}/ticket", l.id),
            json!({"role": role}),
        )
        .await;
        assert_eq!(res.status(), 200, "{role}");
        let body: VoiceTokenResponse = res.json().await.unwrap();
        assert_eq!(body.url, app.cfg.livekit_url);
        let claims = TokenVerifier::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
            .verify(&body.token)
            .unwrap();
        assert_eq!(
            claims.video.room,
            format!("{}~s", l.id),
            "never the voice room"
        );
        assert_eq!(claims.sub, format!("{uid}#{role}"));
        assert_eq!(claims.video.can_publish_sources, vec![sources]);
        assert_eq!(claims.video.can_publish_data, Some(false));
        assert_eq!(claims.video.can_subscribe, Some(true));
    }
    // the LiveKit webhook ignores the stream room: its name isn't a ChannelId
    assert!(format!("{}~s", l.id).parse::<ChannelId>().is_err());
}

#[tokio::test]
async fn tickets_need_you_in_that_voice_channel() {
    let app = spawn().await;
    let (uid, tok) = register(&app, "alex").await;
    let l = lounge(&app, &tok).await;
    let url = format!("/streams/{}/ticket", l.id);
    assert_eq!(
        post_json(&app, &tok, &url, json!({"role":"v"}))
            .await
            .status(),
        403
    );
    // in voice elsewhere
    let other = create_server(&app, &tok, "Other").await;
    let chans: Vec<Channel> =
        get_json(&app, &tok, &format!("/servers/{}/channels", other.id)).await;
    let elsewhere = chans
        .iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    app.voice.join(elsewhere.id, uid, "PA_test");
    assert_eq!(
        post_json(&app, &tok, &url, json!({"role":"v"}))
            .await
            .status(),
        403
    );
}

#[tokio::test]
async fn tickets_refuse_text_and_dm_channels() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let s = create_server(&app, &a, "Main").await;
    let g = general(&app, &a, s.id).await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    app.voice.join(g.id, a_id, "PA_t");
    app.voice.join(dm.id, a_id, "PA_d");
    for ch in [g.id, dm.id] {
        let r = post_json(
            &app,
            &a,
            &format!("/streams/{ch}/ticket"),
            json!({"role":"v"}),
        )
        .await;
        assert_eq!(r.status(), 400, "{ch}");
    }
}

#[tokio::test]
async fn voice_tickets_publish_microphone_only() {
    let app = spawn().await;
    let (_, tok) = register(&app, "alex").await;
    let l = lounge(&app, &tok).await;
    let r = post_json(&app, &tok, &format!("/voice/{}/token", l.id), json!({})).await;
    let t: VoiceTokenResponse = r.json().await.unwrap();
    let claims = TokenVerifier::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .verify(&t.token)
        .unwrap();
    assert_eq!(claims.video.can_publish_sources, vec!["microphone"]);
}

#[tokio::test]
async fn go_live_broadcasts_stream_started() {
    let app = spawn().await;
    let (l, (a_id, a), _, mut rx) = pair(&app).await;
    let r = go_live(&app, &a, l.id, settings()).await;
    assert_eq!(r.status(), 200);
    let info: StreamInfo = r.json().await.unwrap();
    assert_eq!((info.channel_id, info.user_id), (l.id, a_id));
    match next_stream_event(&mut rx).await {
        Event::StreamStarted { stream } => {
            assert_eq!(stream.user_id, a_id);
            assert_eq!(
                stream.settings,
                StreamSettings {
                    height: 1080,
                    fps: 60,
                    audio: true
                }
            );
            assert_eq!(stream.status, StreamStatus::Live);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn go_live_requires_being_in_the_voice_channel() {
    let app = spawn().await;
    let (l, _, (_, b), _) = pair(&app).await;
    // not in the call
    let (c_id, c) = register(&app, "jo").await;
    assert_eq!(go_live(&app, &c, l.id, settings()).await.status(), 403);
    // in the call but never joined the server
    app.voice.join(l.id, c_id, "PA_c");
    assert_eq!(go_live(&app, &c, l.id, settings()).await.status(), 403);
    assert_eq!(go_live(&app, &b, l.id, settings()).await.status(), 200);
}

#[tokio::test]
async fn go_live_twice_is_stream_updated() {
    let app = spawn().await;
    let (l, (_, a), _, mut rx) = pair(&app).await;
    assert_eq!(go_live(&app, &a, l.id, settings()).await.status(), 200);
    assert!(matches!(
        next_stream_event(&mut rx).await,
        Event::StreamStarted { .. }
    ));
    let again = json!({"settings": {"height": 720, "fps": 30, "audio": false}});
    assert_eq!(go_live(&app, &a, l.id, again).await.status(), 200);
    match next_stream_event(&mut rx).await {
        Event::StreamUpdated { stream } => assert_eq!(stream.settings.height, 720),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn go_live_rejects_odd_settings() {
    let app = spawn().await;
    let (l, (_, a), _, _) = pair(&app).await;
    for (h, f) in [(900, 30), (1080, 45)] {
        let body = json!({"settings": {"height": h, "fps": f, "audio": true}});
        assert_eq!(go_live(&app, &a, l.id, body).await.status(), 400, "{h}p{f}");
    }
    assert!(app.streams.all().is_empty());
}

#[tokio::test]
async fn stop_broadcasts_stopped_and_404s_when_not_live() {
    let app = spawn().await;
    let (l, (a_id, a), _, mut rx) = pair(&app).await;
    assert_eq!(stop(&app, &a, l.id).await.status(), 404);
    go_live(&app, &a, l.id, settings()).await;
    assert!(matches!(
        next_stream_event(&mut rx).await,
        Event::StreamStarted { .. }
    ));
    assert_eq!(stop(&app, &a, l.id).await.status(), 204);
    match next_stream_event(&mut rx).await {
        Event::StreamEnded {
            channel_id,
            user_id,
            reason,
        } => {
            assert_eq!(
                (channel_id, user_id, reason),
                (l.id, a_id, StreamEndReason::Stopped)
            );
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(stop(&app, &a, l.id).await.status(), 404);
}

#[tokio::test]
async fn watch_adds_a_viewer_and_unwatch_removes_it() {
    let app = spawn().await;
    let (l, (a_id, a), (b_id, b), mut rx) = pair(&app).await;
    let watch = format!("/streams/{}/watch", l.id);
    let unwatch = format!("/streams/{}/unwatch", l.id);
    // not live yet
    assert_eq!(
        post_json(&app, &b, &watch, json!({"streamer": a_id}))
            .await
            .status(),
        404
    );
    go_live(&app, &a, l.id, settings()).await;
    assert!(matches!(
        next_stream_event(&mut rx).await,
        Event::StreamStarted { .. }
    ));
    // yourself
    assert_eq!(
        post_json(&app, &a, &watch, json!({"streamer": a_id}))
            .await
            .status(),
        400
    );

    assert_eq!(
        post_json(&app, &b, &watch, json!({"streamer": a_id}))
            .await
            .status(),
        204
    );
    match next_stream_event(&mut rx).await {
        Event::ViewersChanged {
            channel_id,
            user_id,
            viewers,
        } => {
            assert_eq!((channel_id, user_id, viewers), (l.id, a_id, vec![b_id]));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        post_json(&app, &b, &unwatch, json!({"streamer": a_id}))
            .await
            .status(),
        204
    );
    match next_stream_event(&mut rx).await {
        Event::ViewersChanged { viewers, .. } => assert!(viewers.is_empty()),
        other => panic!("unexpected {other:?}"),
    }
    // not in voice
    let (_, c) = register(&app, "jo").await;
    assert_eq!(
        post_json(&app, &c, &watch, json!({"streamer": a_id}))
            .await
            .status(),
        403
    );
}

/// A late unwatch (the stream already ended, or we already left voice) is not an error.
#[tokio::test]
async fn unwatch_is_idempotent() {
    let app = spawn().await;
    let (l, (a_id, a), (b_id, b), mut rx) = pair(&app).await;
    let unwatch = format!("/streams/{}/unwatch", l.id);
    // not live: nothing to leave
    assert_eq!(
        post_json(&app, &b, &unwatch, json!({"streamer": a_id}))
            .await
            .status(),
        204
    );
    go_live(&app, &a, l.id, settings()).await;
    assert!(matches!(
        next_stream_event(&mut rx).await,
        Event::StreamStarted { .. }
    ));
    post_json(
        &app,
        &b,
        &format!("/streams/{}/watch", l.id),
        json!({"streamer": a_id}),
    )
    .await;
    assert!(matches!(
        next_stream_event(&mut rx).await,
        Event::ViewersChanged { .. }
    ));
    // the viewer's voice dropped (no webhook yet): the unwatch still lands and unlists them
    app.voice.leave(l.id, b_id, "PA_test");
    assert_eq!(
        post_json(&app, &b, &unwatch, json!({"streamer": a_id}))
            .await
            .status(),
        204
    );
    match next_stream_event(&mut rx).await {
        Event::ViewersChanged { viewers, .. } => assert!(viewers.is_empty()),
        other => panic!("unexpected {other:?}"),
    }
    // twice
    assert_eq!(
        post_json(&app, &b, &unwatch, json!({"streamer": a_id}))
            .await
            .status(),
        204
    );
    // the stream is gone
    assert_eq!(stop(&app, &a, l.id).await.status(), 204);
    assert_eq!(
        post_json(&app, &b, &unwatch, json!({"streamer": a_id}))
            .await
            .status(),
        204
    );
    // still only for people who can see the channel
    let (_, outsider) = register(&app, "kai").await;
    assert_eq!(
        post_json(&app, &outsider, &unwatch, json!({"streamer": a_id}))
            .await
            .status(),
        403
    );
}

#[tokio::test]
async fn leaving_voice_ends_your_stream() {
    let app = spawn().await;
    let (l, (a_id, a), (b_id, b), mut rx) = pair(&app).await;
    go_live(&app, &a, l.id, settings()).await;
    assert!(matches!(
        next_stream_event(&mut rx).await,
        Event::StreamStarted { .. }
    ));
    post_json(
        &app,
        &b,
        &format!("/streams/{}/watch", l.id),
        json!({"streamer": a_id}),
    )
    .await;
    assert!(matches!(
        next_stream_event(&mut rx).await,
        Event::ViewersChanged { .. }
    ));

    // a viewer who leaves voice is removed
    webhook_left(&app, l.id, b_id).await;
    match next_stream_event(&mut rx).await {
        Event::ViewersChanged {
            user_id, viewers, ..
        } => {
            assert_eq!(user_id, a_id);
            assert!(viewers.is_empty());
        }
        other => panic!("unexpected {other:?}"),
    }
    // the streamer leaving ends the stream
    webhook_left(&app, l.id, a_id).await;
    match next_stream_event(&mut rx).await {
        Event::StreamEnded {
            user_id, reason, ..
        } => {
            assert_eq!((user_id, reason), (a_id, StreamEndReason::LeftVoice));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(app.streams.all().is_empty());
}

#[tokio::test]
async fn a_late_leave_for_another_channel_keeps_the_new_stream() {
    let app = spawn().await;
    let (l, (a_id, a), _, _) = pair(&app).await;
    let other = create_server(&app, &a, "Other").await;
    let chans: Vec<Channel> = get_json(&app, &a, &format!("/servers/{}/channels", other.id)).await;
    let old = chans
        .iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    app.voice.join(old.id, a_id, "PA_test");
    // the old channel's leave lands after the move
    assert_eq!(go_live(&app, &a, l.id, settings()).await.status(), 200);
    webhook_left(&app, old.id, a_id).await;
    assert!(app.streams.get(l.id, a_id).is_some());
}

#[tokio::test]
async fn ready_lists_live_streams() {
    use futures::{SinkExt, StreamExt};
    use pulse_protocol::gateway::ClientFrame;
    use tokio_tungstenite::tungstenite::Message as Ws;

    let app = spawn().await;
    let (l, (_, a), (_, b), _) = pair(&app).await;
    go_live(&app, &a, l.id, settings()).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap();
    let hello = serde_json::to_string(&ClientFrame::Hello {
        token: b,
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(hello)).await.unwrap();
    let ready = loop {
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Ws::Text(t) = m
            && let ServerFrame::Ready(r) = serde_json::from_str(&t).unwrap()
        {
            break r;
        }
    };
    assert_eq!(ready.streams.len(), 1);
    assert_eq!(ready.streams[0].channel_id, l.id);
}

async fn stream_webhook(app: &TestApp, event: &str, ch: ChannelId, identity: &str) {
    stream_webhook_sid(app, event, ch, identity, "PA_stream").await;
}

/// `stream_webhook` for one particular connection (`sid` changes on every join).
async fn stream_webhook_sid(app: &TestApp, event: &str, ch: ChannelId, identity: &str, sid: &str) {
    let mut grant = String::new();
    if event == "participant_joined" {
        grant = grant_livekit(app, &format!("{ch}~s"), identity).await;
    }
    let body = json!({"event": event, "id": "EV_test", "room": {"name": format!("{ch}~s")},
        "participant": {"identity": identity, "sid": sid, "attributes": {"pulse_grant": grant}}})
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

async fn live_with_viewer(
    app: &TestApp,
    rx: &mut Receiver<ServerFrame>,
    l: &Channel,
    a: &(UserId, String),
    b: &(UserId, String),
) {
    go_live(app, &a.1, l.id, settings()).await;
    assert!(matches!(
        next_stream_event(rx).await,
        Event::StreamStarted { .. }
    ));
    post_json(
        app,
        &b.1,
        &format!("/streams/{}/watch", l.id),
        json!({"streamer": a.0}),
    )
    .await;
    assert!(matches!(
        next_stream_event(rx).await,
        Event::ViewersChanged { .. }
    ));
}

#[tokio::test]
async fn streamer_drop_marks_reconnecting_then_times_out() {
    let app = spawn().await;
    let (l, a, b, mut rx) = pair(&app).await;
    live_with_viewer(&app, &mut rx, &l, &a, &b).await;
    stream_webhook(&app, "participant_left", l.id, &format!("{}#v", a.0)).await;
    match next_stream_event(&mut rx).await {
        Event::StreamUpdated { stream } => assert_eq!(stream.status, StreamStatus::Reconnecting),
        other => panic!("unexpected {other:?}"),
    }
    match next_stream_event(&mut rx).await {
        Event::StreamEnded {
            user_id, reason, ..
        } => assert_eq!((user_id, reason), (a.0, StreamEndReason::TimedOut)),
        other => panic!("unexpected {other:?}"),
    }
    assert!(app.streams.all().is_empty());
}

#[tokio::test]
async fn streamer_back_in_time_stays_live() {
    let app = spawn().await;
    let (l, a, b, mut rx) = pair(&app).await;
    live_with_viewer(&app, &mut rx, &l, &a, &b).await;
    let ident = format!("{}#v", a.0);
    stream_webhook(&app, "participant_left", l.id, &ident).await;
    assert!(matches!(
        next_stream_event(&mut rx).await,
        Event::StreamUpdated { stream } if stream.status == StreamStatus::Reconnecting
    ));
    stream_webhook(&app, "participant_joined", l.id, &ident).await;
    assert!(matches!(
        next_stream_event(&mut rx).await,
        Event::StreamUpdated { stream } if stream.status == StreamStatus::Live
    ));
    let quiet = tokio::time::timeout(Duration::from_millis(600), next_stream_event(&mut rx)).await;
    assert!(quiet.is_err(), "no event expected, got {quiet:?}");
    assert_eq!(
        app.streams.get(l.id, a.0).unwrap().status,
        StreamStatus::Live
    );
}

#[tokio::test]
async fn audio_identity_events_are_ignored() {
    let app = spawn().await;
    let (l, a, b, mut rx) = pair(&app).await;
    live_with_viewer(&app, &mut rx, &l, &a, &b).await;
    stream_webhook(&app, "participant_left", l.id, &format!("{}#a", a.0)).await;
    stream_webhook(&app, "participant_left", l.id, &format!("{}#a", b.0)).await;
    stream_webhook(&app, "participant_left", l.id, "not-an-identity").await;
    let quiet = tokio::time::timeout(Duration::from_millis(600), next_stream_event(&mut rx)).await;
    assert!(quiet.is_err(), "no event expected, got {quiet:?}");
    let info = app.streams.get(l.id, a.0).unwrap();
    assert_eq!(info.status, StreamStatus::Live);
    assert_eq!(info.viewers, vec![b.0]);
}

#[tokio::test]
async fn stream_room_webhooks_never_touch_voice() {
    let app = spawn().await;
    let (l, a, _b, mut rx) = pair(&app).await;
    let before = app.voice.members(l.id);
    let stranger = register(&app, "kim").await.0;
    stream_webhook(&app, "participant_joined", l.id, &format!("{}#v", a.0)).await;
    stream_webhook(&app, "participant_joined", l.id, &format!("{stranger}#v")).await;
    stream_webhook(&app, "participant_left", l.id, &format!("{stranger}#v")).await;
    let seen = tokio::time::timeout(Duration::from_millis(400), async {
        loop {
            if let Some(ServerFrame::Event(e)) = rx.recv().await
                && matches!(e, Event::VoiceJoined { .. } | Event::VoiceLeft { .. })
            {
                return e;
            }
        }
    })
    .await;
    assert!(seen.is_err(), "voice event leaked: {seen:?}");
    assert_eq!(app.voice.members(l.id), before);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_sessions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn viewer_leaving_the_stream_room_is_unlisted() {
    let app = spawn().await;
    let (l, a, b, mut rx) = pair(&app).await;
    live_with_viewer(&app, &mut rx, &l, &a, &b).await;
    stream_webhook(&app, "participant_left", l.id, &format!("{}#v", b.0)).await;
    match next_stream_event(&mut rx).await {
        Event::ViewersChanged {
            user_id, viewers, ..
        } => {
            assert_eq!(user_id, a.0);
            assert!(viewers.is_empty());
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        app.streams.get(l.id, a.0).unwrap().status,
        StreamStatus::Live
    );
}

async fn put_preview(
    app: &TestApp,
    tok: &str,
    ch: ChannelId,
    content_type: &str,
    body: Vec<u8>,
) -> reqwest::Response {
    app.http
        .put(app.url(&format!("/streams/{ch}/preview")))
        .bearer_auth(tok)
        .header("Content-Type", content_type)
        .body(body)
        .send()
        .await
        .unwrap()
}

async fn get_preview(
    app: &TestApp,
    tok: &str,
    ch: ChannelId,
    streamer: UserId,
    if_none_match: Option<&str>,
) -> reqwest::Response {
    let mut req = app
        .http
        .get(app.url(&format!("/streams/{ch}/{streamer}/preview")))
        .bearer_auth(tok);
    if let Some(e) = if_none_match {
        req = req.header("If-None-Match", e);
    }
    req.send().await.unwrap()
}

fn rgba(w: u32, h: u32, rgb: [u8; 3]) -> RgbaImage {
    RgbaImage::from_pixel(w, h, Rgba([rgb[0], rgb[1], rgb[2], 255]))
}

/// A real (lossless) WebP of `w`×`h` in one colour.
fn webp(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    WebPEncoder::new_lossless(&mut out)
        .encode(rgba(w, h, rgb).as_raw(), w, h, ExtendedColorType::Rgba8)
        .unwrap();
    out
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    rgba(w, h, [200, 30, 30])
        .write_to(&mut out, ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

fn riff(body: Vec<u8>) -> Vec<u8> {
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

fn chunk(fourcc: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = fourcc.to_vec();
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}

/// A WebP in the extended format carrying an EXIF chunk with `marker` in it (as a camera or
/// editor would leave location or device data).
fn webp_with_exif(w: u32, h: u32, marker: &[u8]) -> Vec<u8> {
    let simple = webp(w, h, [10, 120, 200]);
    let image_chunk = &simple[12..]; // after RIFF <size> WEBP
    let mut vp8x = vec![0x18, 0, 0, 0]; // alpha + EXIF flags, 3 reserved bytes
    vp8x.extend_from_slice(&(w - 1).to_le_bytes()[..3]);
    vp8x.extend_from_slice(&(h - 1).to_le_bytes()[..3]);
    let mut exif = b"Exif\0\0".to_vec();
    exif.extend_from_slice(marker);
    let mut body = b"WEBP".to_vec();
    body.extend(chunk(b"VP8X", &vp8x));
    body.extend_from_slice(image_chunk);
    body.extend(chunk(b"EXIF", &exif));
    riff(body)
}

/// A few dozen bytes claiming a 16384×16384 lossless image: decoding it would need 1 GiB.
fn dimension_bomb() -> Vec<u8> {
    let (w, h) = (16384u32, 16384u32);
    let bits: u32 = (w - 1) | ((h - 1) << 14) | (1 << 28);
    let mut vp8l = vec![0x2f];
    vp8l.extend_from_slice(&bits.to_le_bytes());
    vp8l.extend_from_slice(&[0u8; 64]);
    let mut body = b"WEBP".to_vec();
    body.extend(chunk(b"VP8L", &vp8l));
    riff(body)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in bytes {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 == 1 {
                (c >> 1) ^ 0xEDB8_8320
            } else {
                c >> 1
            };
        }
    }
    !c
}

/// A valid PNG header claiming 100000×100000 RGBA (40 GB decoded), with a token IDAT.
fn png_bomb() -> Vec<u8> {
    let png_chunk = |kind: &[u8; 4], data: &[u8]| {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        let mut crc_in = kind.to_vec();
        crc_in.extend_from_slice(data);
        out.extend_from_slice(&crc_in);
        out.extend_from_slice(&crc32(&crc_in).to_be_bytes());
        out
    };
    let mut ihdr = 100_000u32.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&100_000u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    out.extend(png_chunk(b"IHDR", &ihdr));
    out.extend(png_chunk(
        b"IDAT",
        &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
    ));
    out.extend(png_chunk(b"IEND", &[]));
    out
}

/// A baseline JPEG frame header claiming 65535×65535.
fn jpeg_bomb() -> Vec<u8> {
    let mut out = vec![
        0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 8, 0xFF, 0xFF, 0xFF, 0xFF, 3,
    ];
    out.extend_from_slice(&[1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(rgba(w, h, [40, 80, 160]))
        .to_rgb8()
        .write_to(&mut out, ImageFormat::Jpeg)
        .unwrap();
    out.into_inner()
}

fn animated_webp() -> Vec<u8> {
    let mut enc = webp_animation::Encoder::new((32, 18)).unwrap();
    enc.add_frame(rgba(32, 18, [255, 0, 0]).as_raw(), 0)
        .unwrap();
    enc.add_frame(rgba(32, 18, [0, 0, 255]).as_raw(), 100)
        .unwrap();
    enc.finalize(200).unwrap().to_vec()
}

/// The served preview, decoded: (bytes, width, height).
async fn served(app: &TestApp, tok: &str, ch: ChannelId, streamer: UserId) -> (Vec<u8>, u32, u32) {
    let r = get_preview(app, tok, ch, streamer, None).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "image/webp");
    let bytes = r.bytes().await.unwrap().to_vec();
    assert_eq!(image::guess_format(&bytes).unwrap(), ImageFormat::WebP);
    let img = image::load_from_memory_with_format(&bytes, ImageFormat::WebP).unwrap();
    (bytes, img.width(), img.height())
}

#[tokio::test]
async fn preview_round_trips_with_etag() {
    let app = spawn().await;
    let (l, (a_id, a), (b_id, b), _rx) = pair(&app).await;
    assert_eq!(go_live(&app, &a, l.id, settings()).await.status(), 200);
    // live but no image yet
    assert_eq!(get_preview(&app, &b, l.id, a_id, None).await.status(), 404);

    let img = webp(480, 270, [20, 200, 90]);
    assert_eq!(
        put_preview(&app, &a, l.id, "image/webp", img.clone())
            .await
            .status(),
        204
    );
    // a viewer who is not in voice can still see it
    app.voice.leave(l.id, b_id, "PA_test");
    let r = get_preview(&app, &b, l.id, a_id, None).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "image/webp");
    let etag = r.headers()["etag"].to_str().unwrap().to_string();
    let (_, w, h) = served(&app, &b, l.id, a_id).await;
    assert_eq!((w, h), (480, 270));

    let r = get_preview(&app, &b, l.id, a_id, Some(&etag)).await;
    assert_eq!(r.status(), 304);
    assert_eq!(r.headers()["etag"].to_str().unwrap(), etag);

    // a new image changes the etag
    put_preview(&app, &a, l.id, "image/webp", webp(480, 270, [250, 250, 0])).await;
    let r = get_preview(&app, &b, l.id, a_id, Some(&etag)).await;
    assert_eq!(r.status(), 200);
    assert_ne!(r.headers()["etag"].to_str().unwrap(), etag);
}

#[tokio::test]
async fn preview_is_re_encoded_without_metadata() {
    let app = spawn().await;
    let (l, (a_id, a), (_, b), _rx) = pair(&app).await;
    go_live(&app, &a, l.id, settings()).await;
    let marker = b"GPS 51.5007N 0.1246W secret-device-serial";
    let upload = webp_with_exif(480, 270, marker);
    assert!(upload.windows(marker.len()).any(|w| w == marker));
    assert_eq!(
        put_preview(&app, &a, l.id, "image/webp", upload.clone())
            .await
            .status(),
        204
    );
    let (bytes, w, h) = served(&app, &b, l.id, a_id).await;
    assert_eq!((w, h), (480, 270));
    assert_ne!(bytes, upload);
    assert!(!bytes.windows(marker.len()).any(|w| w == marker));
    assert!(!bytes.windows(4).any(|w| w == b"EXIF"));
}

#[tokio::test]
async fn preview_is_fitted_within_480x270_and_never_upscaled() {
    let app = spawn().await;
    let (l, (a_id, a), (_, b), _rx) = pair(&app).await;
    go_live(&app, &a, l.id, settings()).await;
    for ((iw, ih), want) in [
        ((1920, 1080), (480, 270)),
        ((1000, 1000), (270, 270)),
        ((960, 200), (480, 100)),
        ((100, 50), (100, 50)),
    ] {
        let r = put_preview(&app, &a, l.id, "image/webp", webp(iw, ih, [9, 99, 199])).await;
        assert_eq!(r.status(), 204, "{iw}x{ih}");
        let (_, w, h) = served(&app, &b, l.id, a_id).await;
        assert_eq!((w, h), want, "{iw}x{ih}");
    }
}

#[tokio::test]
async fn png_and_jpeg_previews_are_served_as_real_webps() {
    // What a webview without WebP encoding sends instead, under its real type.
    let app = spawn().await;
    let (l, (a_id, a), (_, b), _rx) = pair(&app).await;
    go_live(&app, &a, l.id, settings()).await;
    for (ct, upload) in [("image/png", png(480, 270)), ("image/jpeg", jpeg(480, 270))] {
        let r = put_preview(&app, &a, l.id, ct, upload.clone()).await;
        assert_eq!(r.status(), 204, "{ct}");
        let (bytes, w, h) = served(&app, &b, l.id, a_id).await;
        assert_eq!((w, h), (480, 270), "{ct}");
        assert_ne!(bytes, upload, "{ct}");
    }
}

#[tokio::test]
async fn a_preview_must_be_what_its_content_type_says() {
    let app = spawn().await;
    let (l, (a_id, a), (_, b), _rx) = pair(&app).await;
    go_live(&app, &a, l.id, settings()).await;
    for (ct, body) in [
        ("image/webp", png(48, 27)),
        ("image/png", webp(48, 27, [0, 0, 0])),
        ("image/jpeg", png(48, 27)),
    ] {
        let r = put_preview(&app, &a, l.id, ct, body).await;
        assert_eq!(r.status(), 400, "{ct}");
    }
    assert_eq!(get_preview(&app, &b, l.id, a_id, None).await.status(), 404);
}

#[tokio::test]
async fn preview_rejects_unusable_images_and_stores_nothing() {
    let app = spawn().await;
    let (l, (a_id, a), (_, b), _rx) = pair(&app).await;
    go_live(&app, &a, l.id, settings()).await;
    let good = webp(480, 270, [1, 2, 3]);
    let truncated = good[..good.len() / 2].to_vec();
    let bad: [(&str, &str, Vec<u8>); 10] = [
        ("garbage", "image/webp", b"RIFF....WEBPfake".to_vec()),
        (
            "text",
            "image/webp",
            b"hello, this is not an image".to_vec(),
        ),
        ("empty", "image/webp", Vec::new()),
        ("truncated", "image/webp", truncated),
        ("webp dimension bomb", "image/webp", dimension_bomb()),
        ("png dimension bomb", "image/png", png_bomb()),
        ("jpeg dimension bomb", "image/jpeg", jpeg_bomb()),
        (
            "over the side limit",
            "image/webp",
            webp(2000, 100, [1, 2, 3]),
        ),
        ("png over the side limit", "image/png", png(100, 1921)),
        ("animated", "image/webp", animated_webp()),
    ];
    for (what, ct, body) in &bad {
        assert!(body.len() <= 100 * 1024, "{what}");
        let r = put_preview(&app, &a, l.id, ct, body.clone()).await;
        assert_eq!(r.status(), 400, "{what}");
        // nothing stored
        assert_eq!(
            get_preview(&app, &b, l.id, a_id, None).await.status(),
            404,
            "{what}"
        );
    }
    // A rejected upload leaves the last good preview in place, untouched.
    put_preview(&app, &a, l.id, "image/webp", good).await;
    let before = served(&app, &b, l.id, a_id).await.0;
    for (what, ct, body) in bad {
        let r = put_preview(&app, &a, l.id, ct, body).await;
        assert_eq!(r.status(), 400, "{what}");
        assert_eq!(served(&app, &b, l.id, a_id).await.0, before, "{what}");
    }
}

#[tokio::test]
async fn preview_needs_you_live() {
    let app = spawn().await;
    let (l, (_, a), (_, b), _rx) = pair(&app).await;
    // in voice but not live
    assert_eq!(
        put_preview(&app, &a, l.id, "image/webp", webp(48, 27, [0, 0, 0]))
            .await
            .status(),
        404
    );
    // someone else live does not make it yours
    go_live(&app, &a, l.id, settings()).await;
    assert_eq!(
        put_preview(&app, &b, l.id, "image/webp", webp(48, 27, [0, 0, 0]))
            .await
            .status(),
        404
    );
}

#[tokio::test]
async fn preview_rejects_big_or_unlisted_types() {
    let app = spawn().await;
    let (l, (a_id, a), _, _rx) = pair(&app).await;
    go_live(&app, &a, l.id, settings()).await;
    assert_eq!(
        put_preview(&app, &a, l.id, "image/gif", webp(48, 27, [0, 0, 0]))
            .await
            .status(),
        415
    );
    assert_eq!(
        put_preview(&app, &a, l.id, "text/plain", webp(48, 27, [0, 0, 0]))
            .await
            .status(),
        415
    );
    assert_eq!(
        put_preview(&app, &a, l.id, "image/webp", vec![0; 100 * 1024 + 1])
            .await
            .status(),
        413
    );
    assert_eq!(get_preview(&app, &a, l.id, a_id, None).await.status(), 404);
    // exactly the cap gets past the size check (and is then refused as not an image)
    assert_eq!(
        put_preview(&app, &a, l.id, "image/webp", vec![0; 100 * 1024])
            .await
            .status(),
        400
    );
}

#[tokio::test]
async fn preview_uploads_are_rate_limited_per_user() {
    let mut cfg = test_config();
    cfg.limits.preview_user = Rate::new(2, 0.0);
    let app = spawn_with(cfg).await;
    let (l, (_, a), _, _rx) = pair(&app).await;
    go_live(&app, &a, l.id, settings()).await;
    for _ in 0..2 {
        let r = put_preview(&app, &a, l.id, "image/webp", webp(48, 27, [0, 0, 0])).await;
        assert_eq!(r.status(), 204);
    }
    let r = put_preview(&app, &a, l.id, "image/webp", webp(48, 27, [0, 0, 0])).await;
    assert_eq!(r.status(), 429);
}

#[test]
fn default_preview_limit_allows_the_client_cadence() {
    // The client sends one 1 s after going live, then one every 15 s; a few restarts in a row
    // must not trip it.
    let r = LimitsConfig::default().preview_user;
    assert!(r.burst >= 4);
    assert!(r.refill_per_sec >= 1.0 / 15.0);
    assert!(r.refill_per_sec <= 1.0);
}

#[tokio::test]
async fn preview_is_gone_after_stop() {
    let app = spawn().await;
    let (l, (a_id, a), (_, b), _rx) = pair(&app).await;
    go_live(&app, &a, l.id, settings()).await;
    put_preview(&app, &a, l.id, "image/webp", webp(48, 27, [0, 0, 0])).await;
    assert_eq!(get_preview(&app, &b, l.id, a_id, None).await.status(), 200);
    assert_eq!(stop(&app, &a, l.id).await.status(), 204);
    assert_eq!(get_preview(&app, &b, l.id, a_id, None).await.status(), 404);
    // going live again starts without the old image
    go_live(&app, &a, l.id, settings()).await;
    assert_eq!(get_preview(&app, &b, l.id, a_id, None).await.status(), 404);
}

#[tokio::test]
async fn preview_needs_server_membership() {
    let app = spawn().await;
    let (l, (a_id, a), _, _rx) = pair(&app).await;
    go_live(&app, &a, l.id, settings()).await;
    put_preview(&app, &a, l.id, "image/webp", webp(48, 27, [0, 0, 0])).await;
    let (_, outsider) = register(&app, "kai").await;
    assert_eq!(
        get_preview(&app, &outsider, l.id, a_id, None)
            .await
            .status(),
        403
    );
}

/// LiveKit kicks the old connection when the same identity joins again, and that old
/// connection's leave can land after the new one's join: it must not touch the current state.
#[tokio::test]
async fn a_late_leave_for_a_replaced_stream_connection_is_ignored() {
    let app = spawn().await;
    let (l, a, b, mut rx) = pair(&app).await;
    live_with_viewer(&app, &mut rx, &l, &a, &b).await;
    let (streamer, viewer) = (format!("{}#v", a.0), format!("{}#v", b.0));
    stream_webhook_sid(&app, "participant_joined", l.id, &streamer, "PA_a1").await;
    stream_webhook_sid(&app, "participant_joined", l.id, &viewer, "PA_b1").await;
    // both reconnect; the old connections' leaves arrive late
    stream_webhook_sid(&app, "participant_joined", l.id, &streamer, "PA_a2").await;
    stream_webhook_sid(&app, "participant_joined", l.id, &viewer, "PA_b2").await;
    stream_webhook_sid(&app, "participant_left", l.id, &streamer, "PA_a1").await;
    stream_webhook_sid(&app, "participant_left", l.id, &viewer, "PA_b1").await;
    let quiet = tokio::time::timeout(Duration::from_millis(600), next_stream_event(&mut rx)).await;
    assert!(quiet.is_err(), "no event expected, got {quiet:?}");
    let info = app.streams.get(l.id, a.0).unwrap();
    assert_eq!(info.status, StreamStatus::Live);
    assert_eq!(info.viewers, vec![b.0]);
    // the current connection's leave still counts
    stream_webhook_sid(&app, "participant_left", l.id, &streamer, "PA_a2").await;
    assert!(matches!(
        next_stream_event(&mut rx).await,
        Event::StreamUpdated { stream } if stream.status == StreamStatus::Reconnecting
    ));
}

// --- liveness check: LiveKit says whether a live streamer's `#v` is still in the stream room ---

mod liveness {
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use livekit_protocol as lkp;
    use pulse_server::streams::liveness::{
        Checked, LiveKitRooms, StreamRooms, check, spawn as spawn_checks,
    };

    use super::common::livekit::*;
    use super::*;

    const GRACE: Duration = Duration::from_secs(60);

    /// LiveKit's answers per room: the identities connected, or `None` for "couldn't ask".
    #[derive(Default)]
    struct FakeRooms(Mutex<HashMap<String, Option<HashSet<String>>>>);

    impl FakeRooms {
        fn set(&self, ch: ChannelId, answer: Option<&[String]>) {
            self.0.lock().unwrap().insert(
                format!("{ch}~s"),
                answer.map(|ids| ids.iter().cloned().collect()),
            );
        }
    }

    impl StreamRooms for FakeRooms {
        async fn connected(&self, room: &str) -> anyhow::Result<HashSet<String>> {
            match self.0.lock().unwrap().get(room) {
                Some(Some(ids)) => Ok(ids.clone()),
                Some(None) => anyhow::bail!("LiveKit unreachable"),
                None => Ok(HashSet::new()), // no such room
            }
        }
    }

    /// (streams ended, rooms LiveKit couldn't be asked about)
    fn counts(c: &Checked) -> (usize, usize) {
        (c.ended, c.unknown.len())
    }

    fn v(user: UserId) -> String {
        format!("{user}#v")
    }

    /// Stream events already queued (none waited for).
    fn queued(rx: &mut Receiver<ServerFrame>) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(f) = rx.try_recv() {
            if let ServerFrame::Event(
                e @ (Event::StreamStarted { .. }
                | Event::StreamUpdated { .. }
                | Event::StreamEnded { .. }
                | Event::ViewersChanged { .. }),
            ) = f
            {
                out.push(e);
            }
        }
        out
    }

    /// alex and sam both live in the lounge, each watching the other.
    async fn both_live_and_watching(
        app: &TestApp,
    ) -> (ChannelId, UserId, UserId, Receiver<ServerFrame>) {
        let (l, (a_id, a), (b_id, b), mut rx) = pair(app).await;
        for tok in [&a, &b] {
            assert_eq!(go_live(app, tok, l.id, settings()).await.status(), 200);
        }
        let watch = format!("/streams/{}/watch", l.id);
        assert_eq!(
            post_json(app, &b, &watch, json!({"streamer": a_id}))
                .await
                .status(),
            204
        );
        assert_eq!(
            post_json(app, &a, &watch, json!({"streamer": b_id}))
                .await
                .status(),
            204
        );
        for _ in 0..4 {
            next_stream_event(&mut rx).await; // two starts, two viewer changes
        }
        assert!(queued(&mut rx).is_empty());
        (l.id, a_id, b_id, rx)
    }

    #[tokio::test]
    async fn a_streamer_missing_from_the_room_for_the_grace_is_ended_with_the_watches() {
        let app = spawn().await;
        let (ch, a_id, b_id, mut rx) = both_live_and_watching(&app).await;
        let s = app.state();
        let rooms = FakeRooms::default();
        rooms.set(ch, Some(&[v(b_id), format!("{a_id}#a")])); // alex's audio alone isn't the stream
        let t0 = Instant::now();

        assert_eq!(counts(&check(&s, &rooms, t0, GRACE).await), (0, 0));
        let almost = t0 + GRACE - Duration::from_millis(1);
        assert_eq!(counts(&check(&s, &rooms, almost, GRACE).await), (0, 0));
        assert!(queued(&mut rx).is_empty(), "nothing ends within the grace");
        assert!(app.streams.get(ch, a_id).is_some());

        assert_eq!(counts(&check(&s, &rooms, t0 + GRACE, GRACE).await), (1, 0));
        let events = queued(&mut rx);
        assert!(
            events.iter().any(|e| matches!(e, Event::StreamEnded { user_id, reason: StreamEndReason::TimedOut, .. } if *user_id == a_id)),
            "{events:?}"
        );
        // alex's watch of sam went with the connection
        assert!(
            events.iter().any(|e| matches!(e, Event::ViewersChanged { user_id, viewers, .. } if *user_id == b_id && viewers.is_empty())),
            "{events:?}"
        );
        assert!(app.streams.get(ch, a_id).is_none());
        assert!(app.streams.get(ch, b_id).is_some(), "sam is still there");
    }

    #[tokio::test]
    async fn a_streamer_livekit_has_stays_live() {
        let app = spawn().await;
        let (ch, a_id, b_id, mut rx) = both_live_and_watching(&app).await;
        let rooms = FakeRooms::default();
        rooms.set(ch, Some(&[v(a_id), v(b_id)]));
        let t0 = Instant::now();
        for n in 0..10 {
            assert_eq!(
                counts(&check(&app.state(), &rooms, t0 + GRACE * n, GRACE).await),
                (0, 0)
            );
        }
        assert!(queued(&mut rx).is_empty());
        assert_eq!(app.streams.all().len(), 2);
    }

    #[tokio::test]
    async fn back_in_the_room_within_the_grace_stops_the_clock() {
        let app = spawn().await;
        let (ch, a_id, b_id, _rx) = both_live_and_watching(&app).await;
        let rooms = FakeRooms::default();
        let t0 = Instant::now();
        rooms.set(ch, Some(&[v(b_id)]));
        check(&app.state(), &rooms, t0, GRACE).await;
        rooms.set(ch, Some(&[v(a_id), v(b_id)]));
        check(&app.state(), &rooms, t0 + GRACE / 2, GRACE).await;
        rooms.set(ch, Some(&[v(b_id)]));
        assert_eq!(
            counts(&check(&app.state(), &rooms, t0 + GRACE, GRACE).await),
            (0, 0)
        );
        assert!(app.streams.get(ch, a_id).is_some());
    }

    #[tokio::test]
    async fn livekit_that_cannot_be_asked_ends_nothing() {
        let app = spawn().await;
        let (ch, _a_id, _b_id, mut rx) = both_live_and_watching(&app).await;
        let rooms = FakeRooms::default();
        rooms.set(ch, None);
        let t0 = Instant::now();
        for n in 0..10 {
            assert_eq!(
                counts(&check(&app.state(), &rooms, t0 + GRACE * n, GRACE).await),
                (0, 1)
            );
        }
        assert!(queued(&mut rx).is_empty());
        assert_eq!(app.streams.all().len(), 2);
        // and the first answer after the outage only starts the clock
        rooms.set(ch, Some(&[]));
        assert_eq!(
            counts(&check(&app.state(), &rooms, t0 + GRACE * 10, GRACE).await),
            (0, 0)
        );
    }

    /// alex live in the lounge, then the leave webhook: `Reconnecting`, with `media_timeout` to
    /// come back.
    async fn reconnecting(
        media_timeout: Duration,
    ) -> (TestApp, ChannelId, UserId, Receiver<ServerFrame>) {
        let mut cfg = test_config();
        cfg.stream_media_timeout = media_timeout;
        let app = spawn_with(cfg).await;
        let (l, (a_id, a), _, mut rx) = pair(&app).await;
        assert_eq!(go_live(&app, &a, l.id, settings()).await.status(), 200);
        stream_webhook(&app, "participant_left", l.id, &v(a_id)).await;
        assert!(matches!(
            next_stream_event(&mut rx).await,
            Event::StreamStarted { .. }
        ));
        assert!(matches!(
            next_stream_event(&mut rx).await,
            Event::StreamUpdated { stream } if stream.status == StreamStatus::Reconnecting
        ));
        (app, l.id, a_id, rx)
    }

    #[tokio::test]
    async fn a_reconnecting_stream_missing_from_the_room_is_left_to_its_own_timer() {
        let (app, ch, a_id, mut rx) = reconnecting(Duration::from_secs(600)).await;
        let rooms = FakeRooms::default();
        let t0 = Instant::now();
        for n in 0..3 {
            let c = check(&app.state(), &rooms, t0 + GRACE * n, GRACE).await;
            assert_eq!(counts(&c), (0, 0));
        }
        assert_eq!(
            app.streams.get(ch, a_id).unwrap().status,
            StreamStatus::Reconnecting
        );
        assert!(queued(&mut rx).is_empty());
    }

    #[tokio::test]
    async fn a_reconnecting_stream_livekit_lists_again_is_live_again_and_its_timer_stands_down() {
        // the leave webhook came, the rejoin's join webhook didn't
        let (app, ch, a_id, mut rx) = reconnecting(Duration::from_millis(300)).await;
        let rooms = FakeRooms::default();
        rooms.set(ch, Some(&[v(a_id)]));
        check(&app.state(), &rooms, Instant::now(), GRACE).await;
        assert!(matches!(
            next_stream_event(&mut rx).await,
            Event::StreamUpdated { stream } if stream.user_id == a_id && stream.status == StreamStatus::Live
        ));
        tokio::time::sleep(Duration::from_millis(500)).await; // past the leave's timer
        assert_eq!(
            app.streams.get(ch, a_id).unwrap().status,
            StreamStatus::Live
        );
        assert!(queued(&mut rx).is_empty(), "no end, no second update");
    }

    fn participant(identity: String, state: lkp::participant_info::State) -> lkp::ParticipantInfo {
        lkp::ParticipantInfo {
            identity,
            sid: "PA_x".into(),
            state: state as i32,
            ..Default::default()
        }
    }

    /// The app's state with LiveKit's API at `url`.
    fn state_with(app: &TestApp, url: String) -> pulse_server::AppState {
        let mut s = app.state();
        s.cfg = Arc::new(pulse_server::config::Config {
            livekit_api_url: url,
            ..(*app.cfg).clone()
        });
        s
    }

    #[tokio::test]
    async fn after_a_server_restart_reannounced_streams_are_kept_by_livekits_answer() {
        // A restarted server knows no streams and no `#v` connections; LiveKit (not restarted)
        // still has them, and the clients re-announce. A fresh app's empty state is that
        // restart: what's checked here is that LiveKit's answer, through the real API client,
        // keeps the re-announced streams.
        let app = spawn().await;
        let (l, (a_id, a), (b_id, b), mut rx) = pair(&app).await;
        let room = format!("{}~s", l.id);
        let active = lkp::participant_info::State::Active;
        let live = SharedRooms::default();
        live.lock().unwrap().push((
            room.clone(),
            vec![participant(v(a_id), active), participant(v(b_id), active)],
        ));
        let s = state_with(&app, fake_livekit_shared(live.clone()).await);
        let rooms = LiveKitRooms::new(&s.cfg);
        for tok in [&a, &b] {
            assert_eq!(go_live(&app, tok, l.id, settings()).await.status(), 200); // the re-announce
        }
        let t0 = Instant::now();
        for n in 0..3 {
            assert_eq!(
                counts(&check(&s, &rooms, t0 + GRACE * n, GRACE).await),
                (0, 0)
            );
        }
        assert_eq!(app.streams.all().len(), 2);
        queued(&mut rx);

        // alex's client dies: LiveKit is still tearing the connection down (it doesn't count)
        live.lock().unwrap()[0].1[0].state = lkp::participant_info::State::Disconnected as i32;
        assert_eq!(
            counts(&check(&s, &rooms, t0 + GRACE * 3, GRACE).await),
            (0, 0)
        );
        assert_eq!(
            counts(&check(&s, &rooms, t0 + GRACE * 4, GRACE).await),
            (1, 0)
        );
        assert!(app.streams.get(l.id, a_id).is_none());
        assert!(app.streams.get(l.id, b_id).is_some());
        assert!(
            queued(&mut rx)
                .iter()
                .any(|e| matches!(e, Event::StreamEnded { user_id, .. } if *user_id == a_id))
        );

        // then sam's: LiveKit closes the room, which then lists nobody
        live.lock().unwrap().clear();
        assert_eq!(
            counts(&check(&s, &rooms, t0 + GRACE * 5, GRACE).await),
            (0, 0)
        );
        assert_eq!(
            counts(&check(&s, &rooms, t0 + GRACE * 6, GRACE).await),
            (1, 0)
        );
        assert!(app.streams.all().is_empty());
    }

    #[tokio::test]
    async fn the_real_client_treats_an_unreachable_livekit_as_dont_know() {
        let app = spawn().await; // its LiveKit API URL leads nowhere
        let (l, (a_id, a), _, _rx) = pair(&app).await;
        go_live(&app, &a, l.id, settings()).await;
        let rooms = LiveKitRooms::new(&app.cfg);
        let t0 = Instant::now();
        for n in 0..3 {
            assert_eq!(
                counts(&check(&app.state(), &rooms, t0 + GRACE * n, GRACE).await),
                (0, 1)
            );
        }
        assert!(app.streams.get(l.id, a_id).is_some());
    }

    /// A RoomService whose ListParticipants answers `status` with a Twirp `code`, and whose
    /// ListRooms works (LiveKit is up; this one question failed).
    async fn fake_livekit_refusing(status: u16, code: &'static str) -> String {
        let app = axum::Router::new().route(
            "/twirp/livekit.RoomService/ListParticipants",
            axum::routing::post(move || async move {
                (
                    axum::http::StatusCode::from_u16(status).unwrap(),
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    format!(r#"{{"code":"{code}","msg":"refused"}}"#),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn livekit_errors_and_silence_are_dont_know_and_say_why() {
        let app = spawn().await;
        let (l, (a_id, a), _, _rx) = pair(&app).await;
        go_live(&app, &a, l.id, settings()).await;
        let t0 = Instant::now();
        for (url, why) in [
            (fake_livekit_refusing(503, "unavailable").await, "refused"),
            (fake_livekit_refusing(500, "internal").await, "refused"),
            (
                fake_livekit_refusing(401, "unauthenticated").await,
                "refused",
            ),
            (fake_livekit_hanging().await, "time"), // "timed out" or "didn't answer in time"
        ] {
            let s = state_with(&app, url);
            let rooms = LiveKitRooms::with_budget(&s.cfg, Duration::from_millis(200));
            for n in 0..3 {
                let c = check(&s, &rooms, t0 + GRACE * n, GRACE).await;
                assert_eq!(counts(&c), (0, 1), "{c:?}");
                assert!(c.unknown[0].1.contains(why), "{c:?}");
            }
        }
        assert!(app.streams.get(l.id, a_id).is_some());
    }

    #[tokio::test]
    async fn the_background_check_ends_a_stream_livekit_has_lost_and_keeps_the_others() {
        let app = spawn().await;
        let (l, (a_id, a), (b_id, b), mut rx) = pair(&app).await;
        for tok in [&a, &b] {
            assert_eq!(go_live(&app, tok, l.id, settings()).await.status(), 200);
        }
        let live = SharedRooms::default();
        live.lock().unwrap().push((
            format!("{}~s", l.id),
            vec![participant(v(b_id), lkp::participant_info::State::Active)],
        ));
        let s = state_with(&app, fake_livekit_shared(live).await);
        spawn_checks(s, Duration::from_millis(50), Duration::from_millis(200));
        let ended = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Event::StreamEnded {
                    user_id, reason, ..
                } = next_stream_event(&mut rx).await
                {
                    return (user_id, reason);
                }
            }
        })
        .await
        .expect("the background check never ended alex's stream");
        assert_eq!(ended, (a_id, StreamEndReason::TimedOut));
        tokio::time::sleep(Duration::from_millis(400)).await; // more rounds: sam stays
        assert!(app.streams.get(l.id, b_id).is_some());
    }
}

#[tokio::test]
async fn an_end_that_a_new_go_live_overtook_is_followed_by_the_stream_again() {
    // The ending path removed the stream, then (before its StreamEnded went out) the same person
    // went live again: clients must end up seeing the new stream, whatever order they got it in.
    let app = spawn().await;
    let (l, (a_id, a), _, mut rx) = pair(&app).await;
    assert_eq!(go_live(&app, &a, l.id, settings()).await.status(), 200);
    assert!(matches!(
        next_stream_event(&mut rx).await,
        Event::StreamStarted { .. }
    ));
    pulse_server::streams::routes::publish_ended(
        &app.state(),
        l.id,
        a_id,
        StreamEndReason::TimedOut,
    )
    .await;
    assert!(matches!(
        next_stream_event(&mut rx).await,
        Event::StreamEnded { user_id, .. } if user_id == a_id
    ));
    assert!(matches!(
        next_stream_event(&mut rx).await,
        Event::StreamUpdated { stream } if stream.user_id == a_id && stream.status == StreamStatus::Live
    ));
    // an end with no stream behind it is just the end
    app.streams.end(l.id, a_id);
    pulse_server::streams::routes::publish_ended(
        &app.state(),
        l.id,
        a_id,
        StreamEndReason::Stopped,
    )
    .await;
    assert!(matches!(
        next_stream_event(&mut rx).await,
        Event::StreamEnded { .. }
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(200), next_stream_event(&mut rx))
            .await
            .is_err()
    );
}
