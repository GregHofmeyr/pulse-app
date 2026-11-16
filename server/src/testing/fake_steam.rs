//! A stand-in for Steam (#379) on 127.0.0.1: the Web API calls achievements make, the OpenID endpoint, achievement
//! icons and the store CDN's game art. [`FakeSteam::configure`] points a [`Config`] at it, with a key, so nothing
//! leaves the machine. SteamIDs here are obviously fake (`765000000000000xx`).
//!
//! - `/ISteamUserStats/GetPlayerAchievements/v1/?key&steamid&appid`: the player's achievements in a game added with
//!   [`FakeSteam::add_game`]; 403 "not public" for a [`FakeSteam::set_private`] player; 400 "no stats" for a game
//!   without achievements or one never added.
//! - `/ISteamUserStats/GetSchemaForGame/v2/`, `/ISteamUserStats/GetGlobalAchievementPercentagesForApp/v2/`.
//! - `/IPlayerService/GetOwnedGames/v1/`: the games given [`FakeSteam::own`]; an empty response when private.
//! - `/ISteamUser/GetPlayerSummaries/v2/`: the persona from [`FakeSteam::player`].
//! - `/openid/login`: `checkid_setup` redirects to `return_to` signed in as [`FakeSteam::sign_in_as`]; a POST with
//!   `check_authentication` says `is_valid:true` once for a signature it issued.
//! - `/icons/<file>` and `/steam/apps/<appid>/<file>`: a PNG.
//!
//! Every Web API call needs [`FAKE_KEY`] (403 otherwise). Calls are counted ([`FakeSteam::calls`]).

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, RawQuery, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use reqwest::Url;

use crate::config::Config;

/// The key the fake accepts.
pub const FAKE_KEY: &str = "fake-steam-key";
/// The icon host the schema names.
pub const ICON_HOST: &str = "steamicons.test";
const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const CLAIMED_PREFIX: &str = "https://steamcommunity.com/openid/id/";

#[derive(Default)]
struct Game {
    name: String,
    /// (api name, display name, description), in order.
    achievements: Vec<(String, String, String)>,
    percent: HashMap<String, f32>,
}

#[derive(Default)]
struct Player {
    persona: String,
    private: bool,
    /// app → playtime minutes
    owned: Vec<(u32, u64)>,
    /// app → api name → unlock time
    unlocked: HashMap<u32, HashMap<String, i64>>,
}

#[derive(Default)]
struct World {
    games: HashMap<u32, Game>,
    players: HashMap<String, Player>,
    next_login: Option<String>,
    issued_sigs: HashSet<String>,
    /// Web API calls by method name.
    calls: HashMap<String, usize>,
}

#[derive(Clone)]
struct Shared {
    port: u16,
    world: Arc<Mutex<World>>,
    png: Arc<Vec<u8>>,
}

pub struct FakeSteam {
    pub port: u16,
    world: Arc<Mutex<World>>,
}

impl FakeSteam {
    pub async fn start() -> FakeSteam {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let world = Arc::new(Mutex::new(World::default()));
        let shared = Shared {
            port,
            world: world.clone(),
            png: Arc::new(png()),
        };
        let app = Router::new()
            .route(
                "/ISteamUserStats/GetPlayerAchievements/v1/",
                get(player_achievements),
            )
            .route("/ISteamUserStats/GetSchemaForGame/v2/", get(schema))
            .route(
                "/ISteamUserStats/GetGlobalAchievementPercentagesForApp/v2/",
                get(percentages),
            )
            .route("/IPlayerService/GetOwnedGames/v1/", get(owned))
            .route("/ISteamUser/GetPlayerSummaries/v2/", get(summaries))
            .route("/openid/login", get(login).post(check))
            .route("/icons/{file}", get(picture))
            .route("/steam/apps/{app}/{file}", get(picture_app))
            .with_state(shared);
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        FakeSteam { port, world }
    }

