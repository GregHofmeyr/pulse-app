//! `GET /embed/{provider}/{id}`: a tiny static page that iframes the provider's official player.
//! The app iframes this page (its CSP allows only our origin as a frame), so the player gets an
//! https parent with a real origin and Referer, which YouTube (error 153) and Twitch (`parent=`)
//! need. Unauthenticated and public; off (404) without `PULSE_PUBLIC_URL`.
//!
//! The player URL comes only from [`providers::player_url`] (templates plus a validated id), so
//! the page is no open redirect or frame of anything else; it never answers with a redirect.
//!
//! YouTube's page carries one fixed inline script, allowed by its hash only ([`RELAY`]): it tells
//! the app when the video ended or the player failed, so the card goes back to its thumbnail
//! (#292: on Linux the video plays in a window of its own, and closing that window fails the
//! player). Every other provider's page has no script.

use std::sync::LazyLock;

use axum::Router;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, RawQuery, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine;
use pulse_protocol::rest::EmbedProvider;
use reqwest::Url;
use sha2::{Digest, Sha256};

use super::providers;
use crate::AppState;
use crate::error::AppError;

/// The latest a YouTube `start` may be: a week, in seconds.
pub const MAX_START: u32 = 7 * 86400;

/// Who may frame the page: the app's own origins (Windows WebView2, Linux, macOS).
const FRAME_ANCESTORS: &str = "http://tauri.localhost https://tauri.localhost tauri:";

/// YouTube's page script: listens to the player (YouTube's iframe API messages, on the player's
/// own origin, from this page's one frame) and tells the app (`parent`) only that the video ended
/// or the player failed. Nothing else is read or passed on. [`RELAY_HASH`] allows exactly it.
pub const RELAY: &str = concat!(
    "(function(){var f=document.querySelector('iframe'),o=new URL(f.src).origin,n=0,t;",
    "function say(m){try{f.contentWindow.postMessage(JSON.stringify(m),o)}catch(_){}}",
    "function hello(){say({event:'listening',id:1,channel:'widget'});",
    "say({event:'command',func:'addEventListener',args:['onStateChange'],id:1,channel:'widget'});",
    "say({event:'command',func:'addEventListener',args:['onError'],id:1,channel:'widget'})}",
    "function tell(s){parent.postMessage({pulse:'player',state:s},'*')}",
    "f.addEventListener('load',function(){clearInterval(t);n=0;hello();",
    "t=setInterval(function(){if(++n>20)clearInterval(t);else hello()},250)});",
    "addEventListener('message',function(e){if(e.origin!==o||e.source!==f.contentWindow)return;",
    "var d;try{d=typeof e.data==='string'?JSON.parse(e.data):e.data}catch(_){return}",
    "if(!d||typeof d!=='object')return;clearInterval(t);",
    "var s=d.event==='onStateChange'?d.info:d.event==='infoDelivery'&&d.info?d.info.playerState:null;",
    "if(s===0)tell('ended');else if(d.event==='onError')tell('error')})})();"
);

/// The CSP source that allows [`RELAY`] and nothing else.
pub static RELAY_HASH: LazyLock<String> = LazyLock::new(|| {
    format!(
        "'sha256-{}'",
        base64::engine::general_purpose::STANDARD.encode(Sha256::digest(RELAY.as_bytes()))
    )
});

/// Whether the provider's page carries [`RELAY`].
fn relays(p: EmbedProvider) -> bool {
    p == EmbedProvider::Youtube
}

pub fn router() -> Router<AppState> {
    Router::new().route("/embed/{provider}/{id}", get(page))
}

