//! The rules every name you can choose follows: display names, server nicknames and private
//! nicknames alike (#47, #48).

use unicode_properties::{GeneralCategory, GeneralCategoryGroup, UnicodeGeneralCategory};

use crate::error::{AppError, AppResult};

/// Longest name, in characters, after trimming.
pub const NAME_MAX: usize = 32;

/// What a piece of chosen text broke: the rules [`clean`] and [`clean_typing_text`] share.
enum Broken {
    Length,
    Control,
    Invisible,
}

/// The name rules on trimmed, non-empty text: at most 32 characters, no line breaks, control or
/// direction characters, and something that draws.
fn check(s: &str) -> Result<(), Broken> {
    if s.chars().count() > NAME_MAX {
        return Err(Broken::Length);
    }
    if s.chars().any(|c| c.is_control() || breaks_layout(c)) {
        return Err(Broken::Control);
    }
    if s.chars().all(|c| c.is_whitespace() || invisible(c)) {
        return Err(Broken::Invisible);
    }
    Ok(())
}

/// A name, trimmed: 1–32 characters, no line breaks or control characters. Not unique.
pub fn clean(s: &str) -> AppResult<String> {
    let s = s.trim();
    let msg = match if s.is_empty() {
        Err(Broken::Length)
    } else {
        check(s)
    } {
        Ok(()) => return Ok(s.to_owned()),
        Err(Broken::Length) => "Names can be 1 to 32 characters.",
        Err(Broken::Control) => "Names can't contain line breaks or control characters.",
        Err(Broken::Invisible) => "Names need at least one character you can see.",
    };
    Err(AppError::BadRequest(msg.into()))
}

/// What shows after "<name> is" while someone types (#652), as the name rules have it (trimmed,
/// at most 32 characters, no line breaks, control or direction characters), and stricter: no
/// format characters (Unicode Cf) or other invisible ones at all, except a joiner between two
/// drawn characters (inside an emoji) and the tags of a subdivision flag (right after 🏴); and no
/// links. Blank is `None`: the plain "is typing". The client's `typingTextProblem`
/// (client/ui/src/lib/typingLine.ts) is the same rule.
pub fn clean_typing_text(s: &str) -> AppResult<Option<String>> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let bad = |m: &str| Err(AppError::BadRequest(m.into()));
    match check(s) {
        Ok(()) => {}
        Err(Broken::Length) => return bad("Typing text can be up to 32 characters."),
        Err(Broken::Control) => {
            return bad("Typing text can't contain line breaks or control characters.");
        }
        Err(Broken::Invisible) => return bad("Typing text can't contain invisible characters."),
    }
    if has_hidden(s) {
        return bad("Typing text can't contain invisible characters.");
    }
    if has_link(s) {
        return bad("Typing text can't contain links.");
    }
    Ok(Some(s.to_owned()))
}

/// Whether text holds a character that draws nothing or hides what's around it: any format
/// character (Unicode Cf) or [`invisible`] one, but a joiner between two drawn characters (an
/// emoji's parts) and a run of tag characters straight after 🏴 (a subdivision flag) are allowed.
fn has_hidden(s: &str) -> bool {
    let c: Vec<char> = s.chars().collect();
    let drawn = |x: Option<&char>| x.is_some_and(|x| !x.is_whitespace() && !hides(*x));
    let mut flag = false; // inside 🏴's tag run
    c.iter().enumerate().any(|(i, &x)| {
        let tag = matches!(x, '\u{e0020}'..='\u{e007f}');
        flag = tag && (flag || (i > 0 && c[i - 1] == '\u{1f3f4}'));
        hides(x)
            && !flag
            && !(x == '\u{200d}' && i > 0 && drawn(c.get(i - 1)) && drawn(c.get(i + 1)))
    })
}

/// A format character (Cf: zero-width ones, direction marks, the soft hyphen, tags, interlinear
/// annotation marks) or an [`invisible`] one.
fn hides(c: char) -> bool {
    invisible(c) || c.general_category() == GeneralCategory::Format
}

/// The characters that read as a dot in a domain: `.` and its lookalikes (ideographic, fullwidth
/// and halfwidth full stops, the one-dot leader, the hyphenation point, the small full stop).
fn dot(c: char) -> bool {
    matches!(
        c,
        '.' | '\u{3002}' | '\u{ff0e}' | '\u{ff61}' | '\u{2024}' | '\u{2027}' | '\u{fe52}'
    )
}

