//! A member's Deadlock section (#390): their rank, last match, most-played heroes, their games with you, and their
//! best duo among the server's members. Each part only while its owner shares it; mate and hero stats are read from
//! deadlock-api at most hourly per person.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{DeadlockDuo, DeadlockHeroLine, DeadlockLastMatch, DeadlockProfile};

use super::api::{HeroStats, MateStats};
use super::assets;
use crate::AppState;
use crate::error::AppResult;

/// How long one person's mate and hero stats are trusted.
pub const STATS_TTL: Duration = Duration::from_secs(3600);
/// How soon a failed read is tried again.
pub const STATS_RETRY: Duration = Duration::from_secs(600);
/// Games together before a duo counts for "best duo".
pub const DUO_MIN_GAMES: u64 = 3;
/// Heroes shown.
pub const TOP_HEROES: usize = 3;
/// How long one read may hold the profile up.
pub const READ_DEADLINE: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub mates: Vec<MateStats>,
    pub heroes: Vec<HeroStats>,
}

#[derive(Default)]
pub struct Cache {
    map: Mutex<HashMap<u32, (Instant, Option<Stats>)>>,
}

impl Cache {
    fn get(&self, account: u32) -> Option<Option<Stats>> {
        let m = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let (at, st) = m.get(&account)?;
        let ttl = if st.is_some() { STATS_TTL } else { STATS_RETRY };
        (at.elapsed() < ttl).then(|| st.clone())
    }

    fn put(&self, account: u32, st: Option<Stats>) {
        let mut m = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if m.len() > 1000 {
            m.retain(|_, (at, _)| at.elapsed() < STATS_TTL);
        }
        m.insert(account, (Instant::now(), st));
    }
}

/// `account`'s mate and hero stats: cached, else read (within [`READ_DEADLINE`]); `None` when unavailable.
pub async fn stats(s: &AppState, account: u32) -> Option<Stats> {
    let cache = s.deadlock.squad_cache();
    if let Some(hit) = cache.get(account) {
        return hit;
    }
    let api = s.deadlock.api();
    let read = async { tokio::join!(api.mate_stats(account), api.hero_stats(account)) };
    let got = match tokio::time::timeout(READ_DEADLINE, read).await {
        Ok((Ok(mates), Ok(heroes))) => Some(Stats { mates, heroes }),
        _ => None,
    };
    cache.put(account, got.clone());
    got
}

/// The best win rate (then the most games) among `mates` with at least [`DUO_MIN_GAMES`] together.
pub fn best_duo(mates: &[(UserId, u64, u64)]) -> Option<(UserId, u64, u64)> {
    mates
        .iter()
        .filter(|(_, _, games)| *games >= DUO_MIN_GAMES)
        .max_by(|a, b| {
            // wins/games compared without floats: a.w * b.g vs b.w * a.g
            (u128::from(a.1) * u128::from(b.2))
                .cmp(&(u128::from(b.1) * u128::from(a.2)))
                .then(a.2.cmp(&b.2))
        })
        .copied()
}

