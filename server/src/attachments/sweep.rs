//! The attachments sweep (#57): uploads never sent, files of deleted messages, and temporary files
//! past their time. Each object goes first and then its row (an expired temporary file keeps its
//! row, marked purged, so its card can say Expired); a failed delete keeps the row for the next run.

use std::time::Duration;

use chrono::{DateTime, Utc};

use super::{PENDING_TTL_HOURS, Stored};
use crate::AppState;
use crate::db::stamp;
use crate::error::AppResult;

const EVERY: Duration = Duration::from_secs(600);
/// Past an upload's time before the sweep touches it: a send refuses uploads older than
/// [`PENDING_TTL_HOURS`], so by now none that passed that check can still be in flight.
const GRACE_HOURS: i64 = 1;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Uploads never sent (or whose message and channel were purged).
    pub pending: u32,
    /// Files of deleted messages.
    pub deleted: u32,
    /// Temporary files past their time.
    pub expired: u32,
}

pub async fn run(s: &AppState, now: DateTime<Utc>) -> AppResult<SweepReport> {
    let mut report = SweepReport::default();
    let Some(r2) = &s.r2 else { return Ok(report) };

    // Never sent. A purged message's or channel's rows look the same (SET NULL), so this also
    // catches whatever a purge's own delete missed.
    let pending: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, key, expires_at FROM attachments
         WHERE message_id IS NULL AND created_at < ?",
    )
    .bind(stamp(
        now - chrono::Duration::hours(PENDING_TTL_HOURS + GRACE_HOURS),
    ))
    .fetch_all(&s.db)
    .await?;
    for (id, key, expires_at) in pending {
        let st = Stored {
            key,
            temp: expires_at.is_some(),
        };
        if let Err(e) = r2.delete(&st.object_key()).await {
            tracing::warn!(error = ?e, "couldn't delete an unsent upload's object");
            continue;
        }
        // Guarded: never one a send took in the meantime.
        let n = sqlx::query("DELETE FROM attachments WHERE id = ? AND message_id IS NULL")
            .bind(&id)
            .execute(&s.db)
            .await?
            .rows_affected();
        report.pending += n as u32;
    }

    // Files of deleted messages: the delete removes them at once; this retries what it couldn't.
    let deleted: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT a.id, a.key, a.expires_at FROM attachments a
         JOIN messages m ON m.id = a.message_id WHERE m.deleted_at IS NOT NULL",
    )
    .fetch_all(&s.db)
    .await?;
    for (id, key, expires_at) in deleted {
        let st = Stored {
            key,
            temp: expires_at.is_some(),
        };
        if let Err(e) = r2.delete(&st.object_key()).await {
            tracing::warn!(error = ?e, "couldn't delete a deleted message's file");
            continue;
        }
        sqlx::query("DELETE FROM attachments WHERE id = ?")
            .bind(&id)
            .execute(&s.db)
            .await?;
        report.deleted += 1;
    }

    // Temporary files past their time: the bytes go, the row stays (the card says Expired).
    let expired: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, key FROM attachments
         WHERE expires_at IS NOT NULL AND purged_at IS NULL AND message_id IS NOT NULL
           AND expires_at <= ?",
    )
    .bind(stamp(now))
    .fetch_all(&s.db)
    .await?;
    for (id, key) in expired {
        let st = Stored { key, temp: true };
        if let Err(e) = r2.delete(&st.object_key()).await {
            tracing::warn!(error = ?e, "couldn't delete an expired file");
            continue;
        }
        sqlx::query("UPDATE attachments SET purged_at = ? WHERE id = ?")
            .bind(stamp(now))
            .bind(&id)
            .execute(&s.db)
            .await?;
        report.expired += 1;
    }
    Ok(report)
}

/// Run once now and then every ten minutes. Call after binding the listener.
pub fn spawn(s: AppState) {
    if s.r2.is_none() {
        return;
    }
    tokio::spawn(async move {
        loop {
            match run(&s, Utc::now()).await {
                Ok(r) if r.pending + r.deleted + r.expired > 0 => tracing::info!(
                    pending = r.pending,
                    deleted = r.deleted,
                    expired = r.expired,
                    "attachments sweep"
                ),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = ?e, "attachments sweep failed"),
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}
