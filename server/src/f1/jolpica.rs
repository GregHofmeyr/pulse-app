//! Jolpica-F1 (#386), the Ergast successor: free, no key, rate limited (4 calls a second, 500 an hour). Only the
//! server calls it, one call at a time server-wide, at least [`F1Config::min_gap`] apart and at most
//! [`F1Config::hourly_budget`] an hour, and every answer is cached in the database (`f1_cache`), so a restart keeps
//! the cache and a busy chat never turns into a burst of calls. When Jolpica can't answer (or the budget is spent),
//! whatever the cache has answers, however old.
//!
//! What's asked, and how often:
//! - the season's calendar (`current.json`): every [`SCHEDULE_TTL_HOURS`];
//! - a round's results: only from [`RESULTS_AFTER_MINUTES`] after lights out, every few minutes until they're in
//!   ([`results_poll`]), then never again;
//! - the standings after a round: the same way, once the results are in;
//! - the season's entry list (for the first race's predictions): daily.

use std::collections::VecDeque;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use pulse_protocol::rest::{
    F1ConstructorRow, F1Driver, F1FastestLap, F1ResultRow, F1ResultsCard, F1Session, F1SessionKind,
    F1SprintCard, F1StandingRow, F1Weekend, f1_limits,
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::config::F1Config;
use crate::db::stamp;
use crate::error::AppResult;

pub const SCHEDULE_TTL_HOURS: i64 = 6;
pub const ENTRY_LIST_TTL_HOURS: i64 = 24;
/// Results aren't asked for before this long after lights out (a race runs about an hour and a half).
pub const RESULTS_AFTER_MINUTES: i64 = 100;
/// A sprint's results aren't asked for before this long after it starts (a sprint runs about half an hour).
pub const SPRINT_AFTER_MINUTES: i64 = 45;
/// Nor after this long: a race whose results never came is left alone.
pub const RESULTS_GIVE_UP_HOURS: i64 = 48;
/// How long the results card waits for the standings once the results are in, before going without them.
pub const STANDINGS_GRACE_MINUTES: i64 = 60;
const TIMEOUT: Duration = Duration::from_secs(15);

fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent("Pulse (self-hosted chat; F1 race weekends)")
            .build()
            .unwrap_or_default()
    })
}

/// The calls made in the last hour, and when the last one was: one call at a time, spaced out.
#[derive(Default)]
struct Gate {
    calls: VecDeque<Instant>,
}

static GATE: tokio::sync::Mutex<Option<Gate>> = tokio::sync::Mutex::const_new(None);

/// Ask Jolpica for `path` (under the API base, query included): the body when it answered 200 with JSON, else
/// `None`. Waits its turn, and refuses (`None`) once the hour's budget is spent.
async fn call(cfg: &F1Config, path: &str) -> Option<String> {
    let mut gate = GATE.lock().await;
    let gate = gate.get_or_insert_with(Gate::default);
    let hour = Duration::from_secs(3600);
    while gate.calls.front().is_some_and(|t| t.elapsed() >= hour) {
        gate.calls.pop_front();
    }
    if gate.calls.len() >= cfg.hourly_budget {
        tracing::debug!("Jolpica budget for the hour spent; answering from the cache");
        return None;
    }
    if let Some(last) = gate.calls.back() {
        let since = last.elapsed();
        if since < cfg.min_gap {
            tokio::time::sleep(cfg.min_gap - since).await;
        }
    }
    gate.calls.push_back(Instant::now());
    let url = format!("{}/{path}", cfg.api_base.trim_end_matches('/'));
    let r = match http().get(&url).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            tracing::warn!(status = %r.status(), "Jolpica refused");
            return None;
        }
        Err(e) => {
            tracing::warn!(error = %e.without_url(), "Jolpica failed");
            return None;
        }
    };
    let body = r.text().await.ok()?;
    serde_json::from_str::<serde_json::Value>(&body).ok()?;
    Some(body)
}

/// How a cached answer ages.
#[derive(Clone, Copy, Debug)]
pub struct Freshness {
    /// Asked again once it's this old...
    pub ttl: TimeDelta,
    /// ...unless it has rows and this is set: then it's kept for good.
    pub final_when_filled: bool,
}

/// A cached answer.
#[derive(Clone, Debug)]
pub struct Cached {
    pub body: String,
    /// When it first had rows.
    pub filled_at: Option<String>,
}

