//! The Steam Web API calls achievements need (#379), and Steam's OpenID `check_authentication`. Every Web API call
//! takes one from the server-wide daily allowance ([`Budget`]); the key is only ever put in the request URL, which is
//! never logged (errors say which call failed and how, never the URL).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use reqwest::Url;
use serde::Deserialize;

use crate::config::SteamConfig;

/// Why a call gave nothing.
#[derive(Debug, PartialEq, Eq)]
pub enum ApiError {
    /// The server has no Steam Web API key.
    NoKey,
    /// Today's allowance is spent.
    Budget,
    /// Steam won't show this person's game details.
    Private,
    /// The game has no achievements or stats.
    NoStats,
    /// Anything else (network, a 5xx, an answer we can't read): try again later.
    Failed(String),
}

/// One achievement as a player has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerAchievement {
    pub api_name: String,
    pub achieved: bool,
    /// When it was unlocked (unix seconds); 0 when not.
    pub unlock_time: i64,
}

/// A player's achievements in one game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerStats {
    pub game: Option<String>,
    pub achievements: Vec<PlayerAchievement>,
}

/// One achievement of a game's schema.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct SchemaAchievement {
    pub api_name: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
}

/// A game's achievements as Steam names them.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct Schema {
    pub game: Option<String>,
    pub achievements: Vec<SchemaAchievement>,
}

/// A game someone owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedGame {
    pub app_id: u32,
    pub name: Option<String>,
    pub playtime_minutes: u64,
    pub has_stats: bool,
}

/// A game someone played in the last two weeks (#383).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentGame {
    pub app_id: u32,
    /// Minutes in the last two weeks.
    pub minutes_2weeks: u64,
}

/// What a player's summary says of the game they're in (#381): raw, as Steam gives it; the poller checks it
/// ([`super::join`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InGame {
    pub steam_id: String,
    /// `gameid`: the app they're in, when Steam shows it.
    pub game_id: Option<String>,
    /// `lobbysteamid`.
    pub lobby: Option<String>,
    /// `gameserverip`: `ip:port`.
    pub server: Option<String>,
}

/// The most SteamIDs one `GetPlayerSummaries` call takes.
pub const SUMMARIES_MAX: usize = 100;

/// The daily allowance: `cap` calls per UTC day, server-wide.
pub struct Budget {
    cap: u32,
    state: Mutex<(i64, u32, bool)>,
}

impl Budget {
    pub fn new(cap: u32) -> Self {
        Self {
            cap,
            state: Mutex::new((0, 0, false)),
        }
    }

    /// Take one call on `day` (days since the epoch); false when today's are spent (warned once a day).
    pub fn take_on(&self, day: i64) -> bool {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.0 != day {
            *st = (day, 0, false);
        }
        if st.1 >= self.cap {
            if !st.2 {
                st.2 = true;
                tracing::warn!(
                    cap = self.cap,
                    "Steam Web API daily allowance spent; achievements pause until tomorrow (UTC)"
                );
            }
            return false;
        }
        st.1 += 1;
        true
    }

    pub fn take(&self) -> bool {
        self.take_on(chrono::Utc::now().timestamp().div_euclid(86_400))
    }

    /// Calls made today.
    pub fn used(&self) -> u32 {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).1
    }
}

pub struct Api {
    http: reqwest::Client,
    key: Option<String>,
    base: String,
    openid_endpoint: String,
    pub budget: Budget,
}

impl Api {
    pub fn new(cfg: &SteamConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("pulse-server")
            .build()
            .expect("the Steam HTTP client builds");
        Self {
            http,
            key: cfg.api_key.clone(),
            base: cfg.api_base.trim_end_matches('/').to_string(),
            openid_endpoint: cfg.openid_endpoint.clone(),
            budget: Budget::new(cfg.daily_cap),
        }
    }

    pub fn has_key(&self) -> bool {
        self.key.is_some()
    }

    fn url(&self, path: &str, params: &[(&str, &str)], keyed: bool) -> Result<Url, ApiError> {
        let mut url = Url::parse(&format!("{}{path}", self.base))
            .map_err(|_| ApiError::Failed("bad Steam API base".into()))?;
        {
            let mut q = url.query_pairs_mut();
            if keyed {
                q.append_pair("key", self.key.as_deref().ok_or(ApiError::NoKey)?);
            }
            for (k, v) in params {
                q.append_pair(k, v);
            }
            q.append_pair("format", "json");
        }
        Ok(url)
    }

