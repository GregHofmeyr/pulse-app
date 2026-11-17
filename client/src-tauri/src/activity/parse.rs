//! What a media session says, read as [`NowPlaying`] (pure, built and tested on every platform): MPRIS metadata
//! (Linux) and the Windows media session's fields, and which sessions count as music players at all (#322).
//!
//! The allowlist is strict and documented in `docs/listening-players.md`: an OS media session counts only when its
//! Windows app id (`SourceAppUserModelId`) or MPRIS bus name is a known music player's. Browsers play anything, and
//! neither Windows nor (mostly) MPRIS says which site: a browser's session counts only when its metadata looks like a
//! music site's (an artist *and* an album, which music sites set and video sites don't), or, where MPRIS gives the
//! page's address, only on a known music site.

use pulse_protocol::rest::MusicPlayer;
use serde::Serialize;

use super::{NowPlaying, Seen};

/// Which Settings switch governs a player (#322): Spotify's is "Read media activity" itself, the rest each have a
/// "Share music from …" switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Spotify,
    /// Desktop players: Apple Music, Tidal, Deezer, foobar2000, YouTube Music's desktop app.
    Players,
    /// Music in a browser (a tab, or YouTube Music installed from one).
    Browsers,
}

/// The "Share music from other players" switches (#322), both on unless turned off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Sources {
    pub players: bool,
    pub browsers: bool,
}

impl Default for Sources {
    fn default() -> Self {
        Sources {
            players: true,
            browsers: true,
        }
    }
}

impl Sources {
    pub fn allows(self, g: Group) -> bool {
        match g {
            Group::Spotify => true,
            Group::Players => self.players,
            Group::Browsers => self.browsers,
        }
    }
}

/// A media session the allowlist knows: which player it is, and which switch governs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    pub player: MusicPlayer,
    pub group: Group,
}

const fn src(player: MusicPlayer, group: Group) -> Option<Source> {
    Some(Source { player, group })
}

/// YouTube Music's id as an app installed from Chrome or Edge (`Chrome._crx_<id>`, `MSEdge._crx_<id>`).
const YTM_PWA: &str = "cinhimbnkkaeohfgghhklpknlkffjgod";

/// Which player a Windows media session is, by its app's AppUserModelID (`SourceAppUserModelId`): a packaged app's
/// `Family_publisher!App`, a desktop app's exe name or the id it set itself. `None` for anything not on the list.
pub fn classify_gsmtc(app_id: &str) -> Option<Source> {
    use Group::{Browsers, Players};
    use MusicPlayer::*;
    let id = app_id.trim().to_ascii_lowercase();
    let exe = id.rsplit(['\\', '/']).next().unwrap_or(&id);
    // Spotify: the desktop app's exe, or the Microsoft Store app
    if exe == "spotify.exe" || id.starts_with("spotifyab.spotifymusic_") {
        return src(MusicPlayer::Spotify, Group::Spotify);
    }
    // YouTube Music installed from a Chromium browser: the id names the app itself, so no metadata check
    if let Some((browser, app)) = id.split_once("._crx_") {
        return (matches!(browser, "chrome" | "msedge" | "brave") && app == YTM_PWA).then_some(
            Source {
                player: YoutubeMusic,
                group: Browsers,
            },
        );
    }
    match exe {
        // th-ch/youtube-music, the desktop app
        "com.github.th-ch.youtube-music" | "youtube music.exe" => src(YoutubeMusic, Players),
        "tidal.exe" | "com.squirrel.tidal.tidal" => src(Tidal, Players),
        "deezer.exe" | "com.deezer.deezer-desktop" => src(Deezer, Players),
        "foobar2000.exe" | "foobar2000" => src(Foobar2000, Players),
        _ if id.starts_with("appleinc.applemusicwin_") => src(AppleMusic, Players),
        _ if id.starts_with("deezer.62021768415af_") => src(Deezer, Players),
        // browsers: Chrome, Edge, Brave, Vivaldi, Opera (their exe or the id they set, with a per-install suffix for
        // some), and Firefox (its id is a hash of its install folder: the default folders' only)
        _ if is_browser(exe) => src(Browser, Browsers),
        _ => None,
    }
}

