//! Steam hours (#383): about once an hour, each linked person who shares Steam hours gets one
//! `GetRecentlyPlayedGames` call; the minutes they played over the last two weeks (`playtime_2weeks`, summed) are
//! stored for their profile card and each server's board, and their best fortnight feeds Touch Grass.

use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::SteamHoursEntry;
use sqlx::SqlitePool;

use super::api::ApiError;
use crate::AppState;
use crate::badges::catalogue::stat;
use crate::db::now;
use crate::error::AppResult;

/// The most people a server's board shows.
pub const BOARD_MAX: i64 = 10;
/// Hours older than this aren't shown on a board (the server stopped reading them: no key any more, or Steam down).
const STALE_HOURS: i64 = 48;

/// Read everyone who shares Steam hours, one after another.
pub async fn tick(s: &AppState) {
    if !s.steam.api().has_key() {
        return;
    }
    let rows: Vec<(String, String)> =
        match sqlx::query_as("SELECT user_id, steam_id FROM steam_links WHERE share_hours = 1")
            .fetch_all(&s.db)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "Steam links not read for hours");
                return;
            }
        };
    for (user, steam_id) in rows {
        let Ok(user) = user.parse::<UserId>() else {
            continue;
        };
        if s.steam.backed_off(user) {
            continue;
        }
        if let Err(e) = one(s, user, &steam_id).await {
            tracing::warn!(error = ?e, "Steam hours not read");
        }
    }
}

/// Read `user`'s hours now in the background (they just linked, or turned "Share Steam hours" on).
pub fn kick_one(s: &AppState, user: UserId) {
    if !s.steam.api().has_key() {
        return;
    }
    let s = s.clone();
    tokio::spawn(async move {
        let link = match super::store::link_of(&s.db, user).await {
            Ok(Some(l)) if l.share_hours => l,
            Ok(_) => return,
            Err(e) => {
                tracing::warn!(error = %e, "Steam link not read for hours");
                return;
            }
        };
        if let Err(e) = one(&s, user, &link.steam_id).await {
            tracing::warn!(error = ?e, "Steam hours not read");
        }
    });
}

/// Read `user`'s last two weeks once and store them (only while they're still linked to `steam_id` and share
/// hours). A private profile clears what was shown.
pub async fn one(s: &AppState, user: UserId, steam_id: &str) -> AppResult<()> {
    let minutes: Option<u64> = match s.steam.api().recent_games(steam_id).await {
        Ok(games) => Some(games.iter().map(|g| g.minutes_2weeks).sum()),
        Err(ApiError::Private) => None,
        Err(ApiError::NoKey | ApiError::Budget | ApiError::NoStats) => return Ok(()),
        Err(ApiError::Failed(why)) => {
            tracing::debug!(why, "Steam hours unavailable for now");
            return Ok(());
        }
    };
    let before = super::store::link_of(&s.db, user)
        .await?
        .and_then(|l| l.recent_minutes);
    let mut tx = crate::db::begin_write(&s.db).await?;
    let stored = sqlx::query(
        "UPDATE steam_links SET recent_minutes = ?, recent_at = ?
         WHERE user_id = ? AND steam_id = ? AND share_hours = 1",
    )
    .bind(minutes.map(|m| m as i64))
    .bind(now())
    .bind(user.to_string())
    .bind(steam_id)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    if stored && let Some(m) = minutes {
        crate::badges::stats::set_max(&mut tx, user, stat::STEAM_FORTNIGHT_SECS, m as i64 * 60)
            .await?;
    }
    tx.commit().await?;
    if !stored {
        return Ok(()); // unlinked or switched off meanwhile
    }
    if minutes.is_some() {
        crate::badges::engine::after(s, &[user]).await;
    }
    if before != minutes.map(|m| m as i64) {
        super::store::tell(s, user).await;
    }
    Ok(())
}

/// Server `server`'s board: its members who share Steam hours and played some in the last two weeks, most first. The
/// same for every member, including anyone a member hides what they're playing from (#438).
pub async fn board(db: &SqlitePool, server: ServerId) -> AppResult<Vec<SteamHoursEntry>> {
    let fresh = crate::db::stamp(chrono::Utc::now() - chrono::Duration::hours(STALE_HOURS));
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT l.user_id, l.recent_minutes FROM steam_links l
         JOIN server_members m ON m.user_id = l.user_id AND m.server_id = ?
         WHERE l.share_hours = 1 AND l.recent_minutes > 0 AND l.recent_at >= ?
         ORDER BY l.recent_minutes DESC, l.user_id LIMIT ?",
    )
    .bind(server.to_string())
    .bind(fresh)
    .bind(BOARD_MAX)
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|(u, m)| {
            Ok(SteamHoursEntry {
                user_id: u.parse::<UserId>().map_err(anyhow::Error::from)?,
                minutes: m.clamp(0, u32::MAX as i64) as u32,
            })
        })
        .collect()
}
