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

/// The LiveKit participant attribute that carries a token's grant id.
pub const GRANT_ATTRIBUTE: &str = "pulse_grant";

/// Unused grants older than this are pruned. A LiveKit token lives 10 minutes, so only a grant
/// whose join webhook was lost could still matter after a day.
const GRANT_PRUNE_AFTER: Duration = Duration::days(1);

/// A participant (LiveKit room + identity) to remove from LiveKit.
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct LiveKitParticipant {
    pub room: String,
    pub identity: String,
}

/// What `revoke_others` took away.
#[derive(Debug, Default)]
pub struct Revoked {
    /// Sessions that were still live (unexpired); expired rows are deleted but not counted.
    pub live: u64,
    /// The participants that joined LiveKit with those sessions' grants.
    pub participants: Vec<LiveKitParticipant>,
}

/// Record a grant for a LiveKit token about to be issued to `token`'s session for `identity` in
/// `room`; returns its id (128 random bits, base64url) for the token's `pulse_grant` attribute.
/// `None` if the session is gone or expired, so no token is minted for a session that a
/// concurrent revoke just deleted. Also prunes this session's old, never-joined grants.
pub async fn record_grant(
    db: &SqlitePool,
    token: &str,
    room: &str,
    identity: &str,
) -> anyhow::Result<Option<String>> {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    let id = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let (h, now) = (hash_token(token), Utc::now());
    let done = sqlx::query(
        "INSERT INTO livekit_grants (id, token_hash, room, identity, issued_at)
         SELECT ?, token_hash, ?, ?, ? FROM sessions WHERE token_hash = ? AND expires_at > ?",
    )
    .bind(&id)
    .bind(room)
    .bind(identity)
    .bind(ts(now))
    .bind(&h)
    .bind(ts(now))
    .execute(db)
    .await?;
    if done.rows_affected() == 0 {
        return Ok(None);
    }
    sqlx::query(
        "DELETE FROM livekit_grants WHERE token_hash = ? AND issued_at < ?
         AND id NOT IN (SELECT grant_id FROM livekit_joined)",
    )
    .bind(&h)
    .bind(ts(now - GRANT_PRUNE_AFTER))
    .execute(db)
    .await?;
    Ok(Some(id))
}

/// A LiveKit join webhook: accept it only if `grant` (the participant's `pulse_grant`
/// attribute) belongs to a live session and was issued for this very room + identity, and if
/// so remember it as the grant this participant joined with.
pub async fn admit_join(
    db: &SqlitePool,
    grant: Option<&str>,
    room: &str,
    identity: &str,
) -> anyhow::Result<bool> {
    let Some(grant) = grant else {
        return Ok(false);
    };
    let now = ts(Utc::now());
    let done = sqlx::query(
        "INSERT INTO livekit_joined (room, identity, grant_id, joined_at)
         SELECT g.room, g.identity, g.id, ? FROM livekit_grants g
         JOIN sessions s ON s.token_hash = g.token_hash
         WHERE g.id = ? AND g.room = ? AND g.identity = ? AND s.expires_at > ?
         ON CONFLICT (room, identity) DO UPDATE SET grant_id = excluded.grant_id, joined_at = excluded.joined_at",
    )
    .bind(&now)
    .bind(grant)
    .bind(room)
    .bind(identity)
    .bind(&now)
    .execute(db)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Delete this session; returns the LiveKit participants that joined with its grants.
pub async fn revoke(db: &SqlitePool, token: &str) -> anyhow::Result<Vec<LiveKitParticipant>> {
    let h = hash_token(token);
    let mut tx = crate::db::begin_write(db).await?;
    let participants = sqlx::query_as(
        "SELECT j.room, j.identity FROM livekit_joined j JOIN livekit_grants g ON g.id = j.grant_id
         WHERE g.token_hash = ?",
    )
    .bind(&h)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
        .bind(&h)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(participants)
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
    let participants = sqlx::query_as(
        "SELECT j.room, j.identity FROM livekit_joined j
         JOIN livekit_grants g ON g.id = j.grant_id
         JOIN sessions s ON s.token_hash = g.token_hash
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
        participants,
    })
}
