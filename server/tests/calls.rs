//! Calls in DMs and groups, with ringing (#50).

mod common;

use std::time::Duration;

use base64::Engine;
use common::*;
use futures::SinkExt;
use livekit_api::access_token::AccessToken;
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{Channel, Message, RingEndReason, StartCallResponse};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::Message as Ws;

const QUIET: Duration = Duration::from_millis(400);

async fn hello(app: &TestApp, token: &str) -> (Socket, Ready) {
    let mut ws = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap()
        .0;
    ws.send(Ws::text(
        serde_json::to_string(&ClientFrame::Hello {
            token: token.into(),
            client_version: pulse_protocol::PROTOCOL_VERSION,
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    loop {
        match next_frame_beating(&mut ws, Duration::from_secs(3)).await {
            Some(ServerFrame::Ready(r)) => return (ws, r),
            Some(_) => continue,
            None => panic!("no Ready"),
        }
    }
}

/// Every event until the socket has been quiet for `quiet`, without presence noise.
async fn drain_for(ws: &mut Socket, quiet: Duration) -> Vec<Event> {
    let mut out = vec![];
    while let Some(f) = next_frame_beating(ws, quiet).await {
        if let ServerFrame::Event(e) = f
            && !matches!(e, Event::PresenceChanged { .. } | Event::UserCreated { .. })
        {
            out.push(e);
        }
    }
    out
}

async fn drain(ws: &mut Socket) -> Vec<Event> {
    drain_for(ws, QUIET).await
}

fn is_call_event(e: &Event) -> bool {
    matches!(
        e,
        Event::CallStarted { .. }
            | Event::CallEnded { .. }
            | Event::CallRinging { .. }
            | Event::CallRingEnded { .. }
            | Event::VoiceJoined { .. }
            | Event::VoiceLeft { .. }
            | Event::MessageCreated { .. }
            | Event::MessageUpdated { .. }
    )
}

fn rings(events: &[Event]) -> Vec<&pulse_protocol::rest::CallRing> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::CallRinging { ring } => Some(ring),
            _ => None,
        })
        .collect()
}

fn ring_ends(events: &[Event]) -> Vec<(UserId, RingEndReason)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::CallRingEnded {
                user_id, reason, ..
            } => Some((*user_id, *reason)),
            _ => None,
        })
        .collect()
}

fn signed(app: &TestApp, body: &str) -> String {
    let sum = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body.as_bytes()));
    AccessToken::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .with_sha256(&sum)
        .to_jwt()
        .unwrap()
}

