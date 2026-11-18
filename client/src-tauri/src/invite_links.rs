//! `pulse://invite/<code>` links (#476): the invite page's "Open in Pulse".
//!
//! The scheme is registered for the user at startup on Windows (`tauri-plugin-deep-link`), and a second launch hands
//! its link to the running app (`tauri-plugin-single-instance`'s `deep-link` feature). Only `pulse://invite/<code>`
//! with a plain code is acted on, and only by telling the UI (the `invite-link` event, or `take_invite_link` for a
//! link that started the app before the UI listened), which shows the invite's card: nothing joins on its own.

use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, State};

/// A link that arrived before the UI took it.
#[derive(Default)]
pub struct Pending(Mutex<Option<String>>);

/// The invite code in `url`, if it's exactly `pulse://invite/<code>` (letters and digits, at most 32).
pub fn code_in(url: &str) -> Option<String> {
    let rest = url.strip_prefix("pulse://invite/")?;
    let code = rest.trim_end_matches('/');
    (!code.is_empty() && code.len() <= 32 && code.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then(|| code.to_string())
}

/// Act on links that opened the app: the newest invite among them is kept for the UI and announced.
pub fn opened(app: &AppHandle, urls: impl IntoIterator<Item = String>) {
    let Some(code) = urls.into_iter().filter_map(|u| code_in(&u)).last() else {
        return;
    };
    tracing::info!("invite link opened");
    if let Some(p) = app.try_state::<Pending>() {
        *p.0.lock().unwrap() = Some(code.clone());
    }
    let _ = app.emit_to("main", "invite-link", code);
    crate::bring_to_front(app);
}

/// The invite link that opened the app, once (the UI asks when it starts listening).
#[tauri::command]
pub fn take_invite_link(pending: State<'_, Pending>) -> Option<String> {
    pending.0.lock().unwrap().take()
}

#[cfg(test)]
mod tests {
    use super::code_in;

    #[test]
    fn only_a_plain_invite_code_is_taken() {
        assert_eq!(
            code_in("pulse://invite/abcDEF2345"),
            Some("abcDEF2345".into())
        );
        assert_eq!(
            code_in("pulse://invite/abcDEF2345/"),
            Some("abcDEF2345".into())
        );
        for bad in [
            "pulse://invite/",
            "pulse://invite/abc?join=1",
            "pulse://invite/../settings",
            "pulse://join/abcDEF2345",
            "https://evil.example/invite/abcDEF2345",
            "pulse://invite/abc def",
            &format!("pulse://invite/{}", "a".repeat(33)),
        ] {
            assert_eq!(code_in(bad), None, "{bad}");
        }
    }
}
