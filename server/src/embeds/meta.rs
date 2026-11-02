//! Reading a preview's text out of what a site sent: a page's OpenGraph / Twitter-card tags and
//! `<title>`, and an oEmbed answer's fields. Pure; no network. Nothing here interprets markup:
//! values come out as plain text (entities decoded), to be [`clean`]ed and shown as text.

/// Most chars of a preview's title.
pub const TITLE_MAX: usize = 200;
/// Most chars of a preview's description.
pub const DESCRIPTION_MAX: usize = 300;
/// Most chars of a site name.
pub const SITE_NAME_MAX: usize = 80;
/// Most chars of an author's name.
pub const AUTHOR_MAX: usize = 80;

/// What a page's head says about it, decoded but not yet cleaned. `image` is as written (maybe
/// relative: the caller resolves it against the page's final URL).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageMeta {
    pub title: Option<String>,
    pub description: Option<String>,
    pub site_name: Option<String>,
    pub image: Option<String>,
}

/// The preview fields of `html`'s head: `og:*` first, then `twitter:*`, then `<title>` and
/// `<meta name="description">`. The first of each tag counts. Reading stops at `</head>` or
/// `<body>`; `<script>`, `<style>` and comments are skipped whole.
pub fn page_meta(html: &str) -> PageMeta {
    let mut tags = Tags::default();
    let lower = html.to_ascii_lowercase();
    let b = lower.as_bytes();
    let mut i = 0;
    while let Some(k) = lower[i..].find('<') {
        i += k + 1;
        if lower[i..].starts_with("!--") {
            i = lower[i..].find("-->").map_or(b.len(), |e| i + e + 3);
            continue;
        }
        let name_len = b[i..]
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric() || **c == b'/' || **c == b'!')
            .count();
        let name = &lower[i..i + name_len];
        // ASCII lowercasing keeps every byte where it was, so offsets into `lower` are offsets
        // into `html`.
        let (attrs, end) = attributes(html, i + name_len);
        i = end;
        match name {
            "/head" | "body" => break,
            "script" | "style" | "noscript" | "template" => {
                let close = format!("</{name}");
                i = lower[i..].find(&close).map_or(b.len(), |e| i + e);
            }
            "title" => {
                let stop = lower[i..].find("</title").map_or(b.len(), |e| i + e);
                tags.first("title", decode(&html[i..stop]));
                i = stop;
            }
            "meta" => {
                let key = attrs
                    .iter()
                    .find(|(k, _)| k == "property" || k == "name")
                    .map(|(_, v)| v.trim().to_ascii_lowercase());
                let content = attrs.iter().find(|(k, _)| k == "content").map(|(_, v)| v);
                if let (Some(key), Some(content)) = (key, content) {
                    tags.first(&key, content.clone());
                }
            }
            _ => {}
        }
    }
    let pick = |keys: &[&str]| keys.iter().find_map(|k| tags.get(k));
    PageMeta {
        title: pick(&["og:title", "twitter:title", "title"]),
        description: pick(&["og:description", "twitter:description", "description"]),
        site_name: pick(&["og:site_name"]),
        image: pick(&[
            "og:image",
            "og:image:url",
            "og:image:secure_url",
            "twitter:image",
            "twitter:image:src",
        ]),
    }
}

/// The tags seen so far, first one of each name kept.
#[derive(Default)]
struct Tags(Vec<(String, String)>);

impl Tags {
    fn first(&mut self, key: &str, value: String) {
        if !self.0.iter().any(|(k, _)| k == key) {
            self.0.push((key.to_owned(), value));
        }
    }
    /// A tag's value, if it has any text.
    fn get(&self, key: &str) -> Option<String> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    }
}

/// The attributes of the tag whose name ends at `at` (lowercased names, decoded values), and
/// where the tag ends (just past its `>`).
fn attributes(html: &str, mut at: usize) -> (Vec<(String, String)>, usize) {
    let b = html.as_bytes();
    let mut out = Vec::new();
    loop {
        while at < b.len() && (b[at].is_ascii_whitespace() || b[at] == b'/') {
            at += 1;
        }
        if at >= b.len() {
            return (out, b.len());
        }
        if b[at] == b'>' {
            return (out, at + 1);
        }
        let start = at;
        while at < b.len() && !b[at].is_ascii_whitespace() && !matches!(b[at], b'=' | b'>' | b'/') {
            at += 1;
        }
        let name = html[start..at].to_ascii_lowercase();
        while at < b.len() && b[at].is_ascii_whitespace() {
            at += 1;
        }
        if at >= b.len() || b[at] != b'=' {
            out.push((name, String::new()));
            continue;
        }
        at += 1;
        while at < b.len() && b[at].is_ascii_whitespace() {
            at += 1;
        }
        let value = match b.get(at) {
            Some(&q @ (b'"' | b'\'')) => {
                let vstart = at + 1;
                let vend = html[vstart..]
                    .find(q as char)
                    .map_or(b.len(), |e| vstart + e);
                at = (vend + 1).min(b.len());
                &html[vstart..vend]
            }
            _ => {
                let vstart = at;
                while at < b.len() && !b[at].is_ascii_whitespace() && b[at] != b'>' {
                    at += 1;
                }
                &html[vstart..at]
            }
        };
        out.push((name, decode(value)));
    }
}