fn is_browser(id: &str) -> bool {
    const IDS: [&str; 6] = ["chrome", "msedge", "brave", "vivaldi", "opera", "firefox"];
    // 64-bit and 32-bit Firefox in their default folders
    const FIREFOX: [&str; 2] = ["308046b0af4a39cb", "e7cf176e110c211b"];
    if let Some(name) = id.strip_suffix(".exe") {
        return IDS.contains(&name);
    }
    FIREFOX.contains(&id)
        || IDS
            .iter()
            .any(|b| id == *b || id.strip_prefix(b).is_some_and(|r| r.starts_with('.')))
}

/// The MPRIS bus-name prefix every player's name starts with.
pub const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";

/// Which player an MPRIS bus name is (`org.mpris.MediaPlayer2.<app>`, often with `.instance<pid>` after it). `None`
/// for anything not on the list.
pub fn classify_mpris(bus_name: &str) -> Option<Source> {
    use Group::{Browsers, Players};
    use MusicPlayer::*;
    let rest = bus_name.strip_prefix(MPRIS_PREFIX)?;
    let app = rest.split('.').next().unwrap_or(rest);
    match app {
        // the desktop app, its Flatpak and Snap alike (`spotifyd` and others are not the app)
        "spotify" => src(MusicPlayer::Spotify, Group::Spotify),
        "youtube-music" | "YoutubeMusic" => src(YoutubeMusic, Players),
        // Cider, the Apple Music client for Linux
        "cider" | "Cider" => src(AppleMusic, Players),
        "tidal-hifi" => src(Tidal, Players),
        "deezer" | "Deezer" => src(Deezer, Players),
        "chromium"
        | "chrome"
        | "google-chrome"
        | "brave"
        | "vivaldi"
        | "opera"
        | "edge"
        | "msedge"
        | "firefox"
        | "librewolf"
        | "plasma-browser-integration" => src(Browser, Browsers),
        _ => None,
    }
}

/// The MPRIS metadata fields Pulse reads (the D-Bus map, already unpacked).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MprisMeta {
    /// `mpris:trackid`: Spotify's is `/com/spotify/track/<id>` (older apps: `spotify:track:<id>`).
    pub trackid: Option<String>,
    /// `xesam:url`: `https://open.spotify.com/track/<id>`; a browser's (where it gives one) is the page's.
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

/// The music site a browser page is on, by its address (`https://<host>/…`): `None` for any other site, which is
/// then not shown at all.
pub fn music_site(url: &str) -> Option<MusicPlayer> {
    let rest = url.trim().strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?.to_ascii_lowercase();
    match host.as_str() {
        "music.youtube.com" => Some(MusicPlayer::YoutubeMusic),
        "open.spotify.com" => Some(MusicPlayer::Spotify),
        "music.apple.com" | "beta.music.apple.com" => Some(MusicPlayer::AppleMusic),
        "listen.tidal.com" => Some(MusicPlayer::Tidal),
        "www.deezer.com" | "deezer.com" => Some(MusicPlayer::Deezer),
        _ => None,
    }
}

/// Whether a browser's metadata reads as a music site's: an artist and an album (YouTube Music, Spotify's and the
/// other web players set an album; a video site, YouTube's included, doesn't).
fn looks_like_music(artist: Option<&str>, album: Option<&str>) -> bool {
    some_text(artist).is_some() && some_text(album).is_some()
}

/// What an MPRIS player plays: only while its `PlaybackStatus` is `Playing`, with a title, and not an ad. A browser's
/// counts only on a known music site (when it gives the page's address) or when it looks like music (when it
/// doesn't). Only Spotify's keeps a Spotify id.
pub fn now_playing_mpris(source: Source, status: Option<&str>, m: &MprisMeta) -> Seen {
    if status != Some("Playing") || m.trackid.as_deref().is_some_and(is_ad) {
        return None;
    }
    let artists: Vec<&str> = m
        .artists
        .iter()
        .map(|a| a.trim())
        .filter(|a| !a.is_empty())
        .collect();
    let artist = (!artists.is_empty()).then(|| artists.join(", "));
    let player = match (source.player, m.url.as_deref()) {
        (MusicPlayer::Browser, Some(url)) if some_text(Some(url)).is_some() => music_site(url)?,
        (MusicPlayer::Browser, _) if !looks_like_music(artist.as_deref(), m.album.as_deref()) => {
            return None;
        }
        (p, _) => p,
    };
    Some(NowPlaying {
        title: some_text(m.title.as_deref())?,
        artist,
        album: some_text(m.album.as_deref()),
        spotify_id: (player == MusicPlayer::Spotify)
            .then(|| {
                m.trackid
                    .as_deref()
                    .and_then(spotify_id)
                    .or_else(|| m.url.as_deref().and_then(spotify_id))
            })
            .flatten(),
        player,
    })
}

