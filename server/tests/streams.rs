mod common;

use std::time::Duration;

use base64::Engine;
use common::*;
use livekit_api::access_token::{AccessToken, TokenVerifier};
use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{
    Channel, StreamEndReason, StreamInfo, StreamSettings, StreamStatus, VoiceTokenResponse,
};
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
    let body = json!({"event": event, "id": "EV_test", "room": {"name": format!("{ch}~s")},
        "participant": {"identity": identity, "sid": "PA_stream"}})
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

#[tokio::test]
async fn preview_round_trips_with_etag() {
    let app = spawn().await;
    let (l, (a_id, a), (b_id, b), _rx) = pair(&app).await;
    assert_eq!(go_live(&app, &a, l.id, settings()).await.status(), 200);
    // live but no image yet
    assert_eq!(get_preview(&app, &b, l.id, a_id, None).await.status(), 404);

    let img = b"RIFF....WEBPfake".to_vec();
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
    assert_eq!(r.bytes().await.unwrap().to_vec(), img);

    let r = get_preview(&app, &b, l.id, a_id, Some(&etag)).await;
    assert_eq!(r.status(), 304);
    assert_eq!(r.headers()["etag"].to_str().unwrap(), etag);

    // a new image changes the etag
    put_preview(&app, &a, l.id, "image/webp", b"other".to_vec()).await;
    let r = get_preview(&app, &b, l.id, a_id, Some(&etag)).await;
    assert_eq!(r.status(), 200);
    assert_ne!(r.headers()["etag"].to_str().unwrap(), etag);
}

#[tokio::test]
async fn preview_needs_you_live() {
    let app = spawn().await;
    let (l, (_, a), (_, b), _rx) = pair(&app).await;
    // in voice but not live
    assert_eq!(
        put_preview(&app, &a, l.id, "image/webp", vec![1, 2, 3])
            .await
            .status(),
        404
    );
    // someone else live does not make it yours
    go_live(&app, &a, l.id, settings()).await;
    assert_eq!(
        put_preview(&app, &b, l.id, "image/webp", vec![1, 2, 3])
            .await
            .status(),
        404
    );
}

#[tokio::test]
async fn preview_rejects_big_or_non_webp() {
    let app = spawn().await;
    let (l, (a_id, a), _, _rx) = pair(&app).await;
    go_live(&app, &a, l.id, settings()).await;
    assert_eq!(
        put_preview(&app, &a, l.id, "image/png", vec![1, 2, 3])
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
    // exactly the cap is fine
    assert_eq!(
        put_preview(&app, &a, l.id, "image/webp", vec![0; 100 * 1024])
            .await
            .status(),
        204
    );
}

#[tokio::test]
async fn preview_is_gone_after_stop() {
    let app = spawn().await;
    let (l, (a_id, a), (_, b), _rx) = pair(&app).await;
    go_live(&app, &a, l.id, settings()).await;
    put_preview(&app, &a, l.id, "image/webp", vec![1, 2, 3]).await;
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
    put_preview(&app, &a, l.id, "image/webp", vec![1, 2, 3]).await;
    let (_, outsider) = register(&app, "kai").await;
    assert_eq!(
        get_preview(&app, &outsider, l.id, a_id, None)
            .await
            .status(),
        403
    );
}
