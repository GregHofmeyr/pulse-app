//! A Steam game's art for "Playing" (#380): `header.jpg`, `library_hero.jpg` and `logo.png` from Steam's public store
//! CDN by app id ([`crate::config::Endpoints::steam_cdn`]), each fetched through the guarded [`super::fetch::Fetcher`]
//! and re-encoded and stored like a link thumbnail. Remembered by app id, server-wide, like link previews: a game
//! many people play is fetched once a day. Clients never reach Valve; they get media hashes.

use std::path::Path;
use std::time::Instant;

use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{EmbedThumb, GameArt};
use reqwest::Url;
use sqlx::SqlitePool;

use super::{CACHE_FAILURE_TTL, CACHE_MAX, CACHE_TTL, Fetching, Unfurler, keep_thumb, unfurl};
use crate::limits::Limiter;

/// One game's art as remembered: `None` when Steam had none of it.
pub(super) struct CachedArt {
    at: Instant,
    art: Option<GameArt>,
}

impl CachedArt {
    fn fresh(&self, now: Instant) -> bool {
        let ttl = if self.art.is_some() {
            CACHE_TTL
        } else {
            CACHE_FAILURE_TTL
        };
        now.duration_since(self.at) < ttl
    }
}

/// The three pieces' URLs under `base`, as (hero, header, logo).
pub fn art_urls(base: &str, app_id: u32) -> [String; 3] {
    let base = base.trim_end_matches('/');
    ["library_hero.jpg", "header.jpg", "logo.png"].map(|f| format!("{base}/{app_id}/{f}"))
}

/// Whether `app_id` can be a Steam app: not 0 (nothing runs), and below 2^31 (a non-Steam shortcut's id has the
/// top bit set and nothing on the CDN).
pub fn valid_app_id(app_id: u32) -> bool {
    app_id != 0 && app_id < 1 << 31
}

/// Keep every piece of `art` (restart their cleanup clocks); `None` if any was cleaned up already (fetch it again).
async fn kept(db: &SqlitePool, art: &GameArt) -> Option<GameArt> {
    async fn one(db: &SqlitePool, t: &Option<EmbedThumb>) -> Result<Option<EmbedThumb>, ()> {
        match t {
            None => Ok(None),
            Some(t) => match keep_thumb(db, t.clone()).await {
                Ok(Some(t)) => Ok(Some(t)),
                _ => Err(()),
            },
        }
    }
    Some(GameArt {
        hero: one(db, &art.hero).await.ok()?,
        header: one(db, &art.header).await.ok()?,
        logo: one(db, &art.logo).await.ok()?,
    })
}

impl Unfurler {
    /// Steam app `app_id`'s art, from the cache when fresh (and still stored), else fetched (one hit of `budget` for
    /// `by`; refused, there's none, and nothing is remembered). `None` when Steam has none of it.
    pub async fn steam_art(
        &self,
        db: &SqlitePool,
        media_dir: &Path,
        by: UserId,
        budget: &Limiter,
        app_id: u32,
    ) -> Option<GameArt> {
        if !valid_app_id(app_id) {
            return None;
        }
        if let Some(hit) = self.steam_cached(db, app_id).await {
            return hit;
        }
        // one fetch per game at a time: whoever comes second finds the first one's art in the cache
        let _fetching = Fetching::enter(self, format!("steam:{app_id}")).await;
        if let Some(hit) = self.steam_cached(db, app_id).await {
            return hit;
        }
        if !budget.hit(&by.to_string()) {
            return None;
        }
        let [hero, header, logo] = art_urls(&self.endpoints.steam_cdn, app_id);
        let one = |u: String| async move {
            let url = Url::parse(&u).ok()?;
            unfurl::thumb(&self.fetcher, db, media_dir, by, &url).await
        };
        let run = async { tokio::join!(one(hero), one(header), one(logo)) };
        let (hero, header, logo) = tokio::time::timeout(self.deadline, run)
            .await
            .unwrap_or_else(|_| {
                tracing::debug!("game art ran past its deadline");
                (None, None, None)
            });
        let art = (hero.is_some() || header.is_some() || logo.is_some()).then_some(GameArt {
            hero,
            header,
            logo,
        });
        self.steam_remember(app_id, art.clone());
        art
    }

    /// The picture at `url` (an achievement's icon, #379, from Steam's own answer), fetched through the guarded fetcher
    /// within the link deadline, re-encoded and stored like a link thumbnail. `None` if any of that fails.
    pub async fn remote_thumb(
        &self,
        db: &SqlitePool,
        media_dir: &Path,
        by: UserId,
        url: &str,
    ) -> Option<EmbedThumb> {
        let url = Url::parse(url).ok()?;
        tokio::time::timeout(
            self.deadline,
            unfurl::thumb(&self.fetcher, db, media_dir, by, &url),
        )
        .await
        .ok()
        .flatten()
    }

    /// `Some(art)` when `app_id` is cached, fresh, and its files are all still there.
    async fn steam_cached(&self, db: &SqlitePool, app_id: u32) -> Option<Option<GameArt>> {
        let hit = {
            let cache = self.steam.lock().unwrap_or_else(|e| e.into_inner());
            cache
                .get(&app_id)
                .filter(|c| c.fresh(Instant::now()))
                .map(|c| c.art.clone())?
        };
        match hit {
            None => Some(None),
            Some(art) => kept(db, &art).await.map(Some),
        }
    }

    fn steam_remember(&self, app_id: u32, art: Option<GameArt>) {
        let mut cache = self.steam.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if cache.len() >= CACHE_MAX && !cache.contains_key(&app_id) {
            cache.retain(|_, c| c.fresh(now));
            if cache.len() >= CACHE_MAX
                && let Some(oldest) = cache.iter().min_by_key(|(_, c)| c.at).map(|(k, _)| *k)
            {
                cache.remove(&oldest);
            }
        }
        cache.insert(app_id, CachedArt { at: now, art });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_by_app_id() {
        assert_eq!(
            art_urls("https://cdn.example/steam/apps/", 570),
            [
                "https://cdn.example/steam/apps/570/library_hero.jpg",
                "https://cdn.example/steam/apps/570/header.jpg",
                "https://cdn.example/steam/apps/570/logo.png",
            ]
        );
    }

    #[test]
    fn only_real_steam_ids() {
        assert!(valid_app_id(570));
        assert!(!valid_app_id(0));
        assert!(!valid_app_id(0x8000_0000), "a non-Steam shortcut");
        assert!(!valid_app_id(u32::MAX));
    }
}
