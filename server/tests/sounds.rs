//! Soundboard sounds: upload validation, create/list/edit/delete, Ready, events, media cleanup.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::ServerId;
use pulse_protocol::rest::{ApiError, MediaInfo, Sound};
use pulse_server::media;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello(app: &TestApp, token: &str) -> (Socket, Ready) {
    let (mut ws, _) = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap();
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    match next_frame(&mut ws).await {
        Some(ServerFrame::Ready(r)) => (ws, r),
        other => panic!("expected Ready, got {other:?}"),
    }
}

async fn next_frame(ws: &mut Socket) -> Option<ServerFrame> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        if let Ws::Text(t) = m {
            let f: ServerFrame = serde_json::from_str(&t).unwrap();
            if matches!(
                f,
                ServerFrame::Event(Event::PresenceChanged { .. } | Event::UserCreated { .. })
            ) {
                continue;
            }
            return Some(f);
        }
    }
}

async fn next_event(ws: &mut Socket) -> Event {
    match next_frame(ws).await {
        Some(ServerFrame::Event(e)) => e,
        other => panic!("expected an event, got {other:?}"),
    }
}

/// Ogg's CRC-32 (poly 0x04c11db7, no reflection, no xor), for building valid pages.
fn crc(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &b in data {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04c1_1db7
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn page(header_type: u8, granule: i64, seq: u32, packets: &[&[u8]]) -> Vec<u8> {
    let mut lacing = Vec::new();
    for p in packets {
        let mut n = p.len();
        while n >= 255 {
            lacing.push(255);
            n -= 255;
        }
        lacing.push(n as u8);
    }
    let mut out = b"OggS".to_vec();
    out.push(0);
    out.push(header_type);
    out.extend_from_slice(&granule.to_le_bytes());
    out.extend_from_slice(&7u32.to_le_bytes());
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.push(lacing.len() as u8);
    out.extend_from_slice(&lacing);
    for p in packets {
        out.extend_from_slice(p);
    }
    let c = crc(&out);
    out[22..26].copy_from_slice(&c.to_le_bytes());
    out
}

/// A valid Ogg Opus file of `ms` milliseconds (the server never decodes, so the audio packets
/// are filler).
fn ogg_opus(ms: u32) -> Vec<u8> {
    let pre_skip = 312u16;
    let mut head = b"OpusHead".to_vec();
    head.extend_from_slice(&[1, 1]);
    head.extend_from_slice(&pre_skip.to_le_bytes());
    head.extend_from_slice(&48_000u32.to_le_bytes());
    head.extend_from_slice(&[0, 0, 0]);
    let mut tags = b"OpusTags".to_vec();
    tags.extend_from_slice(&0u32.to_le_bytes());
    tags.extend_from_slice(&0u32.to_le_bytes());
    let mut out = page(2, 0, 0, &[&head]);
    out.extend(page(0, 0, 1, &[&tags]));
    let audio = [[1u8, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12]];
    let packets: Vec<&[u8]> = audio.iter().map(|p| &p[..]).collect();
    out.extend(page(
        4,
        i64::from(ms) * 48 + i64::from(pre_skip),
        2,
        &packets,
    ));
    out
}

async fn upload_bytes(app: &TestApp, token: &str, bytes: Vec<u8>) -> reqwest::Response {
    app.http
        .post(app.url("/media/sound"))
        .bearer_auth(token)
        .body(bytes)
        .send()
        .await
        .unwrap()
}

async fn upload(app: &TestApp, token: &str, ms: u32) -> MediaInfo {
    let r = upload_bytes(app, token, ogg_opus(ms)).await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

async fn join(app: &TestApp, token: &str, server: ServerId) {
    let r = post_json(app, token, &format!("/servers/{server}/join"), json!({})).await;
    assert!(r.status().is_success());
}

async fn create(
    app: &TestApp,
    token: &str,
    server: ServerId,
    body: serde_json::Value,
) -> reqwest::Response {
    post_json(app, token, &format!("/servers/{server}/sounds"), body).await
}

async fn create_ok(app: &TestApp, token: &str, server: ServerId, hash: &str) -> Sound {
    let r = create(
        app,
        token,
        server,
        json!({ "name": "airhorn", "emoji": "📯", "media_hash": hash, "volume": 120 }),
    )
    .await;
    assert_eq!(r.status(), 201);
    r.json().await.unwrap()
}

async fn patch(app: &TestApp, token: &str, id: &str, body: serde_json::Value) -> reqwest::Response {
    app.http
        .patch(app.url(&format!("/sounds/{id}")))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

async fn delete(app: &TestApp, token: &str, id: &str) -> reqwest::Response {
    app.http
        .delete(app.url(&format!("/sounds/{id}")))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
}

async fn error_message(r: reqwest::Response) -> String {
    r.json::<ApiError>().await.unwrap().message
}

#[tokio::test]
async fn upload_accepts_a_short_ogg_opus() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let bytes = ogg_opus(1500);
    let r = upload_bytes(&app, &alex, bytes.clone()).await;
    assert_eq!(r.status(), 200);
    let info: MediaInfo = r.json().await.unwrap();
    assert_eq!(info.content_type, "audio/ogg");
    let r = app
        .http
        .get(app.url(&format!("/media/{}", info.hash)))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "audio/ogg");
    assert_eq!(r.bytes().await.unwrap().as_ref(), bytes.as_slice());
}

#[tokio::test]
async fn upload_refuses_not_ogg_too_long_and_too_big() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let r = upload_bytes(&app, &alex, b"definitely not audio".to_vec()).await;
    assert_eq!(r.status(), 400);
    let r = upload_bytes(&app, &alex, ogg_opus(7000)).await;
    assert_eq!(r.status(), 400);
    assert!(error_message(r).await.contains("6 seconds"));
    let r = upload_bytes(&app, &alex, vec![0; 300 * 1024]).await;
    assert_eq!(r.status(), 413);
}

#[tokio::test]
async fn member_creates_and_everyone_sees_it() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let srv = create_server(&app, &alex, "Main").await;
    join(&app, &sam, srv.id).await;
    let media = upload(&app, &alex, 2000).await;

    let (mut ws, _) = hello(&app, &sam).await;
    let s = create_ok(&app, &alex, srv.id, &media.hash).await;
    assert_eq!(s.name, "airhorn");
    assert_eq!(s.emoji.as_deref(), Some("📯"));
    assert_eq!(s.volume, 120);
    assert_eq!(s.duration_ms, 2000);
    assert_eq!(s.plays, 0);
    assert_eq!(s.added_by, Some(alex_id));
    assert_eq!(s.server_id, srv.id);

    match next_event(&mut ws).await {
        Event::SoundCreated { sound } => assert_eq!(sound, s),
        other => panic!("{other:?}"),
    }
    let r = app
        .http
        .get(app.url(&format!("/servers/{}/sounds", srv.id)))
        .bearer_auth(&sam)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let list: Vec<Sound> = r.json().await.unwrap();
    assert_eq!(list, vec![s.clone()]);
    let (_, ready) = hello(&app, &sam).await;
    assert_eq!(ready.sounds, vec![s]);
}