/// `s` with HTML's character references replaced: the named ones previews use and numeric ones.
/// Anything else is left as written; a numeric reference to no character becomes U+FFFD.
pub fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        // Bytes, not a slice of the str: the window may end inside a multi-byte char.
        let window = &rest.as_bytes()[..rest.len().min(12)];
        let Some(semi) = window.iter().position(|&c| c == b';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..semi];
        let c = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => name.strip_prefix('#').map(|n| {
                let code = match n.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => n.parse::<u32>().ok(),
                };
                code.filter(|&c| c != 0)
                    .and_then(char::from_u32)
                    .unwrap_or('\u{fffd}')
            }),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `s` fit to show as one line of a preview: control characters and text-direction overrides
/// dropped, whitespace runs made one space, trimmed, and cut to `max_chars` with `…`. `None`
/// when nothing is left.
pub fn clean(s: &str, max_chars: usize) -> Option<String> {
    let mut out = String::with_capacity(s.len().min(max_chars * 4));
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(c);
    }
    if out.is_empty() || max_chars == 0 {
        return None;
    }
    if out.chars().count() > max_chars {
        let mut cut: String = out.chars().take(max_chars - 1).collect();
        cut.truncate(cut.trim_end().len());
        cut.push('…');
        out = cut;
    }
    Some(out)
}

/// The fields of an oEmbed answer a preview uses. Anything else in it (its `html` above all) is
/// ignored; a field that isn't a string counts as absent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OEmbed {
    pub title: Option<String>,
    pub description: Option<String>,
    pub author_name: Option<String>,
    pub provider_name: Option<String>,
    pub thumbnail_url: Option<String>,
}

