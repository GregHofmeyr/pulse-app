//! GIF search (#58). GIPHY's terms want its API called straight from the app (never proxied) and its answers never
//! cached, so the core asks the Pulse server for the key once a session (`/gifs/config`, signed-in only) and then
//! calls GIPHY itself for every search and trending page, handing the picker GIPHY's own URLs untouched. The key
//! never reaches the webview. Favourites live on the Pulse server.

use std::sync::Mutex;
use std::time::Duration;

use pulse_protocol::rest::Gif;
use serde::{Deserialize, Deserializer, Serialize};
use tauri::{AppHandle, State};

use crate::api::{Api, ApiError};
use crate::commands::Core;

/// GIPHY's GIF endpoints.
pub const GIPHY_API: &str = "https://api.giphy.com/v1/gifs";
/// GIFs per page (GIPHY's beta keys allow up to 50).
pub const PAGE: u32 = 24;
/// GIPHY's longest search.
const MAX_QUERY_CHARS: usize = 50;
/// The furthest GIPHY pages each feed.
const SEARCH_MAX_OFFSET: u32 = 4999;
const TRENDING_MAX_OFFSET: u32 = 499;

const BUSY: &str = "GIF search is busy right now. Try again in a minute.";
const KEY_REFUSED: &str =
    "GIPHY refused this server's key, so GIF search isn't working. Tell whoever runs the server.";
const UNREACHABLE: &str = "Can't reach GIPHY right now. Try again in a bit.";

/// One GIF in the picker, as GIPHY gave it: `preview` and `still` are GIPHY's URLs, untouched (its terms).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GifResult {
    pub id: String,
    pub title: String,
    /// The GIF's GIPHY page: what a sent GIF's message says.
    pub url: String,
    /// The GIF's own size (its shape, for the message).
    pub width: u32,
    pub height: u32,
    /// The 200 px wide rendition (WebP when GIPHY has one) and its size.
    pub preview: String,
    pub preview_width: u32,
    pub preview_height: u32,
    /// Its first frame.
    pub still: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GifPage {
    pub gifs: Vec<GifResult>,
    /// The offset of the next page, `None` at the end.
    pub next: Option<u32>,
}

/// GIPHY's sizes come as strings ("200"); take numbers too.
fn size<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum N {
        S(String),
        N(u32),
    }
    Ok(match Option::<N>::deserialize(d)? {
        Some(N::S(s)) => s.parse().unwrap_or(0),
        Some(N::N(n)) => n,
        None => 0,
    })
}

#[derive(Deserialize)]
struct Rendition {
    #[serde(default, deserialize_with = "size")]
    width: u32,
    #[serde(default, deserialize_with = "size")]
    height: u32,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    webp: Option<String>,
}

#[derive(Deserialize)]
struct Images {
    original: Option<Rendition>,
    fixed_width: Option<Rendition>,
    fixed_width_still: Option<Rendition>,
}

#[derive(Deserialize)]
struct Item {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    images: Images,
}

#[derive(Deserialize)]
struct Pagination {
    #[serde(default)]
    total_count: u32,
    #[serde(default)]
    count: u32,
    #[serde(default)]
    offset: u32,
}

#[derive(Deserialize)]
struct Answer {
    data: Vec<Item>,
    pagination: Option<Pagination>,
}

fn https(u: Option<&String>) -> Option<String> {
    u.filter(|u| u.starts_with("https://")).cloned()
}

/// A GIF the picker can show: GIPHY's id is letters and digits, and it has a preview, a still and a size. GIPHY's
/// order is kept (its terms); one missing what it needs to draw is left out.
fn result(i: Item) -> Option<GifResult> {
    if i.id.is_empty() || !i.id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let fw = i.images.fixed_width?;
    let preview = https(fw.webp.as_ref()).or_else(|| https(fw.url.as_ref()))?;
    let still = https(i.images.fixed_width_still?.url.as_ref())?;
    let (width, height) = match i.images.original {
        Some(o) if o.width > 0 && o.height > 0 => (o.width, o.height),
        _ => (fw.width, fw.height),
    };
    if width == 0 || height == 0 || fw.width == 0 || fw.height == 0 {
        return None;
    }
    let url = https(Some(&i.url)).unwrap_or_else(|| format!("https://giphy.com/gifs/{}", i.id));
    Some(GifResult {
        id: i.id,
        title: i.title.trim().to_string(),
        url,
        width,
        height,
        preview,
        preview_width: fw.width,
        preview_height: fw.height,
        still,
    })
}

/// GIPHY's API.
pub struct Giphy {
    base: String,
    http: reqwest::Client,
}

