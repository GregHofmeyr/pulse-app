//! "Listening to…" (#275): what someone's own Spotify app is playing, as their Pulse reads it from the OS media
//! session and sends it (`ClientFrame::Listening`).
//!
//! In memory only, per gateway connection (`gateway::hub`): it's as ephemeral as the connection that sent it. Who
//! sees it is presence's rule (`presence::view_for`): a viewer gets someone's activity only while that person looks
//! online to them, from the same per-viewer pass that announces presence (`Hub::sync_presence`), so Invisible,
//! "appear offline to" and offline never show any. The album art comes from the embed resolver's card for the
//! song's Spotify link (no Spotify API keys), never from a URL the client names.
//!
//! "Playing" (#380): the game someone's Pulse detects on Windows (`ClientFrame::Playing`), held and announced the same
//! way, beside "Listening to". Its art comes from Steam's CDN by app id ([`crate::embeds::steam`]), fetched by the
//! server, never by clients.

use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{
    EmbedProvider, EmbedThumb, GameArt, Listening, ListeningUpdate, MusicPlayer, Playing,
    PlayingUpdate,
};

use crate::AppState;
use crate::embeds::providers;
use crate::gateway::hub::ConnId;

/// The longest title, artist or album kept, in characters.
pub const MAX_TEXT: usize = 200;

/// One line of plain text, as shown: control characters out, whitespace runs as one space, trimmed, at most
/// [`MAX_TEXT`] characters. `None` when nothing's left.
fn text(raw: &str) -> Option<String> {
    let mut out = String::new();
    for word in raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
    {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    let out: String = out.chars().take(MAX_TEXT).collect();
    let out = out.trim_end().to_owned();
    (!out.is_empty()).then_some(out)
}

/// A Spotify id the link-out and the art can use: `track:<id>` or `episode:<id>`.
fn spotify_id(raw: &str) -> Option<String> {
    let (kind, _) = raw.split_once(':')?;
    (matches!(kind, "track" | "episode") && providers::valid_id(EmbedProvider::Spotify, raw))
        .then(|| raw.to_owned())
}

/// What a client's update says, checked: `None` when there's no title (nothing to show). The art is the server's to
/// add ([`with_art`]).
/// Only Spotify's keeps a Spotify id (#322): another player's never gets a Spotify link or art. A player this build
/// doesn't know is dropped, and Spotify is held as no `player` at all, the shape older clients know.
pub fn clean(u: ListeningUpdate) -> Option<Listening> {
    let player = match u.player {
        None | Some(MusicPlayer::Spotify) => None,
        Some(MusicPlayer::Other) => return None,
        Some(p) => Some(p),
    };
    Some(Listening {
        title: text(&u.title)?,
        artist: u.artist.as_deref().and_then(text),
        album: u.album.as_deref().and_then(text),
        spotify_id: u
            .spotify_id
            .as_deref()
            .filter(|_| player.is_none())
            .and_then(spotify_id),
        art: None,
        player,
    })
}

/// The Spotify link for `spotify_id` (`track:<id>` → `https://open.spotify.com/track/<id>`).
pub fn spotify_link(spotify_id: &str) -> Option<String> {
    let (kind, id) = spotify_id.split_once(':')?;
    Some(format!("https://open.spotify.com/{kind}/{id}"))
}

/// `l` with its album art: `known` when it's for the same Spotify id (a pause and resume of the same song fetches
/// nothing), else the thumbnail of the link's Spotify card, from the resolver's cache or fetched within `me`'s art
/// budget. A card whose thumbnail file was already cleaned up gives none.
pub async fn with_art(
    s: &AppState,
    me: UserId,
    mut l: Listening,
    known: Option<(String, Option<EmbedThumb>)>,
) -> Listening {
    let Some(id) = l.spotify_id.clone() else {
        return l;
    };
    if let Some((was, art)) = known
        && was == id
    {
        l.art = art;
        return l;
    }
    let Some(link) = spotify_link(&id) else {
        return l;
    };
    let card = s
        .embeds
        .unfurl(
            &s.db,
            &s.cfg.media_dir,
            me,
            &s.limits.listening_art_user,
            &link,
        )
        .await
        .filter(|c| c.provider == EmbedProvider::Spotify);
    if let Some(thumb) = card.and_then(|c| c.thumb) {
        l.art = crate::embeds::keep_thumb(&s.db, thumb)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = ?e, "listening art not kept");
                None
            });
    }
    l
}

