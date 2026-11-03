//! Ghost identities (spec §5.2): `g_` and 16 random bytes in base32, new for every watch session
//! and never derived from the user id, with `#v` / `#a` for the two stream-room connections.

use pulse_protocol::rest::StreamRole;
use rand::RngCore;

/// RFC 4648 base32, lowercase, no padding.
pub fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in bytes {
        buf = (buf << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buf >> bits) & 31) as usize] as char);
        }
        buf &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(ALPHABET[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn random_base32() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    base32(&bytes)
}

/// A fresh ghost id: `g_` and 16 random bytes in base32 (26 characters).
pub fn new_ghost_id() -> String {
    format!("g_{}", random_base32())
}

/// A fresh `pulse_grant` for a ghost ticket (random, like every grant).
pub fn new_grant() -> String {
    random_base32()
}

pub fn suffix(role: StreamRole) -> &'static str {
    match role {
        StreamRole::V => "v",
        StreamRole::A => "a",
    }
}

/// `"g_<26 base32>#v"` or `#a` → (id, role); anything else is not a ghost identity.
pub fn parse_ghost(identity: &str) -> Option<(&str, StreamRole)> {
    let (id, suffix) = identity.split_once('#')?;
    let body = id.strip_prefix("g_")?;
    let base32 = |c: char| c.is_ascii_lowercase() || ('2'..='7').contains(&c);
    if body.len() != 26 || !body.chars().all(base32) {
        return None;
    }
    let role = match suffix {
        "v" => StreamRole::V,
        "a" => StreamRole::A,
        _ => return None,
    };
    Some((id, role))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_protocol::ids::UserId;

    #[test]
    fn base32_is_rfc4648_lowercase() {
        assert_eq!(base32(b""), "");
        assert_eq!(base32(b"f"), "my");
        assert_eq!(base32(b"foobar"), "mzxw6ytboi");
        assert_eq!(base32(&[0xff; 5]), "77777777");
        assert_eq!(base32(&[0; 5]), "aaaaaaaa");
    }

    #[test]
    fn ghost_ids_are_random_base32_and_never_the_user_id() {
        let user = UserId::new().to_string();
        let ids: Vec<String> = (0..50).map(|_| new_ghost_id()).collect();
        for id in &ids {
            assert!(parse_ghost(&format!("{id}#v")).is_some(), "{id}");
            assert!(!id.to_uppercase().contains(&user), "{id}");
        }
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn parse_ghost_takes_only_ghost_v_and_a() {
        let id = new_ghost_id();
        assert_eq!(
            parse_ghost(&format!("{id}#v")),
            Some((id.as_str(), StreamRole::V))
        );
        assert_eq!(
            parse_ghost(&format!("{id}#a")),
            Some((id.as_str(), StreamRole::A))
        );
        assert_eq!(parse_ghost(&format!("{id}#v2")), None);
        assert_eq!(parse_ghost(&id), None);
        assert_eq!(parse_ghost(&format!("{}#v", UserId::new())), None);
        assert_eq!(parse_ghost("g_short#v"), None);
        assert_eq!(parse_ghost(&format!("{}#v", id.to_uppercase())), None);
    }
}