impl Giphy {
    pub fn new(base: &str) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .expect("http client");
        Self {
            base: base.trim_end_matches('/').to_string(),
            http,
        }
    }

    /// One page of `query`'s results, or of trending when it is empty.
    async fn page(&self, key: &Key, query: &str, offset: u32) -> Result<GifPage, ApiError> {
        let query: String = query.trim().chars().take(MAX_QUERY_CHARS).collect();
        let (path, max) = if query.is_empty() {
            ("trending", TRENDING_MAX_OFFSET)
        } else {
            ("search", SEARCH_MAX_OFFSET)
        };
        let offset = offset.min(max);
        let (limit, offset_s) = (PAGE.to_string(), offset.to_string());
        let mut params: Vec<(&str, &str)> = vec![
            ("api_key", &key.api_key),
            ("limit", &limit),
            ("offset", &offset_s),
            ("rating", &key.rating),
        ];
        if !query.is_empty() {
            params.push(("q", &query));
        }
        let resp = self
            .http
            .get(format!("{}/{path}", self.base))
            .query(&params)
            .send()
            .await
            .map_err(|e| {
                // never the URL: it carries the key
                tracing::warn!(error = %e.without_url(), "GIPHY request failed");
                told(503, UNREACHABLE)
            })?;
        let status = resp.status().as_u16();
        match status {
            200..=299 => {}
            429 => return Err(told(429, BUSY)),
            401 | 403 => return Err(told(status, KEY_REFUSED)),
            _ => {
                tracing::warn!(status, "GIPHY answered with an error");
                return Err(told(503, UNREACHABLE));
            }
        }
        let a: Answer = resp.json().await.map_err(|e| {
            tracing::warn!(error = %e.without_url(), "GIPHY's answer didn't read");
            told(503, UNREACHABLE)
        })?;
        let got = a.data.len() as u32;
        let next = a.pagination.and_then(|p| {
            let n = p.offset.max(offset) + p.count.max(got);
            (p.count.max(got) > 0 && n < p.total_count && n <= max).then_some(n)
        });
        Ok(GifPage {
            gifs: a.data.into_iter().filter_map(result).collect(),
            next,
        })
    }
}

fn told(status: u16, message: &str) -> ApiError {
    ApiError::Told {
        status,
        message: message.into(),
    }
}

/// The server's GIPHY key and rating.
#[derive(Clone)]
struct Key {
    api_key: String,
    rating: String,
}

/// GIPHY, and the key per server for this run (asked for again after GIPHY refuses it, or when there was none).
pub struct Gifs {
    giphy: Giphy,
    key: Mutex<Option<(String, Key)>>,
}

impl Default for Gifs {
    fn default() -> Self {
        Self::new(GIPHY_API)
    }
}

impl Gifs {
    pub fn new(giphy_base: &str) -> Self {
        Self {
            giphy: Giphy::new(giphy_base),
            key: Mutex::new(None),
        }
    }

    /// The key of `api`'s server; `None` when GIFs aren't set up there.
    async fn key(&self, api: &Api, token: &str) -> Result<Option<Key>, ApiError> {
        if let Some((server, key)) = self.key.lock().unwrap().as_ref()
            && server == api.base()
        {
            return Ok(Some(key.clone()));
        }
        let c = api.gif_config(token).await?;
        let Some(api_key) = c.api_key else {
            return Ok(None);
        };
        let key = Key {
            api_key,
            rating: c.rating,
        };
        *self.key.lock().unwrap() = Some((api.base().to_string(), key.clone()));
        Ok(Some(key))
    }

    /// One page of GIFs for `query` (trending when it is blank) from `offset`; `None` when GIFs aren't set up on
    /// `api`'s server.
    pub async fn feed(
        &self,
        api: &Api,
        token: &str,
        query: &str,
        offset: u32,
    ) -> Result<Option<GifPage>, ApiError> {
        let Some(key) = self.key(api, token).await? else {
            return Ok(None);
        };
        let r = self.giphy.page(&key, query, offset).await;
        if let Err(ApiError::Told {
            status: 401 | 403, ..
        }) = &r
        {
            // the key may have changed on the server since: ask again next time
            *self.key.lock().unwrap() = None;
        }
        r.map(Some)
    }
}

/// A page of GIFs for the picker (`query` blank: trending); `None` when GIFs aren't set up on this server.
#[tauri::command]
pub async fn gif_feed(
    app: AppHandle,
    core: State<'_, Core>,
    gifs: State<'_, Gifs>,
    query: String,
    offset: Option<u32>,
) -> Result<Option<GifPage>, ApiError> {
    let (api, token) = core.current()?;
    core.check(
        &app,
        gifs.feed(&api, &token, &query, offset.unwrap_or(0)).await,
    )
}

