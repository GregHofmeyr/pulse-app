//! Steam's rows (#379): links, each person's per-game achievement state, and what Settings is told.

use std::collections::HashSet;

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{SteamLinked, SteamStatus};
use sqlx::{SqliteConnection, SqlitePool};

use super::achievements::{completion_rate, perfect_games};
use crate::AppState;
use crate::db::now;
use crate::error::AppResult;

/// Backfill states as stored.
pub const PENDING: &str = "pending";
pub const PRIVATE: &str = "private";
pub const DONE: &str = "done";

/// One link as the server uses it.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct Link {
    pub user_id: String,
    pub steam_id: String,
    pub persona: Option<String>,
    pub share_achievements: bool,
    pub backfill: String,
    pub private_since: Option<String>,
    pub completion_rate: Option<f64>,
    pub perfect_games: i64,
    pub share_hours: bool,
    pub recent_minutes: Option<i64>,
    pub share_join: bool,
    pub deadlock_matches: bool,
    pub deadlock_rank: bool,
    pub deadlock_squad: bool,
}

const LINK_COLS: &str = "user_id, steam_id, persona, share_achievements, backfill, private_since, completion_rate, perfect_games, share_hours, recent_minutes, share_join, deadlock_matches, deadlock_rank, deadlock_squad";

pub async fn link_of(db: &SqlitePool, user: UserId) -> sqlx::Result<Option<Link>> {
    sqlx::query_as(&format!(
        "SELECT {LINK_COLS} FROM steam_links WHERE user_id = ?"
    ))
    .bind(user.to_string())
    .fetch_optional(db)
    .await
}

/// What `user`'s Settings shows.
pub async fn status(s: &AppState, user: UserId) -> AppResult<SteamStatus> {
    let link = link_of(&s.db, user).await?;
    Ok(SteamStatus {
        linking_available: s.cfg.embeds.public_url.is_some(),
        achievements_available: s.steam.api().has_key(),
        deadlock_available: s.deadlock.api().available(),
        link: link.map(|l| SteamLinked {
            steam_id: l.steam_id,
            persona: l.persona,
            share_achievements: l.share_achievements,
            private: l.private_since.is_some(),
            backfill_done: l.backfill == DONE,
            completion_rate: l.completion_rate.map(|r| r as f32),
            perfect_games: l.perfect_games.max(0) as u32,
            share_hours: l.share_hours,
            recent_minutes: l
                .recent_minutes
                .filter(|_| l.share_hours)
                .map(|m| m.max(0) as u32),
            share_join: l.share_join,
            deadlock_matches: l.deadlock_matches,
            deadlock_rank: l.deadlock_rank,
            deadlock_squad: l.deadlock_squad,
        }),
    })
}

/// Tell `user`'s own sessions their Steam status (Settings follows it).
pub async fn tell(s: &AppState, user: UserId) {
    match status(s, user).await {
        Ok(status) => {
            s.hub
                .publish(
                    &s.db,
                    Event::SteamLinkChanged {
                        user_id: user,
                        status,
                    },
                )
                .await
        }
        Err(e) => tracing::warn!(error = ?e, "Steam status not told"),
    }
}

/// How a link attempt went.
#[derive(Debug, PartialEq, Eq)]
pub enum Linked {
    New,
    Same,
    /// That Steam account is someone else's.
    Taken,
}

/// Link `steam_id` to `user`: refused when it's someone else's; the same account again changes nothing; another
/// account replaces theirs (its state goes, and a backfill starts).
pub async fn link(
    db: &SqlitePool,
    user: UserId,
    steam_id: &str,
    persona: Option<&str>,
) -> AppResult<Linked> {
    let mut tx = crate::db::begin_write(db).await?;
    let owner: Option<String> =
        sqlx::query_scalar("SELECT user_id FROM steam_links WHERE steam_id = ?")
            .bind(steam_id)
            .fetch_optional(&mut *tx)
            .await?;
    match owner {
        Some(o) if o == user.to_string() => {
            if let Some(p) = persona {
                sqlx::query("UPDATE steam_links SET persona = ? WHERE user_id = ?")
                    .bind(p)
                    .bind(user.to_string())
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
            return Ok(Linked::Same);
        }
        Some(_) => return Ok(Linked::Taken),
        None => {}
    }
    clear(&mut tx, user).await?;
    sqlx::query(
        "INSERT INTO steam_links (user_id, steam_id, persona, linked_at) VALUES (?, ?, ?, ?)",
    )
    .bind(user.to_string())
    .bind(steam_id)
    .bind(persona)
    .bind(now())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Linked::New)
}

/// Unlink `user`: the link and every game's state go (posted cards and the awards' records stay); whether there was
/// one.
pub async fn unlink(db: &SqlitePool, user: UserId) -> AppResult<bool> {
    let mut tx = crate::db::begin_write(db).await?;
    let had = clear(&mut tx, user).await?;
    tx.commit().await?;
    Ok(had)
}

