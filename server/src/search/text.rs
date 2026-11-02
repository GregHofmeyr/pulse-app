//! Search text: the words of a query, the FTS5 expression they become, the text a hit shows and
//! where the words are in it. No database here.

use pulse_protocol::rest::HighlightRange;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::messages::reactions::custom_token;

/// Most terms one query uses; past this the rest is ignored (each term is one more FTS lookup).
pub const MAX_TERMS: usize = 16;

/// One thing the query asks for: a word (matches any token it starts) or a quoted phrase
/// (these tokens, in this order, exactly).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Term {
    Word(String),
    Phrase(Vec<String>),
}

/// Lowercase, without accents: how the index's tokenizer (`unicode61 remove_diacritics 2`)
/// compares words, near enough to place highlights.
pub fn fold(s: &str) -> String {
    s.nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

/// A word of text (a run of letters and digits, as the tokenizer splits it), with where it sits
/// in UTF-16 code units.
struct Token {
    start: u32,
    end: u32,
    folded: String,
}

fn tokens(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut at = 0u32;
    let mut cur: Option<(u32, String)> = None;
    fn close(cur: &mut Option<(u32, String)>, end: u32, out: &mut Vec<Token>) {
        if let Some((start, word)) = cur.take() {
            let folded = fold(&word);
            // Only combining marks fold to nothing, and nothing can match that.
            if !folded.is_empty() {
                out.push(Token { start, end, folded });
            }
        }
    }
    for c in text.chars() {
        // A decomposed accent is part of its letter's word, as the tokenizer treats it.
        if c.is_alphanumeric() || is_combining_mark(c) {
            cur.get_or_insert_with(|| (at, String::new())).1.push(c);
        } else {
            close(&mut cur, at, &mut out);
        }
        at += c.len_utf16() as u32;
    }
    close(&mut cur, at, &mut out);
    out
}

/// The query's words and "quoted phrases", folded. A word with punctuation in it is several
/// words, as the tokenizer would split it. At most [`MAX_TERMS`].
pub fn terms(q: &str) -> Vec<Term> {
    let mut out = Vec::new();
    for (i, part) in q.split('"').enumerate() {
        let words: Vec<String> = tokens(part).into_iter().map(|t| t.folded).collect();
        // Odd parts were inside quotes (an unclosed quote runs to the end).
        if i % 2 == 1 {
            if !words.is_empty() {
                out.push(Term::Phrase(words));
            }
        } else {
            out.extend(words.into_iter().map(Term::Word));
        }
    }
    out.truncate(MAX_TERMS);
    out
}

/// The FTS5 expression: every term quoted (so nothing typed is FTS syntax), words as prefixes,
/// all required. `None` when the query has no words.
pub fn fts_query(terms: &[Term]) -> Option<String> {
    if terms.is_empty() {
        return None;
    }
    Some(
        terms
            .iter()
            .map(|t| match t {
                Term::Word(w) => format!("\"{w}\"*"),
                Term::Phrase(p) => format!("\"{}\"", p.join(" ")),
            })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// A message's text as a hit shows it: custom emoji tokens as `:name:` (their ids are not
/// words anyone typed), then each quote image caption on its own line.
pub fn display_text(content: &str, captions: &[String]) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        match custom_token(rest) {
            Some((name, _, len)) => {
                out.push(':');
                out.push_str(name);
                out.push(':');
                rest = &rest[len..];
            }
            None => {
                out.push('<');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    for c in captions {
        out.push('\n');
        out.push_str(c);
    }
    out
}

/// Whether a hit matched only through custom emoji ids: the index holds `<:name:ID>`'s id as a
/// word, so "01j" finds every message with a custom emoji. True when some term is missing from
/// what the hit shows (`shown`) and every missing one is a word that starts one of `content`'s
/// emoji ids. A term missing for another reason (this folding and the index's differ on a few
/// letters, such as ł and ø) is not a reason to drop a real match.
pub fn matched_only_by_emoji_ids(content: &str, shown: &str, terms: &[Term]) -> bool {
    let ids: Vec<String> = content
        .match_indices('<')
        .filter_map(|(i, _)| custom_token(&content[i..]).map(|(_, id, _)| fold(id)))
        .collect();
    let mut missing = terms
        .iter()
        .filter(|t| !highlights(shown, std::slice::from_ref(*t)).1)
        .peekable();
    missing.peek().is_some()
        && missing.all(|t| matches!(t, Term::Word(w) if ids.iter().any(|id| id.starts_with(w.as_str()))))
}

/// Where the terms are in `text`, merged and in order, and whether every term was found.
pub fn highlights(text: &str, terms: &[Term]) -> (Vec<HighlightRange>, bool) {
    let toks = tokens(text);
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    let mut all = true;
    for term in terms {
        let before = ranges.len();
        match term {
            Term::Word(w) => ranges.extend(
                toks.iter()
                    .filter(|t| t.folded.starts_with(w.as_str()))
                    .map(|t| (t.start, t.end)),
            ),
            Term::Phrase(p) => ranges.extend(
                toks.windows(p.len())
                    .filter(|win| win.iter().zip(p).all(|(t, w)| &t.folded == w))
                    .map(|win| (win[0].start, win[win.len() - 1].end)),
            ),
        }
        all &= ranges.len() > before;
    }
    ranges.sort_unstable();
    let mut merged: Vec<HighlightRange> = Vec::new();
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start <= last.end => last.end = last.end.max(end),
            _ => merged.push(HighlightRange { start, end }),
        }
    }
    (merged, all)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Term {
        Term::Word(s.into())
    }
    fn r(start: u32, end: u32) -> HighlightRange {
        HighlightRange { start, end }
    }

    #[test]
    fn terms_split_words_and_phrases() {
        assert_eq!(
            terms(r#"hi "big Clip" Café-run"#),
            vec![
                w("hi"),
                Term::Phrase(vec!["big".into(), "clip".into()]),
                w("cafe"),
                w("run"),
            ]
        );
        // An unclosed quote runs to the end; an empty one is nothing.
        assert_eq!(
            terms(r#"a "" "b c"#),
            vec![w("a"), Term::Phrase(vec!["b".into(), "c".into()])]
        );
        assert_eq!(terms(&"x ".repeat(40)).len(), MAX_TERMS);
    }

    /// Whatever is typed, the expression is only quoted tokens: FTS5 operators can't sneak in.
    #[test]
    fn fts_syntax_is_inert() {
        let q = fts_query(&terms(r#"a OR -b* NEAR( c:d (e) ^f"#)).unwrap();
        assert_eq!(q, r#""a"* "or"* "b"* "near"* "c"* "d"* "e"* "f"*"#);
        // A stray quote opens a phrase, which is just as inert.
        let q = fts_query(&terms(r#"a" OR -b*"#)).unwrap();
        assert_eq!(q, r#""a"* "or b""#);
        assert_eq!(
            fts_query(&terms(r#""big clip" x"#)).unwrap(),
            r#""big clip" "x"*"#
        );
    }

    #[test]
    fn empty_query_has_no_fts() {
        assert_eq!(fts_query(&terms("  -- ** \"\" ")), None);
    }

    #[test]
    fn display_reduces_custom_emoji() {
        assert_eq!(
            display_text(
                "gg <:party_time:01JABCDEF> <a:spin:01JXYZ> <b>",
                &["a goat".into()]
            ),
            "gg :party_time: :spin: <b>\na goat"
        );
    }

    #[test]
    fn highlights_prefix_and_accents() {
        let (got, all) = highlights("Clips at the café", &terms("clip cafe"));
        assert_eq!(got, vec![r(0, 5), r(13, 17)]);
        assert!(all);
    }

    #[test]
    fn highlights_phrase_exact() {
        let t = terms(r#""big clip""#);
        assert_eq!(highlights("a big clip here", &t), (vec![r(2, 10)], true));
        // A phrase isn't a prefix match, and its words must be next to each other.
        assert!(!highlights("a big clips", &t).1);
        assert!(!highlights("big red clip", &t).1);
    }

    #[test]
    fn highlights_utf16_offsets() {
        // 🔥 is two UTF-16 code units.
        assert_eq!(highlights("🔥 clip", &terms("clip")).0, vec![r(3, 7)]);
    }

    #[test]
    fn highlights_say_when_a_word_is_missing() {
        let (got, all) = highlights("pizza tonight", &terms("pizza 01j"));
        assert_eq!(got, vec![r(0, 5)]);
        assert!(!all);
        // A custom emoji's id is only in the stored text, not in what's shown.
        let shown = display_text("look <:party:01JABC>", &[]);
        assert!(!highlights(&shown, &terms("01j")).1);
    }

    /// A hit is dropped only when the words it lacks on screen are the start of an emoji id: a
    /// word this folding misses (ł, ø: the index folds them, NFD doesn't) is still a real match.
    #[test]
    fn drops_only_hits_matched_by_emoji_ids() {
        let content = "Łódź trip <:party:01JABC>";
        let shown = display_text(content, &[]);
        assert!(matched_only_by_emoji_ids(content, &shown, &terms("01j")));
        assert!(matched_only_by_emoji_ids(content, &shown, &terms("trip 01jab")));
        assert!(!matched_only_by_emoji_ids(content, &shown, &terms("lodz")));
        assert!(!matched_only_by_emoji_ids(content, &shown, &terms("lodz 01j")));
        assert!(!matched_only_by_emoji_ids(content, &shown, &terms("trip")));
        // A phrase can't be an id's start.
        assert!(!matched_only_by_emoji_ids(content, &shown, &terms("\"01jabc\"")));
    }
}
