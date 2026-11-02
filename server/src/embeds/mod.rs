//! Link embeds: previews of the links in a message (migration 0012), stored per message and link
//! and sent out in the message's `embeds`.

pub mod fetch;
pub mod links;
pub mod meta;
pub mod providers;
pub mod routes;
pub mod unfurl;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
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

/// How long a preview is reused for the same link...
pub const CACHE_TTL: Duration = Duration::from_secs(24 * 3600);
/// ...and a link that gave none.
pub const CACHE_FAILURE_TTL: Duration = Duration::from_secs(3600);
/// Most links remembered; the oldest goes first.
pub const CACHE_MAX: usize = 1000;
/// Most links unfurled at once, server-wide.
pub const MAX_CONCURRENT: usize = 4;

/// Builds link previews: the guarded fetcher, the oEmbed endpoints, a cache by normalised URL
/// and a cap on how many run at once. One per server (`AppState::embeds`).
pub struct Unfurler {
    fetcher: fetch::Fetcher,
    endpoints: Endpoints,
    cache: Mutex<HashMap<String, Cached>>,
    slots: Semaphore,
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
            slots: Semaphore::new(MAX_CONCURRENT),
        }
    }

    /// `url`'s preview, from the cache when it's fresh there, else built (thumbnail stored as
    /// media uploaded `by` the sender). `None` for no preview. A cached thumbnail may have been
    /// cleaned up since: [`store`] checks before keeping it.
    pub async fn unfurl(
        &self,
        db: &SqlitePool,
        media_dir: &Path,
        by: UserId,
        url: &str,
    ) -> Option<Unfurled> {
        let key = links::normalise(url)?;
        if let Some(hit) = self.cached(&key) {
            return hit;
        }
        let value = {
            // Never closed, so this only waits.
            let _slot = self.slots.acquire().await.ok()?;
            unfurl::run(&self.fetcher, &self.endpoints, db, media_dir, by, &key).await
        };
        self.remember(key, value.clone());
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

pub fn kind_str(k: EmbedKind) -> &'static str {
    match k {
        EmbedKind::Card => "card",
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

/// After a message is sent or edited (and committed, and its own event published): fetch
/// previews for its links in the background, then store them and publish `MessageUpdated`.
/// `content` is the message's content as committed. Nothing happens when it has no links, when
/// `author` is over the unfurl limit (the message stays as it is, without previews), or when
/// the disk is too full for thumbnails.
pub fn spawn_unfurl(s: &AppState, message_id: MessageId, author: UserId, content: String) {
    let links = links::extract(&content);
    if links.is_empty() {
        return;
    }
    if !s.limits.unfurl_user.hit(&author.to_string()) {
        tracing::debug!(%message_id, "over the unfurl limit; no previews");
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
    let unfurled = futures::future::join_all(todo.into_iter().map(|(position, url)| async move {
        let got = s.embeds.unfurl(&s.db, &s.cfg.media_dir, author, &url).await;
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
    }
}
