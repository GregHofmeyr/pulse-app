//! Server banners (#475): only its owner sets or removes one (a picture from the profile banner upload); everyone
//! hears `ServerUpdated` and Ready carries it; invite previews and the invite page show it; it survives the media
//! sweep while in use.
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

async fn set_banner(
    app: &TestApp,
    token: &str,
    server: ServerId,
    hash: Option<&str>,
) -> reqwest::Response {
    app.http
        .put(app.url(&format!("/servers/{server}/banner")))
        .bearer_auth(token)
        .json(&json!({ "hash": hash }))
        .send()
        .await
        .unwrap()
}

fn banner_updates(events: &[Event], server: ServerId) -> Vec<Option<String>> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ServerUpdated { server: s } if s.id == server => Some(s.banner_hash.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn only_the_owner_sets_and_removes_it_and_everyone_hears() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let (_, jo) = register(&app, "jo").await;
    let server = create_server(&app, &alex, "Main").await.id;
    let r = post_json(&app, &sam, &format!("/servers/{server}/join"), json!({})).await;
    assert_eq!(r.status(), 204);
    let (mut sam_ws, ready) = hello(&app, &sam).await;
    let (mut jo_ws, _) = hello(&app, &jo).await;
    assert_eq!(
        ready.servers.iter().find(|s| s.id == server).unwrap().banner_hash,
        None
    );
    drain(&mut sam_ws, 200).await;
    drain(&mut jo_ws, 200).await;

    // a member who isn't its owner can't, nor can an outsider
    let pic = upload(&app, &sam, "/media/banner", png()).await;
    assert_eq!(set_banner(&app, &sam, server, Some(&pic.hash)).await.status(), 403);
    assert_eq!(set_banner(&app, &jo, server, None).await.status(), 403);
    // an avatar upload isn't a banner, and an unknown hash has expired
    let avatar = upload(&app, &alex, "/media/avatar", png()).await;
    assert_eq!(set_banner(&app, &alex, server, Some(&avatar.hash)).await.status(), 400);
    let unknown = "ab".repeat(32);
    assert_eq!(set_banner(&app, &alex, server, Some(&unknown)).await.status(), 400);
    assert!(banner_updates(&drain(&mut sam_ws, 300).await, server).is_empty());

    // the owner sets it (a banner anyone uploaded: hashes are content)
    let r = set_banner(&app, &alex, server, Some(&pic.hash)).await;
    assert_eq!(r.status(), 200);
    let got: Server = r.json().await.unwrap();
    assert_eq!(got.banner_hash.as_deref(), Some(pic.hash.as_str()));
    assert_eq!(got.banner_still, None, "a still banner has no separate still");
    // everyone hears, members and outsiders alike (servers are public)
    assert_eq!(
        banner_updates(&drain(&mut sam_ws, 400).await, server),
        vec![Some(pic.hash.clone())]
    );
    assert_eq!(
        banner_updates(&drain(&mut jo_ws, 400).await, server),
        vec![Some(pic.hash.clone())]
    );
    // the same again changes nothing and says nothing
    assert_eq!(set_banner(&app, &alex, server, Some(&pic.hash)).await.status(), 200);
    assert!(banner_updates(&drain(&mut sam_ws, 300).await, server).is_empty());

    // a new connection's Ready and the server list carry it
    let (_ws, ready) = hello(&app, &jo).await;
    let s = ready.servers.iter().find(|s| s.id == server).unwrap();
    assert_eq!(s.banner_hash.as_deref(), Some(pic.hash.as_str()));
    let list: Vec<Server> = get_json(&app, &jo, "/servers").await;
    let s = list.iter().find(|s| s.id == server).unwrap();
    assert_eq!(s.banner_hash.as_deref(), Some(pic.hash.as_str()));

    // the owner removes it
    let r = set_banner(&app, &alex, server, None).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Server>().await.unwrap().banner_hash, None);
    assert_eq!(
        banner_updates(&drain(&mut sam_ws, 400).await, server),
        vec![None]
    );
}

#[tokio::test]
async fn an_animated_banner_carries_its_still() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let server = create_server(&app, &alex, "Main").await.id;
    let pic = upload(&app, &alex, "/media/banner", gif(3)).await;
    let still = pic.still_hash.clone().unwrap();
    let got: Server = set_banner(&app, &alex, server, Some(&pic.hash))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(got.banner_still.as_deref(), Some(still.as_str()));
}

#[tokio::test]
async fn a_deleted_server_takes_no_banner() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let server = create_server(&app, &alex, "Main").await.id;
    let pic = upload(&app, &alex, "/media/banner", png()).await;
    let r = app
        .http
        .delete(app.url(&format!("/servers/{server}")))
        .bearer_auth(&alex)
        .json(&json!({ "name": "Main" }))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    assert_eq!(set_banner(&app, &alex, server, Some(&pic.hash)).await.status(), 404);
}

#[tokio::test]
async fn invite_previews_and_the_invite_page_show_it() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let server = create_server(&app, &alex, "Main").await.id;
    let r = post_json(&app, &alex, &format!("/servers/{server}/invites"), json!({})).await;
    assert_eq!(r.status(), 200);
    let code = r.json::<ServerInvite>().await.unwrap().code;

    // none yet: the card draws its gradient, the page has no banner
    let p: InvitePreview = get_json(&app, &sam, &format!("/invites/{code}")).await;
    assert_eq!(p.banner_hash, None);
    let page = app
        .http
        .get(app.url(&format!("/i/{code}")))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!page.contains("class=\"banner\""), "{page}");

    let pic = upload(&app, &alex, "/media/banner", gif(3)).await;
    let still = pic.still_hash.clone().unwrap();
    assert_eq!(set_banner(&app, &alex, server, Some(&pic.hash)).await.status(), 200);
    let p: InvitePreview = get_json(&app, &sam, &format!("/invites/{code}")).await;
    assert_eq!(p.banner_hash.as_deref(), Some(pic.hash.as_str()));
    assert_eq!(p.banner_still.as_deref(), Some(still.as_str()));
    let page = app
        .http
        .get(app.url(&format!("/i/{code}")))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        page.contains(&format!("<img src=\"/media/{}\"", pic.hash)),
        "{page}"
    );
    // it holds still for someone who asked for less motion
    assert!(
        page.contains(&format!(
            "media=\"(prefers-reduced-motion: reduce)\" srcset=\"/media/{still}\""
        )),
        "{page}"
    );
}

#[tokio::test]
async fn the_sweep_keeps_a_banner_in_use() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let server = create_server(&app, &alex, "Main").await.id;
    let pic = upload(&app, &alex, "/media/banner", gif(3)).await;
    assert_eq!(set_banner(&app, &alex, server, Some(&pic.hash)).await.status(), 200);
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
    // the banner and its still stay
    assert_eq!(pulse_server::media::cleanup(&app.db, dir, grace).await.unwrap(), 0);
    assert_eq!(rows().await, 2);

    assert_eq!(set_banner(&app, &alex, server, None).await.status(), 200);
    assert_eq!(pulse_server::media::cleanup(&app.db, dir, grace).await.unwrap(), 2);
    assert_eq!(rows().await, 0);
}
