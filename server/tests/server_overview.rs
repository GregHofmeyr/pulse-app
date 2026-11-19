//! Server settings › Overview (#55): only its owner sets or removes a server's icon (a picture from the avatar upload)
//! or renames it; its members alone hear `ServerUpdated` (#80), and Ready carries it; invite previews show the icon;
//! it survives the media sweep while in use.
mod common;

use std::io::Cursor;
use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use image::{ImageFormat, Rgba, RgbaImage};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::ServerId;
use pulse_protocol::rest::{InvitePreview, MediaInfo, Server, ServerInvite};
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
    let img = RgbaImage::from_pixel(100, 40, Rgba([10, 120, 240, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

fn gif(frames: usize) -> Vec<u8> {
    use image::codecs::gif::GifEncoder;
    use image::{Delay, Frame};
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        for i in 0..frames {
            let img = RgbaImage::from_pixel(40, 16, Rgba([(i * 80) as u8, 40, 90, 255]));
            let delay = Delay::from_numer_denom_ms(50, 1);
            enc.encode_frame(Frame::from_parts(img, 0, 0, delay))
                .unwrap();
        }
    }
    out
}

async fn upload(app: &TestApp, token: &str, path: &str, body: Vec<u8>) -> MediaInfo {
    let r = app
        .http
        .post(app.url(path))
        .bearer_auth(token)
        .header("content-type", "application/octet-stream")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

async fn put(
    app: &TestApp,
    token: &str,
    path: String,
    body: serde_json::Value,
) -> reqwest::Response {
    app.http
        .put(app.url(&path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

async fn set_icon(
    app: &TestApp,
    token: &str,
    server: ServerId,
    hash: Option<&str>,
) -> reqwest::Response {
    put(
        app,
        token,
        format!("/servers/{server}/icon"),
        json!({ "hash": hash }),
    )
    .await
}

async fn rename(app: &TestApp, token: &str, server: ServerId, name: &str) -> reqwest::Response {
    put(
        app,
        token,
        format!("/servers/{server}/name"),
        json!({ "name": name }),
    )
    .await
}

fn updates(events: &[Event], server: ServerId) -> Vec<Server> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ServerUpdated { server: s } if s.id == server => Some(s.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn only_the_owner_sets_and_removes_the_icon_and_only_members_hear() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let (_, jo) = register(&app, "jo").await;
    let server = create_server(&app, &alex, "Main").await.id;
    assert_eq!(join_server(&app, &sam, server).await.status(), 200);
    let (mut sam_ws, _) = hello(&app, &sam).await;
    let (mut jo_ws, _) = hello(&app, &jo).await;
    drain(&mut sam_ws, 200).await;
    drain(&mut jo_ws, 200).await;

    // a member who isn't its owner can't; to an outsider the server isn't there (#80)
    let pic = upload(&app, &sam, "/media/avatar", png()).await;
    assert_eq!(
        set_icon(&app, &sam, server, Some(&pic.hash)).await.status(),
        403
    );
    assert_eq!(set_icon(&app, &jo, server, None).await.status(), 404);
    // a banner upload isn't an icon, and an unknown hash has expired
    let banner = upload(&app, &alex, "/media/banner", png()).await;
    assert_eq!(
        set_icon(&app, &alex, server, Some(&banner.hash))
            .await
            .status(),
        400
    );
    let unknown = "ab".repeat(32);
    assert_eq!(
        set_icon(&app, &alex, server, Some(&unknown)).await.status(),
        400
    );
    assert!(updates(&drain(&mut sam_ws, 300).await, server).is_empty());

    // the owner sets it
    let r = set_icon(&app, &alex, server, Some(&pic.hash)).await;
    assert_eq!(r.status(), 200);
    let got: Server = r.json().await.unwrap();
    assert_eq!(got.icon_hash.as_deref(), Some(pic.hash.as_str()));
    assert_eq!(got.icon_still, None, "a still icon has no separate still");
    let heard = updates(&drain(&mut sam_ws, 400).await, server);
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].icon_hash.as_deref(), Some(pic.hash.as_str()));
    assert!(updates(&drain(&mut jo_ws, 400).await, server).is_empty());
    // the same again says nothing
    assert_eq!(
        set_icon(&app, &alex, server, Some(&pic.hash))
            .await
            .status(),
        200
    );
    assert!(updates(&drain(&mut sam_ws, 300).await, server).is_empty());

    // a member's new Ready carries it; an invite preview shows it
    let (_ws, ready) = hello(&app, &sam).await;
    let s = ready.servers.iter().find(|s| s.id == server).unwrap();
    assert_eq!(s.icon_hash.as_deref(), Some(pic.hash.as_str()));
    let r = post_json(
        &app,
        &alex,
        &format!("/servers/{server}/invites"),
        json!({}),
    )
    .await;
    let code = r.json::<ServerInvite>().await.unwrap().code;
    let p: InvitePreview = get_json(&app, &jo, &format!("/invites/{code}")).await;
    assert_eq!(p.icon_hash.as_deref(), Some(pic.hash.as_str()));

    // the owner removes it
    let r = set_icon(&app, &alex, server, None).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Server>().await.unwrap().icon_hash, None);
    let heard = updates(&drain(&mut sam_ws, 400).await, server);
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].icon_hash, None);
}

#[tokio::test]
async fn an_animated_icon_carries_its_still() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let server = create_server(&app, &alex, "Main").await.id;
    let pic = upload(&app, &alex, "/media/avatar", gif(3)).await;
    let still = pic.still_hash.clone().unwrap();
    let got: Server = set_icon(&app, &alex, server, Some(&pic.hash))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(got.icon_still.as_deref(), Some(still.as_str()));
}

