//! The storage sweeper: removes what posting and deleting leave behind when a call to the object
//! store failed or never came, so nothing stays in the bucket forever.

use std::time::Duration;

use chrono::{DateTime, Utc};

use super::{TICKET_TTL_HOURS, object_key};
use crate::AppState;
use crate::db::stamp;
use crate::error::AppResult;

const EVERY: Duration = Duration::from_secs(600);
/// A clip's media row is written in the same transaction as its `clips` row, so a row without
/// one is an orphan; the grace period only keeps the sweeper clear of anything in flight.
const ORPHAN_AFTER_HOURS: i64 = 1;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub tickets: u32,
    pub orphans: u32,
}

/// Delete tickets older than 24 h and unreferenced clip media older than 1 h, the object first and
/// then the row. A failed object delete keeps the row, so the next run tries again.
pub async fn run(s: &AppState, now: DateTime<Utc>) -> AppResult<SweepReport> {
    let mut report = SweepReport::default();
    let Some(r2) = &s.r2 else { return Ok(report) };

    let tickets: Vec<(String, String)> =
        sqlx::query_as("SELECT id, key FROM clip_uploads WHERE created_at < ?")
            .bind(stamp(now - chrono::Duration::hours(TICKET_TTL_HOURS)))
            .fetch_all(&s.db)
            .await?;
    for (id, key) in tickets {
        if let Err(e) = r2.delete(&object_key(&key)).await {
            tracing::warn!(error = ?e, "couldn't delete an expired clip upload's object");
            continue;
        }
        sqlx::query("DELETE FROM clip_uploads WHERE id = ?")
            .bind(id)
            .execute(&s.db)
            .await?;
        report.tickets += 1;
    }

    let orphans: Vec<String> = sqlx::query_scalar(
        "SELECT hash FROM media
         WHERE kind = 'clip' AND created_at < ?
           AND NOT EXISTS (SELECT 1 FROM clips WHERE clips.video_key = media.hash)",
    )
    .bind(stamp(now - chrono::Duration::hours(ORPHAN_AFTER_HOURS)))
    .fetch_all(&s.db)
    .await?;
    for hash in orphans {
        if let Err(e) = r2.delete(&object_key(&hash)).await {
            tracing::warn!(error = ?e, "couldn't delete an orphaned clip's object");
            continue;
        }
        // Guarded: a clip posted in the meantime keeps its row.
        let gone = sqlx::query(
            "DELETE FROM media WHERE hash = ? AND kind = 'clip'
             AND NOT EXISTS (SELECT 1 FROM clips WHERE clips.video_key = media.hash)",
        )
        .bind(hash)
        .execute(&s.db)
        .await?
        .rows_affected();
        report.orphans += gone as u32;
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
                Ok(r) if r.tickets + r.orphans > 0 => {
                    tracing::info!(tickets = r.tickets, orphans = r.orphans, "clip sweep");
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = ?e, "clip sweep failed"),
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}
