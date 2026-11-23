//! A plan's forecast (#387) from Open-Meteo (free, no key): its geocoding API turns the place into a spot, and its
//! daily forecast (16 days) gives the plan's day there, with its hourly temperatures the one at the plan's start
//! (#399). Only the server calls them, and it caches hard, in the
//! database so a restart keeps the cache: a place for [`GEOCODE_TTL_DAYS`] (a place it doesn't know too, so a typo
//! isn't asked about again and again), a spot's forecast for [`FORECAST_TTL_HOURS`]. When Open-Meteo can't be
//! reached, a stale forecast still answers; with none, there is no forecast. Calls go one at a time server-wide.
//!
//! Plans are local (#451): only the home country's places are looked up (`PULSE_PLANS_COUNTRY`, `ZA` by default),
//! and a place is never just Open-Meteo's first result: of its top [`CANDIDATES`] there, the first whose name matches
//! the place is taken; with none, the place is unknown and there is no forecast (free text like "test" or "someone's
//! house" stays free text, and nowhere abroad is guessed at).

use std::sync::OnceLock;
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use pulse_protocol::rest::{PlanForecast, plan_limits};
use reqwest::Url;
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::config::PlansConfig;
use crate::db::stamp;
use crate::error::AppResult;

pub const GEOCODE_TTL_DAYS: i64 = 30;
pub const FORECAST_TTL_HOURS: i64 = 3;
const TIMEOUT: Duration = Duration::from_secs(10);

/// One outside call at a time: a burst of cards opening never fans out into a burst of calls, and a second asker
/// finds the first one's answer in the cache.
static CALLS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent("Pulse")
            .build()
            .unwrap_or_default()
    })
}

/// A place as Open-Meteo found it.
#[derive(Clone, Debug, PartialEq)]
pub struct Spot {
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
}

impl Spot {
    /// The forecast cache's key: two decimals (about a kilometre) is as fine as a daily forecast gets.
    fn key(&self) -> String {
        format!("{:.2},{:.2}", self.latitude, self.longitude)
    }
}

#[derive(Deserialize)]
struct GeoAnswer {
    #[serde(default)]
    results: Vec<GeoResult>,
}

#[derive(Debug, Deserialize)]
struct GeoResult {
    name: String,
    latitude: f64,
    longitude: f64,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    country_code: Option<String>,
}

/// How many results Open-Meteo is asked for: enough to find the one whose name matches among its fuzzy matches.
const CANDIDATES: usize = 10;

/// A name as compared: lower case, accents off, words single-spaced, no punctuation.
fn fold(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let plain: String = s
        .nfkd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                ' '
            }
        })
        .collect();
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How well a result's name is the place asked for (`q`, folded): exactly (2), or the place is its leading word(s)
/// (1: "cape" for Cape Town); else not at all (0: "durbs" isn't Durbanville).
fn name_match(g: &GeoResult, q: &str) -> u8 {
    let name = fold(&g.name);
    if name == q {
        2
    } else if name.starts_with(&format!("{q} ")) {
        1
    } else {
        0
    }
}

/// The result that is the place asked for, or none: of those in `home` (an ISO 3166 alpha-2 code; Open-Meteo is
/// asked for only those, and anything else is never taken) whose name matches, the first exact one, else the first
/// it leads.
fn choose(results: Vec<GeoResult>, query: &str, home: &str) -> Option<GeoResult> {
    let q = fold(query);
    if q.is_empty() {
        return None;
    }
    let mut best: Option<(u8, GeoResult)> = None;
    for g in results {
        let at_home = g
            .country_code
            .as_deref()
            .is_some_and(|c| c.eq_ignore_ascii_case(home));
        let m = if at_home { name_match(&g, &q) } else { 0 };
        if m > best.as_ref().map_or(0, |(b, _)| *b) {
            best = Some((m, g));
        }
    }
    best.map(|(_, g)| g)
}

/// Open-Meteo's daily forecast, as cached.
#[derive(Debug, Deserialize)]
pub struct ForecastAnswer {
    #[serde(default)]
    pub utc_offset_seconds: i64,
    pub daily: Daily,
    /// Absent from a forecast cached before #399.
    #[serde(default)]
    pub hourly: Option<Hourly>,
}