/// The answer for `key` (Jolpica's `path`): cached if fresh, else asked (then cached), else the cache however old.
/// `filled` says whether an answer has rows.
async fn get(
    db: &SqlitePool,
    cfg: &F1Config,
    key: &str,
    path: &str,
    fresh: Freshness,
    filled: fn(&str) -> bool,
    now: DateTime<Utc>,
) -> AppResult<Option<Cached>> {
    let read = || async {
        sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT body, fetched_at, filled_at FROM f1_cache WHERE key = ?",
        )
        .bind(key)
        .fetch_optional(db)
        .await
    };
    let since = stamp(now - fresh.ttl);
    let usable = |row: &(String, String, Option<String>)| {
        row.1 > since || (fresh.final_when_filled && row.2.is_some())
    };
    let as_cached = |row: (String, String, Option<String>)| Cached {
        body: row.0,
        filled_at: row.2,
    };
    if let Some(row) = read().await?
        && usable(&row)
    {
        return Ok(Some(as_cached(row)));
    }
    let Some(body) = call(cfg, path).await else {
        return Ok(read().await?.map(as_cached));
    };
    let filled_now = filled(&body).then(|| stamp(now));
    sqlx::query(
        "INSERT INTO f1_cache (key, body, fetched_at, filled_at) VALUES (?, ?, ?, ?)
         ON CONFLICT (key) DO UPDATE SET body = excluded.body, fetched_at = excluded.fetched_at,
           filled_at = COALESCE(f1_cache.filled_at, excluded.filled_at)",
    )
    .bind(key)
    .bind(&body)
    .bind(stamp(now))
    .bind(&filled_now)
    .execute(db)
    .await?;
    Ok(read().await?.map(as_cached))
}

// --- Jolpica's shapes (the Ergast JSON), as far as Pulse reads them -------------------------------------------------

#[derive(Deserialize)]
struct Answer<T> {
    #[serde(rename = "MRData")]
    data: T,
}

#[derive(Deserialize)]
struct RaceData {
    #[serde(rename = "RaceTable")]
    table: RaceTable,
}

#[derive(Deserialize)]
struct RaceTable {
    #[serde(rename = "Races", default)]
    races: Vec<Race>,
}

#[derive(Deserialize)]
struct When {
    date: String,
    #[serde(default)]
    time: Option<String>,
}

#[derive(Deserialize)]
struct Location {
    #[serde(default)]
    locality: String,
    #[serde(default)]
    country: String,
}

#[derive(Deserialize)]
struct Circuit {
    #[serde(rename = "circuitName")]
    name: String,
    #[serde(rename = "Location")]
    location: Location,
}

#[derive(Deserialize)]
struct Race {
    season: String,
    round: String,
    #[serde(rename = "raceName")]
    race_name: String,
    #[serde(rename = "Circuit")]
    circuit: Circuit,
    date: String,
    #[serde(default)]
    time: Option<String>,
    #[serde(rename = "FirstPractice")]
    fp1: Option<When>,
    #[serde(rename = "SecondPractice")]
    fp2: Option<When>,
    #[serde(rename = "ThirdPractice")]
    fp3: Option<When>,
    #[serde(rename = "SprintQualifying", alias = "SprintShootout")]
    sprint_qualifying: Option<When>,
    #[serde(rename = "Sprint")]
    sprint: Option<When>,
    #[serde(rename = "Qualifying")]
    qualifying: Option<When>,
    #[serde(rename = "Results", default)]
    results: Vec<RaceResult>,
    /// `{season}/{round}/sprint.json`'s rows (#403): the same shape as `Results`.
    #[serde(rename = "SprintResults", default)]
    sprint_results: Vec<RaceResult>,
}

#[derive(Deserialize)]
struct Driver {
    #[serde(rename = "driverId")]
    id: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(rename = "givenName", default)]
    given: String,
    #[serde(rename = "familyName", default)]
    family: String,
}

#[derive(Deserialize)]
struct Constructor {
    name: String,
}

#[derive(Deserialize)]
struct LapTime {
    time: String,
}

#[derive(Deserialize)]
struct FastestLap {
    #[serde(default)]
    rank: Option<String>,
    #[serde(default)]
    lap: Option<String>,
    #[serde(rename = "Time")]
    time: Option<LapTime>,
}

#[derive(Deserialize)]
struct RaceResult {
    position: String,
    #[serde(default)]
    points: String,
    #[serde(rename = "Driver")]
    driver: Driver,
    #[serde(rename = "Constructor")]
    constructor: Option<Constructor>,
    #[serde(default)]
    status: String,
    #[serde(rename = "Time")]
    time: Option<LapTime>,
    #[serde(rename = "FastestLap")]
    fastest_lap: Option<FastestLap>,
}

#[derive(Deserialize)]
struct StandingsData {
    #[serde(rename = "StandingsTable")]
    table: StandingsTable,
}

#[derive(Deserialize)]
struct StandingsTable {
    #[serde(rename = "StandingsLists", default)]
    lists: Vec<StandingsList>,
}

#[derive(Deserialize)]
struct StandingsList {
    round: String,
    #[serde(rename = "DriverStandings", default)]
    rows: Vec<Standing>,
    /// `constructorStandings.json`'s rows (#403).
    #[serde(rename = "ConstructorStandings", default)]
    constructors: Vec<ConstructorStanding>,
}

#[derive(Deserialize)]
struct ConstructorStanding {
    #[serde(default)]
    position: Option<String>,
    #[serde(default)]
    points: String,
    #[serde(default)]
    wins: String,
    #[serde(rename = "Constructor")]
    constructor: Constructor,
}

