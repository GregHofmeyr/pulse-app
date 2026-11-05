//! Group DM icons (#56): any member sets or clears a group's icon (a picture from the avatar
//! upload); every member hears of it, outsiders don't; it survives the media sweep while in use.
mod common;

use std::io::Cursor;
use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use image::{ImageFormat, Rgba, RgbaImage};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::ChannelId;
use pulse_protocol::rest::{Channel, MediaInfo};
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

fn png() -> Vec<u8> {
    let img = RgbaImage::from_pixel(64, 32, Rgba([10, 120, 240, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

async fn upload_avatar(app: &TestApp, token: &str) -> MediaInfo {
    let r = app
        .http
        .post(app.url("/media/avatar?x=0&y=0&w=32&h=32"))
        .bearer_auth(token)
        .header("content-type", "application/octet-stream")
        .body(png())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

async fn set_icon(
    app: &TestApp,
    token: &str,
    channel: ChannelId,
    hash: Option<&str>,
) -> reqwest::Response {
    app.http
        .put(app.url(&format!("/channels/{channel}/icon")))
        .bearer_auth(token)
        .json(&serde_json::json!({ "hash": hash }))
        .send()
        .await
        .unwrap()
}

fn icon_updates(events: &[Event], channel: ChannelId) -> Vec<Option<String>> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ChannelUpdated { channel: c } if c.id == channel => Some(c.icon_hash.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn any_member_sets_and_clears_it_and_every_member_hears() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (s_id, s) = register(&app, "sam").await;
    let (k_id, k) = register(&app, "kim").await;
    let (_, j) = register(&app, "jo").await;
    let g = create_dm(&app, &a, &[s_id, k_id]).await;
    let (mut kim_ws, _) = hello(&app, &k).await;
    let (mut jo_ws, _) = hello(&app, &j).await;
    drain(&mut kim_ws, 200).await;
    drain(&mut jo_ws, 200).await;

    // sam, not the creator, sets it
    let pic = upload_avatar(&app, &s).await;
    let r = set_icon(&app, &s, g.id, Some(&pic.hash)).await;
    assert_eq!(r.status(), 200);
    let ch: Channel = r.json().await.unwrap();
    assert_eq!(ch.icon_hash.as_deref(), Some(pic.hash.as_str()));
    let seen = drain(&mut kim_ws, 400).await;
    assert_eq!(icon_updates(&seen, g.id), vec![Some(pic.hash.clone())]);
    // a system line says who changed it
    assert!(
        seen.iter()
            .any(|e| matches!(e, Event::MessageCreated { message, .. }
            if message.channel_id == g.id && message.content.contains("changed the group icon"))),
        "{seen:?}"
    );
    // outsiders hear nothing
    assert!(icon_updates(&drain(&mut jo_ws, 300).await, g.id).is_empty());

    // a new connection's Ready carries it
    let (_k2, r) = hello(&app, &k).await;
    let mine = r.channels.iter().find(|c| c.id == g.id).unwrap();
    assert_eq!(mine.icon_hash.as_deref(), Some(pic.hash.as_str()));

    // kim clears it
    let r = set_icon(&app, &k, g.id, None).await;
    assert_eq!(r.status(), 200);
    let seen = drain(&mut kim_ws, 400).await;
    assert_eq!(icon_updates(&seen, g.id), vec![None]);
    assert!(
        seen.iter()
            .any(|e| matches!(e, Event::MessageCreated { message, .. }
        if message.content.contains("removed the group icon")))
    );
    // clearing again changes nothing: no event, no line
    assert_eq!(set_icon(&app, &k, g.id, None).await.status(), 200);
    assert!(drain(&mut kim_ws, 300).await.is_empty());
}

#[tokio::test]
async fn only_groups_only_members_only_avatar_pictures() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (s_id, s) = register(&app, "sam").await;
    let (k_id, _) = register(&app, "kim").await;
    let (_, j) = register(&app, "jo").await;
    let g = create_dm(&app, &a, &[s_id, k_id]).await;
    let dm = create_dm(&app, &a, &[s_id]).await;
    let pic = upload_avatar(&app, &a).await;
    // an outsider: as if there were no group
    assert_eq!(
        set_icon(&app, &j, g.id, Some(&pic.hash)).await.status(),
        404
    );
    // a DM has no icon of its own
    assert_eq!(
        set_icon(&app, &a, dm.id, Some(&pic.hash)).await.status(),
        400
    );
    // nor does a server channel
    let server = create_server(&app, &a, "Main").await;
    let chan = general(&app, &a, server.id).await;
    assert!(
        set_icon(&app, &a, chan.id, Some(&pic.hash))
            .await
            .status()
            .is_client_error()
    );
    // only an uploaded avatar picture
    assert_eq!(
        set_icon(&app, &a, g.id, Some(&"ab".repeat(32)))
            .await
            .status(),
        400
    );
    let emoji = app
        .http
        .post(app.url("/media/emoji"))
        .bearer_auth(&s)
        .header("content-type", "application/octet-stream")
        .body(png())
        .send()
        .await
        .unwrap()
        .json::<MediaInfo>()
        .await
        .unwrap();
    assert_eq!(
        set_icon(&app, &a, g.id, Some(&emoji.hash)).await.status(),
        400
    );
    let n: Option<String> = sqlx::query_scalar("SELECT icon_hash FROM channels WHERE id = ?")
        .bind(g.id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(n, None);
}

#[tokio::test]
async fn the_media_sweep_keeps_an_icon_in_use() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (s_id, _) = register(&app, "sam").await;
    let (k_id, _) = register(&app, "kim").await;
    let g = create_dm(&app, &a, &[s_id, k_id]).await;
    let used = upload_avatar(&app, &a).await;
    assert_eq!(
        set_icon(&app, &a, g.id, Some(&used.hash)).await.status(),
        200
    );
    pulse_server::media::cleanup(&app.db, &app.cfg.media_dir, Duration::ZERO)
        .await
        .unwrap();
    let kept: Option<i64> = sqlx::query_scalar("SELECT 1 FROM media WHERE hash = ?")
        .bind(&used.hash)
        .fetch_optional(&app.db)
        .await
        .unwrap();
    assert_eq!(kept, Some(1));
    // cleared: it goes with the next sweep
    set_icon(&app, &a, g.id, None).await;
    pulse_server::media::cleanup(&app.db, &app.cfg.media_dir, Duration::ZERO)
        .await
        .unwrap();
    let gone: Option<i64> = sqlx::query_scalar("SELECT 1 FROM media WHERE hash = ?")
        .bind(&used.hash)
        .fetch_optional(&app.db)
        .await
        .unwrap();
    assert_eq!(gone, None);
}
