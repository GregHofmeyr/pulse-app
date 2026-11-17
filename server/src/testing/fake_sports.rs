//! A stand-in for API-Sports (#385) on 127.0.0.1: api-football v3 under `/football` and api-rugby v1 under `/rugby`,
//! answering in their envelope (`get`, `parameters`, `errors`, `results`, `paging`, `response`) with recorded-shape
//! items. [`FakeSports::configure`] points a [`Config`] at it with a key.
//!
//! - `/football/teams`, `/rugby/teams` (`league`, `season`): the teams given [`FakeSports::add_team`] (football's
//!   nested under `team`, rugby's flat, as the two APIs answer).
//! - `/football/fixtures`: by `league` + `season` + `from`/`to`, or by `ids` (`1-2-3`, with `events`), from
//!   [`FakeSports::add_match`], [`FakeSports::set_score`] and [`FakeSports::add_goal`].
//! - `/rugby/games`: by `league` + `season`, and `date` (the kick-off's UTC day) when given.
//!
//! - `http://sportslogos.test:<port>/media/{football|rugby}/teams/{id}.png` ([`LOGO_HOST`]): a team's logo (#406),
//!   a small PNG; a team id of [`NO_LOGO`] or more has none (404). Counted as `media`.
//!
//! Like the real thing, failures arrive as HTTP 200 with an `errors` object: a missing or wrong key (`token`),
//! [`FakeSports::spend_day`] (`requests`), [`FakeSports::refuse_plan`] (`plan`). [`FakeSports::set_down`] answers 503.
//! Calls are counted by path ([`FakeSports::calls`]) and the last query of each kept ([`FakeSports::last_query`]).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::config::Config;

/// The key [`FakeSports::configure`] gives the server.
pub const FAKE_KEY: &str = "test-sports-key";
/// The logo host (#406), resolved to the fake by [`FakeSports::configure`].
pub const LOGO_HOST: &str = "sportslogos.test";
/// Teams from this id up have no logo.
pub const NO_LOGO: u32 = 900;
const LOOPBACK: std::net::IpAddr = std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

#[derive(Clone)]
struct FakeMatch {
    league: u32,
    id: u32,
    kickoff: i64,
    home: (u32, String),
    away: (u32, String),
    status: String,
    elapsed: Option<u32>,
    score: (Option<u32>, Option<u32>),
    events: Vec<serde_json::Value>,
}

#[derive(Default)]
struct World {
    teams: HashMap<u32, Vec<(u32, String)>>,
    matches: Vec<FakeMatch>,
    down: bool,
    spent: bool,
    plan: bool,
    calls: HashMap<String, usize>,
    queries: HashMap<String, HashMap<String, String>>,
}

pub struct FakeSports {
    pub port: u16,
    world: Arc<Mutex<World>>,
}

type Shared = Arc<Mutex<World>>;
type Q = Query<HashMap<String, String>>;

