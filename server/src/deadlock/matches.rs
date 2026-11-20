//! Post-match cards (#390): linked people who share them have their match history read, every few minutes while
//! they play Deadlock (and for a while after), else about every half hour, within deadlock-api's hourly allowance.
//! Each new match becomes one card per match in the Deadlock channel of every server they're in that has one; friends
//! in the same match end up on the same card. The first read only sets a baseline: old matches are never announced.

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{DeadlockCard, DeadlockMatchCard, DeadlockMatchPlayer};

use super::api::{ApiError, HistoryEntry};
use super::assets::{self, Catalog};
use crate::AppState;
use crate::error::AppResult;
use crate::messages::system;

/// Matches older than this when first seen are recorded but not announced.
pub const FRESH_FOR_SECS: i64 = 24 * 3600;
/// The most new matches announced from one read (the rest are recorded quietly).
pub const MAX_NEW: usize = 3;

/// Which posts a server's Deadlock channel takes.
#[derive(Clone, Copy)]
pub enum Kind {
    Matches,
    Movers,
    Patches,
}

impl Kind {
    fn column(self) -> &'static str {
        match self {
            Kind::Matches => "match_cards",
            Kind::Movers => "rank_movers",
            Kind::Patches => "patch_notes",
        }
    }
}

/// The Deadlock channels taking `kind` (live servers, text channels), with their servers; only those `user` is a
/// member of when given, less those of servers where a member is someone `user` hides their games from (#438).
pub async fn channels(
    s: &AppState,
    kind: Kind,
    user: Option<UserId>,
) -> AppResult<Vec<(String, ChannelId)>> {
    let sql = format!(
        "SELECT d.server_id, d.channel_id FROM server_deadlock_channels d
         JOIN servers sv ON sv.id = d.server_id AND sv.deleted_at IS NULL
         JOIN channels c ON c.id = d.channel_id AND c.deleted_at IS NULL AND c.archived_at IS NULL AND c.kind = 'text'
         WHERE d.{} = 1 AND (?1 IS NULL OR EXISTS (
             SELECT 1 FROM server_members m WHERE m.server_id = d.server_id AND m.user_id = ?1)
             AND NOT {})
         ORDER BY d.server_id",
        kind.column(),
        crate::activity_hidden::hides_in_server("?1", "d.server_id")
    );
    // SAFE: only a fixed column name from `Kind` and `hides_in_server`'s static fragments go in; `user` is bound.
    let rows: Vec<(String, String)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(user.map(|u| u.to_string()))
        .fetch_all(&s.db)
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(sv, ch)| Some((sv, ch.parse().ok()?)))
        .collect())
}

pub async fn display_name(s: &AppState, user: UserId) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT CASE WHEN display_name != '' THEN display_name ELSE username END FROM users WHERE id = ?",
    )
    .bind(user.to_string())
    .fetch_optional(&s.db)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "Someone".into())
}

/// "31:04", "1:02:09".
pub fn duration_text(secs: u32) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The fallback text older clients show for a match card.
pub fn content(card: &DeadlockMatchCard, names: &[String]) -> String {
    let line = |p: &DeadlockMatchPlayer, name: &str| {
        format!(
            "{name} {} as {} · {}/{}/{}",
            if p.won { "won" } else { "lost" },
            p.hero,
            p.kills,
            p.deaths,
            p.assists
        )
    };
    let lines: Vec<String> = card
        .players
        .iter()
        .zip(names)
        .map(|(p, n)| line(p, n))
        .collect();
    format!(
        "⚔️ {} · {}",
        lines.join("; "),
        duration_text(card.duration_s)
    )
}

/// One round: everyone due a read, oldest read first, while the allowance lasts.
pub async fn tick(s: &AppState) {
    let d = &s.deadlock;
    if d.api().backing_off() || d.history_paused() {
        return;
    }
    let rows: Vec<(String, String)> = match sqlx::query_as(
        "SELECT user_id, steam_id FROM steam_links WHERE deadlock_matches = 1",
    )
    .fetch_all(&s.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "Deadlock players not read");
            return;
        }
    };
    let mut due = Vec::new();
    for (user, steam_id) in rows {
        let (Ok(user), Some(account)) = (user.parse::<UserId>(), d.account_of(&steam_id)) else {
            continue;
        };
        let playing = s.hub.playing_of(user);
        if super::is_deadlock(s.steam.app_of(user, playing.as_ref()), playing.as_ref()) {
            d.saw_playing(user);
        }
        let every = if d.recently_played(user) {
            s.cfg.deadlock.active_every
        } else {
            s.cfg.deadlock.idle_every
        };
        let last = d.last_checked(user);
        if last.is_none_or(|t| t.elapsed() >= every) {
            due.push((last, user, account));
        }
    }
    // never read first, then the longest ago
    due.sort_by_key(|(last, _, _)| *last);
    for (_, user, account) in due {
        if !d.api().take_history() {
            break;
        }
        match poll_one(s, user, account).await {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => tracing::warn!(error = ?e, "Deadlock match history not handled"),
        }
    }
}

