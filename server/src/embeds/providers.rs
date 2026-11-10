//! Which provider a link belongs to, its id there, and the provider's player URL. Shared by the
//! unfurler and the embed page; no network here.

use pulse_protocol::rest::EmbedProvider;
use reqwest::Url;

/// A link to something a provider plays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detected {
    pub provider: EmbedProvider,
    /// Valid for [`valid_id`].
    pub id: String,
    /// Where a YouTube video starts, in seconds (at most `page::MAX_START`).
    pub start: Option<u32>,
    /// A 9:16 player (TikTok, Shorts, reels).
    pub vertical: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Detection {
    Known(Detected),
    /// A provider's short link (vm/vt.tiktok.com, tiktok.com/t/, spotify.link,
    /// on.soundcloud.com): what it is shows after following its redirect.
    Short,
    /// Not a provider link (a generic page at most).
    Unknown,
}

/// What `url` links to, by its host (exactly one of the provider's own, so lookalikes such as
/// `youtube.com.example.com` are unknown) and path. Scheme and port don't matter, except that Spotify's app
/// URI for a Jam (`spotify:socialsession:<id>`) is known too.
pub fn detect(url: &Url) -> Detection {
    // Spotify's app URI for a Jam, `spotify:socialsession:<id>` (no host).
    if url.scheme() == "spotify" {
        return match url.path().split(':').collect::<Vec<_>>()[..] {
            ["socialsession", id] => known(jam(id)),
            _ => Detection::Unknown,
        };
    }
    let Some(host) = url.host_str() else {
        return Detection::Unknown;
    };
    let segs: Vec<&str> = url
        .path_segments()
        .map(|s| s.filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    let found = match host {
        "youtube.com"
        | "www.youtube.com"
        | "m.youtube.com"
        | "music.youtube.com"
        | "youtube-nocookie.com"
        | "www.youtube-nocookie.com" => youtube(url, &segs),
        "youtu.be" => match segs[..] {
            [id] => Some(yt(id, url, false)),
            _ => None,
        },
        "tiktok.com" | "www.tiktok.com" | "m.tiktok.com" if segs.first() == Some(&"t") => {
            return Detection::Short;
        }
        "tiktok.com" | "www.tiktok.com" | "m.tiktok.com" => tiktok(&segs),
        "open.spotify.com" => spotify(&segs),
        "soundcloud.com" | "www.soundcloud.com" | "m.soundcloud.com" => soundcloud(&segs),
        "clips.twitch.tv" => match segs[..] {
            [slug] if slug != "embed" => Some(plain(EmbedProvider::Twitch, slug.into())),
            _ => None,
        },
        "twitch.tv" | "www.twitch.tv" | "m.twitch.tv" => match segs[..] {
            [_, "clip", slug] => Some(plain(EmbedProvider::Twitch, slug.into())),
            _ => None,
        },
        "instagram.com" | "www.instagram.com" => instagram(&segs),
        "vm.tiktok.com" | "vt.tiktok.com" | "spotify.link" | "on.soundcloud.com" => {
            return Detection::Short;
        }
        _ => None,
    };
    found.map_or(Detection::Unknown, known)
}

fn known(d: Detected) -> Detection {
    if valid_id(d.provider, &d.id) {
        Detection::Known(d)
    } else {
        Detection::Unknown
    }
}

/// A Spotify Jam invite (#196): a card built from the link alone, never a player.
fn jam(id: &str) -> Detected {
    plain(EmbedProvider::Spotify, format!("{JAM}:{id}"))
}

/// The kind a Spotify Jam's id carries (`jam:<session id>`).
const JAM: &str = "jam";

/// Whether `p`'s `id` is a Spotify Jam invite: no player and nothing to fetch.
pub fn is_jam(p: EmbedProvider, id: &str) -> bool {
    p == EmbedProvider::Spotify && id.split_once(':').is_some_and(|(kind, _)| kind == JAM)
}

/// The Jam invite a `spotify.link` landing page names. A real Jam short link doesn't redirect a link previewer to
/// the Jam: it goes on to a `spotify.app.link` page (200, HTML) that carries the invite's
/// `https://open.spotify.com/socialsession/<id>` (#234). The first such link in `page` that [`detect`] knows as a Jam.
pub fn jam_in_page(page: &str) -> Option<Url> {
    const AT: &str = "https://open.spotify.com/socialsession/";
    let mut rest = page;
    while let Some(i) = rest.find(AT) {
        let tail = &rest[i + AT.len()..];
        let id: String = tail
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if let Ok(u) = Url::parse(&format!("{AT}{id}"))
            && matches!(detect(&u), Detection::Known(ref d) if is_jam(d.provider, &d.id))
        {
            return Some(u);
        }
        rest = tail;
    }
    None
}

fn plain(provider: EmbedProvider, id: String) -> Detected {
    Detected {
        provider,
        id,
        start: None,
        vertical: false,
    }
}

fn yt(id: &str, url: &Url, vertical: bool) -> Detected {
    let param = |name: &str| {
        url.query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
    };
    Detected {
        provider: EmbedProvider::Youtube,
        id: id.into(),
        start: param("t")
            .or_else(|| param("start"))
            .and_then(|t| seconds(&t))
            // The embed page refuses more; a start it can't play is no start.
            .filter(|&s| s <= super::page::MAX_START),
        vertical,
    }
}

fn youtube(url: &Url, segs: &[&str]) -> Option<Detected> {
    match *segs {
        ["watch"] => {
            let v = url.query_pairs().find(|(k, _)| k == "v")?.1;
            Some(yt(&v, url, false))
        }
        ["shorts", id] => Some(yt(id, url, true)),
        ["embed" | "live" | "v", id] => Some(yt(id, url, false)),
        _ => None,
    }
}

/// A YouTube start time: `90`, `90s`, `1m30s` or `1h2m3s`. Zero or unreadable is no start.
fn seconds(t: &str) -> Option<u32> {
    if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) {
        return t.parse().ok().filter(|&n| n > 0);
    }
    let mut total: u32 = 0;
    let mut num = String::new();
    for c in t.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let unit = match c {
            'h' => 3600,
            'm' => 60,
            's' => 1,
            _ => return None,
        };
        let n: u32 = std::mem::take(&mut num).parse().ok()?;
        total = total.checked_add(n.checked_mul(unit)?)?;
    }
    (num.is_empty() && total > 0).then_some(total)
}

