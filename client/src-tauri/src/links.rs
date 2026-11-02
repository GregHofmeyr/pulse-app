//! Web links open in the default browser, never in a Pulse window.
//!
//! Every link the page shows (message links, link preview titles, "open in your browser") is a
//! `target="_blank"` link. The webview turns those into new-window requests, which by itself it
//! either drops (WebKitGTK) or opens as a bare browser window (WebView2). `on_new_window` sends them
//! here instead; the main window's own navigation to a website is caught the same way.

use tauri::webview::NewWindowResponse;

/// A link worth handing to the browser: http or https with a host, and no user name or password
/// in it (`https://pulsechat.co.za@evil.example/` reads as one site and goes to another).
pub fn for_browser(u: &url::Url) -> bool {
    matches!(u.scheme(), "http" | "https")
        && u.host_str().is_some_and(|h| !h.is_empty())
        && u.username().is_empty()
        && u.password().is_none()
}

/// Whether the main window may go to `u` itself: the app's own page (`tauri://localhost`,
/// `http(s)://tauri.localhost`, and the dev server on localhost) and anything that isn't a web
/// page. A website goes to the browser instead ([`main_navigation`]).
pub fn main_may_navigate(u: &url::Url) -> bool {
    if !matches!(u.scheme(), "http" | "https") {
        return true;
    }
    matches!(
        u.host_str(),
        Some("tauri.localhost" | "localhost" | "127.0.0.1" | "[::1]")
    )
}

/// The main window's `on_navigation`: its own pages load, a website opens in the browser.
pub fn main_navigation(u: &url::Url) -> bool {
    if main_may_navigate(u) {
        return true;
    }
    open(u);
    false
}

/// A window's `on_new_window`: no new Pulse window ever; a web link opens in the browser.
pub fn new_window<R: tauri::Runtime>(u: url::Url) -> NewWindowResponse<R> {
    open(&u);
    NewWindowResponse::Deny
}

/// Opens `u` in the default browser if it's [`for_browser`]; anything else is dropped. Logs the
/// host only (a link can carry anything in its path).
pub fn open(u: &url::Url) {
    if !for_browser(u) {
        tracing::info!(scheme = u.scheme(), "link: not opened (not a web link)");
        return;
    }
    let host = u.host_str().unwrap_or_default().to_string();
    match platform::open(u.as_str()) {
        Ok(()) => tracing::info!(host, "link: opened in the browser"),
        Err(e) => tracing::warn!(host, error = %e, "link: the browser didn't open"),
    }
}

#[cfg(windows)]
mod platform {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, PCWSTR, w};

    pub fn open(url: &str) -> Result<(), String> {
        // The shell's "open" verb on an http(s) URL starts the default browser.
        let r = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &HSTRING::from(url),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        // ShellExecuteW reports success as a value above 32.
        if r.0 as isize > 32 {
            Ok(())
        } else {
            Err(format!("ShellExecuteW returned {}", r.0 as isize))
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::process::{Command, Stdio};

    pub fn open(url: &str) -> Result<(), String> {
        // The URL is one argument (no shell); `--` would be read as a file by some openers, and the
        // URL always starts with http, so it can't pass for an option.
        let mut child = Command::new("xdg-open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        // Reaped off the caller's thread (xdg-open can wait for the browser to start).
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    pub fn open(_url: &str) -> Result<(), String> {
        Err("opening links isn't supported on this platform".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> url::Url {
        s.parse().unwrap()
    }

    #[test]
    fn web_links_go_to_the_browser() {
        for ok in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "http://example.com/",
            "https://www.instagram.com/reel/Cabc123/",
            "https://example.com:8443/a?b#c",
        ] {
            assert!(for_browser(&u(ok)), "{ok}");
        }
    }

    #[test]
    fn nothing_else_does() {
        for bad in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,hi",
            "tauri://localhost/",
            "mailto:alex@example.com",
            "ftp://example.com/x",
            // a name that reads as one site and goes to another
            "https://pulsechat.co.za@evil.example/",
            "https://user:pass@example.com/",
        ] {
            assert!(!for_browser(&u(bad)), "{bad}");
        }
    }

    #[test]
    fn the_main_window_loads_its_own_pages_only() {
        for own in [
            "tauri://localhost/",
            "http://tauri.localhost/index.html",
            "https://tauri.localhost/",
            "http://localhost:1420/",
            "http://127.0.0.1:1420/",
            "about:blank",
        ] {
            assert!(main_may_navigate(&u(own)), "{own}");
        }
        for site in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://pulsechat.co.za/",
            "http://example.com/",
            // a look-alike host is a website
            "https://tauri.localhost.evil.example/",
        ] {
            assert!(!main_may_navigate(&u(site)), "{site}");
        }
    }
}
