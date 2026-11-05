//! Linux: freedesktop notifications over D-Bus (zbus, pure Rust). Each slot (a conversation, or a
//! call's ring: [`Notice::key`]) keeps one notification, replaced in place by id, and closed by id
//! when it is taken back (#176: `CloseNotification`, which every service has to offer).
//!
//! One connection sends every notification and one thread on it hears the clicks. Some
//! notification services send the click only to the connection that sent the notification
//! (KDE's does), so a replacement sent from a new connection, as notify-rust's `show` does,
//! would lose its click; and one thread per notification would pile up behind a busy chat.

use super::{Click, Failure, KEEP, Notice, OnClick, file_uri};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::Value;

const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const IFACE: &str = "org.freedesktop.Notifications";

/// Which notification each slot has on screen, and where a click on each of ours leads.
#[derive(Default)]
struct Tracker {
    by_key: HashMap<String, u32>,
    by_id: HashMap<u32, Click>,
    /// Oldest first, for forgetting beyond [KEEP].
    order: VecDeque<u32>,
}

impl Tracker {
    /// The id to replace for slot `key` (its notification still on screen, as far as we know).
    fn replaces_id(&self, key: &str) -> Option<u32> {
        self.by_key.get(key).copied()
    }

    /// The service shows `id` in slot `key` (new, or replaced in place); a click leads to `click`.
    fn shown(&mut self, id: u32, key: &str, click: Click) {
        self.by_key.insert(key.to_string(), id);
        if self.by_id.insert(id, click).is_none() {
            self.order.push_back(id);
        }
        while self.order.len() > KEEP {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            self.by_id.remove(&old);
            self.by_key.retain(|_, v| *v != old);
        }
    }

    /// `id` was clicked: where it leads, if it is ours.
    fn clicked(&self, id: u32) -> Option<Click> {
        self.by_id.get(&id).cloned()
    }

    /// `id` closed: the next one for its conversation is a new notification. Its click target is
    /// kept, for services that say "closed" for an id they only replaced.
    fn closed(&mut self, id: u32) {
        self.by_key.retain(|_, v| *v != id);
    }

    /// Take slot `key` back: the id to close, if one of ours is on screen there. Its click target
    /// is kept, in case the service shows it a moment longer.
    fn withdraw(&mut self, key: &str) -> Option<u32> {
        self.by_key.remove(key)
    }
}

