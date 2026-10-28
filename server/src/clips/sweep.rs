//! The storage sweeper: removes what posting and deleting leave behind when a call to the object
//! store failed or never came, so nothing stays in the bucket forever.

use std::time::Duration;

use chrono::{DateTime, Utc};

use super::{TICKET_TTL_HOURS, object_key};
use crate::AppState;
use crate::db::{begin_write, stamp};
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

    type Ticket = (String, String, String, Option<String>, i64, i64, String);
    let tickets: Vec<Ticket> = sqlx::query_as(
        "SELECT id, key, user_id, channel_id, size, duration_ms, created_at
         FROM clip_uploads WHERE created_at < ?",
    )
    .bind(stamp(now - chrono::Duration::hours(TICKET_TTL_HOURS)))
    .fetch_all(&s.db)
    .await?;
    for ticket in tickets {
        let (id, key, user_id, channel_id, size, duration_ms, created_at) = &ticket;
        // Claim the row in a short transaction (a confirm racing us then finds no ticket), and
        // leave a key a posted clip uses alone. No transaction is open during the storage call.
        let mut tx = begin_write(&s.db).await?;
        let claimed = sqlx::query("DELETE FROM clip_uploads WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let in_use: Option<i64> = sqlx::query_scalar("SELECT 1 FROM clips WHERE video_key = ?")
            .bind(key)
            .fetch_optional(&mut *tx)
            .await?;
        tx.commit().await?;
        if claimed == 0 {
            continue;
        }
        if in_use.is_none()
            && let Err(e) = r2.delete(&object_key(key)).await
        {
            tracing::warn!(error = ?e, "couldn't delete an expired clip upload's object");
            // Put the ticket back as it was (still expired) so the next run retries.
            sqlx::query(
                "INSERT OR IGNORE INTO clip_uploads
                 (id, key, user_id, channel_id, size, duration_ms, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(key)
            .bind(user_id)
            .bind(channel_id)
            .bind(size)
            .bind(duration_ms)
            .bind(created_at)
            .execute(&s.db)
            .await?;
            continue;
        }
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