/// Connection `conn` of `me` says what it's listening to (`None`: nothing): checked, given its art (in the
/// background: the art may take a fetch), held, and announced per viewer, and to their Jam on a stage. An update that
/// arrives while an older one still fetches its art wins: the older one never lands.
pub fn received(
    s: &AppState,
    conn: ConnId,
    me: UserId,
    update: Option<ListeningUpdate>,
    show: bool,
) {
    let ticket = s.hub.listening_ticket(conn);
    let s = s.clone();
    tokio::spawn(async move {
        let listening = match update.and_then(clean) {
            Some(l) => {
                let known = s
                    .hub
                    .listening_held(conn)
                    .and_then(|h| Some((h.spotify_id?, h.art)));
                Some(with_art(&s, me, l, known).await)
            }
            None => None,
        };
        if s.hub
            .set_listening(conn, me, ticket, listening.map(|l| (l, show)))
        {
            s.hub.sync_presence(&s.db, me).await;
            crate::stage::listening_changed(&s, me).await;
        }
    });
}

/// The longest play time believed, in seconds (a month): a bigger one is a broken clock, shown as none.
pub const MAX_ELAPSED: u64 = 30 * 24 * 3600;

/// What a client's "Playing" says, checked: `None` when there's no name. A Steam app id that can't be one is dropped
/// (no art); `started_at` is `now` less the seconds played. The art is the server's to add.
pub fn clean_playing(u: PlayingUpdate, now: chrono::DateTime<chrono::Utc>) -> Option<Playing> {
    Some(Playing {
        name: text(&u.name)?,
        steam_app_id: u
            .steam_app_id
            .filter(|id| crate::embeds::steam::valid_app_id(*id)),
        started_at: u
            .elapsed_secs
            .filter(|s| *s <= MAX_ELAPSED)
            .map(|s| crate::db::stamp(now - chrono::Duration::seconds(s as i64))),
        art: None,
        // only the Steam poller sets it (#381)
        join: None,
    })
}

/// `p` with its Steam art: `known` when it's for the same app id (the same game again fetches nothing), else from the
/// server's Steam art cache or fetched within `me`'s art budget.
pub async fn with_game_art(
    s: &AppState,
    me: UserId,
    mut p: Playing,
    known: Option<(u32, Option<GameArt>)>,
) -> Playing {
    let Some(id) = p.steam_app_id else {
        return p;
    };
    if let Some((was, art)) = known
        && was == id
    {
        p.art = art;
        return p;
    }
    p.art = s
        .embeds
        .steam_art(
            &s.db,
            &s.cfg.media_dir,
            me,
            &s.limits.listening_art_user,
            id,
        )
        .await;
    p
}