#[derive(Deserialize)]
struct Standing {
    #[serde(default)]
    position: Option<String>,
    #[serde(default)]
    points: String,
    #[serde(default)]
    wins: String,
    #[serde(rename = "Driver")]
    driver: Driver,
    #[serde(rename = "Constructors", default)]
    constructors: Vec<Constructor>,
}

#[derive(Deserialize)]
struct DriverData {
    #[serde(rename = "DriverTable")]
    table: DriverTable,
}

#[derive(Deserialize)]
struct DriverTable {
    #[serde(rename = "Drivers", default)]
    drivers: Vec<Driver>,
}

// --- reading them -----------------------------------------------------------------------------------------------

/// Jolpica's date and time ("2026-11-22", "04:00:00Z") as stored (UTC, RFC 3339 with milliseconds).
fn at(date: &str, time: Option<&str>) -> Option<String> {
    let t = time?;
    let t = if t.ends_with('Z') || t.contains('+') {
        t.to_string()
    } else {
        format!("{t}Z")
    };
    let parsed = DateTime::parse_from_rfc3339(&format!("{date}T{t}")).ok()?;
    Some(stamp(parsed.with_timezone(&Utc)))
}

fn driver(d: &Driver, team: Option<&str>) -> F1Driver {
    let name = format!("{} {}", d.given.trim(), d.family.trim());
    F1Driver {
        id: d.id.clone(),
        code: d.code.clone().filter(|c| !c.trim().is_empty()),
        name: name.trim().to_string(),
        team: team.map(str::to_string),
    }
}

/// A race on the calendar as a weekend: `None` when it has no time for lights out.
fn weekend(r: &Race) -> Option<F1Weekend> {
    let race_at = at(&r.date, r.time.as_deref())?;
    let mut sessions: Vec<F1Session> = [
        (F1SessionKind::Practice1, &r.fp1),
        (F1SessionKind::Practice2, &r.fp2),
        (F1SessionKind::Practice3, &r.fp3),
        (F1SessionKind::SprintQualifying, &r.sprint_qualifying),
        (F1SessionKind::Sprint, &r.sprint),
        (F1SessionKind::Qualifying, &r.qualifying),
    ]
    .into_iter()
    .filter_map(|(kind, w)| {
        let w = w.as_ref()?;
        Some(F1Session {
            kind,
            starts_at: at(&w.date, w.time.as_deref())?,
        })
    })
    .collect();
    sessions.sort_by(|a, b| a.starts_at.cmp(&b.starts_at));
    sessions.push(F1Session {
        kind: F1SessionKind::Race,
        starts_at: race_at.clone(),
    });
    Some(F1Weekend {
        season: r.season.parse().ok()?,
        round: r.round.parse().ok()?,
        race_name: r.race_name.clone(),
        circuit: r.circuit.name.clone(),
        locality: r.circuit.location.locality.clone(),
        country: r.circuit.location.country.clone(),
        sessions,
        race_at,
    })
}

/// The weekends in a calendar answer, in round order.
pub fn parse_schedule(body: &str) -> Vec<F1Weekend> {
    let Ok(a) = serde_json::from_str::<Answer<RaceData>>(body) else {
        return vec![];
    };
    let mut out: Vec<F1Weekend> = a.data.table.races.iter().filter_map(weekend).collect();
    out.sort_by_key(|w| w.round);
    out
}

fn races_filled(body: &str) -> bool {
    serde_json::from_str::<Answer<RaceData>>(body)
        .is_ok_and(|a| a.data.table.races.iter().any(|r| !r.results.is_empty()))
}

fn schedule_filled(body: &str) -> bool {
    serde_json::from_str::<Answer<RaceData>>(body).is_ok_and(|a| !a.data.table.races.is_empty())
}

fn standings_filled(body: &str) -> bool {
    serde_json::from_str::<Answer<StandingsData>>(body)
        .is_ok_and(|a| a.data.table.lists.iter().any(|l| !l.rows.is_empty()))
}

fn constructors_filled(body: &str) -> bool {
    serde_json::from_str::<Answer<StandingsData>>(body).is_ok_and(|a| {
        a.data
            .table
            .lists
            .iter()
            .any(|l| !l.constructors.is_empty())
    })
}

fn sprint_filled(body: &str) -> bool {
    serde_json::from_str::<Answer<RaceData>>(body).is_ok_and(|a| {
        a.data
            .table
            .races
            .iter()
            .any(|r| !r.sprint_results.is_empty())
    })
}

fn drivers_filled(body: &str) -> bool {
    serde_json::from_str::<Answer<DriverData>>(body).is_ok_and(|a| !a.data.table.drivers.is_empty())
}

fn number(s: &str) -> f32 {
    s.trim().parse().unwrap_or(0.0)
}

