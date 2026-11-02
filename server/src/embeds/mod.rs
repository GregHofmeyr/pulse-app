//! Link embeds: previews of the links in a message (migration 0012), stored per message and link
//! and sent out in the message's `embeds`.

pub mod fetch;
pub mod links;
pub mod meta;
pub mod page;
pub mod providers;
pub mod routes;
pub mod unfurl;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{MessageId, UserId};
use pulse_protocol::rest::{EmbedKind, EmbedProvider, EmbedThumb, LinkEmbed, Message};
use sqlx::{SqliteConnection, SqlitePool};
use tokio::sync::Semaphore;

pub use unfurl::Unfurled;

use crate::AppState;
use crate::config::{EmbedConfig, Endpoints};
use crate::error::AppResult;
use crate::limits::Limiter;

/// How long a preview is reused for the same link...
pub const CACHE_TTL: Duration = Duration::from_secs(24 * 3600);
/// ...and a link that gave none.
pub const CACHE_FAILURE_TTL: Duration = Duration::from_secs(3600);
/// Most links remembered; the oldest goes first.
pub const CACHE_MAX: usize = 1000;

/// Builds link previews: the guarded fetcher, the oEmbed endpoints, a cache by normalised URL,
/// and the bounds on the work (see [`EmbedConfig`]): how many links run at once and how many
/// may wait for that, a deadline per link, and how many messages one user has in flight. One
/// per server (`AppState::embeds`).
pub struct Unfurler {
    fetcher: fetch::Fetcher,
    endpoints: Endpoints,
    cache: Mutex<HashMap<String, Cached>>,
    slots: Semaphore,
    /// Links waiting for a slot now.
    waiting: AtomicUsize,
    max_waiting: usize,
    deadline: Duration,
    /// Messages each user has previews being fetched for.
    in_flight: Mutex<HashMap<UserId, usize>>,
    per_user_in_flight: usize,
}

/// A message's place among its sender's in-flight unfurls, given back on drop.
pub struct InFlight<'a> {
    unfurler: &'a Unfurler,
    user: UserId,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        let mut map = self
            .unfurler
            .in_flight
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(n) = map.get_mut(&self.user) {
            *n -= 1;
            if *n == 0 {
                map.remove(&self.user);
            }
        }
    }
}

/// Counts a link as waiting for a slot while it lives.
struct Waiting<'a>(&'a AtomicUsize);

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Cached {
    at: Instant,
    /// `None`: the link gave no preview.
    value: Option<Unfurled>,
}

impl Cached {
    fn fresh(&self, now: Instant) -> bool {
        let ttl = if self.value.is_some() {
            CACHE_TTL
        } else {
            CACHE_FAILURE_TTL
        };
        now.duration_since(self.at) < ttl
    }
}

impl Unfurler {
    pub fn new(cfg: &EmbedConfig) -> Self {
        Unfurler {
            fetcher: fetch::Fetcher::new(cfg),
            endpoints: cfg.endpoints.clone(),
            cache: Mutex::new(HashMap::new()),
            slots: Semaphore::new(cfg.max_concurrent),
            waiting: AtomicUsize::new(0),
            max_waiting: cfg.max_waiting,
            deadline: cfg.link_deadline,
            in_flight: Mutex::new(HashMap::new()),
            per_user_in_flight: cfg.per_user_in_flight,
        }
    }