#[tokio::test]
async fn only_the_owner_renames_it_and_only_members_hear() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let (_, jo) = register(&app, "jo").await;
    let server = create_server(&app, &alex, "Main").await.id;
    assert_eq!(join_server(&app, &sam, server).await.status(), 200);
    let (mut sam_ws, _) = hello(&app, &sam).await;
    let (mut jo_ws, _) = hello(&app, &jo).await;
    drain(&mut sam_ws, 200).await;
    drain(&mut jo_ws, 200).await;

    assert_eq!(rename(&app, &sam, server, "Mine").await.status(), 403);
    assert_eq!(rename(&app, &jo, server, "Mine").await.status(), 404);
    assert_eq!(rename(&app, &alex, server, "   ").await.status(), 400);
    assert_eq!(
        rename(&app, &alex, server, &"x".repeat(65)).await.status(),
        400
    );
    assert!(updates(&drain(&mut sam_ws, 300).await, server).is_empty());

    // trimmed, and everyone in it hears
    let r = rename(&app, &alex, server, "  Lounge ").await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Server>().await.unwrap().name, "Lounge");
    let heard = updates(&drain(&mut sam_ws, 400).await, server);
    assert_eq!(
        heard.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        vec!["Lounge"]
    );
    assert!(updates(&drain(&mut jo_ws, 400).await, server).is_empty());
    // the same name again says nothing
    assert_eq!(rename(&app, &alex, server, "Lounge").await.status(), 200);
    assert!(updates(&drain(&mut sam_ws, 300).await, server).is_empty());
    let list: Vec<Server> = get_json(&app, &sam, "/servers").await;
    assert_eq!(list.iter().find(|s| s.id == server).unwrap().name, "Lounge");

    // a deleted server takes no new name
    let r = app
        .http
        .delete(app.url(&format!("/servers/{server}")))
        .bearer_auth(&alex)
        .json(&json!({ "name": "Lounge" }))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    assert_eq!(rename(&app, &alex, server, "Back").await.status(), 404);
}

#[tokio::test]
async fn the_sweep_keeps_an_icon_in_use() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let server = create_server(&app, &alex, "Main").await.id;
    let pic = upload(&app, &alex, "/media/avatar", gif(3)).await;
    assert_eq!(
        set_icon(&app, &alex, server, Some(&pic.hash))
            .await
            .status(),
        200
    );
    sqlx::query("UPDATE media SET created_at = '2020-01-01T00:00:00.000Z'")
        .execute(&app.db)
        .await
        .unwrap();
    let rows = || async {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM media")
            .fetch_one(&app.db)
            .await
            .unwrap()
    };
    let dir = &app.cfg.media_dir;
    let grace = Duration::from_secs(24 * 3600);
    assert_eq!(
        pulse_server::media::cleanup(&app.db, dir, grace)
            .await
            .unwrap(),
        0
    );
    assert_eq!(rows().await, 2);
    assert_eq!(set_icon(&app, &alex, server, None).await.status(), 200);
    assert_eq!(
        pulse_server::media::cleanup(&app.db, dir, grace)
            .await
            .unwrap(),
        2
    );
    assert_eq!(rows().await, 0);
}
