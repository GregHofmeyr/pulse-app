//! Sports score pings (#385): match-day banners, the live score and full time for the teams people follow, in each
//! server's Sports channel, from API-Sports (api-football for the PSL and the Premier League, api-rugby for the URC).
//!
//! The free plan allows 100 requests a day per sport, so every request is budgeted ([`api::Api::take`]):
//!
//! - Each followed league's schedule is read about once a day (one request a league).
//! - A league's team list (for the follow search) is read when someone first searches it and kept a week.
//! - Matches are read live only from kick-off until full time (at most [`crate::config::SportsConfig::window`],
//!   [`crate::config::SportsConfig::max_window`] while still not over), and only for matches with a card: one request
//!   covers every live match of a sport (`/fixtures?ids=` for football, the day's `/games` for rugby), at most every
//!   [`crate::config::SportsConfig::live_every`], further apart when the day's allowance runs short ([`live_gap`]).
//!
//! Without a key (`PULSE_API_SPORTS_KEY`) nothing is ever fetched or posted.
//!
//! - [`api`]: the HTTP client and the daily allowance.
//! - [`teams`]: the cached team lists.
//! - [`poller`]: schedules, banners, live reads and the cards.
//! - [`routes`]: your follows and switch, the team search, each server's Sports channel.

pub mod api;
pub mod logos;
pub mod poller;
pub mod routes;
pub mod teams;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pulse_protocol::rest::{SportsLeague, SportsPhase};

use crate::config::SportsConfig;
use api::Sport;

/// API-Sports' league id for `league`: api-football's 288 (South Africa, Premier Soccer League) and 39 (England,
/// Premier League), api-rugby's 76 (United Rugby Championship).
pub fn league_id(league: SportsLeague) -> u32 {
    match league {
        SportsLeague::Psl => 288,
        SportsLeague::Epl => 39,
        SportsLeague::Urc => 76,
    }
}

pub fn sport_of(league: SportsLeague) -> Sport {
    if league.rugby() {
        Sport::Rugby
    } else {
        Sport::Football
    }
}

/// The season API-Sports files a date under: all three leagues run from about August to June and are named by the
/// year they start in.
pub fn season_for(date: jiff::civil::Date) -> i32 {
    if date.month() >= 7 {
        i32::from(date.year())
    } else {
        i32::from(date.year()) - 1
    }
}

/// Where a match is, from its short status (api-football's and api-rugby's codes).
pub fn phase_of(status: &str) -> SportsPhase {
    match status {
        "TBD" | "NS" => SportsPhase::Upcoming,
        "FT" | "AET" | "PEN" | "AOT" | "AWD" | "AW" | "WO" => SportsPhase::Final,
        "PST" | "POST" | "CANC" | "ABD" => SportsPhase::Off,
        _ => SportsPhase::Live,
    }
}

