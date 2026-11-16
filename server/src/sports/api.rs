//! API-Sports' endpoints Pulse reads (#385): api-football v3 (`/teams`, `/fixtures`) and api-rugby v1 (`/teams`,
//! `/games`). One client, server-wide:
//!
//! - The key goes in the `x-apisports-key` header, never in a URL or a log line. Without one nothing is ever called.
//! - Every request first takes one from its sport's allowance for the UTC day ([`Api::take`]), kept in the database so
//!   a restart doesn't hand out a fresh one: the free plan allows 100 a day per sport and the server stops at
//!   [`crate::config::SportsConfig::daily_cap`].
//! - API-Sports answers most failures with HTTP 200 and an `errors` object (`{"requests": "You have reached the
//!   request limit for the day"}`) instead of the success's empty `[]`: that's read before `response`. The day's limit
//!   spends the rest of the day's allowance; anything else it refuses (the key, the plan's seasons) backs the sport off
//!   for half an hour and leaves a note for Settings.
//! - 429, 5xx and the network failing back the sport off, doubling up to an hour.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use pulse_protocol::rest::SportsScorer;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use sqlx::SqlitePool;

use crate::config::SportsConfig;

/// The longest the client waits between tries.
pub const MAX_BACKOFF: Duration = Duration::from_secs(3600);
/// How long a refusal (`errors` other than the day's limit) backs a sport off.
pub const REFUSED_PAUSE: Duration = Duration::from_secs(1800);
/// The most fixtures one `/fixtures?ids=` call takes.
pub const IDS_PER_CALL: usize = 20;

/// Which API (and allowance) a league is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sport {
    Football,
    Rugby,
}

impl Sport {
    pub const ALL: [Sport; 2] = [Sport::Football, Sport::Rugby];

    pub fn key(self) -> &'static str {
        match self {
            Sport::Football => "football",
            Sport::Rugby => "rugby",
        }
    }

    fn index(self) -> usize {
        match self {
            Sport::Football => 0,
            Sport::Rugby => 1,
        }
    }
}

/// Why a call gave nothing.
#[derive(Debug, PartialEq, Eq)]
pub enum ApiError {
    /// No key: sports aren't set up on this server.
    Off,
    /// The day's allowance for this sport is spent.
    Spent,
    /// API-Sports is failing or the sport is backing off: try again later, quietly.
    Down,
    /// API-Sports refused the request (its `errors`): the key, the plan, a parameter.
    Refused(String),
}

#[derive(Default)]
struct Health {
    fails: u32,
    until: Option<Instant>,
}

pub struct Api {
    http: reqwest::Client,
    football_base: String,
    rugby_base: String,
    keyed: bool,
    cap: u32,
    retry_base: Duration,
    health: [Mutex<Health>; 2],
    /// A line for Settings while something's wrong (cleared by the next answer).
    note: Mutex<Option<String>>,
}

/// A team (`/teams`: api-football nests it under `team`, api-rugby doesn't).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Team {
    pub id: u32,
    pub name: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum TeamItem {
    Nested { team: Team },
    Flat(Team),
}

/// A match as Pulse keeps it, from either API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub id: u32,
    /// Unix seconds.
    pub kickoff: i64,
    /// The short status (`NS`, `1H`, `FT`, …).
    pub status: String,
    /// The minute while live (football's `elapsed`).
    pub minute: Option<u32>,
    pub home: Team,
    pub away: Team,
    pub home_score: Option<u32>,
    pub away_score: Option<u32>,
    /// The goals, when the answer carried events (`/fixtures?ids=` does); `None` = not in this answer.
    pub scorers: Option<Vec<SportsScorer>>,
}

// api-football's fixture (`/fixtures`): `fixture`, `league`, `teams`, `goals`, `score`, and `events` on id lookups.
#[derive(Deserialize)]
struct FxItem {
    fixture: FxFixture,
    teams: Sides,
    goals: Score,
    #[serde(default)]
    events: Option<Vec<FxEvent>>,
}

#[derive(Deserialize)]
struct FxFixture {
    id: u32,
    timestamp: i64,
    status: FxStatus,
}