/// A LiveKit participant webhook (`participant_joined` carries a live grant).
async fn lk(app: &TestApp, event: &str, room: ChannelId, user: UserId, sid: &str) {
    let mut grant = String::new();
    if event == "participant_joined" {
        grant = grant_livekit(app, &room.to_string(), &user.to_string()).await;
    }
    let body = json!({"event": event, "id": "EV", "room": {"name": room.to_string()},
        "participant": {"identity": user.to_string(), "sid": sid, "attributes": {"pulse_grant": grant}}})
    .to_string();
    let r = app
        .http
        .post(app.url("/livekit/webhook"))
        .header("Authorization", signed(app, &body))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

async fn start(app: &TestApp, token: &str, channel: ChannelId) -> reqwest::Response {
    post_json(app, token, &format!("/calls/{channel}"), json!({})).await
}

async fn start_ok(app: &TestApp, token: &str, channel: ChannelId) -> StartCallResponse {
    let r = start(app, token, channel).await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

async fn decline(app: &TestApp, token: &str, channel: ChannelId) -> u16 {
    post_json(app, token, &format!("/calls/{channel}/decline"), json!({}))
        .await
        .status()
        .as_u16()
}

/// A short ring and grace, for tests that wait them out.
async fn spawn_fast() -> TestApp {
    let mut cfg = test_config();
    cfg.calls.ring = Duration::from_millis(500);
    cfg.calls.grace = Duration::from_millis(500);
    spawn_with(cfg).await
}

struct People {
    alex: (UserId, String),
    sam: (UserId, String),
    jo: (UserId, String),
    kim: (UserId, String),
}

async fn people(app: &TestApp) -> People {
    People {
        alex: register(app, "alex").await,
        sam: register(app, "sam").await,
        jo: register(app, "jo").await,
        kim: register(app, "kim").await,
    }
}

async fn lines(app: &TestApp, token: &str, channel: ChannelId) -> Vec<Message> {
    let page: Vec<Message> = get_json(app, token, &format!("/channels/{channel}/messages")).await;
    page.into_iter().filter(|m| m.call.is_some()).collect()
}

#[tokio::test]
async fn ringing_reaches_members_only() {
    let app = spawn().await;
    let p = people(&app).await;
    let dm: Channel = create_dm(&app, &p.alex.1, &[p.sam.0]).await;
    let (mut a, _) = hello(&app, &p.alex.1).await;
    let (mut s, _) = hello(&app, &p.sam.1).await;
    let (mut k, _) = hello(&app, &p.kim.1).await;
    let r = start_ok(&app, &p.alex.1, dm.id).await;
    assert!(r.rang);
    assert_eq!(r.call.started_by, Some(p.alex.0));
    let sam = drain(&mut s).await;
    let got = rings(&sam);
    assert_eq!(got.len(), 1, "{sam:?}");
    assert_eq!((got[0].caller_id, got[0].channel_id), (p.alex.0, dm.id));
    let expires = chrono::DateTime::parse_from_rfc3339(&got[0].expires_at).unwrap();
    let left = expires
        .signed_duration_since(chrono::Utc::now())
        .num_seconds();
    assert!((25..=30).contains(&left), "rings for 30 s, {left} left");
    assert!(sam.iter().any(|e| matches!(e, Event::CallStarted { .. })));
    let alex = drain(&mut a).await;
    assert!(rings(&alex).is_empty(), "never the caller");
    assert!(alex.iter().any(|e| matches!(e, Event::CallStarted { .. })));
    assert!(
        drain(&mut k).await.iter().all(|e| !is_call_event(e)),
        "not an outsider"
    );

    // a group rings everyone but the caller
    let group = create_dm(&app, &p.sam.1, &[p.alex.0, p.jo.0]).await;
    let (mut j, _) = hello(&app, &p.jo.1).await;
    drain(&mut a).await;
    drain(&mut s).await;
    start_ok(&app, &p.sam.1, group.id).await;
    assert_eq!(rings(&drain(&mut a).await).len(), 1);
    assert_eq!(rings(&drain(&mut j).await).len(), 1);
    assert!(rings(&drain(&mut s).await).is_empty());
    assert!(drain(&mut k).await.iter().all(|e| !is_call_event(e)));
}

#[tokio::test]
async fn starting_rings_once_and_joining_rings_nobody() {
    let app = spawn().await;
    let p = people(&app).await;
    let dm = create_dm(&app, &p.alex.1, &[p.sam.0]).await;
    let (mut s, _) = hello(&app, &p.sam.1).await;
    let first = start_ok(&app, &p.alex.1, dm.id).await;
    drain(&mut s).await;
    let again = start_ok(&app, &p.sam.1, dm.id).await;
    assert!(!again.rang, "a call already going rings nobody");
    assert_eq!(again.call.id, first.call.id);
    let (mut a, _) = hello(&app, &p.alex.1).await;
    assert!(rings(&drain(&mut a).await).is_empty());
    assert_eq!(lines(&app, &p.alex.1, dm.id).await.len(), 1, "one line");
}

#[tokio::test]
async fn server_channels_never_ring_or_make_calls() {
    let app = spawn().await;
    let p = people(&app).await;
    let srv = create_server(&app, &p.alex.1, "Main").await;
    let chans: Vec<Channel> =
        get_json(&app, &p.alex.1, &format!("/servers/{}/channels", srv.id)).await;
    let lounge = chans
        .iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    assert_eq!(start(&app, &p.alex.1, lounge.id).await.status(), 400);
    let (mut a, _) = hello(&app, &p.alex.1).await;
    lk(&app, "participant_joined", lounge.id, p.alex.0, "PA1").await;
    let ev = drain(&mut a).await;
    assert!(ev.iter().any(|e| matches!(e, Event::VoiceJoined { .. })));
    assert!(
        !ev.iter()
            .any(|e| matches!(e, Event::CallStarted { .. } | Event::MessageCreated { .. })),
        "{ev:?}"
    );
    // an outsider can't start one in someone else's DM
    let dm = create_dm(&app, &p.alex.1, &[p.sam.0]).await;
    assert_eq!(start(&app, &p.kim.1, dm.id).await.status(), 404);
    assert_eq!(decline(&app, &p.kim.1, dm.id).await, 404);
}

#[tokio::test]
async fn a_ring_times_out_everywhere() {
    let app = spawn_fast().await;
    let p = people(&app).await;
    let dm = create_dm(&app, &p.alex.1, &[p.sam.0]).await;
    // sam's call lasts beyond the grace: alex is in it, so it doesn't end first
    let (mut s1, _) = hello(&app, &p.sam.1).await;
    let (mut s2, _) = hello(&app, &p.sam.1).await;
    start_ok(&app, &p.alex.1, dm.id).await;
    lk(&app, "participant_joined", dm.id, p.alex.0, "PA1").await;
    for ws in [&mut s1, &mut s2] {
        let ev = drain_for(ws, Duration::from_millis(900)).await;
        assert_eq!(
            ring_ends(&ev),
            vec![(p.sam.0, RingEndReason::TimedOut)],
            "{ev:?}"
        );
    }
}

#[tokio::test]
async fn decline_ends_my_ring_only_and_tells_nobody() {
    let app = spawn().await;
    let p = people(&app).await;
    let group = create_dm(&app, &p.alex.1, &[p.sam.0, p.jo.0]).await;
    let (mut a, _) = hello(&app, &p.alex.1).await;
    let (mut s1, _) = hello(&app, &p.sam.1).await;
    let (mut s2, _) = hello(&app, &p.sam.1).await;
    let (mut j, _) = hello(&app, &p.jo.1).await;
    start_ok(&app, &p.alex.1, group.id).await;
    for ws in [&mut a, &mut s1, &mut s2, &mut j] {
        drain(ws).await;
    }
    assert_eq!(decline(&app, &p.sam.1, group.id).await, 204);
    for ws in [&mut s1, &mut s2] {
        assert_eq!(
            ring_ends(&drain(ws).await),
            vec![(p.sam.0, RingEndReason::Declined)]
        );
    }
    assert!(drain(&mut a).await.is_empty(), "the caller is never told");
    assert!(drain(&mut j).await.is_empty());
    // declining again (or with no ring) is quiet
    assert_eq!(decline(&app, &p.sam.1, group.id).await, 204);
    assert!(drain(&mut s1).await.is_empty());
}

#[tokio::test]
async fn answering_on_one_device_stops_the_other() {
    let app = spawn().await;
    let p = people(&app).await;
    let dm = create_dm(&app, &p.alex.1, &[p.sam.0]).await;
    let (mut s1, _) = hello(&app, &p.sam.1).await;
    let (mut s2, _) = hello(&app, &p.sam.1).await;
    start_ok(&app, &p.alex.1, dm.id).await;
    drain(&mut s1).await;
    drain(&mut s2).await;
    lk(&app, "participant_joined", dm.id, p.sam.0, "PS1").await;
    for ws in [&mut s1, &mut s2] {
        assert_eq!(
            ring_ends(&drain(ws).await),
            vec![(p.sam.0, RingEndReason::Answered)]
        );
    }
}

#[tokio::test]
async fn the_call_ending_ends_every_ring_and_the_line_says_who_joined() {
    let app = spawn_fast().await;
    let p = people(&app).await;
    let group = create_dm(&app, &p.alex.1, &[p.sam.0, p.jo.0]).await;
    let (mut a, _) = hello(&app, &p.alex.1).await;
    let (mut j, _) = hello(&app, &p.jo.1).await;
    let call = start_ok(&app, &p.alex.1, group.id).await.call;
    lk(&app, "participant_joined", group.id, p.alex.0, "PA1").await;
    lk(&app, "participant_joined", group.id, p.sam.0, "PS1").await;
    lk(&app, "participant_left", group.id, p.sam.0, "PS1").await;
    // the ring outlives the call here: ring 500 ms from the start, everyone gone at once
    lk(&app, "participant_left", group.id, p.alex.0, "PA1").await;
    let jo = drain_for(&mut j, Duration::from_millis(1200)).await;
    let ended = ring_ends(&jo);
    assert_eq!(ended.len(), 1, "{jo:?}");
    assert!(
        matches!(
            ended[0].1,
            RingEndReason::CallEnded | RingEndReason::TimedOut
        ),
        "{ended:?}"
    );
    let alex = drain_for(&mut a, Duration::from_millis(1200)).await;
    assert!(alex.iter().any(
        |e| matches!(e, Event::CallEnded { call_id, channel_id } if *call_id == call.id && *channel_id == group.id)
    ));
    let updated = alex
        .iter()
        .find_map(|e| match e {
            Event::MessageUpdated { message } if message.call.is_some() => Some(message.clone()),
            _ => None,
        })
        .expect("the line is updated");
    let line = updated.call.unwrap();
    assert!(line.ended_at.is_some());
    let mut joined = line.joined.clone();
    joined.sort();
    let mut want = vec![p.alex.0, p.sam.0];
    want.sort();
    assert_eq!(joined, want, "jo missed it");
    // once, stored
    let stored = lines(&app, &p.jo.1, group.id).await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].call.as_ref().unwrap().joined.len(), 2);
    let (_, ready) = hello(&app, &p.jo.1).await;
    assert!(ready.calls.is_empty() && ready.ringing.is_empty());
}

