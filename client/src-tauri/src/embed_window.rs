//! Link previews: where players may load from, and the Instagram pop-out window.
//!
//! Inline players are iframes of the server's embed page (`/embed/{provider}/{id}`), which the
//! app CSP allows from production only. Instagram's embed misbehaves in an iframe, so its play
//! button opens its embed page as a top-level page in a separate window instead.

use std::path::PathBuf;
use std::time::Duration;

use pulse_protocol::rest::EmbedProvider;
use tauri::webview::NewWindowResponse;
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

/// The page the pop-out shows for a preview: Instagram's own embed page, from an id that passes
/// the server's rules (`p:<code>` or `reel:<code>`, code 5 to 64 of `[A-Za-z0-9_-]`). `None` for
/// any other provider (they play inline) or a bad id.
pub fn popout_url(provider: EmbedProvider, id: &str) -> Option<String> {
    if provider != EmbedProvider::Instagram {
        return None;
    }
    let (kind, code) = id.split_once(':')?;
    let ok = matches!(kind, "p" | "reel")
        && (5..=64).contains(&code.len())
        && code.bytes().all(slug_char);
    ok.then(|| format!("https://www.instagram.com/{kind}/{code}/embed/"))
}

/// The main window's webview data directory. The pop-out uses the same one: WebView2 allows one
/// environment per data directory with the same options, and one is enough.
pub struct WebviewData(pub PathBuf);

/// Opens the pop-out for a preview, replacing any open one (one at a time; closing a window ends
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
) -> Result<(), String> {
    let url = popout_url(provider, &id).ok_or("This preview doesn't open in a window.")?;
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
    WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::External(url))
        .title("Pulse – Instagram")
        .inner_size(420.0, 760.0)
        .data_directory(data.0.clone())
        // Instagram's own links and redirects stay on https; nothing else loads.
        .on_navigation(|u| u.scheme() == "https")
        // window.open / target=_blank: refused (no further windows from remote content).
        .on_new_window(|_, _| NewWindowResponse::Deny)
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
            popout_url(Instagram, "p:Cabc_12-x").as_deref(),
            Some("https://www.instagram.com/p/Cabc_12-x/embed/")
        );
        assert_eq!(
            popout_url(Instagram, "reel:Cabc_12-x").as_deref(),
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
            assert_eq!(popout_url(Instagram, bad), None, "{bad}");
        }
        assert!(popout_url(Instagram, &format!("p:{}", "a".repeat(64))).is_some());
        assert!(popout_url(Instagram, "p:abcde").is_some());
    }

    #[test]
    fn other_providers_play_inline() {
        for p in [
            Youtube, Tiktok, Spotify, Soundcloud, Twitch, Generic, Unknown,
        ] {
            assert_eq!(popout_url(p, "p:Cabc_12-x"), None, "{p:?}");
        }
    }
}
