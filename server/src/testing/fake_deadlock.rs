//! A stand-in for deadlock-api.com (#390) on 127.0.0.1, shaped after its OpenAPI spec. [`FakeDeadlock::configure`]
//! points a [`Config`] at it (and its art host at loopback), so nothing leaves the machine. Account ids here are the
//! fake SteamIDs `765000000000000xx` minus [`FAKE_STEAM_BASE`]: account `xx`.
//!
//! - `/v1/players/{id}/match-history`, `/v1/players/{id}/mate-stats`, `/v1/players/hero-stats?account_ids`,
//!   `/v1/players/rank?account_ids` (comma-separated): what the test gave [`FakeDeadlock::add_match`],
//!   [`FakeDeadlock::mate`], [`FakeDeadlock::hero_stats`] and [`FakeDeadlock::set_rank`].
//! - `/v1/assets/heroes`, `/v1/assets/ranks`: two heroes and three tiers, their art under `/art/` on [`ART_HOST`].
//! - `/v2/patches`: the updates given [`FakeDeadlock::add_patch`]. `/ISteamNews/GetNewsForApp/v2/`: the same, as
//!   Steam's news (a test points the Steam base here to see the fallback).
//! - [`FakeDeadlock::set_down`]: every API call answers 503; [`FakeDeadlock::limit_history`]: match history 429s.
//!
//! The key, when a test sets one, must arrive as `X-API-KEY`. Calls are counted ([`FakeDeadlock::calls`]).

use std::collections::HashMap;
use std::io::Cursor;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use reqwest::Url;

use crate::config::Config;

/// `765000000000000xx` − this = account `xx`.
pub const FAKE_STEAM_BASE: u64 = 76_500_000_000_000_000;
/// The art host the asset lists name.
pub const ART_HOST: &str = "deadlockart.test";
/// The heroes the fake knows.
pub const INFERNUS: u32 = 1;
pub const HAZE: u32 = 13;
const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

#[derive(Default)]
struct World {
    history: HashMap<u32, Vec<serde_json::Value>>,
    ranks: HashMap<u32, u32>,
    mates: HashMap<u32, Vec<(u32, u64, u64)>>,
    heroes: HashMap<u32, Vec<(u32, u64, u64)>>,
    patches: Vec<(String, String, String, String)>,
    down: bool,
    limited: bool,
    key: Option<String>,
    calls: HashMap<String, usize>,
}

#[derive(Clone)]
struct Shared {
    port: u16,
    world: Arc<Mutex<World>>,
    png: Arc<Vec<u8>>,
}

pub struct FakeDeadlock {
    pub port: u16,
    world: Arc<Mutex<World>>,
}

impl FakeDeadlock {
    pub async fn start() -> FakeDeadlock {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let world = Arc::new(Mutex::new(World::default()));
        let shared = Shared {
            port,
            world: world.clone(),
            png: Arc::new(png()),
        };
        let app = Router::new()
            .route("/v1/players/{id}/match-history", get(history))
            .route("/v1/players/{id}/mate-stats", get(mates))
            .route("/v1/players/hero-stats", get(hero_stats))
            .route("/v1/players/rank", get(ranks))
            .route("/v1/assets/heroes", get(heroes))
            .route("/v1/assets/ranks", get(rank_assets))
            .route("/v2/patches", get(patches))
            .route("/ISteamNews/GetNewsForApp/v2/", get(steam_news))
            .route("/art/{file}", get(picture))
            .with_state(shared);
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        FakeDeadlock { port, world }
    }

    /// Point `cfg` at the fake: the API, fake SteamIDs that convert, and its art host allowed and reachable.
    pub fn configure(&self, cfg: &mut Config) {
        cfg.deadlock.api_base = format!("http://127.0.0.1:{}", self.port);
        cfg.deadlock.steam_id_base = FAKE_STEAM_BASE;
        cfg.deadlock.art_hosts = vec![ART_HOST.into()];
        cfg.deadlock.retry_base = std::time::Duration::ZERO;
        cfg.embeds.allowed_ips.push(LOOPBACK);
        cfg.embeds.extra_ports.push(self.port);
        cfg.embeds.hosts.push((ART_HOST.into(), LOOPBACK));
    }

    /// Calls must carry this key (`X-API-KEY`).
    pub fn require_key(&self, key: &str) {
        self.world.lock().unwrap().key = Some(key.into());
    }