#[tokio::test]
async fn the_line_counts_unread_for_all_but_the_caller() {
    let app = spawn().await;
    let p = people(&app).await;
    let group = create_dm(&app, &p.alex.1, &[p.sam.0, p.jo.0]).await;
    start_ok(&app, &p.sam.1, group.id).await;
    let unread = |r: &Ready| {
        r.read_states
            .iter()
            .find(|x| x.channel_id == group.id)
            .map(|x| (x.unread, x.mentions))
    };
    let (_, ra) = hello(&app, &p.alex.1).await;
    let (_, rs) = hello(&app, &p.sam.1).await;
    let (_, rj) = hello(&app, &p.jo.1).await;
    assert_eq!(unread(&ra), Some((1, 0)), "unread, not a mention");
    assert_eq!(unread(&rj), Some((1, 0)));
    assert_eq!(unread(&rs), Some((0, 0)), "not for the caller");
    let line = &lines(&app, &p.alex.1, group.id).await[0];
    assert_eq!(line.call.as_ref().unwrap().started_by, Some(p.sam.0));
    assert!(line.content.contains("sam started a call"));
    // the conversation list's latest is the line
    assert_eq!(
        ra.latest
            .iter()
            .find(|m| m.channel_id == group.id)
            .map(|m| m.id),
        Some(line.id)
    );
}