/// Open-Meteo's hourly temperatures: `time` is the place's local hour (`YYYY-MM-DDTHH:00`).
#[derive(Debug, Deserialize)]
pub struct Hourly {
    pub time: Vec<String>,
    pub temperature_2m: Vec<Option<f64>>,
}

#[derive(Debug, Deserialize)]
pub struct Daily {
    pub time: Vec<String>,
    pub weather_code: Vec<Option<f64>>,
    pub temperature_2m_max: Vec<Option<f64>>,
    pub temperature_2m_min: Vec<Option<f64>>,
    #[serde(default)]
    pub precipitation_probability_max: Vec<Option<f64>>,
}

/// The cache key for a place as typed.
fn query_key(place: &str) -> String {
    place.trim().to_lowercase()
}

/// The place as a spot: from the cache, or Open-Meteo (cached either way). `None`: it doesn't know the place, or
/// can't be asked right now.
pub async fn geocode(
    db: &SqlitePool,
    cfg: &PlansConfig,
    place: &str,
    now: Timestamp,
) -> AppResult<Option<Spot>> {
    let key = query_key(place);
    let fresh = stamp(now - crate::times::days(GEOCODE_TTL_DAYS));
    type Cached = (Option<String>, Option<f64>, Option<f64>);
    let read = || async {
        sqlx::query_as::<_, Cached>(
            "SELECT name, latitude, longitude FROM plan_geocodes WHERE query = ? AND fetched_at > ?",
        )
        .bind(&key)
        .bind(&fresh)
        .fetch_optional(db)
        .await
    };
    let spot = |c: Cached| match c {
        (Some(name), Some(latitude), Some(longitude)) => Some(Spot {
            name,
            latitude,
            longitude,
        }),
        _ => None,
    };
    if let Some(c) = read().await? {
        return Ok(spot(c));
    }
    let _one = CALLS.lock().await;
    // someone else may have asked while we waited
    if let Some(c) = read().await? {
        return Ok(spot(c));
    }
    let Some(found) = ask_geocode(cfg, &key).await else {
        return Ok(None);
    };
    sqlx::query(
        "INSERT INTO plan_geocodes (query, name, latitude, longitude, fetched_at) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (query) DO UPDATE SET name = excluded.name, latitude = excluded.latitude,
           longitude = excluded.longitude, fetched_at = excluded.fetched_at",
    )
    .bind(&key)
    .bind(found.as_ref().map(|s| s.name.clone()))
    .bind(found.as_ref().map(|s| s.latitude))
    .bind(found.as_ref().map(|s| s.longitude))
    .bind(stamp(now))
    .execute(db)
    .await?;
    Ok(found)
}

/// Ask Open-Meteo's geocoding: `Some(None)` = it doesn't know the place; `None` = no good answer (try again later).
async fn ask_geocode(cfg: &PlansConfig, query: &str) -> Option<Option<Spot>> {
    let mut url = Url::parse(&format!(
        "{}/v1/search",
        cfg.geocode_base.trim_end_matches('/')
    ))
    .ok()?;
    url.query_pairs_mut()
        .append_pair("name", query)
        .append_pair("count", &CANDIDATES.to_string())
        .append_pair("countryCode", &cfg.country)
        .append_pair("language", "en")
        .append_pair("format", "json");
    let r = match http().get(url).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            tracing::warn!(status = %r.status(), "Open-Meteo geocoding refused");
            return None;
        }
        Err(e) => {
            tracing::warn!(error = %e.without_url(), "Open-Meteo geocoding failed");
            return None;
        }
    };
    let answer: GeoAnswer = r.json().await.ok()?;
    // never just the first result: "test" would be some village far away (#451)
    Some(choose(answer.results, query, &cfg.country).map(|g| Spot {
        name: match g.country {
            Some(c) if !c.is_empty() && c != g.name => format!("{}, {c}", g.name),
            _ => g.name,
        },
        latitude: g.latitude,
        longitude: g.longitude,
    }))
}

