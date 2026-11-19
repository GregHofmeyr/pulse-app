//! deadlock-api.com's endpoints Pulse reads (#390), shaped after its OpenAPI spec (`/openapi.json`). One client,
//! server-wide:
//!
//! - The optional key goes in the `X-API-KEY` header, never in a URL or a log line.
//! - A descriptive User-Agent with no personal details.
//! - 429 and 5xx (and the network failing) back the whole client off, honouring `Retry-After`; after
//!   [`DARK_AFTER`] failures in a row deadlock-api counts as gone dark: logged once, every Deadlock bit hides
//!   ([`Api::available`]) and it's retried with growing gaps until it answers.
//! - Match history from a bot friend costs one of a small hourly allowance (10/h per IP without a key, 300/h with
//!   one): [`Api::take_history`] keeps under it.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::config::DeadlockConfig;

/// Failures in a row after which deadlock-api counts as gone dark.
pub const DARK_AFTER: u32 = 3;
/// The longest the client waits between tries.
pub const MAX_BACKOFF: Duration = Duration::from_secs(3600);
/// Match history reads an hour without a key, and with one (deadlock-api allows 10 and 300: some headroom).
pub const HISTORY_PER_HOUR: u32 = 8;
pub const HISTORY_PER_HOUR_KEYED: u32 = 240;
/// The most accounts one batch rank call takes.
pub const RANK_BATCH: usize = 100;

/// Why a call gave nothing.
#[derive(Debug, PartialEq, Eq)]
pub enum ApiError {
    /// deadlock-api is down or the client is backing off: try again later, quietly.
    Down,
    /// This endpoint's rate limit (429): wait this long before calling it again.
    Limited(Duration),
    /// deadlock-api refused this request (a 4xx other than 429): nothing to do with its health.
    Refused(u16),
}

#[derive(Default)]
struct Health {
    fails: u32,
    until: Option<Instant>,
    warned: bool,
}

struct HourBudget {
    cap: u32,
    start: Instant,
    used: u32,
}

pub struct Api {
    http: reqwest::Client,
    base: String,
    health: Mutex<Health>,
    history: Mutex<HourBudget>,
    retry_base: Duration,
}

/// One match of a player's history (`PlayerMatchHistoryEntry`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct HistoryEntry {
    pub match_id: u64,
    pub hero_id: u32,
    pub start_time: i64,
    pub player_team: i64,
    pub player_kills: u32,
    pub player_deaths: u32,
    pub player_assists: u32,
    pub match_duration_s: u32,
    pub match_result: i64,
    /// 1 = win, 2 = loss, else scored otherwise.
    #[serde(default)]
    pub player_match_outcome: Option<i64>,
    #[serde(default)]
    pub ranked_delta: Option<i32>,
    #[serde(default)]
    pub ranked_display_badge: Option<u32>,
}

impl HistoryEntry {
    pub fn won(&self) -> bool {
        match self.player_match_outcome {
            Some(1) => true,
            Some(2) => false,
            _ => self.match_result == self.player_team,
        }
    }
}

/// One account's rank (`AccountRank`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct AccountRank {
    pub account_id: u32,
    pub badge: u32,
}

/// Matches with one teammate (`MateStats`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct MateStats {
    pub mate_id: u32,
    pub wins: u64,
    pub matches_played: u64,
}

/// One hero's totals for a player (`HeroStats`, the fields used).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct HeroStats {
    pub hero_id: u32,
    pub matches_played: u64,
    pub wins: u64,
}

/// A hero as `/v1/assets/heroes` describes it (the fields used); `images` maps names to URLs.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct HeroAsset {
    pub id: u32,
    pub name: String,
    #[serde(default)]
    pub images: std::collections::HashMap<String, Option<String>>,
}

/// A rank tier as `/v1/assets/ranks` describes it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct RankAsset {
    pub tier: u32,
    pub name: String,
    #[serde(default)]
    pub images: std::collections::HashMap<String, Option<String>>,
}

/// One update from `/v2/patches` (forum changelog or Steam news), or from Steam's own news feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    pub title: String,
    pub link: String,
    /// RFC 3339.
    pub published_at: String,
    /// HTML.
    pub content: String,
}

