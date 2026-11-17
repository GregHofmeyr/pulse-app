//! The sports poller (#385). Every [`crate::config::SportsConfig::poll_every`], only while the server has a key and
//! someone with pings on follows a team in a server with a Sports channel:
//!
//! 1. Each such league's schedule is read when it's [`crate::config::SportsConfig::schedule_every`] old (about daily):
//!    football the next few days' fixtures, rugby the season's games (one request either way).
//! 2. Matches that kick off within [`crate::config::SportsConfig::banner_lead`] get their match-day card in the Sports
//!    channel of every server where a member follows either team.
//! 3. Matches with a card are read live from kick-off until full time, one request per sport per read, at most every
//!    [`crate::config::SportsConfig::live_every`] and further apart when the day's allowance runs short
//!    ([`super::live_gap`]). A change (a goal, a try, half time, full time) edits each card in place: no new messages.
//! 4. A card whose window passes with no live data to end the match says "no live updates" (#406) and stops; done
//!    matches older than a week are dropped.
//!
//! Each card carries both teams' logos (#406, [`super::logos`]), and only goes to Sports channels that take its league
//! (the owner may limit them).

use std::collections::{BTreeMap, BTreeSet};

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{SportsCard, SportsLeague, SportsPhase, SportsScorer, SportsSide};

use super::api::{IDS_PER_CALL, Match, Sport};
use super::{league_id, phase_of, season_for, sport_of};
use crate::AppState;
use crate::error::AppResult;
use crate::messages::system;

/// Requests a sport keeps back from live reads for the day's schedules.
pub const RESERVE: u32 = 2;
/// The shortest gap between tries after a failed schedule read.
pub const SCHEDULE_RETRY: std::time::Duration = std::time::Duration::from_secs(3600);
/// How long done matches are kept.
pub const KEEP_DAYS: i64 = 7;
/// How far ahead a rugby season's games are kept from a schedule read.
pub const AHEAD_DAYS: i64 = 3;

/// Poll forever (never started without a key).
pub fn spawn(s: AppState) {
    if !s.sports.enabled() {
        return;
    }
    tokio::spawn(async move {
        let mut every = tokio::time::interval(s.cfg.sports.poll_every);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            every.tick().await;
            tick(&s).await;
        }
    });
}

/// A match as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fixture {
    pub league: SportsLeague,
    pub id: u32,
    pub kickoff: i64,
    pub home: (u32, String),
    pub away: (u32, String),
    pub status: String,
    pub home_score: Option<u32>,
    pub away_score: Option<u32>,
    pub minute: Option<u32>,
    pub scorers: Vec<SportsScorer>,
    pub done: bool,
}

type Row = (
    String,
    i64,
    i64,
    i64,
    String,
    i64,
    String,
    String,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    String,
    bool,
);

const COLS: &str = "league, fixture_id, kickoff, home_id, home_name, away_id, away_name, status, home_score, away_score, minute, scorers, done";

fn u32_of(v: i64) -> u32 {
    v.clamp(0, u32::MAX as i64) as u32
}

fn from_row(r: Row) -> Option<Fixture> {
    Some(Fixture {
        league: SportsLeague::parse(&r.0)?,
        id: u32_of(r.1),
        kickoff: r.2,
        home: (u32_of(r.3), r.4),
        away: (u32_of(r.5), r.6),
        status: r.7,
        home_score: r.8.map(u32_of),
        away_score: r.9.map(u32_of),
        minute: r.10.map(u32_of),
        scorers: serde_json::from_str(&r.11).unwrap_or_default(),
        done: r.12,
    })
}

pub async fn load(s: &AppState, league: SportsLeague, id: u32) -> AppResult<Option<Fixture>> {
    let row: Option<Row> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM sports_fixtures WHERE league = ? AND fixture_id = ?"
    ))
    .bind(league.key())
    .bind(id)
    .fetch_optional(&s.db)
    .await?;
    Ok(row.and_then(from_row))
}

/// Matches not done yet, by kick-off.
async fn pending(s: &AppState) -> AppResult<Vec<Fixture>> {
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM sports_fixtures WHERE done = 0 ORDER BY kickoff, fixture_id"
    ))
    .fetch_all(&s.db)
    .await?;
    Ok(rows.into_iter().filter_map(from_row).collect())
}