/// What `viewer` sees of `target`'s Deadlock in `server` (both members): nothing when `target` hides what they're
/// playing from `viewer`, as their Steam hours (#438).
pub async fn profile(
    s: &AppState,
    server: ServerId,
    viewer: UserId,
    target: UserId,
) -> AppResult<DeadlockProfile> {
    let mut out = DeadlockProfile::default();
    if !s.deadlock.api().available() || crate::activity_hidden::hides(&s.db, target, viewer).await?
    {
        return Ok(out);
    }
    let link: Option<(String, bool, bool, bool)> = sqlx::query_as(
        "SELECT steam_id, deadlock_matches, deadlock_rank, deadlock_squad FROM steam_links WHERE user_id = ?",
    )
    .bind(target.to_string())
    .fetch_optional(&s.db)
    .await?;
    let Some((steam_id, matches_on, rank_on, squad_on)) = link else {
        return Ok(out);
    };
    let Some(account) = s.deadlock.account_of(&steam_id) else {
        return Ok(out);
    };
    let cat = assets::catalog(s).await;
    if rank_on {
        let badge: Option<i64> = sqlx::query_scalar(
            "SELECT badge FROM deadlock_players WHERE user_id = ? AND account_id = ? AND badge > 0",
        )
        .bind(target.to_string())
        .bind(account)
        .fetch_optional(&s.db)
        .await?
        .flatten();
        if let Some(b) = badge.and_then(|b| u32::try_from(b).ok()) {
            out.rank = Some(assets::rank(s, viewer, cat.as_deref(), b).await);
        }
    }
    if matches_on {
        let last: Option<(i64, i64, i64, i64, bool, String)> = sqlx::query_as(
            "SELECT hero_id, kills, deaths, assists, won, started_at FROM deadlock_matches
             WHERE user_id = ? ORDER BY started_at DESC LIMIT 1",
        )
        .bind(target.to_string())
        .fetch_optional(&s.db)
        .await?;
        if let Some((hero, k, d, a, won, at)) = last {
            let hero = hero.max(0) as u32;
            out.last_match = Some(DeadlockLastMatch {
                hero: cat
                    .as_deref()
                    .map(|c| c.hero_name(hero))
                    .unwrap_or_else(|| format!("Hero {hero}")),
                hero_icon: assets::hero_art(s, viewer, cat.as_deref(), hero, false).await,
                kills: k.max(0) as u32,
                deaths: d.max(0) as u32,
                assists: a.max(0) as u32,
                won,
                started_at: at,
            });
        }
    }
    if !squad_on {
        return Ok(out);
    }
    let Some(st) = stats(s, account).await else {
        return Ok(out);
    };
    let mut heroes = st.heroes.clone();
    heroes.sort_by_key(|h| std::cmp::Reverse(h.matches_played));
    for h in heroes
        .iter()
        .filter(|h| h.matches_played > 0)
        .take(TOP_HEROES)
    {
        out.heroes.push(DeadlockHeroLine {
            hero: cat
                .as_deref()
                .map(|c| c.hero_name(h.hero_id))
                .unwrap_or_else(|| format!("Hero {}", h.hero_id)),
            icon: assets::hero_art(s, viewer, cat.as_deref(), h.hero_id, false).await,
            matches: h.matches_played.min(u32::MAX as u64) as u32,
            wins: h.wins.min(u32::MAX as u64) as u32,
        });
    }
    // the server's members who share squad stats, by account (less anyone hiding their games from `viewer`)
    let members: Vec<(String, String)> = sqlx::query_as(
        "SELECT l.user_id, l.steam_id FROM steam_links l
         JOIN server_members m ON m.user_id = l.user_id AND m.server_id = ?
         WHERE l.deadlock_squad = 1 AND l.user_id != ?
           AND NOT EXISTS (SELECT 1 FROM activity_hidden h WHERE h.user_id = l.user_id AND h.target_id = ?)",
    )
    .bind(server.to_string())
    .bind(target.to_string())
    .bind(viewer.to_string())
    .fetch_all(&s.db)
    .await?;
    let by_account: HashMap<u32, UserId> = members
        .into_iter()
        .filter_map(|(u, sid)| Some((s.deadlock.account_of(&sid)?, u.parse().ok()?)))
        .collect();
    let together: Vec<(UserId, u64, u64)> = st
        .mates
        .iter()
        .filter_map(|m| Some((*by_account.get(&m.mate_id)?, m.wins, m.matches_played)))
        .collect();
    let duo = |(u, w, g): (UserId, u64, u64)| DeadlockDuo {
        user_id: u,
        matches: g.min(u32::MAX as u64) as u32,
        wins: w.min(u32::MAX as u64) as u32,
    };
    if viewer != target {
        out.with_you = together
            .iter()
            .find(|(u, _, g)| *u == viewer && *g > 0)
            .copied()
            .map(duo);
    }
    out.best_duo = best_duo(&together).map(duo);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_best_duo_needs_a_few_games_then_wins_then_games() {
        let (a, b, c) = (UserId::new(), UserId::new(), UserId::new());
        assert_eq!(best_duo(&[]), None);
        assert_eq!(best_duo(&[(a, 2, 2)]), None, "too few games");
        // 3/4 beats 6/10; a tie on rate goes to more games
        assert_eq!(best_duo(&[(a, 6, 10), (b, 3, 4)]).map(|d| d.0), Some(b));
        assert_eq!(best_duo(&[(a, 3, 4), (c, 6, 8)]).map(|d| d.0), Some(c));
    }
}
