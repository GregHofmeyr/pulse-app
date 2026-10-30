//! The Windows toast's XML: a title line and a body line. Kept apart from the WinRT calls so
//! it is tested on every platform.

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