async fn clear(conn: &mut SqliteConnection, user: UserId) -> sqlx::Result<bool> {
    let n = sqlx::query("DELETE FROM steam_links WHERE user_id = ?")
        .bind(user.to_string())
        .execute(&mut *conn)
        .await?
        .rows_affected();
    sqlx::query("DELETE FROM steam_achievement_state WHERE user_id = ?")
        .bind(user.to_string())
        .execute(&mut *conn)
        .await?;
    // Deadlock (#390): what was read for the old account (posted cards and the matches seen stay)
    sqlx::query("DELETE FROM deadlock_players WHERE user_id = ?")
        .bind(user.to_string())
        .execute(&mut *conn)
        .await?;
    Ok(n > 0)
}

/// Change the switches `req` names (one left out stays); whether `user` is linked. Turning "Share Steam hours" off
/// forgets the hours read (#383).
pub async fn set_switches(
    db: &SqlitePool,
    user: UserId,
    req: &pulse_protocol::rest::SteamSettingsRequest,
) -> sqlx::Result<bool> {
    Ok(sqlx::query(
        "UPDATE steam_links SET share_achievements = COALESCE(?, share_achievements),
                share_hours = COALESCE(?, share_hours),
                share_join = COALESCE(?, share_join),
                deadlock_matches = COALESCE(?, deadlock_matches),
                deadlock_rank = COALESCE(?, deadlock_rank),
                deadlock_squad = COALESCE(?, deadlock_squad),
                recent_minutes = CASE WHEN ? = 0 THEN NULL ELSE recent_minutes END,
                recent_at = CASE WHEN ? = 0 THEN NULL ELSE recent_at END
         WHERE user_id = ?",
    )
    .bind(req.share_achievements)
    .bind(req.share_hours)
    .bind(req.share_join)
    .bind(req.deadlock_matches)
    .bind(req.deadlock_rank)
    .bind(req.deadlock_squad)
    .bind(req.share_hours)
    .bind(req.share_hours)
    .bind(user.to_string())
    .execute(db)
    .await?
    .rows_affected()
        > 0)
}

/// Steam refused `user`'s game details: marked (once). Whether that's news.
pub async fn mark_private(db: &SqlitePool, user: UserId) -> sqlx::Result<bool> {
    Ok(sqlx::query(
        "UPDATE steam_links SET private_since = ? WHERE user_id = ? AND private_since IS NULL",
    )
    .bind(now())
    .bind(user.to_string())
    .execute(db)
    .await?
    .rows_affected()
        > 0)
}

/// Steam answered for `user` again: unmarked, and a backfill that met the refusal waits again. Whether that's news.
pub async fn clear_private(db: &SqlitePool, user: UserId) -> sqlx::Result<bool> {
    Ok(sqlx::query(
        "UPDATE steam_links SET private_since = NULL,
                backfill = CASE WHEN backfill = 'private' THEN 'pending' ELSE backfill END
         WHERE user_id = ? AND private_since IS NOT NULL",
    )
    .bind(user.to_string())
    .execute(db)
    .await?
    .rows_affected()
        > 0)
}

pub async fn set_backfill(db: &SqlitePool, user: UserId, state: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE steam_links SET backfill = ? WHERE user_id = ?")
        .bind(state)
        .bind(user.to_string())
        .execute(db)
        .await?;
    Ok(())
}

/// What's stored of `user`'s achievements in `app`: the unlocked names and the total.
pub async fn state(
    db: &SqlitePool,
    user: UserId,
    app: u32,
) -> AppResult<Option<(HashSet<String>, u32)>> {
    let row: Option<(String, i64)> = sqlx::query_as(
        "SELECT unlocked, total FROM steam_achievement_state WHERE user_id = ? AND app_id = ?",
    )
    .bind(user.to_string())
    .bind(app)
    .fetch_optional(db)
    .await?;
    let Some((unlocked, total)) = row else {
        return Ok(None);
    };
    let names: HashSet<String> = serde_json::from_str(&unlocked).map_err(anyhow::Error::from)?;
    Ok(Some((names, total.max(0) as u32)))
}