impl Api {
    pub fn new(cfg: &DeadlockConfig) -> Self {
        let mut headers = HeaderMap::new();
        if let Some(key) = &cfg.api_key
            && let Ok(mut v) = HeaderValue::from_str(key)
        {
            v.set_sensitive(true);
            headers.insert("X-API-KEY", v);
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!(
                "pulse-server/",
                env!("CARGO_PKG_VERSION"),
                " (self-hosted group chat; Deadlock match cards and ranks)"
            ))
            .default_headers(headers)
            .build()
            .expect("the deadlock-api HTTP client builds");
        let cap = if cfg.api_key.is_some() {
            HISTORY_PER_HOUR_KEYED
        } else {
            HISTORY_PER_HOUR
        };
        Self {
            http,
            base: cfg.api_base.trim_end_matches('/').to_string(),
            health: Mutex::default(),
            history: Mutex::new(HourBudget {
                cap,
                start: Instant::now(),
                used: 0,
            }),
            retry_base: cfg.retry_base,
        }
    }

    /// deadlock-api answers (it hasn't failed [`DARK_AFTER`] times in a row): the Deadlock bits show.
    pub fn available(&self) -> bool {
        self.health.lock().unwrap_or_else(|e| e.into_inner()).fails < DARK_AFTER
    }

    /// The client is between tries: nothing should be called now.
    pub fn backing_off(&self) -> bool {
        let h = self.health.lock().unwrap_or_else(|e| e.into_inner());
        h.until.is_some_and(|u| Instant::now() < u)
    }

    /// Take one match history read from this hour's allowance; false when it's spent.
    pub fn take_history(&self) -> bool {
        let mut b = self.history.lock().unwrap_or_else(|e| e.into_inner());
        if b.start.elapsed() >= Duration::from_secs(3600) {
            b.start = Instant::now();
            b.used = 0;
        }
        if b.used >= b.cap {
            return false;
        }
        b.used += 1;
        true
    }

    fn succeeded(&self) {
        let mut h = self.health.lock().unwrap_or_else(|e| e.into_inner());
        if h.warned {
            tracing::info!("deadlock-api answers again: Deadlock cards and stats are back");
        }
        *h = Health::default();
    }

    fn failed(&self, what: &str, why: &str, retry_after: Option<Duration>) {
        let mut h = self.health.lock().unwrap_or_else(|e| e.into_inner());
        h.fails = h.fails.saturating_add(1);
        let wait = retry_after
            .unwrap_or_else(|| backoff(self.retry_base, h.fails))
            .min(MAX_BACKOFF);
        h.until = Some(Instant::now() + wait);
        if h.fails >= DARK_AFTER && !h.warned {
            h.warned = true;
            tracing::warn!(
                what,
                why,
                "deadlock-api isn't answering: Deadlock cards and stats are hidden until it does (retrying with backoff)"
            );
        } else {
            tracing::debug!(what, why, fails = h.fails, "deadlock-api call failed");
        }
    }

