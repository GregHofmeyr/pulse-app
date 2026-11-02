//! The links in a message that get a preview. No network here.

/// Most links one message gets previews for.
pub const MAX_LINKS: usize = 3;

/// The first [`MAX_LINKS`] distinct `http`/`https` links in `content`, normalised. Links in
/// code (inline or fenced) and links wrapped as `<https://…>` (no preview, as on Discord) are
/// skipped. Trailing punctuation and an unbalanced `)` or `]` aren't part of a link, so one at the
/// end of a sentence, in parentheses or in a markdown link comes out right.
pub fn extract(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in outside_code(content) {
        for raw in candidates(part) {
            let Some(url) = normalise(raw) else { continue };
            if !out.contains(&url) {
                out.push(url);
                if out.len() == MAX_LINKS {
                    return out;
                }
            }
        }
    }
    out
}

/// `url` parsed and printed back, for comparing and storing links: `http`/`https` with a host
/// only, without its fragment. `None` for anything else.
pub fn normalise(url: &str) -> Option<String> {
    let mut u = reqwest::Url::parse(url).ok()?;
    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none_or(str::is_empty) {
        return None;
    }
    u.set_fragment(None);
    Some(u.into())
}

/// The text outside code: a run of backticks opens code that the next run of the same length
/// closes, which covers inline code and fenced blocks alike. A run nothing closes is just text.
fn outside_code(content: &str) -> Vec<&str> {
    let b = content.as_bytes();
    let run = |at: usize| b[at..].iter().take_while(|&&c| c == b'`').count();
    // Where the run of `n` backticks closing the one before `from` starts.
    let closing = |mut from: usize, n: usize| {
        while let Some(k) = b[from..].iter().position(|&c| c == b'`') {
            let m = run(from + k);
            if m == n {
                return Some(from + k);
            }
            from += k + m;
        }
        None
    };
    let mut parts = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < b.len() {
        if b[i] != b'`' {
            i += 1;
            continue;
        }
        let n = run(i);
        match closing(i + n, n) {
            Some(c) => {
                // Backticks are ASCII, so these are char boundaries.
                parts.push(&content[start..i]);
                i = c + n;
                start = i;
            }
            None => i += n,
        }
    }
    parts.push(&content[start..]);
    parts
}

/// The raw links in `text` (no code in it), trimmed but not yet parsed.
fn candidates(text: &str) -> Vec<&str> {
    // ASCII lowercasing keeps every byte where it was.
    let lower = text.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(n) = lower[i..].find("http") {
        let at = i + n;
        i = at + "http".len();
        let rest = &lower[at..];
        if !rest.starts_with("http://") && !rest.starts_with("https://") {
            continue;
        }
        // A link starts a word: `xhttps://` isn't one.
        let prev = text[..at].chars().next_back();
        if prev.is_some_and(char::is_alphanumeric) {
            continue;
        }
        let end = text[at..]
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"'))
            .map_or(text.len(), |n| at + n);
        i = end;
        // `<https://…>`: the sender asked for no preview.
        if prev == Some('<') && text[end..].starts_with('>') {
            continue;
        }
        out.push(trim(&text[at..end]));
    }
    out
}