    /// A place for one more of `user`'s messages among the unfurls in flight, `None` when they
    /// already have as many as they may.
    pub fn claim(&self, user: UserId) -> Option<InFlight<'_>> {
        let mut map = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        let n = map.entry(user).or_insert(0);
        if *n >= self.per_user_in_flight {
            return None;
        }
        *n += 1;
        Some(InFlight {
            unfurler: self,
            user,
        })
    }

    /// `url`'s preview, from the cache when it's fresh there, else built (thumbnail stored as
    /// media uploaded `by` the sender, if `thumbs` lets them have one more). `None` for no
    /// preview, also when too many links are already waiting for a slot. A cached thumbnail may
    /// have been cleaned up since: [`store`] checks before keeping it.
    pub async fn unfurl(
        &self,
        db: &SqlitePool,
        media_dir: &Path,
        by: UserId,
        thumbs: &Limiter,
        url: &str,
    ) -> Option<Unfurled> {
        let key = links::normalise(url)?;
        if let Some(hit) = self.cached(&key) {
            return hit;
        }
        let _slot = match self.slots.try_acquire() {
            Ok(slot) => slot,
            Err(_) => {
                let _waiting = Waiting(&self.waiting);
                if self.waiting.fetch_add(1, Ordering::SeqCst) >= self.max_waiting {
                    tracing::debug!("too many link previews waiting; this one gets none");
                    return None;
                }
                // Never closed, so this only waits.
                self.slots.acquire().await.ok()?
            }
        };
        // A preview made without its thumbnail only because of the limit isn't remembered, so
        // the next sender of the link can have it whole.
        let thumb_refused = AtomicBool::new(false);
        let thumb_ok = || {
            let ok = thumbs.hit(&by.to_string());
            if !ok {
                thumb_refused.store(true, Ordering::Relaxed);
            }
            ok
        };
        let run = unfurl::run(
            &self.fetcher,
            &self.endpoints,
            db,
            media_dir,
            by,
            &thumb_ok,
            &key,
        );
        // One deadline over all of the link's fetches; past it, the link gave no preview.
        let value = tokio::time::timeout(self.deadline, run)
            .await
            .unwrap_or_else(|_| {
                tracing::debug!("a link preview ran past its deadline");
                None
            });
        if !thumb_refused.load(Ordering::Relaxed) {
            self.remember(key, value.clone());
        }
        value
    }

    /// `Some(value)` when `key` is cached and fresh.
    fn cached(&self, key: &str) -> Option<Option<Unfurled>> {
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .get(key)
            .filter(|c| c.fresh(Instant::now()))
            .map(|c| c.value.clone())
    }

    fn remember(&self, key: String, value: Option<Unfurled>) {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if cache.len() >= CACHE_MAX && !cache.contains_key(&key) {
            cache.retain(|_, c| c.fresh(now));
            if cache.len() >= CACHE_MAX
                && let Some(oldest) = cache
                    .iter()
                    .min_by_key(|(_, c)| c.at)
                    .map(|(k, _)| k.clone())
            {
                cache.remove(&oldest);
            }
        }
        cache.insert(key, Cached { at: now, value });
    }
}