/// A round's results, in finishing order: (every classified driver, as rows; the fastest lap). Empty when they
/// aren't in yet.
pub fn parse_results(body: &str) -> (Vec<F1ResultRow>, Option<F1FastestLap>) {
    let Ok(a) = serde_json::from_str::<Answer<RaceData>>(body) else {
        return (vec![], None);
    };
    let Some(race) = a.data.table.races.into_iter().next() else {
        return (vec![], None);
    };
    result_rows(&race.results)
}

/// A sprint's results (#403), in finishing order. Empty when they aren't in (or it's no sprint weekend).
pub fn parse_sprint(body: &str) -> Vec<F1ResultRow> {
    let Ok(a) = serde_json::from_str::<Answer<RaceData>>(body) else {
        return vec![];
    };
    a.data
        .table
        .races
        .into_iter()
        .next()
        .map(|r| result_rows(&r.sprint_results).0)
        .unwrap_or_default()
}

/// Result rows in finishing order, and the fastest lap among them.
fn result_rows(results: &[RaceResult]) -> (Vec<F1ResultRow>, Option<F1FastestLap>) {
    let mut rows: Vec<(u16, &RaceResult)> = results
        .iter()
        .filter_map(|r| Some((r.position.trim().parse().ok()?, r)))
        .collect();
    rows.sort_by_key(|(p, _)| *p);
    let fastest = results.iter().find_map(|r| {
        let f = r.fastest_lap.as_ref()?;
        (f.rank.as_deref() == Some("1")).then(|| F1FastestLap {
            driver: driver(&r.driver, r.constructor.as_ref().map(|c| c.name.as_str())),
            lap: f.lap.as_deref().and_then(|l| l.trim().parse().ok()),
            time: f.time.as_ref().map(|t| t.time.clone()).unwrap_or_default(),
        })
    });
    let fastest = fastest.filter(|f| !f.time.is_empty());
    let out = rows
        .into_iter()
        .map(|(position, r)| F1ResultRow {
            position,
            driver: driver(&r.driver, r.constructor.as_ref().map(|c| c.name.as_str())),
            time: r
                .time
                .as_ref()
                .map(|t| t.time.clone())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| r.status.clone()),
            points: number(&r.points),
        })
        .collect();
    (out, fastest)
}

/// The drivers' championship in a standings answer for `round` (none when the answer is for another round).
pub fn parse_standings(body: &str, round: u16) -> Vec<F1StandingRow> {
    let Ok(a) = serde_json::from_str::<Answer<StandingsData>>(body) else {
        return vec![];
    };
    let Some(list) = a
        .data
        .table
        .lists
        .into_iter()
        .find(|l| l.round.trim().parse() == Ok(round))
    else {
        return vec![];
    };
    let mut rows: Vec<F1StandingRow> = list
        .rows
        .iter()
        .filter_map(|s| {
            Some(F1StandingRow {
                position: s.position.as_deref()?.trim().parse().ok()?,
                driver: driver(&s.driver, s.constructors.last().map(|c| c.name.as_str())),
                points: number(&s.points),
                wins: s.wins.trim().parse().unwrap_or(0),
            })
        })
        .collect();
    rows.sort_by_key(|r| r.position);
    rows
}

/// The constructors' championship in a standings answer for `round` (#403; none when it's for another round).
pub fn parse_constructor_standings(body: &str, round: u16) -> Vec<F1ConstructorRow> {
    let Ok(a) = serde_json::from_str::<Answer<StandingsData>>(body) else {
        return vec![];
    };
    let Some(list) = a
        .data
        .table
        .lists
        .into_iter()
        .find(|l| l.round.trim().parse() == Ok(round))
    else {
        return vec![];
    };
    let mut rows: Vec<F1ConstructorRow> = list
        .constructors
        .iter()
        .filter_map(|c| {
            Some(F1ConstructorRow {
                position: c.position.as_deref()?.trim().parse().ok()?,
                team: c.constructor.name.trim().to_string(),
                points: number(&c.points),
                wins: c.wins.trim().parse().unwrap_or(0),
            })
        })
        .collect();
    rows.sort_by_key(|r| r.position);
    rows
}