    /// A match in `account`'s history: `hero`, K/D/A, a win or loss, started `start_time`, and its rank delta and
    /// badge after (ranked only).
    #[allow(clippy::too_many_arguments)]
    pub fn add_match(
        &self,
        account: u32,
        match_id: u64,
        hero: u32,
        kda: (u32, u32, u32),
        won: bool,
        start_time: i64,
        ranked: Option<(i32, u32)>,
    ) {
        let mut v = serde_json::json!({
            "account_id": account, "match_id": match_id, "hero_id": hero, "hero_level": 30,
            "start_time": start_time, "game_mode": 1, "match_mode": if ranked.is_some() { 4 } else { 1 },
            "player_team": 0, "player_kills": kda.0, "player_deaths": kda.1, "player_assists": kda.2,
            "denies": 3, "net_worth": 30000, "last_hits": 150, "match_duration_s": 1864,
            "match_result": if won { 0 } else { 1 }, "objectives_mask_team0": 0, "objectives_mask_team1": 0,
            "player_match_outcome": if won { 1 } else { 2 },
        });
        if let Some((delta, badge)) = ranked {
            v["ranked_delta"] = delta.into();
            v["ranked_display_badge"] = badge.into();
        }
        let mut w = self.world.lock().unwrap();
        let h = w.history.entry(account).or_default();
        // newest first, as the real one answers
        h.insert(0, v);
    }

    pub fn set_rank(&self, account: u32, badge: u32) {
        self.world.lock().unwrap().ranks.insert(account, badge);
    }

    /// `account` played `games` with `mate` (on the same team) and won `wins`.
    pub fn mate(&self, account: u32, mate: u32, wins: u64, games: u64) {
        let mut w = self.world.lock().unwrap();
        w.mates
            .entry(account)
            .or_default()
            .push((mate, wins, games));
    }

    pub fn hero_stats(&self, account: u32, hero: u32, wins: u64, games: u64) {
        let mut w = self.world.lock().unwrap();
        w.heroes
            .entry(account)
            .or_default()
            .push((hero, wins, games));
    }

    /// An update in the feed (published RFC 3339, HTML content).
    pub fn add_patch(&self, title: &str, link: &str, published: &str, content: &str) {
        self.world.lock().unwrap().patches.push((
            title.into(),
            link.into(),
            published.into(),
            content.into(),
        ));
    }

    pub fn set_down(&self, down: bool) {
        self.world.lock().unwrap().down = down;
    }

    pub fn limit_history(&self, on: bool) {
        self.world.lock().unwrap().limited = on;
    }

    pub fn calls(&self, what: &str) -> usize {
        self.world
            .lock()
            .unwrap()
            .calls
            .get(what)
            .copied()
            .unwrap_or(0)
    }
}

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(48, 48, image::Rgba([90, 40, 160, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn ids(q: Option<String>) -> Vec<u32> {
    Url::parse(&format!("http://x/?{}", q.unwrap_or_default()))
        .ok()
        .and_then(|u| {
            u.query_pairs()
                .find(|(k, _)| k == "account_ids")
                .map(|(_, v)| v.split(',').filter_map(|i| i.trim().parse().ok()).collect())
        })
        .unwrap_or_default()
}

/// Count the call and check the key and the down switch; `Some` is the answer to give instead.
fn gate(s: &Shared, what: &str, h: &HeaderMap) -> Option<Response> {
    let mut w = s.world.lock().unwrap();
    *w.calls.entry(what.into()).or_default() += 1;
    if w.down {
        return Some((StatusCode::SERVICE_UNAVAILABLE, "down").into_response());
    }
    if let Some(k) = &w.key
        && h.get("x-api-key").and_then(|v| v.to_str().ok()) != Some(k.as_str())
    {
        return Some((StatusCode::UNAUTHORIZED, "key").into_response());
    }
    None
}

async fn history(State(s): State<Shared>, h: HeaderMap, Path(id): Path<u32>) -> Response {
    if let Some(r) = gate(&s, "match-history", &h) {
        return r;
    }
    let w = s.world.lock().unwrap();
    if w.limited {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "120")],
            "slow down",
        )
            .into_response();
    }
    axum::Json(w.history.get(&id).cloned().unwrap_or_default()).into_response()
}

async fn mates(State(s): State<Shared>, h: HeaderMap, Path(id): Path<u32>) -> Response {
    if let Some(r) = gate(&s, "mate-stats", &h) {
        return r;
    }
    let w = s.world.lock().unwrap();
    let list: Vec<_> = w
        .mates
        .get(&id)
        .into_iter()
        .flatten()
        .map(|(m, wins, games)| serde_json::json!({"mate_id": m, "wins": wins, "matches_played": games, "matches": []}))
        .collect();
    axum::Json(list).into_response()
}