    /// GET `path` and hand the status and body to `read`. One call of the allowance.
    async fn call<T>(
        &self,
        what: &'static str,
        url: Url,
        read: impl FnOnce(u16, &[u8]) -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        if !self.budget.take() {
            return Err(ApiError::Budget);
        }
        let r = self.http.get(url).send().await.map_err(|e| {
            // without the URL: it carries the key
            ApiError::Failed(format!("{what}: {}", e.without_url()))
        })?;
        let status = r.status().as_u16();
        let body = r
            .bytes()
            .await
            .map_err(|e| ApiError::Failed(format!("{what}: {}", e.without_url())))?;
        read(status, &body)
    }

    pub async fn player_achievements(
        &self,
        steam_id: &str,
        app_id: u32,
    ) -> Result<PlayerStats, ApiError> {
        let app = app_id.to_string();
        let url = self.url(
            "/ISteamUserStats/GetPlayerAchievements/v1/",
            &[("steamid", steam_id), ("appid", &app)],
            true,
        )?;
        self.call("GetPlayerAchievements", url, parse_player_achievements)
            .await
    }

    pub async fn schema(&self, app_id: u32) -> Result<Schema, ApiError> {
        let app = app_id.to_string();
        let url = self.url(
            "/ISteamUserStats/GetSchemaForGame/v2/",
            &[("appid", &app), ("l", "english")],
            true,
        )?;
        self.call("GetSchemaForGame", url, parse_schema).await
    }

    pub async fn global_percentages(&self, app_id: u32) -> Result<HashMap<String, f32>, ApiError> {
        let app = app_id.to_string();
        let url = self.url(
            "/ISteamUserStats/GetGlobalAchievementPercentagesForApp/v2/",
            &[("gameid", &app)],
            false,
        )?;
        self.call(
            "GetGlobalAchievementPercentagesForApp",
            url,
            parse_percentages,
        )
        .await
    }

    /// The games `steam_id` owns; `Err(Private)` when Steam shows none of them (a private profile answers an empty
    /// response).
    pub async fn owned_games(&self, steam_id: &str) -> Result<Vec<OwnedGame>, ApiError> {
        let url = self.url(
            "/IPlayerService/GetOwnedGames/v1/",
            &[
                ("steamid", steam_id),
                ("include_appinfo", "1"),
                ("include_played_free_games", "1"),
            ],
            true,
        )?;
        self.call("GetOwnedGames", url, parse_owned).await
    }

    /// What `steam_id` played in the last two weeks (#383); `Err(Private)` when Steam shows nothing (a private
    /// profile answers an empty response, someone who played nothing a `total_count` of 0).
    pub async fn recent_games(&self, steam_id: &str) -> Result<Vec<RecentGame>, ApiError> {
        let url = self.url(
            "/IPlayerService/GetRecentlyPlayedGames/v1/",
            &[("steamid", steam_id)],
            true,
        )?;
        self.call("GetRecentlyPlayedGames", url, parse_recent).await
    }

    /// The game each of `steam_ids` (at most [`SUMMARIES_MAX`]) is in, from one `GetPlayerSummaries` (#381). Players
    /// Steam leaves out (private, unknown) are missing.
    pub async fn in_game(&self, steam_ids: &[&str]) -> Result<Vec<InGame>, ApiError> {
        if steam_ids.is_empty() {
            return Ok(vec![]);
        }
        let ids = steam_ids[..steam_ids.len().min(SUMMARIES_MAX)].join(",");
        let url = self.url(
            "/ISteamUser/GetPlayerSummaries/v2/",
            &[("steamids", &ids)],
            true,
        )?;
        self.call("GetPlayerSummaries", url, parse_in_game).await
    }

    /// `steam_id`'s persona name; `None` without a key or when Steam doesn't say.
    pub async fn persona(&self, steam_id: &str) -> Option<String> {
        if !self.has_key() {
            return None;
        }
        let url = self
            .url(
                "/ISteamUser/GetPlayerSummaries/v2/",
                &[("steamids", steam_id)],
                true,
            )
            .ok()?;
        self.call("GetPlayerSummaries", url, |status, body| {
            parse_persona(status, body, steam_id).ok_or(ApiError::Failed("no persona".into()))
        })
        .await
        .ok()
    }