    /// Point `cfg` at the fake: the Web API (with [`FAKE_KEY`]), OpenID, game art, and the guarded fetcher allowed to
    /// reach it for icons. Sets a public URL when there's none.
    pub fn configure(&self, cfg: &mut Config) {
        cfg.steam.api_key = Some(FAKE_KEY.into());
        cfg.steam.api_base = format!("http://127.0.0.1:{}", self.port);
        cfg.steam.openid_endpoint = self.openid_endpoint();
        cfg.steam.backfill_gap = std::time::Duration::from_millis(1);
        if cfg.embeds.public_url.is_none() {
            cfg.embeds.public_url = Some(Url::parse("https://pulse.example.test").unwrap());
        }
        cfg.embeds.endpoints.steam_cdn = format!("http://{ICON_HOST}:{}/steam/apps", self.port);
        cfg.embeds.allowed_ips.push(LOOPBACK);
        cfg.embeds.extra_ports.push(self.port);
        cfg.embeds.hosts.push((ICON_HOST.into(), LOOPBACK));
    }

    pub fn openid_endpoint(&self) -> String {
        format!("http://127.0.0.1:{}/openid/login", self.port)
    }

    /// A game with these achievements (api name, display name, description), each held by `percent`% of players
    /// (the first `percent`, the second half that, …).
    pub fn add_game(
        &self,
        app: u32,
        name: &str,
        achievements: &[(&str, &str, &str)],
        percent: f32,
    ) {
        let mut w = self.world.lock().unwrap();
        let mut p = percent;
        let mut g = Game {
            name: name.into(),
            ..Game::default()
        };
        for (api, display, desc) in achievements {
            g.achievements
                .push((api.to_string(), display.to_string(), desc.to_string()));
            g.percent.insert(api.to_string(), p);
            p /= 2.0;
        }
        w.games.insert(app, g);
    }

    /// A player with this persona.
    pub fn player(&self, steam_id: &str, persona: &str) {
        let mut w = self.world.lock().unwrap();
        w.players.entry(steam_id.into()).or_default().persona = persona.into();
    }

    /// `steam_id` owns `app` with this much play time.
    pub fn own(&self, steam_id: &str, app: u32, minutes: u64) {
        let mut w = self.world.lock().unwrap();
        w.players
            .entry(steam_id.into())
            .or_default()
            .owned
            .push((app, minutes));
    }

    /// `steam_id` unlocked `api` in `app` at `at` (unix seconds).
    pub fn unlock(&self, steam_id: &str, app: u32, api: &str, at: i64) {
        let mut w = self.world.lock().unwrap();
        w.players
            .entry(steam_id.into())
            .or_default()
            .unlocked
            .entry(app)
            .or_default()
            .insert(api.into(), at);
    }

    pub fn set_private(&self, steam_id: &str, private: bool) {
        let mut w = self.world.lock().unwrap();
        w.players.entry(steam_id.into()).or_default().private = private;
    }

    /// The next OpenID sign-in is as `steam_id`.
    pub fn sign_in_as(&self, steam_id: &str) {
        self.world.lock().unwrap().next_login = Some(steam_id.into());
    }

    /// Web API calls of `method` so far (`GetPlayerAchievements`, `GetOwnedGames`, …).
    pub fn calls(&self, method: &str) -> usize {
        self.world
            .lock()
            .unwrap()
            .calls
            .get(method)
            .copied()
            .unwrap_or(0)
    }

