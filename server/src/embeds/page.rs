//! `GET /embed/{provider}/{id}`: a tiny static page that iframes the provider's official player.
//! The app iframes this page (its CSP allows only our origin as a frame), so the player gets an
//! https parent with a real origin and Referer, which YouTube (error 153) and Twitch (`parent=`)
//! need. Unauthenticated and public; off (404) without `PULSE_PUBLIC_URL`.
//!
//! The player URL comes only from [`providers::player_url`] (templates plus a validated id), so
//! the page is no open redirect or frame of anything else; it never answers with a redirect.

use axum::Router;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, RawQuery, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use pulse_protocol::rest::EmbedProvider;
use reqwest::Url;

use super::providers;
use crate::AppState;
use crate::error::AppError;

/// The latest a YouTube `start` may be: a week, in seconds.
pub const MAX_START: u32 = 7 * 86400;

/// Who may frame the page: the app's own origins (Windows WebView2, Linux, macOS).
const FRAME_ANCESTORS: &str = "http://tauri.localhost https://tauri.localhost tauri:";

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
    let csp = format!(
        "default-src 'none'; style-src 'unsafe-inline'; frame-src {origin}; frame-ancestors {FRAME_ANCESTORS}; base-uri 'none'; form-action 'none'"
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

/// The page: black, full size, one iframe, no script.
fn html(p: EmbedProvider, player: &str) -> String {
    let src = escape(player);
    let title = label(p);
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
            "referrerpolicy=\"strict-origin-when-cross-origin\" title=\"{title} player\"></iframe></body>\n",
            "</html>\n"
        ),
        title = title,
        src = src,
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
