//! A stand-in for the sites link previews read, on 127.0.0.1: every oEmbed endpoint, a few
//! pages and a thumbnail. [`FakeProvider::config`] points an [`EmbedConfig`] at it (fixed DNS
//! answers for the names below, loopback and its port allowed), so nothing leaves the machine.
//!
//! - `http://oembed.test:<port>/oembed/<provider>?url=…`: an oEmbed answer whose thumbnail is
//!   [`FakeProvider::thumb_url`]. A `url` containing `missing` gets a 404, and Instagram answers
//!   only for a `url` containing `Ccaption` (as its tokenless endpoint often refuses).
//! - `http://site.test:<port>/page`: an OpenGraph page with a relative `og:image`; `/bare` has no
//!   metadata; `/held` answers like `/page` once [`FakeProvider::release`] is called.
//! - `http://www.twitch.tv:<port>/<user>/clip/<slug>`: a clip page's OpenGraph.
//! - `http://www.tiktok.com:<port>/t/<code>/`: a short link redirecting to [`TIKTOK_URL`].
//! - `http://spotify.link:<port>/<code>`: a short link redirecting to [`SPOTIFY_JAM_URL`].
//! - `/thumb.png`: a 1280×720 PNG.

use std::collections::HashMap;
use std::io::Cursor;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, Query, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::config::{EmbedConfig, Endpoints};

pub const YOUTUBE_ID: &str = "dQw4w9WgXcQ";
pub const TIKTOK_ID: &str = "7000000000000000001";
/// Where the fake's TikTok short link leads (not requested: the id is all the unfurler needs).
pub const TIKTOK_URL: &str = "https://www.tiktok.com/@alex/video/7000000000000000001";
pub const SPOTIFY_ID: &str = "4uLU6hMCjMI75M1A2tKUQC";
pub const SPOTIFY_JAM_ID: &str = "ABC123abc123ABC123abc123";
/// Where the fake's `spotify.link` short link leads: a Jam invite (never requested).
pub const SPOTIFY_JAM_URL: &str = "https://open.spotify.com/socialsession/ABC123abc123ABC123abc123";

/// The fake's names, all answered with 127.0.0.1.
pub const HOSTS: [&str; 6] = [
    "oembed.test",
    "spotify.link",
    "site.test",
    "www.tiktok.com",
    "www.twitch.tv",
    "clips.twitch.tv",
];

const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

#[derive(Clone)]
struct Shared {
    port: u16,
    /// Path and query of every request, in order.
    log: Arc<Mutex<Vec<String>>>,
    /// `/held` answers once this is true.
    gate: tokio::sync::watch::Receiver<bool>,
    thumb: Arc<Vec<u8>>,
}

pub struct FakeProvider {
    pub port: u16,
    log: Arc<Mutex<Vec<String>>>,
    gate: tokio::sync::watch::Sender<bool>,
}

impl FakeProvider {
    pub async fn start() -> FakeProvider {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let (gate, gate_rx) = tokio::sync::watch::channel(false);
        let shared = Shared {
            port,
            log: log.clone(),
            gate: gate_rx,
            thumb: Arc::new(png(1280, 720)),
        };
        let app = Router::new()
            .route("/oembed/{provider}", get(oembed))
            .route("/page", get(|| async { og_page() }))
            .route(
                "/bare",
                get(|| async { html("<html><head></head><body>hello</body></html>".into()) }),
            )
            .route("/held", get(held))
            .route("/{user}/clip/{slug}", get(clip))
            // TikTok's short links end in a slash.
            .route(
                "/t/{code}/",
                get(|| async { (StatusCode::FOUND, [(header::LOCATION, TIKTOK_URL)]) }),
            )
            // spotify.link's short links are one segment; any other host's are not found.
            .route("/{code}", get(spotify_short))
            .route(
                "/thumb.png",
                get(|State(s): State<Shared>| async move {
                    ([(header::CONTENT_TYPE, "image/png")], s.thumb.to_vec())
                }),
            )
            .layer(middleware::from_fn_with_state(shared.clone(), record))
            .with_state(shared);
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        FakeProvider { port, log, gate }
    }