/// The spot's 16-day forecast: cached if fresh, else from Open-Meteo (then cached); a stale one when Open-Meteo
/// can't answer.
async fn forecast_for(
    db: &SqlitePool,
    cfg: &PlansConfig,
    spot: &Spot,
    now: Timestamp,
) -> AppResult<Option<ForecastAnswer>> {
    let key = spot.key();
    let read = || async {
        sqlx::query_as::<_, (String, String)>(
            "SELECT body, fetched_at FROM plan_forecasts WHERE spot = ?",
        )
        .bind(&key)
        .fetch_optional(db)
        .await
    };
    let fresh = stamp(now - SignedDuration::from_hours(FORECAST_TTL_HOURS));
    let parse = |body: &str| serde_json::from_str::<ForecastAnswer>(body).ok();
    let cached = read().await?;
    if let Some((body, at)) = &cached
        && *at > fresh
    {
        return Ok(parse(body));
    }
    let _one = CALLS.lock().await;
    let cached = read().await?;
    if let Some((body, at)) = &cached
        && *at > fresh
    {
        return Ok(parse(body));
    }
    match ask_forecast(cfg, spot).await {
        Some(body) => {
            sqlx::query(
                "INSERT INTO plan_forecasts (spot, body, fetched_at) VALUES (?, ?, ?)
                 ON CONFLICT (spot) DO UPDATE SET body = excluded.body, fetched_at = excluded.fetched_at",
            )
            .bind(&key)
            .bind(&body)
            .bind(stamp(now))
            .execute(db)
            .await?;
            Ok(parse(&body))
        }
        None => Ok(cached.and_then(|(body, _)| parse(&body))),
    }
}

/// Ask Open-Meteo for a spot's daily forecast: the body as it came (checked to read), or `None`.
async fn ask_forecast(cfg: &PlansConfig, spot: &Spot) -> Option<String> {
    let mut url = Url::parse(&format!(
        "{}/v1/forecast",
        cfg.forecast_base.trim_end_matches('/')
    ))
    .ok()?;
    url.query_pairs_mut()
        .append_pair("latitude", &format!("{:.4}", spot.latitude))
        .append_pair("longitude", &format!("{:.4}", spot.longitude))
        .append_pair(
            "daily",
            "weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max",
        )
        .append_pair("hourly", "temperature_2m")
        .append_pair("timezone", "auto")
        .append_pair("forecast_days", &plan_limits::FORECAST_DAYS.to_string());
    let r = match http().get(url).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            tracing::warn!(status = %r.status(), "Open-Meteo forecast refused");
            return None;
        }
        Err(e) => {
            tracing::warn!(error = %e.without_url(), "Open-Meteo forecast failed");
            return None;
        }
    };
    let body = r.text().await.ok()?;
    serde_json::from_str::<ForecastAnswer>(&body).ok()?;
    Some(body)
}

/// The largest offset from UTC a place has (with room to spare), in seconds.
const MAX_OFFSET_SECS: i64 = 18 * 3600;

/// The day a plan starting at `starts_at` falls on at the spot (whose offset from UTC the forecast gives), out of
/// the forecast, with the temperature in the hour it starts when the forecast has it; `None` when that day isn't in
/// it.
pub fn pick(answer: &ForecastAnswer, spot: &Spot, starts_at: Timestamp) -> Option<PlanForecast> {
    // The offset is the forecast service's: kept to a real one (within ±18 h, as time zones are), and added
    // checked, so a strange answer can't panic the request.
    let offset = answer
        .utc_offset_seconds
        .clamp(-MAX_OFFSET_SECS, MAX_OFFSET_SECS);
    let local = starts_at
        .checked_add(SignedDuration::from_secs(offset))
        .ok()?;
    let date = local.strftime("%Y-%m-%d").to_string();
    let hour = local.strftime("%Y-%m-%dT%H:00").to_string();
    let temp_at_start = answer.hourly.as_ref().and_then(|h| {
        let i = h.time.iter().position(|t| *t == hour)?;
        h.temperature_2m.get(i).copied().flatten().map(|t| t as f32)
    });
    let d = &answer.daily;
    let i = d.time.iter().position(|t| *t == date)?;
    let code = (*d.weather_code.get(i)?)?;
    Some(PlanForecast {
        place: spot.name.clone(),
        date,
        code: code.clamp(0.0, 255.0) as u8,
        temp_max: (*d.temperature_2m_max.get(i)?)? as f32,
        temp_min: (*d.temperature_2m_min.get(i)?)? as f32,
        rain_chance: d
            .precipitation_probability_max
            .get(i)
            .copied()
            .flatten()
            .map(|p| p.clamp(0.0, 100.0) as u8),
        temp_at_start,
    })
}

