//! Desktop notifications for messages. The page decides what to show and when (lib/notify.ts);
//! this shows it with the system's own notifications and reports a click back. One worker thread
//! talks to the system, so a slow notification service never holds up a command or the UI.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

mod none;
use none::Backend;

/// Emitted to the page when a notification is clicked (after the window is brought up).
pub const CLICK_EVENT: &str = "pulse://notification-click";
/// Toasts need an AppUserModelID that a Start menu shortcut carries. A dev run has no shortcut,
/// so it borrows PowerShell's (the toast then says it came from PowerShell).
pub const POWERSHELL_APP_ID: &str =
    "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";

// The page cuts the body at 140 characters; these only stop a broken page from sending pages of text.
const TITLE_MAX: usize = 200;
const BODY_MAX: usize = 400;
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
    let flat = |s: &str, max| {
        crate::logging::one_line(&s.replace(['\r', '\n', '\t'], " "), max)
            .trim()
            .to_string()
    };
    Some(Notice {
        title: flat(&n.title, TITLE_MAX),
        body: flat(&n.body, BODY_MAX),
        thread_id: n.thread_id.filter(|t| id_ok(t)),
        channel_id: n.channel_id,
    })
}

/// True only the first time: the system refusing (notifications off for the app, no
/// notification service) is said once per session, then Pulse stays quiet. Messages still ding.
fn first_failure(flag: &AtomicBool) -> bool {
    !flag.swap(true, Ordering::Relaxed)
}

static FAILED: AtomicBool = AtomicBool::new(false);

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
                    if let Err(e) = backend.show(&n)
                        && first_failure(&FAILED)
                    {
                        tracing::warn!(error = %e, "notifications: the system didn't show one (no more warnings this session)");
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
        assert!(n.title.len() <= TITLE_MAX);
        assert!(n.body.len() <= BODY_MAX);
        assert_eq!(n.channel_id, "C1");
        assert_eq!(n.thread_id.as_deref(), Some("Q1"));
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
    fn a_failure_warns_once() {
        let flag = std::sync::atomic::AtomicBool::new(false);
        assert!(first_failure(&flag));
        assert!(!first_failure(&flag));
        assert!(!first_failure(&flag));
    }
}