    /// An `EmbedConfig` whose endpoints are this fake's and whose fetcher may reach it.
    pub fn config(&self) -> EmbedConfig {
        let base = |p: &str| format!("http://oembed.test:{}/oembed/{p}", self.port);
        EmbedConfig {
            endpoints: Endpoints {
                youtube: base("youtube"),
                tiktok: base("tiktok"),
                spotify: base("spotify"),
                soundcloud: base("soundcloud"),
                instagram: base("instagram"),
            },
            allowed_ips: vec![LOOPBACK],
            extra_ports: vec![self.port],
            hosts: HOSTS.iter().map(|h| (h.to_string(), LOOPBACK)).collect(),
            timeout: Duration::from_secs(5),
            ..EmbedConfig::default()
        }
    }

    /// `http://<host>:<port><path>`.
    pub fn url(&self, host: &str, path: &str) -> String {
        format!("http://{host}:{}{path}", self.port)
    }

    /// The thumbnail every oEmbed answer names.
    pub fn thumb_url(&self) -> String {
        self.url("site.test", "/thumb.png")
    }

    /// Path and query of every request so far.
    pub fn requests(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    pub fn hits(&self) -> usize {
        self.log.lock().unwrap().len()
    }

    /// Let `/held` answer (it waits until then).
    pub fn release(&self) {
        self.gate.send_replace(true);
    }
}

async fn record(State(s): State<Shared>, req: Request, next: Next) -> Response {
    let pq = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_default();
    s.log.lock().unwrap().push(pq);
    next.run(req).await
}

fn html(body: String) -> Response {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response()
}

fn og_page() -> Response {
    html(
        r#"<!doctype html><html><head>
        <title>Fallback title</title>
        <meta property="og:title" content="A page &amp; more">
        <meta property="og:description" content="What the page is about.">
        <meta property="og:site_name" content="Example">
        <meta property="og:image" content="/thumb.png">
        </head><body><p>Body</p></body></html>"#
            .into(),
    )
}

async fn held(State(s): State<Shared>) -> Response {
    let mut gate = s.gate.clone();
    let _ = gate.wait_for(|open| *open).await;
    og_page()
}

async fn clip(State(s): State<Shared>, Path((user, slug)): Path<(String, String)>) -> Response {
    html(format!(
        r#"<html><head><meta property="og:title" content="{slug} by {user}">
        <meta property="og:image" content="http://site.test:{}/thumb.png"></head></html>"#,
        s.port
    ))
}

async fn oembed(
    State(s): State<Shared>,
    Path(provider): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let target = q.get("url").cloned().unwrap_or_default();
    if target.contains("missing") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let thumb = format!("http://site.test:{}/thumb.png", s.port);
    let body = match provider.as_str() {
        "youtube" => serde_json::json!({
            "type": "video", "title": "A video with a <b>tag</b>", "author_name": "alex",
            "provider_name": "YouTube", "thumbnail_url": thumb,
            "html": "<iframe src=\"https://www.youtube.com/embed/x\"></iframe>",
        }),
        "tiktok" => serde_json::json!({
            "type": "video", "title": "A dance", "author_name": "sam",
            "provider_name": "TikTok", "thumbnail_url": thumb,
            "html": "<blockquote><script src=\"https://www.tiktok.com/embed.js\"></script></blockquote>",
        }),
        "spotify" => serde_json::json!({
            "type": "rich", "title": "A song", "provider_name": "Spotify", "thumbnail_url": thumb,
        }),
        "soundcloud" => serde_json::json!({
            "type": "rich", "title": "A track by alex", "description": "Recorded at home.",
            "author_name": "alex", "provider_name": "SoundCloud", "thumbnail_url": thumb,
        }),
        "instagram" if target.contains("Ccaption") => serde_json::json!({
            "type": "rich", "title": "A caption", "author_name": "alex",
            "provider_name": "Instagram", "html": "<blockquote></blockquote>",
        }),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// A plain `w`×`h` PNG.
pub fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([30, 120, 200, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

async fn spotify_short(req: Request) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if host.split(':').next() == Some("spotify.link") {
        (StatusCode::FOUND, [(header::LOCATION, SPOTIFY_JAM_URL)]).into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}
