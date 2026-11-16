//! The Deadlock poller (#390): every [`crate::config::DeadlockConfig::poll_every`] the match histories that are due;
//! every `rank_every` the ranks, the rank movers and the patch notes. While deadlock-api is backing off nothing is
//! called (the client logged it once).

use std::time::Instant;

use crate::AppState;

/// How long the matches seen are kept (profiles show the last one).
pub const KEEP_MATCHES: chrono::Duration = chrono::Duration::days(90);

/// Poll forever.
pub fn spawn(s: AppState) {
    tokio::spawn(async move {
        let mut every = tokio::time::interval(s.cfg.deadlock.poll_every);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut slow_at: Option<Instant> = None;
        loop {
            every.tick().await;
            if !in_use(&s).await {
                continue;
            }
            super::matches::tick(&s).await;
            if slow_at.is_none_or(|t| t.elapsed() >= s.cfg.deadlock.rank_every) {
                slow_at = Some(Instant::now());
                slow_tick(&s).await;
            }
        }
    });
}

/// Whether anyone has linked Steam or picked a Deadlock channel: until then deadlock-api is never called.
async fn in_use(s: &AppState) -> bool {
    sqlx::query_scalar::<_, i64>(
        "SELECT (SELECT COUNT(*) FROM steam_links) + (SELECT COUNT(*) FROM server_deadlock_channels)",
    )
        .fetch_one(&s.db)
        .await
        .is_ok_and(|n| n > 0)
}

/// Ranks, the weekly movers, patch notes, and the old matches pruned.
pub async fn slow_tick(s: &AppState) {
    if let Err(e) = super::ranks::refresh(s).await {
        tracing::warn!(error = ?e, "Deadlock ranks not refreshed");
    }
    if let Err(e) = super::ranks::movers_tick(s).await {
        tracing::warn!(error = ?e, "Deadlock rank movers not handled");
    }
    if let Err(e) = super::patches::tick(s).await {
        tracing::warn!(error = ?e, "Deadlock patch notes not handled");
    }
    let cutoff = crate::db::stamp(chrono::Utc::now() - KEEP_MATCHES);
    if let Err(e) = sqlx::query("DELETE FROM deadlock_matches WHERE started_at < ?")
        .bind(cutoff)
        .execute(&s.db)
        .await
    {
        tracing::warn!(error = %e, "old Deadlock matches not pruned");
    }
}
