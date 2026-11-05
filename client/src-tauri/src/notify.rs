//! Desktop notifications for messages. The page decides what to show and when (lib/notify.ts);
//! this shows it with the system's own notifications and reports a click back. One worker thread
//! talks to the system, so a slow notification service never holds up a command or the UI.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

pub mod avatar;
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
/// How many conversations' notifications are remembered (for their clicks, and on Windows their
/// toast objects); the oldest is let go first.
#[cfg_attr(not(any(windows, target_os = "linux", test)), allow(dead_code))]
const KEEP: usize = 64;

/// What the page asks to show.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub title: String,
    pub body: String,
    pub channel_id: String,
    #[serde(default)]
    pub thread_id: Option<String>,
    /// The author's avatar (a media hash; an animated one's still), shown beside the text.
    #[serde(default)]
    pub avatar: Option<String>,
    /// That avatar as a local PNG, filled in by the core (never from the page).
    #[serde(skip)]
    pub image: Option<PathBuf>,
    /// The slot this notice replaces, and can be taken back by ([`Notifier::withdraw`], #176).
    /// Missing: the conversation's (`channel_id`), so a chat's messages replace each other. A
    /// call's ring has its own (`call-<id>`), apart from that chat's messages.
    #[serde(default)]
    pub tag: Option<String>,
}

impl Notice {
    /// The slot: Windows' toast tag, the Linux notification the next one replaces.
    pub fn key(&self) -> &str {
        self.tag.as_deref().unwrap_or(&self.channel_id)
    }
}

/// What the worker thread does: show a notice, or take back the one in a slot.
#[derive(Debug, PartialEq)]
enum Job {
    Show(Notice),
    Withdraw(String),
}

/// A system's notifications, as the worker drives them (one backend per platform).
trait Sink {
    fn show(&mut self, n: &Notice) -> Result<(), Failure>;
    /// Take back what's in slot `key`, if anything of ours is (best effort: a service that can't
    /// is left alone).
    fn withdraw(&mut self, key: &str);
}

impl Sink for Backend {
    fn show(&mut self, n: &Notice) -> Result<(), Failure> {
        Backend::show(self, n)
    }
    fn withdraw(&mut self, key: &str) {
        Backend::withdraw(self, key)
    }
}

/// The worker: every job in order, until the sender goes.
fn serve(rx: mpsc::Receiver<Job>, sink: &mut impl Sink) {
    for job in rx {
        match job {
            Job::Show(n) => {
                if let Err(f) = show_with_fallback(&n, |n| sink.show(n)) {
                    log_failure(&f);
                }
            }
            Job::Withdraw(key) => sink.withdraw(&key),
        }
    }
}

/// A slot key from the page: an id-like string (what [`clean`] keeps), or nothing.
fn clean_key(key: &str) -> Option<String> {
    (!key.is_empty() && key.len() <= ID_MAX && !key.chars().any(char::is_control))
        .then(|| key.to_string())
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
        avatar: n.avatar.filter(|h| crate::media::is_hash(h)),
        image: n.image,
        // a tag that can't be one: the notice goes in its conversation's slot
        tag: n.tag.as_deref().and_then(clean_key),
    })
}

/// Show `n`; one the system won't take with its avatar goes again without it, so an image can
/// only ever cost the image, never the notification.
fn show_with_fallback(
    n: &Notice,
    mut show: impl FnMut(&Notice) -> Result<(), Failure>,
) -> Result<(), Failure> {
    match show(n) {
        Err(_) if n.image.is_some() => show(&Notice {
            image: None,
            ..n.clone()
        }),
        r => r,
    }
}

/// A file path as the `file:///` URI notification services and toasts take.
#[cfg_attr(not(any(windows, target_os = "linux", test)), allow(dead_code))]
fn file_uri(path: &std::path::Path) -> Option<String> {
    url::Url::from_file_path(path).ok().map(String::from)
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
    tx: mpsc::Sender<Job>,
}

