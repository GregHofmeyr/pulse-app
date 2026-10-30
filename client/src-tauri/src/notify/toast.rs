//! The Windows toast's XML: a title line and a body line. Kept apart from the WinRT calls so
//! it is tested on every platform.

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

pub fn toast_xml(title: &str, body: &str) -> String {
    format!(
        r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual></toast>"#,
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
            r#"<toast><visual><binding template="ToastGeneric"><text>sam</text><text>hello</text></binding></visual></toast>"#
        );
    }

    #[test]
    fn message_text_stays_text() {
        let x = toast_xml("sam & \"jo\"", "<b>hi</b> it's 1 > 0");
        assert!(x.contains("<text>sam &amp; &quot;jo&quot;</text>"));
        assert!(x.contains("<text>&lt;b&gt;hi&lt;/b&gt; it&apos;s 1 &gt; 0</text>"));
    }
}