/// Whether a Windows media session (by its app's AppUserModelID) is the Spotify app: the desktop app's `Spotify.exe`
/// or the Microsoft Store app's `SpotifyAB.SpotifyMusic_…!Spotify`. Not a browser playing open.spotify.com (its
/// session is the browser's), nor anything that merely has "spotify" in its name.
pub fn is_spotify_app(app_id: &str) -> bool {
    classify_gsmtc(app_id).is_some_and(|s| s.player == MusicPlayer::Spotify)
}

/// `GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing`.
pub const GSMTC_PLAYING: i32 = 4;

/// What a Windows media session plays: only while its status is Playing, with a title. Windows gives no Spotify id,
/// and no page address for a browser: a browser tab's counts only when it looks like music.
pub fn now_playing_gsmtc(
    source: Source,
    status: i32,
    title: &str,
    artist: &str,
    album: &str,
) -> Seen {
    if status != GSMTC_PLAYING
        || (source.player == MusicPlayer::Browser && !looks_like_music(Some(artist), Some(album)))
    {
        return None;
    }
    Some(NowPlaying {
        title: some_text(Some(title))?,
        artist: some_text(Some(artist)),
        album: some_text(Some(album)),
        spotify_id: None,
        player: source.player,
    })
}

/// How much a player is preferred when more than one plays at once: Spotify, then a desktop player, then a browser.
fn rank(p: MusicPlayer) -> u8 {
    match p {
        MusicPlayer::Spotify => 0,
        MusicPlayer::Browser => 2,
        _ => 1,
    }
}