/// Whether text carries something that reads as a link: a scheme (`://`), `www.`, or a domain
/// (a letter or digit, a dot or lookalike, then two letters of any script, as in `pulse.app` or
/// `пример.рф`). `v1.2` and `...` aren't; `brain.exe` is, and that's fine. Letters and digits are
/// Unicode categories L and N, as the client's `\p{L}` and `\p{N}`.
fn has_link(s: &str) -> bool {
    let c: Vec<char> = s
        .chars()
        .map(|x| if dot(x) { '.' } else { x.to_ascii_lowercase() })
        .collect();
    let text: String = c.iter().collect();
    if text.contains("://") || text.contains("www.") {
        return true;
    }
    let letter = |x: char| x.general_category_group() == GeneralCategoryGroup::Letter;
    let number = |x: char| x.general_category_group() == GeneralCategoryGroup::Number;
    (1..c.len().saturating_sub(2)).any(|i| {
        c[i] == '.'
            && (letter(c[i - 1]) || number(c[i - 1]))
            && letter(c[i + 1])
            && letter(c[i + 2])
    })
}

/// Unicode line and paragraph separators, and the direction embeddings, overrides and isolates
/// that can turn the text after a name around (in a system line, say).
fn breaks_layout(c: char) -> bool {
    matches!(c, '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Characters that draw nothing: zero-width spaces, joiners and marks, word joiners, the BOM, the
/// Hangul fillers, the soft hyphen, the braille blank and the Mongolian variation selectors.
/// Allowed inside a name (emoji need joiners), never as all of it.
fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{ad}'
            | '\u{034f}'
            | '\u{115f}'
            | '\u{1160}'
            | '\u{180b}'..='\u{180f}'
            | '\u{200b}'..='\u{200f}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2800}'
            | '\u{3164}'
            | '\u{feff}'
            | '\u{ffa0}'
    )
}

/// Combining marks stacked on one character that make a name Zalgo (#209, #210).
pub const ZALGO_MARKS: usize = 3;

/// Whether `name` is Zalgo: some character carries [`ZALGO_MARKS`] or more combining marks.
/// Variation selectors (an emoji's text or picture form) aren't counted. The client's `isZalgo`
/// (client/ui/src/lib/profileDraft.ts) is the same rule, for the "Zalgo? Really?" hint while
/// typing; this one, on save, earns the hidden badge.
pub fn is_zalgo(name: &str) -> bool {
    let mut run = 0;
    for c in name.chars() {
        if matches!(c, '\u{fe00}'..='\u{fe0f}' | '\u{e0100}'..='\u{e01ef}') {
            continue;
        }
        run = if unicode_normalization::char::is_combining_mark(c) {
            run + 1
        } else {
            0
        };
        if run >= ZALGO_MARKS {
            return true;
        }
    }
    false
}

