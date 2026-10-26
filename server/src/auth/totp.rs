//! TOTP (RFC 6238) codes, the at-rest encryption of their secrets, and recovery codes.

use anyhow::{anyhow, bail};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use pulse_protocol::ids::UserId;
use rand::{Rng, RngCore};
use sha2::{Digest, Sha256};
use totp_rs::{Algorithm, Builder, Secret};

const STEP_SECS: u64 = 30;
const NONCE_LEN: usize = 12;
const RECOVERY_ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
const RECOVERY_COUNT: usize = 10;

/// The 32-byte key that encrypts TOTP secrets at rest (`PULSE_TOTP_KEY`, 64 hex chars).
#[derive(Clone)]
pub struct TotpKey([u8; 32]);

impl std::fmt::Debug for TotpKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TotpKey(..)")
    }
}

impl TotpKey {
    /// Exactly 64 hex characters, either case. Errors never include the input.
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        let bytes = hex::decode(s).map_err(|_| anyhow!("not valid hex"))?;
        let key = <[u8; 32]>::try_from(bytes).map_err(|_| anyhow!("not 64 hex characters"))?;
        Ok(Self(key))
    }
}

/// A fresh 160-bit secret (RFC 4226 recommends 160 bits; authenticator apps expect it).
pub fn new_secret() -> [u8; 20] {
    let mut s = [0u8; 20];
    rand::rng().fill_bytes(&mut s);
    s
}

/// The secret as an authenticator app wants to see it: RFC 4648 base32, no padding.
pub fn secret_base32(secret: &[u8]) -> String {
    Secret::new(secret.into()).to_base32()
}

/// The `otpauth://` URI a QR code carries. Usernames are `[a-z0-9_.]` today; anything else is
/// percent-encoded in case that ever changes. Everything else is fixed.
pub fn otpauth_uri(username: &str, secret: &[u8]) -> String {
    let mut label = String::new();
    for b in username.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            label.push(b as char);
        } else {
            label.push_str(&format!("%{b:02X}"));
        }
    }
    format!(
        "otpauth://totp/Pulse:{label}?secret={}&issuer=Pulse&algorithm=SHA1&digits=6&period=30",
        secret_base32(secret)
    )
}

fn aad(user: UserId) -> String {
    format!("pulse-totp-v1:{user}")
}

/// `nonce || ciphertext+tag`, bound to the user so a blob can't be moved to another account.
pub fn encrypt(key: &TotpKey, user: UserId, secret: &[u8]) -> Vec<u8> {
    let cipher = ChaCha20Poly1305::new((&key.0).into());
    let mut nonce = [0u8; NONCE_LEN];
    rand::rng().fill_bytes(&mut nonce);
    let aad = aad(user);
    let ct = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: secret,
                aad: aad.as_bytes(),
            },
        )
        .expect("ChaCha20-Poly1305 encryption of a short message cannot fail");
    [nonce.as_slice(), &ct].concat()
}

/// Fails for a wrong key, a different user, or any altered byte. The error says nothing more.
pub fn decrypt(key: &TotpKey, user: UserId, blob: &[u8]) -> anyhow::Result<Vec<u8>> {
    if blob.len() < NONCE_LEN {
        bail!("stored secret is too short");
    }
    let (nonce, ct) = blob.split_at(NONCE_LEN);
    let aad = aad(user);
    ChaCha20Poly1305::new((&key.0).into())
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ct,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| anyhow!("stored secret does not decrypt"))
}

/// A TOTP code is exactly 6 ASCII digits once spaces are removed.
pub fn is_totp_shaped(code: &str) -> bool {
    let mut n = 0;
    for b in code.bytes().filter(|b| *b != b' ') {
        if !b.is_ascii_digit() {
            return false;
        }
        n += 1;
    }
    n == 6
}