async fn hero_stats(State(s): State<Shared>, h: HeaderMap, RawQuery(q): RawQuery) -> Response {
    if let Some(r) = gate(&s, "hero-stats", &h) {
        return r;
    }
    let w = s.world.lock().unwrap();
    let mut list = Vec::new();
    for a in ids(q) {
        for (hero, wins, games) in w.heroes.get(&a).into_iter().flatten() {
            list.push(serde_json::json!({"account_id": a, "hero_id": hero, "matches_played": games, "wins": wins,
                "last_played": 0, "time_played": 0, "kills": 0, "deaths": 0, "assists": 0}));
        }
    }
    axum::Json(list).into_response()
}

async fn ranks(State(s): State<Shared>, h: HeaderMap, RawQuery(q): RawQuery) -> Response {
    if let Some(r) = gate(&s, "rank", &h) {
        return r;
    }
    let w = s.world.lock().unwrap();
    let list: Vec<_> = ids(q)
        .into_iter()
        .map(|a| {
            let badge = w.ranks.get(&a).copied().unwrap_or(0);
            serde_json::json!({"account_id": a, "badge": badge, "rank": badge / 10, "subrank": badge % 10,
                "last_match": null})
        })
        .collect();
    axum::Json(list).into_response()
}

fn art(s: &Shared, file: &str) -> String {
    format!("http://{ART_HOST}:{}/art/{file}", s.port)
}

async fn heroes(State(s): State<Shared>, h: HeaderMap) -> Response {
    if let Some(r) = gate(&s, "heroes", &h) {
        return r;
    }
    axum::Json(serde_json::json!([
        {"id": INFERNUS, "name": "Infernus", "class_name": "hero_inferno", "images": {
            "icon_hero_card": art(&s, "inferno_card.png"), "icon_image_small": art(&s, "inferno_sm.png"),
            "name_image": null}},
        {"id": HAZE, "name": "Haze", "class_name": "hero_haze", "images": {
            "icon_hero_card": art(&s, "haze_card.png"), "icon_image_small": art(&s, "haze_sm.png"),
            // not an allowed host: never fetched
            "minimap_image": "https://elsewhere.example/haze.png"}}
    ]))
    .into_response()
}

async fn rank_assets(State(s): State<Shared>, h: HeaderMap) -> Response {
    if let Some(r) = gate(&s, "ranks", &h) {
        return r;
    }
    let tier = |t: u32, name: &str| {
        let mut images = serde_json::json!({"small": art(&s, &format!("rank{t}.png"))});
        for sub in 1..=6 {
            images[format!("small_subrank{sub}")] = art(&s, &format!("rank{t}_{sub}.png")).into();
        }
        serde_json::json!({"tier": t, "name": name, "color": "#000000", "images": images})
    };
    axum::Json(serde_json::json!([
        tier(7, "Emissary"),
        tier(8, "Oracle"),
        tier(9, "Phantom")
    ]))
    .into_response()
}

async fn patches(State(s): State<Shared>, h: HeaderMap) -> Response {
    if let Some(r) = gate(&s, "patches", &h) {
        return r;
    }
    let w = s.world.lock().unwrap();
    let list: Vec<_> = w
        .patches
        .iter()
        .rev()
        .map(|(title, link, published, content)| {
            serde_json::json!({"source": "steam", "title": title, "link": link, "pub_date": published,
                "guid": {"is_perma_link": true, "text": link}, "content": content})
        })
        .collect();
    axum::Json(list).into_response()
}

async fn steam_news(State(s): State<Shared>) -> Response {
    let mut w = s.world.lock().unwrap();
    *w.calls.entry("steam-news".into()).or_default() += 1;
    let items: Vec<_> = w
        .patches
        .iter()
        .map(|(title, link, published, content)| {
            let date = chrono::DateTime::parse_from_rfc3339(published)
                .map(|t| t.timestamp())
                .unwrap_or(0);
            serde_json::json!({"gid": "1", "title": title, "url": link, "contents": content, "date": date})
        })
        .collect();
    axum::Json(serde_json::json!({"appnews": {"appid": 1422450, "newsitems": items}}))
        .into_response()
}

async fn picture(State(s): State<Shared>, Path(_file): Path<String>) -> Response {
    *s.world
        .lock()
        .unwrap()
        .calls
        .entry("art".into())
        .or_default() += 1;
    ([(header::CONTENT_TYPE, "image/png")], s.png.to_vec()).into_response()
}
