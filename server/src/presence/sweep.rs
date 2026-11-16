//! Status timers (#45 DND, #153 Invisible, #152 custom status). Views already treat an expired timer as Online (or
//! no custom status); this turns the stored status back to Online, clears the custom status, and tells everyone
//! whose view changed.

use std::time::Duration;

use pulse_protocol::ids::UserId;
use sqlx::SqliteConnection;

use crate::AppState;
use crate::db::{begin_write, now};

/// How often expired timers are looked for: the most a timer can overrun for others (the
/// person's own clients flip on time by themselves).
pub const SWEEP_EVERY: Duration = Duration::from_secs(15);

/// On `tx`, turn every DND or Invisible whose timer ran out by `now` back to Online, as turning
/// it off would (an Invisible's freeze and off-list rows go too); only `who`'s if given. Returns
/// whose ended, for announcing after commit. Also run before a status change of one's own, so it
/// never builds on a run-out status the sweep hasn't reached yet.
pub(super) async fn end_expired(
    tx: &mut SqliteConnection,
    now: &str,
    who: Option<UserId>,
) -> sqlx::Result<Vec<String>> {
    let ended: Vec<(String, String)> = sqlx::query_as(
        "UPDATE users SET presence = 'online', dnd_until = NULL, invisible_at = NULL \
         WHERE presence IN ('dnd', 'invisible') AND dnd_until IS NOT NULL AND dnd_until <= ?1 \
         AND (?2 IS NULL OR id = ?2) RETURNING id, presence",
    )
    .bind(now)
    .bind(who.map(|u| u.to_string()))
    .fetch_all(&mut *tx)
    .await?;
    for (id, _) in &ended {
        // Off-list rows only matter during the invisibility they were taken off in.
        sqlx::query("DELETE FROM presence_hidden WHERE owner_id = ? AND active = 0")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    Ok(ended.into_iter().map(|(id, _)| id).collect())
}

/// End every DND, Invisible and custom status whose timer ran out; returns how many timers ended.
pub async fn sweep_timers(s: &AppState) -> anyhow::Result<usize> {
    // A cheap look first, so an idle sweep takes no write lock.
    let any: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM users WHERE (presence IN ('dnd', 'invisible') \
         AND dnd_until IS NOT NULL AND dnd_until <= ?1) \
         OR (custom_until IS NOT NULL AND custom_until <= ?1) LIMIT 1",
    )
    .bind(now())
    .fetch_optional(&s.db)
    .await?;
    if any.is_none() {
        return Ok(0);
    }
    let mut tx = begin_write(&s.db).await?;
    let at = now();
    let ended = end_expired(&mut tx, &at, None).await?;
    let custom = super::custom::end_expired(&mut tx, &at, None).await?;
    tx.commit().await?;
    for id in &ended {
        super::routes::announce_own(s, id.parse()?).await;
    }
    for id in &custom {
        super::custom::announce_own(s, id.parse()?).await;
    }
    let mut whose: Vec<&String> = ended.iter().chain(&custom).collect();
    whose.sort();
    whose.dedup();
    for id in whose {
        let id: UserId = id.parse()?;
        s.hub.sync_presence(&s.db, id).await;
    }
    Ok(ended.len() + custom.len())
}

pub fn spawn_sweeper(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        loop {
            tick.tick().await;
            if let Err(e) = sweep_timers(&state).await {
                tracing::warn!(error = %e, "status timer sweep failed");
            }
        }
    });
}