fn tiktok(segs: &[&str]) -> Option<Detected> {
    let id = match *segs {
        [user, "video", id] if user.starts_with('@') => id,
        ["v", file] => file.strip_suffix(".html")?,
        ["embed", "v2", id] | ["embed", id] | ["player", "v1", id] => id,
        _ => return None,
    };
    Some(Detected {
        vertical: true,
        ..plain(EmbedProvider::Tiktok, id.into())
    })
}

const SPOTIFY_TYPES: [&str; 6] = ["track", "album", "playlist", "episode", "show", "artist"];

fn spotify(mut segs: &[&str]) -> Option<Detected> {
    if segs.first().is_some_and(|s| s.starts_with("intl-")) {
        segs = &segs[1..];
    }
    if segs.first() == Some(&"embed") {
        segs = &segs[1..];
    }
    match *segs {
        // A Jam invite: `open.spotify.com/socialsession/<id>`.
        ["socialsession", id] => Some(jam(id)),
        [kind, id] => Some(plain(EmbedProvider::Spotify, format!("{kind}:{id}"))),
        _ => None,
    }
}

/// SoundCloud's own pages, which look like `<user>/…` but aren't a user's.
const SOUNDCLOUD_RESERVED: [&str; 14] = [
    "discover",
    "search",
    "you",
    "stream",
    "upload",
    "pages",
    "charts",
    "settings",
    "messages",
    "notifications",
    "people",
    "tags",
    "popular",
    "signin",
];

/// A user's pages that aren't a track.
const SOUNDCLOUD_USER_PAGES: [&str; 11] = [
    "sets",
    "likes",
    "tracks",
    "albums",
    "playlists",
    "reposts",
    "followers",
    "following",
    "comments",
    "popular-tracks",
    "spotlight",
];

fn soundcloud(segs: &[&str]) -> Option<Detected> {
    if SOUNDCLOUD_RESERVED.contains(segs.first()?) {
        return None;
    }
    match *segs {
        [_, track] if !SOUNDCLOUD_USER_PAGES.contains(&track) => {}
        [_, "sets", _] => {}
        _ => return None,
    }
    Some(plain(EmbedProvider::Soundcloud, segs.join("/")))
}

