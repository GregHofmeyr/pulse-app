use pulse_protocol::ids::UserId;
use rand::Rng;
use sqlx::{SqliteConnection, SqlitePool};

const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";

/// How long an unused invite stays valid.
pub const TTL_HOURS: i64 = 24;

/// Invites created at or before this instant have expired.
pub fn cutoff(now: chrono::DateTime<chrono::Utc>) -> String {
    (now - chrono::TimeDelta::hours(TTL_HOURS)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Create a single-use invite code (10 chars, unambiguous alphabet), valid for 24 hours.
pub async fn create(db: &SqlitePool, by: Option<UserId>) -> anyhow::Result<String> {
    insert(&mut *db.acquire().await?, by).await
}

/// [`create`] on a connection the caller holds (e.g. a write transaction that checked a cap).
pub async fn insert(conn: &mut SqliteConnection, by: Option<UserId>) -> anyhow::Result<String> {
    // ThreadRng is !Send: keep it out of the async state machine.
    let code: String = {
        let mut rng = rand::rng();
        (0..10)
            .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
            .collect()
    };
    sqlx::query("INSERT INTO invites (code, created_by, created_at) VALUES (?, ?, ?)")
        .bind(&code)
        .bind(by.map(|u| u.to_string()))
        .bind(crate::db::now())
        .execute(conn)
        .await?;
    Ok(code)
}

/// Invites `by` created that are still unused and unexpired.
pub async fn outstanding(conn: &mut SqliteConnection, by: UserId) -> sqlx::Result<i64> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM invites WHERE created_by = ? AND used_by IS NULL AND created_at > ?",
    )
    .bind(by.to_string())
    .bind(cutoff(chrono::Utc::now()))
    .fetch_one(conn)
    .await
}