async fn has_card(s: &AppState, f: &Fixture) -> AppResult<bool> {
    let n: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sports_posts WHERE league = ? AND fixture_id = ?")
            .bind(f.league.key())
            .bind(f.id)
            .fetch_one(&s.db)
            .await?;
    Ok(n > 0)
}

/// One round.
pub async fn tick(s: &AppState) {
    if !s.sports.enabled() {
        return;
    }
    let leagues = match active_leagues(s).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = ?e, "followed sports leagues not read");
            return;
        }
    };
    if leagues.is_empty() {
        return;
    }
    for league in leagues {
        if let Err(e) = schedule(s, league).await {
            tracing::warn!(error = ?e, league = league.key(), "sports schedule not handled");
        }
    }
    if let Err(e) = banners(s).await {
        tracing::warn!(error = ?e, "sports banners not handled");
    }
    for sport in Sport::ALL {
        if let Err(e) = live(s, sport).await {
            tracing::warn!(error = ?e, sport = sport.key(), "live sports not handled");
        }
    }
    if let Err(e) = expire(s).await {
        tracing::warn!(error = ?e, "old sports matches not handled");
    }
}

/// The leagues someone with pings on follows a team in while in a server with a Sports channel that takes that
/// league (#406: its owner may limit them).
pub async fn active_leagues(s: &AppState) -> AppResult<Vec<SportsLeague>> {
    let keys: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT f.league FROM sports_follows f
         LEFT JOIN sports_prefs p ON p.user_id = f.user_id
         JOIN server_members m ON m.user_id = f.user_id
         JOIN server_sports_channels d ON d.server_id = m.server_id
         JOIN servers sv ON sv.id = d.server_id AND sv.deleted_at IS NULL
         JOIN channels c ON c.id = d.channel_id AND c.deleted_at IS NULL AND c.archived_at IS NULL
         WHERE COALESCE(p.pings, 1) = 1
           AND (d.leagues IS NULL OR EXISTS (SELECT 1 FROM json_each(d.leagues) j WHERE j.value = f.league))
         ORDER BY f.league",
    )
    .fetch_all(&s.db)
    .await?;
    Ok(keys.iter().filter_map(|k| SportsLeague::parse(k)).collect())
}

/// Read `league`'s schedule when it's due.
pub async fn schedule(s: &AppState, league: SportsLeague) -> AppResult<()> {
    let at = format!("schedule_at:{}", league.key());
    let tried = format!("schedule_tried:{}", league.key());
    if !super::due(&s.db, &at, s.cfg.sports.schedule_every).await
        || !super::due(&s.db, &tried, SCHEDULE_RETRY).await
    {
        return Ok(());
    }
    super::mark(&s.db, &tried).await?;
    let now = chrono::Utc::now();
    let today = now.date_naive();
    let season = season_for(today);
    let api = s.sports.api();
    let got = if league.rugby() {
        api.games(&s.db, league_id(league), season, None)
            .await
            .map(|games| {
                let (from, to) = (
                    now.timestamp() - 86_400,
                    now.timestamp() + AHEAD_DAYS * 86_400,
                );
                games
                    .into_iter()
                    .filter(|g| (from..=to).contains(&g.kickoff))
                    .collect::<Vec<_>>()
            })
    } else {
        api.fixtures(
            &s.db,
            league_id(league),
            season,
            today,
            today + chrono::Days::new(2),
        )
        .await
    };
    let matches = match got {
        Ok(m) => m,
        Err(e) => {
            tracing::debug!(league = league.key(), error = ?e, "sports schedule not read");
            return Ok(());
        }
    };
    super::mark(&s.db, &at).await?;
    for m in matches {
        if upsert(s, league, &m).await?
            && let Some(f) = load(s, league, m.id).await?
        {
            // a kick-off moved or a match called off: the cards already up say so
            sync_cards(s, &f, false).await?;
        }
    }
    Ok(())
}