#[tokio::test]
async fn a_call_nobody_connects_to_ends_after_the_grace() {
    let app = spawn_fast().await;
    let p = people(&app).await;
    let dm = create_dm(&app, &p.alex.1, &[p.sam.0]).await;
    let (mut s, _) = hello(&app, &p.sam.1).await;
    start_ok(&app, &p.alex.1, dm.id).await;
    let ev = drain_for(&mut s, Duration::from_millis(1200)).await;
    assert!(
        ev.iter().any(|e| matches!(e, Event::CallEnded { .. })),
        "{ev:?}"
    );
    assert_eq!(
        ring_ends(&ev).len(),
        1,
        "the ring stops once, however it ends"
    );
    let line = &lines(&app, &p.sam.1, dm.id).await[0];
    assert!(line.call.as_ref().unwrap().joined.is_empty());
}

#[tokio::test]
async fn a_reconnect_within_the_grace_keeps_the_call() {
    let mut cfg = test_config();
    cfg.calls.grace = Duration::from_millis(800);
    let app = spawn_with(cfg).await;
    let p = people(&app).await;
    let dm = create_dm(&app, &p.alex.1, &[p.sam.0]).await;
    let (mut a, _) = hello(&app, &p.alex.1).await;
    lk(&app, "participant_joined", dm.id, p.alex.0, "PA1").await;
    lk(&app, "participant_left", dm.id, p.alex.0, "PA1").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    lk(&app, "participant_joined", dm.id, p.alex.0, "PA2").await;
    let ev = drain_for(&mut a, Duration::from_millis(1200)).await;
    assert_eq!(
        ev.iter()
            .filter(|e| matches!(e, Event::CallStarted { .. }))
            .count(),
        1
    );
    assert!(
        !ev.iter().any(|e| matches!(e, Event::CallEnded { .. })),
        "{ev:?}"
    );
    assert_eq!(lines(&app, &p.alex.1, dm.id).await.len(), 1);
}