impl OEmbed {
    /// `None` unless `body` is a JSON object.
    pub fn parse(body: &[u8]) -> Option<OEmbed> {
        let v: serde_json::Value = serde_json::from_slice(body).ok()?;
        let obj = v.as_object()?;
        let s = |k: &str| {
            obj.get(k)
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .filter(|v| !v.trim().is_empty())
        };
        Some(OEmbed {
            title: s("title"),
            description: s("description"),
            author_name: s("author_name"),
            provider_name: s("provider_name"),
            thumbnail_url: s("thumbnail_url"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn og_wins_over_twitter_over_title() {
        let m = page_meta(
            r#"<!doctype html><html><head>
            <title>Plain title</title>
            <meta name="description" content="Plain description">
            <meta name="twitter:title" content="Twitter title">
            <meta name="twitter:image" content="/tw.png">
            <meta property="og:title" content="OG title">
            <meta property="og:description" content="OG description">
            <meta property="og:site_name" content="Example">
            <meta property="og:image" content="https://example.com/og.png">
            </head><body></body></html>"#,
        );
        assert_eq!(
            m,
            PageMeta {
                title: Some("OG title".into()),
                description: Some("OG description".into()),
                site_name: Some("Example".into()),
                image: Some("https://example.com/og.png".into()),
            }
        );
        let m = page_meta(
            r#"<head><TITLE>Plain</TITLE><META NAME="Description" CONTENT="Desc">
               <meta name="twitter:title" content="Tw"><meta name="twitter:image" content="/tw.png">"#,
        );
        assert_eq!(m.title.as_deref(), Some("Tw"));
        assert_eq!(m.description.as_deref(), Some("Desc"));
        assert_eq!(m.image.as_deref(), Some("/tw.png"));
        assert_eq!(m.site_name, None);
        let m = page_meta("<head><title>  Just a title </title></head>");
        assert_eq!(m.title.as_deref(), Some("Just a title"));
        // The first of a tag counts; an empty one is no value.
        let m = page_meta(
            r#"<meta property="og:title" content=""><meta property="og:title" content="Second">
               <title>T</title>"#,
        );
        assert_eq!(m.title.as_deref(), Some("T"));
    }

    #[test]
    fn attribute_forms() {
        let m = page_meta(
            "<meta content='Single' property='og:title'/><meta property=og:site_name content=Bare>",
        );
        assert_eq!(m.title.as_deref(), Some("Single"));
        assert_eq!(m.site_name.as_deref(), Some("Bare"));
    }

    #[test]
    fn entities_are_decoded() {
        assert_eq!(
            decode(
                "Tom &amp; Jerry &lt;3 &quot;hi&quot; &#39;x&apos; a&nbsp;b &#233; &#xE9; &#X1F600;"
            ),
            "Tom & Jerry <3 \"hi\" 'x' a\u{a0}b é é 😀"
        );
        // Unknown names and stray ampersands stay as written; a reference to no char is U+FFFD.
        assert_eq!(
            decode("a & b &copy; &#0; &#xD800; &"),
            "a & b &copy; \u{fffd} \u{fffd} &"
        );
        let m = page_meta(
            r#"<title>A &amp; B</title><meta property="og:description" content="x &gt; y">"#,
        );
        assert_eq!(m.title.as_deref(), Some("A & B"));
        assert_eq!(m.description.as_deref(), Some("x > y"));
    }

    #[test]
    fn markup_in_values_is_plain_text() {
        let m = page_meta(
            r#"<head><meta property="og:title" content="&lt;script&gt;alert(1)&lt;/script&gt;">
               <meta property="og:description" content='a" onerror="alert(1)'>
               <meta property="og:site_name" content="<img src=x onerror=alert(1)>"></head>"#,
        );
        assert_eq!(m.title.as_deref(), Some("<script>alert(1)</script>"));
        assert_eq!(m.description.as_deref(), Some("a\" onerror=\"alert(1)"));
        assert_eq!(m.site_name.as_deref(), Some("<img src=x onerror=alert(1)>"));
        // Markup inside `<title>` is its text, not tags.
        let m = page_meta("<title><b>Bold</b> &amp; <script>x</script></title>");
        assert_eq!(m.title.as_deref(), Some("<b>Bold</b> & <script>x</script>"));
    }

    #[test]
    fn scripts_and_comments_are_skipped() {
        let m = page_meta(
            r#"<head><script>var s = '<meta property="og:title" content="Script">';</script>
               <!-- <meta property="og:title" content="Comment"> -->
               <style>/* <title>Style</title> */</style>
               <meta property="og:title" content="Real"></head>"#,
        );
        assert_eq!(m.title.as_deref(), Some("Real"));
    }

    #[test]
    fn stops_at_the_end_of_the_head() {
        let m = page_meta(
            r#"<head><title>Head</title></head><body><meta property="og:title" content="Body">
               <meta property="og:image" content="/late.png"></body>"#,
        );
        assert_eq!(m.title.as_deref(), Some("Head"));
        assert_eq!(m.image, None);
        // No `</head>`: the body still ends it.
        let m = page_meta(r#"<title>T</title><body><meta property="og:image" content="/x.png">"#);
        assert_eq!(m.image, None);
    }

    #[test]
    fn truncated_html_does_not_panic() {
        for s in [
            "<",
            "<meta",
            "<meta property=",
            "<meta property=\"og:title",
            "<title>unterminated",
            "<!-- never closed",
            "<script>never closed",
            "&#x",
            "é<é",
        ] {
            let _ = page_meta(s);
        }
        assert_eq!(
            page_meta("<title>unterminated").title.as_deref(),
            Some("unterminated")
        );
    }

    #[test]
    fn clean_makes_one_short_line() {
        assert_eq!(clean("  a \n\t b  ", 10).as_deref(), Some("a b"));
        assert_eq!(clean("a\u{0}b\u{7}c\u{202e}d", 10).as_deref(), Some("abcd"));
        assert_eq!(clean(" \n ", 10), None);
        assert_eq!(clean("", 10), None);
        assert_eq!(clean("abcdefghij", 10).as_deref(), Some("abcdefghij"));
        assert_eq!(clean("abcdefghijk", 10).as_deref(), Some("abcdefghi…"));
        // Counted in chars, and no space left before the ellipsis.
        assert_eq!(clean("ééééé ééé", 7).as_deref(), Some("ééééé…"));
    }

    #[test]
    fn oembed_takes_only_its_text_fields() {
        let o = OEmbed::parse(
            br#"{"title":"T","author_name":"alex","provider_name":"YouTube",
                 "thumbnail_url":"https://example.com/t.jpg","html":"<iframe></iframe>",
                 "description":5}"#,
        )
        .unwrap();
        assert_eq!(
            o,
            OEmbed {
                title: Some("T".into()),
                description: None,
                author_name: Some("alex".into()),
                provider_name: Some("YouTube".into()),
                thumbnail_url: Some("https://example.com/t.jpg".into()),
            }
        );
        assert_eq!(OEmbed::parse(b"[1]"), None);
        assert_eq!(OEmbed::parse(b"<html>"), None);
    }
}
