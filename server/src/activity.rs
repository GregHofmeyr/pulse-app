//! "Listening to…" (#275): what someone's own Spotify app is playing, as their Pulse reads it from the OS media
//! session and sends it (`ClientFrame::Listening`).
//!
//! In memory only, per gateway connection (`gateway::hub`): it's as ephemeral as the connection that sent it. Who
//! sees it is presence's rule (`presence::view_for`): a viewer gets someone's activity only while that person looks
//! online to them, from the same per-viewer pass that announces presence (`Hub::sync_presence`), so Invisible,
//! "appear offline to" and offline never show any. The album art comes from the embed resolver's card for the
//! song's Spotify link (no Spotify API keys), never from a URL the client names.

use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{EmbedProvider, EmbedThumb, Listening, ListeningUpdate};

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
pub fn clean(u: ListeningUpdate) -> Option<Listening> {
    Some(Listening {
        title: text(&u.title)?,
        artist: u.artist.as_deref().and_then(text),
        album: u.album.as_deref().and_then(text),
        spotify_id: u.spotify_id.as_deref().and_then(spotify_id),
        art: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn update(title: &str) -> ListeningUpdate {
        ListeningUpdate {
            title: title.into(),
            artist: None,
            album: None,
            spotify_id: None,
        }
    }

    #[test]
    fn text_is_one_clean_line() {
        let l = clean(ListeningUpdate {
            title: "  Song\u{0007}  name\n(live) ".into(),
            artist: Some("\t".into()),
            album: Some("Album".into()),
            spotify_id: None,
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

    #[test]
    fn links_for_ids() {
        assert_eq!(
            spotify_link("track:4uLU6hMCjMI75M1A2tKUQC").as_deref(),
            Some("https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC")
        );
    }
}
