//! A plan's forecast (#387) from Open-Meteo (free, no key): its geocoding API turns the place into a spot, and its
//! daily forecast (16 days) gives the plan's day there. Only the server calls them, and it caches hard, in the
//! database so a restart keeps the cache: a place for [`GEOCODE_TTL_DAYS`] (a place it doesn't know too, so a typo
//! isn't asked about again and again), a spot's forecast for [`FORECAST_TTL_HOURS`]. When Open-Meteo can't be
//! reached, a stale forecast still answers; with none, there is no forecast. Calls go one at a time server-wide.

use std::sync::OnceLock;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
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

#[derive(Deserialize)]
struct GeoResult {
    name: String,
    latitude: f64,
    longitude: f64,
    #[serde(default)]
    country: Option<String>,
}

/// Open-Meteo's daily forecast, as cached.
#[derive(Debug, Deserialize)]
pub struct ForecastAnswer {
    #[serde(default)]
    pub utc_offset_seconds: i64,
    pub daily: Daily,
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
    now: DateTime<Utc>,
) -> AppResult<Option<Spot>> {
    let key = query_key(place);
    let fresh = stamp(now - TimeDelta::days(GEOCODE_TTL_DAYS));
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
        .append_pair("count", "1")
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
    Some(answer.results.into_iter().next().map(|g| Spot {
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
    now: DateTime<Utc>,
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
    let fresh = stamp(now - TimeDelta::hours(FORECAST_TTL_HOURS));
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

/// The day a plan starting at `starts_at` falls on at the spot (whose offset from UTC the forecast gives), out of
/// the forecast; `None` when that day isn't in it.
pub fn pick(
    answer: &ForecastAnswer,
    spot: &Spot,
    starts_at: DateTime<Utc>,
) -> Option<PlanForecast> {
    let local = starts_at + TimeDelta::seconds(answer.utc_offset_seconds);
    let date = local.format("%Y-%m-%d").to_string();
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
    })
}

/// Whether a plan starting at `starts_at` can have a forecast at `now`: it hasn't long started, and its day is
/// within the forecast's reach.
pub fn in_window(starts_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    starts_at > now - TimeDelta::hours(12)
        && starts_at < now + TimeDelta::days(plan_limits::FORECAST_DAYS)
}

/// The forecast for `place` on the day of `starts_at`, if there is one.
pub async fn forecast(
    db: &SqlitePool,
    cfg: &PlansConfig,
    place: &str,
    starts_at: DateTime<Utc>,
    now: DateTime<Utc>,
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

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
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
    }

    #[test]
    fn only_plans_within_the_forecast_s_reach_get_one() {
        let now = t("2026-11-16T10:00:00Z");
        assert!(in_window(t("2026-11-20T10:00:00Z"), now));
        assert!(in_window(t("2026-12-01T09:00:00Z"), now));
        assert!(!in_window(t("2026-12-02T11:00:00Z"), now));
        assert!(!in_window(t("2026-11-15T10:00:00Z"), now));
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
