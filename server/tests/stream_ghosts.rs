//! Ghost viewers (#46, spec §5): a server member outside the voice channel watches a stream that
//! allows it under a fresh anonymous identity. Everyone (the streamer too) sees only a count, and
//! the identity never reaches the database or another client. LiveKit is the fake RoomService
//! (`common::livekit`), which keeps a participant list and logs removals.

mod common;

use std::time::Duration;

use base64::Engine;
use common::livekit::{Removed, SharedRooms, fake_livekit_full};
use common::*;
use livekit_api::access_token::{AccessToken, Claims, TokenVerifier};
use livekit_protocol as lkp;
use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use pulse_protocol::rest::{Channel, VoiceTokenResponse};
use pulse_server::auth::session::GRANT_ATTRIBUTE;
use pulse_server::limits::Rate;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc::Receiver;

/// alex streams in the Lounge, kim is in that call and listens to the gateway, sam is in the
/// server but not in the call (the ghost), jo is not in the server.
struct World {
    server: ServerId,
    lounge: Channel,
    alex: (UserId, String),
    sam: (UserId, String),
    kim: (UserId, String),
    jo: (UserId, String),
    /// kim's gateway: what someone in the call sees.
    kim_rx: Receiver<ServerFrame>,
    /// alex's gateway: what the streamer sees.
    alex_rx: Receiver<ServerFrame>,
}

struct Lk {
    rooms: SharedRooms,
    removed: Removed,
}

impl Lk {
    fn connect(&self, room: &str, identity: &str, sid: &str, grant: &str) {
        let p = lkp::ParticipantInfo {
            identity: identity.to_owned(),
            sid: sid.to_owned(),
            state: lkp::participant_info::State::Active as i32,
            attributes: [(GRANT_ATTRIBUTE.to_owned(), grant.to_owned())].into(),
            ..Default::default()
        };
        let mut rooms = self.rooms.lock().unwrap();
        match rooms.iter_mut().find(|(r, _)| r == room) {
            Some((_, ps)) => {
                ps.retain(|q| q.identity != identity);
                ps.push(p);
            }
            None => rooms.push((room.to_owned(), vec![p])),
        }
    }

    fn removed(&self) -> Vec<(String, String)> {
        self.removed.lock().unwrap().clone()
    }
}

async fn spawn_lk(cfg: pulse_server::config::Config) -> (TestApp, Lk) {
    let rooms = SharedRooms::default();
    let (url, removed, _) = fake_livekit_full(rooms.clone()).await;
    let mut cfg = cfg;
    cfg.livekit_api_url = url;
    (spawn_with(cfg).await, Lk { rooms, removed })
}

async fn world(app: &TestApp) -> World {
    let alex = register(app, "alex").await;
    let sam = register(app, "sam").await;
    let kim = register(app, "kim").await;
    let jo = register(app, "jo").await;
    let s = create_server(app, &alex.1, "Main").await;
    for who in [&sam, &kim] {
        let r = post_json(app, &who.1, &format!("/servers/{}/join", s.id), json!({})).await;
        assert!(r.status().is_success());
    }
    let chans: Vec<Channel> = get_json(app, &alex.1, &format!("/servers/{}/channels", s.id)).await;
    let lounge = chans
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    app.voice.join(lounge.id, alex.0, "PA_alex");
    app.voice.join(lounge.id, kim.0, "PA_kim");
    let kim_rx = app.hub.register(kim.0, String::new()).rx;
    let alex_rx = app.hub.register(alex.0, String::new()).rx;
    World {
        server: s.id,
        lounge,
        alex,
        sam,
        kim,
        jo,
        kim_rx,
        alex_rx,
    }
}

fn settings(allow_ghosts: bool) -> serde_json::Value {
    json!({"settings": {"height": 1080, "fps": 60, "audio": true, "allow_ghosts": allow_ghosts}})
}

async fn go_live(app: &TestApp, tok: &str, ch: ChannelId, allow_ghosts: bool) {
    let r = post_json(
        app,
        tok,
        &format!("/streams/{ch}/live"),
        settings(allow_ghosts),
    )
    .await;
    assert_eq!(r.status(), 200);
}

async fn ticket(
    app: &TestApp,
    tok: &str,
    ch: ChannelId,
    role: &str,
    ghost: bool,
) -> reqwest::Response {
    post_json(
        app,
        tok,
        &format!("/streams/{ch}/ticket"),
        json!({"role": role, "ghost": ghost}),
    )
    .await
}