/// Store `user`'s achievements in `app`, only while they're still linked to `steam_id` (an unlink in the meantime
/// wins). Whether it was stored.
pub async fn save_state(
    db: &SqlitePool,
    user: UserId,
    steam_id: &str,
    app: u32,
    unlocked: &HashSet<String>,
    total: u32,
) -> AppResult<bool> {
    let mut names: Vec<&String> = unlocked.iter().collect();
    names.sort();
    let json = serde_json::to_string(&names).map_err(anyhow::Error::from)?;
    let n = sqlx::query(
        "INSERT INTO steam_achievement_state (user_id, app_id, unlocked, total, updated_at)
         SELECT ?1, ?2, ?3, ?4, ?5 WHERE EXISTS
           (SELECT 1 FROM steam_links WHERE user_id = ?1 AND steam_id = ?6)
         ON CONFLICT (user_id, app_id) DO UPDATE SET unlocked = excluded.unlocked,
                                                    total = excluded.total,
                                                    updated_at = excluded.updated_at",
    )
    .bind(user.to_string())
    .bind(app)
    .bind(json)
    .bind(total)
    .bind(now())
    .bind(steam_id)
    .execute(db)
    .await?
    .rows_affected();
    Ok(n > 0)
}

/// Recompute `user`'s completion rate and 100 % games from every stored game, store them, and raise the
/// Completionist counter and set Achievement Hunter's (badges are checked after). Returns the two.
pub async fn recompute(s: &AppState, user: UserId) -> AppResult<(Option<f32>, u32)> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT unlocked, total FROM steam_achievement_state WHERE user_id = ?")
            .bind(user.to_string())
            .fetch_all(&s.db)
            .await?;
    let games: Vec<(u32, u32)> = rows
        .into_iter()
        .map(|(u, t)| {
            let n = serde_json::from_str::<Vec<String>>(&u)
                .map(|v| v.len())
                .unwrap_or(0);
            (n as u32, t.max(0) as u32)
        })
        .collect();
    let rate = completion_rate(&games);
    let perfect = perfect_games(&games);
    let hunter = super::achievements::hunter_percent(&games);
    let mut tx = crate::db::begin_write(&s.db).await?;
    sqlx::query("UPDATE steam_links SET completion_rate = ?, perfect_games = ? WHERE user_id = ?")
        .bind(rate.map(f64::from))
        .bind(perfect)
        .bind(user.to_string())
        .execute(&mut *tx)
        .await?;
    crate::badges::stats::set_max(
        &mut tx,
        user,
        crate::badges::catalogue::stat::STEAM_PERFECT_GAMES,
        perfect as i64,
    )
    .await?;
    // the current rate (progress may go down); the badge's tiers never do
    crate::badges::stats::set(
        &mut tx,
        user,
        crate::badges::catalogue::stat::STEAM_COMPLETION_PCT,
        hunter as i64,
    )
    .await?;
    tx.commit().await?;
    crate::badges::engine::after(s, &[user]).await;
    Ok((rate, perfect))
}

/// [`recompute`] everyone linked, once at start: a badge whose tiers changed (Completionist got tiers) is awarded to
/// those whose stored games already reach it, without waiting for their next unlock.
pub async fn recompute_all(s: &AppState) {
    let users: Vec<String> = match sqlx::query_scalar("SELECT user_id FROM steam_links")
        .fetch_all(&s.db)
        .await
    {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!(error = %e, "Steam links not read for badges");
            return;
        }
    };
    for user in users {
        let Ok(user) = user.parse::<UserId>() else {
            continue;
        };
        if let Err(e) = recompute(s, user).await {
            tracing::warn!(error = ?e, "Steam badges not rechecked");
        }
    }
}

/// `user`'s Steam hours over the last two weeks, as `viewer`'s profile card shows them (#383): while they share hours,
/// once read, and not to someone they hide what they're playing from (#436).
pub async fn profile_hours(
    conn: &mut SqliteConnection,
    user: UserId,
    viewer: UserId,
) -> sqlx::Result<Option<u32>> {
    let m: Option<Option<i64>> = sqlx::query_scalar(
        "SELECT recent_minutes FROM steam_links l WHERE l.user_id = ? AND l.share_hours = 1
           AND NOT EXISTS (SELECT 1 FROM activity_hidden h WHERE h.user_id = l.user_id AND h.target_id = ?)",
    )
    .bind(user.to_string())
    .bind(viewer.to_string())
    .fetch_optional(conn)
    .await?;
    Ok(m.flatten().map(|m| m.max(0) as u32))
}

/// The profile card's Steam stat: the completion rate and 100 % games, while `user` shares achievements and there's
/// a rate to show.
pub async fn profile_stat(
    conn: &mut SqliteConnection,
    user: UserId,
) -> sqlx::Result<(Option<f32>, Option<u32>)> {
    let row: Option<(Option<f64>, i64)> = sqlx::query_as(
        "SELECT completion_rate, perfect_games FROM steam_links
         WHERE user_id = ? AND share_achievements = 1 AND completion_rate IS NOT NULL",
    )
    .bind(user.to_string())
    .fetch_optional(conn)
    .await?;
    Ok(match row {
        Some((Some(rate), perfect)) => (Some(rate as f32), Some(perfect.max(0) as u32)),
        _ => (None, None),
    })
}
