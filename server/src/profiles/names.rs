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
    if s.chars().any(char::is_control) {
        return Err(AppError::BadRequest(
            "Names can't contain line breaks or control characters.".into(),
        ));
    }
    Ok(s.to_owned())
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
        assert!(clean("👩‍👩‍👧").is_ok());
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
    fn a_blank_nickname_is_a_reset() {
        assert_eq!(clean_nickname(None).unwrap(), None);
        assert_eq!(clean_nickname(Some("  ")).unwrap(), None);
        assert_eq!(
            clean_nickname(Some(" Kim ")).unwrap().as_deref(),
            Some("Kim")
        );
        assert!(clean_nickname(Some(&"k".repeat(33))).is_err());
    }
}