#[derive(Deserialize)]
struct FxStatus {
    short: String,
    #[serde(default)]
    elapsed: Option<u32>,
}

#[derive(Deserialize)]
struct Sides {
    home: Team,
    away: Team,
}

#[derive(Deserialize)]
struct Score {
    #[serde(default)]
    home: Option<u32>,
    #[serde(default)]
    away: Option<u32>,
}

#[derive(Deserialize)]
struct FxEvent {
    time: FxTime,
    team: FxId,
    #[serde(default)]
    player: Option<FxPlayer>,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    detail: Option<String>,
}

#[derive(Deserialize)]
struct FxTime {
    #[serde(default)]
    elapsed: Option<u32>,
    #[serde(default)]
    extra: Option<u32>,
}

#[derive(Deserialize)]
struct FxId {
    #[serde(default)]
    id: Option<u32>,
}

#[derive(Deserialize)]
struct FxPlayer {
    #[serde(default)]
    name: Option<String>,
}

// api-rugby's game (`/games`): `id`, `date`, `timestamp`, `status`, `teams`, `scores`, `periods`.
#[derive(Deserialize)]
struct RgItem {
    id: u32,
    timestamp: i64,
    status: RgStatus,
    teams: Sides,
    scores: Score,
}

#[derive(Deserialize)]
struct RgStatus {
    short: String,
}

impl FxItem {
    fn into_match(self) -> Match {
        let home_id = self.teams.home.id;
        let scorers = self.events.map(|events| {
            events
                .into_iter()
                .filter(|e| e.kind == "Goal")
                .filter(|e| e.detail.as_deref() != Some("Missed Penalty"))
                .filter_map(|e| {
                    let name = e.player.and_then(|p| p.name)?;
                    let note = match e.detail.as_deref() {
                        Some("Own Goal") => Some("og".to_string()),
                        Some("Penalty") => Some("pen".to_string()),
                        _ => None,
                    };
                    Some(SportsScorer {
                        // the event's team as given (an own goal isn't moved: the payload is taken at its word)
                        home: e.team.id == Some(home_id),
                        name,
                        minute: e.time.elapsed.map(|m| m + e.time.extra.unwrap_or(0)),
                        note,
                    })
                })
                .collect()
        });
        Match {
            id: self.fixture.id,
            kickoff: self.fixture.timestamp,
            status: self.fixture.status.short,
            minute: self.fixture.status.elapsed,
            home: self.teams.home,
            away: self.teams.away,
            home_score: self.goals.home,
            away_score: self.goals.away,
            scorers,
        }
    }
}

impl RgItem {
    fn into_match(self) -> Match {
        Match {
            id: self.id,
            kickoff: self.timestamp,
            status: self.status.short,
            minute: None,
            home: self.teams.home,
            away: self.teams.away,
            home_score: self.scores.home,
            away_score: self.scores.away,
            scorers: None,
        }
    }
}

/// What an `errors` field says: `None` on success (`[]`, `{}` or absent).
fn errors_of(v: &serde_json::Value) -> Option<Vec<(String, String)>> {
    let obj = v.as_object()?;
    if obj.is_empty() {
        return None;
    }
    Some(
        obj.iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
            .collect(),
    )
}

/// The UTC day the allowance counts.
pub fn quota_day() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

impl Api {
    pub fn new(cfg: &SportsConfig) -> Self {
        let mut headers = HeaderMap::new();
        if let Some(key) = &cfg.api_key
            && let Ok(mut v) = HeaderValue::from_str(key)
        {
            v.set_sensitive(true);
            headers.insert("x-apisports-key", v);
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!(
                "pulse-server/",
                env!("CARGO_PKG_VERSION"),
                " (self-hosted group chat; sports score cards)"
            ))
            .default_headers(headers)
            .build()
            .expect("the API-Sports HTTP client builds");
        Self {
            http,
            football_base: cfg.football_base.trim_end_matches('/').to_string(),
            rugby_base: cfg.rugby_base.trim_end_matches('/').to_string(),
            keyed: cfg.api_key.is_some(),
            cap: cfg.daily_cap,
            retry_base: cfg.retry_base,
            health: Default::default(),
            note: Mutex::default(),
        }
    }