/// Store what was read of a match (not once it's done); true when something shown changed.
pub async fn upsert(s: &AppState, league: SportsLeague, m: &Match) -> AppResult<bool> {
    let before = load(s, league, m.id).await?;
    if before.as_ref().is_some_and(|b| b.done) {
        return Ok(false);
    }
    let scorers = m
        .scorers
        .clone()
        .or_else(|| before.as_ref().map(|b| b.scorers.clone()))
        .unwrap_or_default();
    let after = Fixture {
        league,
        id: m.id,
        kickoff: m.kickoff,
        home: (m.home.id, m.home.name.trim().to_string()),
        away: (m.away.id, m.away.name.trim().to_string()),
        status: m.status.clone(),
        home_score: m.home_score,
        away_score: m.away_score,
        minute: m.minute,
        scorers,
        done: false,
    };
    if before.as_ref() == Some(&after) {
        return Ok(false);
    }
    sqlx::query(&format!(
        "INSERT INTO sports_fixtures ({COLS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0)
         ON CONFLICT (league, fixture_id) DO UPDATE SET kickoff = excluded.kickoff, home_id = excluded.home_id,
             home_name = excluded.home_name, away_id = excluded.away_id, away_name = excluded.away_name,
             status = excluded.status, home_score = excluded.home_score, away_score = excluded.away_score,
             minute = excluded.minute, scorers = excluded.scorers
         WHERE done = 0"
    ))
    .bind(league.key())
    .bind(after.id)
    .bind(after.kickoff)
    .bind(after.home.0)
    .bind(&after.home.1)
    .bind(after.away.0)
    .bind(&after.away.1)
    .bind(&after.status)
    .bind(after.home_score)
    .bind(after.away_score)
    .bind(after.minute)
    .bind(serde_json::to_string(&after.scorers).unwrap_or_else(|_| "[]".into()))
    .execute(&s.db)
    .await?;
    Ok(true)
}

async fn set_done(s: &AppState, f: &Fixture) -> sqlx::Result<()> {
    sqlx::query("UPDATE sports_fixtures SET done = 1 WHERE league = ? AND fixture_id = ?")
        .bind(f.league.key())
        .bind(f.id)
        .execute(&s.db)
        .await?;
    Ok(())
}

/// Match-day cards for what kicks off within the banner lead.
pub async fn banners(s: &AppState) -> AppResult<()> {
    let now = chrono::Utc::now().timestamp();
    let lead = s.cfg.sports.banner_lead.as_secs() as i64;
    let window = s.cfg.sports.window.as_secs() as i64;
    for f in pending(s).await? {
        if f.kickoff - lead > now || f.kickoff + window < now {
            continue;
        }
        match phase_of(&f.status) {
            SportsPhase::Off | SportsPhase::Final if !has_card(s, &f).await? => {
                set_done(s, &f).await?
            }
            SportsPhase::Off | SportsPhase::Final => {
                sync_cards(s, &f, false).await?;
                set_done(s, &f).await?;
            }
            _ => sync_cards(s, &f, true).await?,
        }
    }
    Ok(())
}

/// Minutes of match windows left today (UTC) among `fixtures`, overlaps counted once.
pub fn window_minutes_left(fixtures: &[(i64, i64)], now: i64) -> u64 {
    let day_end = (now / 86_400 + 1) * 86_400;
    let mut spans: Vec<(i64, i64)> = fixtures
        .iter()
        .map(|(start, end)| ((*start).max(now), (*end).min(day_end)))
        .filter(|(a, b)| b > a)
        .collect();
    spans.sort();
    let mut total = 0i64;
    let mut cur: Option<(i64, i64)> = None;
    for (a, b) in spans {
        match cur {
            Some((ca, cb)) if a <= cb => cur = Some((ca, cb.max(b))),
            Some((ca, cb)) => {
                total += cb - ca;
                cur = Some((a, b));
            }
            None => cur = Some((a, b)),
        }
    }
    if let Some((a, b)) = cur {
        total += b - a;
    }
    (total.max(0) as u64).div_ceil(60)
}

