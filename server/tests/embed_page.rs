//! `GET /embed/{provider}/{id}`: the public page that iframes a provider's official player for the
//! app. Off without `PULSE_PUBLIC_URL`; ids validated per provider; never a redirect.
mod common;

use common::*;
use pulse_server::config::{Config, EmbedConfig};
use pulse_server::embeds::page::{RELAY, RELAY_HASH};
use reqwest::{StatusCode, Url};

const ANCESTORS: &str = "frame-ancestors http://tauri.localhost https://tauri.localhost tauri:";

async fn app_at(public: Option<&str>) -> TestApp {
    spawn_with(Config {
        embeds: EmbedConfig {
            public_url: public.map(|u| Url::parse(u).unwrap()),
            ..EmbedConfig::default()
        },
        ..test_config()
    })
    .await
}

async fn app() -> TestApp {
    app_at(Some("https://pulse.example.com")).await
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

async fn get(app: &TestApp, path: &str) -> reqwest::Response {
    client().get(app.url(path)).send().await.unwrap()
}

fn header<'a>(r: &'a reqwest::Response, name: &str) -> &'a str {
    r.headers()
        .get(name)
        .unwrap_or_else(|| panic!("no {name}"))
        .to_str()
        .unwrap()
}

/// The page's one iframe `src`, still attribute-escaped.
fn iframe_src(body: &str) -> &str {
    assert_eq!(body.matches("<iframe").count(), 1, "{body}");
    let start = body.find("<iframe src=\"").unwrap() + "<iframe src=\"".len();
    let end = start + body[start..].find('"').unwrap();
    &body[start..end]
}

/// A 200 with the page's headers; returns the iframe's escaped src and the CSP.
async fn page(app: &TestApp, path: &str) -> (String, String) {
    let r = get(app, path).await;
    assert_eq!(r.status(), StatusCode::OK, "{path}");
    assert_eq!(header(&r, "content-type"), "text/html; charset=utf-8");
    assert_eq!(header(&r, "cache-control"), "public, max-age=3600");
    assert_eq!(header(&r, "x-content-type-options"), "nosniff");
    assert_eq!(
        header(&r, "referrer-policy"),
        "strict-origin-when-cross-origin"
    );
    let csp = header(&r, "content-security-policy").to_owned();
    let body = r.text().await.unwrap();
    // No script, but YouTube's one fixed relay (#292), allowed by its hash.
    let lower = body.to_lowercase();
    if path.starts_with("/embed/youtube/") {
        assert_eq!(lower.matches("<script").count(), 1, "{body}");
        assert!(
            body.contains(&format!("<script>{RELAY}</script>")),
            "{body}"
        );
    } else {
        assert!(!lower.contains("<script"), "{body}");
    }
    assert!(
        body.contains(r#"<meta name="referrer" content="strict-origin-when-cross-origin">"#),
        "{body}"
    );
    assert!(body.contains(
        r#"allow="autoplay; encrypted-media; fullscreen; picture-in-picture" allowfullscreen referrerpolicy="strict-origin-when-cross-origin""#
    ), "{body}");
    (iframe_src(&body).to_owned(), csp)
}

fn csp_for(origin: &str) -> String {
    let script = if origin == "https://www.youtube-nocookie.com" {
        format!("script-src {}; ", *RELAY_HASH)
    } else {
        String::new()
    };
    format!(
        "default-src 'none'; {script}style-src 'unsafe-inline'; frame-src {origin}; {ANCESTORS}; base-uri 'none'; form-action 'none'"
    )
}

#[test]
fn the_relay_hash_is_the_relays_sha256() {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let want = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(RELAY.as_bytes()));
    assert_eq!(*RELAY_HASH, format!("'sha256-{want}'"));
    // It only ever tells the app "ended" or "error", and talks to the player on its own origin.
    assert!(RELAY.contains("parent.postMessage({pulse:'player',state:s},'*')"));
    assert!(RELAY.contains("e.origin!==o||e.source!==f.contentWindow"));
}