#[tokio::test]
async fn simultaneous_starts_make_one_call() {
    let app = spawn().await;
    let p = people(&app).await;
    let dm = create_dm(&app, &p.alex.1, &[p.sam.0]).await;
    let (mut a, _) = hello(&app, &p.alex.1).await;
    let (mut s, _) = hello(&app, &p.sam.1).await;
    let (x, y) = tokio::join!(start(&app, &p.alex.1, dm.id), start(&app, &p.sam.1, dm.id));
    let x: StartCallResponse = x.json().await.unwrap();
    let y: StartCallResponse = y.json().await.unwrap();
    assert_eq!(x.call.id, y.call.id);
    assert_eq!(u8::from(x.rang) + u8::from(y.rang), 1);
    assert_eq!(lines(&app, &p.alex.1, dm.id).await.len(), 1);
    let rung = rings(&drain(&mut a).await).len() + rings(&drain(&mut s).await).len();
    assert_eq!(rung, 1);
}

#[tokio::test]
async fn call_presence_never_leaks() {
    let app = spawn_fast().await;
    let p = people(&app).await;
    let dm = create_dm(&app, &p.alex.1, &[p.sam.0]).await;
    let (mut k, _) = hello(&app, &p.kim.1).await;
    start_ok(&app, &p.alex.1, dm.id).await;
    lk(&app, "participant_joined", dm.id, p.alex.0, "PA1").await;
    lk(&app, "participant_joined", dm.id, p.sam.0, "PS1").await;
    let (_, kr) = hello(&app, &p.kim.1).await;
    assert!(kr.calls.is_empty() && kr.ringing.is_empty());
    assert!(kr.voice.iter().all(|r| r.channel_id != dm.id));
    lk(&app, "participant_left", dm.id, p.sam.0, "PS1").await;
    lk(&app, "participant_left", dm.id, p.alex.0, "PA1").await;
    let ev = drain_for(&mut k, Duration::from_millis(1200)).await;
    assert!(ev.iter().all(|e| !is_call_event(e)), "{ev:?}");
    // the members do see it
    let (_, sr) = hello(&app, &p.sam.1).await;
    assert!(sr.calls.is_empty(), "ended by now");
}