/// Whether a plan starting at `starts_at` can have a forecast at `now`: it hasn't long started, and its day is
/// within the forecast's reach.
pub fn in_window(starts_at: Timestamp, now: Timestamp) -> bool {
    starts_at > now - SignedDuration::from_hours(12)
        && starts_at < now + crate::times::days(plan_limits::FORECAST_DAYS)
}

/// The forecast for `place` on the day of `starts_at`, if there is one.
pub async fn forecast(
    db: &SqlitePool,
    cfg: &PlansConfig,
    place: &str,
    starts_at: Timestamp,
    now: Timestamp,
) -> AppResult<Option<PlanForecast>> {
    if !in_window(starts_at, now) {
        return Ok(None);
    }
    let Some(spot) = geocode(db, cfg, place, now).await? else {
        return Ok(None);
    };
    let Some(answer) = forecast_for(db, cfg, &spot, now).await? else {
        return Ok(None);
    };
    Ok(pick(&answer, &spot, starts_at))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Timestamp {
        crate::times::parse(s).unwrap()
    }

    #[test]
    fn the_plan_s_day_is_its_day_at_the_place() {
        let answer: ForecastAnswer = serde_json::from_str(
            r#"{"utc_offset_seconds":7200,"daily":{"time":["2026-11-20","2026-11-21"],
                "weather_code":[61,0],"temperature_2m_max":[24.6,30.1],"temperature_2m_min":[14.2,null],
                "precipitation_probability_max":[80,null]}}"#,
        )
        .unwrap();
        let spot = Spot {
            name: "Cape Town, South Africa".into(),
            latitude: -33.92,
            longitude: 18.42,
        };
        // 21:30 UTC on the 20th is 23:30 there: still the 20th
        let f = pick(&answer, &spot, t("2026-11-20T21:30:00Z")).unwrap();
        assert_eq!(f.date, "2026-11-20");
        assert_eq!((f.code, f.rain_chance), (61, Some(80)));
        assert!((f.temp_max - 24.6).abs() < 0.01);
        // 22:30 UTC is past midnight there: the 21st, whose low is missing, so no forecast
        assert_eq!(pick(&answer, &spot, t("2026-11-20T22:30:00Z")), None);
        // a day not in it
        assert_eq!(pick(&answer, &spot, t("2026-11-25T10:00:00Z")), None);
        // cached before #399: no hourly, so no temperature at the start
        assert_eq!(f.temp_at_start, None);
    }

    #[test]
    fn a_strange_offset_from_the_forecast_never_panics() {
        let spot = Spot {
            name: "Cape Town, South Africa".into(),
            latitude: -33.92,
            longitude: 18.42,
        };
        for offset in [i64::MAX, i64::MIN, 10_000_000_000] {
            let answer: ForecastAnswer = serde_json::from_str(&format!(
                r#"{{"utc_offset_seconds":{offset},"daily":{{"time":["2026-11-20"],
                    "weather_code":[0],"temperature_2m_max":[30.0],"temperature_2m_min":[15.0]}}}}"#
            ))
            .unwrap();
            // clamped to ±18 h: a day near it, or none, but an answer
            let _ = pick(&answer, &spot, t("2026-11-20T12:00:00Z"));
            let _ = pick(&answer, &spot, Timestamp::MAX);
            let _ = pick(&answer, &spot, Timestamp::MIN);
        }
    }

    #[test]
    fn the_temperature_is_the_one_in_the_hour_it_starts_there() {
        let answer: ForecastAnswer = serde_json::from_str(
            r#"{"utc_offset_seconds":7200,"daily":{"time":["2026-11-20"],
                "weather_code":[0],"temperature_2m_max":[30.0],"temperature_2m_min":[15.0]},
                "hourly":{"time":["2026-11-20T17:00","2026-11-20T18:00","2026-11-20T19:00"],
                "temperature_2m":[27.4,25.6,null]}}"#,
        )
        .unwrap();
        let spot = Spot {
            name: "Cape Town, South Africa".into(),
            latitude: -33.92,
            longitude: 18.42,
        };
        // 16:30 UTC is 18:30 there: the 18:00 hour
        let f = pick(&answer, &spot, t("2026-11-20T16:30:00Z")).unwrap();
        assert!((f.temp_at_start.unwrap() - 25.6).abs() < 0.01);
        // an hour it has no value for, or not in it: the day's forecast without one
        assert_eq!(
            pick(&answer, &spot, t("2026-11-20T17:00:00Z"))
                .unwrap()
                .temp_at_start,
            None
        );
        assert_eq!(
            pick(&answer, &spot, t("2026-11-20T08:00:00Z"))
                .unwrap()
                .temp_at_start,
            None
        );
    }

    #[test]
    fn only_plans_within_the_forecast_s_reach_get_one() {
        let now = t("2026-11-16T10:00:00Z");
        assert!(in_window(t("2026-11-20T10:00:00Z"), now));
        assert!(in_window(t("2026-12-01T09:00:00Z"), now));
        assert!(!in_window(t("2026-12-02T11:00:00Z"), now));
        assert!(!in_window(t("2026-11-15T10:00:00Z"), now));
    }

    fn geo(name: &str, cc: &str) -> GeoResult {
        GeoResult {
            name: name.into(),
            latitude: 0.0,
            longitude: 0.0,
            country: None,
            country_code: Some(cc.into()),
        }
    }

    fn chosen(results: Vec<GeoResult>, query: &str) -> Option<String> {
        choose(results, query, "ZA").map(|g| g.name)
    }

    #[test]
    fn names_fold_for_comparing() {
        assert_eq!(fold("  Cape   Town "), "cape town");
        assert_eq!(fold("Zürich"), "zurich");
        assert_eq!(fold("someone's house"), "someone s house");
        assert_eq!(fold("Port-Elizabeth"), "port elizabeth");
    }

    #[test]
    fn only_a_home_place_whose_name_matches() {
        // an exact name beats one the place only leads, and either beats a fuzzy match listed first
        let r = vec![geo("Stellenbosch Farms", "ZA"), geo("Stellenbosch", "ZA")];
        assert_eq!(chosen(r, "stellenbosch").as_deref(), Some("Stellenbosch"));
        let r = vec![geo("Kaapstad Oos", "ZA"), geo("Cape Town", "ZA")];
        assert_eq!(chosen(r, "Cape Town").as_deref(), Some("Cape Town"));
        // leading words do; case and accents don't matter
        assert!(chosen(vec![geo("Cape Town", "ZA")], "cape").is_some());
        assert!(chosen(vec![geo("Cape Town", "ZA")], "CAPE TOWN").is_some());
        assert!(chosen(vec![geo("Pietermaritzburg", "ZA")], "piétermaritzburg").is_some());
        // a fuzzy match isn't the place
        assert_eq!(chosen(vec![geo("Durbanville", "ZA")], "durbs"), None);
        assert_eq!(chosen(vec![geo("Muizenberg", "ZA")], "berg"), None);
        assert_eq!(chosen(vec![geo("East London", "ZA")], "london"), None);
        assert_eq!(chosen(vec![geo("Tesvillage", "ZA")], "test"), None);
        assert_eq!(chosen(vec![], "someone's house"), None);
        assert_eq!(chosen(vec![geo("X", "ZA")], "  "), None);
        // never abroad, even called exactly that
        assert_eq!(chosen(vec![geo("London", "GB")], "london"), None);
        assert_eq!(chosen(vec![geo("Test", "XX")], "test"), None);
        // another home country
        assert_eq!(
            choose(vec![geo("London", "GB")], "london", "gb").map(|g| g.name),
            Some("London".into())
        );
    }

    #[test]
    fn spots_and_places_key_the_cache() {
        let s = Spot {
            name: "x".into(),
            latitude: -33.9249,
            longitude: 18.4241,
        };
        assert_eq!(s.key(), "-33.92,18.42");
        assert_eq!(query_key("  Cape Town "), "cape town");
    }
}