/// Read `sport`'s live matches once if it's time, and edit their cards.
pub async fn live(s: &AppState, sport: Sport) -> AppResult<()> {
    let cfg = &s.cfg.sports;
    let now = chrono::Utc::now().timestamp();
    let window = cfg.window.as_secs() as i64;
    let max_window = cfg.max_window.as_secs() as i64;
    let mut carded = Vec::new();
    for f in pending(s).await? {
        if sport_of(f.league) == sport && has_card(s, &f).await? {
            carded.push(f);
        }
    }
    let live_now: Vec<&Fixture> = carded
        .iter()
        .filter(|f| {
            f.kickoff <= now
                && (now <= f.kickoff + window
                    || (now <= f.kickoff + max_window && phase_of(&f.status) == SportsPhase::Live))
        })
        .collect();
    if live_now.is_empty() {
        return Ok(());
    }
    let spans: Vec<(i64, i64)> = carded
        .iter()
        .map(|f| (f.kickoff, f.kickoff + window))
        .collect();
    let left = s.sports.api().left_today(&s.db, sport).await;
    let Some(gap) = super::live_gap(
        cfg.live_every,
        window_minutes_left(&spans, now).max(1),
        left.saturating_sub(RESERVE),
    ) else {
        return Ok(());
    };
    if s.sports
        .live_read_at(sport)
        .is_some_and(|t| t.elapsed() < gap)
    {
        return Ok(());
    }
    s.sports.mark_live_read(sport);
    let mut read: Vec<(SportsLeague, Match)> = Vec::new();
    match sport {
        Sport::Football => {
            let ids: Vec<u32> = live_now.iter().map(|f| f.id).collect();
            for chunk in ids.chunks(IDS_PER_CALL) {
                match s.sports.api().fixtures_by_id(&s.db, chunk).await {
                    Ok(got) => {
                        for m in got {
                            if let Some(f) = live_now.iter().find(|f| f.id == m.id) {
                                read.push((f.league, m));
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!(error = ?e, "live football not read");
                        break;
                    }
                }
            }
        }
        Sport::Rugby => {
            // one request per league and day covers every game on it
            let mut days: BTreeMap<(SportsLeague, chrono::NaiveDate), BTreeSet<u32>> =
                BTreeMap::new();
            for f in &live_now {
                if let Some(t) = chrono::DateTime::from_timestamp(f.kickoff, 0) {
                    days.entry((f.league, t.date_naive()))
                        .or_default()
                        .insert(f.id);
                }
            }
            for ((league, day), ids) in days {
                match s
                    .sports
                    .api()
                    .games(&s.db, league_id(league), season_for(day), Some(day))
                    .await
                {
                    Ok(got) => read.extend(
                        got.into_iter()
                            .filter(|m| ids.contains(&m.id))
                            .map(|m| (league, m)),
                    ),
                    Err(e) => {
                        tracing::debug!(error = ?e, "live rugby not read");
                        break;
                    }
                }
            }
        }
    }
    for (league, m) in read {
        let changed = upsert(s, league, &m).await?;
        let Some(f) = load(s, league, m.id).await? else {
            continue;
        };
        if changed {
            sync_cards(s, &f, true).await?;
        }
        if matches!(phase_of(&f.status), SportsPhase::Final | SportsPhase::Off) {
            set_done(s, &f).await?;
        }
    }
    Ok(())
}

/// Whether a carded match has run out of live data (#406): still on match day once its window has passed (no live
/// read ever moved it on), or still not over past its longest window.
pub fn stale(f: &Fixture, now: i64, window: i64, max_window: i64) -> bool {
    match phase_of(&f.status) {
        SportsPhase::Final | SportsPhase::Off => false,
        SportsPhase::Upcoming => now > f.kickoff + window,
        SportsPhase::Live => now > f.kickoff + max_window,
    }
}

/// Matches past their longest window are done (no more reads), and a card left without live data says so (#406);
/// done ones older than [`KEEP_DAYS`] go.
pub async fn expire(s: &AppState) -> AppResult<()> {
    let now = chrono::Utc::now().timestamp();
    let (window, max_window) = (
        s.cfg.sports.window.as_secs() as i64,
        s.cfg.sports.max_window.as_secs() as i64,
    );
    for f in pending(s).await? {
        if stale(&f, now, window, max_window) && has_card(s, &f).await? {
            sync_cards_as(s, &f, false, true).await?;
            set_done(s, &f).await?;
        }
    }
    sqlx::query("UPDATE sports_fixtures SET done = 1 WHERE done = 0 AND kickoff < ?")
        .bind(now - s.cfg.sports.max_window.as_secs() as i64)
        .execute(&s.db)
        .await?;
    let old = now - KEEP_DAYS * 86_400;
    sqlx::query(
        "DELETE FROM sports_posts WHERE (league, fixture_id) IN
             (SELECT league, fixture_id FROM sports_fixtures WHERE done = 1 AND kickoff < ?)",
    )
    .bind(old)
    .execute(&s.db)
    .await?;
    sqlx::query("DELETE FROM sports_fixtures WHERE done = 1 AND kickoff < ?")
        .bind(old)
        .execute(&s.db)
        .await?;
    Ok(())
}

/// The servers' Sports channels that take `f`'s league (#406) where a member with pings on follows either team of
/// `f`, with those members.
pub async fn targets(s: &AppState, f: &Fixture) -> AppResult<Vec<(ChannelId, Vec<UserId>)>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT d.channel_id, f.user_id FROM server_sports_channels d
         JOIN servers sv ON sv.id = d.server_id AND sv.deleted_at IS NULL
         JOIN channels c ON c.id = d.channel_id AND c.deleted_at IS NULL AND c.archived_at IS NULL AND c.kind = 'text'
         JOIN server_members m ON m.server_id = d.server_id
         JOIN sports_follows f ON f.user_id = m.user_id AND f.league = ?1 AND f.team_id IN (?2, ?3)
         LEFT JOIN sports_prefs p ON p.user_id = f.user_id
         WHERE COALESCE(p.pings, 1) = 1
           AND (d.leagues IS NULL OR EXISTS (SELECT 1 FROM json_each(d.leagues) j WHERE j.value = ?1))
         ORDER BY d.channel_id, f.created_at, f.user_id",
    )
    .bind(f.league.key())
    .bind(f.home.0)
    .bind(f.away.0)
    .fetch_all(&s.db)
    .await?;
    let mut out: Vec<(ChannelId, Vec<UserId>)> = Vec::new();
    for (ch, user) in rows {
        let (Ok(ch), Ok(user)) = (ch.parse::<ChannelId>(), user.parse::<UserId>()) else {
            continue;
        };
        match out.last_mut() {
            Some((c, users)) if *c == ch => {
                if !users.contains(&user) {
                    users.push(user);
                }
            }
            _ => out.push((ch, vec![user])),
        }
    }
    Ok(out)
}

