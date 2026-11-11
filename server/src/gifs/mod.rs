//! GIFs (#58). GIPHY's terms forbid proxying its API or media and caching or storing what it serves, so the server
//! never calls GIPHY: it hands its key to signed-in apps (`/gifs/config`), which search GIPHY themselves. What Pulse
//! keeps is a GIF's id, title and size: on a message sent from the picker (`messages.gif`) and in each person's
//! favourites. Clients draw a GIF from GIPHY's media links for its id.

pub mod routes;

use pulse_protocol::rest::{Gif, gif_limits};
use reqwest::Url;

use crate::error::{AppError, AppResult};

pub use routes::router;

/// A GIPHY id: letters and digits only, so it is safe in a media URL as it is.
pub fn valid_id(id: &str) -> bool {
    (1..=gif_limits::ID_CHARS).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// A GIPHY media host: `media.giphy.com`, `media0.giphy.com`…, or `i.giphy.com`.
fn media_host(host: &str) -> bool {
    host == "i.giphy.com"
        || host
            .strip_suffix(".giphy.com")
            .and_then(|h| h.strip_prefix("media"))
            .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
}

/// The GIF a GIPHY link is to: a GIF's page (`giphy.com/gifs/<slug>-<id>`, `giphy.com/gifs/<id>`), its embed page
/// (`giphy.com/embed/<id>`) or one of its media files (`media.giphy.com/media/<id>/giphy.gif`,
/// `media2.giphy.com/media/v1.<…>/<id>/200w.webp?…`, `i.giphy.com/<id>.webp`). `None` for anything else, a lookalike
/// host (`giphy.com.example`) included. `link` is one link with nothing around it.
pub fn gif_id(link: &str) -> Option<String> {
    if link.chars().any(char::is_whitespace) {
        return None;
    }
    let url = Url::parse(link).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let segs: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let id = match (host.as_str(), &segs[..]) {
        ("giphy.com" | "www.giphy.com", ["gifs", slug, ..]) => slug.rsplit('-').next()?,
        ("giphy.com" | "www.giphy.com", ["embed", id, ..]) => id,
        (h, ["media", v, id, _file]) if media_host(h) && v.starts_with("v1.") => id,
        (h, ["media", id, _file]) if media_host(h) => id,
        ("i.giphy.com", [file]) => file.split_once('.')?.0,
        _ => return None,
    };
    valid_id(id).then(|| id.to_string())
}

/// A GIF as sent or starred, made safe to keep: the id checked, the size within bounds, the title without control
/// characters, trimmed and cut to [`gif_limits::TITLE_CHARS`].
pub fn clean(g: Gif) -> AppResult<Gif> {
    if !valid_id(&g.id) {
        return Err(AppError::BadRequest("that isn't a GIPHY GIF id".into()));
    }
    let side = 1..=gif_limits::MAX_SIDE;
    if !side.contains(&g.width) || !side.contains(&g.height) {
        return Err(AppError::BadRequest("a GIF's size is out of range".into()));
    }
    let title: String = g.title.chars().filter(|c| !c.is_control()).collect();
    let title = title.trim().chars().take(gif_limits::TITLE_CHARS).collect();
    Ok(Gif { title, ..g })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "3o7TKSjRrfIPjeiVyM";

    #[test]
    fn giphy_links_name_their_gif() {
        for link in [
            format!("https://giphy.com/gifs/excited-yes-{ID}"),
            format!("https://www.giphy.com/gifs/{ID}"),
            format!("http://giphy.com/gifs/cat-{ID}/fullscreen"),
            format!("https://GIPHY.com/gifs/x-{ID}#top"),
            format!("https://giphy.com/embed/{ID}"),
            format!("https://media.giphy.com/media/{ID}/giphy.gif"),
            format!("https://media4.giphy.com/media/{ID}/200w.webp"),
            format!(
                "https://media2.giphy.com/media/v1.Y2lkPTc5MGI3/{ID}/giphy.webp?cid=1&rid=giphy.webp&ct=g"
            ),
            format!("https://i.giphy.com/{ID}.webp"),
            format!("https://i.giphy.com/media/{ID}/giphy.gif"),
        ] {
            assert_eq!(gif_id(&link).as_deref(), Some(ID), "{link}");
        }
    }

    #[test]
    fn anything_else_names_none() {
        for link in [
            "https://giphy.com/",
            "https://giphy.com/explore/cats",
            "https://giphy.com/clips/abc-def",
            "https://giphy.com/stickers",
            "https://giphy.com.evil.example/gifs/abc",
            "https://evilgiphy.com/gifs/abc",
            "https://mediax.giphy.com/media/abc/giphy.gif",
            "https://media.giphy.com/media/../giphy.gif",
            "https://media.giphy.com/media/a%2Fb/giphy.gif",
            "ftp://giphy.com/gifs/abc",
            "giphy.com/gifs/abc",
            "https://giphy.com/gifs/abc def",
            "https://tenor.com/view/abc-123",
            "not a link",
        ] {
            assert_eq!(gif_id(link), None, "{link}");
        }
    }

    #[test]
    fn ids_are_letters_and_digits() {
        assert!(valid_id(ID));
        assert!(!valid_id(""));
        assert!(!valid_id("a-b"));
        assert!(!valid_id("a/b"));
        assert!(!valid_id(&"a".repeat(gif_limits::ID_CHARS + 1)));
    }
}
