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

/// A grant whose participant left is pruned after this: LiveKit tokens live 10 minutes, plus a
/// margin for a reconnect that rejoins with the same (refreshed) token.
const GRANT_LEFT_FOR: Duration = Duration::minutes(15);
/// Never-joined grants kept per session (the newest); older ones are pruned. They are not aged
/// out: a lost join webhook makes a connected grant look never-joined, and it must stay known
/// for a later revoke and for that device's next full reconnect. 100 is well above a hover
/// sweep (the per-user limit allows 60 at once), and hundreds of token requests from that one
/// device before such a grant is evicted. With the rate limit this bounds the table per session.
pub const UNJOINED_GRANTS_KEPT: usize = 100;
/// Most finished or expired-session grants one token request prunes (bounds the work).
const PRUNE_BATCH: i64 = 100;

/// A grant of a revoked session: LiveKit participants carrying `id` in `room` are removed.
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct RevokedGrant {
    pub id: String,
    pub room: String,
}

/// What `revoke_others` took away.
#[derive(Debug, Default)]
pub struct Revoked {
    /// Sessions that were still live (unexpired); expired rows are deleted but not counted.
    pub live: u64,
    /// Those sessions' LiveKit grants.
    pub grants: Vec<RevokedGrant>,
}

/// Record a grant for a LiveKit token about to be issued to `token`'s session for `identity` in
/// `room`; returns its id (128 random bits, base64url) for the token's `pulse_grant` attribute.
/// `None` if the session is gone or expired, so no token is minted for a session that a
/// concurrent revoke just deleted. Also prunes: this session's never-joined grants beyond the
/// newest [`UNJOINED_GRANTS_KEPT`], and a batch of anyone's grants whose participant left more
/// than 15 min ago or whose session expired (revoked sessions' grants go with the session).
pub async fn record_grant(
    db: &SqlitePool,
    token: &str,
    room: &str,
    identity: &str,
) -> anyhow::Result<Option<String>> {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    let id = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let (now, h) = (Utc::now(), hash_token(token));
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
        "DELETE FROM livekit_grants WHERE token_hash = ?1 AND joined_sid IS NULL AND id NOT IN (
           SELECT id FROM livekit_grants WHERE token_hash = ?1 AND joined_sid IS NULL
           ORDER BY issued_at DESC, rowid DESC LIMIT ?2)",
    )
    .bind(&h)
    .bind(i64::try_from(UNJOINED_GRANTS_KEPT).unwrap_or(i64::MAX))
    .execute(db)
    .await?;
    sqlx::query(
        "DELETE FROM livekit_grants WHERE id IN (SELECT g.id FROM livekit_grants g
           JOIN sessions s ON s.token_hash = g.token_hash
           WHERE g.left_at < ?1 OR s.expires_at <= ?2 LIMIT ?3)",
    )
    .bind(ts(now - GRANT_LEFT_FOR))
    .bind(ts(now))
    .bind(PRUNE_BATCH)
    .execute(db)
    .await?;
    Ok(Some(id))
}

/// A LiveKit join webhook: accept it only if `grant` (the participant's `pulse_grant`
/// attribute) belongs to a live session and was issued for this very room + identity, and if
/// so note participant `sid` as the one connected with it.
pub async fn admit_join(
    db: &SqlitePool,
    grant: Option<&str>,
    room: &str,
    identity: &str,
    sid: &str,
) -> anyhow::Result<bool> {
    let Some(grant) = grant else {
        return Ok(false);
    };
    let done = sqlx::query(
        "UPDATE livekit_grants SET joined_sid = ?, left_at = NULL
         WHERE id = ? AND room = ? AND identity = ?
         AND token_hash IN (SELECT token_hash FROM sessions WHERE expires_at > ?)",
    )
    .bind(sid)
    .bind(grant)
    .bind(room)
    .bind(identity)
    .bind(ts(Utc::now()))
    .execute(db)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// A LiveKit leave webhook: the grant is finished if `sid` is still the participant connected
/// with it (a full reconnect's late leave for the old connection doesn't count).
pub async fn note_left(db: &SqlitePool, grant: &str, sid: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE livekit_grants SET left_at = ? WHERE id = ? AND joined_sid = ?")
        .bind(ts(Utc::now()))
        .bind(grant)
        .bind(sid)
        .execute(db)
        .await?;
    Ok(())
}

/// Grants with a connection on record that hasn't left, as (id, joined_sid): what a restart
/// checks against LiveKit's participant list (`voice::restore`).
pub async fn joined_grants(db: &SqlitePool) -> anyhow::Result<Vec<(String, String)>> {
    Ok(sqlx::query_as(
        "SELECT id, joined_sid FROM livekit_grants WHERE joined_sid IS NOT NULL AND left_at IS NULL",
    )
    .fetch_all(db)
    .await?)
}

/// Delete this session; returns its LiveKit grants (they go with it).
pub async fn revoke(db: &SqlitePool, token: &str) -> anyhow::Result<Vec<RevokedGrant>> {
    let h = hash_token(token);
    let mut tx = crate::db::begin_write(db).await?;
    let grants = sqlx::query_as("SELECT id, room FROM livekit_grants WHERE token_hash = ?")
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
        "SELECT g.id, g.room FROM livekit_grants g JOIN sessions s ON s.token_hash = g.token_hash
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
