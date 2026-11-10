//! One link's preview, per provider: official oEmbed for the players that have a tokenless one,
//! OpenGraph for Twitch clips and everything else, and the thumbnail fetched, re-encoded and
//! stored as `link_thumb` media. Every fetch goes through the guarded [`Fetcher`].
//!
//! oEmbed is asked about the link's canonical URL, built from the validated id (so whatever
//! else the sender's link carried, such as tracking parameters, never reaches the provider):
//! - YouTube `https://www.youtube.com/watch?v=<id>` (Shorts too; YouTube answers for either);
//! - TikTok `https://www.tiktok.com/@<user>/video/<id>`, with the link's user when it has one
//!   (`@_` otherwise: TikTok finds the video by its id);
//! - Spotify `https://open.spotify.com/<type>/<id>`, SoundCloud `https://soundcloud.com/<path>`,
//!   Instagram `https://www.instagram.com/<p|reel>/<code>/`.
//!
//! A Spotify Jam invite (`open.spotify.com/socialsession/<id>`, or a `spotify.link` short link
//! that resolves to one) fetches nothing: its page gives a preview fetcher nothing usable, so its
//! card is fixed text built from the link.
//!
//! Twitch has no tokenless oEmbed, so a clip's card is its page's OpenGraph, read from the link
//! as sent (`clips.twitch.tv/<slug>` and `twitch.tv/<user>/clip/<slug>` both carry it).

use std::path::Path;

use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{EmbedKind, EmbedProvider, EmbedThumb};
use reqwest::Url;
use sqlx::SqlitePool;

use super::fetch::{Fetcher, Want};
use super::meta::{self, OEmbed};
use super::providers::{self, Detected, Detection};
use crate::config::Endpoints;
use crate::media::{self, MediaKind, process};

/// A link's preview as the unfurler builds it: a `LinkEmbed` without its `url`.
#[derive(Clone, Debug, PartialEq)]
pub struct Unfurled {
    pub provider: EmbedProvider,
    pub kind: EmbedKind,
    /// The validated id of a player; `None` for a generic card.
    pub provider_id: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub site_name: Option<String>,
    pub author: Option<String>,
    pub thumb: Option<EmbedThumb>,
    pub start: Option<u32>,
    pub vertical: bool,
}

/// What a card says, before its thumbnail is fetched.
#[derive(Default)]
struct Text {
    title: Option<String>,
    description: Option<String>,
    site_name: Option<String>,
    author: Option<String>,
    /// Absolute, resolved against where it was found.
    image: Option<Url>,
}

/// `url`'s preview (`url` normalised by the caller), or `None` when there's nothing to show.
/// `thumb_ok` is asked just before a thumbnail would be fetched; `false` leaves it out.
pub(super) async fn run(
    fetcher: &Fetcher,
    endpoints: &Endpoints,
    db: &SqlitePool,
    media_dir: &Path,
    by: UserId,
    thumb_ok: &(impl Fn() -> bool + Sync),
    url: &str,
) -> Option<Unfurled> {
    let mut link = Url::parse(url).ok()?;
    let mut found = providers::detect(&link);
    if found == Detection::Short {
        link = match fetcher.resolve_redirects(url).await {
            Ok(u) => u,
            // A real Jam short link lands a previewer on a page naming the Jam instead (#234): read the Jam's link
            // from it. Only the invite is taken; its card is still built from the link alone.
            Err(_) if link.host_str() == Some("spotify.link") => {
                let got = fetcher.get(url, Want::Html).await.ok()?;
                providers::jam_in_page(&String::from_utf8_lossy(&got.body))?
            }
            Err(_) => return None,
        };
        found = providers::detect(&link);
    }
    let (d, text) = match found {
        // A Jam's page says nothing useful to a preview fetcher: its card is built from the
        // link alone, and nothing is fetched (#196).
        Detection::Known(d) if providers::is_jam(d.provider, &d.id) => return Some(jam(d)),
        Detection::Known(d) => {
            let text = match d.provider {
                EmbedProvider::Twitch => {
                    let mut t = page(fetcher, link.as_str()).await?;
                    t.site_name = Some("Twitch".into());
                    t
                }
                // Tokenless, no thumbnail and often refused: the card shows without it.
                EmbedProvider::Instagram => {
                    let mut t = oembed(fetcher, &endpoints.instagram, &canonical(&d, &link), false)
                        .await
                        .unwrap_or_default();
                    t.site_name = Some("Instagram".into());
                    t
                }
                p => {
                    let endpoint = match p {
                        EmbedProvider::Youtube => &endpoints.youtube,
                        EmbedProvider::Tiktok => &endpoints.tiktok,
                        EmbedProvider::Spotify => &endpoints.spotify,
                        _ => &endpoints.soundcloud,
                    };
                    let mut t = oembed(fetcher, endpoint, &canonical(&d, &link), true).await?;
                    t.site_name = t.site_name.or_else(|| Some(name(p).into()));
                    t
                }
            };
            (Some(d), text)
        }
        Detection::Short => return None,
        Detection::Unknown => {
            let mut t = page(fetcher, url).await?;
            if t.title.is_none() && t.image.is_none() {
                return None;
            }
            if t.site_name.is_none() {
                let host = link.host_str().unwrap_or("");
                t.site_name = meta::clean(host.strip_prefix("www.").unwrap_or(host), 80);
            }
            (None, t)
        }
    };
    let thumb = match &text.image {
        Some(img) if thumb_ok() => thumb(fetcher, db, media_dir, by, img).await,
        Some(_) => None,
        None => None,
    };
    Some(match d {
        Some(d) => Unfurled {
            provider: d.provider,
            kind: EmbedKind::Player,
            provider_id: Some(d.id),
            title: text.title,
            description: text.description,
            site_name: text.site_name,
            author: text.author,
            thumb,
            start: d.start,
            vertical: d.vertical,
        },
        None => Unfurled {
            provider: EmbedProvider::Generic,
            kind: EmbedKind::Card,
            provider_id: None,
            title: text.title,
            description: text.description,
            site_name: text.site_name,
            author: None,
            thumb,
            start: None,
            vertical: false,
        },
    })
}