/// Your favourite GIFs, newest first.
#[tauri::command]
pub async fn gif_favourites(app: AppHandle, core: State<'_, Core>) -> Result<Vec<Gif>, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.gif_favourites(&token).await)
}

/// Star (`on`) or unstar a GIF.
#[tauri::command]
pub async fn set_gif_favourite(
    app: AppHandle,
    core: State<'_, Core>,
    gif: Gif,
    on: bool,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    let r = if on {
        api.add_gif_favourite(&token, &gif).await
    } else {
        api.remove_gif_favourite(&token, &gif.id).await
    };
    core.check(&app, r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_server::config::GiphyConfig;
    use pulse_server::testing::{self, fake_giphy};

    async fn server(key: Option<&str>) -> (testing::TestApp, Api, String) {
        let mut cfg = testing::test_config();
        cfg.giphy = GiphyConfig {
            api_key: key.map(Into::into),
            rating: "pg-13".into(),
        };
        let app = testing::spawn_with(cfg).await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        (app, api, token)
    }

    fn query_of(req: &str) -> std::collections::HashMap<String, String> {
        let q = req.split_once('?').map(|(_, q)| q).unwrap_or("");
        url::form_urlencoded::parse(q.as_bytes())
            .into_owned()
            .collect()
    }

    #[tokio::test]
    async fn trending_comes_from_giphy_with_the_servers_key_and_rating() {
        let fake = fake_giphy::FakeGiphy::start().await;
        let (_app, api, token) = server(Some(fake_giphy::KEY)).await;
        let gifs = Gifs::new(&fake.base());
        let page = gifs.feed(&api, &token, "  ", 0).await.unwrap().unwrap();
        assert_eq!(
            page.gifs.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
            ["Fake0a", "Fake0b"],
            "GIPHY's order"
        );
        let g = &page.gifs[0];
        assert_eq!(g.title, "Trending GIF");
        assert_eq!(g.url, "https://giphy.com/gifs/fake-Fake0a");
        assert_eq!((g.width, g.height), (480, 270));
        assert_eq!((g.preview_width, g.preview_height), (200, 113));
        // GIPHY's URLs exactly as it gave them, query and all
        assert_eq!(
            g.preview,
            "https://media1.giphy.com/media/v1.fake/Fake0a/200w.webp?cid=fake&rid=200w.webp&ct=g"
        );
        assert_eq!(
            g.still,
            "https://media1.giphy.com/media/v1.fake/Fake0a/200w_s.gif?cid=fake&rid=200w_s.gif&ct=g"
        );
        assert_eq!(page.next, Some(2));
        let reqs = fake.requests();
        assert_eq!(reqs.len(), 1);
        assert!(reqs[0].starts_with("/v1/gifs/trending?"), "{}", reqs[0]);
        let q = query_of(&reqs[0]);
        assert_eq!(q["api_key"], fake_giphy::KEY);
        assert_eq!(q["rating"], "pg-13");
        assert_eq!(q["limit"], PAGE.to_string());
        assert_eq!(q["offset"], "0");
        assert!(!q.contains_key("q"));
    }

    #[tokio::test]
    async fn search_sends_the_query_trimmed_and_pages_on() {
        let fake = fake_giphy::FakeGiphy::start().await;
        let (_app, api, token) = server(Some(fake_giphy::KEY)).await;
        let gifs = Gifs::new(&fake.base());
        let page = gifs
            .feed(&api, &token, "  happy dance & cats ", 28)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(page.gifs[0].id, "Fake28a");
        assert_eq!(page.gifs[0].title, "happy dance & cats GIF");
        // 28 + 2 reaches the 30 there are
        assert_eq!(page.next, None);
        let reqs = fake.requests();
        assert!(reqs[0].starts_with("/v1/gifs/search?"), "{}", reqs[0]);
        let q = query_of(&reqs[0]);
        assert_eq!(q["q"], "happy dance & cats");
        assert_eq!(q["offset"], "28");
        // a long query is cut to GIPHY's 50 characters
        gifs.feed(&api, &token, &"x".repeat(80), 0).await.unwrap();
        assert_eq!(query_of(&fake.requests()[1])["q"], "x".repeat(50));
    }

    #[tokio::test]
    async fn the_key_is_asked_for_once() {
        let fake = fake_giphy::FakeGiphy::start().await;
        let mut cfg = testing::test_config();
        cfg.giphy = GiphyConfig {
            api_key: Some(fake_giphy::KEY.into()),
            rating: "g".into(),
        };
        // the server answers /gifs/config once, then 429s: a second ask would fail the search
        cfg.limits.gif_user = pulse_server::limits::Rate::new(1, 0.0001);
        let app = testing::spawn_with(cfg).await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let gifs = Gifs::new(&fake.base());
        for q in ["", "cats", "dogs"] {
            gifs.feed(&api, &token, q, 0).await.unwrap().unwrap();
        }
        assert_eq!(fake.requests().len(), 3);
        assert_eq!(query_of(&fake.requests()[2])["rating"], "g");
    }

    #[tokio::test]
    async fn without_a_key_gifs_arent_set_up_and_giphy_is_never_called() {
        let fake = fake_giphy::FakeGiphy::start().await;
        let (_app, api, token) = server(None).await;
        let gifs = Gifs::new(&fake.base());
        assert!(gifs.feed(&api, &token, "cats", 0).await.unwrap().is_none());
        assert!(fake.requests().is_empty());
    }

    #[tokio::test]
    async fn giphy_over_its_limit_says_busy_and_never_signs_you_out() {
        let fake = fake_giphy::FakeGiphy::start().await;
        let (_app, api, token) = server(Some(fake_giphy::KEY)).await;
        let gifs = Gifs::new(&fake.base());
        fake.set_status(429);
        let e = gifs.feed(&api, &token, "cats", 0).await.unwrap_err();
        assert!(
            matches!(&e, ApiError::Told { status: 429, message } if message == BUSY),
            "{e:?}"
        );
        fake.set_status(500);
        let e = gifs.feed(&api, &token, "cats", 0).await.unwrap_err();
        assert!(matches!(&e, ApiError::Told { message, .. } if message == UNREACHABLE));
        assert!(!matches!(e, ApiError::Unauthorized));
    }

    #[tokio::test]
    async fn a_refused_key_is_asked_for_again() {
        let fake = fake_giphy::FakeGiphy::start().await;
        let (_app, api, token) = server(Some("an-old-key")).await;
        let gifs = Gifs::new(&fake.base());
        let e = gifs.feed(&api, &token, "cats", 0).await.unwrap_err();
        assert!(
            matches!(&e, ApiError::Told { status: 401, message } if message == KEY_REFUSED),
            "a refused key is GIPHY's, not a sign-out: {e:?}"
        );
        assert!(gifs.key.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn giphy_unreachable_says_so() {
        let (_app, api, token) = server(Some(fake_giphy::KEY)).await;
        // nothing listens here
        let gifs = Gifs::new("http://127.0.0.1:9/v1/gifs");
        let e = gifs.feed(&api, &token, "cats", 0).await.unwrap_err();
        assert!(matches!(&e, ApiError::Told { message, .. } if message == UNREACHABLE));
    }

    #[test]
    fn a_gif_without_what_it_needs_is_left_out() {
        let item = |id: &str, images: serde_json::Value| -> Item {
            serde_json::from_value(
                serde_json::json!({ "id": id, "title": " t ", "images": images }),
            )
            .unwrap()
        };
        let good = serde_json::json!({
            "fixed_width": { "width": "200", "height": "100", "url": "https://media.giphy.com/media/a/200w.gif" },
            "fixed_width_still": { "width": "200", "height": "100", "url": "https://media.giphy.com/media/a/200w_s.gif" },
        });
        let r = result(item("abc", good.clone())).unwrap();
        // no WebP: the GIF; no original: the preview's size; no page: GIPHY's page for the id
        assert_eq!(r.preview, "https://media.giphy.com/media/a/200w.gif");
        assert_eq!((r.width, r.height), (200, 100));
        assert_eq!(r.url, "https://giphy.com/gifs/abc");
        assert_eq!(r.title, "t");
        assert!(result(item("a/b", good.clone())).is_none());
        assert!(result(item("", good)).is_none());
        assert!(
            result(item(
                "abc",
                serde_json::json!({ "fixed_width": { "width": "200", "height": "100", "url": "https://x/a.gif" } })
            ))
            .is_none(),
            "no still"
        );
        assert!(
            result(item(
                "abc",
                serde_json::json!({
                    "fixed_width": { "width": "200", "height": "100", "url": "http://media.giphy.com/a.gif" },
                    "fixed_width_still": { "url": "https://media.giphy.com/a_s.gif" },
                })
            ))
            .is_none(),
            "https only"
        );
    }

    #[tokio::test]
    async fn favourites_round_trip_through_the_server() {
        let (_app, api, token) = server(None).await;
        let g = Gif {
            id: "abc123".into(),
            title: "Yes".into(),
            width: 480,
            height: 270,
        };
        api.add_gif_favourite(&token, &g).await.unwrap();
        assert_eq!(api.gif_favourites(&token).await.unwrap(), vec![g.clone()]);
        api.remove_gif_favourite(&token, &g.id).await.unwrap();
        assert!(api.gif_favourites(&token).await.unwrap().is_empty());
    }
}