/// For services that read the body as markup: `<`, `>` and `&` from a message stay text.
fn escape_markup(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub struct Backend {
    conn: Option<Connection>,
    tracker: Arc<Mutex<Tracker>>,
    on_click: OnClick,
    /// Whether the service reads the body as markup, once it has answered.
    markup: Option<bool>,
}

impl Backend {
    pub fn new(_app_id: String, on_click: OnClick) -> Self {
        Backend {
            conn: None,
            tracker: Arc::default(),
            on_click,
            markup: None,
        }
    }

    /// Blocks on D-Bus: called on the notifier thread only. Every failure is the service's (none
    /// running, or it refused): the text was cleaned (no control characters), so D-Bus takes it.
    pub fn show(&mut self, n: &Notice) -> Result<(), Failure> {
        let refused = |e: zbus::Error| Failure::Refused(e.to_string());
        let conn = self.connection().map_err(refused)?;
        if self.markup.is_none() {
            // asked again next time if the service isn't there yet
            if let Ok(reply) =
                conn.call_method(Some(DEST), PATH, Some(IFACE), "GetCapabilities", &())
                && let Ok(caps) = reply.body().deserialize::<Vec<String>>()
            {
                self.markup = Some(caps.iter().any(|c| c == "body-markup"));
            }
        }
        let body = if self.markup == Some(true) {
            escape_markup(&n.body)
        } else {
            n.body.clone()
        };
        let replaces = lock(&self.tracker).replaces_id(n.key()).unwrap_or(0);
        let mut hints: HashMap<&str, Value> = HashMap::new();
        hints.insert("category", Value::from("im.received"));
        // Pulse plays its own message sound
        hints.insert("suppress-sound", Value::from(true));
        // the author's avatar (the spec's image-path: a file:// URI); services without image
        // support ignore it
        if let Some(uri) = n.image.as_deref().and_then(file_uri) {
            hints.insert("image-path", Value::from(uri));
        }
        // "default" is a click on the notification itself
        let actions = ["default", "Open"];
        let reply = conn
            .call_method(
                Some(DEST),
                PATH,
                Some(IFACE),
                "Notify",
                &(
                    "Pulse",
                    replaces,
                    "",
                    &n.title,
                    &body,
                    &actions[..],
                    hints,
                    -1i32,
                ),
            )
            .map_err(refused)?;
        let id: u32 = reply.body().deserialize().map_err(refused)?;
        lock(&self.tracker).shown(
            id,
            n.key(),
            Click {
                channel_id: n.channel_id.clone(),
                thread_id: n.thread_id.clone(),
            },
        );
        Ok(())
    }

    /// Close what's in slot `key`, if one of ours is (blocking, on the notifier thread). A service
    /// that can't close it leaves it up: nothing more to do.
    pub fn withdraw(&mut self, key: &str) {
        let Some(id) = lock(&self.tracker).withdraw(key) else {
            return;
        };
        let Some(conn) = &self.conn else { return };
        if let Err(e) = conn.call_method(Some(DEST), PATH, Some(IFACE), "CloseNotification", &(id,))
        {
            tracing::debug!(error = %e, "notifications: couldn't close one");
        }
    }

    /// The session bus connection, made (with its click listener) on first use.
    fn connection(&mut self) -> zbus::Result<Connection> {
        if let Some(c) = &self.conn {
            return Ok(c.clone());
        }
        let conn = Connection::session()?;
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(IFACE)?
            .path(PATH)?
            .build();
        let signals = MessageIterator::for_match_rule(rule, &conn, None)?;
        let tracker = self.tracker.clone();
        let on_click = self.on_click.clone();
        std::thread::Builder::new()
            .name("pulse-notify-listen".into())
            .spawn(move || listen(signals, &tracker, &on_click))
            .map_err(|e| zbus::Error::Failure(e.to_string()))?;
        self.conn = Some(conn.clone());
        Ok(conn)
    }
}

/// Clicks and closes, for the app's lifetime (every app's notifications pass by: only ours count).
fn listen(signals: MessageIterator, tracker: &Mutex<Tracker>, on_click: &OnClick) {
    for msg in signals {
        let Ok(msg) = msg else { continue };
        let header = msg.header();
        match header.member().map(|m| m.as_str()) {
            Some("ActionInvoked") => {
                if let Ok((id, action)) = msg.body().deserialize::<(u32, String)>()
                    && action == "default"
                    && let Some(c) = lock(tracker).clicked(id)
                {
                    on_click(c);
                }
            }
            Some("NotificationClosed") => {
                if let Ok((id, _reason)) = msg.body().deserialize::<(u32, u32)>() {
                    lock(tracker).closed(id);
                }
            }
            _ => {}
        }
    }
}

fn lock(t: &Mutex<Tracker>) -> std::sync::MutexGuard<'_, Tracker> {
    t.lock().unwrap_or_else(|e| e.into_inner())
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
    fn the_first_one_replaces_nothing() {
        let mut t = Tracker::default();
        assert_eq!(t.replaces_id("C1"), None);
        t.shown(7, "C1", click("C1", None));
        assert_eq!(t.replaces_id("C1"), Some(7));
        assert_eq!(t.replaces_id("C2"), None);
    }

    #[test]
    fn a_click_leads_to_the_newest_replacement() {
        let mut t = Tracker::default();
        t.shown(7, "C1", click("C1", None));
        t.shown(7, "C1", click("C1", Some("Q1")));
        t.shown(7, "C1", click("C1", Some("Q2")));
        assert_eq!(t.clicked(7), Some(click("C1", Some("Q2"))));
    }

    #[test]
    fn another_apps_notification_is_not_ours() {
        let mut t = Tracker::default();
        t.shown(7, "C1", click("C1", None));
        assert_eq!(t.clicked(99), None);
    }

    #[test]
    fn closed_stops_replacing_but_a_late_click_still_leads_there() {
        // some daemons say "closed" for an id they replaced in place: the one on screen must
        // still open its conversation
        let mut t = Tracker::default();
        t.shown(7, "C1", click("C1", Some("Q1")));
        t.closed(7);
        assert_eq!(t.replaces_id("C1"), None);
        assert_eq!(t.clicked(7), Some(click("C1", Some("Q1"))));
    }

    #[test]
    fn a_closed_id_does_not_touch_a_newer_one() {
        let mut t = Tracker::default();
        t.shown(7, "C1", click("C1", None));
        t.shown(8, "C1", click("C1", None));
        t.closed(7);
        assert_eq!(t.replaces_id("C1"), Some(8));
    }

    #[test]
    fn conversations_are_separate() {
        let mut t = Tracker::default();
        t.shown(7, "C1", click("C1", None));
        t.shown(8, "C2", click("C2", None));
        assert_eq!(t.clicked(8), Some(click("C2", None)));
        assert_eq!(t.replaces_id("C1"), Some(7));
    }

    #[test]
    fn it_remembers_a_bounded_number_of_ids() {
        let mut t = Tracker::default();
        for id in 0..(KEEP as u32 + 10) {
            t.shown(id, &format!("C{id}"), click(&format!("C{id}"), None));
        }
        assert_eq!(t.clicked(0), None);
        assert_eq!(t.replaces_id("C0"), None);
        let last = KEEP as u32 + 9;
        assert_eq!(t.clicked(last), Some(click(&format!("C{last}"), None)));
    }

    /// #176: a call's ring has its own slot beside its conversation's messages; taking it back
    /// closes that one only, and a later ring is a new notification.
    #[test]
    fn a_ring_is_taken_back_by_its_slot_and_messages_stay() {
        let mut t = Tracker::default();
        t.shown(7, "C1", click("C1", None));
        t.shown(8, "call-K1", click("C1", None));
        assert_eq!(t.replaces_id("C1"), Some(7));
        assert_eq!(t.replaces_id("call-K1"), Some(8));
        assert_eq!(t.withdraw("call-K1"), Some(8));
        assert_eq!(t.withdraw("call-K1"), None, "once");
        assert_eq!(t.replaces_id("call-K1"), None);
        assert_eq!(t.replaces_id("C1"), Some(7), "the message stays");
        assert_eq!(
            t.clicked(8),
            Some(click("C1", None)),
            "a late click still leads there"
        );
        assert_eq!(
            t.withdraw("call-K9"),
            None,
            "nothing of ours: nothing to close"
        );
    }

    #[test]
    fn a_ring_the_service_already_closed_is_not_closed_again() {
        let mut t = Tracker::default();
        t.shown(8, "call-K1", click("C1", None));
        t.closed(8);
        assert_eq!(t.withdraw("call-K1"), None);
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
            avatar: None,
            image: None,
            tag: None,
        })
        .expect("the notification daemon took it");
    }

    /// #176 by hand: a ring's notification shows, then closes 3 s later (if the daemon closes on
    /// request). `cargo test -p pulse-client --lib linux_ring_notification_is_taken_back -- --ignored`
    #[test]
    #[ignore]
    fn linux_ring_notification_is_taken_back() {
        let mut b = Backend::new(String::new(), std::sync::Arc::new(|_| {}));
        b.show(&Notice {
            title: "sam is calling you".into(),
            body: "Sent by a test; it should close by itself in 3 s.".into(),
            channel_id: "TEST".into(),
            thread_id: None,
            avatar: None,
            image: None,
            tag: Some("call-TEST".into()),
        })
        .expect("the notification daemon took it");
        std::thread::sleep(std::time::Duration::from_secs(3));
        b.withdraw("call-TEST");
        assert_eq!(lock(&b.tracker).replaces_id("call-TEST"), None);
    }
}