async fn page(
    State(s): State<AppState>,
    path: Result<Path<(String, String)>, PathRejection>,
    RawQuery(query): RawQuery,
) -> Result<Response, AppError> {
    let public = s.cfg.embeds.public_url.as_ref().ok_or(AppError::NotFound)?;
    let Path((provider, id)) = path.map_err(|_| AppError::NotFound)?;
    let provider = super::parse_provider(&provider);
    let start = start(provider, query.as_deref()).ok_or(AppError::NotFound)?;
    let parent = public.host_str().ok_or(AppError::NotFound)?;
    let player = providers::player_url(provider, &id, start, parent).ok_or(AppError::NotFound)?;
    let origin = Url::parse(&player)
        .map_err(|_| AppError::NotFound)?
        .origin()
        .ascii_serialization();
    let script = if relays(provider) {
        format!("script-src {}; ", *RELAY_HASH)
    } else {
        String::new()
    };
    let csp = format!(
        "default-src 'none'; {script}style-src 'unsafe-inline'; frame-src {origin}; frame-ancestors {FRAME_ANCESTORS}; base-uri 'none'; form-action 'none'"
    );
    Ok((
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8".to_owned()),
            (header::CACHE_CONTROL, "public, max-age=3600".to_owned()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
            (
                header::REFERRER_POLICY,
                "strict-origin-when-cross-origin".to_owned(),
            ),
            (header::CONTENT_SECURITY_POLICY, csp),
        ],
        html(provider, &player),
    )
        .into_response())
}

/// The query's `start`: `Some(None)` without one; `Some(Some(n))` for YouTube with digits only,
/// at most [`MAX_START`]; `None` (404) for anything else, a repeated `start` included. Other
/// parameters are ignored, never passed on.
fn start(provider: EmbedProvider, query: Option<&str>) -> Option<Option<u32>> {
    let Some(query) = query else {
        return Some(None);
    };
    let mut base = Url::parse("http://query.invalid/").ok()?;
    base.set_query(Some(query));
    let mut starts = base.query_pairs().filter(|(k, _)| k == "start");
    let Some((_, v)) = starts.next() else {
        return Some(None);
    };
    if starts.next().is_some() || provider != EmbedProvider::Youtube {
        return None;
    }
    if v.is_empty() || v.len() > 6 || !v.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u32 = v.parse().ok()?;
    (n <= MAX_START).then_some(Some(n))
}

fn label(p: EmbedProvider) -> &'static str {
    match p {
        EmbedProvider::Youtube => "YouTube",
        EmbedProvider::Tiktok => "TikTok",
        EmbedProvider::Spotify => "Spotify",
        EmbedProvider::Soundcloud => "SoundCloud",
        EmbedProvider::Twitch => "Twitch",
        EmbedProvider::Instagram => "Instagram",
        EmbedProvider::Generic | EmbedProvider::Unknown => "Media",
    }
}

/// The page: black, full size, one iframe; no script but YouTube's [`RELAY`].
fn html(p: EmbedProvider, player: &str) -> String {
    let src = escape(player);
    let title = label(p);
    let script = if relays(p) {
        format!("<script>{RELAY}</script>")
    } else {
        String::new()
    };
    format!(
        concat!(
            "<!doctype html>\n",
            "<html lang=\"en\">\n",
            "<head>\n",
            "<meta charset=\"utf-8\">\n",
            "<meta name=\"referrer\" content=\"strict-origin-when-cross-origin\">\n",
            "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
            "<title>{title} player</title>\n",
            "<style>html,body{{margin:0;width:100%;height:100%;background:#000;overflow:hidden}}",
            "iframe{{display:block;width:100%;height:100%;border:0}}</style>\n",
            "</head>\n",
            "<body><iframe src=\"{src}\" ",
            "allow=\"autoplay; encrypted-media; fullscreen; picture-in-picture\" allowfullscreen ",
            "referrerpolicy=\"strict-origin-when-cross-origin\" title=\"{title} player\"></iframe>{script}</body>\n",
            "</html>\n"
        ),
        title = title,
        src = src,
        script = script,
    )
}

/// Escaped for a double-quoted HTML attribute.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_attribute_text() {
        assert_eq!(escape(r#"a&b"c'd<e>f"#), "a&amp;b&quot;c&#39;d&lt;e&gt;f");
    }
}