fn claims(app: &TestApp, jwt: &str) -> Claims {
    TokenVerifier::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .verify(jwt)
        .unwrap()
}

/// A ghost ticket for `role`: its identity and grant.
async fn ghost_ticket(app: &TestApp, tok: &str, ch: ChannelId, role: &str) -> (String, String) {
    let r = ticket(app, tok, ch, role, true).await;
    assert_eq!(r.status(), 200, "{role}");
    let t: VoiceTokenResponse = r.json().await.unwrap();
    let c = claims(app, &t.token);
    (c.sub, c.attributes[GRANT_ATTRIBUTE].clone())
}

async fn watch(
    app: &TestApp,
    tok: &str,
    ch: ChannelId,
    streamer: UserId,
    ghost: bool,
) -> reqwest::Response {
    post_json(
        app,
        tok,
        &format!("/streams/{ch}/watch"),
        json!({"streamer": streamer, "ghost": ghost}),
    )
    .await
}

fn signed(app: &TestApp, body: &str) -> String {
    let sum = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body.as_bytes()));
    AccessToken::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .with_sha256(&sum)
        .to_jwt()
        .unwrap()
}

async fn webhook(app: &TestApp, event: &str, room: &str, identity: &str, sid: &str, grant: &str) {
    let body = json!({"event": event, "id": "EV_test", "room": {"name": room},
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

/// sam watches alex as a ghost with both connections up (in the fake LiveKit and admitted by
/// their join webhooks); the ghost's id.
async fn ghost_watching(app: &TestApp, lk: &Lk, w: &World) -> String {
    let ch = w.lounge.id;
    let room = format!("{ch}~s");
    let (v, gv) = ghost_ticket(app, &w.sam.1, ch, "v").await;
    let (a, ga) = ghost_ticket(app, &w.sam.1, ch, "a").await;
    lk.connect(&room, &v, "PA_gv", &gv);
    lk.connect(&room, &a, "PA_ga", &ga);
    webhook(app, "participant_joined", &room, &v, "PA_gv", &gv).await;
    webhook(app, "participant_joined", &room, &a, "PA_ga", &ga).await;
    assert_eq!(watch(app, &w.sam.1, ch, w.alex.0, true).await.status(), 204);
    v.trim_end_matches("#v").to_owned()
}

/// A ghost's `#v` comes up (its join webhook; nothing removes a refused one here); its id.
async fn connected(app: &TestApp, tok: &str, ch: ChannelId) -> String {
    let (v, gv) = ghost_ticket(app, tok, ch, "v").await;
    webhook(
        app,
        "participant_joined",
        &format!("{ch}~s"),
        &v,
        "PA_gv",
        &gv,
    )
    .await;
    v.trim_end_matches("#v").to_owned()
}

/// Every stream event queued on `rx` within a short wait.
async fn stream_events(rx: &mut Receiver<ServerFrame>) -> Vec<Event> {
    let mut out = Vec::new();
    while let Ok(Some(f)) = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await {
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

/// The ghost counts in the `ViewersChanged` events among `events`.
fn ghost_counts(events: &[Event]) -> Vec<u32> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ViewersChanged { ghosts, .. } => Some(*ghosts),
            _ => None,
        })
        .collect()
}

/// Polls `f` for up to two seconds (LiveKit removals run in the background).
async fn eventually(mut f: impl FnMut() -> bool) -> bool {
    for _ in 0..100 {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn ghost_tickets_are_anonymous_subscribe_only_and_stream_room_only() {
    let app = spawn().await;
    let w = world(&app).await;
    let ch = w.lounge.id;
    go_live(&app, &w.alex.1, ch, true).await;
    for role in ["v", "a"] {
        let t: VoiceTokenResponse = ticket(&app, &w.sam.1, ch, role, true)
            .await
            .json()
            .await
            .unwrap();
        let c = claims(&app, &t.token);
        assert_eq!(c.video.room, format!("{ch}~s"), "never the voice room");
        assert!(c.video.room_join);
        assert_eq!(c.video.can_publish, Some(false));
        assert!(c.video.can_publish_sources.is_empty());
        assert_eq!(c.video.can_publish_data, Some(false));
        assert_eq!(c.video.can_subscribe, Some(true));
        let (id, suffix) = c.sub.split_once('#').unwrap();
        assert_eq!(suffix, role);
        assert!(id.starts_with("g_") && id.len() == 28, "{id}");
        assert!(
            !c.sub.to_uppercase().contains(&w.sam.0.to_string()),
            "never the user id"
        );
        assert!(c.name.is_empty(), "no display name either");
        assert_eq!(c.exp - c.nbf, 600, "ten minutes");
    }
    // one ghost per watch session: both connections and a refresh share the id
    let (v, _) = ghost_ticket(&app, &w.sam.1, ch, "v").await;
    let (a, _) = ghost_ticket(&app, &w.sam.1, ch, "a").await;
    assert_eq!(v.trim_end_matches("#v"), a.trim_end_matches("#a"));
    // nothing about it reaches the database
    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM livekit_grants WHERE identity LIKE 'g\\_%' ESCAPE '\\' OR room LIKE ?",
    )
    .bind(format!("{ch}~s"))
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn who_may_watch_as_a_ghost() {
    let app = spawn().await;
    let w = world(&app).await;
    let ch = w.lounge.id;
    // nothing live: nothing to watch
    assert_eq!(ticket(&app, &w.sam.1, ch, "v", true).await.status(), 403);
    go_live(&app, &w.alex.1, ch, true).await;
    // out of the call, as yourself: no
    assert_eq!(ticket(&app, &w.sam.1, ch, "v", false).await.status(), 403);
    assert_eq!(
        watch(&app, &w.sam.1, ch, w.alex.0, false).await.status(),
        403
    );
    // in the call, as a ghost: no (you watch as yourself)
    assert_eq!(ticket(&app, &w.kim.1, ch, "v", true).await.status(), 403);
    assert_eq!(
        watch(&app, &w.kim.1, ch, w.alex.0, true).await.status(),
        403
    );
    // not in the server: no
    assert_eq!(ticket(&app, &w.jo.1, ch, "v", true).await.status(), 403);
    assert_eq!(watch(&app, &w.jo.1, ch, w.alex.0, true).await.status(), 403);
    // a server member out of the call: yes
    assert_eq!(ticket(&app, &w.sam.1, ch, "v", true).await.status(), 200);
    assert_eq!(
        watch(&app, &w.sam.1, ch, w.alex.0, true).await.status(),
        204
    );
    // a stream that isn't there
    assert_eq!(watch(&app, &w.sam.1, ch, w.kim.0, true).await.status(), 404);
    // a voice-only stream in the room closes it to ghosts
    go_live(&app, &w.kim.1, ch, false).await;
    assert_eq!(ticket(&app, &w.sam.1, ch, "v", true).await.status(), 403);
    assert_eq!(
        watch(&app, &w.sam.1, ch, w.alex.0, true).await.status(),
        403
    );
}

#[tokio::test]
async fn ghost_watch_endpoint_stays_rate_limited() {
    let mut cfg = test_config();
    cfg.limits.mutate_user = Rate::new(3, 0.0);
    let app = spawn_with(cfg).await;
    let w = world(&app).await;
    let ch = w.lounge.id;
    go_live(&app, &w.alex.1, ch, true).await; // alex's own budget
    for _ in 0..3 {
        assert_eq!(
            watch(&app, &w.sam.1, ch, w.alex.0, true).await.status(),
            204
        );
    }
    assert_eq!(
        watch(&app, &w.sam.1, ch, w.alex.0, true).await.status(),
        429
    );
}

#[tokio::test]
async fn ghost_tickets_stay_rate_limited() {
    let mut cfg = test_config();
    cfg.limits.livekit_token_user = Rate::new(2, 0.0);
    let app = spawn_with(cfg).await;
    let w = world(&app).await;
    let ch = w.lounge.id;
    go_live(&app, &w.alex.1, ch, true).await;
    for _ in 0..2 {
        assert_eq!(ticket(&app, &w.sam.1, ch, "v", true).await.status(), 200);
    }
    assert_eq!(ticket(&app, &w.sam.1, ch, "v", true).await.status(), 429);
}

#[tokio::test]
async fn everyone_sees_a_count_and_nobody_sees_who() {
    let app = spawn().await;
    let mut w = world(&app).await;
    let ch = w.lounge.id;
    go_live(&app, &w.alex.1, ch, true).await;
    let sam_ghost = connected(&app, &w.sam.1, ch).await;
    assert_eq!(
        watch(&app, &w.sam.1, ch, w.alex.0, true).await.status(),
        204
    );
    // watching twice is still one ghost
    assert_eq!(
        watch(&app, &w.sam.1, ch, w.alex.0, true).await.status(),
        204
    );
    assert_eq!(app.streams.get(ch, w.alex.0).unwrap().ghosts, 1);
    post_json(
        &app,
        &w.sam.1,
        &format!("/streams/{ch}/unwatch"),
        json!({"streamer": w.alex.0}),
    )
    .await;

    for (who, rx) in [("kim", &mut w.kim_rx), ("alex", &mut w.alex_rx)] {
        let events = stream_events(rx).await;
        assert_eq!(ghost_counts(&events), vec![1, 0], "{who}");
        for e in &events {
            if let Event::ViewersChanged { viewers, .. } = e {
                assert!(viewers.is_empty(), "{who}: a ghost is never named");
            }
            let text = serde_json::to_string(e).unwrap();
            assert!(!text.contains(&w.sam.0.to_string()), "{who}: {text}");
            assert!(!text.contains(&sam_ghost), "{who}: {text}");
            assert!(!text.contains("\"g_"), "{who}: {text}");
        }
    }
}

#[tokio::test]
async fn ready_carries_the_count_only() {
    use futures::{SinkExt, StreamExt};
    use pulse_protocol::gateway::ClientFrame;
    use tokio_tungstenite::tungstenite::Message as Ws;

    let app = spawn().await;
    let w = world(&app).await;
    let ch = w.lounge.id;
    go_live(&app, &w.alex.1, ch, true).await;
    let sam_ghost = connected(&app, &w.sam.1, ch).await;
    watch(&app, &w.sam.1, ch, w.alex.0, true).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap();
    let hello = serde_json::to_string(&ClientFrame::Hello {
        token: w.kim.1.clone(),
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
    assert_eq!(ready.streams[0].ghosts, 1);
    assert!(ready.streams[0].viewers.is_empty());
    let streams = serde_json::to_string(&ready.streams).unwrap();
    assert!(!streams.contains(&w.sam.0.to_string()));
    let all = serde_json::to_string(&ready).unwrap();
    assert!(!all.contains(sam_ghost.trim_end_matches("#v")));
}

#[tokio::test]
async fn ghost_connections_are_admitted_only_with_their_grant() {
    let (app, lk) = spawn_lk(test_config()).await;
    let w = world(&app).await;
    let ch = w.lounge.id;
    let room = format!("{ch}~s");
    go_live(&app, &w.alex.1, ch, true).await;
    let (v, gv) = ghost_ticket(&app, &w.sam.1, ch, "v").await;
    let (a, _) = ghost_ticket(&app, &w.sam.1, ch, "a").await;
    lk.connect(&room, &v, "PA_gv", &gv);
    webhook(&app, "participant_joined", &room, &v, "PA_gv", &gv).await;
    // #a with #v's grant, and a made-up ghost: both refused and removed
    lk.connect(&room, &a, "PA_ga", &gv);
    webhook(&app, "participant_joined", &room, &a, "PA_ga", &gv).await;
    let forged = "g_aaaaaaaaaaaaaaaaaaaaaaaaaa#v";
    lk.connect(&room, forged, "PA_f", &gv);
    webhook(&app, "participant_joined", &room, forged, "PA_f", &gv).await;
    assert!(
        eventually(|| {
            let r = lk.removed();
            r.contains(&(room.clone(), a.clone())) && r.contains(&(room.clone(), forged.into()))
        })
        .await,
        "{:?}",
        lk.removed()
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !lk.removed().contains(&(room.clone(), v.clone())),
        "the real one stays"
    );
    // a ghost identity is never let into the voice room
    lk.connect(&ch.to_string(), &v, "PA_gvv", &gv);
    webhook(
        &app,
        "participant_joined",
        &ch.to_string(),
        &v,
        "PA_gvv",
        &gv,
    )
    .await;
    assert!(!app.voice.members(ch).iter().any(|u| *u == w.sam.0));
}

#[tokio::test]
async fn a_ghost_counts_only_once_its_connection_is_up() {
    let app = spawn().await;
    let mut w = world(&app).await;
    let ch = w.lounge.id;
    go_live(&app, &w.alex.1, ch, true).await;
    let (v, gv) = ghost_ticket(&app, &w.sam.1, ch, "v").await;
    // watching before the connection is up (or with none at all): nobody yet
    watch(&app, &w.sam.1, ch, w.alex.0, true).await;
    assert_eq!(app.streams.get(ch, w.alex.0).unwrap().ghosts, 0);
    webhook(
        &app,
        "participant_joined",
        &format!("{ch}~s"),
        &v,
        "PA_gv",
        &gv,
    )
    .await;
    assert_eq!(app.streams.get(ch, w.alex.0).unwrap().ghosts, 1);
    assert_eq!(
        ghost_counts(&stream_events(&mut w.kim_rx).await),
        vec![0, 1]
    );
}

#[tokio::test]
async fn a_ghost_leaving_ends_its_session_and_the_next_watch_is_a_new_ghost() {
    let (app, lk) = spawn_lk(test_config()).await;
    let mut w = world(&app).await;
    let ch = w.lounge.id;
    let room = format!("{ch}~s");
    go_live(&app, &w.alex.1, ch, true).await;
    let id = ghost_watching(&app, &lk, &w).await;
    assert_eq!(app.streams.get(ch, w.alex.0).unwrap().ghosts, 1);
    // #a alone leaving doesn't end it
    webhook(
        &app,
        "participant_left",
        &room,
        &format!("{id}#a"),
        "PA_ga",
        "",
    )
    .await;
    assert_eq!(app.streams.get(ch, w.alex.0).unwrap().ghosts, 1);
    webhook(
        &app,
        "participant_left",
        &room,
        &format!("{id}#v"),
        "PA_gv",
        "",
    )
    .await;
    assert_eq!(app.streams.get(ch, w.alex.0).unwrap().ghosts, 0);
    assert_eq!(
        ghost_counts(&stream_events(&mut w.kim_rx).await),
        vec![1, 0]
    );
    let (again, _) = ghost_ticket(&app, &w.sam.1, ch, "v").await;
    assert_ne!(
        again,
        format!("{id}#v"),
        "a fresh identity per watch session"
    );
}

#[tokio::test]
async fn switching_ghosts_off_mid_stream_removes_them() {
    let (app, lk) = spawn_lk(test_config()).await;
    let mut w = world(&app).await;
    let ch = w.lounge.id;
    let room = format!("{ch}~s");
    go_live(&app, &w.alex.1, ch, true).await;
    let id = ghost_watching(&app, &lk, &w).await;
    stream_events(&mut w.kim_rx).await;
    go_live(&app, &w.alex.1, ch, false).await; // an update: the stream goes on
    assert!(
        eventually(|| {
            let r = lk.removed();
            r.contains(&(room.clone(), format!("{id}#v")))
                && r.contains(&(room.clone(), format!("{id}#a")))
        })
        .await,
        "{:?}",
        lk.removed()
    );
    let info = app.streams.get(ch, w.alex.0).unwrap();
    assert_eq!((info.ghosts, info.settings.allow_ghosts), (0, false));
    let events = stream_events(&mut w.kim_rx).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::StreamUpdated { stream } if !stream.settings.allow_ghosts))
    );
    assert_eq!(ghost_counts(&events), vec![0]);
    // and they can't come back while it's voice-only
    assert_eq!(ticket(&app, &w.sam.1, ch, "v", true).await.status(), 403);
}

#[tokio::test]
async fn another_voice_only_stream_in_the_room_removes_ghosts_too() {
    let (app, lk) = spawn_lk(test_config()).await;
    let w = world(&app).await;
    let ch = w.lounge.id;
    let room = format!("{ch}~s");
    go_live(&app, &w.alex.1, ch, true).await;
    let id = ghost_watching(&app, &lk, &w).await;
    go_live(&app, &w.kim.1, ch, false).await;
    assert!(
        eventually(|| lk.removed().contains(&(room.clone(), format!("{id}#v")))).await,
        "{:?}",
        lk.removed()
    );
    assert_eq!(app.streams.get(ch, w.alex.0).unwrap().ghosts, 0);
}

#[tokio::test]
async fn a_ghost_who_joins_voice_becomes_a_named_viewer() {
    let (app, lk) = spawn_lk(test_config()).await;
    let mut w = world(&app).await;
    let ch = w.lounge.id;
    let room = format!("{ch}~s");
    go_live(&app, &w.alex.1, ch, true).await;
    let id = ghost_watching(&app, &lk, &w).await;
    stream_events(&mut w.kim_rx).await;
    // sam's voice connection comes up (LiveKit's join webhook)
    let grant = grant_livekit(&app, &ch.to_string(), &w.sam.0.to_string()).await;
    lk.connect(&ch.to_string(), &w.sam.0.to_string(), "PA_sam", &grant);
    webhook(
        &app,
        "participant_joined",
        &ch.to_string(),
        &w.sam.0.to_string(),
        "PA_sam",
        &grant,
    )
    .await;
    assert!(app.voice.members(ch).contains(&w.sam.0));
    assert!(
        eventually(|| {
            let r = lk.removed();
            r.contains(&(room.clone(), format!("{id}#v")))
                && r.contains(&(room.clone(), format!("{id}#a")))
        })
        .await,
        "{:?}",
        lk.removed()
    );
    assert_eq!(app.streams.get(ch, w.alex.0).unwrap().ghosts, 0);
    // they reconnect as themselves and are named
    let t: VoiceTokenResponse = ticket(&app, &w.sam.1, ch, "v", false)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(claims(&app, &t.token).sub, format!("{}#v", w.sam.0));
    assert_eq!(
        watch(&app, &w.sam.1, ch, w.alex.0, false).await.status(),
        204
    );
    let info = app.streams.get(ch, w.alex.0).unwrap();
    assert_eq!((info.viewers, info.ghosts), (vec![w.sam.0], 0));
    let events = stream_events(&mut w.kim_rx).await;
    assert_eq!(ghost_counts(&events), vec![0, 0]);
}

#[tokio::test]
async fn signing_out_removes_that_devices_ghost() {
    let (app, lk) = spawn_lk(test_config()).await;
    let w = world(&app).await;
    let ch = w.lounge.id;
    let room = format!("{ch}~s");
    go_live(&app, &w.alex.1, ch, true).await;
    let id = ghost_watching(&app, &lk, &w).await;
    let r = post_json(&app, &w.sam.1, "/auth/logout", json!({})).await;
    assert!(r.status().is_success(), "{}", r.status());
    assert!(
        eventually(|| lk.removed().contains(&(room.clone(), format!("{id}#v")))).await,
        "{:?}",
        lk.removed()
    );
    assert_eq!(app.streams.get(ch, w.alex.0).unwrap().ghosts, 0);
}

#[tokio::test]
async fn leaving_the_server_removes_their_stream_connections() {
    let (app, lk) = spawn_lk(test_config()).await;
    let w = world(&app).await;
    let ch = w.lounge.id;
    let room = format!("{ch}~s");
    go_live(&app, &w.alex.1, ch, true).await;
    let id = ghost_watching(&app, &lk, &w).await;
    pulse_server::streams::ghosts::member_removed(&app.state(), w.server, w.sam.0).await;
    assert!(
        eventually(|| {
            let r = lk.removed();
            r.contains(&(room.clone(), format!("{id}#v")))
                && r.contains(&(room.clone(), format!("{id}#a")))
        })
        .await,
        "{:?}",
        lk.removed()
    );
    assert_eq!(app.streams.get(ch, w.alex.0).unwrap().ghosts, 0);
    // a named viewer's stream connections go too
    let grant = grant_livekit(&app, &room, &format!("{}#v", w.kim.0)).await;
    lk.connect(&room, &format!("{}#v", w.kim.0), "PA_kv", &grant);
    watch(&app, &w.kim.1, ch, w.alex.0, false).await;
    pulse_server::streams::ghosts::member_removed(&app.state(), w.server, w.kim.0).await;
    assert!(
        lk.removed()
            .contains(&(room.clone(), format!("{}#v", w.kim.0)))
    );
    assert!(app.streams.get(ch, w.alex.0).unwrap().viewers.is_empty());
}

#[tokio::test]
async fn previews_follow_the_ghost_rule() {
    let app = spawn().await;
    let w = world(&app).await;
    let ch = w.lounge.id;
    go_live(&app, &w.alex.1, ch, true).await;
    app.streams
        .set_preview(ch, w.alex.0, bytes::Bytes::from_static(b"webp"));
    let get = |tok: String| {
        let app = &app;
        async move {
            app.http
                .get(app.url(&format!("/streams/{ch}/{}/preview", w.alex.0)))
                .bearer_auth(tok)
                .send()
                .await
                .unwrap()
                .status()
        }
    };
    assert_eq!(get(w.sam.1.clone()).await, 200, "a would-be ghost");
    go_live(&app, &w.alex.1, ch, false).await;
    assert_eq!(get(w.sam.1.clone()).await, 404, "voice-only");
    assert_eq!(get(w.kim.1.clone()).await, 200, "in the call");
}
