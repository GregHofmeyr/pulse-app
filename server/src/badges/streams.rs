//! Stream sessions, for "Chat, did you see that?" (hours streamed). Streams only exist in server
//! voice channels, so every one counts. A row is opened when a stream goes live and closed by the
//! end of that same stream (matched on its `started_at`), so a stream that ends and starts again
//! at once never has its new row closed by the old one's end. Bookkeeping never fails the
//! stream: errors are logged.

use pulse_protocol::rest::StreamInfo;
use sqlx::SqlitePool;

use super::catalogue::stat;
use super::time::parse;
use super::{engine, stats};
use crate::AppState;
use crate::db::{begin_write, now};
use crate::error::AppResult;

/// A stream went live (`Started::New`).
pub async fn started(s: &AppState, info: &StreamInfo) {
    let r = sqlx::query(
        "INSERT INTO stream_sessions (id, user_id, channel_id, started_at) VALUES (?, ?, ?, ?)",
    )
    .bind(pulse_protocol::ids::next_ulid().to_string())
    .bind(info.user_id.to_string())
    .bind(info.channel_id.to_string())
    .bind(&info.started_at)
    .execute(&s.db)
    .await;
    if let Err(e) = r {
        tracing::warn!(error = ?e, user = %info.user_id, "couldn't record a stream session");
    }
}

/// A stream ended (every `s.streams.end` that removed one): close its row and count its time.
pub async fn ended(s: &AppState, info: &StreamInfo) {
    match close(s, info).await {
        Ok(true) => engine::after(s, &[info.user_id]).await,
        Ok(false) => {}
        Err(e) => {
            tracing::warn!(error = ?e, user = %info.user_id, "couldn't close a stream session")
        }
    }
}

async fn close(s: &AppState, info: &StreamInfo) -> AppResult<bool> {
    let mut tx = begin_write(&s.db).await?;
    let row: Option<(String, String)> = sqlx::query_as(
        "UPDATE stream_sessions SET ended_at = ? WHERE id = (
             SELECT id FROM stream_sessions
             WHERE user_id = ? AND channel_id = ? AND started_at = ? AND ended_at IS NULL
             ORDER BY id LIMIT 1)
         RETURNING started_at, ended_at",
    )
    .bind(now())
    .bind(info.user_id.to_string())
    .bind(info.channel_id.to_string())
    .bind(&info.started_at)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((start, end)) = row else {
        return Ok(false);
    };
    let secs = (parse(&end)? - parse(&start)?).num_seconds();
    if secs > 0 {
        stats::add(&mut tx, info.user_id, stat::STREAM_SECS, secs).await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// At startup nothing is live (streams live in memory), so rows a previous run left open are
/// closed at zero length, as voice sessions are: when they really ended can't be known.
pub async fn reconcile_on_startup(db: &SqlitePool) -> anyhow::Result<()> {
    let n = sqlx::query("UPDATE stream_sessions SET ended_at = started_at WHERE ended_at IS NULL")
        .execute(db)
        .await?
        .rows_affected();
    if n > 0 {
        tracing::warn!(
            closed = n,
            "closed stream sessions left open by a previous run"
        );
    }
    Ok(())
}