/// Drop what ends a sentence rather than a link: trailing punctuation, and closing brackets
/// that nothing in the link opened.
fn trim(mut s: &str) -> &str {
    let unbalanced =
        |s: &str, open: char, close: char| s.matches(open).count() < s.matches(close).count();
    while let Some(c) = s.chars().next_back() {
        let drop = matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"')
            || (c == ')' && unbalanced(s, '(', ')'))
            || (c == ']' && unbalanced(s, '[', ']'));
        if !drop {
            break;
        }
        s = &s[..s.len() - c.len_utf8()];
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ex(s: &str) -> Vec<String> {
        extract(s)
    }

    #[test]
    fn finds_links_in_text() {
        assert_eq!(
            ex("look https://example.com/a and http://example.com/b"),
            ["https://example.com/a", "http://example.com/b"]
        );
        assert_eq!(ex("https://example.com"), ["https://example.com/"]);
        assert!(ex("no links here").is_empty());
        // Glued to a word: not a link.
        assert!(ex("xhttps://example.com/a").is_empty());
    }

    #[test]
    fn skips_code_and_angle_brackets() {
        assert!(ex("`https://example.com/a`").is_empty());
        assert!(ex("```\nhttps://example.com/a\n```").is_empty());
        assert!(ex("```rust\nlet u = \"https://example.com/a\";\n```").is_empty());
        assert!(ex("<https://example.com/a>").is_empty());
        // Only the code is skipped.
        assert_eq!(
            ex("`https://example.com/a` https://example.com/b"),
            ["https://example.com/b"]
        );
        assert_eq!(
            ex("```\nhttps://example.com/a\n```\nhttps://example.com/b"),
            ["https://example.com/b"]
        );
        assert_eq!(
            ex("<https://example.com/a> https://example.com/b"),
            ["https://example.com/b"]
        );
        // An unclosed backtick is just a character.
        assert_eq!(ex("` https://example.com/a"), ["https://example.com/a"]);
        // `<` without its `>` right after the link doesn't suppress it.
        assert_eq!(ex("<https://example.com/a b>"), ["https://example.com/a"]);
    }

    #[test]
    fn at_most_three_distinct() {
        assert_eq!(
            ex(
                "https://example.com/1 https://example.com/2 https://example.com/3 https://example.com/4"
            ),
            [
                "https://example.com/1",
                "https://example.com/2",
                "https://example.com/3"
            ]
        );
        // The same link twice (here differing only by fragment and host case) counts once.
        assert_eq!(
            ex("https://example.com/1 https://EXAMPLE.com/1#top https://example.com/2"),
            ["https://example.com/1", "https://example.com/2"]
        );
    }

    #[test]
    fn trims_trailing_punctuation() {
        for s in [
            "see https://example.com/a.",
            "see https://example.com/a,",
            "see https://example.com/a!?",
            "\"https://example.com/a\"",
            "'https://example.com/a'",
            "https://example.com/a;:",
        ] {
            assert_eq!(ex(s), ["https://example.com/a"], "{s}");
        }
        // Inside the path it stays.
        assert_eq!(ex("https://example.com/a.b"), ["https://example.com/a.b"]);
    }

    #[test]
    fn parentheses_and_markdown_links() {
        assert_eq!(
            ex("(see https://example.com/a_(b))."),
            ["https://example.com/a_(b)"]
        );
        assert_eq!(ex("(https://example.com/a)"), ["https://example.com/a"]);
        assert_eq!(ex("[x](https://example.com/y)"), ["https://example.com/y"]);
        assert_eq!(
            ex("[x](https://example.com/y \"title\")"),
            ["https://example.com/y"]
        );
        assert_eq!(ex("[https://example.com/z]"), ["https://example.com/z"]);
        assert_eq!(
            ex("https://example.com/w?q=[1]"),
            ["https://example.com/w?q=[1]"]
        );
    }

    #[test]
    fn other_schemes_are_ignored() {
        assert!(ex("ftp://example.com/a file:///etc/hosts mailto:alex@example.com").is_empty());
        assert!(ex("javascript:alert(1)").is_empty());
        // A scheme with nothing after it.
        assert!(ex("https:// and http://").is_empty());
    }

    #[test]
    fn normalise_forms() {
        assert_eq!(
            normalise("HTTPS://Example.COM/a#frag").as_deref(),
            Some("https://example.com/a")
        );
        assert_eq!(
            normalise("http://example.com").as_deref(),
            Some("http://example.com/")
        );
        assert_eq!(normalise("ftp://example.com/"), None);
        assert_eq!(normalise("not a url"), None);
        assert_eq!(normalise("file:///etc/hosts"), None);
    }
}