#[tokio::test]
async fn a_webhook_join_opens_a_call_without_ringing() {
    let app = spawn().await;
    let p = people(&app).await;
    let dm = create_dm(&app, &p.alex.1, &[p.sam.0]).await;
    let (mut s, _) = hello(&app, &p.sam.1).await;
    lk(&app, "participant_joined", dm.id, p.alex.0, "PA1").await;
    let ev = drain(&mut s).await;
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::CallStarted { call } if call.started_by == Some(p.alex.0)))
    );
    assert!(rings(&ev).is_empty());
    // then the start route joins it, ringing nobody
    let r = start_ok(&app, &p.sam.1, dm.id).await;
    assert!(!r.rang);
}

#[tokio::test]
async fn starting_calls_is_rate_limited() {
    let mut cfg = test_config();
    cfg.limits.call_user = pulse_server::limits::Rate::new(2, 0.0001);
    let app = spawn_with(cfg).await;
    let p = people(&app).await;
    let dm = create_dm(&app, &p.alex.1, &[p.sam.0]).await;
    assert_eq!(start(&app, &p.alex.1, dm.id).await.status(), 200);
    assert_eq!(start(&app, &p.alex.1, dm.id).await.status(), 200);
    assert_eq!(start(&app, &p.alex.1, dm.id).await.status(), 429);
    assert_eq!(
        start(&app, &p.sam.1, dm.id).await.status(),
        200,
        "per person"
    );
}

#[tokio::test]
async fn ready_carries_open_calls_and_my_rings() {
    let app = spawn().await;
    let p = people(&app).await;
    let dm = create_dm(&app, &p.alex.1, &[p.sam.0]).await;
    let call = start_ok(&app, &p.alex.1, dm.id).await.call;
    let (_, sr) = hello(&app, &p.sam.1).await;
    assert_eq!(sr.calls, vec![call.clone()]);
    assert_eq!(sr.ringing.len(), 1);
    assert_eq!(sr.ringing[0].call_id, call.id);
    let (_, ar) = hello(&app, &p.alex.1).await;
    assert_eq!(ar.calls.len(), 1);
    assert!(ar.ringing.is_empty());
}

#[tokio::test]
async fn leaving_a_group_stops_its_ring() {
    let app = spawn().await;
    let p = people(&app).await;
    let group = create_dm(&app, &p.alex.1, &[p.sam.0, p.jo.0]).await;
    let (mut j, _) = hello(&app, &p.jo.1).await;
    start_ok(&app, &p.alex.1, group.id).await;
    drain(&mut j).await;
    let r = app
        .http
        .delete(app.url(&format!("/channels/{}/members/{}", group.id, p.jo.0)))
        .bearer_auth(&p.jo.1)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    let ev = drain(&mut j).await;
    assert_eq!(
        ring_ends(&ev),
        vec![(p.jo.0, RingEndReason::CallEnded)],
        "{ev:?}"
    );
}

// --- after a restart (#50: `voice::restore` picks open calls back up) ---

fn live_participant(user: UserId, sid: &str) -> livekit_protocol::ParticipantInfo {
    livekit_protocol::ParticipantInfo {
        identity: user.to_string(),
        sid: sid.into(),
        state: livekit_protocol::participant_info::State::Active as i32,
        joined_at: chrono::Utc::now().timestamp(),
        ..Default::default()
    }
}

/// A fresh server on a copy of `app`'s database (the restart), with fast timings, booted against a
/// fake LiveKit that lists `rooms`.
async fn restart(
    app: &TestApp,
    rooms: common::livekit::Rooms,
) -> (TestApp, pulse_server::AppState) {
    let mut cfg = test_config();
    cfg.calls.ring = Duration::from_millis(500);
    cfg.calls.grace = Duration::from_millis(500);
    let after = spawn_copy(app, cfg).await;
    let mut state = after.state();
    state.cfg = std::sync::Arc::new(pulse_server::config::Config {
        livekit_api_url: common::livekit::fake_livekit(rooms).await,
        ..(*after.cfg).clone()
    });
    pulse_server::voice::restore::on_startup(&state, &pulse_server::db::now())
        .await
        .unwrap();
    (after, state)
}