    /// Ask Steam whether the OpenID assertion in `params` (every `openid.*` field it sent) is its own. Not a Web API
    /// call: no key, not counted.
    pub async fn check_authentication(&self, params: &[(String, String)]) -> bool {
        let body = super::openid::check_body(params);
        let r = self
            .http
            .post(&self.openid_endpoint)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body)
            .send()
            .await;
        match r {
            Ok(r) if r.status().is_success() => match r.text().await {
                Ok(t) => super::openid::is_valid(&t),
                Err(_) => false,
            },
            Ok(r) => {
                tracing::warn!(status = r.status().as_u16(), "Steam OpenID check refused");
                false
            }
            Err(e) => {
                tracing::warn!(error = %e.without_url(), "Steam OpenID check failed");
                false
            }
        }
    }
}

fn json<'a, T: Deserialize<'a>>(what: &str, body: &'a [u8]) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|e| ApiError::Failed(format!("{what}: {e}")))
}

/// `GetPlayerAchievements`: 200 with `playerstats.achievements`; 403 "not public"; 400 "no stats".
pub fn parse_player_achievements(status: u16, body: &[u8]) -> Result<PlayerStats, ApiError> {
    #[derive(Deserialize)]
    struct Outer {
        playerstats: Option<Inner>,
    }
    #[derive(Deserialize)]
    struct Inner {
        #[serde(default)]
        success: bool,
        #[serde(default)]
        error: Option<String>,
        #[serde(rename = "gameName", default)]
        game_name: Option<String>,
        #[serde(default)]
        achievements: Vec<Ach>,
    }
    #[derive(Deserialize)]
    struct Ach {
        apiname: String,
        achieved: u8,
        #[serde(default)]
        unlocktime: i64,
    }
    let error_text = || -> String {
        serde_json::from_slice::<Outer>(body)
            .ok()
            .and_then(|o| o.playerstats?.error)
            .unwrap_or_default()
            .to_ascii_lowercase()
    };
    if status == 403 || error_text().contains("not public") {
        return Err(ApiError::Private);
    }
    if status == 400 && error_text().contains("no stats") {
        return Err(ApiError::NoStats);
    }
    if status != 200 {
        return Err(ApiError::Failed(format!(
            "GetPlayerAchievements: HTTP {status}"
        )));
    }
    let o: Outer = json("GetPlayerAchievements", body)?;
    let Some(p) = o.playerstats else {
        return Err(ApiError::Failed(
            "GetPlayerAchievements: no playerstats".into(),
        ));
    };
    if !p.success {
        let e = p.error.unwrap_or_default().to_ascii_lowercase();
        return Err(if e.contains("not public") {
            ApiError::Private
        } else if e.contains("no stats") {
            ApiError::NoStats
        } else {
            ApiError::Failed("GetPlayerAchievements: not a success".into())
        });
    }
    Ok(PlayerStats {
        game: p.game_name.filter(|g| !g.trim().is_empty()),
        achievements: p
            .achievements
            .into_iter()
            .map(|a| PlayerAchievement {
                api_name: a.apiname,
                achieved: a.achieved != 0,
                unlock_time: a.unlocktime,
            })
            .collect(),
    })
}

/// `GetSchemaForGame`: `game.availableGameStats.achievements` (absent for a game without any).
pub fn parse_schema(status: u16, body: &[u8]) -> Result<Schema, ApiError> {
    if status != 200 {
        return Err(ApiError::Failed(format!("GetSchemaForGame: HTTP {status}")));
    }
    #[derive(Deserialize)]
    struct Outer {
        #[serde(default)]
        game: Option<Game>,
    }
    #[derive(Deserialize)]
    struct Game {
        #[serde(rename = "gameName", default)]
        game_name: Option<String>,
        #[serde(rename = "availableGameStats", default)]
        stats: Option<Stats>,
    }
    #[derive(Deserialize)]
    struct Stats {
        #[serde(default)]
        achievements: Vec<Ach>,
    }
    #[derive(Deserialize)]
    struct Ach {
        name: String,
        #[serde(rename = "displayName", default)]
        display_name: Option<String>,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        icon: Option<String>,
    }
    let o: Outer = json("GetSchemaForGame", body)?;
    let Some(g) = o.game else {
        return Ok(Schema::default());
    };
    Ok(Schema {
        game: g.game_name.filter(|n| !n.trim().is_empty()),
        achievements: g
            .stats
            .map(|s| s.achievements)
            .unwrap_or_default()
            .into_iter()
            .map(|a| SchemaAchievement {
                name: a
                    .display_name
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| a.name.clone()),
                api_name: a.name,
                description: a.description.filter(|d| !d.trim().is_empty()),
                icon: a.icon.filter(|i| !i.is_empty()),
            })
            .collect(),
    })
}

