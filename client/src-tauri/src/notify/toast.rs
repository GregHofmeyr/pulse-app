//! The Windows toast's XML: a title line and a body line. Kept apart from the WinRT calls so
//! it is tested on every platform.

use super::KEEP;
use std::collections::{HashMap, VecDeque};

/// The newest toast per conversation, for at most [KEEP] conversations: the oldest goes first.
pub struct KeepNewest<V> {
    by_channel: HashMap<String, V>,
    order: VecDeque<String>,
}

impl<V> Default for KeepNewest<V> {
    fn default() -> Self {
        KeepNewest {
            by_channel: HashMap::new(),
            order: VecDeque::new(),
        }
    }
}

impl<V> KeepNewest<V> {
    pub fn insert(&mut self, channel: String, v: V) {
        if self.by_channel.insert(channel.clone(), v).is_some() {
            self.order.retain(|c| *c != channel);
        }
        self.order.push_back(channel);
        while self.order.len() > KEEP {
            if let Some(old) = self.order.pop_front() {
                self.by_channel.remove(&old);
            }
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.by_channel.len()
    }

    #[cfg(test)]
    fn contains(&self, channel: &str) -> bool {
        self.by_channel.contains_key(channel)
    }
}

/// Why the notifier won't show toasts, from its `NotificationSetting` (0 = enabled). A value
/// newer than these isn't taken as a refusal.
pub fn refusal(setting: i32) -> Option<&'static str> {
    match setting {
        1 => Some("notifications are off for Pulse"),
        2 => Some("notifications are off for this user"),
        3 => Some("notifications are off by group policy"),
        4 => Some("Windows doesn't know this app (no Start menu shortcut with its AppUserModelID)"),
        _ => None,
    }
}

/// XML 1.0's `Char`: tab, newline, carriage return and everything from space up, except
/// U+FFFE and U+FFFF (a Rust `char` is never a surrogate).
fn xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{FFFD}' | '\u{10000}'..)
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if xml_char(c) => out.push(c),
            // XML 1.0 can't hold it even escaped, and one would make the whole toast fail to load
            _ => {}
        }
    }
    out
}

/// Silent: Pulse already plays its own message sound, so one alert makes one sound.
pub fn toast_xml(title: &str, body: &str) -> String {
    format!(
        r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual><audio silent="true"/></toast>"#,
        escape(title),
        escape(body)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_text_lines_in_the_generic_template() {
        assert_eq!(
            toast_xml("sam", "hello"),
            r#"<toast><visual><binding template="ToastGeneric"><text>sam</text><text>hello</text></binding></visual><audio silent="true"/></toast>"#
        );
    }

    #[test]
    fn the_notifiers_setting_says_whether_it_will_show() {
        // Windows' NotificationSetting values
        assert_eq!(refusal(0), None);
        assert_eq!(refusal(1), Some("notifications are off for Pulse"));
        assert_eq!(refusal(2), Some("notifications are off for this user"));
        assert_eq!(refusal(3), Some("notifications are off by group policy"));
        assert_eq!(
            refusal(4),
            Some("Windows doesn't know this app (no Start menu shortcut with its AppUserModelID)")
        );
        // a value newer than we know doesn't stop a toast
        assert_eq!(refusal(9), None);
    }

    #[test]
    fn kept_toasts_are_capped_and_the_oldest_conversation_goes() {
        let mut k = KeepNewest::default();
        for i in 0..KEEP + 3 {
            k.insert(format!("C{i}"), i);
        }
        assert_eq!(k.len(), KEEP);
        assert!(!k.contains("C0") && !k.contains("C1") && !k.contains("C2"));
        assert!(k.contains("C3") && k.contains(&format!("C{}", KEEP + 2)));
    }

    #[test]
    fn a_new_toast_for_a_kept_conversation_replaces_it_and_makes_it_newest() {
        let mut k = KeepNewest::default();
        for i in 0..KEEP {
            k.insert(format!("C{i}"), i);
        }
        k.insert("C0".into(), 100);
        assert_eq!(k.len(), KEEP);
        k.insert("NEW".into(), 0);
        // C1 was the oldest now, not C0
        assert!(k.contains("C0") && !k.contains("C1"));
    }

    #[test]
    fn characters_xml_cannot_hold_are_left_out() {
        // (lone surrogates can't be in a Rust string, so these are all that's left to drop)
        let x = toast_xml("sam\u{FFFE}", "a\u{FFFF}b\u{0}c\u{1}d\u{B}e\u{1F}f");
        assert!(x.contains("<text>sam</text>"));
        assert!(x.contains("<text>abcdef</text>"));
        // tab, newline, emoji and the rest of Unicode stay
        assert!(
            toast_xml("t", "a\tb\nc😀\u{FFFD}\u{E000}")
                .contains("<text>a\tb\nc😀\u{FFFD}\u{E000}</text>")
        );
    }

    #[test]
    fn message_text_stays_text() {
        let x = toast_xml("sam & \"jo\"", "<b>hi</b> it's 1 > 0");
        assert!(x.contains("<text>sam &amp; &quot;jo&quot;</text>"));
        assert!(x.contains("<text>&lt;b&gt;hi&lt;/b&gt; it&apos;s 1 &gt; 0</text>"));
    }
}
