//! Linux: freedesktop notifications over D-Bus (notify-rust on zbus). Each conversation keeps
//! one notification, replaced in place (its id), and one thread waits for its click or close.

use super::{Click, Notice, OnClick};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Which notification each conversation has on screen, and where a click on it leads.
#[derive(Default)]
struct Replacer {
    by_channel: HashMap<String, Entry>,
}

struct Entry {
    id: u32,
    click: Click,
}

impl Replacer {
    /// The id to replace for `channel` (its notification still on screen, as far as we know).
    fn replaces_id(&self, channel: &str) -> Option<u32> {
        self.by_channel.get(channel).map(|e| e.id)
    }

    /// The daemon shows `id` for `click`'s conversation. True: start a waiter for `id`. False: it
    /// replaced the one already being waited on, which now leads to `click`.
    fn shown(&mut self, id: u32, click: Click) -> bool {
        if let Some(e) = self.by_channel.get_mut(&click.channel_id)
            && e.id == id
        {
            e.click = click;
            return false;
        }
        self.by_channel
            .insert(click.channel_id.clone(), Entry { id, click });
        true
    }

    /// The waiter for `id` saw it clicked or closed: forget it; a click leads to its newest target.
    fn ended(&mut self, id: u32, clicked: bool) -> Option<Click> {
        let channel = self
            .by_channel
            .iter()
            .find(|(_, e)| e.id == id)
            .map(|(c, _)| c.clone())?;
        let e = self.by_channel.remove(&channel)?;
        clicked.then_some(e.click)
    }
}

/// For daemons that read the body as markup: `<`, `>` and `&` from a message stay text.
fn escape_markup(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub struct Backend {
    replacer: Arc<Mutex<Replacer>>,
    on_click: OnClick,
    /// Asked once: whether the daemon reads the body as markup.
    markup: Option<bool>,
}

impl Backend {
    pub fn new(_app_id: String, on_click: OnClick) -> Self {
        Backend {
            replacer: Arc::default(),
            on_click,
            markup: None,
        }
    }

    /// Blocks on D-Bus: called on the notifier thread only.
    pub fn show(&mut self, n: &Notice) -> Result<(), String> {
        let markup = *self.markup.get_or_insert_with(|| {
            notify_rust::get_capabilities()
                .map(|c| c.iter().any(|x| x == "body-markup"))
                .unwrap_or(false)
        });
        let replaces = lock(&self.replacer).replaces_id(&n.channel_id);
        let mut note = notify_rust::Notification::new();
        note.appname("Pulse")
            .summary(&n.title)
            .body(&if markup {
                escape_markup(&n.body)
            } else {
                n.body.clone()
            })
            // "default" is a click on the notification itself
            .action("default", "Open")
            .hint(notify_rust::Hint::Category("im.received".into()));
        if let Some(id) = replaces {
            note.id(id);
        }
        let handle = note.show().map_err(|e| e.to_string())?;
        let id = handle.id();
        let click = Click {
            channel_id: n.channel_id.clone(),
            thread_id: n.thread_id.clone(),
        };
        if !lock(&self.replacer).shown(id, click) {
            return Ok(());
        }
        let replacer = self.replacer.clone();
        let on_click = self.on_click.clone();
        // Waiting for the click blocks until the notification is clicked or closed: its own thread.
        std::thread::Builder::new()
            .name("pulse-notify-wait".into())
            .spawn(move || {
                let mut clicked = false;
                handle.wait_for_action(|action| clicked = action == "default");
                let target = lock(&replacer).ended(id, clicked);
                if let Some(c) = target {
                    on_click(c);
                }
            })
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

fn lock(r: &Mutex<Replacer>) -> std::sync::MutexGuard<'_, Replacer> {
    r.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click(channel: &str, thread: Option<&str>) -> Click {
        Click {
            channel_id: channel.into(),
            thread_id: thread.map(Into::into),
        }
    }

    #[test]
    fn the_first_one_replaces_nothing_and_gets_a_waiter() {
        let mut r = Replacer::default();
        assert_eq!(r.replaces_id("C1"), None);
        assert!(r.shown(7, click("C1", None)));
        assert_eq!(r.replaces_id("C1"), Some(7));
        assert_eq!(r.replaces_id("C2"), None);
    }

    #[test]
    fn a_replacement_reuses_the_waiter_and_a_click_gets_the_newest() {
        let mut r = Replacer::default();
        assert!(r.shown(7, click("C1", None)));
        // the daemon replaced 7 in place: the waiter on 7 is still there
        assert!(!r.shown(7, click("C1", Some("Q1"))));
        assert!(!r.shown(7, click("C1", Some("Q2"))));
        assert_eq!(r.ended(7, true), Some(click("C1", Some("Q2"))));
        // delivered once
        assert_eq!(r.ended(7, true), None);
        assert_eq!(r.replaces_id("C1"), None);
    }

    #[test]
    fn closed_forgets_the_conversation() {
        let mut r = Replacer::default();
        assert!(r.shown(7, click("C1", None)));
        assert_eq!(r.ended(7, false), None);
        assert_eq!(r.replaces_id("C1"), None);
        // a new one after that starts its own waiter
        assert!(r.shown(9, click("C1", None)));
    }

    #[test]
    fn a_new_id_for_the_conversation_gets_a_new_waiter() {
        let mut r = Replacer::default();
        assert!(r.shown(7, click("C1", None)));
        // the daemon didn't replace (or 7 was gone): a different id
        assert!(r.shown(8, click("C1", Some("Q1"))));
        // the old waiter ending changes nothing for the new one
        assert_eq!(r.ended(7, true), None);
        assert_eq!(r.replaces_id("C1"), Some(8));
        assert_eq!(r.ended(8, true), Some(click("C1", Some("Q1"))));
    }

    #[test]
    fn conversations_are_separate() {
        let mut r = Replacer::default();
        assert!(r.shown(7, click("C1", None)));
        assert!(r.shown(8, click("C2", None)));
        assert_eq!(r.ended(8, true), Some(click("C2", None)));
        assert_eq!(r.replaces_id("C1"), Some(7));
    }

    #[test]
    fn markup_is_escaped_when_the_daemon_reads_it() {
        assert_eq!(escape_markup("a < b & c > d"), "a &lt; b &amp; c &gt; d");
    }

    /// Sends one real notification to the running notification daemon. By hand only:
    /// `cargo test -p pulse-client --lib linux_desktop_notification_shows -- --ignored`
    #[test]
    #[ignore]
    fn linux_desktop_notification_shows() {
        let mut b = Backend::new(String::new(), std::sync::Arc::new(|_| {}));
        b.show(&Notice {
            title: "Pulse test notification".into(),
            body: "Sent by a test; nothing to do.".into(),
            channel_id: "TEST".into(),
            thread_id: None,
        })
        .expect("the notification daemon took it");
    }
}
