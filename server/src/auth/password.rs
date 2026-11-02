use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};

/// argon2id with library defaults.
pub fn hash(password: &str) -> anyhow::Result<String> {
    let mut raw = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rng(), &mut raw);
    let salt = SaltString::encode_b64(&raw).map_err(|e| anyhow::anyhow!("salt: {e}"))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("hash: {e}"))
}

pub fn verify(password: &str, stored: &str) -> bool {
    PasswordHash::new(stored)
        .map(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
        .unwrap_or(false)
}

/// At most this many argon2 operations at once: bounds the CPU/RAM an attacker can burn.
static ARGON_SLOTS: std::sync::LazyLock<tokio::sync::Semaphore> =
    std::sync::LazyLock::new(|| tokio::sync::Semaphore::new(2));

/// `hash` on a blocking thread, behind the concurrency cap.
pub async fn hash_async(password: String) -> anyhow::Result<String> {
    let _slot = ARGON_SLOTS.acquire().await?;
    tokio::task::spawn_blocking(move || hash(&password)).await?
}

/// `verify` on a blocking thread, behind the concurrency cap.
pub async fn verify_async(password: String, stored: String) -> bool {
    let Ok(_slot) = ARGON_SLOTS.acquire().await else {
        return false;
    };
    tokio::task::spawn_blocking(move || verify(&password, &stored))
        .await
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    /// Made by argon2 0.5.3 (`hash` above) before the 0.6 bump. Stored hashes in `users` look like
    /// this, so every later argon2 must still verify it.
    const PHC_0_5_3: &str = "$argon2id$v=19$m=19456,t=2,p=1$JaAIg0p3Bn/Hc1JEYRxrqw$SPHlswzl7Je6czRCD9kXbg9Se9JjG9ez6th3pT3FJ/Y";
    const PASSWORD: &str = "correct horse battery staple";

    #[test]
    fn hash_from_argon2_0_5_still_verifies() {
        assert!(super::verify(PASSWORD, PHC_0_5_3));
        assert!(!super::verify("correct horse battery stapler", PHC_0_5_3));
    }

    #[test]
    fn new_hashes_keep_the_stored_parameters() {
        let h = super::hash(PASSWORD).unwrap();
        // argon2id v19, 19 MiB, 2 passes, 1 lane: same as the hashes already stored.
        let rest = h
            .strip_prefix("$argon2id$v=19$m=19456,t=2,p=1$")
            .unwrap_or_else(|| panic!("parameters changed: {h}"));
        let (salt, out) = rest.split_once('$').unwrap();
        assert_eq!(salt.len(), 22, "16-byte salt"); // unpadded B64
        assert_eq!(out.len(), 43, "32-byte output");
        assert!(super::verify(PASSWORD, &h));
        assert!(!super::verify("wrong", &h));
    }

    #[test]
    fn garbage_never_verifies() {
        assert!(!super::verify(PASSWORD, ""));
        assert!(!super::verify(PASSWORD, "x"));
        assert!(!super::verify(
            PASSWORD,
            "$argon2id$v=19$m=19456,t=2,p=1$bad"
        ));
    }
}