/// The step (unix time / 30) in {t-1, t, t+1} whose code equals `code`, or `None`. The caller
/// rejects a step that isn't newer than the last one used. Every candidate is compared in
/// constant time and all three are always computed.
pub fn match_step(secret: &[u8], code: &str, now_unix: u64) -> Option<i64> {
    if !is_totp_shaped(code) {
        return None;
    }
    let code: Vec<u8> = code.bytes().filter(|b| *b != b' ').collect();
    let totp = Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_skew(0)
        .with_step_duration(STEP_SECS)
        .with_secret(secret.to_vec())
        .build_noncompliant();
    let now_step = (now_unix / STEP_SECS) as i64;
    let mut found = None;
    for step in now_step - 1..=now_step + 1 {
        if step < 0 {
            continue;
        }
        let expected = totp.generate(step as u64 * STEP_SECS).to_string();
        if ct_eq(expected.as_bytes(), &code) {
            found = Some(step);
        }
    }
    found
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Ten fresh recovery codes, shown as `xxxx-xxxx-xxxx-xxxx`.
pub fn new_recovery_codes() -> Vec<String> {
    let mut rng = rand::rng();
    (0..RECOVERY_COUNT)
        .map(|_| {
            let chars: Vec<char> = (0..16)
                .map(|_| RECOVERY_ALPHABET[rng.random_range(0..RECOVERY_ALPHABET.len())] as char)
                .collect();
            chars
                .chunks(4)
                .map(|g| g.iter().collect::<String>())
                .collect::<Vec<_>>()
                .join("-")
        })
        .collect()
}

/// Spaces and dashes removed, lowercased: what is hashed and compared.
pub fn normalize_recovery(code: &str) -> String {
    code.chars()
        .filter(|c| *c != ' ' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

/// SHA-256 hex of the normalised code (high-entropy, so no slow hash needed).
pub fn hash_recovery(code: &str) -> String {
    hex::encode(Sha256::digest(normalize_recovery(code).as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rfc_secret() -> &'static [u8] {
        b"12345678901234567890"
    }

    #[test]
    fn rfc6238_vectors() {
        // RFC 6238 lists 8-digit codes; HOTP truncation is `mod 10^digits`, so their last six
        // digits are the 6-digit codes.
        for (t, code) in [
            (59, "287082"),
            (1_111_111_109, "081804"),
            (1_111_111_111, "050471"),
            (1_234_567_890, "005924"),
            (2_000_000_000, "279037"),
            (20_000_000_000, "353130"),
        ] {
            assert_eq!(
                match_step(rfc_secret(), code, t),
                Some(t as i64 / 30),
                "t={t}"
            );
        }
    }

    #[test]
    fn window_is_plus_minus_one_step() {
        let t = 1_111_111_109u64; // step 37037037; code for it is 081804
        let step = (t / 30) as i64;
        let code = "081804";
        assert_eq!(match_step(rfc_secret(), code, t), Some(step));
        assert_eq!(match_step(rfc_secret(), code, t + 30), Some(step));
        assert_eq!(match_step(rfc_secret(), code, t - 30), Some(step));
        assert_eq!(match_step(rfc_secret(), code, t + 60), None);
        assert_eq!(match_step(rfc_secret(), code, t - 60), None);
    }

    #[test]
    fn malformed_codes_are_refused() {
        for c in [
            "28708",
            "2870822",
            "28708a",
            "",
            "      ",
            "-87082",
            "٢٨٧٠٨٢",
        ] {
            assert_eq!(match_step(rfc_secret(), c, 59), None, "{c:?}");
        }
        assert_eq!(match_step(rfc_secret(), "287 082", 59), Some(1));
    }

    #[test]
    fn totp_shape() {
        assert!(is_totp_shaped("123456"));
        assert!(is_totp_shaped(" 123 456 "));
        assert!(!is_totp_shaped("12345"));
        assert!(!is_totp_shaped("1234567"));
        assert!(!is_totp_shaped("12345a"));
        assert!(!is_totp_shaped("abcd-efgh-jkmn-pqrs"));
    }

    const KEY_HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    #[test]
    fn encrypt_round_trip_and_failures() {
        let key = TotpKey::parse(KEY_HEX).unwrap();
        let other = TotpKey::parse(&"ab".repeat(32)).unwrap();
        let (alex, sam) = (UserId::new(), UserId::new());
        let secret = new_secret();
        let blob = encrypt(&key, alex, &secret);
        assert_eq!(blob.len(), 12 + 20 + 16);
        assert_eq!(decrypt(&key, alex, &blob).unwrap(), secret);
        assert!(decrypt(&other, alex, &blob).is_err());
        assert!(decrypt(&key, sam, &blob).is_err());
        for i in [0, 12, blob.len() - 1] {
            let mut bad = blob.clone();
            bad[i] ^= 1;
            assert!(decrypt(&key, alex, &bad).is_err(), "byte {i}");
        }
        assert!(decrypt(&key, alex, &blob[..10]).is_err());
        assert_ne!(encrypt(&key, alex, &secret), blob, "nonce is random");
    }

    #[test]
    fn key_parse() {
        assert!(TotpKey::parse(KEY_HEX).is_ok());
        assert!(TotpKey::parse(&KEY_HEX.to_uppercase()).is_ok());
        for bad in [
            KEY_HEX[1..].to_string(),
            format!("{KEY_HEX}0"),
            "zz".repeat(32),
            String::new(),
        ] {
            let e = TotpKey::parse(&bad).unwrap_err();
            let msg = format!("{e:#}");
            if !bad.is_empty() {
                assert!(!msg.contains(&bad), "error echoes input: {msg}");
            }
        }
        assert!(!format!("{:?}", TotpKey::parse(KEY_HEX).unwrap()).contains("0102"));
    }

    #[test]
    fn secrets_and_uri() {
        let s = new_secret();
        assert_eq!(s.len(), 20);
        assert_ne!(s, new_secret());
        assert_eq!(
            secret_base32(rfc_secret()),
            "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"
        );
        assert_eq!(
            otpauth_uri("alex", rfc_secret()),
            "otpauth://totp/Pulse:alex?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Pulse&algorithm=SHA1&digits=6&period=30"
        );
    }

    #[test]
    fn recovery_codes() {
        let codes = new_recovery_codes();
        assert_eq!(codes.len(), 10);
        let set: std::collections::HashSet<_> = codes.iter().collect();
        assert_eq!(set.len(), 10);
        for c in &codes {
            let ok = c.len() == 19
                && c.split('-').count() == 4
                && c.split('-')
                    .all(|g| g.len() == 4 && g.bytes().all(|b| RECOVERY_ALPHABET.contains(&b)));
            assert!(ok, "bad format {c}");
        }
        assert_eq!(
            normalize_recovery(" ABCD-efgh jkmn-PQRS "),
            "abcdefghjkmnpqrs"
        );
        let c = &codes[0];
        assert_eq!(
            hash_recovery(c),
            hash_recovery(&c.to_uppercase().replace('-', " "))
        );
        assert_eq!(
            hash_recovery(c),
            hex::encode(Sha256::digest(normalize_recovery(c)))
        );
        assert_ne!(hash_recovery(c), hash_recovery(&codes[1]));
    }
}
