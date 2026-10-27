//! Periodic database cleanup. Runs hourly next to the media cleanup (`media::spawn_cleanup`).
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use sqlx::SqlitePool;

use crate::auth::invites;
use crate::error::AppResult;

/// Bug and crash reports are kept this long.
pub const REPORT_RETENTION_DAYS: i64 = 90;

/// How many rows one pass removed, per table.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pruned {
    pub sessions: u64,
    pub invites: u64,
    pub mfa_tickets: u64,
    pub reports: u64,
}

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Delete what has outlived its purpose, one statement per table:
/// - sessions past their expiry (their LiveKit grants go with them, by cascade);
/// - invites past their 24-hour life, used or not (nothing references an invite row, and an
///   expired one is already refused as unknown, so the answer doesn't change);
/// - expired two-factor login tickets;
/// - reports older than [`REPORT_RETENTION_DAYS`].
pub async fn prune(db: &SqlitePool, now: DateTime<Utc>) -> AppResult<Pruned> {
    let sessions = sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
        .bind(ts(now))
        .execute(db)
        .await?
        .rows_affected();
    let invites = sqlx::query("DELETE FROM invites WHERE created_at <= ?")
        .bind(invites::cutoff(now))
        .execute(db)
        .await?
        .rows_affected();
    let mfa_tickets = sqlx::query("DELETE FROM mfa_tickets WHERE expires_at <= ?")
        .bind(ts(now))
        .execute(db)
        .await?
        .rows_affected();
    let reports = sqlx::query("DELETE FROM bug_reports WHERE created_at <= ?")
        .bind(ts(now - TimeDelta::days(REPORT_RETENTION_DAYS)))
        .execute(db)
        .await?
        .rows_affected();
    let pruned = Pruned {
        sessions,
        invites,
        mfa_tickets,
        reports,
    };
    if pruned != Pruned::default() {
        tracing::info!(
            sessions,
            invites,
            mfa_tickets,
            reports,
            "expired rows pruned"
        );
    }
    Ok(pruned)
}

/// Hourly: [`prune`].
pub fn spawn_prune(db: SqlitePool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            if let Err(e) = prune(&db, Utc::now()).await {
                tracing::warn!(error = %e, "database cleanup failed");
            }
        }
    });
}