/// What to show of what several players play (each `None` when not playing): the most preferred one, the first of
/// equals.
pub fn best(seen: impl IntoIterator<Item = Seen>) -> Seen {
    let mut out: Seen = None;
    for np in seen.into_iter().flatten() {
        if out
            .as_ref()
            .is_none_or(|o| rank(np.player) < rank(o.player))
        {
            out = Some(np);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "4uLU6hMCjMI75M1A2tKUQC";
    const SPOTIFY: Source = Source {
        player: MusicPlayer::Spotify,
        group: Group::Spotify,
    };
    const BROWSER: Source = Source {
        player: MusicPlayer::Browser,
        group: Group::Browsers,
    };

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
        let np = now_playing_mpris(SPOTIFY, Some("Playing"), &meta()).unwrap();
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
            now_playing_mpris(SPOTIFY, Some("Playing"), &m)
                .unwrap()
                .spotify_id,
            Some(format!("track:{ID}"))
        );
    }

    #[test]
    fn mpris_paused_stopped_ads_and_untitled_are_nothing() {
        for status in [Some("Paused"), Some("Stopped"), None, Some("playing")] {
            assert_eq!(
                now_playing_mpris(SPOTIFY, status, &meta()),
                None,
                "{status:?}"
            );
        }
        let ad = MprisMeta {
            trackid: Some(format!("/com/spotify/ad/{ID}")),
            title: Some("Advertisement".into()),
            ..meta()
        };
        assert_eq!(now_playing_mpris(SPOTIFY, Some("Playing"), &ad), None);
        let untitled = MprisMeta {
            title: Some("  ".into()),
            ..meta()
        };
        assert_eq!(now_playing_mpris(SPOTIFY, Some("Playing"), &untitled), None);
        let bare = MprisMeta {
            title: Some("Local file".into()),
            ..MprisMeta::default()
        };
        let np = now_playing_mpris(SPOTIFY, Some("Playing"), &bare).unwrap();
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
        let np = now_playing_gsmtc(SPOTIFY, GSMTC_PLAYING, " Song ", "Alex", "").unwrap();
        assert_eq!(np.title, "Song");
        assert_eq!(np.artist.as_deref(), Some("Alex"));
        assert_eq!((np.album, np.spotify_id), (None, None));
        // Closed, Opened, Changing, Stopped, Paused
        for status in [0, 1, 2, 3, 5] {
            assert_eq!(
                now_playing_gsmtc(SPOTIFY, status, "Song", "Alex", "Album"),
                None
            );
        }
        assert_eq!(
            now_playing_gsmtc(SPOTIFY, GSMTC_PLAYING, "", "Alex", "Album"),
            None
        );
    }

    /// The Windows allowlist (#322), with app ids as Windows reports them.
    #[test]
    fn windows_players_by_app_id() {
        use MusicPlayer::*;
        for (id, player, group) in [
            ("Spotify.exe", Spotify, Group::Spotify),
            (
                "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
                Spotify,
                Group::Spotify,
            ),
            (
                "AppleInc.AppleMusicWin_nzyj5cx40ttqa!App",
                AppleMusic,
                Group::Players,
            ),
            ("TIDAL.exe", Tidal, Group::Players),
            ("com.squirrel.TIDAL.TIDAL", Tidal, Group::Players),
            ("Deezer.exe", Deezer, Group::Players),
            (
                "Deezer.62021768415AF_q7m17pa7q8kj0!Deezer.Music",
                Deezer,
                Group::Players,
            ),
            (
                r"C:\Program Files\foobar2000\foobar2000.exe",
                Foobar2000,
                Group::Players,
            ),
            (
                "com.github.th-ch.youtube-music",
                YoutubeMusic,
                Group::Players,
            ),
            (
                "Chrome._crx_cinhimbnkkaeohfgghhklpknlkffjgod",
                YoutubeMusic,
                Group::Browsers,
            ),
            (
                "MSEdge._crx_cinhimbnkkaeohfgghhklpknlkffjgod",
                YoutubeMusic,
                Group::Browsers,
            ),
            ("Chrome", Browser, Group::Browsers),
            ("MSEdge", Browser, Group::Browsers),
            ("Brave", Browser, Group::Browsers),
            ("Chrome.ABCDEFGHIJ", Browser, Group::Browsers),
            ("brave.exe", Browser, Group::Browsers),
            ("308046B0AF4A39CB", Browser, Group::Browsers),
        ] {
            assert_eq!(classify_gsmtc(id), Some(Source { player, group }), "{id}");
        }
        for no in [
            "",
            "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic",
            "Microsoft.ZuneVideo_8wekyb3d8bbwe!Microsoft.ZuneVideo",
            "VLC.exe",
            "Discord.exe",
            "Chrome._crx_agimnkijcaahngcdmfeangaknmldooml", // YouTube (not Music) installed from Chrome
            "Chromebook.exe",
            "Chromecast",
            "NotSpotify.exe",
            "spotify.exe.evil",
            "tidal.exe.evil",
            "Microsoft.MicrosoftEdge.Stable_8wekyb3d8bbwe!App",
            "1234567890ABCDEF",
        ] {
            assert_eq!(classify_gsmtc(no), None, "{no}");
        }
    }

    /// The Linux allowlist (#322), with bus names as players register them.
    #[test]
    fn linux_players_by_bus_name() {
        use MusicPlayer::*;
        for (name, player, group) in [
            ("org.mpris.MediaPlayer2.spotify", Spotify, Group::Spotify),
            (
                "org.mpris.MediaPlayer2.youtube-music",
                YoutubeMusic,
                Group::Players,
            ),
            ("org.mpris.MediaPlayer2.tidal-hifi", Tidal, Group::Players),
            ("org.mpris.MediaPlayer2.cider", AppleMusic, Group::Players),
            (
                "org.mpris.MediaPlayer2.chromium.instance4821",
                Browser,
                Group::Browsers,
            ),
            (
                "org.mpris.MediaPlayer2.brave.instance77",
                Browser,
                Group::Browsers,
            ),
            (
                "org.mpris.MediaPlayer2.firefox.instance_1_23",
                Browser,
                Group::Browsers,
            ),
            (
                "org.mpris.MediaPlayer2.plasma-browser-integration",
                Browser,
                Group::Browsers,
            ),
        ] {
            assert_eq!(
                classify_mpris(name),
                Some(Source { player, group }),
                "{name}"
            );
        }
        for no in [
            "org.mpris.MediaPlayer2.spotifyd",
            "org.mpris.MediaPlayer2.vlc",
            "org.mpris.MediaPlayer2.mpv",
            "org.mpris.MediaPlayer2.kdeconnect.mpris_abc",
            "org.mpris.MediaPlayer2.pulsetest_1",
            "org.mpris.MediaPlayer2",
            "org.example.spotify",
            "spotify",
        ] {
            assert_eq!(classify_mpris(no), None, "{no}");
        }
    }

    /// A browser tab on Windows (no page address): music sites' metadata (artist and album) counts, a video's
    /// (YouTube sets the channel as artist, no album) doesn't.
    #[test]
    fn a_browser_tab_counts_only_when_it_looks_like_music() {
        let ytm = now_playing_gsmtc(BROWSER, GSMTC_PLAYING, "Song", "Alex", "Album").unwrap();
        assert_eq!(ytm.player, MusicPlayer::Browser);
        assert_eq!(ytm.spotify_id, None);
        for (title, artist, album) in [
            ("Funny cat compilation", "Sam's channel", ""),
            ("A video", "", ""),
            ("Something", "", "Album"),
            ("Meeting recording", "  ", "  "),
        ] {
            assert_eq!(
                now_playing_gsmtc(BROWSER, GSMTC_PLAYING, title, artist, album),
                None,
                "{title}"
            );
        }
        // a desktop player needs no album
        let tidal = Source {
            player: MusicPlayer::Tidal,
            group: Group::Players,
        };
        let np = now_playing_gsmtc(tidal, GSMTC_PLAYING, "Song", "Kim", "").unwrap();
        assert_eq!(np.player, MusicPlayer::Tidal);
    }

    /// A browser on Linux: with the page's address, only known music sites count (and say which); without it, the
    /// same metadata rule as Windows. Only Spotify's keeps an id.
    #[test]
    fn a_browser_page_counts_only_on_a_music_site() {
        let m = |url: Option<&str>| MprisMeta {
            trackid: None,
            url: url.map(str::to_owned),
            title: Some("Song".into()),
            artists: vec!["Jo".into()],
            album: Some("Album".into()),
        };
        let at = |url: Option<&str>| now_playing_mpris(BROWSER, Some("Playing"), &m(url));
        assert_eq!(
            at(Some("https://music.youtube.com/watch?v=abc"))
                .unwrap()
                .player,
            MusicPlayer::YoutubeMusic
        );
        assert_eq!(
            at(Some("https://listen.tidal.com/album/1")).unwrap().player,
            MusicPlayer::Tidal
        );
        let sp = at(Some(&format!("https://open.spotify.com/track/{ID}"))).unwrap();
        assert_eq!(
            (sp.player, sp.spotify_id),
            (MusicPlayer::Spotify, Some(format!("track:{ID}")))
        );
        for no in [
            "https://www.youtube.com/watch?v=abc",
            "https://music.youtube.com.evil.test/watch",
            "http://music.youtube.com/watch",
            "file:///home/alex/video.mp4",
        ] {
            assert_eq!(at(Some(no)), None, "{no}");
        }
        assert_eq!(at(None).unwrap().player, MusicPlayer::Browser);
        let video = MprisMeta {
            album: None,
            ..m(None)
        };
        assert_eq!(now_playing_mpris(BROWSER, Some("Playing"), &video), None);
        // a desktop player's Spotify-looking id is never kept
        let ytm = Source {
            player: MusicPlayer::YoutubeMusic,
            group: Group::Players,
        };
        let np = now_playing_mpris(ytm, Some("Playing"), &meta()).unwrap();
        assert_eq!(
            (np.player, np.spotify_id),
            (MusicPlayer::YoutubeMusic, None)
        );
    }

    #[test]
    fn spotify_wins_then_a_player_then_a_browser() {
        let np = |t: &str, player| NowPlaying {
            title: t.into(),
            artist: None,
            album: None,
            spotify_id: None,
            player,
        };
        let b = np("b", MusicPlayer::Browser);
        let d = np("d", MusicPlayer::Deezer);
        let s = np("s", MusicPlayer::Spotify);
        assert_eq!(best([None, Some(b.clone()), None]), Some(b.clone()));
        assert_eq!(best([Some(b.clone()), Some(d.clone())]), Some(d.clone()));
        assert_eq!(best([Some(b), Some(d.clone()), Some(s.clone())]), Some(s));
        assert_eq!(
            best([Some(d.clone()), Some(np("t", MusicPlayer::Tidal))]),
            Some(d),
            "the first of equals"
        );
        assert_eq!(best([None, None]), None);
    }

    #[test]
    fn sources_switches() {
        let off = Sources {
            players: false,
            browsers: false,
        };
        assert!(off.allows(Group::Spotify));
        assert!(!off.allows(Group::Players) && !off.allows(Group::Browsers));
        assert!(Sources::default().allows(Group::Browsers));
    }
}