    /// GET `path` with `query` and read the JSON answer.
    async fn get<T: DeserializeOwned>(
        &self,
        what: &'static str,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, ApiError> {
        if self.backing_off() {
            return Err(ApiError::Down);
        }
        let r = match self
            .http
            .get(format!("{}{path}", self.base))
            .query(query)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                self.failed(what, &e.without_url().to_string(), None);
                return Err(ApiError::Down);
            }
        };
        let status = r.status().as_u16();
        let retry_after = retry_after(r.headers());
        if status == 429 {
            // up, but this endpoint wants a pause: only its caller waits
            return Err(ApiError::Limited(
                retry_after
                    .unwrap_or(Duration::from_secs(600))
                    .min(MAX_BACKOFF),
            ));
        }
        if status >= 500 {
            self.failed(what, &format!("HTTP {status}"), retry_after);
            return Err(ApiError::Down);
        }
        if status != 200 {
            return Err(ApiError::Refused(status));
        }
        let body = match r.bytes().await {
            Ok(b) => b,
            Err(e) => {
                self.failed(what, &e.without_url().to_string(), None);
                return Err(ApiError::Down);
            }
        };
        match serde_json::from_slice(&body) {
            Ok(v) => {
                self.succeeded();
                Ok(v)
            }
            Err(e) => {
                self.failed(what, &format!("unreadable answer: {e}"), None);
                Err(ApiError::Down)
            }
        }
    }

    /// `/v1/players/{account_id}/match-history`: newest data when they're friends with a deadlock-api bot, else
    /// the stored history. Call [`Api::take_history`] first.
    pub async fn match_history(&self, account: u32) -> Result<Vec<HistoryEntry>, ApiError> {
        self.get(
            "match-history",
            &format!("/v1/players/{account}/match-history"),
            &[],
        )
        .await
    }

    /// `/v1/players/rank` for `accounts` (batched by [`RANK_BATCH`]); protected accounts are left out.
    pub async fn ranks(&self, accounts: &[u32]) -> Result<Vec<AccountRank>, ApiError> {
        let mut out = Vec::new();
        for chunk in accounts.chunks(RANK_BATCH) {
            let ids = chunk
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let got: Vec<AccountRank> = self
                .get("rank", "/v1/players/rank", &[("account_ids", ids)])
                .await?;
            out.extend(got);
        }
        Ok(out)
    }

    /// `/v1/players/{account_id}/mate-stats`: everyone they've had on their team, normal games.
    pub async fn mate_stats(&self, account: u32) -> Result<Vec<MateStats>, ApiError> {
        self.get(
            "mate-stats",
            &format!("/v1/players/{account}/mate-stats"),
            &[],
        )
        .await
    }

    /// `/v1/players/hero-stats` for one account.
    pub async fn hero_stats(&self, account: u32) -> Result<Vec<HeroStats>, ApiError> {
        self.get(
            "hero-stats",
            "/v1/players/hero-stats",
            &[("account_ids", account.to_string())],
        )
        .await
    }

    pub async fn heroes(&self) -> Result<Vec<HeroAsset>, ApiError> {
        self.get("heroes", "/v1/assets/heroes", &[]).await
    }

    pub async fn rank_assets(&self) -> Result<Vec<RankAsset>, ApiError> {
        self.get("ranks", "/v1/assets/ranks", &[]).await
    }

    /// `/v2/patches`: the forum changelog and Steam news, as one feed.
    pub async fn patches(&self) -> Result<Vec<Patch>, ApiError> {
        let raw: Vec<serde_json::Value> = self.get("patches", "/v2/patches", &[]).await?;
        Ok(raw.iter().filter_map(parse_feed_item).collect())
    }

    /// Steam's own news for Deadlock (`ISteamNews/GetNewsForApp`, no key), the fallback when `/v2/patches` fails.
    /// Not deadlock-api: its failures don't count against its health.
    pub async fn steam_news(&self, steam_base: &str) -> Option<Vec<Patch>> {
        let url = format!(
            "{}/ISteamNews/GetNewsForApp/v2/",
            steam_base.trim_end_matches('/')
        );
        let r = self
            .http
            .get(url)
            .query(&[
                ("appid", pulse_protocol::rest::DEADLOCK_APP_ID.to_string()),
                ("count", "10".into()),
                ("format", "json".into()),
            ])
            .send()
            .await
            .ok()?;
        if r.status().as_u16() != 200 {
            return None;
        }
        parse_steam_news(&r.bytes().await.ok()?)
    }
}

/// `base`, doubling per failure in a row.
fn backoff(base: Duration, fails: u32) -> Duration {
    base * 2u32.saturating_pow(fails.saturating_sub(1).min(7))
}

/// `Retry-After` in seconds (the form deadlock-api sends).
fn retry_after(h: &HeaderMap) -> Option<Duration> {
    let secs: u64 = h.get("retry-after")?.to_str().ok()?.trim().parse().ok()?;
    Some(Duration::from_secs(secs.max(1)))
}

/// One `/v2/patches` item: `title`, `link`, `pub_date`, `content` (forum and Steam items alike).
pub fn parse_feed_item(v: &serde_json::Value) -> Option<Patch> {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    let published = crate::times::parse(&s("pub_date")?).ok()?;
    Some(Patch {
        title: s("title")?,
        link: s("link")?,
        published_at: crate::db::stamp(published),
        content: s("content").unwrap_or_default(),
    })
}

