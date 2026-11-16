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
//!
//! A watch party's page (`/embed/youtube/{id}?party=1`, #384) carries [`PARTY`] instead: YouTube's player without its
//! own controls, which the app drives (play, pause, seek, volume) and hears (state, time, duration, errors) through
//! this page, so everyone in the call watches the same moment.

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
/// or the player failed, with YouTube's error code (a number, or null): error 153 means YouTube
/// refused to play framed in the app, and the app pops the video out instead (#88). Nothing else
/// is read or passed on. [`RELAY_HASH`] allows exactly it.
pub const RELAY: &str = concat!(
    "(function(){var f=document.querySelector('iframe'),o=new URL(f.src).origin,n=0,t;",
    "function say(m){try{f.contentWindow.postMessage(JSON.stringify(m),o)}catch(_){}}",
    "function hello(){say({event:'listening',id:1,channel:'widget'});",
    "say({event:'command',func:'addEventListener',args:['onStateChange'],id:1,channel:'widget'});",
    "say({event:'command',func:'addEventListener',args:['onError'],id:1,channel:'widget'})}",
    "function tell(s,c){parent.postMessage({pulse:'player',state:s,code:c},'*')}",
    "f.addEventListener('load',function(){clearInterval(t);n=0;hello();",
    "t=setInterval(function(){if(++n>20)clearInterval(t);else hello()},250)});",
    "addEventListener('message',function(e){if(e.origin!==o||e.source!==f.contentWindow)return;",
    "var d;try{d=typeof e.data==='string'?JSON.parse(e.data):e.data}catch(_){return}",
    "if(!d||typeof d!=='object')return;clearInterval(t);",
    "var s=d.event==='onStateChange'?d.info:d.event==='infoDelivery'&&d.info?d.info.playerState:null;",
    "if(s===0)tell('ended',null);else if(d.event==='onError')tell('error',typeof d.info==='number'?d.info:null)})})();"
);

/// The CSP source that allows [`RELAY`] and nothing else.
pub static RELAY_HASH: LazyLock<String> = LazyLock::new(|| {
    format!(
        "'sha256-{}'",
        base64::engine::general_purpose::STANDARD.encode(Sha256::digest(RELAY.as_bytes()))
    )
});

/// A watch party page's script (#384): what [`RELAY`] does, both ways. From the app (`parent`, a
/// `{pulse:'party-cmd'}` message) it passes on only play, pause, a seek to a number of seconds, a volume (0 to 100),
/// mute and unmute, each as YouTube's own iframe API command; from the player (its own origin, this page's one frame)
/// it tells the app only the player's state, time and duration, as numbers, and YouTube's error code. Nothing else is
/// read or passed on. [`PARTY_HASH`] allows exactly it.
pub const PARTY: &str = concat!(
    "(function(){var f=document.querySelector('iframe'),o=new URL(f.src).origin,n=0,t;",
    "function say(m){try{f.contentWindow.postMessage(JSON.stringify(m),o)}catch(_){}}",
    "function cmd(c,a){say({event:'command',func:c,args:a||[],id:1,channel:'widget'})}",
    "function hello(){say({event:'listening',id:1,channel:'widget'});",
    "cmd('addEventListener',['onStateChange']);cmd('addEventListener',['onError'])}",
    "function tell(m){m.pulse='party';parent.postMessage(m,'*')}",
    "function num(x,a,b){return typeof x==='number'&&isFinite(x)&&x>=a&&x<=b}",
    "f.addEventListener('load',function(){clearInterval(t);n=0;hello();",
    "t=setInterval(function(){if(++n>20)clearInterval(t);else hello()},250)});",
    "addEventListener('message',function(e){",
    "if(parent!==window&&e.source===parent){var c=e.data;if(!c||typeof c!=='object'||c.pulse!=='party-cmd')return;",
    "if(c.func==='play')cmd('playVideo');else if(c.func==='pause')cmd('pauseVideo');",
    "else if(c.func==='seek'&&num(c.to,0,172800))cmd('seekTo',[c.to,true]);",
    "else if(c.func==='volume'&&num(c.to,0,100))cmd('setVolume',[c.to]);",
    "else if(c.func==='mute')cmd('mute');else if(c.func==='unmute')cmd('unMute');return}",
    "if(e.origin!==o||e.source!==f.contentWindow)return;",
    "var d;try{d=typeof e.data==='string'?JSON.parse(e.data):e.data}catch(_){return}",
    "if(!d||typeof d!=='object')return;clearInterval(t);",
    "if(d.event==='onError'){tell({state:'error',code:typeof d.info==='number'?d.info:null});return}",
    "var i=d.event==='onStateChange'?{playerState:d.info}:(d.event==='infoDelivery'||d.event==='initialDelivery')?d.info:null;",
    "if(!i||typeof i!=='object')return;var m={state:'info'};",
    "if(typeof i.playerState==='number')m.player=i.playerState;",
    "if(num(i.currentTime,0,1e7))m.time=i.currentTime;",
    "if(num(i.duration,0,1e7))m.duration=i.duration;",
    "tell(m)})})();"
);