#[tokio::test]
async fn the_page_is_off_without_a_public_url() {
    let app = app_at(None).await;
    for path in [
        "/embed/youtube/dQw4w9WgXcQ",
        "/embed/twitch/FunnyClip-abc_12",
    ] {
        let r = get(&app, path).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn each_provider_frames_its_official_player() {
    let app = app().await;
    let cases = [
        (
            "/embed/youtube/dQw4w9WgXcQ",
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1&amp;enablejsapi=1",
            "https://www.youtube-nocookie.com",
        ),
        (
            "/embed/tiktok/7000000000000000001",
            "https://www.tiktok.com/player/v1/7000000000000000001?autoplay=1&amp;rel=0",
            "https://www.tiktok.com",
        ),
        (
            "/embed/spotify/album%3A4uLU6hMCjMI75M1A2tKUQC",
            "https://open.spotify.com/embed/album/4uLU6hMCjMI75M1A2tKUQC",
            "https://open.spotify.com",
        ),
        (
            "/embed/spotify/track:4uLU6hMCjMI75M1A2tKUQC",
            "https://open.spotify.com/embed/track/4uLU6hMCjMI75M1A2tKUQC",
            "https://open.spotify.com",
        ),
        (
            // The client sends the id with encodeURIComponent: its slashes arrive as %2F.
            "/embed/soundcloud/alex%2Fsets%2Fsummer-mix",
            "https://w.soundcloud.com/player/?url=https%3A%2F%2Fsoundcloud.com%2Falex%2Fsets%2Fsummer-mix&amp;auto_play=true",
            "https://w.soundcloud.com",
        ),
        (
            "/embed/twitch/FunnyClip-abc_12",
            "https://clips.twitch.tv/embed?clip=FunnyClip-abc_12&amp;parent=pulse.example.com&amp;parent=tauri.localhost&amp;parent=localhost&amp;autoplay=true",
            "https://clips.twitch.tv",
        ),
        (
            "/embed/instagram/reel%3ACabc_12-x",
            "https://www.instagram.com/reel/Cabc_12-x/embed/",
            "https://www.instagram.com",
        ),
        (
            "/embed/instagram/p:Cabc_12-x",
            "https://www.instagram.com/p/Cabc_12-x/embed/",
            "https://www.instagram.com",
        ),
    ];
    for (path, src, origin) in cases {
        let (got, csp) = page(&app, path).await;
        assert_eq!(got, src, "{path}");
        assert_eq!(csp, csp_for(origin), "{path}");
    }
}

#[tokio::test]
async fn invalid_ids_and_unknown_providers_are_404() {
    let app = app().await;
    let long = "a".repeat(300);
    let paths = [
        "/embed/youtube/%2E%2E%2Fadmin",
        "/embed/youtube/..%2F..%2Fhealth",
        "/embed/youtube/dQw4w9WgX%2Fc",
        "/embed/soundcloud/alex%2F..%2Fsets",
        "/embed/youtube/dQw4w9WgX%22Q",
        "/embed/youtube/dQw4w9W%3CgXQ",
        "/embed/twitch/a%22%3E%3Cscript%3E",
        "/embed/youtube/dQw4w9WgXcQx",
        "/embed/youtube/short",
        "/embed/generic/dQw4w9WgXcQ",
        "/embed/unknown/dQw4w9WgXcQ",
        "/embed/vimeo/123456789",
        "/embed/YouTube/dQw4w9WgXcQ",
        "/embed/spotify/4uLU6hMCjMI75M1A2tKUQC",
        "/embed/spotify/user%3A4uLU6hMCjMI75M1A2tKUQC",
        "/embed/instagram/tv%3ACabc_12-x",
        "/embed/tiktok/12ab4567",
        "/embed/youtube",
        "/embed",
    ];
    for p in paths {
        let r = get(&app, p).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{p}");
        assert!(r.headers().get("content-security-policy").is_none(), "{p}");
    }
    let p = format!("/embed/twitch/{long}");
    assert_eq!(get(&app, &p).await.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn never_redirects() {
    let app = app().await;
    let paths = [
        "/embed/youtube/dQw4w9WgXcQ",
        "/embed/youtube/dQw4w9WgXcQ/",
        "/embed/youtube/dQw4w9WgXcQ?start=90",
        "/embed/youtube/dQw4w9WgXcQ?start=bad",
        "/embed/youtube//dQw4w9WgXcQ",
        "/embed//dQw4w9WgXcQ",
        "/embed/",
        "/embed/youtube/",
        "/embed/youtube/%2F%2Fevil.test",
        "/embed/youtube/https%3A%2F%2Fevil.test",
        "/embed/twitch/x?parent=evil.test",
        "/embed/generic/https%3A%2F%2Fevil.test",
        "/EMBED/youtube/dQw4w9WgXcQ",
    ];
    for p in paths {
        let r = get(&app, p).await;
        assert!(!r.status().is_redirection(), "{p}: {}", r.status());
        assert!(r.headers().get("location").is_none(), "{p}");
    }
}

#[tokio::test]
async fn start_only_for_youtube_and_only_digits() {
    let app = app().await;
    let yt = "/embed/youtube/dQw4w9WgXcQ";
    let base = "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1&amp;enablejsapi=1";

    let (src, _) = page(&app, &format!("{yt}?start=90")).await;
    assert_eq!(src, format!("{base}&amp;start=90"));
    let (src, _) = page(&app, &format!("{yt}?start=604800")).await;
    assert_eq!(src, format!("{base}&amp;start=604800"));
    // Zero is the beginning: no start in the player URL.
    let (src, _) = page(&app, &format!("{yt}?start=0")).await;
    assert_eq!(src, base);
    // Other parameters are ignored, never passed on.
    let (src, _) = page(&app, &format!("{yt}?foo=1&autoplay=0")).await;
    assert_eq!(src, base);

    for q in [
        "start=604801",
        "start=99999999999999999999",
        "start=-1",
        "start=%2B5",
        "start=1.5",
        "start=1e3",
        "start=",
        "start=%2090",
        "start=90s",
        "start=1&start=2",
    ] {
        let r = get(&app, &format!("{yt}?{q}")).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{q}");
    }
    for p in [
        "/embed/tiktok/7000000000000000001?start=5",
        "/embed/twitch/FunnyClip-abc_12?start=5",
        "/embed/spotify/album%3A4uLU6hMCjMI75M1A2tKUQC?start=5",
    ] {
        let r = get(&app, p).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{p}");
    }
}

#[tokio::test]
async fn twitch_parent_is_the_public_host() {
    // The port isn't part of Twitch's parent: the host name only.
    let app = app_at(Some("https://chat.example.com:8443")).await;
    let (src, csp) = page(&app, "/embed/twitch/Clip1").await;
    assert_eq!(
        src,
        "https://clips.twitch.tv/embed?clip=Clip1&amp;parent=chat.example.com&amp;parent=tauri.localhost&amp;parent=localhost&amp;autoplay=true"
    );
    assert_eq!(csp, csp_for("https://clips.twitch.tv"));
}