    /// The server has a key: sports are set up.
    pub fn enabled(&self) -> bool {
        self.keyed
    }

    pub fn note(&self) -> Option<String> {
        self.note.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set_note(&self, note: Option<String>) {
        *self.note.lock().unwrap_or_else(|e| e.into_inner()) = note;
    }

    /// `sport` is between tries: nothing should be called now.
    pub fn backing_off(&self, sport: Sport) -> bool {
        let h = self.health[sport.index()]
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        h.until.is_some_and(|u| Instant::now() < u)
    }

    /// Requests made today for `sport`.
    pub async fn used_today(&self, db: &SqlitePool, sport: Sport) -> u32 {
        sqlx::query_scalar::<_, i64>("SELECT used FROM sports_quota WHERE sport = ? AND day = ?")
            .bind(sport.key())
            .bind(quota_day())
            .fetch_optional(db)
            .await
            .ok()
            .flatten()
            .map_or(0, |n| n.clamp(0, u32::MAX as i64) as u32)
    }

    /// Requests left today for `sport`.
    pub async fn left_today(&self, db: &SqlitePool, sport: Sport) -> u32 {
        self.cap.saturating_sub(self.used_today(db, sport).await)
    }

    /// Take one request from today's allowance for `sport`; false when it's spent (or the database can't say).
    pub async fn take(&self, db: &SqlitePool, sport: Sport) -> bool {
        let r = sqlx::query(
            "INSERT INTO sports_quota (sport, day, used) VALUES (?1, ?2, 1)
             ON CONFLICT (sport, day) DO UPDATE SET used = used + 1 WHERE used < ?3",
        )
        .bind(sport.key())
        .bind(quota_day())
        .bind(self.cap as i64)
        .execute(db)
        .await;
        match r {
            Ok(r) => r.rows_affected() == 1 && self.cap > 0,
            Err(e) => {
                tracing::warn!(error = %e, "sports allowance not counted: no request made");
                false
            }
        }
    }

    /// Spend the rest of today's allowance (API-Sports said the day's limit is reached).
    async fn spend_day(&self, db: &SqlitePool, sport: Sport) {
        let r = sqlx::query(
            "INSERT INTO sports_quota (sport, day, used) VALUES (?1, ?2, ?3)
             ON CONFLICT (sport, day) DO UPDATE SET used = MAX(used, ?3)",
        )
        .bind(sport.key())
        .bind(quota_day())
        .bind(self.cap as i64)
        .execute(db)
        .await;
        if let Err(e) = r {
            tracing::warn!(error = %e, "sports allowance not marked spent");
        }
    }

    fn succeeded(&self, sport: Sport) {
        *self.health[sport.index()]
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Health::default();
        self.set_note(None);
    }

    fn failed(&self, sport: Sport, what: &str, why: &str, wait: Option<Duration>) {
        let mut h = self.health[sport.index()]
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        h.fails = h.fails.saturating_add(1);
        let wait = wait
            .unwrap_or_else(|| {
                self.retry_base * 2u32.saturating_pow(h.fails.saturating_sub(1).min(7))
            })
            .min(MAX_BACKOFF);
        h.until = Some(Instant::now() + wait);
        if h.fails == 3 {
            tracing::warn!(
                sport = sport.key(),
                what,
                why,
                "API-Sports isn't answering: scores paused (retrying with backoff)"
            );
        } else {
            tracing::debug!(
                sport = sport.key(),
                what,
                why,
                fails = h.fails,
                "API-Sports call failed"
            );
        }
    }

    /// GET `path` with `query` on `sport`'s API and read `response[]`, item by item (an item that doesn't read is
    /// skipped, not the whole answer).
    async fn get<T: DeserializeOwned>(
        &self,
        db: &SqlitePool,
        sport: Sport,
        what: &'static str,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Vec<T>, ApiError> {
        if !self.keyed {
            return Err(ApiError::Off);
        }
        if self.backing_off(sport) {
            return Err(ApiError::Down);
        }
        if !self.take(db, sport).await {
            return Err(ApiError::Spent);
        }
        let base = match sport {
            Sport::Football => &self.football_base,
            Sport::Rugby => &self.rugby_base,
        };
        let r = match self
            .http
            .get(format!("{base}{path}"))
            .query(query)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                self.failed(sport, what, &e.without_url().to_string(), None);
                return Err(ApiError::Down);
            }
        };
        let status = r.status().as_u16();
        if status == 429 || status >= 500 {
            self.failed(sport, what, &format!("HTTP {status}"), None);
            return Err(ApiError::Down);
        }
        if status != 200 {
            self.failed(sport, what, &format!("HTTP {status}"), Some(REFUSED_PAUSE));
            self.set_note(Some(
                "API-Sports refused the server's requests: scores are paused.".into(),
            ));
            return Err(ApiError::Refused(format!("HTTP {status}")));
        }
        let body: serde_json::Value = match r.json().await {
            Ok(b) => b,
            Err(e) => {
                self.failed(
                    sport,
                    what,
                    &format!("unreadable answer: {}", e.without_url()),
                    None,
                );
                return Err(ApiError::Down);
            }
        };
        if let Some(errors) = body.get("errors").and_then(errors_of) {
            return Err(self.refused(db, sport, what, &errors).await);
        }
        self.succeeded(sport);
        let items = body
            .get("response")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(items
            .into_iter()
            .filter_map(|v| match serde_json::from_value(v) {
                Ok(t) => Some(t),
                Err(e) => {
                    tracing::debug!(what, error = %e, "an API-Sports item didn't read: skipped");
                    None
                }
            })
            .collect())
    }

    /// An `errors` answer: the day's limit spends the day; the per-minute limit backs off a little; anything else
    /// (key, plan, parameters) backs off for [`REFUSED_PAUSE`] with a note. Messages are logged, the key never is.
    async fn refused(
        &self,
        db: &SqlitePool,
        sport: Sport,
        what: &str,
        errors: &[(String, String)],
    ) -> ApiError {
        let keys: Vec<&str> = errors.iter().map(|(k, _)| k.as_str()).collect();
        let text = errors
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join("; ");
        if keys.contains(&"requests") {
            self.spend_day(db, sport).await;
            self.set_note(Some(
                "Today's sports requests are used up: scores resume tomorrow.".into(),
            ));
            tracing::info!(
                sport = sport.key(),
                "API-Sports' daily limit reached: no more requests today"
            );
            return ApiError::Spent;
        }
        if keys.contains(&"rateLimit") {
            self.failed(sport, what, &text, Some(Duration::from_secs(120)));
            return ApiError::Down;
        }
        self.failed(sport, what, &text, Some(REFUSED_PAUSE));
        let note = if keys.contains(&"plan") {
            "The server's API-Sports plan can't read this season: scores are paused."
        } else if keys.iter().any(|k| *k == "token" || *k == "access") {
            "API-Sports refused the server's key: scores are paused."
        } else {
            "API-Sports refused a request: scores are paused for a while."
        };
        self.set_note(Some(note.into()));
        tracing::warn!(sport = sport.key(), what, errors = %text, "API-Sports refused a request");
        ApiError::Refused(text)
    }

    /// A league's teams for a season (`/teams?league=&season=`).
    pub async fn teams(
        &self,
        db: &SqlitePool,
        sport: Sport,
        league: u32,
        season: i32,
    ) -> Result<Vec<Team>, ApiError> {
        let items: Vec<TeamItem> = self
            .get(
                db,
                sport,
                "teams",
                "/teams",
                &[
                    ("league", league.to_string()),
                    ("season", season.to_string()),
                ],
            )
            .await?;
        Ok(items
            .into_iter()
            .map(|t| match t {
                TeamItem::Nested { team } | TeamItem::Flat(team) => team,
            })
            .collect())
    }

    /// api-football: a league's fixtures from `from` to `to` (`/fixtures?league=&season=&from=&to=`).
    pub async fn fixtures(
        &self,
        db: &SqlitePool,
        league: u32,
        season: i32,
        from: chrono::NaiveDate,
        to: chrono::NaiveDate,
    ) -> Result<Vec<Match>, ApiError> {
        let items: Vec<FxItem> = self
            .get(
                db,
                Sport::Football,
                "fixtures",
                "/fixtures",
                &[
                    ("league", league.to_string()),
                    ("season", season.to_string()),
                    ("from", from.to_string()),
                    ("to", to.to_string()),
                ],
            )
            .await?;
        Ok(items.into_iter().map(FxItem::into_match).collect())
    }

    /// api-football: up to [`IDS_PER_CALL`] fixtures by id, with their events (`/fixtures?ids=1-2-3`).
    pub async fn fixtures_by_id(
        &self,
        db: &SqlitePool,
        ids: &[u32],
    ) -> Result<Vec<Match>, ApiError> {
        let ids = ids
            .iter()
            .take(IDS_PER_CALL)
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join("-");
        let items: Vec<FxItem> = self
            .get(
                db,
                Sport::Football,
                "fixtures-live",
                "/fixtures",
                &[("ids", ids)],
            )
            .await?;
        Ok(items.into_iter().map(FxItem::into_match).collect())
    }

    /// api-rugby: a league's games this season, or on one day (`/games?league=&season=[&date=]`).
    pub async fn games(
        &self,
        db: &SqlitePool,
        league: u32,
        season: i32,
        date: Option<chrono::NaiveDate>,
    ) -> Result<Vec<Match>, ApiError> {
        let mut query = vec![
            ("league", league.to_string()),
            ("season", season.to_string()),
        ];
        if let Some(d) = date {
            query.push(("date", d.to_string()));
        }
        let items: Vec<RgItem> = self
            .get(
                db,
                Sport::Rugby,
                if date.is_some() {
                    "games-live"
                } else {
                    "games"
                },
                "/games",
                &query,
            )
            .await?;
        Ok(items.into_iter().map(RgItem::into_match).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_read_as_empty_on_success() {
        assert_eq!(errors_of(&serde_json::json!([])), None);
        assert_eq!(errors_of(&serde_json::json!({})), None);
        assert_eq!(
            errors_of(
                &serde_json::json!({"requests": "You have reached the request limit for the day"})
            ),
            Some(vec![(
                "requests".into(),
                "You have reached the request limit for the day".into()
            )])
        );
    }

    #[test]
    fn a_fixture_reads_with_its_goals() {
        let v = serde_json::json!({
            "fixture": {"id": 1_200_001, "referee": null, "timezone": "UTC", "date": "2026-11-21T17:30:00+00:00",
                "timestamp": 1_795_282_200, "periods": {"first": 1_795_282_200, "second": null},
                "venue": {"id": 1, "name": "Stadium", "city": "Durban"},
                "status": {"long": "Second Half", "short": "2H", "elapsed": 63, "extra": null}},
            "league": {"id": 288, "name": "Premier Soccer League", "country": "South-Africa", "season": 2026},
            "teams": {"home": {"id": 10, "name": "Home FC", "logo": "x", "winner": null},
                      "away": {"id": 20, "name": "Away FC", "logo": "y", "winner": null}},
            "goals": {"home": 2, "away": 1},
            "score": {"halftime": {"home": 1, "away": 0}, "fulltime": {"home": null, "away": null}},
            "events": [
                {"time": {"elapsed": 41, "extra": null}, "team": {"id": 10, "name": "Home FC"},
                 "player": {"id": 1, "name": "A. Striker"}, "assist": {"id": null, "name": null},
                 "type": "Goal", "detail": "Penalty", "comments": null},
                {"time": {"elapsed": 45, "extra": 2}, "team": {"id": 10, "name": "Home FC"},
                 "player": {"id": 2, "name": "B. Back"}, "assist": {"id": null, "name": null},
                 "type": "Card", "detail": "Yellow Card", "comments": null},
                {"time": {"elapsed": 50, "extra": null}, "team": {"id": 20, "name": "Away FC"},
                 "player": {"id": 3, "name": "C. Unlucky"}, "assist": {"id": null, "name": null},
                 "type": "Goal", "detail": "Own Goal", "comments": null},
                {"time": {"elapsed": 58, "extra": null}, "team": {"id": 20, "name": "Away FC"},
                 "player": {"id": 4, "name": "D. Wide"}, "assist": {"id": null, "name": null},
                 "type": "Goal", "detail": "Missed Penalty", "comments": null},
                {"time": {"elapsed": 61, "extra": null}, "team": {"id": 20, "name": "Away FC"},
                 "player": {"id": 5, "name": "E. Nine"}, "assist": {"id": 3, "name": "C. Unlucky"},
                 "type": "Goal", "detail": "Normal Goal", "comments": null}
            ]
        });
        let m = serde_json::from_value::<FxItem>(v).unwrap().into_match();
        assert_eq!(
            (m.id, m.kickoff, m.status.as_str(), m.minute),
            (1_200_001, 1_795_282_200, "2H", Some(63))
        );
        assert_eq!(
            (m.home.name.as_str(), m.home_score, m.away_score),
            ("Home FC", Some(2), Some(1))
        );
        let scorers = m.scorers.unwrap();
        let lines: Vec<(bool, &str, Option<u32>, Option<&str>)> = scorers
            .iter()
            .map(|s| (s.home, s.name.as_str(), s.minute, s.note.as_deref()))
            .collect();
        assert_eq!(
            lines,
            vec![
                (true, "A. Striker", Some(41), Some("pen")),
                (false, "C. Unlucky", Some(50), Some("og")),
                (false, "E. Nine", Some(61), None),
            ]
        );
    }

    #[test]
    fn a_fixture_list_without_events_has_no_scorers_and_no_score_before_kickoff() {
        let v = serde_json::json!({
            "fixture": {"id": 7, "timestamp": 1_795_282_200, "status": {"long": "Not Started", "short": "NS", "elapsed": null}},
            "league": {"id": 39, "season": 2026},
            "teams": {"home": {"id": 1, "name": "A"}, "away": {"id": 2, "name": "B"}},
            "goals": {"home": null, "away": null}
        });
        let m = serde_json::from_value::<FxItem>(v).unwrap().into_match();
        assert_eq!((m.home_score, m.minute, m.scorers), (None, None, None));
    }

    #[test]
    fn a_rugby_game_and_both_team_shapes_read() {
        let v = serde_json::json!({
            "id": 30_001, "date": "2026-11-21T15:00:00+00:00", "time": "15:00", "timestamp": 1_795_273_200,
            "timezone": "UTC", "week": "8", "status": {"long": "Finished", "short": "FT"},
            "country": {"id": 26, "name": "World", "code": null, "flag": null},
            "league": {"id": 76, "name": "United Rugby Championship", "type": "Cup", "season": 2026},
            "teams": {"home": {"id": 501, "name": "Coastal XV", "logo": "x"}, "away": {"id": 502, "name": "Highveld XV", "logo": "y"}},
            "scores": {"home": 27, "away": 20},
            "periods": {"first": {"home": 13, "away": 10}, "second": {"home": 14, "away": 10}}
        });
        let m = serde_json::from_value::<RgItem>(v).unwrap().into_match();
        assert_eq!(
            (m.id, m.status.as_str(), m.home_score, m.away_score),
            (30_001, "FT", Some(27), Some(20))
        );
        assert_eq!(m.away.name, "Highveld XV");
        let nested: TeamItem = serde_json::from_value(
            serde_json::json!({"team": {"id": 1, "name": "A", "code": "AAA"}, "venue": {}}),
        )
        .unwrap();
        let flat: TeamItem = serde_json::from_value(
            serde_json::json!({"id": 2, "name": "B", "logo": "z", "national": false}),
        )
        .unwrap();
        assert!(matches!(nested, TeamItem::Nested { team } if team.id == 1));
        assert!(matches!(flat, TeamItem::Flat(t) if t.name == "B"));
    }
}
