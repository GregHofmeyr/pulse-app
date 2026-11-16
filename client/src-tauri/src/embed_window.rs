//! Link previews: where players may load from, and the pop-out window.
//!
//! Inline players are iframes of the server's embed page (`/embed/{provider}/{id}`), which the
//! app CSP allows from production only. Instagram's embed misbehaves in an iframe, so its play
//! button opens its embed page as a top-level page in a separate window instead. YouTube and
//! Twitch can pop out the same way (#88): YouTube when its player fails with error 153 (it
//! wouldn't accept being framed by the app), Twitch on request, since its "parent" refusal can't
//! be seen from outside its frame.

use std::path::PathBuf;
use std::time::Duration;

use pulse_protocol::rest::EmbedProvider;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

/// The one origin the app CSP lets players frame (`frame-src` in every tauri*.conf.json).
pub const EMBED_ORIGIN: &str = "https://pulsechat.co.za";

/// The pop-out's window label. No capability names it (tests/capabilities.rs).
pub const LABEL: &str = "embed";

/// `EMBED_ORIGIN` when `server_url` (the signed-in server's address) is exactly that origin:
/// https, that host, the default port, no path. Any other server's embed page couldn't be framed.
pub fn embed_origin_for(server_url: &str) -> Option<&'static str> {
    let u = url::Url::parse(server_url).ok()?;
    let same = u.scheme() == "https"
        && u.host_str() == Some("pulsechat.co.za")
        // `Url` drops a port that is the scheme's default, so `:443` is `None` too
        && u.port().is_none()
        && u.username().is_empty()
        && u.password().is_none()
        && u.path() == "/"
        && u.query().is_none()
        && u.fragment().is_none();
    same.then_some(EMBED_ORIGIN)
}

fn slug_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// The page the pop-out shows for a preview, from an id that passes the server's rules for it;
/// `None` for a provider that doesn't pop out or a bad id.
/// - Instagram: its own embed page (`p:<code>` or `reel:<code>`, code 5 to 64 of `[A-Za-z0-9_-]`).
/// - YouTube: the video's watch page (id 11 of `[A-Za-z0-9_-]`), at `start` seconds (at most a
///   week, as the embed page takes).
/// - Twitch: the clip's own page (slug 1 to 100 of `[A-Za-z0-9_-]`).
pub fn popout_url(provider: EmbedProvider, id: &str, start: Option<u32>) -> Option<String> {
    let slug = |s: &str, min: usize, max: usize| {
        (min..=max).contains(&s.len()) && s.bytes().all(slug_char)
    };
    match provider {
        EmbedProvider::Instagram => {
            let (kind, code) = id.split_once(':')?;
            let ok = matches!(kind, "p" | "reel") && slug(code, 5, 64);
            ok.then(|| format!("https://www.instagram.com/{kind}/{code}/embed/"))
        }
        EmbedProvider::Youtube => {
            if !slug(id, 11, 11) {
                return None;
            }
            Some(match start.filter(|s| (1..=MAX_START).contains(s)) {
                Some(s) => format!("https://www.youtube.com/watch?v={id}&t={s}s"),
                None => format!("https://www.youtube.com/watch?v={id}"),
            })
        }
        EmbedProvider::Twitch => slug(id, 1, 100).then(|| format!("https://clips.twitch.tv/{id}")),
        _ => None,
    }
}

/// The latest a YouTube start may be: a week, in seconds (the server's `page::MAX_START`).
const MAX_START: u32 = 7 * 86400;

/// Where a provider's pop-out may go: that provider's own pages and redirects, https on the
/// default port, no credentials. Anything else, another site or a `javascript:` or `data:` URL,
/// is refused.
/// - Instagram: `instagram.com` or a subdomain, `facebook.com`'s (its login and consent pages),
///   and its media on `*.cdninstagram.com`.
/// - YouTube: `youtube.com` and `youtu.be`, and `google.com`'s (consent, sign-in, and the
///   "unusual traffic" check some networks get).
/// - Twitch: `twitch.tv` or a subdomain.
pub fn popout_may_navigate(provider: EmbedProvider, u: &url::Url) -> bool {
    let under = |host: &str, domain: &str| {
        host.strip_suffix(domain)
            .is_some_and(|rest| rest.is_empty() || rest.ends_with('.'))
    };
    let Some(url::Host::Domain(host)) = u.host() else {
        return false;
    };
    let site = match provider {
        EmbedProvider::Instagram => {
            under(host, "instagram.com")
                || under(host, "facebook.com")
                || host
                    .strip_suffix(".cdninstagram.com")
                    .is_some_and(|sub| !sub.is_empty())
        }
        EmbedProvider::Youtube => {
            under(host, "youtube.com") || under(host, "youtu.be") || under(host, "google.com")
        }
        EmbedProvider::Twitch => under(host, "twitch.tv"),
        _ => false,
    };
    u.scheme() == "https"
        && u.port().is_none()
        && u.username().is_empty()
        && u.password().is_none()
        && site
}

