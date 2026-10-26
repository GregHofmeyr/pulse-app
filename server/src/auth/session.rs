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

pub fn hash_token(token: &str) -> String {
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

/// Whether a session row with this hash still exists (expiry is not re-checked here).
pub async fn exists(db: &SqlitePool, token_hash: &str) -> anyhow::Result<bool> {
    let found: Option<i64> = sqlx::query_scalar("SELECT 1 FROM sessions WHERE token_hash = ?")
        .bind(token_hash)
        .fetch_optional(db)
        .await?;
    Ok(found.is_some())
}

/// A LiveKit room + participant identity a session was issued a token for.
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct LiveKitGrant {
    pub room: String,
    pub identity: String,
}

/// What `revoke_others` took away.
#[derive(Debug, Default)]
pub struct Revoked {
    /// Sessions that were still live (unexpired); expired rows are deleted but not counted.
    pub live: u64,
    /// LiveKit identities those sessions last held, to remove from their rooms.
    pub grants: Vec<LiveKitGrant>,
}

/// Remember that `token`'s session was just issued a LiveKit token for `identity` in `room`
/// (the last one issued owns the identity). False if the session is gone or expired, so a
/// token is never minted for a session that a concurrent revoke just deleted.
pub async fn record_grant(
    db: &SqlitePool,
    token: &str,
    room: &str,
    identity: &str,
) -> anyhow::Result<bool> {
    let now = ts(Utc::now());
    let done = sqlx::query(
        "INSERT INTO livekit_grants (room, identity, token_hash, issued_at)
         SELECT ?, ?, token_hash, ? FROM sessions WHERE token_hash = ? AND expires_at > ?
         ON CONFLICT (room, identity) DO UPDATE SET token_hash = excluded.token_hash, issued_at = excluded.issued_at",
    )
    .bind(room)
    .bind(identity)
    .bind(&now)
    .bind(hash_token(token))
    .bind(&now)
    .execute(db)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Whether a still-valid session was issued the latest LiveKit token for `identity` in `room`.
pub async fn grant_is_live(db: &SqlitePool, room: &str, identity: &str) -> anyhow::Result<bool> {
    let found: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM livekit_grants g JOIN sessions s ON s.token_hash = g.token_hash
         WHERE g.room = ? AND g.identity = ? AND s.expires_at > ?",
    )
    .bind(room)
    .bind(identity)
    .bind(ts(Utc::now()))
    .fetch_optional(db)
    .await?;
    Ok(found.is_some())
}

/// Delete this session; returns the LiveKit identities it still held (its grants go with it).
pub async fn revoke(db: &SqlitePool, token: &str) -> anyhow::Result<Vec<LiveKitGrant>> {
    let h = hash_token(token);
    let mut tx = crate::db::begin_write(db).await?;
    let grants = sqlx::query_as("SELECT room, identity FROM livekit_grants WHERE token_hash = ?")
        .bind(&h)
        .fetch_all(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
        .bind(&h)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(grants)
}

/// Delete every session of `user` except the one `keep` belongs to, expired ones included.
pub async fn revoke_others(db: &SqlitePool, user: UserId, keep: &str) -> anyhow::Result<Revoked> {
    let (user, keep) = (user.to_string(), hash_token(keep));
    let mut tx = crate::db::begin_write(db).await?;
    let live: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sessions WHERE user_id = ? AND token_hash != ? AND expires_at > ?",
    )
    .bind(&user)
    .bind(&keep)
    .bind(ts(Utc::now()))
    .fetch_one(&mut *tx)
    .await?;
    let grants = sqlx::query_as(
        "SELECT g.room, g.identity FROM livekit_grants g JOIN sessions s ON s.token_hash = g.token_hash
         WHERE s.user_id = ? AND s.token_hash != ?",
    )
    .bind(&user)
    .bind(&keep)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = ? AND token_hash != ?")
        .bind(&user)
        .bind(&keep)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Revoked {
        live: u64::try_from(live).unwrap_or(0),
        grants,
    })
}