fn instagram(mut segs: &[&str]) -> Option<Detected> {
    // `/<user>/reel/<code>/` is the same reel.
    if let [_, "p" | "reel" | "reels" | "tv", _] = *segs {
        segs = &segs[1..];
    }
    let (kind, code) = match *segs {
        ["p", code] => ("p", code),
        ["reel" | "reels" | "tv", code] => ("reel", code),
        _ => return None,
    };
    Some(Detected {
        vertical: kind == "reel",
        ..plain(EmbedProvider::Instagram, format!("{kind}:{code}"))
    })
}

/// `s` is `min..=max` chars, each one `ok`.
fn chars(s: &str, min: usize, max: usize, ok: impl Fn(u8) -> bool) -> bool {
    (min..=max).contains(&s.len()) && s.bytes().all(ok)
}

fn slug_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Whether `id` is a well-formed id for `p`. Ids hold only characters that need no escaping in a
/// URL, which is what makes [`player_url`]'s templates safe.
pub fn valid_id(p: EmbedProvider, id: &str) -> bool {
    match p {
        EmbedProvider::Youtube => chars(id, 11, 11, slug_char),
        EmbedProvider::Tiktok => chars(id, 5, 25, |b| b.is_ascii_digit()),
        EmbedProvider::Spotify => id.split_once(':').is_some_and(|(kind, id)| {
            if kind == JAM {
                // Spotify doesn't document a session id's shape: a URL-safe slug of sane length.
                return chars(id, 8, 64, slug_char);
            }
            SPOTIFY_TYPES.contains(&kind) && chars(id, 22, 22, |b| b.is_ascii_alphanumeric())
        }),
        EmbedProvider::Soundcloud => {
            let segs: Vec<&str> = id.split('/').collect();
            let shape = match segs[..] {
                [_, track] => track != "sets",
                [_, sets, _] => sets == "sets",
                _ => false,
            };
            shape
                && segs.iter().all(|s| {
                    chars(s, 1, 100, |b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'
                    })
                })
        }
        EmbedProvider::Twitch => chars(id, 1, 100, slug_char),
        EmbedProvider::Instagram => id.split_once(':').is_some_and(|(kind, code)| {
            matches!(kind, "p" | "reel") && chars(code, 5, 64, slug_char)
        }),
        EmbedProvider::Generic | EmbedProvider::Unknown => false,
    }
}