/// `GetGlobalAchievementPercentagesForApp`: each `percent` a number or (newer answers) a string.
pub fn parse_percentages(status: u16, body: &[u8]) -> Result<HashMap<String, f32>, ApiError> {
    if status != 200 {
        return Err(ApiError::Failed(format!(
            "GetGlobalAchievementPercentagesForApp: HTTP {status}"
        )));
    }
    #[derive(Deserialize)]
    struct Outer {
        achievementpercentages: Option<Inner>,
    }
    #[derive(Deserialize)]
    struct Inner {
        #[serde(default)]
        achievements: Vec<Ach>,
    }
    #[derive(Deserialize)]
    struct Ach {
        name: String,
        percent: serde_json::Value,
    }
    let o: Outer = json("GetGlobalAchievementPercentagesForApp", body)?;
    Ok(o.achievementpercentages
        .map(|i| i.achievements)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|a| {
            let p = match &a.percent {
                serde_json::Value::Number(n) => n.as_f64()?,
                serde_json::Value::String(s) => s.trim().parse().ok()?,
                _ => return None,
            };
            (p.is_finite() && (0.0..=100.0).contains(&p)).then_some((a.name, p as f32))
        })
        .collect())
}

/// `GetOwnedGames`: `response.games`; an empty `response` is a private profile.
pub fn parse_owned(status: u16, body: &[u8]) -> Result<Vec<OwnedGame>, ApiError> {
    if status == 403 {
        return Err(ApiError::Private);
    }
    if status != 200 {
        return Err(ApiError::Failed(format!("GetOwnedGames: HTTP {status}")));
    }
    #[derive(Deserialize)]
    struct Outer {
        response: Option<Inner>,
    }
    #[derive(Deserialize)]
    struct Inner {
        #[serde(default)]
        games: Option<Vec<G>>,
    }
    #[derive(Deserialize)]
    struct G {
        appid: u32,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        playtime_forever: u64,
        #[serde(default)]
        has_community_visible_stats: bool,
    }
    let o: Outer = json("GetOwnedGames", body)?;
    let Some(games) = o.response.and_then(|r| r.games) else {
        return Err(ApiError::Private);
    };
    Ok(games
        .into_iter()
        .map(|g| OwnedGame {
            app_id: g.appid,
            name: g.name,
            playtime_minutes: g.playtime_forever,
            has_stats: g.has_community_visible_stats,
        })
        .collect())
}

/// `GetRecentlyPlayedGames`: `response.games` (absent when they played nothing, with `total_count` 0); an empty
/// `response` is a private profile.
pub fn parse_recent(status: u16, body: &[u8]) -> Result<Vec<RecentGame>, ApiError> {
    if status == 403 {
        return Err(ApiError::Private);
    }
    if status != 200 {
        return Err(ApiError::Failed(format!(
            "GetRecentlyPlayedGames: HTTP {status}"
        )));
    }
    #[derive(Deserialize)]
    struct Outer {
        response: Option<Inner>,
    }
    #[derive(Deserialize)]
    struct Inner {
        #[serde(default)]
        total_count: Option<u64>,
        #[serde(default)]
        games: Option<Vec<G>>,
    }
    #[derive(Deserialize)]
    struct G {
        appid: u32,
        #[serde(default)]
        playtime_2weeks: u64,
    }
    let o: Outer = json("GetRecentlyPlayedGames", body)?;
    let Some(r) = o.response else {
        return Err(ApiError::Private);
    };
    match (r.total_count, r.games) {
        (_, Some(games)) => Ok(games
            .into_iter()
            .map(|g| RecentGame {
                app_id: g.appid,
                // two weeks at most: anything more is nonsense
                minutes_2weeks: g.playtime_2weeks.min(14 * 24 * 60),
            })
            .collect()),
        (Some(_), None) => Ok(vec![]),
        (None, None) => Err(ApiError::Private),
    }
}