/// Read `user`'s match history once and announce what's new; false when the round should stop (deadlock-api is down
/// or wants a pause).
pub async fn poll_one(s: &AppState, user: UserId, account: u32) -> AppResult<bool> {
    let history = match s.deadlock.api().match_history(account).await {
        Ok(h) => h,
        Err(ApiError::Limited(wait)) => {
            s.deadlock.pause_history(wait);
            return Ok(false);
        }
        Err(ApiError::Down) => return Ok(false),
        Err(ApiError::Refused(status)) => {
            tracing::debug!(status, "Deadlock match history refused for one player");
            s.deadlock.mark_checked(user);
            return Ok(true);
        }
    };
    s.deadlock.mark_checked(user);
    let player = super::ranks::ensure_player(&s.db, user, account).await?;
    let newest = history.iter().map(|h| h.match_id).max();
    let Some(last) = player.last_match_id else {
        // the baseline: nothing before now is news
        set_last(s, user, account, newest.unwrap_or(0)).await?;
        return Ok(true);
    };
    let last = last.max(0) as u64;
    let mut new: Vec<&HistoryEntry> = history.iter().filter(|h| h.match_id > last).collect();
    if new.is_empty() {
        return Ok(true);
    }
    new.sort_by_key(|h| h.match_id);
    if let Some(n) = newest {
        set_last(s, user, account, n).await?;
    }
    let now = jiff::Timestamp::now().as_second();
    let announce_from = new.len().saturating_sub(MAX_NEW);
    let cat = assets::catalog(s).await;
    for (i, h) in new.into_iter().enumerate() {
        record(s, user, h).await?;
        if i >= announce_from && now - h.start_time <= FRESH_FOR_SECS {
            announce(s, user, h, cat.as_deref()).await?;
        }
    }
    Ok(true)
}

async fn set_last(s: &AppState, user: UserId, account: u32, id: u64) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE deadlock_players SET last_match_id = ? WHERE user_id = ? AND account_id = ?",
    )
    .bind(id as i64)
    .bind(user.to_string())
    .bind(account)
    .execute(&s.db)
    .await?;
    Ok(())
}

fn started_at(h: &HistoryEntry) -> String {
    jiff::Timestamp::from_second(h.start_time)
        .ok()
        .map(crate::db::stamp)
        .unwrap_or_else(crate::db::now)
}

async fn record(s: &AppState, user: UserId, h: &HistoryEntry) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO deadlock_matches
             (user_id, match_id, hero_id, kills, deaths, assists, won, duration_s, rank_delta, badge_after, started_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(user.to_string())
    .bind(h.match_id as i64)
    .bind(h.hero_id)
    .bind(h.player_kills)
    .bind(h.player_deaths)
    .bind(h.player_assists)
    .bind(h.won())
    .bind(h.match_duration_s)
    .bind(h.ranked_delta)
    .bind(h.ranked_display_badge)
    .bind(started_at(h))
    .execute(&s.db)
    .await?;
    Ok(())
}

/// `user`'s line on a card for `h`.
pub async fn player_line(
    s: &AppState,
    user: UserId,
    h: &HistoryEntry,
    cat: Option<&Catalog>,
) -> DeadlockMatchPlayer {
    let shows_rank: bool =
        sqlx::query_scalar("SELECT deadlock_rank FROM steam_links WHERE user_id = ?")
            .bind(user.to_string())
            .fetch_optional(&s.db)
            .await
            .ok()
            .flatten()
            .unwrap_or(false);
    let rank = match h.ranked_display_badge.filter(|b| *b > 0 && shows_rank) {
        Some(b) => Some(assets::rank(s, user, cat, b).await),
        None => None,
    };
    DeadlockMatchPlayer {
        user_id: user,
        hero_id: h.hero_id,
        hero: cat
            .map(|c| c.hero_name(h.hero_id))
            .unwrap_or_else(|| format!("Hero {}", h.hero_id)),
        hero_art: assets::hero_art(s, user, cat, h.hero_id, true).await,
        kills: h.player_kills,
        deaths: h.player_deaths,
        assists: h.player_assists,
        won: h.won(),
        rank_delta: h.ranked_delta.filter(|_| shows_rank),
        rank,
    }
}

