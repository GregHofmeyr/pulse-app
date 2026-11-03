//! The `badge_stats` counters. Hooks bump them inside the transactions where things happen; the
//! engine reads them after commit. Missing counters read as 0.

use std::collections::HashMap;

use pulse_protocol::ids::UserId;
use sqlx::SqliteConnection;

use crate::error::AppResult;

/// Add `delta` (which may be negative) to a counter, never going below 0; the new value.
pub async fn add(
    conn: &mut SqliteConnection,
    user: UserId,
    key: &str,
    delta: i64,
) -> AppResult<i64> {
    Ok(sqlx::query_scalar(
        "INSERT INTO badge_stats (user_id, key, value) VALUES (?, ?, MAX(0, ?)) \
         ON CONFLICT (user_id, key) DO UPDATE SET value = MAX(0, value + ?) RETURNING value",
    )
    .bind(user.to_string())
    .bind(key)
    .bind(delta)
    .bind(delta)
    .fetch_one(conn)
    .await?)
}

/// Raise a counter to `value` if it is lower; the new value.
pub async fn set_max(
    conn: &mut SqliteConnection,
    user: UserId,
    key: &str,
    value: i64,
) -> AppResult<i64> {
    Ok(sqlx::query_scalar(
        "INSERT INTO badge_stats (user_id, key, value) VALUES (?, ?, ?) \
         ON CONFLICT (user_id, key) DO UPDATE SET value = MAX(value, excluded.value) \
         RETURNING value",
    )
    .bind(user.to_string())
    .bind(key)
    .bind(value)
    .fetch_one(conn)
    .await?)
}

pub async fn set(
    conn: &mut SqliteConnection,
    user: UserId,
    key: &str,
    value: i64,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO badge_stats (user_id, key, value) VALUES (?, ?, ?) \
         ON CONFLICT (user_id, key) DO UPDATE SET value = excluded.value",
    )
    .bind(user.to_string())
    .bind(key)
    .bind(value)
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn get(conn: &mut SqliteConnection, user: UserId, key: &str) -> AppResult<i64> {
    let v: Option<i64> =
        sqlx::query_scalar("SELECT value FROM badge_stats WHERE user_id = ? AND key = ?")
            .bind(user.to_string())
            .bind(key)
            .fetch_optional(conn)
            .await?;
    Ok(v.unwrap_or(0))
}

/// Every counter the user has.
pub async fn all(conn: &mut SqliteConnection, user: UserId) -> AppResult<HashMap<String, i64>> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT key, value FROM badge_stats WHERE user_id = ?")
            .bind(user.to_string())
            .fetch_all(conn)
            .await?;
    Ok(rows.into_iter().collect())
}