/// The provider's official player for `id`: the only place an iframe URL is built. `None` for an
/// invalid id, a provider without a player, or (Twitch) a `parent_host` that isn't a host name.
/// `start` (seconds) is for YouTube only; `parent_host` (our public host) for Twitch only.
pub fn player_url(
    p: EmbedProvider,
    id: &str,
    start: Option<u32>,
    parent_host: &str,
) -> Option<String> {
    if !valid_id(p, id) || is_jam(p, id) {
        return None;
    }
    let url = match p {
        EmbedProvider::Youtube => {
            let start = start
                .filter(|&s| s > 0)
                .map(|s| format!("&start={s}"))
                .unwrap_or_default();
            format!("https://www.youtube-nocookie.com/embed/{id}?autoplay=1{start}")
        }
        EmbedProvider::Tiktok => format!("https://www.tiktok.com/player/v1/{id}?autoplay=1&rel=0"),
        EmbedProvider::Spotify => {
            let (kind, id) = id.split_once(':')?;
            format!("https://open.spotify.com/embed/{kind}/{id}")
        }
        // The track's URL is a query value: its slashes are escaped like the rest of it.
        EmbedProvider::Soundcloud => format!(
            "https://w.soundcloud.com/player/?url=https%3A%2F%2Fsoundcloud.com%2F{}&auto_play=true",
            id.replace('/', "%2F")
        ),
        EmbedProvider::Twitch => {
            let host_ok = chars(parent_host, 1, 253, |b| {
                b.is_ascii_alphanumeric() || b == b'.' || b == b'-'
            });
            if !host_ok {
                return None;
            }
            format!(
                "https://clips.twitch.tv/embed?clip={id}&parent={parent_host}&parent=tauri.localhost&parent=localhost&autoplay=true"
            )
        }
        EmbedProvider::Instagram => {
            let (kind, code) = id.split_once(':')?;
            format!("https://www.instagram.com/{kind}/{code}/embed/")
        }
        EmbedProvider::Generic | EmbedProvider::Unknown => return None,
    };
    Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_jam_named_on_a_landing_page() {
        let page = r#"<img src="https://shareables.scdn.co/publish/socialsession/x.png">
            <a href="https://open.spotify.com/socialsession/AbC123abc123_-x?si=1&amp;t=2">Open</a>"#;
        assert_eq!(
            jam_in_page(page).map(String::from).as_deref(),
            Some("https://open.spotify.com/socialsession/AbC123abc123_-x")
        );
        assert_eq!(
            jam_in_page("https://open.spotify.com/socialsession/ab"),
            None
        );
        assert_eq!(
            jam_in_page("https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC"),
            None
        );
        assert_eq!(jam_in_page(""), None);
    }
    use EmbedProvider::*;

    const YT: &str = "dQw4w9WgXcQ";
    const TT: &str = "7000000000000000001";
    const SP: &str = "4uLU6hMCjMI75M1A2tKUQC";

    fn det(url: &str) -> Detection {
        detect(&Url::parse(url).unwrap())
    }

    fn known(p: EmbedProvider, id: &str, start: Option<u32>, vertical: bool) -> Detection {
        Detection::Known(Detected {
            provider: p,
            id: id.into(),
            start,
            vertical,
        })
    }

    #[test]
    fn youtube_forms() {
        let plain = known(Youtube, YT, None, false);
        for u in [
            format!("https://www.youtube.com/watch?v={YT}"),
            format!("https://youtube.com/watch?feature=share&v={YT}"),
            format!("https://m.youtube.com/watch?v={YT}"),
            format!("https://music.youtube.com/watch?v={YT}&list=PL1"),
            format!("https://youtu.be/{YT}"),
            format!("https://www.youtube.com/embed/{YT}"),
            format!("https://www.youtube-nocookie.com/embed/{YT}"),
            format!("https://www.youtube.com/live/{YT}?si=abc"),
            format!("http://www.youtube.com:8080/watch?v={YT}"),
        ] {
            assert_eq!(det(&u), plain, "{u}");
        }
        assert_eq!(
            det(&format!("https://youtu.be/{YT}?t=1m30s")),
            known(Youtube, YT, Some(90), false)
        );
        assert_eq!(
            det(&format!("https://www.youtube.com/watch?v={YT}&t=90s")),
            known(Youtube, YT, Some(90), false)
        );
        assert_eq!(
            det(&format!("https://www.youtube.com/watch?v={YT}&t=90")),
            known(Youtube, YT, Some(90), false)
        );
        assert_eq!(
            det(&format!("https://www.youtube.com/watch?v={YT}&t=1h2m3s")),
            known(Youtube, YT, Some(3723), false)
        );
        assert_eq!(
            det(&format!("https://www.youtube.com/embed/{YT}?start=42")),
            known(Youtube, YT, Some(42), false)
        );
        // A start it can't read is dropped, not the video.
        assert_eq!(
            det(&format!("https://www.youtube.com/watch?v={YT}&t=soon")),
            plain
        );
        // So is one past the embed page's limit (a week).
        assert_eq!(
            det(&format!("https://www.youtube.com/watch?v={YT}&t=604800")),
            known(Youtube, YT, Some(604_800), false)
        );
        for t in ["604801", "168h0m1s", "4294967295"] {
            assert_eq!(
                det(&format!("https://www.youtube.com/watch?v={YT}&t={t}")),
                plain,
                "{t}"
            );
        }
        assert_eq!(
            det(&format!("https://www.youtube.com/shorts/{YT}")),
            known(Youtube, YT, None, true)
        );
        // Not a video.
        for u in [
            "https://www.youtube.com/",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQQ",
            "https://www.youtube.com/@alex",
            "https://www.youtube.com/playlist?list=PL1",
            "https://youtu.be/",
            "https://youtu.be/dQw4w9WgX!Q",
        ] {
            assert_eq!(det(u), Detection::Unknown, "{u}");
        }
    }

    #[test]
    fn tiktok_forms() {
        let v = known(Tiktok, TT, None, true);
        for u in [
            format!("https://www.tiktok.com/@alex/video/{TT}"),
            format!("https://www.tiktok.com/@alex/video/{TT}?is_from_webapp=1"),
            format!("https://tiktok.com/@alex.sam_1/video/{TT}/"),
            format!("https://m.tiktok.com/v/{TT}.html"),
            format!("https://www.tiktok.com/embed/v2/{TT}"),
            format!("https://www.tiktok.com/embed/{TT}"),
            format!("https://www.tiktok.com/player/v1/{TT}"),
        ] {
            assert_eq!(det(&u), v, "{u}");
        }
        for u in [
            "https://vm.tiktok.com/ZMabcdef/",
            "https://vt.tiktok.com/ZSabcdef/",
            "https://www.tiktok.com/t/ZTabcdef/",
        ] {
            assert_eq!(det(u), Detection::Short, "{u}");
        }
        for u in [
            "https://www.tiktok.com/@alex",
            "https://www.tiktok.com/@alex/video/123",
            "https://www.tiktok.com/@alex/video/12345678901234567890123456",
            "https://www.tiktok.com/@alex/video/70000x",
            "https://www.tiktok.com/foryou",
        ] {
            assert_eq!(det(u), Detection::Unknown, "{u}");
        }
    }

    #[test]
    fn spotify_forms() {
        for (u, id) in [
            (
                format!("https://open.spotify.com/track/{SP}"),
                format!("track:{SP}"),
            ),
            (
                format!("https://open.spotify.com/intl-de/track/{SP}?si=x"),
                format!("track:{SP}"),
            ),
            (
                format!("https://open.spotify.com/album/{SP}"),
                format!("album:{SP}"),
            ),
            (
                format!("https://open.spotify.com/playlist/{SP}"),
                format!("playlist:{SP}"),
            ),
            (
                format!("https://open.spotify.com/episode/{SP}"),
                format!("episode:{SP}"),
            ),
            (
                format!("https://open.spotify.com/show/{SP}"),
                format!("show:{SP}"),
            ),
            (
                format!("https://open.spotify.com/artist/{SP}"),
                format!("artist:{SP}"),
            ),
            (
                format!("https://open.spotify.com/embed/track/{SP}"),
                format!("track:{SP}"),
            ),
        ] {
            assert_eq!(det(&u), known(Spotify, &id, None, false), "{u}");
        }
        assert_eq!(det("https://spotify.link/abcDEF123"), Detection::Short);
        for u in [
            format!("https://open.spotify.com/user/{SP}"),
            "https://open.spotify.com/track/short".to_string(),
            "https://open.spotify.com/".to_string(),
            format!("https://spotify.com/track/{SP}"),
        ] {
            assert_eq!(det(&u), Detection::Unknown, "{u}");
        }
    }

    #[test]
    fn spotify_jam_forms() {
        const JAM: &str = "ABC123abc123ABC123abc123";
        let jam = known(Spotify, &format!("jam:{JAM}"), None, false);
        for u in [
            format!("https://open.spotify.com/socialsession/{JAM}"),
            format!("https://open.spotify.com/socialsession/{JAM}?si=x&dlsi=y"),
            format!("https://open.spotify.com/intl-de/socialsession/{JAM}"),
            format!("spotify:socialsession:{JAM}"),
            format!("SPOTIFY:socialsession:{JAM}"),
        ] {
            assert_eq!(det(&u), jam, "{u}");
        }
        for u in [
            "https://open.spotify.com/socialsession/".to_string(),
            "https://open.spotify.com/socialsession/short".to_string(),
            format!("https://open.spotify.com/socialsession/{JAM}/extra"),
            format!("https://open.spotify.com/socialsession/{}", "a".repeat(65)),
            "https://open.spotify.com/socialsession/abc-def%2Fghi".to_string(),
            format!("https://spotify.com/socialsession/{JAM}"),
            format!("spotify:track:{JAM}"),
            format!("spotify:socialsession:{JAM}:x"),
            "spotify:socialsession:a/b/c/d/e/f/g".to_string(),
        ] {
            assert_eq!(det(&u), Detection::Unknown, "{u}");
        }
        // A Jam has no player: the embed page refuses it.
        assert!(valid_id(Spotify, &format!("jam:{JAM}")));
        assert!(is_jam(Spotify, &format!("jam:{JAM}")));
        assert!(!is_jam(Spotify, &format!("track:{SP}")));
        assert!(!is_jam(Generic, &format!("jam:{JAM}")));
        assert_eq!(
            player_url(Spotify, &format!("jam:{JAM}"), None, "pulse.example.com"),
            None
        );
    }

    #[test]
    fn soundcloud_forms() {
        assert_eq!(
            det("https://soundcloud.com/alex-music/first-track"),
            known(Soundcloud, "alex-music/first-track", None, false)
        );
        assert_eq!(
            det("https://m.soundcloud.com/alex_1/first-track?si=1"),
            known(Soundcloud, "alex_1/first-track", None, false)
        );
        assert_eq!(
            det("https://soundcloud.com/alex/sets/summer-mix/"),
            known(Soundcloud, "alex/sets/summer-mix", None, false)
        );
        assert_eq!(det("https://on.soundcloud.com/AbCd1"), Detection::Short);
        for u in [
            "https://soundcloud.com/discover/sets",
            "https://soundcloud.com/search/sounds",
            "https://soundcloud.com/you/library",
            "https://soundcloud.com/stream/x",
            "https://soundcloud.com/upload/x",
            "https://soundcloud.com/pages/privacy",
            "https://soundcloud.com/charts/top",
            "https://soundcloud.com/alex",
            "https://soundcloud.com/alex/likes",
            "https://soundcloud.com/alex/sets",
            "https://soundcloud.com/alex/first-track/s-SECRET",
            "https://soundcloud.com/alex/first.track",
        ] {
            assert_eq!(det(u), Detection::Unknown, "{u}");
        }
    }

    #[test]
    fn twitch_forms() {
        let c = known(Twitch, "FunnyClip-abc_12", None, false);
        assert_eq!(det("https://clips.twitch.tv/FunnyClip-abc_12"), c);
        assert_eq!(det("https://www.twitch.tv/alex/clip/FunnyClip-abc_12"), c);
        assert_eq!(det("https://m.twitch.tv/alex/clip/FunnyClip-abc_12?x=1"), c);
        for u in [
            "https://www.twitch.tv/alex",
            "https://www.twitch.tv/videos/123",
            "https://clips.twitch.tv/",
            "https://clips.twitch.tv/bad.slug",
        ] {
            assert_eq!(det(u), Detection::Unknown, "{u}");
        }
    }

    #[test]
    fn instagram_forms() {
        let post = known(Instagram, "p:Cabc_12-x", None, false);
        let reel = known(Instagram, "reel:Cabc_12-x", None, true);
        assert_eq!(det("https://www.instagram.com/p/Cabc_12-x/"), post);
        assert_eq!(det("https://instagram.com/p/Cabc_12-x"), post);
        assert_eq!(det("https://www.instagram.com/alex/p/Cabc_12-x/"), post);
        assert_eq!(
            det("https://www.instagram.com/reel/Cabc_12-x/?igsh=1"),
            reel
        );
        assert_eq!(det("https://www.instagram.com/reels/Cabc_12-x/"), reel);
        assert_eq!(det("https://www.instagram.com/tv/Cabc_12-x/"), reel);
        assert_eq!(det("https://www.instagram.com/alex/reel/Cabc_12-x/"), reel);
        for u in [
            "https://www.instagram.com/alex/",
            "https://www.instagram.com/p/abc/",
            "https://www.instagram.com/explore/tags/cats/",
            "https://www.instagram.com/stories/alex/123/",
        ] {
            assert_eq!(det(u), Detection::Unknown, "{u}");
        }
    }

    #[test]
    fn lookalike_hosts_are_unknown() {
        for u in [
            format!("https://youtube.com.example.com/watch?v={YT}"),
            format!("https://notyoutube.com/watch?v={YT}"),
            format!("https://youtu.be.example.com/{YT}"),
            format!("https://evil-tiktok.com/@alex/video/{TT}"),
            format!("https://open.spotify.com.example.test/track/{SP}"),
            "https://instagram.com.example.com/p/Cabc_12-x/".to_string(),
            "https://example.com/watch?v=dQw4w9WgXcQ".to_string(),
        ] {
            assert_eq!(det(&u), Detection::Unknown, "{u}");
        }
    }

    #[test]
    fn ids_are_validated() {
        assert!(valid_id(Youtube, YT));
        assert!(!valid_id(Youtube, "dQw4w9WgXc"));
        assert!(!valid_id(Youtube, "dQw4w9WgX/Q"));
        assert!(valid_id(Tiktok, "12345"));
        assert!(!valid_id(Tiktok, "1234"));
        assert!(!valid_id(Tiktok, "12345a"));
        assert!(valid_id(Spotify, &format!("track:{SP}")));
        assert!(!valid_id(Spotify, &format!("user:{SP}")));
        assert!(!valid_id(Spotify, SP));
        assert!(valid_id(Soundcloud, "alex/first-track"));
        assert!(valid_id(Soundcloud, "alex/sets/summer"));
        assert!(!valid_id(Soundcloud, "alex"));
        assert!(!valid_id(Soundcloud, "alex/../x"));
        assert!(!valid_id(Soundcloud, "Alex/track"));
        assert!(!valid_id(Soundcloud, &format!("alex/{}", "a".repeat(101))));
        assert!(valid_id(Twitch, "FunnyClip-abc_12"));
        assert!(!valid_id(Twitch, ""));
        assert!(!valid_id(Twitch, &"a".repeat(101)));
        assert!(valid_id(Instagram, "p:Cabc1"));
        assert!(valid_id(Instagram, "reel:Cabc1"));
        assert!(!valid_id(Instagram, "tv:Cabc1"));
        assert!(!valid_id(Instagram, "p:Cab"));
        for bad in ["", "../", "%2F", "a\"b", "<x>", "a b", "a?b", "a#b", "a&b"] {
            for p in [Youtube, Tiktok, Spotify, Soundcloud, Twitch, Instagram] {
                assert!(!valid_id(p, bad), "{p:?} {bad}");
            }
        }
        assert!(!valid_id(Generic, "x"));
        assert!(!valid_id(Unknown, "x"));
    }

    #[test]
    fn player_urls_come_from_templates() {
        assert_eq!(
            player_url(Youtube, YT, None, "pulse.example.com").as_deref(),
            Some("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1")
        );
        assert_eq!(
            player_url(Youtube, YT, Some(90), "pulse.example.com").as_deref(),
            Some("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1&start=90")
        );
        assert_eq!(
            player_url(Tiktok, TT, Some(5), "pulse.example.com").as_deref(),
            Some("https://www.tiktok.com/player/v1/7000000000000000001?autoplay=1&rel=0")
        );
        assert_eq!(
            player_url(Spotify, &format!("album:{SP}"), None, "pulse.example.com").as_deref(),
            Some("https://open.spotify.com/embed/album/4uLU6hMCjMI75M1A2tKUQC")
        );
        assert_eq!(
            player_url(
                Soundcloud,
                "alex/sets/summer-mix",
                None,
                "pulse.example.com"
            )
            .as_deref(),
            Some(
                "https://w.soundcloud.com/player/?url=https%3A%2F%2Fsoundcloud.com%2Falex%2Fsets%2Fsummer-mix&auto_play=true"
            )
        );
        assert_eq!(
            player_url(Twitch, "FunnyClip-abc_12", None, "pulse.example.com").as_deref(),
            Some(
                "https://clips.twitch.tv/embed?clip=FunnyClip-abc_12&parent=pulse.example.com&parent=tauri.localhost&parent=localhost&autoplay=true"
            )
        );
        assert_eq!(
            player_url(Instagram, "p:Cabc_12-x", None, "pulse.example.com").as_deref(),
            Some("https://www.instagram.com/p/Cabc_12-x/embed/")
        );
        assert_eq!(
            player_url(Instagram, "reel:Cabc_12-x", None, "pulse.example.com").as_deref(),
            Some("https://www.instagram.com/reel/Cabc_12-x/embed/")
        );
    }

    #[test]
    fn player_urls_refuse_bad_input() {
        for p in [Youtube, Tiktok, Spotify, Soundcloud, Twitch, Instagram] {
            assert_eq!(player_url(p, "../x", None, "pulse.example.com"), None);
            assert_eq!(
                player_url(p, "a\"><script>", None, "pulse.example.com"),
                None
            );
        }
        assert_eq!(player_url(Generic, "x", None, "pulse.example.com"), None);
        assert_eq!(player_url(Unknown, YT, None, "pulse.example.com"), None);
        // Twitch's parent goes into the URL too: a host name only.
        for parent in ["", "a&parent=evil.test", "a/b", "a b", "a\"b"] {
            assert_eq!(player_url(Twitch, "Clip1", None, parent), None, "{parent}");
        }
    }
}