impl Notifier {
    pub fn new(app_id: String, on_click: OnClick) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        let spawned = std::thread::Builder::new()
            .name("pulse-notify".into())
            .spawn(move || serve(rx, &mut Backend::new(app_id, on_click)));
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "notifications: no worker thread");
        }
        Self { tx }
    }

    pub fn send(&self, n: Notice) {
        if let Some(n) = clean(n) {
            let _ = self.tx.send(Job::Show(n));
        }
    }

    /// Take back the notice in slot `key` (a [`Notice::tag`]), if one of ours is up: after
    /// anything sent before it, so a show and its withdraw can't cross.
    pub fn withdraw(&self, key: &str) {
        if let Some(key) = clean_key(key) {
            let _ = self.tx.send(Job::Withdraw(key));
        }
    }
}

/// Show a message notification (the page has decided it should go out), with its author's avatar
/// when that can be had quickly (see [avatar]).
#[tauri::command]
pub async fn notify_message(
    notifier: tauri::State<'_, Notifier>,
    core: tauri::State<'_, crate::commands::Core>,
    notice: Notice,
) -> Result<(), String> {
    let mut notice = Notice {
        image: None,
        ..notice
    };
    if let Some(hash) = notice.avatar.as_deref() {
        notice.image = avatar::local_png(core.media_session(), hash).await;
    }
    notifier.send(notice);
    Ok(())
}