/// A nickname to set: blank (or none) is a reset, anything else must be a [`clean`] name.
pub fn clean_nickname(s: Option<&str>) -> AppResult<Option<String>> {
    match s.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => clean(s).map(Some),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_may_have_capitals_spaces_and_emoji() {
        assert_eq!(clean("  Alex 🎸 the Great ").unwrap(), "Alex 🎸 the Great");
        assert_eq!(clean("Ünïcødé ñame").unwrap(), "Ünïcødé ñame");
        // Emoji joined into one picture are still allowed (they count as several characters).
        assert!(clean("👩\u{200d}👩\u{200d}👧").is_ok());
    }

    #[test]
    fn names_are_1_to_32_characters() {
        assert!(clean("").is_err());
        assert!(clean("    ").is_err());
        assert!(clean(&"x".repeat(32)).is_ok());
        assert!(clean(&"x".repeat(33)).is_err());
        // characters, not bytes
        assert!(clean(&"é".repeat(32)).is_ok());
    }

    #[test]
    fn names_have_no_line_breaks_or_control_characters() {
        for bad in ["a\nb", "a\rb", "a\tb", "a\u{7}b", "a\u{0}b", "a\u{85}b"] {
            assert!(clean(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn unicode_line_breaks_and_direction_overrides_are_refused() {
        for bad in [
            "a\u{2028}b",
            "a\u{2029}b",
            "a\u{202e}b",
            "a\u{2066}b",
            "\u{202d}x",
        ] {
            assert!(clean(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_name_needs_something_you_can_see() {
        for bad in [
            "\u{200b}",
            "\u{200b}\u{feff}",
            " \u{2060} ",
            "\u{3164}",
            "\u{200d}",
        ] {
            assert!(clean(bad).is_err(), "{bad:?}");
        }
        // joiners inside a visible name are fine
        assert!(clean("a\u{200d}b").is_ok());
    }

    #[test]
    fn a_blank_nickname_is_a_reset() {
        assert_eq!(clean_nickname(None).unwrap(), None);
        assert_eq!(clean_nickname(Some("  ")).unwrap(), None);
        assert_eq!(
            clean_nickname(Some(" Kim ")).unwrap().as_deref(),
            Some("Kim")
        );
        assert!(clean_nickname(Some(&"k".repeat(33))).is_err());
    }

    /// A literal copy of the cases in client/ui/src/lib/profileDraft.test.ts (`isZalgo`): change
    /// one side and both tests fail.
    const ZALGO_CASES: &[(&str, bool)] = &[
        ("alex", false),
        ("\u{dc}n\u{ef}c\u{f8}d\u{e9} \u{f1}ame", false),
        ("e\u{301}\u{302}", false),
        ("Vie\u{323}\u{302}t", false),
        ("a\u{301} b\u{302}\u{303}", false),
        ("\u{2764}\u{fe0f}\u{fe0f}\u{fe0f}", false),
        ("1\u{fe0f}\u{20e3}", false),
        ("Z\u{335}\u{352}\u{316}", true),
        ("Z\u{335}\u{fe0f}\u{352}\u{316}", true),
        ("kim\u{300}\u{301}\u{302}\u{303}\u{304}\u{305}", true),
        (" \u{300}\u{301}\u{302}", true),
        ("\u{300}\u{301}\u{302}", true),
    ];

    #[test]
    fn zalgo_is_three_marks_on_one_character() {
        assert_eq!(ZALGO_MARKS, 3);
        for (name, want) in ZALGO_CASES {
            assert_eq!(is_zalgo(name), *want, "{name:?}");
        }
    }

    // ---- typing text (#652) ----

    #[test]
    fn blank_typing_text_is_the_default() {
        assert_eq!(clean_typing_text("").unwrap(), None);
        assert_eq!(clean_typing_text("   ").unwrap(), None);
    }

    #[test]
    fn typing_text_is_trimmed_and_up_to_32_characters() {
        assert_eq!(
            clean_typing_text("  cooking  ").unwrap().as_deref(),
            Some("cooking")
        );
        assert_eq!(
            clean_typing_text("typing a novel 🍳").unwrap().as_deref(),
            Some("typing a novel 🍳")
        );
        assert!(clean_typing_text(&"x".repeat(32)).is_ok());
        assert_eq!(
            clean_typing_text(&"x".repeat(33)).unwrap_err().to_string(),
            "Typing text can be up to 32 characters."
        );
        // characters, not bytes
        assert!(clean_typing_text(&"é".repeat(32)).is_ok());
    }

    #[test]
    fn typing_text_has_no_line_breaks_controls_or_direction_tricks() {
        for bad in [
            "a\nb",
            "a\rb",
            "a\tb",
            "a\u{0}b",
            "a\u{2028}b",
            "a\u{202e}b",
            "\u{2066}yapping",
        ] {
            assert_eq!(
                clean_typing_text(bad).unwrap_err().to_string(),
                "Typing text can't contain line breaks or control characters.",
                "{bad:?}"
            );
        }
    }

    #[test]
    fn typing_text_has_no_zero_width_tricks() {
        for bad in [
            "\u{200b}",
            "co\u{200b}oking",
            "yap\u{feff}",
            "\u{2060}plotting",
            "\u{200d}barking",
            "barking\u{200d}",
            "a \u{200d} b",
        ] {
            assert!(clean_typing_text(bad).is_err(), "{bad:?}");
        }
        // a joiner inside an emoji is part of the picture
        assert!(clean_typing_text("cooking 👩\u{200d}🍳").is_ok());
    }

    #[test]
    fn typing_text_has_no_links() {
        for bad in [
            "https://example.com",
            "at http://x",
            "www.example",
            "on pulse.app",
            "join discord.gg/abc",
            "EXAMPLE.COM",
        ] {
            assert_eq!(
                clean_typing_text(bad).unwrap_err().to_string(),
                "Typing text can't contain links.",
                "{bad:?}"
            );
        }
        // dots that aren't a domain are fine
        for ok in ["typing a novel...", "on v1.2", "cooking. again", "3.14"] {
            assert!(clean_typing_text(ok).is_ok(), "{ok:?}");
        }
    }

    /// The typing-text rules as both sides keep them (#652 review). A literal copy is in
    /// client/ui/src/lib/typingLine.test.ts (`TYPING_TEXT_CASES`): change one side and both tests
    /// fail, so the server never refuses what the client let through. `None`: accepted.
    const TYPING_TEXT_CASES: &[(&str, Option<&str>)] = &[
        ("cooking", None),
        ("cooking 👩\u{200d}🍳", None),
        (
            "from 🏴\u{e0067}\u{e0062}\u{e0073}\u{e0063}\u{e0074}\u{e007f}",
            None,
        ),
        ("hearts ❤\u{fe0f}", None),
        ("typing a novel...", None),
        ("on v1.2", None),
        ("3.14", None),
        ("cooking. again", None),
        ("\u{200b}", Some(INVISIBLE_MSG)),
        ("yap\u{200b}ping", Some(INVISIBLE_MSG)),
        ("\u{200d}barking", Some(INVISIBLE_MSG)),
        ("a \u{200d} b", Some(INVISIBLE_MSG)),
        ("yap\u{61c}ping", Some(INVISIBLE_MSG)),
        ("yap\u{e0020}ping", Some(INVISIBLE_MSG)),
        ("yap\u{e0041}ping", Some(INVISIBLE_MSG)),
        ("yap\u{e007f}ping", Some(INVISIBLE_MSG)),
        ("yap\u{ad}ping", Some(INVISIBLE_MSG)),
        ("yap\u{2800}ping", Some(INVISIBLE_MSG)),
        ("\u{2800}", Some(INVISIBLE_MSG)),
        ("yap\u{180b}ping", Some(INVISIBLE_MSG)),
        ("yap\u{180c}ping", Some(INVISIBLE_MSG)),
        ("yap\u{180d}ping", Some(INVISIBLE_MSG)),
        ("yap\u{180f}ping", Some(INVISIBLE_MSG)),
        ("yap\u{fff9}ping", Some(INVISIBLE_MSG)),
        ("yap\u{fffa}ping", Some(INVISIBLE_MSG)),
        ("yap\u{fffb}ping", Some(INVISIBLE_MSG)),
        ("a\nb", Some(CONTROL_MSG)),
        ("\u{202e}gnippay", Some(CONTROL_MSG)),
        ("https://example.com", Some(LINK_MSG)),
        ("www.example", Some(LINK_MSG)),
        ("WWW.EXAMPLE", Some(LINK_MSG)),
        ("on pulse.app", Some(LINK_MSG)),
        ("pulse\u{3002}app", Some(LINK_MSG)),
        ("pulse\u{ff0e}app", Some(LINK_MSG)),
        ("pulse\u{ff61}app", Some(LINK_MSG)),
        ("pulse\u{2024}app", Some(LINK_MSG)),
        ("pulse\u{2027}app", Some(LINK_MSG)),
        ("pulse\u{fe52}app", Some(LINK_MSG)),
        ("пример.рф", Some(LINK_MSG)),
        ("brain.exe", Some(LINK_MSG)),
    ];
    const INVISIBLE_MSG: &str = "Typing text can't contain invisible characters.";
    const CONTROL_MSG: &str = "Typing text can't contain line breaks or control characters.";
    const LINK_MSG: &str = "Typing text can't contain links.";

    #[test]
    fn typing_text_cases_match_the_client() {
        for (text, want) in TYPING_TEXT_CASES {
            let got = clean_typing_text(text).err().map(|e| e.to_string());
            assert_eq!(got.as_deref(), *want, "{text:?}");
        }
    }

    #[test]
    fn the_new_invisible_characters_are_never_all_of_a_name() {
        for bad in ["\u{2800}", "\u{ad}", "\u{180b}\u{180f}"] {
            assert!(clean(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn zalgo_names_are_still_allowed() {
        assert!(clean("Z\u{335}\u{352}\u{316}a\u{334}\u{30d}\u{323}").is_ok());
    }
}