impl FakeSports {
    pub async fn start() -> FakeSports {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let world: Shared = Arc::default();
        let app = Router::new()
            .route("/football/teams", get(football_teams))
            .route("/football/fixtures", get(fixtures))
            .route("/rugby/teams", get(rugby_teams))
            .route("/rugby/games", get(games))
            .route("/media/{sport}/teams/{file}", get(logo))
            .with_state(world.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        FakeSports { port, world }
    }

    /// Point `cfg` at the fake, with [`FAKE_KEY`] and no backoff waits.
    pub fn configure(&self, cfg: &mut Config) {
        cfg.sports.api_key = Some(FAKE_KEY.into());
        cfg.sports.football_base = format!("http://127.0.0.1:{}/football", self.port);
        cfg.sports.rugby_base = format!("http://127.0.0.1:{}/rugby", self.port);
        cfg.sports.retry_base = std::time::Duration::ZERO;
        cfg.sports.logo_base = format!("http://{LOGO_HOST}:{}/media", self.port);
        cfg.sports.logo_hosts = vec![LOGO_HOST.into()];
        cfg.embeds.allowed_ips.push(LOOPBACK);
        cfg.embeds.extra_ports.push(self.port);
        cfg.embeds.hosts.push((LOGO_HOST.into(), LOOPBACK));
    }

    pub fn add_team(&self, league: u32, id: u32, name: &str) {
        let mut w = self.world.lock().unwrap();
        w.teams.entry(league).or_default().push((id, name.into()));
    }

    /// A match in `league` (API-Sports' id) kicking off at `kickoff` (unix seconds), not started.
    pub fn add_match(
        &self,
        league: u32,
        id: u32,
        kickoff: i64,
        home: (u32, &str),
        away: (u32, &str),
    ) {
        self.world.lock().unwrap().matches.push(FakeMatch {
            league,
            id,
            kickoff,
            home: (home.0, home.1.into()),
            away: (away.0, away.1.into()),
            status: "NS".into(),
            elapsed: None,
            score: (None, None),
            events: vec![],
        });
    }

    /// The match's status (`1H`, `HT`, `FT`, …), minute and score.
    pub fn set_score(&self, id: u32, status: &str, elapsed: Option<u32>, home: u32, away: u32) {
        let mut w = self.world.lock().unwrap();
        let m = w.matches.iter_mut().find(|m| m.id == id).unwrap();
        m.status = status.into();
        m.elapsed = elapsed;
        m.score = (Some(home), Some(away));
    }

    /// A goal event (`detail`: `Normal Goal`, `Penalty`, `Own Goal`, `Missed Penalty`).
    pub fn add_goal(&self, id: u32, minute: u32, team: u32, player: &str, detail: &str) {
        let mut w = self.world.lock().unwrap();
        let m = w.matches.iter_mut().find(|m| m.id == id).unwrap();
        let team_name = if m.home.0 == team {
            m.home.1.clone()
        } else {
            m.away.1.clone()
        };
        m.events.push(serde_json::json!({
            "time": {"elapsed": minute, "extra": null},
            "team": {"id": team, "name": team_name, "logo": "https://media.api-sports.io/football/teams/0.png"},
            "player": {"id": 9000 + m.events.len(), "name": player},
            "assist": {"id": null, "name": null},
            "type": "Goal", "detail": detail, "comments": null
        }));
    }

    pub fn set_down(&self, down: bool) {
        self.world.lock().unwrap().down = down;
    }

    /// Answer every call with the day's limit reached.
    pub fn spend_day(&self) {
        self.world.lock().unwrap().spent = true;
    }

    /// Answer every call with the free plan's season refusal.
    pub fn refuse_plan(&self) {
        self.world.lock().unwrap().plan = true;
    }

    /// Calls to `path` (`football/fixtures`, `rugby/games`, …).
    pub fn calls(&self, path: &str) -> usize {
        self.world
            .lock()
            .unwrap()
            .calls
            .get(path)
            .copied()
            .unwrap_or(0)
    }

    /// Every call so far.
    pub fn total_calls(&self) -> usize {
        self.world.lock().unwrap().calls.values().sum()
    }

    pub fn last_query(&self, path: &str) -> HashMap<String, String> {
        self.world
            .lock()
            .unwrap()
            .queries
            .get(path)
            .cloned()
            .unwrap_or_default()
    }
}

/// A team's logo (#406): no key needed, as on API-Sports' media host.
async fn logo(
    State(w): State<Shared>,
    axum::extract::Path((_sport, file)): axum::extract::Path<(String, String)>,
) -> Response {
    *w.lock().unwrap().calls.entry("media".into()).or_default() += 1;
    let id: Option<u32> = file.strip_suffix(".png").and_then(|i| i.parse().ok());
    match id {
        Some(id) if id < NO_LOGO => (
            [(axum::http::header::CONTENT_TYPE, "image/png")],
            super::fake_provider::png(96, 96),
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

fn envelope(get: &str, q: &HashMap<String, String>, response: Vec<serde_json::Value>) -> Response {
    let n = response.len();
    axum::Json(serde_json::json!({
        "get": get, "parameters": q, "errors": [], "results": n,
        "paging": {"current": 1, "total": 1}, "response": response
    }))
    .into_response()
}

fn failure(get: &str, q: &HashMap<String, String>, errors: serde_json::Value) -> Response {
    axum::Json(serde_json::json!({
        "get": get, "parameters": q, "errors": errors, "results": 0,
        "paging": {"current": 1, "total": 1}, "response": []
    }))
    .into_response()
}

/// Count the call and answer for the key and the switches; `None` = go on.
fn gate(w: &Shared, path: &str, h: &HeaderMap, q: &HashMap<String, String>) -> Option<Response> {
    let mut w = w.lock().unwrap();
    *w.calls.entry(path.into()).or_default() += 1;
    w.queries.insert(path.into(), q.clone());
    let get = path.rsplit('/').next().unwrap_or(path);
    if w.down {
        return Some(StatusCode::SERVICE_UNAVAILABLE.into_response());
    }
    if h.get("x-apisports-key").and_then(|v| v.to_str().ok()) != Some(FAKE_KEY) {
        return Some(failure(
            get,
            q,
            serde_json::json!({"token": "Error/Missing application key. Go to https://www.api-football.com/documentation-v3 to learn how to get your API application key."}),
        ));
    }
    if w.spent {
        return Some(failure(
            get,
            q,
            serde_json::json!({"requests": "You have reached the request limit for the day, Go to https://dashboard.api-football.com to upgrade your plan."}),
        ));
    }
    if w.plan {
        return Some(failure(
            get,
            q,
            serde_json::json!({"plan": "Free plans do not have access to this season, try from 2021 to 2023."}),
        ));
    }
    None
}

fn league_of(q: &HashMap<String, String>) -> Option<u32> {
    q.get("league")?.parse().ok()
}

async fn football_teams(State(w): State<Shared>, h: HeaderMap, Query(q): Q) -> Response {
    if let Some(r) = gate(&w, "football/teams", &h, &q) {
        return r;
    }
    let teams = w
        .lock()
        .unwrap()
        .teams
        .get(&league_of(&q).unwrap_or(0))
        .cloned()
        .unwrap_or_default();
    let items = teams
        .into_iter()
        .map(|(id, name)| {
            serde_json::json!({
                "team": {"id": id, "name": name, "code": null, "country": "South-Africa", "founded": 1970,
                    "national": false, "logo": format!("https://media.api-sports.io/football/teams/{id}.png")},
                "venue": {"id": 1, "name": "Stadium", "address": null, "city": "Durban", "capacity": 50000,
                    "surface": "grass", "image": "https://media.api-sports.io/football/venues/1.png"}
            })
        })
        .collect();
    envelope("teams", &q, items)
}

async fn rugby_teams(State(w): State<Shared>, h: HeaderMap, Query(q): Q) -> Response {
    if let Some(r) = gate(&w, "rugby/teams", &h, &q) {
        return r;
    }
    let teams = w
        .lock()
        .unwrap()
        .teams
        .get(&league_of(&q).unwrap_or(0))
        .cloned()
        .unwrap_or_default();
    let items = teams
        .into_iter()
        .map(|(id, name)| {
            serde_json::json!({
                "id": id, "name": name, "logo": format!("https://media.api-sports.io/rugby/teams/{id}.png"),
                "national": false, "founded": null, "arena": {"name": null, "capacity": null, "location": null},
                "country": {"id": 26, "name": "World", "code": null, "flag": null}
            })
        })
        .collect();
    envelope("teams", &q, items)
}

fn fixture_json(m: &FakeMatch, with_events: bool) -> serde_json::Value {
    let date = chrono::DateTime::from_timestamp(m.kickoff, 0)
        .unwrap()
        .to_rfc3339();
    let long = match m.status.as_str() {
        "NS" => "Not Started",
        "1H" => "First Half",
        "HT" => "Halftime",
        "2H" => "Second Half",
        "FT" => "Match Finished",
        "PST" => "Match Postponed",
        _ => "In Progress",
    };
    let mut v = serde_json::json!({
        "fixture": {"id": m.id, "referee": null, "timezone": "UTC", "date": date, "timestamp": m.kickoff,
            "periods": {"first": null, "second": null},
            "venue": {"id": 1, "name": "Stadium", "city": "Durban"},
            "status": {"long": long, "short": m.status, "elapsed": m.elapsed, "extra": null}},
        "league": {"id": m.league, "name": "League", "country": "World", "logo": "", "flag": null,
            "season": 2026, "round": "Regular Season - 10"},
        "teams": {"home": {"id": m.home.0, "name": m.home.1, "logo": "", "winner": null},
                  "away": {"id": m.away.0, "name": m.away.1, "logo": "", "winner": null}},
        "goals": {"home": m.score.0, "away": m.score.1},
        "score": {"halftime": {"home": null, "away": null}, "fulltime": {"home": null, "away": null},
                  "extratime": {"home": null, "away": null}, "penalty": {"home": null, "away": null}}
    });
    if with_events {
        v["events"] = serde_json::Value::Array(m.events.clone());
        v["lineups"] = serde_json::json!([]);
        v["statistics"] = serde_json::json!([]);
        v["players"] = serde_json::json!([]);
    }
    v
}

async fn fixtures(State(w): State<Shared>, h: HeaderMap, Query(q): Q) -> Response {
    if let Some(r) = gate(&w, "football/fixtures", &h, &q) {
        return r;
    }
    let matches = w.lock().unwrap().matches.clone();
    if let Some(ids) = q.get("ids") {
        let ids: Vec<u32> = ids.split('-').filter_map(|i| i.parse().ok()).collect();
        let items = matches
            .iter()
            .filter(|m| ids.contains(&m.id))
            .map(|m| fixture_json(m, true))
            .collect();
        return envelope("fixtures", &q, items);
    }
    let day = |key: &str| {
        q.get(key)
            .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
    };
    let (from, to) = (day("from"), day("to"));
    let league = league_of(&q);
    let items = matches
        .iter()
        .filter(|m| Some(m.league) == league)
        .filter(|m| {
            let d = chrono::DateTime::from_timestamp(m.kickoff, 0)
                .unwrap()
                .date_naive();
            from.is_none_or(|f| d >= f) && to.is_none_or(|t| d <= t)
        })
        .map(|m| fixture_json(m, false))
        .collect();
    envelope("fixtures", &q, items)
}

async fn games(State(w): State<Shared>, h: HeaderMap, Query(q): Q) -> Response {
    if let Some(r) = gate(&w, "rugby/games", &h, &q) {
        return r;
    }
    let matches = w.lock().unwrap().matches.clone();
    let league = league_of(&q);
    let date = q
        .get("date")
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
    let items = matches
        .iter()
        .filter(|m| Some(m.league) == league)
        .filter(|m| date.is_none_or(|d| chrono::DateTime::from_timestamp(m.kickoff, 0).unwrap().date_naive() == d))
        .map(|m| {
            let t = chrono::DateTime::from_timestamp(m.kickoff, 0).unwrap();
            let long = match m.status.as_str() {
                "NS" => "Not Started",
                "FT" => "Finished",
                "HT" => "Half Time",
                _ => "In Play",
            };
            serde_json::json!({
                "id": m.id, "date": t.to_rfc3339(), "time": t.format("%H:%M").to_string(), "timestamp": m.kickoff,
                "timezone": "UTC", "week": "8", "status": {"long": long, "short": m.status},
                "country": {"id": 26, "name": "World", "code": null, "flag": null},
                "league": {"id": m.league, "name": "United Rugby Championship", "type": "Cup",
                    "logo": "https://media.api-sports.io/rugby/leagues/76.png", "season": 2026},
                "teams": {"home": {"id": m.home.0, "name": m.home.1, "logo": ""},
                          "away": {"id": m.away.0, "name": m.away.1, "logo": ""}},
                "scores": {"home": m.score.0, "away": m.score.1},
                "periods": {"first": {"home": null, "away": null}, "second": {"home": null, "away": null},
                    "overtime": {"home": null, "away": null}, "second_overtime": {"home": null, "away": null}}
            })
        })
        .collect();
    envelope("games", &q, items)
}