/// Post `user`'s match `h` in each of their Deadlock channels: added to the match's card there if a friend's line
/// already made one, else a new card.
pub async fn announce(
    s: &AppState,
    user: UserId,
    h: &HistoryEntry,
    cat: Option<&Catalog>,
) -> AppResult<()> {
    let channels = channels(s, Kind::Matches, Some(user)).await?;
    if channels.is_empty() {
        return Ok(());
    }
    let line = player_line(s, user, h, cat).await;
    for (server, ch) in channels {
        if let Err(e) = post_in(s, &server, ch, h, &line).await {
            tracing::warn!(error = ?e, "Deadlock match card not posted");
        }
    }
    Ok(())
}

async fn names_of(s: &AppState, card: &DeadlockMatchCard) -> Vec<String> {
    let mut names = Vec::new();
    for p in &card.players {
        names.push(display_name(s, p.user_id).await);
    }
    names
}

/// Whether `user` now hides their games from a member of `server` (#438).
async fn hides_in(s: &AppState, user: UserId, server: &str) -> AppResult<bool> {
    // SAFE: `hides_in_server`'s static fragments with two placeholders; both values are bound.
    let sql = format!(
        "SELECT {}",
        crate::activity_hidden::hides_in_server("?2", "?1")
    );
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(server)
        .bind(user.to_string())
        .fetch_one(&s.db)
        .await?)
}

async fn post_in(
    s: &AppState,
    server: &str,
    ch: ChannelId,
    h: &HistoryEntry,
    line: &DeadlockMatchPlayer,
) -> AppResult<()> {
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT p.message_id FROM deadlock_match_posts p
         JOIN messages m ON m.id = p.message_id AND m.deleted_at IS NULL
         WHERE p.match_id = ? AND p.channel_id = ?",
    )
    .bind(h.match_id as i64)
    .bind(ch.to_string())
    .fetch_optional(&s.db)
    .await?;
    if let Some(id) = existing.and_then(|i| i.parse::<MessageId>().ok())
        && let Some(msg) = crate::messages::routes::load(&s.db, None, id).await?
        && let Some(DeadlockCard::Match(mut card)) = msg.deadlock.map(|b| *b)
    {
        if card.players.iter().any(|p| p.user_id == line.user_id) {
            return Ok(());
        }
        // the card goes out again whole: a line whose player now hides their games from someone in this server
        // leaves it rather than reach them (#438)
        let mut kept = Vec::with_capacity(card.players.len() + 1);
        for p in card.players {
            if !hides_in(s, p.user_id, server).await? {
                kept.push(p);
            }
        }
        card.players = kept;
        card.players.push(line.clone());
        let text = content(&card, &names_of(s, &card).await);
        let payload = system::deadlock_payload(&DeadlockCard::Match(card))?;
        sqlx::query("UPDATE messages SET content = ?, system_payload = ? WHERE id = ? AND deleted_at IS NULL")
            .bind(&text)
            .bind(payload.to_string())
            .bind(id.to_string())
            .execute(&s.db)
            .await?;
        if let Some(message) = crate::messages::routes::load(&s.db, None, id).await? {
            s.hub
                .publish(&s.db, Event::MessageUpdated { message })
                .await;
        }
        return Ok(());
    }
    let card = DeadlockMatchCard {
        match_id: h.match_id,
        duration_s: h.match_duration_s,
        players: vec![line.clone()],
    };
    let text = content(&card, &names_of(s, &card).await);
    let msg = system::post(
        s,
        ch,
        text,
        system::deadlock_payload(&DeadlockCard::Match(card))?,
    )
    .await?;
    sqlx::query(
        "INSERT INTO deadlock_match_posts (match_id, channel_id, message_id) VALUES (?, ?, ?)
         ON CONFLICT (match_id, channel_id) DO UPDATE SET message_id = excluded.message_id",
    )
    .bind(h.match_id as i64)
    .bind(ch.to_string())
    .bind(msg.id.to_string())
    .execute(&s.db)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_and_fallback_text() {
        assert_eq!(duration_text(1864), "31:04");
        assert_eq!(duration_text(3729), "1:02:09");
        let p = |won| DeadlockMatchPlayer {
            user_id: UserId::new(),
            hero_id: 1,
            hero: "Infernus".into(),
            hero_art: None,
            kills: 12,
            deaths: 3,
            assists: 8,
            won,
            rank_delta: None,
            rank: None,
        };
        let card = DeadlockMatchCard {
            match_id: 1,
            duration_s: 1864,
            players: vec![p(true), p(false)],
        };
        assert_eq!(
            content(&card, &["alex".into(), "sam".into()]),
            "⚔️ alex won as Infernus · 12/3/8; sam lost as Infernus · 12/3/8 · 31:04"
        );
    }
}