/// The pop-out's title and size for a provider: portrait for Instagram, wide for video.
fn window_look(provider: EmbedProvider) -> (&'static str, f64, f64) {
    match provider {
        EmbedProvider::Youtube => ("Pulse – YouTube", 1024.0, 640.0),
        EmbedProvider::Twitch => ("Pulse – Twitch", 1024.0, 640.0),
        _ => ("Pulse – Instagram", 420.0, 760.0),
    }
}

/// The main window's webview data directory. The pop-out uses the same one: WebView2 allows one
/// environment per data directory with the same options, and one is enough.
pub struct WebviewData(pub PathBuf);

/// Opens the pop-out for a preview (`start`: a YouTube start time, seconds), replacing any open one (one at a time; closing a window ends
/// its playback).
///
/// The window shows a remote page and gets no IPC: no capability lists it, and Tauri 2.12 refuses
/// every command from a non-local origin unless a `remote` capability grants it (tauri 2.12.2,
/// `src/webview/mod.rs`, the ACL check in `on_message`, "remote content can never reach custom
/// commands unless an explicit `remote` capability has been configured").
///
/// Async: building a window from a synchronous command can deadlock on Windows.
#[tauri::command]
pub async fn open_embed_window(
    app: AppHandle,
    data: State<'_, WebviewData>,
    provider: EmbedProvider,
    id: String,
    start: Option<u32>,
) -> Result<(), String> {
    let url = popout_url(provider, &id, start).ok_or("This preview doesn't open in a window.")?;
    let url: tauri::Url = url.parse().map_err(|e| format!("{e}"))?;
    if let Some(old) = app.get_webview_window(LABEL) {
        old.destroy().map_err(|e| e.to_string())?;
        // The label is freed once the event loop has handled the destroy.
        for _ in 0..40 {
            if app.get_webview_window(LABEL).is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    let (title, width, height) = window_look(provider);
    WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::External(url))
        .title(title)
        .inner_size(width, height)
        .data_directory(data.0.clone())
        // The provider's own pages, redirects and media only; nothing else loads.
        .on_navigation(move |u| popout_may_navigate(provider, u))
        // window.open / target=_blank ("View on Instagram"): never another window; a web link
        // opens in the browser.
        .on_new_window(|u, _| crate::links::new_window(u))
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use EmbedProvider::*;

    #[test]
    fn embed_origin_only_for_production() {
        assert_eq!(
            embed_origin_for("https://pulsechat.co.za"),
            Some(EMBED_ORIGIN)
        );
        assert_eq!(
            embed_origin_for("https://pulsechat.co.za/"),
            Some(EMBED_ORIGIN)
        );
        assert_eq!(
            embed_origin_for("https://PulseChat.co.za"),
            Some(EMBED_ORIGIN)
        );
        assert_eq!(
            embed_origin_for("https://pulsechat.co.za:443"),
            Some(EMBED_ORIGIN)
        );
        for other in [
            "http://pulsechat.co.za",
            "https://pulsechat.co.za:8443",
            "https://voice.pulsechat.co.za",
            "https://pulsechat.co.za.example.com",
            "https://example.com",
            "http://127.0.0.1:7890",
            "https://pulsechat.co.za/api",
            "not a url",
            "",
        ] {
            assert_eq!(embed_origin_for(other), None, "{other}");
        }
    }

    #[test]
    fn instagram_pops_out_to_its_embed_page() {
        assert_eq!(
            popout_url(Instagram, "p:Cabc_12-x", None).as_deref(),
            Some("https://www.instagram.com/p/Cabc_12-x/embed/")
        );
        assert_eq!(
            popout_url(Instagram, "reel:Cabc_12-x", None).as_deref(),
            Some("https://www.instagram.com/reel/Cabc_12-x/embed/")
        );
    }

    #[test]
    fn only_a_valid_instagram_id_pops_out() {
        let long = format!("p:{}", "a".repeat(65));
        for bad in [
            "",
            "Cabc_12-x",
            "tv:Cabc_12-x",
            "reels:Cabc_12-x",
            "p:abcd",
            long.as_str(),
            "p:Cabc/../x",
            "p:Cabc_12-x?a=1",
            "p:Cabc 12x",
            "p:Cabc%2Fx",
            "p:",
            ":Cabc_12-x",
        ] {
            assert_eq!(popout_url(Instagram, bad, None), None, "{bad}");
        }
        assert!(popout_url(Instagram, &format!("p:{}", "a".repeat(64)), None).is_some());
        assert!(popout_url(Instagram, "p:abcde", None).is_some());
    }

    #[test]
    fn the_popout_stays_on_instagram_and_facebook() {
        for ok in [
            "https://www.instagram.com/p/Cabc_12-x/embed/",
            "https://instagram.com/accounts/login/",
            "https://help.instagram.com/x",
            "https://scontent-jnb1-1.cdninstagram.com/v/t51/a.jpg",
            "https://www.facebook.com/plugins/x",
            "https://facebook.com/",
            "https://m.facebook.com/x",
        ] {
            assert!(
                popout_may_navigate(Instagram, &url::Url::parse(ok).unwrap()),
                "{ok}"
            );
        }
        for bad in [
            "http://www.instagram.com/p/Cabc_12-x/embed/",
            "https://example.com/",
            "https://instagram.com.example.com/",
            "https://evilinstagram.com/",
            "https://notfacebook.com/",
            "https://cdninstagram.com.example.test/a.jpg",
            "https://www.instagram.com:8443/",
            "https://alex@www.instagram.com/",
            "https://192.0.2.1/",
            "javascript:alert(1)",
            "data:text/html,hi",
            "file:///etc/passwd",
            "about:blank",
        ] {
            assert!(
                !popout_may_navigate(Instagram, &url::Url::parse(bad).unwrap()),
                "{bad}"
            );
        }
    }

    #[test]
    fn youtube_pops_out_to_its_watch_page() {
        assert_eq!(
            popout_url(Youtube, "dQw4w9WgXcQ", None).as_deref(),
            Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
        );
        assert_eq!(
            popout_url(Youtube, "dQw4w9WgXcQ", Some(90)).as_deref(),
            Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=90s")
        );
        // a start the embed page wouldn't take is dropped, not passed on
        for start in [Some(0), Some(MAX_START + 1)] {
            assert_eq!(
                popout_url(Youtube, "dQw4w9WgXcQ", start).as_deref(),
                Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
            );
        }
        for bad in [
            "",
            "dQw4w9WgXc",
            "dQw4w9WgXcQQ",
            "dQw4w9WgX/Q",
            "dQw4w9W&t=Q",
        ] {
            assert_eq!(popout_url(Youtube, bad, None), None, "{bad}");
        }
    }

    #[test]
    fn twitch_pops_out_to_its_clip_page() {
        assert_eq!(
            popout_url(Twitch, "FunnyClip-aB_12", None).as_deref(),
            Some("https://clips.twitch.tv/FunnyClip-aB_12")
        );
        let long = "a".repeat(101);
        for bad in ["", long.as_str(), "a/b", "a?b=1", "a b", "a%2Fb"] {
            assert_eq!(popout_url(Twitch, bad, None), None, "{bad}");
        }
    }

    #[test]
    fn each_popout_stays_on_its_own_provider() {
        let may = |p, u: &str| popout_may_navigate(p, &url::Url::parse(u).unwrap());
        for ok in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://consent.youtube.com/m",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://accounts.google.com/x",
            "https://www.google.com/sorry/index",
        ] {
            assert!(may(Youtube, ok), "{ok}");
        }
        for ok in [
            "https://clips.twitch.tv/FunnyClip",
            "https://www.twitch.tv/alex/clip/FunnyClip",
            "https://id.twitch.tv/login",
        ] {
            assert!(may(Twitch, ok), "{ok}");
        }
        for bad in [
            "http://www.youtube.com/",
            "https://youtube.com.example.com/",
            "https://notyoutube.com/",
            "https://www.youtube.com:8443/",
            "https://www.instagram.com/",
            "https://www.twitch.tv/",
            "javascript:alert(1)",
        ] {
            assert!(!may(Youtube, bad), "{bad}");
        }
        for bad in [
            "https://www.youtube.com/",
            "https://twitch.tv.example.com/",
            "https://eviltwitch.tv/",
            "http://clips.twitch.tv/x",
        ] {
            assert!(!may(Twitch, bad), "{bad}");
        }
        // Instagram's window can't wander onto YouTube either
        assert!(!may(Instagram, "https://www.youtube.com/"));
        for p in [Tiktok, Spotify, Soundcloud, Generic, Unknown] {
            assert!(!may(p, "https://www.youtube.com/"), "{p:?}");
        }
    }

    #[test]
    fn other_providers_play_inline() {
        for p in [Tiktok, Spotify, Soundcloud, Generic, Unknown] {
            assert_eq!(popout_url(p, "p:Cabc_12-x", None), None, "{p:?}");
            assert_eq!(popout_url(p, "dQw4w9WgXcQ", None), None, "{p:?}");
        }
    }
}
