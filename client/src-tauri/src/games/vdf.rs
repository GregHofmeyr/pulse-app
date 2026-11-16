//! Steam's text KeyValues ("VDF"), just enough to read `steamapps/libraryfolders.vdf` (every Steam library, on every
//! drive) and `appmanifest_<appid>.acf` (a game's name) (#380). Pure: text in, values out.
//!
//! The format: `"key" "value"` pairs and `"key" { … }` blocks, quoted strings with `\\`, `\"`, `\n` and `\t`
//! escapes, `//` line comments, and the odd unquoted token or `[$WIN32]` condition (skipped). Keys compare without
//! case, as Steam's do.

use std::path::PathBuf;

/// A value: a string, or a block of keyed values in file order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kv {
    Str(String),
    Obj(Vec<(String, Kv)>),
}

impl Kv {
    /// The first value under `key` (any case), if this is a block.
    pub fn get(&self, key: &str) -> Option<&Kv> {
        match self {
            Kv::Obj(items) => items
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Kv::Str(_) => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Kv::Str(s) => Some(s),
            Kv::Obj(_) => None,
        }
    }
}

#[derive(Debug, PartialEq)]
enum Tok {
    Str(String),
    Open,
    Close,
}

fn tokens(text: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' => out.push(Tok::Open),
            '}' => out.push(Tok::Close),
            '"' => {
                let mut s = String::new();
                while let Some(c) = it.next() {
                    match c {
                        '"' => break,
                        '\\' => match it.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(other) => s.push(other),
                            None => break,
                        },
                        c => s.push(c),
                    }
                }
                out.push(Tok::Str(s));
            }
            '/' if it.peek() == Some(&'/') => {
                for c in it.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            // a platform condition after a value: not ours to judge, skipped
            '[' => {
                for c in it.by_ref() {
                    if c == ']' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {}
            c => {
                let mut s = String::from(c);
                while let Some(&n) = it.peek() {
                    if n.is_whitespace() || matches!(n, '{' | '}' | '"') {
                        break;
                    }
                    s.push(n);
                    it.next();
                }
                out.push(Tok::Str(s));
            }
        }
    }
    out
}

/// A whole file as one block of its top-level keys. Forgiving: a cut-off file gives what was read.
pub fn parse(text: &str) -> Kv {
    let toks = tokens(text.trim_start_matches('\u{feff}'));
    let mut i = 0;
    Kv::Obj(block(&toks, &mut i))
}

fn block(toks: &[Tok], i: &mut usize) -> Vec<(String, Kv)> {
    let mut items = Vec::new();
    while *i < toks.len() {
        match &toks[*i] {
            Tok::Close => {
                *i += 1;
                break;
            }
            Tok::Open => {
                // a block with no key: read and dropped
                *i += 1;
                block(toks, i);
            }
            Tok::Str(key) => {
                *i += 1;
                match toks.get(*i) {
                    Some(Tok::Str(v)) => {
                        items.push((key.clone(), Kv::Str(v.clone())));
                        *i += 1;
                    }
                    Some(Tok::Open) => {
                        *i += 1;
                        items.push((key.clone(), Kv::Obj(block(toks, i))));
                    }
                    Some(Tok::Close) | None => {}
                }
            }
        }
    }
    items
}

/// Every library folder `libraryfolders.vdf` lists: today's form (`"0" { "path" "D:\\SteamLibrary" … }`) and the old
/// one (`"1" "D:\\SteamLibrary"`).
pub fn library_paths(text: &str) -> Vec<PathBuf> {
    let root = parse(text);
    let Some(Kv::Obj(items)) = root.get("libraryfolders") else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|(k, v)| match v {
            Kv::Obj(_) => v.get("path").and_then(Kv::str),
            Kv::Str(s) if k.chars().all(|c| c.is_ascii_digit()) => Some(s.as_str()),
            Kv::Str(_) => None,
        })
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
        .collect()
}

/// A game's name from its `appmanifest_<appid>.acf`, when it has one.
pub fn manifest_name(text: &str) -> Option<String> {
    let name = parse(text)
        .get("AppState")?
        .get("name")?
        .str()?
        .trim()
        .to_owned();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIBRARIES: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"contentid"		"1234"
		"apps"
		{
			"228980"		"123"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"label"		"Games"
		"apps"
		{
			"570"		"456"
		}
	}
}
"#;

    #[test]
    fn libraries_on_every_drive() {
        assert_eq!(
            library_paths(LIBRARIES),
            vec![
                PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from(r"D:\SteamLibrary")
            ]
        );
    }

    #[test]
    fn the_old_library_form() {
        let old = r#""LibraryFolders"
{
	"TimeNextStatsReport"		"1600000000"
	"ContentStatsID"		"-1234"
	"1"		"E:\\Games\\Steam"
}"#;
        assert_eq!(library_paths(old), vec![PathBuf::from(r"E:\Games\Steam")]);
    }

    #[test]
    fn nothing_from_junk() {
        assert!(library_paths("").is_empty());
        assert!(library_paths("{{{ \"x\"").is_empty());
        assert!(library_paths(r#""libraryfolders" { "0" { "label" "x" } }"#).is_empty());
    }

    #[test]
    fn a_manifest_names_its_game() {
        let acf = "\u{feff}\"AppState\"\n{\n\t\"appid\"\t\t\"570\"\n\t// a comment\n\t\"name\"\t\t\"Space \\\"Game\\\" 2\"\n\t\"installdir\"\t\t\"space\"\n}\n";
        assert_eq!(manifest_name(acf).as_deref(), Some("Space \"Game\" 2"));
        assert_eq!(manifest_name(r#""AppState" { "appid" "1" }"#), None);
        assert_eq!(manifest_name(r#""AppState" { "name" "  " }"#), None);
        assert_eq!(manifest_name("not a manifest"), None);
    }

    #[test]
    fn conditions_and_bare_tokens_are_read_past() {
        let kv = parse(r#""a" { "b" "1" [$WIN32] c 2 "d" { } }"#);
        let a = kv.get("A").unwrap();
        assert_eq!(a.get("b").and_then(Kv::str), Some("1"));
        assert_eq!(a.get("c").and_then(Kv::str), Some("2"));
        assert_eq!(a.get("d"), Some(&Kv::Obj(vec![])));
    }
}