/// The season's entry list, by family name.
pub fn parse_drivers(body: &str) -> Vec<F1Driver> {
    let Ok(a) = serde_json::from_str::<Answer<DriverData>>(body) else {
        return vec![];
    };
    let mut out: Vec<F1Driver> = a
        .data
        .table
        .drivers
        .iter()
        .map(|d| driver(d, None))
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

// --- what the rest of the server asks -----------------------------------------------------------------------------

/// The current season's calendar (empty when Jolpica can't be reached and nothing is cached).
pub async fn schedule(
    db: &SqlitePool,
    cfg: &F1Config,
    now: DateTime<Utc>,
) -> AppResult<Vec<F1Weekend>> {
    let fresh = Freshness {
        ttl: TimeDelta::hours(SCHEDULE_TTL_HOURS),
        final_when_filled: false,
    };
    let c = get(
        db,
        cfg,
        "schedule:current",
        "current.json?limit=100",
        fresh,
        schedule_filled,
        now,
    )
    .await?;
    Ok(c.map(|c| parse_schedule(&c.body)).unwrap_or_default())
}

/// The next race weekend whose lights out is still to come.
pub fn next_weekend(schedule: &[F1Weekend], now: DateTime<Utc>) -> Option<&F1Weekend> {
    let at = stamp(now);
    schedule.iter().find(|w| w.race_at > at)
}

/// How long the results can go unasked, `since_race` after lights out: often at first, then less.
pub fn results_poll(since_race: TimeDelta) -> TimeDelta {
    if since_race < TimeDelta::hours(6) {
        TimeDelta::minutes(5)
    } else {
        TimeDelta::minutes(30)
    }
}

/// A round's results as cached or asked: `None` when they aren't in.
async fn results(
    db: &SqlitePool,
    cfg: &F1Config,
    w: &F1Weekend,
    now: DateTime<Utc>,
) -> AppResult<Option<(Cached, Vec<F1ResultRow>, Option<F1FastestLap>)>> {
    let race_at = race_time(w);
    let fresh = Freshness {
        ttl: results_poll(now - race_at),
        final_when_filled: true,
    };
    let key = format!("results:{}:{}", w.season, w.round);
    let path = format!("{}/{}/results.json?limit=100", w.season, w.round);
    let Some(c) = get(db, cfg, &key, &path, fresh, races_filled, now).await? else {
        return Ok(None);
    };
    let (rows, fastest) = parse_results(&c.body);
    if rows.is_empty() {
        return Ok(None);
    }
    Ok(Some((c, rows, fastest)))
}

/// The results card for a round, once it can go: the results are in, and the standings after it are too (or the
/// grace for them is over). `None` until then.
pub async fn results_card(
    db: &SqlitePool,
    cfg: &F1Config,
    w: &F1Weekend,
    now: DateTime<Utc>,
) -> AppResult<Option<(F1ResultsCard, Vec<String>)>> {
    let Some((c, rows, fastest_lap)) = results(db, cfg, w, now).await? else {
        return Ok(None);
    };
    let fresh = Freshness {
        ttl: results_poll(now - race_time(w)),
        final_when_filled: true,
    };
    let key = format!("standings:{}:{}", w.season, w.round);
    let path = format!("{}/{}/driverStandings.json?limit=100", w.season, w.round);
    let standings = get(db, cfg, &key, &path, fresh, standings_filled, now)
        .await?
        .map(|s| parse_standings(&s.body, w.round))
        .unwrap_or_default();
    // the constructors' table (#403), asked for once the drivers' is in: the card waits for both, within the grace
    let constructors = if standings.is_empty() {
        vec![]
    } else {
        let key = format!("constructors:{}:{}", w.season, w.round);
        let path = format!(
            "{}/{}/constructorStandings.json?limit=100",
            w.season, w.round
        );
        get(db, cfg, &key, &path, fresh, constructors_filled, now)
            .await?
            .map(|s| parse_constructor_standings(&s.body, w.round))
            .unwrap_or_default()
    };
    if standings.is_empty() || constructors.is_empty() {
        let since = c
            .filled_at
            .as_deref()
            .and_then(|f| DateTime::parse_from_rfc3339(f).ok())
            .map(|f| now - f.with_timezone(&Utc))
            .unwrap_or_default();
        if since < TimeDelta::minutes(STANDINGS_GRACE_MINUTES) {
            return Ok(None);
        }
    }
    let podium: Vec<String> = rows
        .iter()
        .take(f1_limits::PODIUM)
        .map(|r| r.driver.id.clone())
        .collect();
    let card = F1ResultsCard {
        season: w.season,
        round: w.round,
        race_name: w.race_name.clone(),
        country: w.country.clone(),
        results: rows.into_iter().take(f1_limits::RESULT_ROWS).collect(),
        fastest_lap,
        standings: standings.into_iter().take(f1_limits::RESULT_ROWS).collect(),
        constructor_standings: constructors,
    };
    Ok(Some((card, podium)))
}

/// When a weekend's sprint starts, if it has one.
pub fn sprint_start(w: &F1Weekend) -> Option<DateTime<Utc>> {
    w.sessions
        .iter()
        .find(|s| s.kind == F1SessionKind::Sprint)
        .and_then(|s| DateTime::parse_from_rfc3339(&s.starts_at).ok())
        .map(|t| t.with_timezone(&Utc))
}

/// The sprint's results card for a round (#403), once they're in: `None` until then. Asked for like the race's
/// results, from [`SPRINT_AFTER_MINUTES`] after the sprint starts, never again once in.
pub async fn sprint_card(
    db: &SqlitePool,
    cfg: &F1Config,
    w: &F1Weekend,
    now: DateTime<Utc>,
) -> AppResult<Option<F1SprintCard>> {
    let Some(start) = sprint_start(w) else {
        return Ok(None);
    };
    let fresh = Freshness {
        ttl: results_poll(now - start),
        final_when_filled: true,
    };
    let key = format!("sprint:{}:{}", w.season, w.round);
    let path = format!("{}/{}/sprint.json?limit=100", w.season, w.round);
    let Some(c) = get(db, cfg, &key, &path, fresh, sprint_filled, now).await? else {
        return Ok(None);
    };
    let rows = parse_sprint(&c.body);
    if rows.is_empty() {
        return Ok(None);
    }
    Ok(Some(F1SprintCard {
        season: w.season,
        round: w.round,
        race_name: w.race_name.clone(),
        country: w.country.clone(),
        results: rows.into_iter().take(f1_limits::RESULT_ROWS).collect(),
    }))
}

/// Who can be picked on a weekend card: the previous round's field (with their teams), or before the first race the
/// season's entry list. Empty when neither can be had right now.
pub async fn field(
    db: &SqlitePool,
    cfg: &F1Config,
    w: &F1Weekend,
    now: DateTime<Utc>,
) -> AppResult<Vec<F1Driver>> {
    if w.round > 1 {
        let prev = w.round - 1;
        let fresh = Freshness {
            ttl: TimeDelta::minutes(30),
            final_when_filled: true,
        };
        let key = format!("results:{}:{prev}", w.season);
        let path = format!("{}/{prev}/results.json?limit=100", w.season);
        if let Some(c) = get(db, cfg, &key, &path, fresh, races_filled, now).await? {
            let (rows, _) = parse_results(&c.body);
            if !rows.is_empty() {
                let mut drivers: Vec<F1Driver> = rows.into_iter().map(|r| r.driver).collect();
                drivers.sort_by(|a, b| a.name.cmp(&b.name));
                return Ok(drivers);
            }
        }
    }
    let fresh = Freshness {
        ttl: TimeDelta::hours(ENTRY_LIST_TTL_HOURS),
        final_when_filled: false,
    };
    let key = format!("drivers:{}", w.season);
    let path = format!("{}/drivers.json?limit=100", w.season);
    Ok(get(db, cfg, &key, &path, fresh, drivers_filled, now)
        .await?
        .map(|c| parse_drivers(&c.body))
        .unwrap_or_default())
}

/// Lights out, read back.
pub fn race_time(w: &F1Weekend) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&w.race_at)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

