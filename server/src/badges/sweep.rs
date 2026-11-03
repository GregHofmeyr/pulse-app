//! Early Adopter: everyone who joined in the first week from the launch day. The launch date is
//! configuration (`PULSE_LAUNCH_DATE`); with none set, nobody gets it.

use std::time::Duration;

use chrono::{DateTime, NaiveTime, Utc};
use pulse_protocol::ids::UserId;

use super::catalogue::stat;
use super::time::SAST;
use super::{engine, stats};
use crate::AppState;
use crate::db::stamp;
use crate::error::AppResult;

const EVERY: Duration = Duration::from_secs(3600);
/// Joining before launch day + this many days is early.
const WEEK_DAYS: i64 = 7;

/// Once the launch day (00:00 South African time) has come, give Early Adopter to everyone who
/// joined before the week from it ended. Returns the users newly given it.
pub async fn run(s: &AppState, now: DateTime<Utc>) -> AppResult<Vec<UserId>> {
    let Some(day) = s.cfg.launch_date else {
        return Ok(Vec::new());
    };
    let start = day
        .and_time(NaiveTime::MIN)
        .and_local_timezone(SAST)
        .single()
        .expect("a fixed offset is never ambiguous")
        .with_timezone(&Utc);
    if now < start {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM users WHERE created_at < ?
         AND id NOT IN (SELECT user_id FROM badge_stats WHERE key = ? AND value >= 1)",
    )
    .bind(stamp(start + chrono::Duration::days(WEEK_DAYS)))
    .bind(stat::EARLY_ADOPTER)
    .fetch_all(&s.db)
    .await?;
    let mut raised = Vec::new();
    let mut conn = s.db.acquire().await?;
    for id in ids {
        let Ok(user) = id.parse::<UserId>() else {
            continue;
        };
        stats::set_max(&mut conn, user, stat::EARLY_ADOPTER, 1).await?;
        raised.push(user);
    }
    drop(conn);
    engine::after(s, &raised).await;
    Ok(raised)
}

/// Run once now and then hourly. Call after binding the listener.
pub fn spawn(s: AppState) {
    if s.cfg.launch_date.is_none() {
        return;
    }
    tokio::spawn(async move {
        loop {
            match run(&s, Utc::now()).await {
                Ok(r) if !r.is_empty() => tracing::info!(users = r.len(), "early adopter sweep"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = ?e, "early adopter sweep failed"),
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}