#[tokio::test]
async fn an_open_call_is_picked_up_after_a_restart() {
    let app = spawn().await;
    let p = people(&app).await;
    let dm = create_dm(&app, &p.alex.1, &[p.sam.0]).await;
    let call = start_ok(&app, &p.alex.1, dm.id).await.call;
    lk(&app, "participant_joined", dm.id, p.alex.0, "PA1").await;
    lk(&app, "participant_joined", dm.id, p.sam.0, "PS1").await;
    lk(&app, "participant_left", dm.id, p.sam.0, "PS1").await;

    // LiveKit kept the call going through the deploy: alex is still in it
    let (after, _state) = restart(
        &app,
        vec![(dm.id.to_string(), vec![live_participant(p.alex.0, "PA1")])],
    )
    .await;
    let (mut s, ready) = hello(&after, &p.sam.1).await;
    assert_eq!(ready.calls.len(), 1);
    assert_eq!(ready.calls[0].id, call.id, "the same call, not a new one");
    assert!(ready.ringing.is_empty(), "a restart drops ringing");
    // it outlives the grace while alex is in it
    tokio::time::sleep(Duration::from_millis(800)).await;
    let (_, again) = hello(&after, &p.sam.1).await;
    assert_eq!(again.calls.len(), 1);

    lk(&after, "participant_left", dm.id, p.alex.0, "PA1").await;
    let ev = drain_for(&mut s, Duration::from_millis(1200)).await;
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::CallEnded { call_id, .. } if *call_id == call.id)),
        "{ev:?}"
    );
    let stored = lines(&after, &p.sam.1, dm.id).await;
    assert_eq!(stored.len(), 1, "one line across the restart");
    let line = stored[0].call.as_ref().unwrap();
    assert!(line.ended_at.is_some());
    assert!(
        line.joined.contains(&p.sam.0),
        "sam was in it: never a missed call"
    );
    assert!(line.joined.contains(&p.alex.0));
}

#[tokio::test]
async fn an_open_call_with_nobody_left_ends_after_a_restart() {
    let app = spawn().await;
    let p = people(&app).await;
    let dm = create_dm(&app, &p.alex.1, &[p.sam.0]).await;
    let call = start_ok(&app, &p.alex.1, dm.id).await.call;
    lk(&app, "participant_joined", dm.id, p.alex.0, "PA1").await;
    let (after, _state) = restart(&app, vec![]).await;
    let (_, ready) = hello(&after, &p.sam.1).await;
    assert_eq!(ready.calls.len(), 1, "still open, for its grace");
    tokio::time::sleep(Duration::from_millis(900)).await;
    let (_, later) = hello(&after, &p.sam.1).await;
    assert!(later.calls.is_empty());
    let ended: Option<String> = sqlx::query_scalar("SELECT ended_at FROM calls WHERE id = ?")
        .bind(call.id.to_string())
        .fetch_one(&after.db)
        .await
        .unwrap();
    assert!(ended.is_some());
}

#[tokio::test]
async fn a_call_that_began_while_down_is_opened_by_the_restore() {
    let app = spawn().await;
    let p = people(&app).await;
    let dm = create_dm(&app, &p.alex.1, &[p.sam.0]).await;
    let (after, _state) = restart(
        &app,
        vec![(dm.id.to_string(), vec![live_participant(p.sam.0, "PS1")])],
    )
    .await;
    let (_, ready) = hello(&after, &p.alex.1).await;
    assert_eq!(ready.calls.len(), 1);
    assert_eq!(ready.calls[0].started_by, Some(p.sam.0));
    assert!(ready.ringing.is_empty(), "only the start route rings");
}