/// RFC 3339 (UTC, seconds) for unix seconds.
fn rfc3339(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

/// The card for `f`, with `followers`.
pub fn card(f: &Fixture, followers: Vec<UserId>) -> SportsCard {
    let phase = phase_of(&f.status);
    SportsCard {
        league: f.league,
        fixture_id: f.id,
        phase,
        kickoff: rfc3339(f.kickoff),
        status: f.status.clone(),
        minute: f.minute.filter(|_| phase == SportsPhase::Live),
        home: SportsSide {
            team_id: f.home.0,
            name: f.home.1.clone(),
            score: f.home_score.filter(|_| phase != SportsPhase::Upcoming),
            logo: None,
        },
        away: SportsSide {
            team_id: f.away.0,
            name: f.away.1.clone(),
            score: f.away_score.filter(|_| phase != SportsPhase::Upcoming),
            logo: None,
        },
        scorers: f.scorers.clone(),
        followers,
        no_updates: false,
    }
}

/// The fallback text older apps show for a card.
pub fn content(c: &SportsCard, kickoff_unix: i64) -> String {
    let ball = if c.league.rugby() { "🏉" } else { "⚽" };
    let (h, a) = (&c.home.name, &c.away.name);
    let score = |side: &SportsSide| side.score.map_or("–".to_string(), |n| n.to_string());
    if c.no_updates {
        return match (c.home.score, c.away.score) {
            (Some(_), Some(_)) => format!(
                "{ball} {h} {} – {} {a}: no live updates, check back later",
                score(&c.home),
                score(&c.away)
            ),
            _ => format!("{ball} {h} v {a}: no live updates, check back later"),
        };
    }
    match c.phase {
        SportsPhase::Upcoming => format!(
            "{ball} Match day: {h} v {a}, kick-off {} SAST",
            super::sast(kickoff_unix)
        ),
        SportsPhase::Live => {
            let when = match (c.status.as_str(), c.minute) {
                ("HT", _) => "Half time".to_string(),
                (_, Some(m)) => format!("Live {m}'"),
                _ => "Live".to_string(),
            };
            format!(
                "{ball} {when}: {h} {} – {} {a}",
                score(&c.home),
                score(&c.away)
            )
        }
        SportsPhase::Final => {
            format!(
                "{ball} Full time: {h} {} – {} {a}",
                score(&c.home),
                score(&c.away)
            )
        }
        SportsPhase::Off => {
            let why = match c.status.as_str() {
                "CANC" => "cancelled",
                "ABD" => "abandoned",
                _ => "postponed",
            };
            format!("{ball} {h} v {a}: {why}")
        }
    }
}

/// Bring `f`'s cards up to date: each existing card edited, and (with `create`) a new card in each Sports channel
/// that should have one and doesn't (only while the match is still to come or on).
pub async fn sync_cards(s: &AppState, f: &Fixture, create: bool) -> AppResult<()> {
    sync_cards_as(s, f, create, false).await
}

/// [`sync_cards`], the cards saying they stopped updating when `no_updates` (#406).
pub async fn sync_cards_as(
    s: &AppState,
    f: &Fixture,
    create: bool,
    no_updates: bool,
) -> AppResult<()> {
    let mut targets = targets(s, f).await?;
    // both logos (#406), fetched once (charged to a follower) and remembered after
    let by = targets.iter().find_map(|(_, users)| users.first().copied());
    let (home_logo, away_logo) = (
        super::logos::logo(s, by, f.league, f.home.0).await,
        super::logos::logo(s, by, f.league, f.away.0).await,
    );
    let existing: Vec<(String, String)> = sqlx::query_as(
        "SELECT p.channel_id, p.message_id FROM sports_posts p
         JOIN messages m ON m.id = p.message_id AND m.deleted_at IS NULL
         WHERE p.league = ? AND p.fixture_id = ?",
    )
    .bind(f.league.key())
    .bind(f.id)
    .fetch_all(&s.db)
    .await?;
    let mut posted: Vec<(ChannelId, MessageId)> = existing
        .into_iter()
        .filter_map(|(c, m)| Some((c.parse().ok()?, m.parse().ok()?)))
        .collect();
    // cards whose channel no longer has a follower still get the score (followers empty)
    for (ch, _) in &posted {
        if !targets.iter().any(|(c, _)| c == ch) {
            targets.push((*ch, vec![]));
        }
    }
    let open = matches!(
        phase_of(&f.status),
        SportsPhase::Upcoming | SportsPhase::Live
    );
    for (ch, followers) in targets {
        let mut c = card(f, followers);
        c.home.logo = home_logo.clone();
        c.away.logo = away_logo.clone();
        c.no_updates = no_updates;
        let text = content(&c, f.kickoff);
        let payload = system::sports_payload(&c)?;
        if let Some(i) = posted.iter().position(|(pc, _)| *pc == ch) {
            let (_, id) = posted.swap_remove(i);
            edit(s, id, &text, &payload).await?;
        } else if create
            && open
            && let Err(e) = post(s, f, ch, text, payload).await
        {
            tracing::warn!(error = ?e, "sports card not posted");
        }
    }
    Ok(())
}

async fn edit(
    s: &AppState,
    id: MessageId,
    text: &str,
    payload: &serde_json::Value,
) -> AppResult<()> {
    let r = sqlx::query(
        "UPDATE messages SET content = ?, system_payload = ? WHERE id = ? AND deleted_at IS NULL
             AND (content != ? OR system_payload IS NOT ?)",
    )
    .bind(text)
    .bind(payload.to_string())
    .bind(id.to_string())
    .bind(text)
    .bind(payload.to_string())
    .execute(&s.db)
    .await?;
    if r.rows_affected() > 0
        && let Some(message) = crate::messages::routes::load(&s.db, None, id).await?
    {
        s.hub
            .publish(&s.db, Event::MessageUpdated { message })
            .await;
    }
    Ok(())
}

async fn post(
    s: &AppState,
    f: &Fixture,
    ch: ChannelId,
    text: String,
    payload: serde_json::Value,
) -> AppResult<()> {
    let msg = system::post(s, ch, text, payload).await?;
    sqlx::query(
        "INSERT INTO sports_posts (league, fixture_id, channel_id, message_id) VALUES (?, ?, ?, ?)
         ON CONFLICT (league, fixture_id, channel_id) DO UPDATE SET message_id = excluded.message_id",
    )
    .bind(f.league.key())
    .bind(f.id)
    .bind(ch.to_string())
    .bind(msg.id.to_string())
    .execute(&s.db)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(status: &str) -> Fixture {
        Fixture {
            league: SportsLeague::Psl,
            id: 1,
            kickoff: 1_795_282_200,
            home: (10, "Home FC".into()),
            away: (20, "Away FC".into()),
            status: status.into(),
            home_score: Some(2),
            away_score: Some(1),
            minute: Some(63),
            scorers: vec![],
            done: false,
        }
    }

    #[test]
    fn fallback_text_follows_the_match() {
        let t = |status: &str| {
            let f = fixture(status);
            content(&card(&f, vec![]), f.kickoff)
        };
        assert_eq!(
            t("NS"),
            "⚽ Match day: Home FC v Away FC, kick-off 19:30 SAST"
        );
        assert_eq!(t("2H"), "⚽ Live 63': Home FC 2 – 1 Away FC");
        assert_eq!(t("HT"), "⚽ Half time: Home FC 2 – 1 Away FC");
        assert_eq!(t("FT"), "⚽ Full time: Home FC 2 – 1 Away FC");
        assert_eq!(t("PST"), "⚽ Home FC v Away FC: postponed");
        let mut rugby = fixture("FT");
        rugby.league = SportsLeague::Urc;
        assert!(content(&card(&rugby, vec![]), rugby.kickoff).starts_with("🏉 Full time"));
    }

    #[test]
    fn a_card_left_without_live_data_says_so() {
        let w = 150 * 60;
        let max = 4 * 3600;
        let f = fixture("NS");
        assert!(!stale(&f, f.kickoff + w, w, max), "still in its window");
        assert!(
            stale(&f, f.kickoff + w + 1, w, max),
            "match day, window gone"
        );
        let live = fixture("2H");
        assert!(
            !stale(&live, live.kickoff + w + 1, w, max),
            "live reads go on"
        );
        assert!(stale(&live, live.kickoff + max + 1, w, max));
        for done in ["FT", "PST"] {
            assert!(
                !stale(&fixture(done), f.kickoff + max + 1, w, max),
                "{done}"
            );
        }
        let mut c = card(&f, vec![]);
        c.no_updates = true;
        assert_eq!(
            content(&c, f.kickoff),
            "⚽ Home FC v Away FC: no live updates, check back later"
        );
        let mut c = card(&live, vec![]);
        c.no_updates = true;
        assert_eq!(
            content(&c, f.kickoff),
            "⚽ Home FC 2 – 1 Away FC: no live updates, check back later"
        );
    }

    #[test]
    fn a_card_shows_no_score_before_kickoff_and_no_minute_after() {
        let c = card(&fixture("NS"), vec![]);
        assert_eq!(
            (c.home.score, c.minute, c.kickoff.as_str()),
            (None, None, "2026-11-21T17:30:00Z")
        );
        let c = card(&fixture("FT"), vec![]);
        assert_eq!(
            (c.home.score, c.away.score, c.minute),
            (Some(2), Some(1), None)
        );
    }

    #[test]
    fn window_minutes_count_overlaps_once_and_stop_at_midnight() {
        let day = 1_795_219_200; // 2026-11-21T00:00:00Z
        let now = day + 12 * 3600;
        let m = |h: i64, min: i64| day + h * 3600 + min * 60;
        let w = 150 * 60;
        // 13:30 and 16:00 overlap by 20 minutes; 18:30 overlaps 16:00 by 20 too: 13:30–21:00
        let three = [
            (m(13, 30), m(13, 30) + w),
            (m(16, 0), m(16, 0) + w),
            (m(18, 30), m(18, 30) + w),
        ];
        assert_eq!(window_minutes_left(&three, now), 450);
        // half way through the first
        assert_eq!(window_minutes_left(&three, m(14, 45)), 375);
        // a 22:45 kick-off counts only until midnight
        assert_eq!(window_minutes_left(&[(m(22, 45), m(22, 45) + w)], now), 75);
        assert_eq!(window_minutes_left(&[], now), 0);
    }
}
