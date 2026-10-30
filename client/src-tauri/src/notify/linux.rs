//! Linux: freedesktop notifications over D-Bus (zbus, pure Rust). Each conversation keeps one
//! notification, replaced in place by id.
//!
//! One connection sends every notification and one thread on it hears the clicks. Some
//! notification services send the click only to the connection that sent the notification
//! (KDE's does), so a replacement sent from a new connection, as notify-rust's `show` does,
//! would lose its click; and one thread per notification would pile up behind a busy chat.

use super::{Click, Failure, Notice, OnClick};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::Value;

const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const IFACE: &str = "org.freedesktop.Notifications";
/// How many of our notification ids are remembered for a click.
const KEEP: usize = 64;

/// Which notification each conversation has on screen, and where a click on each of ours leads.
#[derive(Default)]
struct Tracker {
    by_channel: HashMap<String, u32>,
    by_id: HashMap<u32, Click>,
    /// Oldest first, for forgetting beyond [KEEP].
    order: VecDeque<u32>,
}

impl Tracker {
    /// The id to replace for `channel` (its notification still on screen, as far as we know).
    fn replaces_id(&self, channel: &str) -> Option<u32> {
        self.by_channel.get(channel).copied()
    }

    /// The service shows `id` for `click`'s conversation (new, or replaced in place).
    fn shown(&mut self, id: u32, click: Click) {
        self.by_channel.insert(click.channel_id.clone(), id);
        if self.by_id.insert(id, click).is_none() {
            self.order.push_back(id);
        }
        while self.order.len() > KEEP {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            self.by_id.remove(&old);
            self.by_channel.retain(|_, v| *v != old);
        }
    }

    /// `id` was clicked: where it leads, if it is ours.
    fn clicked(&self, id: u32) -> Option<Click> {
        self.by_id.get(&id).cloned()
    }

    /// `id` closed: the next one for its conversation is a new notification. Its click target is
    /// kept, for services that say "closed" for an id they only replaced.
    fn closed(&mut self, id: u32) {
        self.by_channel.retain(|_, v| *v != id);
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
        let replaces = lock(&self.tracker).replaces_id(&n.channel_id).unwrap_or(0);
        let mut hints: HashMap<&str, Value> = HashMap::new();
        hints.insert("category", Value::from("im.received"));
        // Pulse plays its own message sound
        hints.insert("suppress-sound", Value::from(true));
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
            Click {
                channel_id: n.channel_id.clone(),
                thread_id: n.thread_id.clone(),
            },
        );
        Ok(())
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
        t.shown(7, click("C1", None));
        assert_eq!(t.replaces_id("C1"), Some(7));
        assert_eq!(t.replaces_id("C2"), None);
    }

    #[test]
    fn a_click_leads_to_the_newest_replacement() {
        let mut t = Tracker::default();
        t.shown(7, click("C1", None));
        t.shown(7, click("C1", Some("Q1")));
        t.shown(7, click("C1", Some("Q2")));
        assert_eq!(t.clicked(7), Some(click("C1", Some("Q2"))));
    }

    #[test]
    fn another_apps_notification_is_not_ours() {
        let mut t = Tracker::default();
        t.shown(7, click("C1", None));
        assert_eq!(t.clicked(99), None);
    }

    #[test]
    fn closed_stops_replacing_but_a_late_click_still_leads_there() {
        // some daemons say "closed" for an id they replaced in place: the one on screen must
        // still open its conversation
        let mut t = Tracker::default();
        t.shown(7, click("C1", Some("Q1")));
        t.closed(7);
        assert_eq!(t.replaces_id("C1"), None);
        assert_eq!(t.clicked(7), Some(click("C1", Some("Q1"))));
    }

    #[test]
    fn a_closed_id_does_not_touch_a_newer_one() {
        let mut t = Tracker::default();
        t.shown(7, click("C1", None));
        t.shown(8, click("C1", None));
        t.closed(7);
        assert_eq!(t.replaces_id("C1"), Some(8));
    }

    #[test]
    fn conversations_are_separate() {
        let mut t = Tracker::default();
        t.shown(7, click("C1", None));
        t.shown(8, click("C2", None));
        assert_eq!(t.clicked(8), Some(click("C2", None)));
        assert_eq!(t.replaces_id("C1"), Some(7));
    }

    #[test]
    fn it_remembers_a_bounded_number_of_ids() {
        let mut t = Tracker::default();
        for id in 0..(KEEP as u32 + 10) {
            t.shown(id, click(&format!("C{id}"), None));
        }
        assert_eq!(t.clicked(0), None);
        assert_eq!(t.replaces_id("C0"), None);
        let last = KEEP as u32 + 9;
        assert_eq!(t.clicked(last), Some(click(&format!("C{last}"), None)));
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
