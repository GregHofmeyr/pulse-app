//! Live achievements (#379): every [`crate::config::SteamConfig::poll_every`], each linked person who shares
//! achievements and plays a Steam game (or stopped less than `tail` ago) gets one `GetPlayerAchievements` call; what's
//! new since the stored state becomes a card. Steam refusing (private game details) backs them off for an hour and
//! tells their Settings. Steam hours (#383) are read from the same loop, about hourly.

use pulse_protocol::ids::UserId;

use super::api::ApiError;
use super::{ERROR_BACKOFF, PRIVATE_BACKOFF, achievements, backfill, cards, store};
use crate::AppState;

/// How much earlier than the detected start an unlock still counts for a game never seen before (detection runs
/// every 15 s, and clocks differ).
const SESSION_SLACK_SECS: i64 = 120;
/// How often the poller also retries backfills that met a private profile.
const BACKFILL_RETRY_EVERY: u32 = 30;
/// How often (in rounds) everyone's Steam hours are read (#383): about hourly, and at start.
const HOURS_EVERY: u32 = 30;

/// Poll forever.
pub fn spawn(s: AppState) {
    if !s.steam.api().has_key() {
        tracing::info!("no PULSE_STEAM_API_KEY: Steam achievements are off");
        return;
    }
    tokio::spawn(async move {
        let mut n: u32 = 0;
        let mut every = tokio::time::interval(s.cfg.steam.poll_every);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            every.tick().await;
            if n.is_multiple_of(HOURS_EVERY) {
                let s = s.clone();
                tokio::spawn(async move { super::hours::tick(&s).await });
            }
            tick(&s).await;
            n = n.wrapping_add(1);
            if n.is_multiple_of(BACKFILL_RETRY_EVERY) {
                backfill::kick(&s);
            }
        }
    });
}

/// One round: everyone who shares achievements and is playing (or just stopped).
pub async fn tick(s: &AppState) {
    if !s.steam.api().has_key() {
        return;
    }
    let rows: Vec<(String, String)> = match sqlx::query_as(
        "SELECT user_id, steam_id FROM steam_links WHERE share_achievements = 1",
    )
    .fetch_all(&s.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "Steam links not read");
            return;
        }
    };
    for (user, steam_id) in rows {
        let Ok(user) = user.parse::<UserId>() else {
            continue;
        };
        let playing = s.hub.playing_of(user);
        let now_app = s.steam.app_of(user, playing.as_ref()).map(|app| {
            let start = playing
                .as_ref()
                .and_then(|p| p.started_at.as_deref())
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.timestamp())
                .unwrap_or_else(|| chrono::Utc::now().timestamp());
            (app, start - SESSION_SLACK_SECS)
        });
        let Some((app, session_start)) = s.steam.observe(user, now_app, s.cfg.steam.tail) else {
            continue;
        };
        if s.steam.backed_off(user) || s.steam.has_no_stats(user, app) {
            continue;
        }
        let game_hint = playing.map(|p| p.name);
        if let Err(e) = poll_one(s, user, &steam_id, app, session_start, game_hint.as_deref()).await
        {
            tracing::warn!(error = ?e, "Steam achievements poll failed");
        }
    }
}

/// Read `user`'s achievements in `app` once and act on what changed.
pub async fn poll_one(
    s: &AppState,
    user: UserId,
    steam_id: &str,
    app: u32,
    session_start: i64,
    game_hint: Option<&str>,
) -> crate::error::AppResult<()> {
    let stats = match s.steam.api().player_achievements(steam_id, app).await {
        Ok(st) => st,
        Err(ApiError::Private) => {
            s.steam.back_off(user, PRIVATE_BACKOFF);
            if store::mark_private(&s.db, user).await? {
                store::tell(s, user).await;
            }
            return Ok(());
        }
        Err(ApiError::NoStats) => {
            s.steam.mark_no_stats(user, app);
            return Ok(());
        }
        Err(ApiError::NoKey | ApiError::Budget) => return Ok(()),
        Err(ApiError::Failed(why)) => {
            tracing::debug!(why, "Steam achievements unavailable for now");
            s.steam.back_off(user, ERROR_BACKOFF);
            return Ok(());
        }
    };
    let mut changed = false;
    if store::clear_private(&s.db, user).await? {
        changed = true;
        backfill::kick(s);
    }
    if stats.achievements.is_empty() {
        // a game with stats but no achievements: nothing to follow
        s.steam.mark_no_stats(user, app);
        if changed {
            store::tell(s, user).await;
        }
        return Ok(());
    }
    let stored = store::state(&s.db, user, app).await?;
    let new: Vec<(String, i64)> = achievements::new_unlocks(
        stored.as_ref().map(|(u, _)| u),
        &stats.achievements,
        session_start,
    )
    .into_iter()
    .map(|a| (a.api_name.clone(), a.unlock_time))
    .collect();
    let unlocked = achievements::unlocked(&stats.achievements);
    let total = stats.achievements.len() as u32;
    let after = (unlocked.len() as u32, total);
    let before = stored.as_ref().map(|(u, t)| (u.len() as u32, *t));
    let completed = achievements::completed_now(before, after, !new.is_empty());
    if before != Some(after) || stored.as_ref().is_some_and(|(u, _)| *u != unlocked) {
        if !store::save_state(&s.db, user, steam_id, app, &unlocked, total).await? {
            return Ok(()); // unlinked meanwhile
        }
        store::recompute(s, user).await?;
        changed = true;
    }
    let hint = stats.game.as_deref().or(game_hint);
    cards::announce(s, user, app, hint, &new, after.0, after.1, completed).await?;
    if changed {
        store::tell(s, user).await;
    }
    Ok(())
}
