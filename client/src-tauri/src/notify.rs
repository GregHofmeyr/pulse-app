//! Desktop notifications for messages. The page decides what to show and when (lib/notify.ts);
//! this shows it with the system's own notifications and reports a click back. One worker thread
//! talks to the system, so a slow notification service never holds up a command or the UI.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::Backend;
#[cfg(any(windows, test))]
mod toast;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::Backend;
#[cfg(not(any(target_os = "linux", windows)))]
mod none;
#[cfg(not(any(target_os = "linux", windows)))]
use none::Backend;

/// Emitted to the page when a notification is clicked (after the window is brought up).
pub const CLICK_EVENT: &str = "pulse://notification-click";
/// Toasts need an AppUserModelID that a Start menu shortcut carries. A dev run has no shortcut,
/// so it borrows PowerShell's (the toast then says it came from PowerShell).
pub const POWERSHELL_APP_ID: &str =
    "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";

// The page cuts the body (lib/notify.ts, 140 graphemes); these, in characters, are far above
// anything it sends (139 family emoji and "…" are 696), so its cut is the only one. They only stop
// a broken page from sending pages of text.
const TITLE_MAX: usize = 1000;
const BODY_MAX: usize = 2000;
/// Channel and message ids are 26-character ULIDs; Windows takes a tag of up to 64.
const ID_MAX: usize = 64;

/// What the page asks to show.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub title: String,
    pub body: String,
    pub channel_id: String,
    #[serde(default)]
    pub thread_id: Option<String>,
}

/// Where a clicked notification leads: the page opens that conversation (and the quote's replies).
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Click {
    pub channel_id: String,
    pub thread_id: Option<String>,
}

pub type OnClick = Arc<dyn Fn(Click) + Send + Sync>;

/// The installed app's AppUserModelID is the bundle identifier: the NSIS installer sets it on the
/// Start menu and desktop shortcuts (tauri-bundler's `SetLnkAppUserModelId "${BUNDLEID}"`).
pub fn app_id(identifier: &str, debug: bool) -> String {
    if debug {
        POWERSHELL_APP_ID.to_string()
    } else {
        identifier.to_string()
    }
}

/// Title and body on one line and capped; ids that can't be ids are dropped (the whole notice
/// for the channel, since a click must lead somewhere).
fn clean(n: Notice) -> Option<Notice> {
    let id_ok = |s: &str| !s.is_empty() && s.len() <= ID_MAX;
    if !id_ok(&n.channel_id) {
        return None;
    }
    let flat = |s: &str, max: usize| {
        let line = crate::logging::one_line(&s.replace(['\r', '\n', '\t'], " "), usize::MAX);
        let line = line.trim();
        if line.chars().count() <= max {
            return line.to_string();
        }
        let mut cut: String = line.chars().take(max - 1).collect();
        cut.push('…');
        cut
    };
    Some(Notice {
        title: flat(&n.title, TITLE_MAX),
        body: flat(&n.body, BODY_MAX),
        thread_id: n.thread_id.filter(|t| id_ok(t)),
        channel_id: n.channel_id,
    })
}

/// Why a notification didn't show.
#[derive(Debug)]
pub enum Failure {
    /// The system won't show any: notifications off for the app or the user, an app Windows
    /// doesn't know, no notification service. Says so once per session; messages still ding.
    Refused(String),
    /// The system couldn't take this one (its content). Says so every time (budgeted), so it
    /// never hides a refusal that comes later.
    BadNotice(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Refused(e) | Failure::BadNotice(e) => f.write_str(e),
        }
    }
}

/// Whether to log `f`: a refusal only the first time (`refused_said` remembers), anything else
/// always.
fn should_warn(f: &Failure, refused_said: &AtomicBool) -> bool {
    match f {
        Failure::Refused(_) => !refused_said.swap(true, Ordering::Relaxed),
        Failure::BadNotice(_) => true,
    }
}

static REFUSED_SAID: AtomicBool = AtomicBool::new(false);
static BAD_NOTICE_LOG: std::sync::Mutex<Option<crate::logging::LineBudget>> =
    std::sync::Mutex::new(None);

fn log_failure(f: &Failure) {
    if !should_warn(f, &REFUSED_SAID) {
        return;
    }
    match f {
        Failure::Refused(e) => {
            tracing::warn!(error = %e, "notifications: the system won't show them (no more warnings about this this session)")
        }
        Failure::BadNotice(e) => crate::logging::log_budgeted(
            &BAD_NOTICE_LOG,
            5,
            0.1,
            || tracing::warn!(error = %e, "notifications: the system couldn't take one"),
            |n| tracing::warn!("notifications: {n} lines dropped (too many)"),
        ),
    }
}

/// Hands notices to the worker thread; never blocks.
pub struct Notifier {
    tx: mpsc::Sender<Notice>,
}