/// A Spotify Jam invite's card: fixed text, no thumbnail. A card, not a player (there is none),
/// so a client that doesn't know Jams shows it as a plain Spotify card.
fn jam(d: Detected) -> Unfurled {
    Unfurled {
        provider: d.provider,
        kind: EmbedKind::Card,
        provider_id: Some(d.id),
        title: Some("Join the Jam".into()),
        description: None,
        site_name: Some(name(d.provider).into()),
        author: None,
        thumb: None,
        start: None,
        vertical: false,
    }
}

/// A provider's name, for a card whose oEmbed didn't give one.
fn name(p: EmbedProvider) -> &'static str {
    match p {
        EmbedProvider::Youtube => "YouTube",
        EmbedProvider::Tiktok => "TikTok",
        EmbedProvider::Spotify => "Spotify",
        EmbedProvider::Soundcloud => "SoundCloud",
        EmbedProvider::Twitch => "Twitch",
        EmbedProvider::Instagram => "Instagram",
        EmbedProvider::Generic | EmbedProvider::Unknown => "",
    }
}

/// The URL oEmbed is asked about (see the module doc). `link` is the detected link, for
/// TikTok's user.
fn canonical(d: &Detected, link: &Url) -> String {
    match d.provider {
        EmbedProvider::Youtube => format!("https://www.youtube.com/watch?v={}", d.id),
        EmbedProvider::Tiktok => {
            let user = link
                .path_segments()
                .and_then(|mut s| s.next())
                .and_then(|u| u.strip_prefix('@'))
                .filter(|u| {
                    !u.is_empty()
                        && u.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
                })
                .unwrap_or("_");
            format!("https://www.tiktok.com/@{user}/video/{}", d.id)
        }
        EmbedProvider::Spotify => {
            let (kind, id) = d.id.split_once(':').unwrap_or(("track", &d.id));
            format!("https://open.spotify.com/{kind}/{id}")
        }
        EmbedProvider::Soundcloud => format!("https://soundcloud.com/{}", d.id),
        EmbedProvider::Instagram => {
            let (kind, code) = d.id.split_once(':').unwrap_or(("p", &d.id));
            format!("https://www.instagram.com/{kind}/{code}/")
        }
        EmbedProvider::Twitch | EmbedProvider::Generic | EmbedProvider::Unknown => link.to_string(),
    }
}

/// The card text an oEmbed `endpoint` gives for `target`, `None` when the call fails.
async fn oembed(fetcher: &Fetcher, endpoint: &str, target: &str, json: bool) -> Option<Text> {
    let mut params = vec![("url", target)];
    if json {
        params.push(("format", "json"));
    }
    let q = Url::parse_with_params(endpoint, &params).ok()?;
    let got = fetcher.get(q.as_str(), Want::Json).await.ok()?;
    let o = OEmbed::parse(&got.body)?;
    Some(Text {
        title: o.title.and_then(|s| meta::clean(&s, meta::TITLE_MAX)),
        description: o
            .description
            .and_then(|s| meta::clean(&s, meta::DESCRIPTION_MAX)),
        site_name: o
            .provider_name
            .and_then(|s| meta::clean(&s, meta::SITE_NAME_MAX)),
        author: o
            .author_name
            .and_then(|s| meta::clean(&s, meta::AUTHOR_MAX)),
        image: o.thumbnail_url.and_then(|u| got.final_url.join(&u).ok()),
    })
}

/// The card text of the page at `url`, `None` when it can't be fetched.
async fn page(fetcher: &Fetcher, url: &str) -> Option<Text> {
    let got = fetcher.get(url, Want::Html).await.ok()?;
    // Up to a megabyte of markup to scan: off the async workers.
    let body = got.body;
    let m = tokio::task::spawn_blocking(move || meta::page_meta(&String::from_utf8_lossy(&body)))
        .await
        .ok()?;
    Some(Text {
        title: m.title.and_then(|s| meta::clean(&s, meta::TITLE_MAX)),
        description: m
            .description
            .and_then(|s| meta::clean(&s, meta::DESCRIPTION_MAX)),
        site_name: m
            .site_name
            .and_then(|s| meta::clean(&s, meta::SITE_NAME_MAX)),
        author: None,
        // Relative to where the page came from, after its redirects.
        image: m.image.and_then(|u| got.final_url.join(u.trim()).ok()),
    })
}

/// The thumbnail at `url`, fetched, re-encoded and stored; `None` if any of that fails (the card
/// goes without one).
async fn thumb(
    fetcher: &Fetcher,
    db: &SqlitePool,
    media_dir: &Path,
    by: UserId,
    url: &Url,
) -> Option<EmbedThumb> {
    let got = fetcher.get(url.as_str(), Want::Image).await.ok()?;
    let processed = media::routes::run_processing(move || process::link_thumb(&got.body))
        .await
        .ok()?
        .ok()?;
    match media::put(db, media_dir, MediaKind::LinkThumb, &processed, by).await {
        Ok(info) => Some(EmbedThumb {
            hash: info.hash,
            width: info.width,
            height: info.height,
        }),
        Err(e) => {
            tracing::warn!(error = %e, "link thumbnail not stored");
            None
        }
    }
}