/// The weekend's first session.
pub fn weekend_start(w: &F1Weekend) -> DateTime<Utc> {
    w.sessions
        .first()
        .and_then(|s| DateTime::parse_from_rfc3339(&s.starts_at).ok())
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|| race_time(w))
}

/// When qualifying starts (lights out when there's none on the calendar).
pub fn qualifying_start(w: &F1Weekend) -> DateTime<Utc> {
    w.sessions
        .iter()
        .find(|s| s.kind == F1SessionKind::Qualifying)
        .and_then(|s| DateTime::parse_from_rfc3339(&s.starts_at).ok())
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|| race_time(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEDULE: &str = r#"{"MRData":{"xmlns":"","series":"f1","limit":"100","offset":"0","total":"2",
      "RaceTable":{"season":"2030","Races":[
        {"season":"2030","round":"2","raceName":"Harbour Grand Prix","Circuit":{"circuitId":"harbour",
          "circuitName":"Harbour Street Circuit","Location":{"lat":"1","long":"2","locality":"Port Town","country":"UK"}},
          "date":"2030-03-15","time":"07:00:00Z","FirstPractice":{"date":"2030-03-13","time":"03:30:00Z"},
          "SprintQualifying":{"date":"2030-03-13","time":"07:30:00Z"},"Sprint":{"date":"2030-03-14","time":"03:00:00Z"},
          "Qualifying":{"date":"2030-03-14","time":"07:00:00Z"}},
        {"season":"2030","round":"1","raceName":"Desert Grand Prix","Circuit":{"circuitId":"desert",
          "circuitName":"Desert Park","Location":{"lat":"1","long":"2","locality":"Dune City","country":"Australia"}},
          "date":"2030-03-08","time":"04:00:00Z","FirstPractice":{"date":"2030-03-06","time":"01:30:00Z"},
          "SecondPractice":{"date":"2030-03-06","time":"05:00:00Z"},"ThirdPractice":{"date":"2030-03-07","time":"01:30:00Z"},
          "Qualifying":{"date":"2030-03-07","time":"05:00:00Z"}},
        {"season":"2030","round":"3","raceName":"No Time Grand Prix","Circuit":{"circuitId":"x",
          "circuitName":"X","Location":{"locality":"X","country":"X"}},"date":"2030-03-22"}
      ]}}}"#;

    #[test]
    fn the_calendar_reads_as_weekends_in_round_order() {
        let w = parse_schedule(SCHEDULE);
        assert_eq!(w.len(), 2, "a race with no time is left out");
        assert_eq!(w[0].round, 1);
        assert_eq!(w[0].race_name, "Desert Grand Prix");
        assert_eq!(w[0].race_at, "2030-03-08T04:00:00.000Z");
        let kinds: Vec<F1SessionKind> = w[0].sessions.iter().map(|s| s.kind).collect();
        use F1SessionKind::*;
        assert_eq!(
            kinds,
            [Practice1, Practice2, Practice3, Qualifying, Race],
            "in time order, the race last"
        );
        let kinds: Vec<F1SessionKind> = w[1].sessions.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            [Practice1, SprintQualifying, Sprint, Qualifying, Race]
        );
        assert_eq!(
            (
                w[1].circuit.as_str(),
                w[1].locality.as_str(),
                w[1].country.as_str()
            ),
            ("Harbour Street Circuit", "Port Town", "UK")
        );
        assert_eq!(parse_schedule("nope"), vec![]);
    }

    #[test]
    fn the_next_weekend_is_the_next_lights_out() {
        let w = parse_schedule(SCHEDULE);
        let t = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        assert_eq!(
            next_weekend(&w, t("2030-03-01T00:00:00Z")).unwrap().round,
            1
        );
        assert_eq!(
            next_weekend(&w, t("2030-03-08T03:59:00Z")).unwrap().round,
            1
        );
        assert_eq!(
            next_weekend(&w, t("2030-03-08T04:00:00Z")).unwrap().round,
            2
        );
        assert!(next_weekend(&w, t("2030-03-16T00:00:00Z")).is_none());
        assert_eq!(weekend_start(&w[0]), t("2030-03-06T01:30:00Z"));
        assert_eq!(qualifying_start(&w[1]), t("2030-03-14T07:00:00Z"));
    }

    const RESULTS: &str = r#"{"MRData":{"RaceTable":{"season":"2030","round":"1","Races":[{"season":"2030","round":"1",
      "raceName":"Desert Grand Prix","Circuit":{"circuitName":"Desert Park","Location":{"locality":"Dune City","country":"Australia"}},
      "date":"2030-03-08","time":"04:00:00Z","Results":[
        {"number":"4","position":"2","positionText":"2","points":"18","Driver":{"driverId":"rider_b","code":"RIB",
          "givenName":"Bo","familyName":"Rider"},"Constructor":{"name":"Comet"},"status":"Finished","Time":{"millis":"1","time":"+5.120"},
          "FastestLap":{"rank":"1","lap":"40","Time":{"time":"1:20.001"}}},
        {"number":"1","position":"1","positionText":"1","points":"25","Driver":{"driverId":"racer_a","code":"RAA",
          "givenName":"Ace","familyName":"Racer"},"Constructor":{"name":"Rocket"},"status":"Finished","Time":{"time":"1:30:00.000"},
          "FastestLap":{"rank":"2","lap":"41","Time":{"time":"1:20.500"}}},
        {"number":"9","position":"3","positionText":"R","points":"0.5","Driver":{"driverId":"slow_c",
          "givenName":"Cy","familyName":"Slow"},"status":"Retired"}
      ]}]}}}"#;

    #[test]
    fn results_read_in_finishing_order_with_the_fastest_lap() {
        let (rows, fastest) = parse_results(RESULTS);
        let ids: Vec<&str> = rows.iter().map(|r| r.driver.id.as_str()).collect();
        assert_eq!(ids, ["racer_a", "rider_b", "slow_c"]);
        assert_eq!(rows[0].time, "1:30:00.000");
        assert_eq!(rows[1].time, "+5.120");
        assert_eq!(rows[2].time, "Retired", "no time: the status");
        assert_eq!(rows[0].driver.name, "Ace Racer");
        assert_eq!(rows[0].driver.team.as_deref(), Some("Rocket"));
        assert_eq!(rows[2].driver.code, None);
        assert!((rows[2].points - 0.5).abs() < 0.01);
        let f = fastest.unwrap();
        assert_eq!(
            (f.driver.id.as_str(), f.lap, f.time.as_str()),
            ("rider_b", Some(40), "1:20.001")
        );
        assert!(races_filled(RESULTS));
        let empty = r#"{"MRData":{"RaceTable":{"Races":[]}}}"#;
        assert!(!races_filled(empty));
        assert_eq!(parse_results(empty).0.len(), 0);
    }

    #[test]
    fn standings_are_read_only_for_their_round() {
        let body = r#"{"MRData":{"StandingsTable":{"season":"2030","round":"1","StandingsLists":[{"season":"2030","round":"1",
          "DriverStandings":[
            {"position":"2","points":"18","wins":"0","Driver":{"driverId":"rider_b","givenName":"Bo","familyName":"Rider"},
             "Constructors":[{"name":"Comet"}]},
            {"position":"1","points":"25","wins":"1","Driver":{"driverId":"racer_a","givenName":"Ace","familyName":"Racer"},
             "Constructors":[{"name":"Rocket"}]},
            {"positionText":"-","points":"0","wins":"0","Driver":{"driverId":"gone","givenName":"N","familyName":"O"}}
          ]}]}}}"#;
        let rows = parse_standings(body, 1);
        assert_eq!(rows.len(), 2, "a row with no position is left out");
        assert_eq!(rows[0].driver.id, "racer_a");
        assert_eq!(
            (rows[0].wins, rows[0].driver.team.as_deref()),
            (1, Some("Rocket"))
        );
        assert!(parse_standings(body, 2).is_empty(), "another round's");
        assert!(standings_filled(body));
    }

    #[test]
    fn constructor_standings_read_for_their_round() {
        // the shape of a real constructorStandings.json, made-up teams
        let body = r#"{"MRData":{"xmlns":"","series":"f1","limit":"30","offset":"0","total":"3","StandingsTable":{
          "season":"2030","round":"2","StandingsLists":[{"season":"2030","round":"2","ConstructorStandings":[
            {"position":"2","positionText":"2","points":"40.5","wins":"0","Constructor":{"constructorId":"comet",
             "url":"https://example.test/comet","name":"Comet","nationality":"British"}},
            {"position":"1","positionText":"1","points":"58","wins":"2","Constructor":{"constructorId":"rocket",
             "name":"Rocket","nationality":"Italian"}},
            {"positionText":"-","points":"0","wins":"0","Constructor":{"constructorId":"gone","name":"Gone"}}
          ]}]}}}"#;
        let rows = parse_constructor_standings(body, 2);
        assert_eq!(rows.len(), 2, "a row with no position is left out");
        assert_eq!(
            (rows[0].position, rows[0].team.as_str(), rows[0].wins),
            (1, "Rocket", 2)
        );
        assert!((rows[1].points - 40.5).abs() < 0.01);
        assert!(parse_constructor_standings(body, 1).is_empty());
        assert!(constructors_filled(body));
        assert!(!constructors_filled(
            r#"{"MRData":{"StandingsTable":{"StandingsLists":[]}}}"#
        ));
        // a drivers' answer has no constructors in it
        assert!(!constructors_filled(
            r#"{"MRData":{"StandingsTable":{"StandingsLists":[{"round":"1","DriverStandings":[]}]}}}"#
        ));
    }

    #[test]
    fn sprint_results_read_in_finishing_order() {
        // the shape of a real sprint.json, made-up drivers
        let body = r#"{"MRData":{"total":"2","RaceTable":{"season":"2030","round":"2","Races":[{"season":"2030",
          "round":"2","raceName":"Harbour Grand Prix","Circuit":{"circuitName":"Harbour Street Circuit",
          "Location":{"locality":"Port Town","country":"UK"}},"date":"2030-03-15","time":"07:00:00Z","SprintResults":[
            {"number":"4","position":"2","positionText":"2","points":"7","Driver":{"driverId":"rider_b","code":"RIB",
             "givenName":"Bo","familyName":"Rider"},"Constructor":{"name":"Comet"},"grid":"2","laps":"19",
             "status":"Finished","Time":{"millis":"1","time":"+4.951"},"FastestLap":{"rank":"1","lap":"16","Time":{"time":"1:24.5"}}},
            {"number":"1","position":"1","positionText":"1","points":"8","Driver":{"driverId":"racer_a","code":"RAA",
             "givenName":"Ace","familyName":"Racer"},"Constructor":{"name":"Rocket"},"status":"Finished",
             "Time":{"time":"26:51.033"}}
          ]}]}}}"#;
        let rows = parse_sprint(body);
        let ids: Vec<&str> = rows.iter().map(|r| r.driver.id.as_str()).collect();
        assert_eq!(ids, ["racer_a", "rider_b"]);
        assert_eq!(rows[0].time, "26:51.033");
        assert!((rows[0].points - 8.0).abs() < 0.01);
        assert!(sprint_filled(body));
        let none =
            r#"{"MRData":{"total":"0","RaceTable":{"season":"2030","round":"1","Races":[]}}}"#;
        assert!(parse_sprint(none).is_empty() && !sprint_filled(none));
        // a race's results aren't a sprint's
        assert!(parse_sprint(RESULTS).is_empty());
        let w = parse_schedule(SCHEDULE);
        assert_eq!(sprint_start(&w[0]), None);
        assert!(sprint_start(&w[1]).is_some());
    }

    #[test]
    fn the_results_are_asked_for_often_at_first_then_less() {
        assert_eq!(results_poll(TimeDelta::minutes(100)), TimeDelta::minutes(5));
        assert_eq!(results_poll(TimeDelta::hours(7)), TimeDelta::minutes(30));
    }

    #[test]
    fn times_read_as_utc() {
        assert_eq!(
            at("2030-03-08", Some("04:00:00Z")).as_deref(),
            Some("2030-03-08T04:00:00.000Z")
        );
        assert_eq!(
            at("2030-03-08", Some("04:00:00")).as_deref(),
            Some("2030-03-08T04:00:00.000Z")
        );
        assert_eq!(at("2030-03-08", None), None);
    }
}
