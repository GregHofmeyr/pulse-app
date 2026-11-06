//! The rules every name you can choose follows: display names, server nicknames and private
//! nicknames alike (#47, #48).

use crate::error::{AppError, AppResult};

/// Longest name, in characters, after trimming.
pub const NAME_MAX: usize = 32;

/// A name, trimmed: 1–32 characters, no line breaks or control characters. Not unique.
pub fn clean(s: &str) -> AppResult<String> {
    let s = s.trim();
    if s.is_empty() || s.chars().count() > NAME_MAX {
        return Err(AppError::BadRequest(
            "Names can be 1 to 32 characters.".into(),
        ));
    }
    if s.chars().any(|c| c.is_control() || breaks_layout(c)) {
        return Err(AppError::BadRequest(
            "Names can't contain line breaks or control characters.".into(),
        ));
    }
    if s.chars().all(|c| c.is_whitespace() || invisible(c)) {
        return Err(AppError::BadRequest(
            "Names need at least one character you can see.".into(),
        ));
    }
    Ok(s.to_owned())
}

/// Unicode line and paragraph separators, and the direction embeddings, overrides and isolates
/// that can turn the text after a name around (in a system line, say).
fn breaks_layout(c: char) -> bool {
    matches!(c, '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Characters that draw nothing: zero-width spaces, joiners and marks, word joiners, the BOM and
/// the Hangul fillers. Allowed inside a name (emoji need joiners), never as all of it.
fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{034f}'
            | '\u{115f}'
            | '\u{1160}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{2060}'..='\u{2064}'
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

    #[test]
    fn zalgo_names_are_still_allowed() {
        assert!(clean("Z\u{335}\u{352}\u{316}a\u{334}\u{30d}\u{323}").is_ok());
    }
}