/// The CSP source that allows [`PARTY`] and nothing else.
pub static PARTY_HASH: LazyLock<String> = LazyLock::new(|| {
    format!(
        "'sha256-{}'",
        base64::engine::general_purpose::STANDARD.encode(Sha256::digest(PARTY.as_bytes()))
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
    let party = party(provider, query.as_deref()).ok_or(AppError::NotFound)?;
    let parent = public.host_str().ok_or(AppError::NotFound)?;
    let player = if party {
        providers::youtube_party_url(&id, start)
    } else {
        providers::player_url(provider, &id, start, parent)
    }
    .ok_or(AppError::NotFound)?;
    let origin = Url::parse(&player)
        .map_err(|_| AppError::NotFound)?
        .origin()
        .ascii_serialization();
    let script = if party {
        format!("script-src {}; ", *PARTY_HASH)
    } else if relays(provider) {
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
        html(provider, &player, party),
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

/// The query's `party`: `Some(false)` without one, `Some(true)` for YouTube's `party=1` (once), `None` (404) for
/// anything else.
fn party(provider: EmbedProvider, query: Option<&str>) -> Option<bool> {
    let Some(query) = query else {
        return Some(false);
    };
    let mut base = Url::parse("http://query.invalid/").ok()?;
    base.set_query(Some(query));
    let found: Vec<_> = base
        .query_pairs()
        .filter(|(k, _)| k == "party")
        .map(|(_, v)| v.into_owned())
        .collect();
    match found.as_slice() {
        [] => Some(false),
        [v] if v == "1" && provider == EmbedProvider::Youtube => Some(true),
        _ => None,
    }
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

/// The page: black, full size, one iframe; no script but YouTube's [`RELAY`] (or a watch party's [`PARTY`]).
fn html(p: EmbedProvider, player: &str, party: bool) -> String {
    let src = escape(player);
    let title = label(p);
    let script = if party {
        format!("<script>{PARTY}</script>")
    } else if relays(p) {
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
    fn party_only_for_youtube_and_only_1() {
        use EmbedProvider::{Twitch, Youtube};
        assert_eq!(party(Youtube, None), Some(false));
        assert_eq!(party(Youtube, Some("start=5")), Some(false));
        assert_eq!(party(Youtube, Some("party=1&start=5")), Some(true));
        for q in ["party=0", "party=", "party=yes", "party=1&party=1"] {
            assert_eq!(party(Youtube, Some(q)), None, "{q}");
        }
        assert_eq!(party(Twitch, Some("party=1")), None);
    }

    #[test]
    fn escapes_attribute_text() {
        assert_eq!(escape(r#"a&b"c'd<e>f"#), "a&amp;b&quot;c&#39;d&lt;e&gt;f");
    }
}
