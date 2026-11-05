//! DND timers (#45). Views already treat an expired DND as Online; this turns the stored status
//! back to Online and tells everyone whose view changed.

use std::time::Duration;

use pulse_protocol::ids::UserId;

use crate::AppState;
use crate::db::now;

/// How often expired DND timers are looked for: the most a timer can overrun for others (the
/// person's own clients flip on time by themselves).
pub const SWEEP_EVERY: Duration = Duration::from_secs(15);

/// End every DND whose timer ran out; returns how many.
pub async fn sweep_dnd(s: &AppState) -> anyhow::Result<usize> {
    let ended: Vec<String> = sqlx::query_scalar(
        "UPDATE users SET presence = 'online', dnd_until = NULL \
         WHERE presence = 'dnd' AND dnd_until IS NOT NULL AND dnd_until <= ? RETURNING id",
    )
    .bind(now())
    .fetch_all(&s.db)
    .await?;
    for id in &ended {
        let id: UserId = id.parse()?;
        super::routes::announce_own(s, id).await;
        s.hub.sync_presence(&s.db, id).await;
    }
    Ok(ended.len())
}

pub fn spawn_sweeper(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        loop {
            tick.tick().await;
            if let Err(e) = sweep_dnd(&state).await {
                tracing::warn!(error = %e, "DND sweep failed");
            }
        }
    });
}