/// Connection `conn` of `me` says what game it's playing (`None`: none): checked, given its art (in the background),
/// held, and announced per viewer. With `hide_art` (#379) the app id is noted for the achievement poller only: no art,
/// and nobody is shown the id. A newer update wins over one still fetching its art, as for "Listening to".
pub fn playing_received(s: &AppState, conn: ConnId, me: UserId, update: Option<PlayingUpdate>) {
    let ticket = s.hub.playing_ticket(conn);
    let s = s.clone();
    tokio::spawn(async move {
        let hide_art = update.as_ref().is_some_and(|u| u.hide_art);
        let playing = match update.and_then(|u| clean_playing(u, chrono::Utc::now())) {
            Some(mut p) => {
                // achievements (#379) follow the app id even when art (and so the id) isn't shown
                if let Some(id) = p.steam_app_id {
                    s.steam.note_playing(me, &p.name, id);
                    if hide_art {
                        p.steam_app_id = None;
                    }
                }
                let known = s
                    .hub
                    .playing_held(conn)
                    .and_then(|h| Some((h.steam_app_id?, h.art)));
                Some(with_game_art(&s, me, p, known).await)
            }
            None => None,
        };
        if s.hub.set_playing(conn, me, ticket, playing) {
            s.hub.sync_presence(&s.db, me).await;
            // invites to a game they no longer play end (#436)
            crate::game_invites::follow(&s, me).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(name: &str) -> PlayingUpdate {
        PlayingUpdate {
            name: name.into(),
            steam_app_id: None,
            elapsed_secs: None,
            hide_art: false,
        }
    }

    #[test]
    fn playing_is_cleaned_and_its_start_is_server_time() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-16T20:00:00Z")
            .unwrap()
            .to_utc();
        let p = clean_playing(
            PlayingUpdate {
                steam_app_id: Some(570),
                elapsed_secs: Some(90),
                ..game("  Space\u{0007}  Game\n")
            },
            now,
        )
        .unwrap();
        assert_eq!(p.name, "Space Game");
        assert_eq!(p.steam_app_id, Some(570));
        assert_eq!(p.started_at.as_deref(), Some("2026-11-16T19:58:30.000Z"));
        assert_eq!(p.art, None);
        // no name: nothing; a shortcut's id or a silly play time: left out
        assert_eq!(clean_playing(game(" \u{0000} "), now), None);
        let odd = clean_playing(
            PlayingUpdate {
                steam_app_id: Some(0x8000_0001),
                elapsed_secs: Some(u64::MAX),
                ..game("Chess")
            },
            now,
        )
        .unwrap();
        assert_eq!((odd.steam_app_id, odd.started_at), (None, None));
        assert_eq!(
            clean_playing(game(&"g".repeat(500)), now)
                .unwrap()
                .name
                .chars()
                .count(),
            MAX_TEXT
        );
    }

    fn update(title: &str) -> ListeningUpdate {
        ListeningUpdate {
            title: title.into(),
            artist: None,
            album: None,
            spotify_id: None,
            player: None,
        }
    }

    #[test]
    fn text_is_one_clean_line() {
        let l = clean(ListeningUpdate {
            title: "  Song\u{0007}  name\n(live) ".into(),
            artist: Some("\t".into()),
            album: Some("Album".into()),
            spotify_id: None,
            player: None,
        })
        .unwrap();
        assert_eq!(l.title, "Song name (live)");
        assert_eq!(l.artist, None);
        assert_eq!(l.album.as_deref(), Some("Album"));
        assert_eq!(l.art, None);
    }

    #[test]
    fn no_title_is_nothing() {
        assert_eq!(clean(update("")), None);
        assert_eq!(clean(update(" \u{0000} ")), None);
    }

    #[test]
    fn long_text_is_cut() {
        let l = clean(update(&"é".repeat(500))).unwrap();
        assert_eq!(l.title.chars().count(), MAX_TEXT);
    }

    #[test]
    fn only_track_and_episode_ids() {
        let id = |raw: &str| {
            clean(ListeningUpdate {
                spotify_id: Some(raw.into()),
                ..update("x")
            })
            .unwrap()
            .spotify_id
        };
        assert_eq!(
            id("track:4uLU6hMCjMI75M1A2tKUQC").as_deref(),
            Some("track:4uLU6hMCjMI75M1A2tKUQC")
        );
        assert!(id("episode:4uLU6hMCjMI75M1A2tKUQC").is_some());
        for bad in [
            "album:4uLU6hMCjMI75M1A2tKUQC",
            "jam:abcdefgh",
            "ad:4uLU6hMCjMI75M1A2tKUQC",
            "track:short",
            "track:4uLU6hMCjMI75M1A2tKUQ/",
            "track:4uLU6hMCjMI75M1A2tKUQC/../x",
            "4uLU6hMCjMI75M1A2tKUQC",
        ] {
            assert_eq!(id(bad), None, "{bad}");
        }
    }

    /// Other players (#322): their name is kept, a Spotify id they claim isn't, Spotify is held as no `player` (what
    /// older clients know), and a player this build can't name is nothing.
    #[test]
    fn other_players_never_carry_spotify_bits() {
        let with = |player| ListeningUpdate {
            spotify_id: Some("track:4uLU6hMCjMI75M1A2tKUQC".into()),
            player,
            ..update("x")
        };
        let yt = clean(with(Some(MusicPlayer::YoutubeMusic))).unwrap();
        assert_eq!(yt.player, Some(MusicPlayer::YoutubeMusic));
        assert_eq!(yt.spotify_id, None);
        assert!(!yt.is_spotify());
        let sp = clean(with(Some(MusicPlayer::Spotify))).unwrap();
        assert_eq!(sp.player, None);
        assert!(sp.spotify_id.is_some() && sp.is_spotify());
        assert!(clean(with(None)).unwrap().is_spotify());
        assert_eq!(clean(with(Some(MusicPlayer::Other))), None);
        // the wire: Spotify's has no `player`, another's names it
        let v = serde_json::to_value(&sp).unwrap();
        assert!(v.get("player").is_none(), "{v}");
        assert_eq!(
            serde_json::to_value(&yt).unwrap()["player"],
            "youtube_music"
        );
        // a player a newer client names reads as Other, not an error
        let u: ListeningUpdate =
            serde_json::from_str(r#"{"title":"x","player":"bandcamp"}"#).unwrap();
        assert_eq!(u.player, Some(MusicPlayer::Other));
    }

    #[test]
    fn links_for_ids() {
        assert_eq!(
            spotify_link("track:4uLU6hMCjMI75M1A2tKUQC").as_deref(),
            Some("https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC")
        );
    }
}
