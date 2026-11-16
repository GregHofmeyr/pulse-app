//! One-click Join (#381): every poller round, the linked people who share Join and are playing get one batched
//! `GetPlayerSummaries`; a public `lobbysteamid` or `gameserverip` there, checked strictly, becomes the
//! [`SteamJoin`] inside their "Playing" (so it reaches exactly those who see that). Anything that doesn't check out,
//! a game other than the one they're shown playing, or no longer playing, means no Join.

use std::collections::{HashMap, HashSet};

use pulse_protocol::ids::UserId;
use pulse_protocol::rest::SteamJoin;
use pulse_protocol::steam_join::{valid_lobby, valid_server, valid_steam_id};

use super::api::{InGame, SUMMARIES_MAX};
use crate::AppState;

/// The Join in `g` for the person linked to `steam_id`, who's shown playing `playing_app` (when known): a lobby
/// first, else a server; `None` when Steam shows neither, or they don't check out, or it's another game.
pub fn join_of(g: &InGame, steam_id: &str, playing_app: Option<u32>) -> Option<SteamJoin> {
    if g.steam_id != steam_id || !valid_steam_id(steam_id) {
        return None;
    }
    let app: u32 = g
        .game_id
        .as_deref()
        .filter(|id| !id.starts_with('0') && id.bytes().all(|b| b.is_ascii_digit()))?
        .parse()
        .ok()
        .filter(|id| crate::embeds::steam::valid_app_id(*id))?;
    if playing_app.is_some_and(|p| p != app) {
        return None;
    }
    if let Some(lobby) = g.lobby.as_deref().filter(|l| valid_lobby(l)) {
        return Some(SteamJoin::Lobby {
            app_id: app,
            lobby_id: lobby.to_string(),
            steam_id: steam_id.to_string(),
        });
    }
    let address = g.server.as_deref().and_then(valid_server)?;
    Some(SteamJoin::Server {
        app_id: app,
        address,
    })
}

/// One round: read who's joinable among those who share Join and play now, and tell whoever's view changed.
pub async fn tick(s: &AppState) {
    if !s.steam.api().has_key() {
        return;
    }
    let rows: Vec<(String, String)> =
        match sqlx::query_as("SELECT user_id, steam_id FROM steam_links WHERE share_join = 1")
            .fetch_all(&s.db)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "Steam links not read for Join");
                return;
            }
        };
    // those playing now, by SteamID: (who, the app they're shown playing)
    let mut playing: HashMap<String, (UserId, Option<u32>)> = HashMap::new();
    for (user, steam_id) in rows {
        let Ok(user) = user.parse::<UserId>() else {
            continue;
        };
        let Some(p) = s.hub.playing_of(user) else {
            continue;
        };
        if valid_steam_id(&steam_id) {
            playing.insert(steam_id, (user, s.steam.app_of(user, Some(&p))));
        }
    }
    let mut found: HashMap<UserId, SteamJoin> = HashMap::new();
    let mut read: HashSet<UserId> = HashSet::new();
    let ids: Vec<&str> = playing.keys().map(String::as_str).collect();
    for chunk in ids.chunks(SUMMARIES_MAX) {
        match s.steam.api().in_game(chunk).await {
            Ok(list) => {
                read.extend(
                    chunk
                        .iter()
                        .filter_map(|id| playing.get(*id).map(|(u, _)| *u)),
                );
                for g in list {
                    if let Some((user, app)) = playing.get(&g.steam_id)
                        && let Some(j) = join_of(&g, &g.steam_id, *app)
                    {
                        found.insert(*user, j);
                    }
                }
            }
            Err(e) => tracing::debug!(error = ?e, "Steam summaries unavailable for now"),
        }
    }
    // who had one and now has none: stopped playing, switched Join off, unlinked, or Steam stopped showing it (a
    // failed read keeps what was shown until the next round)
    let mut changed: Vec<UserId> = Vec::new();
    for user in s.hub.steam_joiners() {
        let still_playing = playing.values().any(|(u, _)| *u == user);
        if (!still_playing || read.contains(&user))
            && !found.contains_key(&user)
            && s.hub.set_steam_join(user, None)
        {
            changed.push(user);
        }
    }
    for (user, j) in found {
        if s.hub.set_steam_join(user, Some(j)) {
            changed.push(user);
        }
    }
    for user in changed {
        s.hub.sync_presence(&s.db, user).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Obviously fake: lobby account 1.
    const LOBBY: &str = "109775240917155841";
    const ALEX: &str = "76500000000000001";

    fn g(game: Option<&str>, lobby: Option<&str>, server: Option<&str>) -> InGame {
        InGame {
            steam_id: ALEX.into(),
            game_id: game.map(Into::into),
            lobby: lobby.map(Into::into),
            server: server.map(Into::into),
        }
    }

    #[test]
    fn a_join_needs_a_game_and_a_lobby_or_server_that_check_out() {
        assert_eq!(
            join_of(
                &g(Some("440"), Some(LOBBY), Some("203.0.113.7:27015")),
                ALEX,
                Some(440)
            ),
            Some(SteamJoin::Lobby {
                app_id: 440,
                lobby_id: LOBBY.into(),
                steam_id: ALEX.into()
            }),
            "the lobby first"
        );
        assert_eq!(
            join_of(
                &g(Some("440"), Some("nope"), Some("203.0.113.7:27015")),
                ALEX,
                None
            ),
            Some(SteamJoin::Server {
                app_id: 440,
                address: "203.0.113.7:27015".into()
            })
        );
        assert_eq!(
            join_of(&g(Some("440"), None, None), ALEX, None),
            None,
            "nothing to join"
        );
        assert_eq!(
            join_of(&g(None, Some(LOBBY), None), ALEX, None),
            None,
            "no game"
        );
        assert_eq!(
            join_of(&g(Some("0440"), Some(LOBBY), None), ALEX, None),
            None
        );
        assert_eq!(join_of(&g(Some("x"), Some(LOBBY), None), ALEX, None), None);
        assert_eq!(
            join_of(&g(Some("440"), Some(LOBBY), None), ALEX, Some(570)),
            None,
            "another game"
        );
        assert_eq!(
            join_of(
                &g(Some("440"), Some(LOBBY), None),
                "76500000000000002",
                None
            ),
            None,
            "someone else's"
        );
    }
}
