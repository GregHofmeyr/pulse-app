//! What a media session says, read as [`NowPlaying`] (pure, built and tested on every platform): MPRIS metadata
//! (Linux) and the Windows media session's fields, and which Windows session is Spotify's.

use super::{NowPlaying, Seen};

/// The MPRIS metadata fields Pulse reads (the D-Bus map, already unpacked).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MprisMeta {
    /// `mpris:trackid`: Spotify's is `/com/spotify/track/<id>` (older apps: `spotify:track:<id>`).
    pub trackid: Option<String>,
    /// `xesam:url`: `https://open.spotify.com/track/<id>`.
    pub url: Option<String>,
    pub title: Option<String>,
    pub artists: Vec<String>,
    pub album: Option<String>,
}

const KINDS: [&str; 2] = ["track", "episode"];

/// `track:<id>` / `episode:<id>` from a Spotify id in any of its spellings: `/com/spotify/track/<id>`,
/// `spotify:track:<id>`, `https://open.spotify.com/track/<id>?si=…`. `None` for anything else (an ad, a local file).
pub fn spotify_id(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let rest = raw
        .strip_prefix("/com/spotify/")
        .or_else(|| raw.strip_prefix("spotify:"))
        .or_else(|| raw.strip_prefix("https://open.spotify.com/"))?;
    let rest = rest.split(['?', '#']).next()?;
    let (kind, id) = rest.split_once(['/', ':'])?;
    (KINDS.contains(&kind) && id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then(|| format!("{kind}:{id}"))
}

/// Whether a Spotify id names an ad (Spotify Free plays them through the same session).
fn is_ad(raw: &str) -> bool {
    let raw = raw.trim();
    raw.starts_with("/com/spotify/ad/") || raw.starts_with("spotify:ad:")
}

fn some_text(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// What an MPRIS player plays: only while its `PlaybackStatus` is `Playing`, with a title, and not an ad.
pub fn now_playing_mpris(status: Option<&str>, m: &MprisMeta) -> Seen {
    if status != Some("Playing") || m.trackid.as_deref().is_some_and(is_ad) {
        return None;
    }
    let artists: Vec<&str> = m
        .artists
        .iter()
        .map(|a| a.trim())
        .filter(|a| !a.is_empty())
        .collect();
    Some(NowPlaying {
        title: some_text(m.title.as_deref())?,
        artist: (!artists.is_empty()).then(|| artists.join(", ")),
        album: some_text(m.album.as_deref()),
        spotify_id: m
            .trackid
            .as_deref()
            .and_then(spotify_id)
            .or_else(|| m.url.as_deref().and_then(spotify_id)),
    })
}

/// Whether a Windows media session (by its app's AppUserModelID) is the Spotify app: the desktop app's `Spotify.exe`
/// or the Microsoft Store app's `SpotifyAB.SpotifyMusic_…!Spotify`. Not a browser playing open.spotify.com (its
/// session is the browser's), nor anything that merely has "spotify" in its name.
pub fn is_spotify_app(app_id: &str) -> bool {
    let id = app_id.trim().to_ascii_lowercase();
    let exe = id.rsplit(['\\', '/']).next().unwrap_or(&id);
    exe == "spotify.exe" || id.starts_with("spotifyab.spotifymusic_")
}

/// `GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing`.
pub const GSMTC_PLAYING: i32 = 4;

/// What a Windows media session plays: only while its status is Playing, with a title. Windows gives no Spotify id.
pub fn now_playing_gsmtc(status: i32, title: &str, artist: &str, album: &str) -> Seen {
    if status != GSMTC_PLAYING {
        return None;
    }
    Some(NowPlaying {
        title: some_text(Some(title))?,
        artist: some_text(Some(artist)),
        album: some_text(Some(album)),
        spotify_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "4uLU6hMCjMI75M1A2tKUQC";

    fn meta() -> MprisMeta {
        MprisMeta {
            trackid: Some(format!("/com/spotify/track/{ID}")),
            url: Some(format!("https://open.spotify.com/track/{ID}")),
            title: Some("Song".into()),
            artists: vec!["Alex".into(), " Sam ".into(), "".into()],
            album: Some("Album".into()),
        }
    }

    #[test]
    fn ids_in_every_spelling() {
        let want = Some(format!("track:{ID}"));
        for raw in [
            format!("/com/spotify/track/{ID}"),
            format!("spotify:track:{ID}"),
            format!("https://open.spotify.com/track/{ID}?si=abc"),
        ] {
            assert_eq!(spotify_id(&raw), want, "{raw}");
        }
        assert_eq!(
            spotify_id(&format!("/com/spotify/episode/{ID}")),
            Some(format!("episode:{ID}"))
        );
        for bad in [
            format!("/com/spotify/ad/{ID}"),
            format!("/com/spotify/album/{ID}"),
            "/com/spotify/track/short".to_owned(),
            format!("/org/mpris/MediaPlayer2/track/{ID}"),
            format!("https://open.spotify.com.evil.test/track/{ID}"),
            format!("spotify:local:x:y:{ID}"),
            String::new(),
        ] {
            assert_eq!(spotify_id(&bad), None, "{bad}");
        }
    }

    #[test]
    fn mpris_playing_reads_everything() {
        let np = now_playing_mpris(Some("Playing"), &meta()).unwrap();
        assert_eq!(np.title, "Song");
        assert_eq!(np.artist.as_deref(), Some("Alex, Sam"));
        assert_eq!(np.album.as_deref(), Some("Album"));
        assert_eq!(np.spotify_id, Some(format!("track:{ID}")));
        // an old app's trackid that isn't an id: the URL still gives it
        let m = MprisMeta {
            trackid: Some("/org/mpris/MediaPlayer2/TrackList/NoTrack".into()),
            ..meta()
        };
        assert_eq!(
            now_playing_mpris(Some("Playing"), &m).unwrap().spotify_id,
            Some(format!("track:{ID}"))
        );
    }

    #[test]
    fn mpris_paused_stopped_ads_and_untitled_are_nothing() {
        for status in [Some("Paused"), Some("Stopped"), None, Some("playing")] {
            assert_eq!(now_playing_mpris(status, &meta()), None, "{status:?}");
        }
        let ad = MprisMeta {
            trackid: Some(format!("/com/spotify/ad/{ID}")),
            title: Some("Advertisement".into()),
            ..meta()
        };
        assert_eq!(now_playing_mpris(Some("Playing"), &ad), None);
        let untitled = MprisMeta {
            title: Some("  ".into()),
            ..meta()
        };
        assert_eq!(now_playing_mpris(Some("Playing"), &untitled), None);
        let bare = MprisMeta {
            title: Some("Local file".into()),
            ..MprisMeta::default()
        };
        let np = now_playing_mpris(Some("Playing"), &bare).unwrap();
        assert_eq!((np.artist, np.album, np.spotify_id), (None, None, None));
    }

    #[test]
    fn only_the_spotify_app_on_windows() {
        for yes in [
            "Spotify.exe",
            "spotify.exe",
            r"C:\Users\alex\AppData\Roaming\Spotify\Spotify.exe",
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
        ] {
            assert!(is_spotify_app(yes), "{yes}");
        }
        for no in [
            "",
            "MSEdge",
            "Chrome",
            "brave.exe",
            "308046B0AF4A39CB",
            "NotSpotify.exe",
            "spotify.exe.evil",
            "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic",
            "spotifyd",
        ] {
            assert!(!is_spotify_app(no), "{no}");
        }
    }

    #[test]
    fn gsmtc_playing_only_and_no_id() {
        let np = now_playing_gsmtc(GSMTC_PLAYING, " Song ", "Alex", "").unwrap();
        assert_eq!(np.title, "Song");
        assert_eq!(np.artist.as_deref(), Some("Alex"));
        assert_eq!((np.album, np.spotify_id), (None, None));
        // Closed, Opened, Changing, Stopped, Paused
        for status in [0, 1, 2, 3, 5] {
            assert_eq!(now_playing_gsmtc(status, "Song", "Alex", "Album"), None);
        }
        assert_eq!(now_playing_gsmtc(GSMTC_PLAYING, "", "Alex", "Album"), None);
    }
}
