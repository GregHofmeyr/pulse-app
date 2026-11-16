//! Ranks (#390): everyone linked who shows their rank is read in one batch call about every half hour; the member
//! list and profiles show the stored badge. Once a week the movers (anyone whose rank changed since the last movers)
//! post in each Deadlock channel that takes them.

use std::collections::HashMap;

use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{DeadlockCard, DeadlockMover, DeadlockMoversCard};
use sqlx::SqlitePool;

use super::assets::{self, ladder, rank_text};
use super::matches::{self, Kind};
use crate::AppState;
use crate::db::now;
use crate::error::AppResult;
use crate::messages::system;

/// How often the rank movers post.
pub const MOVERS_EVERY: chrono::Duration = chrono::Duration::days(7);
/// The most movers on one card.
pub const MAX_MOVERS: usize = 10;

/// A person's stored Deadlock row.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct Player {
    pub last_match_id: Option<i64>,
    pub badge: Option<i64>,
}

/// `user`'s row for `account`: made when missing; started over when it was for another account.
pub async fn ensure_player(db: &SqlitePool, user: UserId, account: u32) -> sqlx::Result<Player> {
    sqlx::query(
        "INSERT INTO deadlock_players (user_id, account_id) VALUES (?1, ?2)
         ON CONFLICT (user_id) DO UPDATE SET account_id = excluded.account_id, last_match_id = NULL,
             badge = NULL, rank_at = NULL, week_badge = NULL
         WHERE deadlock_players.account_id != excluded.account_id",
    )
    .bind(user.to_string())
    .bind(account)
    .execute(db)
    .await?;
    sqlx::query_as("SELECT last_match_id, badge FROM deadlock_players WHERE user_id = ?")
        .bind(user.to_string())
        .fetch_one(db)
        .await
}

/// Read everyone's rank (those who show it) and store it.
pub async fn refresh(s: &AppState) -> AppResult<()> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT user_id, steam_id FROM steam_links WHERE deadlock_rank = 1")
            .fetch_all(&s.db)
            .await?;
    let mut by_account: HashMap<u32, UserId> = HashMap::new();
    for (user, steam_id) in rows {
        if let (Ok(u), Some(a)) = (user.parse::<UserId>(), s.deadlock.account_of(&steam_id)) {
            ensure_player(&s.db, u, a).await?;
            by_account.insert(a, u);
        }
    }
    if by_account.is_empty() {
        return Ok(());
    }
    let accounts: Vec<u32> = by_account.keys().copied().collect();
    let Ok(ranks) = s.deadlock.api().ranks(&accounts).await else {
        return Ok(()); // logged by the client; tried again next time
    };
    let at = now();
    for r in ranks {
        let Some(user) = by_account.get(&r.account_id) else {
            continue;
        };
        sqlx::query(
            "UPDATE deadlock_players SET badge = ?1, rank_at = ?2, week_badge = COALESCE(week_badge, ?1)
             WHERE user_id = ?3 AND account_id = ?4",
        )
        .bind(r.badge)
        .bind(&at)
        .bind(user.to_string())
        .bind(r.account_id)
        .execute(&s.db)
        .await?;
    }
    Ok(())
}

/// The stored badges of `server`'s members who show their rank (ranked ones only).
pub async fn of_server(db: &SqlitePool, server: &str) -> sqlx::Result<Vec<(UserId, u32)>> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT p.user_id, p.badge FROM deadlock_players p
         JOIN steam_links l ON l.user_id = p.user_id AND l.deadlock_rank = 1
         JOIN server_members m ON m.user_id = p.user_id AND m.server_id = ?
         WHERE p.badge > 0",
    )
    .bind(server)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(u, b)| Some((u.parse().ok()?, u32::try_from(b).ok()?)))
        .collect())
}

async fn state(db: &SqlitePool, key: &str) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar("SELECT value FROM deadlock_state WHERE key = ?")
        .bind(key)
        .fetch_optional(db)
        .await
}

async fn set_state(db: &SqlitePool, key: &str, value: &str) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO deadlock_state (key, value) VALUES (?, ?) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(db)
    .await?;
    Ok(())
}

/// The weekly rank movers, when a week has passed since the last (the first time only starts the clock).
pub async fn movers_tick(s: &AppState) -> AppResult<()> {
    movers_at(s, chrono::Utc::now()).await
}

/// [`movers_tick`] as if it were `now`.
pub async fn movers_at(s: &AppState, now: chrono::DateTime<chrono::Utc>) -> AppResult<()> {
    let last = state(&s.db, "movers_at")
        .await?
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())
        .map(|t| t.to_utc());
    let Some(last) = last else {
        set_state(&s.db, "movers_at", &crate::db::stamp(now)).await?;
        return Ok(());
    };
    if now - last < MOVERS_EVERY {
        return Ok(());
    }
    // a dark week posts nothing (ranks weren't read) but the week still turns over
    if s.deadlock.api().available() {
        post_movers(s).await?;
    }
    sqlx::query("UPDATE deadlock_players SET week_badge = badge WHERE badge IS NOT NULL")
        .execute(&s.db)
        .await?;
    set_state(&s.db, "movers_at", &crate::db::stamp(now)).await?;
    Ok(())
}

async fn post_movers(s: &AppState) -> AppResult<()> {
    let channels = matches::channels(s, Kind::Movers, None).await?;
    if channels.is_empty() {
        return Ok(());
    }
    let cat = assets::catalog(s).await;
    for (server, ch) in channels {
        let rows: Vec<(String, i64, i64)> = sqlx::query_as(
            "SELECT p.user_id, p.week_badge, p.badge FROM deadlock_players p
             JOIN steam_links l ON l.user_id = p.user_id AND l.deadlock_rank = 1
             JOIN server_members m ON m.user_id = p.user_id AND m.server_id = ?
             WHERE p.badge > 0 AND p.week_badge > 0 AND p.badge != p.week_badge",
        )
        .bind(&server)
        .fetch_all(&s.db)
        .await?;
        let mut moved: Vec<(UserId, u32, u32, i32)> = rows
            .into_iter()
            .filter_map(|(u, from, to)| {
                let (from, to) = (u32::try_from(from).ok()?, u32::try_from(to).ok()?);
                Some((u.parse().ok()?, from, to, ladder(to) - ladder(from)))
            })
            .filter(|m| m.3 != 0)
            .collect();
        if moved.is_empty() {
            continue;
        }
        moved.sort_by_key(|m| std::cmp::Reverse(m.3));
        moved.truncate(MAX_MOVERS);
        let mut movers = Vec::new();
        let mut lines = Vec::new();
        for (user, from, to, steps) in moved {
            let from = assets::rank(s, user, cat.as_deref(), from).await;
            let to = assets::rank(s, user, cat.as_deref(), to).await;
            lines.push(format!(
                "{} {} → {}",
                matches::display_name(s, user).await,
                rank_text(&from),
                rank_text(&to)
            ));
            movers.push(DeadlockMover {
                user_id: user,
                from,
                to,
                steps,
            });
        }
        let card = DeadlockCard::RankMovers(DeadlockMoversCard { movers });
        let text = format!("📈 Deadlock rank movers this week: {}", lines.join(", "));
        if let Err(e) = system::post(s, ch, text, system::deadlock_payload(&card)?).await {
            tracing::warn!(error = ?e, "Deadlock rank movers not posted");
        }
    }
    Ok(())
}