impl Notifier {
    pub fn new(app_id: String, on_click: OnClick) -> Self {
        let (tx, rx) = mpsc::channel::<Notice>();
        let spawned = std::thread::Builder::new()
            .name("pulse-notify".into())
            .spawn(move || {
                let mut backend = Backend::new(app_id, on_click);
                for n in rx {
                    if let Err(f) = backend.show(&n) {
                        log_failure(&f);
                    }
                }
            });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "notifications: no worker thread");
        }
        Self { tx }
    }

    pub fn send(&self, n: Notice) {
        if let Some(n) = clean(n) {
            let _ = self.tx.send(n);
        }
    }
}

/// Show a message notification (the page has decided it should go out).
#[tauri::command]
pub async fn notify_message(
    notifier: tauri::State<'_, Notifier>,
    notice: Notice,
) -> Result<(), String> {
    notifier.send(notice);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notice_reads_the_ui_payload() {
        let n: Notice = serde_json::from_str(
            r#"{"title":"sam","body":"hello","channelId":"C1","threadId":"Q1"}"#,
        )
        .unwrap();
        assert_eq!(
            n,
            Notice {
                title: "sam".into(),
                body: "hello".into(),
                channel_id: "C1".into(),
                thread_id: Some("Q1".into()),
            }
        );
        let n: Notice =
            serde_json::from_str(r#"{"title":"sam","body":"hello","channelId":"C1"}"#).unwrap();
        assert_eq!(n.thread_id, None);
    }

    #[test]
    fn click_is_what_the_ui_listens_for() {
        let c = Click {
            channel_id: "C1".into(),
            thread_id: None,
        };
        assert_eq!(
            serde_json::to_string(&c).unwrap(),
            r#"{"channelId":"C1","threadId":null}"#
        );
        assert_eq!(CLICK_EVENT, "pulse://notification-click");
    }

    #[test]
    fn clean_keeps_it_short_and_on_one_line() {
        let n = clean(Notice {
            title: format!("sam\u{202E}{}", "x".repeat(500)),
            body: "b".repeat(1000),
            channel_id: "C1".into(),
            thread_id: Some("Q1".into()),
        })
        .unwrap();
        assert!(!n.title.contains('\u{202E}'));
        assert_eq!(n.channel_id, "C1");
        assert_eq!(n.thread_id.as_deref(), Some("Q1"));
    }

    #[test]
    fn what_the_page_cut_is_never_cut_again() {
        // the page's longest body: 139 family emoji (18 bytes each) and its "…"
        let body = format!("{}…", "👨‍👩‍👧".repeat(139));
        let name = "👨‍👩‍👧".repeat(32);
        let n = clean(Notice {
            title: name.clone(),
            body: body.clone(),
            channel_id: "C1".into(),
            thread_id: None,
        })
        .unwrap();
        assert_eq!(n.body, body);
        assert_eq!(n.title, name);
    }

    #[test]
    fn a_runaway_text_is_cut_on_a_character_with_its_ellipsis() {
        let n = clean(Notice {
            title: "é".repeat(TITLE_MAX + 50),
            body: "😀".repeat(BODY_MAX + 50),
            channel_id: "C1".into(),
            thread_id: None,
        })
        .unwrap();
        assert_eq!(n.body.chars().count(), BODY_MAX);
        assert!(n.body.ends_with("😀…"));
        assert_eq!(n.title.chars().count(), TITLE_MAX);
        assert!(n.title.ends_with("é…"));
    }

    #[test]
    fn clean_drops_a_notice_without_a_usable_channel() {
        let base = Notice {
            title: "sam".into(),
            body: "hi".into(),
            channel_id: String::new(),
            thread_id: None,
        };
        assert!(clean(base.clone()).is_none());
        assert!(
            clean(Notice {
                channel_id: "C".repeat(65),
                ..base.clone()
            })
            .is_none()
        );
        // a thread id that isn't an id is dropped, the notice stays
        let n = clean(Notice {
            channel_id: "C1".into(),
            thread_id: Some("Q".repeat(65)),
            ..base
        })
        .unwrap();
        assert_eq!(n.thread_id, None);
    }

    #[test]
    fn app_id_is_the_installers_outside_debug() {
        assert_eq!(app_id("app.pulse.client", false), "app.pulse.client");
        assert_eq!(app_id("app.pulse.client", true), POWERSHELL_APP_ID);
    }

    #[test]
    fn a_refusal_warns_once() {
        let flag = std::sync::atomic::AtomicBool::new(false);
        let refused = Failure::Refused("off for this app".into());
        assert!(should_warn(&refused, &flag));
        assert!(!should_warn(&refused, &flag));
        assert!(!should_warn(&refused, &flag));
    }

    #[test]
    fn a_bad_notice_never_hides_a_later_refusal() {
        let flag = std::sync::atomic::AtomicBool::new(false);
        let bad = Failure::BadNotice("the toast didn't load".into());
        assert!(should_warn(&bad, &flag));
        assert!(should_warn(&bad, &flag));
        assert!(should_warn(&Failure::Refused("off".into()), &flag));
    }
}