#[tokio::test]
async fn create_rejects_bad_name_emoji_volume_and_non_sound_media() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let srv = create_server(&app, &alex, "Main").await;
    let media = upload(&app, &alex, 1000).await;
    let ok = |extra: serde_json::Value| {
        let mut b = json!({ "name": "ok", "media_hash": media.hash });
        b.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        b
    };

    for name in ["", "   ", &"x".repeat(33), "bad\nname"] {
        let r = create(&app, &alex, srv.id, ok(json!({ "name": name }))).await;
        assert_eq!(r.status(), 400, "{name:?}");
        assert!(error_message(r).await.contains("1-32 characters"));
    }
    for emoji in ["abc", "a", "two words", ""] {
        let r = create(&app, &alex, srv.id, ok(json!({ "emoji": emoji }))).await;
        assert_eq!(r.status(), 400, "{emoji:?}");
    }
    for volume in [0, 9, 201, 1000] {
        let r = create(&app, &alex, srv.id, ok(json!({ "volume": volume }))).await;
        assert_eq!(r.status(), 400, "{volume}");
    }
    // Defaults: no emoji, volume 100.
    let r = create(&app, &alex, srv.id, ok(json!({}))).await;
    assert_eq!(r.status(), 201);
    let s: Sound = r.json().await.unwrap();
    assert_eq!((s.emoji, s.volume), (None, 100));

    // An emoji upload's hash, and one nobody uploaded, are not sounds.
    let png = image::RgbaImage::from_pixel(64, 32, image::Rgba([10, 120, 240, 255]));
    let mut buf = std::io::Cursor::new(Vec::new());
    png.write_to(&mut buf, image::ImageFormat::Png).unwrap();
    let r = app
        .http
        .post(app.url("/media/emoji"))
        .bearer_auth(&alex)
        .body(buf.into_inner())
        .send()
        .await
        .unwrap();
    let emoji: MediaInfo = r.json().await.unwrap();
    for hash in [emoji.hash, "ab".repeat(32), "nope".into()] {
        let r = create(&app, &alex, srv.id, ok(json!({ "media_hash": hash }))).await;
        assert_eq!(r.status(), 400, "{hash}");
        assert!(error_message(r).await.contains("upload the sound again"));
    }
}