    pub fn total_calls(&self) -> usize {
        self.world.lock().unwrap().calls.values().sum()
    }
}

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(64, 64, image::Rgba([200, 120, 40, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn query(q: Option<String>) -> HashMap<String, String> {
    Url::parse(&format!("http://x/?{}", q.unwrap_or_default()))
        .map(|u| u.query_pairs().into_owned().collect())
        .unwrap_or_default()
}

fn json(status: StatusCode, v: serde_json::Value) -> Response {
    (status, axum::Json(v)).into_response()
}

/// Count the call and check the key; `Some` is the answer to give instead.
fn keyed(
    s: &Shared,
    method: &str,
    q: &HashMap<String, String>,
    need_key: bool,
) -> Option<Response> {
    s.world
        .lock()
        .unwrap()
        .calls
        .entry(method.into())
        .and_modify(|n| *n += 1)
        .or_insert(1);
    if need_key && q.get("key").map(String::as_str) != Some(FAKE_KEY) {
        return Some((StatusCode::FORBIDDEN, "<html>Forbidden</html>").into_response());
    }
    None
}

async fn player_achievements(State(s): State<Shared>, RawQuery(q): RawQuery) -> Response {
    let q = query(q);
    if let Some(r) = keyed(&s, "GetPlayerAchievements", &q, true) {
        return r;
    }
    let w = s.world.lock().unwrap();
    let steam_id = q.get("steamid").cloned().unwrap_or_default();
    let app: u32 = q.get("appid").and_then(|a| a.parse().ok()).unwrap_or(0);
    let player = w.players.get(&steam_id);
    if player.is_some_and(|p| p.private) {
        return json(
            StatusCode::FORBIDDEN,
            serde_json::json!({"playerstats": {"error": "Profile is not public", "success": false}}),
        );
    }
    let Some(game) = w.games.get(&app).filter(|g| !g.achievements.is_empty()) else {
        return json(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"playerstats": {"error": "Requested app has no stats", "success": false}}),
        );
    };
    let got = player.and_then(|p| p.unlocked.get(&app));
    let list: Vec<_> = game
        .achievements
        .iter()
        .map(|(api, _, _)| {
            let at = got.and_then(|g| g.get(api)).copied();
            serde_json::json!({"apiname": api, "achieved": u8::from(at.is_some()), "unlocktime": at.unwrap_or(0)})
        })
        .collect();
    json(
        StatusCode::OK,
        serde_json::json!({"playerstats": {"steamID": steam_id, "gameName": game.name, "achievements": list, "success": true}}),
    )
}

async fn schema(State(s): State<Shared>, RawQuery(q): RawQuery) -> Response {
    let q = query(q);
    if let Some(r) = keyed(&s, "GetSchemaForGame", &q, true) {
        return r;
    }
    let w = s.world.lock().unwrap();
    let app: u32 = q.get("appid").and_then(|a| a.parse().ok()).unwrap_or(0);
    let Some(game) = w.games.get(&app) else {
        return json(StatusCode::OK, serde_json::json!({}));
    };
    let list: Vec<_> = game
        .achievements
        .iter()
        .map(|(api, display, desc)| {
            serde_json::json!({
                "name": api, "defaultvalue": 0, "displayName": display, "hidden": 0, "description": desc,
                "icon": format!("http://{ICON_HOST}:{}/icons/{app}_{api}.png", s.port),
                "icongray": format!("http://{ICON_HOST}:{}/icons/{app}_{api}_gray.png", s.port),
            })
        })
        .collect();
    json(
        StatusCode::OK,
        serde_json::json!({"game": {"gameName": game.name, "gameVersion": "1", "availableGameStats": {"achievements": list}}}),
    )
}

async fn percentages(State(s): State<Shared>, RawQuery(q): RawQuery) -> Response {
    let q = query(q);
    if let Some(r) = keyed(&s, "GetGlobalAchievementPercentagesForApp", &q, false) {
        return r;
    }
    let w = s.world.lock().unwrap();
    let app: u32 = q.get("gameid").and_then(|a| a.parse().ok()).unwrap_or(0);
    let list: Vec<_> = w
        .games
        .get(&app)
        .map(|g| {
            g.percent
                .iter()
                .map(|(k, v)| serde_json::json!({"name": k, "percent": format!("{v:.1}")}))
                .collect()
        })
        .unwrap_or_default();
    json(
        StatusCode::OK,
        serde_json::json!({"achievementpercentages": {"achievements": list}}),
    )
}

async fn owned(State(s): State<Shared>, RawQuery(q): RawQuery) -> Response {
    let q = query(q);
    if let Some(r) = keyed(&s, "GetOwnedGames", &q, true) {
        return r;
    }
    let w = s.world.lock().unwrap();
    let steam_id = q.get("steamid").cloned().unwrap_or_default();
    let Some(p) = w.players.get(&steam_id).filter(|p| !p.private) else {
        return json(StatusCode::OK, serde_json::json!({"response": {}}));
    };
    let games: Vec<_> = p
        .owned
        .iter()
        .map(|(app, minutes)| {
            let g = w.games.get(app);
            serde_json::json!({
                "appid": app, "name": g.map(|g| g.name.clone()), "playtime_forever": minutes,
                "has_community_visible_stats": g.is_some_and(|g| !g.achievements.is_empty()),
            })
        })
        .collect();
    json(
        StatusCode::OK,
        serde_json::json!({"response": {"game_count": games.len(), "games": games}}),
    )
}

async fn summaries(State(s): State<Shared>, RawQuery(q): RawQuery) -> Response {
    let q = query(q);
    if let Some(r) = keyed(&s, "GetPlayerSummaries", &q, true) {
        return r;
    }
    let w = s.world.lock().unwrap();
    let id = q.get("steamids").cloned().unwrap_or_default();
    let players: Vec<_> = w
        .players
        .get(&id)
        .map(|p| serde_json::json!({"steamid": id, "personaname": p.persona}))
        .into_iter()
        .collect();
    json(
        StatusCode::OK,
        serde_json::json!({"response": {"players": players}}),
    )
}

/// `checkid_setup`: straight back to `return_to`, signed in as the next login.
async fn login(State(s): State<Shared>, RawQuery(q): RawQuery) -> Response {
    let q = query(q);
    let (Some(ret), Some(id)) = (
        q.get("openid.return_to"),
        s.world.lock().unwrap().next_login.clone(),
    ) else {
        return (StatusCode::BAD_REQUEST, "no return_to or nobody to sign in").into_response();
    };
    let sig = hex::encode(rand::random::<[u8; 16]>());
    s.world.lock().unwrap().issued_sigs.insert(sig.clone());
    let claimed = format!("{CLAIMED_PREFIX}{id}");
    let endpoint = format!("http://127.0.0.1:{}/openid/login", s.port);
    let mut back = Url::parse(ret).unwrap();
    back.query_pairs_mut()
        .append_pair("openid.ns", "http://specs.openid.net/auth/2.0")
        .append_pair("openid.mode", "id_res")
        .append_pair("openid.op_endpoint", &endpoint)
        .append_pair("openid.claimed_id", &claimed)
        .append_pair("openid.identity", &claimed)
        .append_pair("openid.return_to", ret)
        .append_pair("openid.response_nonce", "2026-11-16T10:00:00Zfake")
        .append_pair("openid.assoc_handle", "1234567890")
        .append_pair(
            "openid.signed",
            "signed,op_endpoint,claimed_id,identity,return_to,response_nonce,assoc_handle",
        )
        .append_pair("openid.sig", &sig);
    (StatusCode::FOUND, [(header::LOCATION, back.to_string())]).into_response()
}

/// `check_authentication`: valid once for a signature it issued, with every field as sent.
async fn check(State(s): State<Shared>, body: String) -> Response {
    let q = query(Some(body));
    let ok = q.get("openid.mode").map(String::as_str) == Some("check_authentication")
        && q.get("openid.sig")
            .is_some_and(|sig| s.world.lock().unwrap().issued_sigs.remove(sig));
    format!("ns:http://specs.openid.net/auth/2.0\nis_valid:{ok}\n").into_response()
}

async fn picture(State(s): State<Shared>, Path(_file): Path<String>) -> Response {
    ([(header::CONTENT_TYPE, "image/png")], s.png.to_vec()).into_response()
}

async fn picture_app(
    State(s): State<Shared>,
    Path((_app, _file)): Path<(String, String)>,
) -> Response {
    ([(header::CONTENT_TYPE, "image/png")], s.png.to_vec()).into_response()
}
