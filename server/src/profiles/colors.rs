//! Name colours: strict `#rrggbb` parsing and the lightening rule shared with the client
//! (plan reconciliation 7).

/// A name colour below this WCAG relative luminance is mixed towards white until it reaches it.
pub const MIN_NAME_LUMINANCE: f64 = 0.39;

/// `#rrggbb`, case-insensitive, and nothing else.
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let b = s.strip_prefix('#')?.as_bytes();
    if b.len() != 6 || !b.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let pair = |i: usize| u8::from_str_radix(std::str::from_utf8(&b[i..i + 2]).ok()?, 16).ok();
    Some([pair(0)?, pair(2)?, pair(4)?])
}

/// `#rrggbb`, lowercase.
pub fn to_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// WCAG relative luminance.
pub fn luminance(c: [u8; 3]) -> f64 {
    let lin = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// `c` unchanged if it is bright enough for a name, else mixed towards white in 1% steps to the
/// smallest mix that is.
pub fn lighten_for_name(c: [u8; 3]) -> [u8; 3] {
    let mix = |k: u32| {
        c.map(|v| {
            let v = f64::from(v);
            (v + (255.0 - v) * f64::from(k) / 100.0 + 0.5).floor() as u8
        })
    };
    if luminance(c) >= MIN_NAME_LUMINANCE {
        return c;
    }
    (1..=100)
        .map(mix)
        .find(|m| luminance(*m) >= MIN_NAME_LUMINANCE)
        .unwrap_or([255; 3])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lightened(s: &str) -> String {
        to_hex(lighten_for_name(parse_hex(s).unwrap()))
    }

    #[test]
    fn lighten_matches_the_shared_vectors() {
        for (from, to) in [
            ("#000000", "#a8a8a8"),
            ("#0000ff", "#a1a1ff"),
            ("#ff0000", "#ff8585"),
            ("#123456", "#9baab8"),
            ("#7f7f7f", "#a8a8a8"),
            ("#2a9d8f", "#64b7ad"),
            ("#ffffff", "#ffffff"),
        ] {
            assert_eq!(lightened(from), to, "{from}");
        }
    }

    #[test]
    fn presets_pass_unchanged() {
        for c in [
            "#ffc4a3", "#d4c2ff", "#a8e6cf", "#a3d8ff", "#ffbe7a", "#ff7fae", "#8fd0ff", "#c9a8ff",
            "#a2f7c2", "#8fd2ff", "#ffe58f", "#ff9d96", "#f6b6ff", "#9fb4ff",
        ] {
            assert_eq!(lightened(c), c);
        }
    }

    #[test]
    fn parse_hex_is_strict() {
        assert_eq!(parse_hex("#ABCDEF"), Some([0xab, 0xcd, 0xef]));
        for bad in ["abcdef", "#abc", "#abcdeg", "#abcdef0", "", "#+bcdef"] {
            assert_eq!(parse_hex(bad), None, "{bad}");
        }
    }
}
