//! Timed mutes running out (#325). Reads already treat a run-out mute as no mute, so nothing changes for a fresh
//! Ready or fetch; but a connected user's devices would keep what they were last told until the next event. This
//! clears each run-out mute (like the DND timers, `presence::sweep`) and publishes its owner's [`MutesChanged`],
//! which brings everything a mute change does: the inbox recount through the hub's inbox worker (told only if the
//! count moved), and clients fetching their inbox lists again (a plain reply the mute hid is back). Older clients
//! already handle both events. A mute that ran out while the server was down is cleared on the first sweep.
//!
//! [`MutesChanged`]: pulse_protocol::gateway::Event::MutesChanged

use std::time::Duration;

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::UserId;

use crate::AppState;
use crate::db::now;
use crate::error::AppResult;

/// How often run-out mutes are looked for: the most a mute can overrun on its owner's devices.
pub const SWEEP_EVERY: Duration = Duration::from_secs(15);

/// Clear every mute whose time ran out and tell each owner their mutes; returns how many mutes were cleared.
pub async fn sweep_mutes(s: &AppState) -> AppResult<usize> {
    let mut tx = crate::db::begin_write(&s.db).await?;
    let ended: Vec<(String, String, String)> = sqlx::query_as(
        "UPDATE notification_prefs SET muted = 0, muted_until = NULL
         WHERE muted = 1 AND muted_until IS NOT NULL AND muted_until <= ?
         RETURNING user_id, target_kind, target_id",
    )
    .bind(now())
    .fetch_all(&mut *tx)
    .await?;
    // a row that only held the mute goes, as an unmute leaves it (the row may also hold a level, #195)
    for (user, kind, id) in &ended {
        sqlx::query(
            "DELETE FROM notification_prefs WHERE user_id = ? AND target_kind = ? AND target_id = ?
             AND muted = 0 AND level IS NULL",
        )
        .bind(user)
        .bind(kind)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    let mut told: Vec<UserId> = Vec::new();
    for (user, _, _) in &ended {
        let user: UserId = user.parse().map_err(anyhow::Error::from)?;
        if told.contains(&user) {
            continue;
        }
        told.push(user);
        let mutes = super::routes::mutes_of(&s.db, user).await?;
        s.hub
            .publish(
                &s.db,
                Event::MutesChanged {
                    user_id: user,
                    mutes,
                },
            )
            .await;
    }
    Ok(ended.len())
}

/// Sweep now and then every [`SWEEP_EVERY`]. Call after binding the listener.
pub fn spawn_sweeper(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        loop {
            tick.tick().await;
            if let Err(e) = sweep_mutes(&state).await {
                tracing::warn!(error = ?e, "mute sweep failed");
            }
        }
    });
}