/// Write a message's previews: `rows` are (position among its links, the link, its preview).
/// Nothing is written, and `false` returned, unless the message is still live with
/// `content_when_read` (an edit or delete during the fetch wins). A thumbnail whose media row is
/// gone (cleaned up while cached) is dropped; one that's there has its cleanup clock restarted in
/// the same transaction, so it can't go before the row referencing it is in. A row already there
/// for the link is updated, keeping its `hidden`.
pub async fn store(
    db: &SqlitePool,
    message_id: MessageId,
    content_when_read: &str,
    rows: Vec<(usize, String, Unfurled)>,
) -> AppResult<bool> {
    let mut tx = crate::db::begin_write(db).await?;
    let content: Option<String> =
        sqlx::query_scalar("SELECT content FROM messages WHERE id = ? AND deleted_at IS NULL")
            .bind(message_id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    if content.as_deref() != Some(content_when_read) {
        return Ok(false);
    }
    let now = crate::db::now();
    for (position, url, mut e) in rows {
        if let Some(t) = &e.thumb {
            let touched = sqlx::query("UPDATE media SET created_at = ? WHERE hash = ?")
                .bind(&now)
                .bind(&t.hash)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            if touched == 0 {
                e.thumb = None;
            }
        }
        let thumb = e.thumb.as_ref();
        sqlx::query(
            "INSERT INTO link_embeds (message_id, position, url, provider, provider_id, kind,
                                      title, description, site_name, author, thumb_hash,
                                      thumb_width, thumb_height, start_secs, vertical, fetched_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (message_id, url) DO UPDATE SET
                position = excluded.position, provider = excluded.provider,
                provider_id = excluded.provider_id, kind = excluded.kind, title = excluded.title,
                description = excluded.description, site_name = excluded.site_name,
                author = excluded.author, thumb_hash = excluded.thumb_hash,
                thumb_width = excluded.thumb_width, thumb_height = excluded.thumb_height,
                start_secs = excluded.start_secs, vertical = excluded.vertical,
                fetched_at = excluded.fetched_at",
        )
        .bind(message_id.to_string())
        .bind(position as i64)
        .bind(&url)
        .bind(provider_str(e.provider))
        .bind(&e.provider_id)
        .bind(kind_str(e.kind))
        .bind(&e.title)
        .bind(&e.description)
        .bind(&e.site_name)
        .bind(&e.author)
        .bind(thumb.map(|t| &t.hash))
        .bind(thumb.map(|t| t.width as i64))
        .bind(thumb.map(|t| t.height as i64))
        .bind(e.start.map(i64::from))
        .bind(e.vertical)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// An `EmbedProvider` as stored (its wire name). `Unknown` is never stored.
pub fn provider_str(p: EmbedProvider) -> &'static str {
    match p {
        EmbedProvider::Youtube => "youtube",
        EmbedProvider::Tiktok => "tiktok",
        EmbedProvider::Spotify => "spotify",
        EmbedProvider::Soundcloud => "soundcloud",
        EmbedProvider::Twitch => "twitch",
        EmbedProvider::Instagram => "instagram",
        EmbedProvider::Generic => "generic",
        EmbedProvider::Unknown => "unknown",
    }
}

/// A stored provider; a name this build doesn't know reads as `Unknown`.
pub fn parse_provider(s: &str) -> EmbedProvider {
    match s {
        "youtube" => EmbedProvider::Youtube,
        "tiktok" => EmbedProvider::Tiktok,
        "spotify" => EmbedProvider::Spotify,
        "soundcloud" => EmbedProvider::Soundcloud,
        "twitch" => EmbedProvider::Twitch,
        "instagram" => EmbedProvider::Instagram,
        "generic" => EmbedProvider::Generic,
        _ => EmbedProvider::Unknown,
    }
}

/// An `EmbedKind` as stored. The unfurler never builds `Unknown`; it would be stored as a card.
pub fn kind_str(k: EmbedKind) -> &'static str {
    match k {
        EmbedKind::Card | EmbedKind::Unknown => "card",
        EmbedKind::Player => "player",
    }
}

pub fn parse_kind(s: &str) -> EmbedKind {
    match s {
        "player" => EmbedKind::Player,
        _ => EmbedKind::Card,
    }
}

type EmbedRow = (
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    bool,
);

/// Fill `embeds` on every live message in `msgs`: its rows that aren't hidden, in link order.
pub async fn attach(db: &SqlitePool, msgs: &mut [Message]) -> AppResult<()> {
    let ids: Vec<String> = msgs
        .iter()
        .filter(|m| !m.deleted)
        .map(|m| m.id.to_string())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let mut q = sqlx::QueryBuilder::new(
        "SELECT message_id, url, provider, provider_id, kind, title, description, site_name,
                author, thumb_hash, thumb_width, thumb_height, start_secs, vertical
         FROM link_embeds WHERE hidden = 0 AND message_id IN (",
    );
    let mut list = q.separated(", ");
    for id in &ids {
        list.push_bind(id);
    }
    q.push(") ORDER BY message_id, position");
    let rows: Vec<EmbedRow> = q.build_query_as().fetch_all(db).await?;
    let mut by_msg: HashMap<String, Vec<LinkEmbed>> = HashMap::new();
    for (
        message_id,
        url,
        provider,
        provider_id,
        kind,
        title,
        description,
        site_name,
        author,
        thumb_hash,
        thumb_width,
        thumb_height,
        start_secs,
        vertical,
    ) in rows
    {
        // A thumbnail needs all three; a row written without its size has none.
        let thumb = match (thumb_hash, thumb_width, thumb_height) {
            (Some(hash), Some(w), Some(h)) => Some(EmbedThumb {
                hash,
                width: w as u32,
                height: h as u32,
            }),
            _ => None,
        };
        by_msg.entry(message_id).or_default().push(LinkEmbed {
            url,
            provider: parse_provider(&provider),
            kind: parse_kind(&kind),
            provider_id,
            title,
            description,
            site_name,
            author,
            thumb,
            start: start_secs.map(|s| s as u32),
            vertical,
        });
    }
    for m in msgs.iter_mut().filter(|m| !m.deleted) {
        m.embeds = by_msg.remove(&m.id.to_string()).unwrap_or_default();
    }
    Ok(())
}

/// Keep only the previews whose link is still in `content` (after an edit, when the stored ones
/// couldn't be read back: better fewer previews than ones for links no longer there).
pub fn keep_linked(embeds: &mut Vec<LinkEmbed>, content: &str) {
    let links = links::extract(content);
    embeds.retain(|e| links.contains(&e.url));
}

/// After a message is sent or edited (and committed, and its own event published): fetch
/// previews for its links in the background, then store them and publish `MessageUpdated`.
/// `content` is the message's content as committed. Nothing happens when it has no links (or
/// none without a preview row already), when `author` already has as many messages' previews
/// in flight as they may or is over the unfurl limit (the message stays as it is, without
/// previews), or when the disk is too full for thumbnails.
pub fn spawn_unfurl(s: &AppState, message_id: MessageId, author: UserId, content: String) {
    let links = links::extract(&content);
    if links.is_empty() {
        return;
    }
    if let Err(e) = crate::media::writes_ok(&s.cfg) {
        tracing::warn!(error = ?e, %message_id, "low on disk; no previews");
        return;
    }
    let s = s.clone();
    tokio::spawn(async move {
        if let Err(e) = unfurl_message(&s, message_id, author, &content, links).await {
            tracing::warn!(error = ?e, %message_id, "couldn't add link previews");
        }
    });
}

async fn unfurl_message(
    s: &AppState,
    message_id: MessageId,
    author: UserId,
    content: &str,
    links: Vec<String>,
) -> AppResult<()> {
    // A link that already has a row keeps it: it was fetched before, or hidden (and stays so).
    let have: Vec<String> = sqlx::query_scalar("SELECT url FROM link_embeds WHERE message_id = ?")
        .bind(message_id.to_string())
        .fetch_all(&s.db)
        .await?;
    let todo: Vec<(usize, String)> = links
        .into_iter()
        .enumerate()
        .filter(|(_, url)| !have.contains(url))
        .collect();
    if todo.is_empty() {
        return Ok(());
    }
    // Only now that there's something to fetch: an edit that keeps its links costs nothing.
    let Some(_in_flight) = s.embeds.claim(author) else {
        tracing::debug!(%message_id, "too many unfurls in flight for the sender; no previews");
        return Ok(());
    };
    if !s.limits.unfurl_user.hit(&author.to_string()) {
        tracing::debug!(%message_id, "over the unfurl limit; no previews");
        return Ok(());
    }
    let thumbs = &s.limits.link_thumb_user;
    let unfurled = futures::future::join_all(todo.into_iter().map(|(position, url)| async move {
        let got = s
            .embeds
            .unfurl(&s.db, &s.cfg.media_dir, author, thumbs, &url)
            .await;
        got.map(|e| (position, url, e))
    }))
    .await;
    let rows: Vec<_> = unfurled.into_iter().flatten().collect();
    if rows.is_empty() || !store(&s.db, message_id, content, rows).await? {
        return Ok(());
    }
    // Loaded and published in order with edits and hides, so an older copy never goes out last.
    let _order = s.hub.in_order().await;
    if let Some(message) = crate::messages::routes::load(&s.db, None, message_id).await?
        && !message.deleted
    {
        s.hub
            .publish(&s.db, Event::MessageUpdated { message })
            .await;
    }
    Ok(())
}

/// Part of an edit to `content` (the UPDATE is the caller's, in `tx`): previews of links no
/// longer in it go, unless hidden (hidden is kept per link, so it holds if the link comes back);
/// the ones that stay take their new place among its links.
pub async fn on_edit(tx: &mut SqliteConnection, id: MessageId, content: &str) -> AppResult<()> {
    let keep = links::extract(content);
    let rows: Vec<(String, bool)> =
        sqlx::query_as("SELECT url, hidden FROM link_embeds WHERE message_id = ?")
            .bind(id.to_string())
            .fetch_all(&mut *tx)
            .await?;
    for (url, hidden) in rows {
        match keep.iter().position(|k| *k == url) {
            Some(position) => {
                sqlx::query("UPDATE link_embeds SET position = ? WHERE message_id = ? AND url = ?")
                    .bind(position as i64)
                    .bind(id.to_string())
                    .bind(&url)
                    .execute(&mut *tx)
                    .await?;
            }
            None if !hidden => {
                sqlx::query("DELETE FROM link_embeds WHERE message_id = ? AND url = ?")
                    .bind(id.to_string())
                    .bind(&url)
                    .execute(&mut *tx)
                    .await?;
            }
            None => {}
        }
    }
    Ok(())
}

/// Part of deleting a message (the soft delete is the caller's): its previews go, so the media
/// cleanup can drop their thumbnails.
pub async fn on_delete(tx: &mut SqliteConnection, id: MessageId) -> AppResult<()> {
    sqlx::query("DELETE FROM link_embeds WHERE message_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for p in [
            EmbedProvider::Youtube,
            EmbedProvider::Tiktok,
            EmbedProvider::Spotify,
            EmbedProvider::Soundcloud,
            EmbedProvider::Twitch,
            EmbedProvider::Instagram,
            EmbedProvider::Generic,
        ] {
            assert_eq!(parse_provider(provider_str(p)), p);
            // The stored name is the wire name.
            assert_eq!(
                serde_json::to_value(p).unwrap(),
                serde_json::json!(provider_str(p))
            );
        }
        assert_eq!(parse_provider("vimeo"), EmbedProvider::Unknown);
        for k in [EmbedKind::Card, EmbedKind::Player] {
            assert_eq!(parse_kind(kind_str(k)), k);
        }
        assert_eq!(parse_kind(kind_str(EmbedKind::Unknown)), EmbedKind::Card);
    }

    fn card(url: &str) -> LinkEmbed {
        LinkEmbed {
            url: url.into(),
            provider: EmbedProvider::Generic,
            kind: EmbedKind::Card,
            provider_id: None,
            title: None,
            description: None,
            site_name: None,
            author: None,
            thumb: None,
            start: None,
            vertical: false,
        }
    }

    /// The fallback when an edit's previews can't be read back: the ones it had, less those
    /// whose link the edit took out.
    #[test]
    fn keep_linked_drops_previews_of_links_edited_away() {
        let (a, b) = ("https://a.example.com/x", "https://b.example.com/y");
        let mut embeds = vec![card(a), card(b)];
        keep_linked(&mut embeds, "now only https://B.example.com/y#top");
        assert_eq!(embeds, vec![card(b)]);
        keep_linked(&mut embeds, "no links");
        assert!(embeds.is_empty());
    }
}