/// `GetNewsForApp`: `appnews.newsitems[]` with `title`, `url`, `contents`, `date` (unix seconds).
pub fn parse_steam_news(body: &[u8]) -> Option<Vec<Patch>> {
    #[derive(Deserialize)]
    struct Outer {
        appnews: Inner,
    }
    #[derive(Deserialize)]
    struct Inner {
        #[serde(default)]
        newsitems: Vec<Item>,
    }
    #[derive(Deserialize)]
    struct Item {
        title: String,
        url: String,
        #[serde(default)]
        contents: String,
        date: i64,
    }
    let o: Outer = serde_json::from_slice(body).ok()?;
    Some(
        o.appnews
            .newsitems
            .into_iter()
            .filter_map(|i| {
                Some(Patch {
                    published_at: crate::db::stamp(jiff::Timestamp::from_second(i.date).ok()?),
                    title: i.title,
                    link: i.url,
                    content: i.contents,
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_entries_read_and_score() {
        let body = br#"[{"account_id":7,"match_id":50000002,"hero_id":1,"hero_level":30,"start_time":1763300000,
            "game_mode":1,"match_mode":4,"player_team":0,"player_kills":12,"player_deaths":3,"player_assists":8,
            "denies":4,"net_worth":40000,"last_hits":200,"match_duration_s":1864,"match_result":0,
            "objectives_mask_team0":1,"objectives_mask_team1":0,"player_match_outcome":1,"ranked_delta":112,
            "ranked_display_badge":83},
            {"account_id":7,"match_id":50000001,"hero_id":13,"hero_level":20,"start_time":1763290000,"game_mode":1,
            "match_mode":1,"player_team":1,"player_kills":2,"player_deaths":9,"player_assists":4,"denies":0,
            "net_worth":20000,"last_hits":90,"match_duration_s":1500,"match_result":0,"objectives_mask_team0":1,
            "objectives_mask_team1":0}]"#;
        let h: Vec<HistoryEntry> = serde_json::from_slice(body).unwrap();
        assert_eq!(h.len(), 2);
        assert!(h[0].won());
        assert_eq!(h[0].ranked_display_badge, Some(83));
        // no outcome: the winning team decides
        assert!(!h[1].won());
        assert_eq!(h[1].ranked_delta, None);
    }

    #[test]
    fn ranks_mates_and_heroes_read() {
        let r: Vec<AccountRank> = serde_json::from_slice(
            br#"[{"account_id":7,"badge":83,"rank":8,"subrank":3,"last_match":null}]"#,
        )
        .unwrap();
        assert_eq!(r[0].badge, 83);
        let m: Vec<MateStats> = serde_json::from_slice(
            br#"[{"mate_id":8,"wins":6,"matches_played":10,"matches":[1,2]}]"#,
        )
        .unwrap();
        assert_eq!(m[0].mate_id, 8);
        let h: Vec<HeroStats> = serde_json::from_slice(
            br#"[{"account_id":7,"hero_id":1,"matches_played":40,"wins":22,"kills":300}]"#,
        )
        .unwrap();
        assert_eq!(h[0].matches_played, 40);
        let a: Vec<HeroAsset> = serde_json::from_slice(
            br#"[{"id":1,"name":"Infernus","class_name":"hero_inferno","images":{"icon_image_small":"https://a/x.png","name_image":null}}]"#,
        )
        .unwrap();
        assert_eq!(
            a[0].images["icon_image_small"].as_deref(),
            Some("https://a/x.png")
        );
    }

    #[test]
    fn feed_items_from_patches_and_steam_news() {
        let item = serde_json::json!({"source":"steam","title":"Minor Update","pub_date":"2026-11-05T05:02:30Z",
            "link":"https://example.test/n/1","guid":{"is_perma_link":true,"text":"x"},"content":"<p>Haze</p>"});
        let p = parse_feed_item(&item).unwrap();
        assert_eq!(p.title, "Minor Update");
        assert!(p.published_at.starts_with("2026-11-05T05:02:30"));
        assert!(parse_feed_item(&serde_json::json!({"title":"x"})).is_none());
        let news = br#"{"appnews":{"appid":1422450,"newsitems":[{"gid":"1","title":"Gameplay Update","url":"https://example.test/n/2","contents":"Haze: more.","date":1762300000}]}}"#;
        let n = parse_steam_news(news).unwrap();
        assert_eq!(n[0].title, "Gameplay Update");
        assert!(parse_steam_news(b"<html>").is_none());
    }

    #[test]
    fn backoff_grows_and_caps() {
        let base = Duration::from_secs(30);
        assert_eq!(backoff(base, 1), Duration::from_secs(30));
        assert_eq!(backoff(base, 2), Duration::from_secs(60));
        assert!(backoff(base, 30) <= Duration::from_secs(30 * 128));
    }

    #[test]
    fn health_goes_dark_after_failures_and_back_on_success() {
        let api = Api::new(&DeadlockConfig::default());
        assert!(api.available());
        for _ in 0..DARK_AFTER {
            api.failed("t", "boom", Some(Duration::ZERO));
        }
        assert!(!api.available());
        api.succeeded();
        assert!(api.available() && !api.backing_off());
    }

    #[test]
    fn the_history_allowance_holds() {
        let api = Api::new(&DeadlockConfig::default());
        for _ in 0..HISTORY_PER_HOUR {
            assert!(api.take_history());
        }
        assert!(!api.take_history());
    }
}