/// Take back a notification shown with `tag` (#176: "sam is calling you", once the ring ends),
/// where the system allows it: Windows removes the toast from Action Center, Linux closes it.
#[tauri::command]
pub fn withdraw_notification(notifier: tauri::State<'_, Notifier>, tag: String) {
    notifier.withdraw(&tag);
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
                avatar: None,
                image: None,
                tag: None,
            }
        );
        let n: Notice =
            serde_json::from_str(r#"{"title":"sam","body":"hello","channelId":"C1"}"#).unwrap();
        assert_eq!(n.thread_id, None);
        assert_eq!(n.avatar, None);
    }

    #[test]
    fn the_avatar_is_a_hash_and_the_image_path_never_comes_from_the_page() {
        let hash = "ab".repeat(32);
        let n: Notice = serde_json::from_str(&format!(
            r#"{{"title":"sam","body":"hi","channelId":"C1","avatar":"{hash}","image":"/etc/passwd"}}"#
        ))
        .unwrap();
        assert_eq!(n.avatar.as_deref(), Some(hash.as_str()));
        assert_eq!(n.image, None);
        let n = clean(Notice {
            avatar: Some("../../x".into()),
            ..n
        })
        .unwrap();
        assert_eq!(n.avatar, None, "not a hash: dropped, the notice stays");
    }

    #[test]
    fn a_notice_refused_with_its_avatar_goes_again_without_it() {
        let n = Notice {
            title: "sam".into(),
            body: "hi".into(),
            channel_id: "C1".into(),
            thread_id: None,
            avatar: None,
            image: Some(PathBuf::from("/x/a.png")),
            tag: None,
        };
        let mut tried = Vec::new();
        let r = show_with_fallback(&n, |n| {
            tried.push(n.image.clone());
            if n.image.is_some() {
                Err(Failure::BadNotice("no image".into()))
            } else {
                Ok(())
            }
        });
        assert!(r.is_ok());
        assert_eq!(tried, [Some(PathBuf::from("/x/a.png")), None]);

        // without an image there is nothing to drop: one try, its error kept
        let mut tries = 0;
        let r = show_with_fallback(&Notice { image: None, ..n }, |_| {
            tries += 1;
            Err(Failure::Refused("off".into()))
        });
        assert!(matches!(r, Err(Failure::Refused(_))));
        assert_eq!(tries, 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_path_becomes_a_file_uri() {
        assert_eq!(
            file_uri(std::path::Path::new("/home/a b/x.png")).as_deref(),
            Some("file:///home/a%20b/x.png")
        );
        assert_eq!(file_uri(std::path::Path::new("relative.png")), None);
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
            avatar: None,
            image: None,
            tag: None,
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
            avatar: None,
            image: None,
            tag: None,
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
            avatar: None,
            image: None,
            tag: None,
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
            avatar: None,
            image: None,
            tag: None,
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

    fn notice(channel: &str, tag: Option<&str>) -> Notice {
        Notice {
            title: "sam is calling you".into(),
            body: "Incoming call".into(),
            channel_id: channel.into(),
            thread_id: None,
            avatar: None,
            image: None,
            tag: tag.map(Into::into),
        }
    }

    /// #176: a ring's notice has its own slot, apart from its conversation's messages.
    #[test]
    fn a_tag_is_the_notices_slot_and_without_one_the_conversation_is() {
        let n: Notice = serde_json::from_str(
            r#"{"title":"sam is calling you","body":"Incoming call","channelId":"C1","tag":"call-K1"}"#,
        )
        .unwrap();
        assert_eq!(n.tag.as_deref(), Some("call-K1"));
        assert_eq!(n.key(), "call-K1");
        assert_eq!(notice("C1", None).key(), "C1");
    }

    #[test]
    fn a_tag_that_cant_be_one_falls_back_to_the_conversation() {
        for bad in ["", &"x".repeat(65), "call-\u{7}K1"] {
            let n = clean(notice("C1", Some(bad))).unwrap();
            assert_eq!(n.tag, None, "{bad:?}");
            assert_eq!(n.key(), "C1");
        }
        let n = clean(notice("C1", Some("call-K1"))).unwrap();
        assert_eq!(n.key(), "call-K1");
        assert_eq!(clean_key("call-K1").as_deref(), Some("call-K1"));
        assert_eq!(clean_key(""), None);
        assert_eq!(clean_key(&"x".repeat(65)), None);
    }

    /// What the worker did, in order.
    #[derive(Default)]
    struct Fake {
        log: Vec<String>,
        refuse_images: bool,
    }

    impl Sink for Fake {
        fn show(&mut self, n: &Notice) -> Result<(), Failure> {
            if self.refuse_images && n.image.is_some() {
                return Err(Failure::BadNotice("no image".into()));
            }
            self.log.push(format!("show {}", n.key()));
            Ok(())
        }
        fn withdraw(&mut self, key: &str) {
            self.log.push(format!("withdraw {key}"));
        }
    }

    #[test]
    fn the_worker_shows_and_withdraws_in_the_order_asked() {
        let (tx, rx) = mpsc::channel();
        tx.send(Job::Show(notice("C1", Some("call-K1")))).unwrap();
        tx.send(Job::Show(notice("C1", None))).unwrap();
        tx.send(Job::Withdraw("call-K1".into())).unwrap();
        drop(tx);
        let mut fake = Fake::default();
        serve(rx, &mut fake);
        // the message notice in the same conversation is left alone
        assert_eq!(fake.log, ["show call-K1", "show C1", "withdraw call-K1"]);
    }

    #[test]
    fn the_worker_still_falls_back_without_the_avatar() {
        let (tx, rx) = mpsc::channel();
        tx.send(Job::Show(Notice {
            image: Some(PathBuf::from("/x/a.png")),
            ..notice("C1", Some("call-K1"))
        }))
        .unwrap();
        drop(tx);
        let mut fake = Fake {
            refuse_images: true,
            ..Fake::default()
        };
        serve(rx, &mut fake);
        assert_eq!(fake.log, ["show call-K1"]);
    }

    #[test]
    fn withdraw_sends_only_a_usable_key() {
        let (tx, rx) = mpsc::channel();
        let n = Notifier { tx };
        n.withdraw("call-K1");
        n.withdraw("");
        n.withdraw(&"x".repeat(65));
        drop(n);
        assert_eq!(
            rx.iter().collect::<Vec<_>>(),
            [Job::Withdraw("call-K1".into())]
        );
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
