//! The one-time read of a new link's games (#379): `GetOwnedGames`, then `GetPlayerAchievements` for every played game
//! with stats, one call every [`crate::config::SteamConfig::backfill_gap`], one person at a time server-wide. Each
//! answer is stored silently as that game's state, so old unlocks are never announced, and the completion rate and
//! 100 % games (Completionist) get their numbers. A private profile pauses it until Steam answers again.

use pulse_protocol::ids::UserId;

use super::api::ApiError;
use super::{achievements, store};
use crate::AppState;

/// Start the worker in the background (it does nothing when no backfill is due, or one is already running).
pub fn kick(s: &AppState) {
    if !s.steam.api().has_key() {
        return;
    }
    let s = s.clone();
    tokio::spawn(async move { run_pending(&s).await });
}

/// The next link due a backfill, not counting those `tried` this run: waiting ones, and private ones Steam last
/// refused over an hour ago.
async fn next_due(s: &AppState, tried: &[String]) -> sqlx::Result<Option<(String, String)>> {
    let hour_ago = crate::db::stamp(jiff::Timestamp::now() - jiff::SignedDuration::from_hours(1));
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT user_id, steam_id FROM steam_links
         WHERE share_achievements = 1
           AND (backfill = 'pending' OR (backfill = 'private' AND private_since <= ?))
         ORDER BY linked_at",
    )
    .bind(hour_ago)
    .fetch_all(&s.db)
    .await?;
    Ok(rows.into_iter().find(|(u, _)| !tried.contains(u)))
}

/// Every backfill that's due, one after another.
pub async fn run_pending(s: &AppState) {
    let Ok(_one) = s.steam.inner.backfill.try_lock() else {
        return; // the running one picks up anything new before it stops
    };
    let mut tried = Vec::new();
    loop {
        let next = match next_due(s, &tried).await {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(error = %e, "Steam backfill queue not read");
                return;
            }
        };
        let Some((user, steam_id)) = next else {
            return;
        };
        tried.push(user.clone());
        let Ok(user) = user.parse::<UserId>() else {
            continue;
        };
        if let Err(e) = one(s, user, &steam_id).await {
            tracing::warn!(error = ?e, "Steam backfill failed");
        }
    }
}

/// How one person's backfill ended.
#[derive(Debug, PartialEq, Eq)]
enum End {
    Done,
    Private,
    /// Unlinked, sharing turned off, or Steam unavailable: left for later.
    Stopped,
}

async fn one(s: &AppState, user: UserId, steam_id: &str) -> crate::error::AppResult<()> {
    let end = read_games(s, user, steam_id).await?;
    let linked = still_linked(s, user, steam_id).await?;
    // The numbers (and any badge they earn) first, then the done mark (#403): whoever sees the backfill done sees its
    // completion stats too.
    if linked {
        store::recompute(s, user).await?;
    }
    match end {
        End::Done => {
            store::set_backfill(&s.db, user, store::DONE).await?;
            store::clear_private(&s.db, user).await?;
        }
        End::Private => {
            store::mark_private(&s.db, user).await?;
            store::set_backfill(&s.db, user, store::PRIVATE).await?;
        }
        End::Stopped => {}
    }
    if linked {
        store::tell(s, user).await;
    }
    Ok(())
}

async fn still_linked(s: &AppState, user: UserId, steam_id: &str) -> sqlx::Result<bool> {
    let hit: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM steam_links WHERE user_id = ? AND steam_id = ? AND share_achievements = 1",
    )
    .bind(user.to_string())
    .bind(steam_id)
    .fetch_optional(&s.db)
    .await?;
    Ok(hit.is_some())
}

async fn read_games(s: &AppState, user: UserId, steam_id: &str) -> crate::error::AppResult<End> {
    let api = s.steam.api();
    let games = match api.owned_games(steam_id).await {
        Ok(g) => g,
        Err(ApiError::Private) => return Ok(End::Private),
        Err(_) => return Ok(End::Stopped),
    };
    let gap = s.cfg.steam.backfill_gap;
    for g in games
        .iter()
        .filter(|g| g.has_stats && g.playtime_minutes > 0)
    {
        tokio::time::sleep(gap).await;
        if !still_linked(s, user, steam_id).await? {
            return Ok(End::Stopped);
        }
        // the game they're playing is the poller's (its session rule decides what's new), as is any it already saw
        if s.steam.tracked(user) == Some(g.app_id)
            || store::state(&s.db, user, g.app_id).await?.is_some()
        {
            continue;
        }
        match api.player_achievements(steam_id, g.app_id).await {
            Ok(st) if st.achievements.is_empty() => {}
            Ok(st) => {
                let unlocked = achievements::unlocked(&st.achievements);
                let total = st.achievements.len() as u32;
                if !store::save_state(&s.db, user, steam_id, g.app_id, &unlocked, total).await? {
                    return Ok(End::Stopped);
                }
            }
            Err(ApiError::NoStats) => {}
            Err(ApiError::Private) => return Ok(End::Private),
            Err(_) => return Ok(End::Stopped),
        }
    }
    Ok(End::Done)
}
