//! Opaque session tokens: 256-bit random, only the SHA-256 is stored, 90-day sliding expiry.

use base64::Engine;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use pulse_protocol::ids::UserId;
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

const LIFETIME_DAYS: i64 = 90;
/// Only rewrite expiry when the last recorded use is older than this (avoids a write per request).
const SLIDE_EVERY: Duration = Duration::hours(1);

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub async fn create(db: &SqlitePool, user: UserId) -> anyhow::Result<String> {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, created_at, last_used_at, expires_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(hash_token(&token))
    .bind(user.to_string())
    .bind(ts(now))
    .bind(ts(now))
    .bind(ts(now + Duration::days(LIFETIME_DAYS)))
    .execute(db)
    .await?;
    Ok(token)
}

/// Returns the user for a valid, unexpired token and slides its expiry forward.
pub async fn authenticate(db: &SqlitePool, token: &str) -> anyhow::Result<Option<UserId>> {
    let h = hash_token(token);
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT user_id, last_used_at, expires_at FROM sessions WHERE token_hash = ?",
    )
    .bind(&h)
    .fetch_optional(db)
    .await?;
    let Some((user_id, last_used, expires)) = row else {
        return Ok(None);
    };
    let now = Utc::now();
    if DateTime::parse_from_rfc3339(&expires)?.with_timezone(&Utc) <= now {
        return Ok(None);
    }
    if DateTime::parse_from_rfc3339(&last_used)?.with_timezone(&Utc) + SLIDE_EVERY < now {
        sqlx::query("UPDATE sessions SET last_used_at = ?, expires_at = ? WHERE token_hash = ?")
            .bind(ts(now))
            .bind(ts(now + Duration::days(LIFETIME_DAYS)))
            .bind(&h)
            .execute(db)
            .await?;
    }
    Ok(Some(user_id.parse()?))
}

pub async fn revoke(db: &SqlitePool, token: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
        .bind(hash_token(token))
        .execute(db)
        .await?;
    Ok(())
}