#[tokio::test]
async fn non_member_cannot_create_edit_or_delete() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, kim) = register(&app, "kim").await;
    let srv = create_server(&app, &alex, "Main").await;
    let media = upload(&app, &alex, 1000).await;
    let mine = upload(&app, &kim, 1100).await;

    let r = create(
        &app,
        &kim,
        srv.id,
        json!({ "name": "nope", "media_hash": mine.hash }),
    )
    .await;
    assert_eq!(r.status(), 403);
    let r = app
        .http
        .get(app.url(&format!("/servers/{}/sounds", srv.id)))
        .bearer_auth(&kim)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);

    let s = create_ok(&app, &alex, srv.id, &media.hash).await;
    assert_eq!(
        patch(&app, &kim, &s.id, json!({ "name": "mine" }))
            .await
            .status(),
        403
    );
    assert_eq!(delete(&app, &kim, &s.id).await.status(), 403);
    assert_eq!(
        patch(&app, &alex, "missing", json!({ "name": "x" }))
            .await
            .status(),
        404
    );
    assert_eq!(delete(&app, &alex, "missing").await.status(), 404);
}

#[tokio::test]
async fn anyone_in_the_server_edits_and_deletes() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let srv = create_server(&app, &alex, "Main").await;
    join(&app, &sam, srv.id).await;
    let media = upload(&app, &alex, 1000).await;
    let s = create_ok(&app, &alex, srv.id, &media.hash).await;

    let (mut ws, _) = hello(&app, &alex).await;
    let r = patch(
        &app,
        &sam,
        &s.id,
        json!({ "name": "  horn  ", "volume": 50 }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let edited: Sound = r.json().await.unwrap();
    assert_eq!(edited.name, "horn");
    assert_eq!(edited.volume, 50);
    // Absent fields stay.
    assert_eq!(edited.emoji.as_deref(), Some("📯"));
    match next_event(&mut ws).await {
        Event::SoundUpdated { sound } => assert_eq!(sound, edited),
        other => panic!("{other:?}"),
    }

    // `emoji: null` clears it; a new one sets it; bad values are refused.
    let cleared: Sound = patch(&app, &sam, &s.id, json!({ "emoji": null }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(cleared.emoji, None);
    assert_eq!(cleared.name, "horn");
    next_event(&mut ws).await;
    let set: Sound = patch(&app, &sam, &s.id, json!({ "emoji": "🔥" }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(set.emoji.as_deref(), Some("🔥"));
    next_event(&mut ws).await;
    for bad in [
        json!({ "name": "" }),
        json!({ "volume": 9 }),
        json!({ "volume": 201 }),
        json!({ "emoji": "abc" }),
    ] {
        assert_eq!(patch(&app, &sam, &s.id, bad).await.status(), 400);
    }

    assert_eq!(delete(&app, &sam, &s.id).await.status(), 204);
    match next_event(&mut ws).await {
        Event::SoundDeleted {
            server_id,
            sound_id,
        } => assert_eq!((server_id, sound_id), (srv.id, s.id.clone())),
        other => panic!("{other:?}"),
    }
    assert_eq!(delete(&app, &sam, &s.id).await.status(), 404);
    let (_, ready) = hello(&app, &alex).await;
    assert!(ready.sounds.is_empty());
}

#[tokio::test]
async fn deleted_sound_media_is_cleaned_up() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let srv = create_server(&app, &alex, "Main").await;
    let media = upload(&app, &alex, 1000).await;
    let s = create_ok(&app, &alex, srv.id, &media.hash).await;
    let file = media::path_for(&app.cfg.media_dir, &media.hash).unwrap();

    let n = media::cleanup(&app.db, &app.cfg.media_dir, Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(n, 0);
    assert!(file.is_file());

    assert_eq!(delete(&app, &alex, &s.id).await.status(), 204);
    let n = media::cleanup(&app.db, &app.cfg.media_dir, Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(n, 1);
    assert!(!file.exists());
}

async fn lounge(app: &TestApp, token: &str, server: ServerId) -> pulse_protocol::ids::ChannelId {
    let chans: Vec<pulse_protocol::rest::Channel> =
        get_json(app, token, &format!("/servers/{server}/channels")).await;
    chans
        .iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap()
        .id
}

async fn play(app: &TestApp, token: &str, id: &str) -> reqwest::Response {
    post_json(app, token, &format!("/sounds/{id}/play"), json!({})).await
}

async fn favourite(app: &TestApp, token: &str, id: &str, on: bool) -> reqwest::Response {
    let url = app.url(&format!("/sounds/{id}/favourite"));
    let req = if on {
        app.http.put(url)
    } else {
        app.http.delete(url)
    };
    req.bearer_auth(token).send().await.unwrap()
}

type Person = (pulse_protocol::ids::UserId, String);

/// A server with alex's sound, plus alex, sam and kim as members, in that order.
async fn soundboard() -> (TestApp, [Person; 3], ServerId, Sound) {
    let app = spawn().await;
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let kim = register(&app, "kim").await;
    let srv = create_server(&app, &alex.1, "Main").await;
    join(&app, &sam.1, srv.id).await;
    join(&app, &kim.1, srv.id).await;
    let media = upload(&app, &alex.1, 1000).await;
    let s = create_ok(&app, &alex.1, srv.id, &media.hash).await;
    (app, [alex, sam, kim], srv.id, s)
}

async fn quiet(ws: &mut Socket) -> bool {
    tokio::time::timeout(Duration::from_millis(300), next_frame(ws))
        .await
        .is_err()
}

#[tokio::test]
async fn play_reaches_only_the_call() {
    let (app, [alex, sam, kim], srv, s) = soundboard().await;
    let room = lounge(&app, &alex.1, srv).await;
    app.state().voice.join(room, alex.0, "PA_a");
    app.state().voice.join(room, sam.0, "PA_s");
    let (mut a_ws, _) = hello(&app, &alex.1).await;
    let (mut s_ws, _) = hello(&app, &sam.1).await;
    let (mut k_ws, _) = hello(&app, &kim.1).await;

    assert_eq!(play(&app, &alex.1, &s.id).await.status(), 204);
    for ws in [&mut a_ws, &mut s_ws] {
        match next_event(ws).await {
            Event::SoundPlayed {
                channel_id,
                user_id,
                sound_id,
                media_hash,
                volume,
                ..
            } => assert_eq!(
                (channel_id, user_id, sound_id, media_hash, volume),
                (room, alex.0, s.id.clone(), s.media_hash.clone(), s.volume)
            ),
            other => panic!("{other:?}"),
        }
    }
    // The player's own list follows; sam's does not.
    assert!(matches!(
        next_event(&mut a_ws).await,
        Event::FrequentSounds { .. }
    ));
    assert!(quiet(&mut s_ws).await);
    assert!(quiet(&mut k_ws).await, "kim is not in the call");
}

/// A stale or forged voice-state entry must not widen who hears a private call: the relay
/// intersects the call's members with the channel's audience.
#[tokio::test]
async fn play_in_a_private_call_reaches_only_its_members() {
    let (app, [alex, sam, _], srv, s) = soundboard().await;
    let jo = register(&app, "jo").await;
    join(&app, &jo.1, srv).await;
    let dm = create_dm(&app, &alex.1, &[sam.0]).await;
    app.state().voice.join(dm.id, alex.0, "PA_a");
    app.state().voice.join(dm.id, sam.0, "PA_s");
    app.state().voice.join(dm.id, jo.0, "PA_j");
    let (mut a_ws, _) = hello(&app, &alex.1).await;
    let (mut s_ws, _) = hello(&app, &sam.1).await;
    let (mut j_ws, _) = hello(&app, &jo.1).await;

    assert_eq!(play(&app, &alex.1, &s.id).await.status(), 204);
    for ws in [&mut a_ws, &mut s_ws] {
        match next_event(ws).await {
            Event::SoundPlayed { channel_id, .. } => assert_eq!(channel_id, dm.id),
            other => panic!("{other:?}"),
        }
    }
    assert!(quiet(&mut j_ws).await, "jo is not in the private channel");
}

#[tokio::test]
async fn play_works_while_muted() {
    let (app, [alex, ..], srv, s) = soundboard().await;
    let room = lounge(&app, &alex.1, srv).await;
    app.state().voice.join(room, alex.0, "PA_a");
    app.state().voice.set_flags(
        alex.0,
        pulse_protocol::gateway::VoiceFlags {
            muted: true,
            deafened: false,
        },
    );
    let (mut ws, _) = hello(&app, &alex.1).await;
    assert_eq!(play(&app, &alex.1, &s.id).await.status(), 204);
    assert!(matches!(
        next_event(&mut ws).await,
        Event::SoundPlayed { .. }
    ));
}

#[tokio::test]
async fn play_outside_a_call_is_409() {
    let (app, [alex, ..], _, s) = soundboard().await;
    let r = play(&app, &alex.1, &s.id).await;
    assert_eq!(r.status(), 409);
    assert!(error_message(r).await.contains("not in a call"));
    // Nothing was counted.
    let (_, ready) = hello(&app, &alex.1).await;
    assert_eq!(ready.sounds[0].plays, 0);
    assert!(ready.frequent_sounds.is_empty());
}

#[tokio::test]
async fn play_from_another_server_you_are_in_works() {
    let (app, [alex, sam, _], _, s) = soundboard().await;
    let other = create_server(&app, &sam.1, "Other").await;
    join(&app, &alex.1, other.id).await;
    // The call is in the other server; the sound belongs to the first.
    let room = lounge(&app, &sam.1, other.id).await;
    app.state().voice.join(room, alex.0, "PA_a");
    app.state().voice.join(room, sam.0, "PA_s");
    let (mut s_ws, _) = hello(&app, &sam.1).await;
    assert_eq!(play(&app, &alex.1, &s.id).await.status(), 204);
    match next_event(&mut s_ws).await {
        Event::SoundPlayed { channel_id, .. } => assert_eq!(channel_id, room),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn non_member_play_is_403() {
    let (app, [alex, ..], srv, s) = soundboard().await;
    let (jo_id, jo) = register(&app, "jo").await;
    let room = lounge(&app, &alex.1, srv).await;
    app.state().voice.join(room, jo_id, "PA_j");
    assert_eq!(play(&app, &jo, &s.id).await.status(), 403);
    assert_eq!(play(&app, &alex.1, "missing").await.status(), 404);
    assert_eq!(favourite(&app, &jo, &s.id, true).await.status(), 403);
}

#[tokio::test]
async fn plays_are_counted() {
    let (app, [alex, ..], srv, s) = soundboard().await;
    let room = lounge(&app, &alex.1, srv).await;
    app.state().voice.join(room, alex.0, "PA_a");
    for _ in 0..3 {
        assert_eq!(play(&app, &alex.1, &s.id).await.status(), 204);
    }
    let list: Vec<Sound> = get_json(&app, &alex.1, &format!("/servers/{srv}/sounds")).await;
    assert_eq!(list[0].plays, 3);
}

#[tokio::test]
async fn frequent_and_favourites_are_per_user() {
    let (app, [alex, sam, _], srv, s) = soundboard().await;
    let room = lounge(&app, &alex.1, srv).await;
    app.state().voice.join(room, alex.0, "PA_a");
    let (mut a_ws, _) = hello(&app, &alex.1).await;
    let (mut s_ws, _) = hello(&app, &sam.1).await;

    assert_eq!(play(&app, &alex.1, &s.id).await.status(), 204);
    assert!(matches!(
        next_event(&mut a_ws).await,
        Event::SoundPlayed { .. }
    ));
    match next_event(&mut a_ws).await {
        Event::FrequentSounds { user_id, frequent } => {
            assert_eq!((user_id, frequent), (alex.0, vec![s.id.clone()]));
        }
        other => panic!("{other:?}"),
    }
    assert!(quiet(&mut s_ws).await);
    assert_eq!(
        hello(&app, &alex.1).await.1.frequent_sounds,
        vec![s.id.clone()]
    );
    assert!(hello(&app, &sam.1).await.1.frequent_sounds.is_empty());

    assert_eq!(favourite(&app, &alex.1, &s.id, true).await.status(), 204);
    match next_event(&mut a_ws).await {
        Event::SoundFavourites {
            user_id,
            favourites,
        } => {
            assert_eq!((user_id, favourites), (alex.0, vec![s.id.clone()]));
        }
        other => panic!("{other:?}"),
    }
    assert!(quiet(&mut s_ws).await);
    assert_eq!(
        hello(&app, &alex.1).await.1.sound_favourites,
        vec![s.id.clone()]
    );
    assert!(hello(&app, &sam.1).await.1.sound_favourites.is_empty());

    assert_eq!(favourite(&app, &alex.1, &s.id, false).await.status(), 204);
    match next_event(&mut a_ws).await {
        Event::SoundFavourites { favourites, .. } => assert!(favourites.is_empty()),
        other => panic!("{other:?}"),
    }
    assert!(hello(&app, &alex.1).await.1.sound_favourites.is_empty());
}

#[tokio::test]
async fn frequent_ranks_by_plays_and_skips_servers_you_left() {
    let (app, [alex, sam, _], srv, s) = soundboard().await;
    let other = create_server(&app, &sam.1, "Other").await;
    join(&app, &alex.1, other.id).await;
    let media = upload(&app, &sam.1, 900).await;
    let t = create_ok(&app, &sam.1, other.id, &media.hash).await;
    let room = lounge(&app, &alex.1, srv).await;
    app.state().voice.join(room, alex.0, "PA_a");
    for id in [&s.id, &t.id, &t.id] {
        assert_eq!(play(&app, &alex.1, id).await.status(), 204);
    }
    let ready = hello(&app, &alex.1).await.1;
    assert_eq!(ready.frequent_sounds, vec![t.id.clone(), s.id.clone()]);

    sqlx::query("DELETE FROM server_members WHERE server_id = ? AND user_id = ?")
        .bind(other.id.to_string())
        .bind(alex.0.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    let ready = hello(&app, &alex.1).await.1;
    assert_eq!(ready.frequent_sounds, vec![s.id.clone()]);
}

#[tokio::test]
async fn eleventh_play_in_a_second_is_429() {
    let mut cfg = test_config();
    cfg.limits.sound_play_user = pulse_server::limits::Rate::new(10, 10.0);
    let app = spawn_with(cfg).await;
    let (alex_id, alex) = register(&app, "alex").await;
    let srv = create_server(&app, &alex, "Main").await;
    let media = upload(&app, &alex, 1000).await;
    let s = create_ok(&app, &alex, srv.id, &media.hash).await;
    let room = lounge(&app, &alex, srv.id).await;
    app.state().voice.join(room, alex_id, "PA_a");
    for i in 0..10 {
        assert_eq!(play(&app, &alex, &s.id).await.status(), 204, "play {i}");
    }
    assert_eq!(play(&app, &alex, &s.id).await.status(), 429);
}

#[tokio::test]
async fn favourite_toggles_are_rate_limited() {
    let mut cfg = test_config();
    cfg.limits.sound_user = pulse_server::limits::Rate::new(3, 0.0001);
    let app = spawn_with(cfg).await;
    let (_, alex) = register(&app, "alex").await;
    let srv = create_server(&app, &alex, "Main").await;
    let media = upload(&app, &alex, 1000).await;
    // creating the sound spends one of the three
    let s = create_ok(&app, &alex, srv.id, &media.hash).await;
    assert_eq!(favourite(&app, &alex, &s.id, true).await.status(), 204);
    assert_eq!(favourite(&app, &alex, &s.id, false).await.status(), 204);
    assert_eq!(favourite(&app, &alex, &s.id, true).await.status(), 429);
    assert_eq!(favourite(&app, &alex, &s.id, false).await.status(), 429);
}

#[tokio::test]
async fn favouriting_a_sound_deleted_mid_request_is_404() {
    let (app, [alex, ..], _, s) = soundboard().await;
    // The sound goes between the membership check and the insert (a delete racing the favourite).
    sqlx::query(
        "CREATE TRIGGER race BEFORE INSERT ON sound_favourites
         BEGIN DELETE FROM sounds WHERE id = NEW.sound_id; END",
    )
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(favourite(&app, &alex.1, &s.id, true).await.status(), 404);
}

/// A custom emoji named `name` in `server`, added by `token`.
async fn custom_emoji(
    app: &TestApp,
    token: &str,
    server: ServerId,
    name: &str,
) -> pulse_protocol::rest::CustomEmoji {
    let png = image::RgbaImage::from_pixel(64, 32, image::Rgba([10, 120, 240, 255]));
    let mut buf = std::io::Cursor::new(Vec::new());
    png.write_to(&mut buf, image::ImageFormat::Png).unwrap();
    let r = app
        .http
        .post(app.url("/media/emoji"))
        .bearer_auth(token)
        .body(buf.into_inner())
        .send()
        .await
        .unwrap();
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

#[tokio::test]
async fn custom_emoji_on_a_sound() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let main = create_server(&app, &alex, "Main").await;
    let other = create_server(&app, &alex, "Other").await;
    let sams = create_server(&app, &sam, "Sams").await;
    let here = custom_emoji(&app, &alex, main.id, "here").await;
    let there = custom_emoji(&app, &alex, other.id, "there").await;
    let foreign = custom_emoji(&app, &sam, sams.id, "foreign").await;
    let gone = custom_emoji(&app, &alex, main.id, "gone").await;
    let r = app
        .http
        .delete(app.url(&format!("/emoji/{}", gone.id)))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    let media = upload(&app, &alex, 1000).await;
    let body = |extra: serde_json::Value| {
        let mut b = json!({ "name": "horn", "media_hash": media.hash });
        b.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        b
    };

    // this server's emoji, and another server alex is in
    for e in [&here, &there] {
        let r = create(&app, &alex, main.id, body(json!({ "emoji_id": e.id }))).await;
        assert_eq!(r.status(), 201, "{}", e.name);
        let s: Sound = r.json().await.unwrap();
        assert_eq!(
            (s.emoji, s.emoji_id.as_deref()),
            (None, Some(e.id.as_str()))
        );
    }
    // a server alex isn't in, a deleted one, one that doesn't exist, both kinds at once
    for extra in [
        json!({ "emoji_id": foreign.id }),
        json!({ "emoji_id": gone.id }),
        json!({ "emoji_id": "nope" }),
        json!({ "emoji_id": here.id, "emoji": "📯" }),
    ] {
        let r = create(&app, &alex, main.id, body(extra.clone())).await;
        assert_eq!(r.status(), 400, "{extra}");
    }

    // editing: a Unicode emoji replaces the custom one, and back
    let s: Sound = create(&app, &alex, main.id, body(json!({ "emoji_id": here.id })))
        .await
        .json()
        .await
        .unwrap();
    let r = patch(&app, &alex, &s.id, json!({ "emoji": "📯" })).await;
    assert_eq!(r.status(), 200);
    let s2: Sound = r.json().await.unwrap();
    assert_eq!((s2.emoji.as_deref(), s2.emoji_id), (Some("📯"), None));
    let r = patch(&app, &alex, &s.id, json!({ "emoji_id": there.id })).await;
    let s3: Sound = r.json().await.unwrap();
    assert_eq!(
        (s3.emoji, s3.emoji_id.as_deref()),
        (None, Some(there.id.as_str()))
    );
    let r = patch(&app, &alex, &s.id, json!({ "emoji_id": null })).await;
    let s4: Sound = r.json().await.unwrap();
    assert_eq!((s4.emoji, s4.emoji_id), (None, None));
    for extra in [
        json!({ "emoji_id": foreign.id }),
        json!({ "emoji_id": here.id, "emoji": "📯" }),
    ] {
        let r = patch(&app, &alex, &s.id, extra.clone()).await;
        assert_eq!(r.status(), 400, "{extra}");
    }
    // the list carries it
    let listed: Vec<Sound> = get_json(&app, &alex, &format!("/servers/{}/sounds", main.id)).await;
    assert!(
        listed
            .iter()
            .any(|x| x.emoji_id.as_deref() == Some(here.id.as_str()))
    );
}

/// Editing a sound's name or boost keeps its custom emoji even when the editor couldn't pick that
/// emoji themselves (it's from a server they aren't in), or it has since been deleted.
#[tokio::test]
async fn an_unchanged_custom_emoji_is_not_checked_again() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let main = create_server(&app, &alex, "Main").await;
    join(&app, &sam, main.id).await;
    let other = create_server(&app, &alex, "Other").await;
    let there = custom_emoji(&app, &alex, other.id, "there").await;
    let here = custom_emoji(&app, &alex, main.id, "here").await;
    let media = upload(&app, &alex, 1000).await;
    let mk = |id: &str| json!({ "name": "horn", "media_hash": media.hash, "emoji_id": id });
    let s: Sound = create(&app, &alex, main.id, mk(&there.id))
        .await
        .json()
        .await
        .unwrap();
    // sam isn't in Other, but sends the emoji back unchanged with the rename (as the dialog does)
    let r = patch(
        &app,
        &sam,
        &s.id,
        json!({ "name": "toot", "emoji": null, "emoji_id": there.id }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let s2: Sound = r.json().await.unwrap();
    assert_eq!(
        (s2.name.as_str(), s2.emoji_id.as_deref()),
        ("toot", Some(there.id.as_str()))
    );
    // but sam can't choose it anew
    let fresh: Sound = create(&app, &alex, main.id, mk(&here.id))
        .await
        .json()
        .await
        .unwrap();
    let r = patch(&app, &sam, &fresh.id, json!({ "emoji_id": there.id })).await;
    assert_eq!(r.status(), 400);
    // a deleted emoji: the rename still works
    let r = app
        .http
        .delete(app.url(&format!("/emoji/{}", here.id)))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    let r = patch(
        &app,
        &alex,
        &fresh.id,
        json!({ "volume": 150, "emoji": null, "emoji_id": here.id }),
    )
    .await;
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn played_carries_the_length() {
    let (app, [alex, _, _], srv, s) = soundboard().await;
    let room = lounge(&app, &alex.1, srv).await;
    app.state().voice.join(room, alex.0, "PA_a");
    let (mut ws, _) = hello(&app, &alex.1).await;
    assert_eq!(play(&app, &alex.1, &s.id).await.status(), 204);
    match next_event(&mut ws).await {
        Event::SoundPlayed { duration_ms, .. } => assert_eq!(duration_ms, s.duration_ms),
        other => panic!("{other:?}"),
    }
}