/// South African time (UTC+2 all year): the fallback text older apps show names kick-off in it.
pub fn sast(unix: i64) -> String {
    jiff::Timestamp::from_second(unix)
        .map(|t| {
            crate::badges::time::SAST
                .to_datetime(t)
                .strftime("%H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// The longest one league's live reads may be apart, given the minutes of match windows left today and the requests
/// left: never closer than `min`, and spread so the day's allowance lasts (`None` = nothing left to spend).
pub fn live_gap(min: Duration, window_minutes_left: u64, requests_left: u32) -> Option<Duration> {
    if requests_left == 0 {
        return None;
    }
    let spread = Duration::from_secs(window_minutes_left * 60 / u64::from(requests_left));
    Some(spread.max(min))
}

#[derive(Clone)]
pub struct Sports {
    inner: Arc<Inner>,
}

struct Inner {
    api: api::Api,
    /// When each sport was last read live.
    live_at: Mutex<HashMap<Sport, Instant>>,
    /// One team list fetch at a time (two people searching at once spend one request).
    teams_lock: tokio::sync::Mutex<()>,
    /// Logo URLs that couldn't be fetched, and when (#406): left alone for [`logos::MISS_RETRY`].
    logo_misses: Mutex<HashMap<String, Instant>>,
}

impl Sports {
    pub fn new(cfg: &SportsConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                api: api::Api::new(cfg),
                live_at: Mutex::default(),
                teams_lock: tokio::sync::Mutex::default(),
                logo_misses: Mutex::default(),
            }),
        }
    }

    pub fn api(&self) -> &api::Api {
        &self.inner.api
    }

    /// The server has an API-Sports key.
    pub fn enabled(&self) -> bool {
        self.inner.api.enabled()
    }

    pub(crate) fn teams_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.inner.teams_lock
    }

    pub fn live_read_at(&self, sport: Sport) -> Option<Instant> {
        let l = self.inner.live_at.lock().unwrap_or_else(|e| e.into_inner());
        l.get(&sport).copied()
    }

    /// Forget when `sport` was last read live, so the next round may read it (tests stepping through a match).
    pub fn forget_live_read(&self, sport: Sport) {
        let mut l = self.inner.live_at.lock().unwrap_or_else(|e| e.into_inner());
        l.remove(&sport);
    }

    /// Whether the logo at `url` failed within [`logos::MISS_RETRY`].
    pub fn logo_missed(&self, url: &str) -> bool {
        let m = self
            .inner
            .logo_misses
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        m.get(url).is_some_and(|t| t.elapsed() < logos::MISS_RETRY)
    }

    pub fn logo_miss(&self, url: &str) {
        let mut m = self
            .inner
            .logo_misses
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // bounded: the teams of three leagues
        if m.len() >= 512 {
            m.retain(|_, t| t.elapsed() < logos::MISS_RETRY);
        }
        m.insert(url.to_string(), Instant::now());
    }

    pub fn mark_live_read(&self, sport: Sport) {
        let mut l = self.inner.live_at.lock().unwrap_or_else(|e| e.into_inner());
        l.insert(sport, Instant::now());
    }
}

/// A marker from `sports_state`.
pub async fn state_get(db: &sqlx::SqlitePool, key: &str) -> Option<String> {
    sqlx::query_scalar("SELECT value FROM sports_state WHERE key = ?")
        .bind(key)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
}

pub async fn state_set(db: &sqlx::SqlitePool, key: &str, value: &str) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO sports_state (key, value) VALUES (?, ?) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(db)
    .await?;
    Ok(())
}

/// Whether the unix-seconds marker `key` is missing or older than `every`.
pub async fn due(db: &sqlx::SqlitePool, key: &str, every: Duration) -> bool {
    let now = jiff::Timestamp::now().as_second();
    state_get(db, key)
        .await
        .and_then(|v| v.parse::<i64>().ok())
        .is_none_or(|at| now - at >= every.as_secs() as i64)
}

/// Set the unix-seconds marker `key` to now.
pub async fn mark(db: &sqlx::SqlitePool, key: &str) -> sqlx::Result<()> {
    state_set(db, key, &jiff::Timestamp::now().as_second().to_string()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seasons_start_in_july() {
        let d = jiff::civil::date;
        assert_eq!(season_for(d(2026, 11, 16)), 2026);
        assert_eq!(season_for(d(2027, 5, 30)), 2026);
        assert_eq!(season_for(d(2027, 7, 1)), 2027);
    }

    #[test]
    fn statuses_map_to_phases() {
        assert_eq!(phase_of("NS"), SportsPhase::Upcoming);
        for live in [
            "1H", "HT", "2H", "ET", "BT", "P", "LIVE", "SUSP", "INT", "INTR", "PT",
        ] {
            assert_eq!(phase_of(live), SportsPhase::Live, "{live}");
        }
        for done in ["FT", "AET", "PEN", "AOT", "AWD", "AW"] {
            assert_eq!(phase_of(done), SportsPhase::Final, "{done}");
        }
        for off in ["PST", "POST", "CANC", "ABD"] {
            assert_eq!(phase_of(off), SportsPhase::Off, "{off}");
        }
    }

    #[test]
    fn kickoff_reads_in_south_african_time() {
        // 2026-11-21T17:30:00Z
        assert_eq!(sast(1_795_282_200), "19:30");
    }

    #[test]
    fn live_reads_spread_when_requests_run_short() {
        let five = Duration::from_secs(300);
        // one match (150 min) and plenty left: every five minutes
        assert_eq!(live_gap(five, 150, 80), Some(five));
        // three back-to-back matches (450 min) with 30 requests left: every 15 minutes
        assert_eq!(live_gap(five, 450, 30), Some(Duration::from_secs(900)));
        assert_eq!(live_gap(five, 150, 0), None);
    }
}