/// `GetPlayerSummaries`: each player's `gameid`, `lobbysteamid` and `gameserverip` (empty strings as none).
pub fn parse_in_game(status: u16, body: &[u8]) -> Result<Vec<InGame>, ApiError> {
    if status != 200 {
        return Err(ApiError::Failed(format!(
            "GetPlayerSummaries: HTTP {status}"
        )));
    }
    #[derive(Deserialize)]
    struct Outer {
        response: Inner,
    }
    #[derive(Deserialize)]
    struct Inner {
        #[serde(default)]
        players: Vec<P>,
    }
    #[derive(Deserialize)]
    struct P {
        steamid: String,
        #[serde(default)]
        gameid: Option<String>,
        #[serde(default)]
        lobbysteamid: Option<String>,
        #[serde(default)]
        gameserverip: Option<String>,
    }
    let o: Outer = json("GetPlayerSummaries", body)?;
    let some = |v: Option<String>| v.filter(|x| !x.trim().is_empty());
    Ok(o.response
        .players
        .into_iter()
        .map(|p| InGame {
            steam_id: p.steamid,
            game_id: some(p.gameid),
            lobby: some(p.lobbysteamid),
            server: some(p.gameserverip),
        })
        .collect())
}

/// `GetPlayerSummaries`: the persona name of `steam_id`.
pub fn parse_persona(status: u16, body: &[u8], steam_id: &str) -> Option<String> {
    if status != 200 {
        return None;
    }
    #[derive(Deserialize)]
    struct Outer {
        response: Inner,
    }
    #[derive(Deserialize)]
    struct Inner {
        #[serde(default)]
        players: Vec<P>,
    }
    #[derive(Deserialize)]
    struct P {
        steamid: String,
        #[serde(default)]
        personaname: Option<String>,
    }
    let o: Outer = serde_json::from_slice(body).ok()?;
    let name = o
        .response
        .players
        .into_iter()
        .find(|p| p.steamid == steam_id)?
        .personaname?;
    crate::steam::one_line(&name, 64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_achievements_and_their_errors() {
        let ok = br#"{"playerstats":{"steamID":"1","gameName":"Space Game","achievements":[
            {"apiname":"A","achieved":1,"unlocktime":1700000000},{"apiname":"B","achieved":0,"unlocktime":0}],
            "success":true}}"#;
        let p = parse_player_achievements(200, ok).unwrap();
        assert_eq!(p.game.as_deref(), Some("Space Game"));
        assert_eq!(p.achievements.len(), 2);
        assert!(p.achievements[0].achieved && !p.achievements[1].achieved);
        assert_eq!(p.achievements[0].unlock_time, 1_700_000_000);
        // stats but no achievements at all
        let none = br#"{"playerstats":{"steamID":"1","gameName":"G","success":true}}"#;
        assert!(
            parse_player_achievements(200, none)
                .unwrap()
                .achievements
                .is_empty()
        );
        let private = br#"{"playerstats":{"error":"Profile is not public","success":false}}"#;
        assert_eq!(
            parse_player_achievements(403, private),
            Err(ApiError::Private)
        );
        let no_stats = br#"{"playerstats":{"error":"Requested app has no stats","success":false}}"#;
        assert_eq!(
            parse_player_achievements(400, no_stats),
            Err(ApiError::NoStats)
        );
        assert!(matches!(
            parse_player_achievements(500, b"oops"),
            Err(ApiError::Failed(_))
        ));
        assert!(matches!(
            parse_player_achievements(200, b"<html>"),
            Err(ApiError::Failed(_))
        ));
    }

    #[test]
    fn schema_reads_names_descriptions_and_icons() {
        let body = br#"{"game":{"gameName":"Space Game","availableGameStats":{"achievements":[
            {"name":"A","defaultvalue":0,"displayName":"First Steps","hidden":0,"description":"Walk.","icon":"https://x/a.jpg","icongray":"https://x/a_g.jpg"},
            {"name":"B","displayName":"","hidden":1,"icon":""}]}}}"#;
        let s = parse_schema(200, body).unwrap();
        assert_eq!(s.game.as_deref(), Some("Space Game"));
        assert_eq!(s.achievements[0].name, "First Steps");
        assert_eq!(s.achievements[0].icon.as_deref(), Some("https://x/a.jpg"));
        // a blank display name falls back to the API name; a hidden one has no description
        assert_eq!(s.achievements[1].name, "B");
        assert_eq!(s.achievements[1].description, None);
        assert_eq!(s.achievements[1].icon, None);
        assert_eq!(parse_schema(200, b"{}").unwrap(), Schema::default());
    }

    #[test]
    fn percentages_as_numbers_or_strings() {
        let body = br#"{"achievementpercentages":{"achievements":[
            {"name":"A","percent":"12.5"},{"name":"B","percent":3.25},{"name":"C","percent":"x"},{"name":"D","percent":400}]}}"#;
        let p = parse_percentages(200, body).unwrap();
        assert_eq!(p.get("A"), Some(&12.5));
        assert_eq!(p.get("B"), Some(&3.25));
        assert!(!p.contains_key("C") && !p.contains_key("D"));
    }

    #[test]
    fn owned_games_and_a_private_profile() {
        let body = br#"{"response":{"game_count":2,"games":[
            {"appid":10,"name":"G","playtime_forever":5,"has_community_visible_stats":true},
            {"appid":20,"playtime_forever":0}]}}"#;
        let g = parse_owned(200, body).unwrap();
        assert_eq!(g.len(), 2);
        assert!(g[0].has_stats && !g[1].has_stats);
        assert_eq!(
            parse_owned(200, br#"{"response":{}}"#),
            Err(ApiError::Private)
        );
    }

    #[test]
    fn persona_of_the_right_player() {
        let body = br#"{"response":{"players":[{"steamid":"2","personaname":"other"},{"steamid":"1","personaname":" alex\n "}]}}"#;
        assert_eq!(parse_persona(200, body, "1").as_deref(), Some("alex"));
        assert_eq!(parse_persona(200, body, "3"), None);
    }

    #[test]
    fn recent_games_none_and_a_private_profile() {
        let body = br#"{"response":{"total_count":2,"games":[
            {"appid":10,"name":"G","playtime_2weeks":90,"playtime_forever":500},
            {"appid":20,"playtime_2weeks":999999}]}}"#;
        let g = parse_recent(200, body).unwrap();
        assert_eq!(
            g[0],
            RecentGame {
                app_id: 10,
                minutes_2weeks: 90
            }
        );
        assert_eq!(g[1].minutes_2weeks, 14 * 24 * 60, "capped at two weeks");
        assert_eq!(
            parse_recent(200, br#"{"response":{"total_count":0}}"#),
            Ok(vec![])
        );
        assert_eq!(
            parse_recent(200, br#"{"response":{}}"#),
            Err(ApiError::Private)
        );
        assert_eq!(parse_recent(403, b""), Err(ApiError::Private));
        assert!(matches!(parse_recent(500, b""), Err(ApiError::Failed(_))));
    }

    #[test]
    fn in_game_reads_lobbies_and_servers() {
        let body = br#"{"response":{"players":[
            {"steamid":"1","gameid":"10","lobbysteamid":"109775240917155841"},
            {"steamid":"2","gameid":"20","gameserverip":"203.0.113.7:27015","lobbysteamid":""},
            {"steamid":"3","personaname":"kim"}]}}"#;
        let p = parse_in_game(200, body).unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].lobby.as_deref(), Some("109775240917155841"));
        assert_eq!(p[1].server.as_deref(), Some("203.0.113.7:27015"));
        assert_eq!(p[1].lobby, None, "an empty string is none");
        assert_eq!(
            p[2],
            InGame {
                steam_id: "3".into(),
                ..InGame::default()
            }
        );
        assert!(matches!(parse_in_game(500, b""), Err(ApiError::Failed(_))));
    }

    #[test]
    fn the_budget_caps_a_day_and_resets_the_next() {
        let b = Budget::new(2);
        assert!(b.take_on(10) && b.take_on(10));
        assert!(!b.take_on(10));
        assert_eq!(b.used(), 2);
        assert!(b.take_on(11), "a new day");
        assert_eq!(b.used(), 1);
    }
}
