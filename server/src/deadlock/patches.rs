//! Patch notes (#390): deadlock-api's `/v2/patches` (the forum changelog and Steam news in one feed), or Steam's own
//! news for Deadlock when that fails, checked about every half hour. Each update not seen before posts once in every
//! Deadlock channel that takes patch notes, with the start of the notes and the heroes they name. The first check only
//! remembers what's there.

use std::collections::HashSet;

use pulse_protocol::rest::{DeadlockCard, DeadlockHeroIcon, DeadlockPatchCard};

use super::api::Patch;
use super::assets::{self, Catalog};
use super::matches::{self, Kind};
use crate::AppState;
use crate::db::now;
use crate::error::AppResult;
use crate::messages::system;

/// Updates older than this when first seen are remembered, not posted.
pub const FRESH_FOR: jiff::SignedDuration = crate::times::days(14);
/// The most updates posted in one check.
pub const MAX_POSTS: usize = 3;
/// The start of the notes shown, in characters.
pub const SUMMARY_CHARS: usize = 280;
/// The most hero icons on a card.
pub const MAX_HEROES: usize = 8;

/// How an update is told from others: the `MM-DD-YYYY` date in its title when there is one (the forum and Steam
/// post the same update under different titles), else its link.
pub fn key_of(p: &Patch) -> String {
    let t = p.title.as_bytes();
    for i in 0..t.len().saturating_sub(9) {
        let w = &t[i..i + 10];
        let digit = |j: usize| w[j].is_ascii_digit();
        if digit(0)
            && digit(1)
            && w[2] == b'-'
            && digit(3)
            && digit(4)
            && w[5] == b'-'
            && (6..10).all(digit)
        {
            return format!("date:{}", String::from_utf8_lossy(w));
        }
    }
    format!("link:{}", p.link)
}

/// Only `https` links are shown.
pub fn safe_link(link: &str) -> Option<String> {
    let u = reqwest::Url::parse(link).ok()?;
    (u.scheme() == "https" && u.host_str().is_some()).then(|| u.to_string())
}

/// The heroes `text` names (whole words), in the order first named, at most [`MAX_HEROES`].
pub fn heroes_named(cat: &Catalog, text: &str) -> Vec<u32> {
    let mut found: Vec<(usize, u32)> = cat
        .heroes
        .iter()
        .filter_map(|(id, h)| {
            let mut from = 0;
            while let Some(i) = text[from..].find(&h.name) {
                let at = from + i;
                let end = at + h.name.len();
                let before = text[..at]
                    .chars()
                    .next_back()
                    .is_none_or(|c| !c.is_alphanumeric());
                let after = text[end..]
                    .chars()
                    .next()
                    .is_none_or(|c| !c.is_alphanumeric());
                if before && after {
                    return Some((at, *id));
                }
                from = end;
            }
            None
        })
        .collect();
    found.sort();
    found
        .into_iter()
        .take(MAX_HEROES)
        .map(|(_, id)| id)
        .collect()
}

/// The feed: deadlock-api's, else Steam's news.
async fn feed(s: &AppState) -> Option<Vec<Patch>> {
    match s.deadlock.api().patches().await {
        Ok(p) => Some(p),
        Err(_) => s.deadlock.api().steam_news(&s.cfg.steam.api_base).await,
    }
}

/// One check.
pub async fn tick(s: &AppState) -> AppResult<()> {
    let Some(mut items) = feed(s).await else {
        return Ok(());
    };
    // oldest first: of the same update posted twice (forum and Steam), the first one counts
    items.sort_by(|a, b| a.published_at.cmp(&b.published_at));
    let seen: HashSet<String> = sqlx::query_scalar::<_, String>("SELECT key FROM deadlock_patches")
        .fetch_all(&s.db)
        .await?
        .into_iter()
        .collect();
    let first = seen.is_empty();
    let mut fresh: Vec<(String, Patch)> = Vec::new();
    let mut keys = seen;
    for p in items {
        let k = key_of(&p);
        if !keys.insert(k.clone()) {
            continue;
        }
        fresh.push((k, p));
    }
    let cutoff = jiff::Timestamp::now() - FRESH_FOR;
    let mut posted = 0;
    let channels = if first {
        vec![]
    } else {
        matches::channels(s, Kind::Patches, None).await?
    };
    let cat = if channels.is_empty() {
        None
    } else {
        assets::catalog(s).await
    };
    for (key, p) in fresh {
        sqlx::query("INSERT OR IGNORE INTO deadlock_patches (key, title, published_at, seen_at) VALUES (?, ?, ?, ?)")
            .bind(&key)
            .bind(crate::steam::one_line(&p.title, 200).unwrap_or_default())
            .bind(&p.published_at)
            .bind(now())
            .execute(&s.db)
            .await?;
        let recent = crate::times::parse(&p.published_at).is_ok_and(|t| t >= cutoff);
        if first || !recent || posted >= MAX_POSTS || channels.is_empty() {
            continue;
        }
        let Some(link) = safe_link(&p.link) else {
            continue;
        };
        posted += 1;
        post(s, &channels, &p, link, cat.as_deref()).await?;
    }
    Ok(())
}

async fn post(
    s: &AppState,
    channels: &[(String, pulse_protocol::ids::ChannelId)],
    p: &Patch,
    link: String,
    cat: Option<&Catalog>,
) -> AppResult<()> {
    let text = super::plain_text(&p.content);
    let title = crate::steam::one_line(&p.title, 200).unwrap_or_else(|| "Deadlock update".into());
    // hero icons are fetched on behalf of the server's owner of the first channel (anyone works; it's for the
    // media row's uploader)
    let by: Option<String> = match channels.first() {
        Some((server, _)) => {
            sqlx::query_scalar("SELECT created_by FROM servers WHERE id = ?")
                .bind(server)
                .fetch_optional(&s.db)
                .await?
        }
        None => None,
    };
    let mut heroes = Vec::new();
    if let (Some(cat), Some(by)) = (cat, by.and_then(|b| b.parse().ok())) {
        for id in heroes_named(cat, &format!("{title} {text}")) {
            heroes.push(DeadlockHeroIcon {
                name: cat.hero_name(id),
                icon: assets::hero_art(s, by, Some(cat), id, false).await,
            });
        }
    }
    let card = DeadlockCard::Patch(DeadlockPatchCard {
        title: title.clone(),
        link: link.clone(),
        published_at: p.published_at.clone(),
        summary: super::excerpt(&text, SUMMARY_CHARS),
        heroes,
    });
    let payload = system::deadlock_payload(&card)?;
    for (_, ch) in channels {
        if let Err(e) = system::post(
            s,
            *ch,
            format!("📜 Deadlock update: {title} {link}"),
            payload.clone(),
        )
        .await
        {
            tracing::warn!(error = ?e, "Deadlock patch notes not posted");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deadlock::assets::Hero;

    fn patch(title: &str, link: &str) -> Patch {
        Patch {
            title: title.into(),
            link: link.into(),
            published_at: "2026-11-05T05:02:30.000Z".into(),
            content: String::new(),
        }
    }

    #[test]
    fn the_same_update_from_the_forum_and_steam_is_one() {
        assert_eq!(
            key_of(&patch("11-04-2026", "https://a.test/1")),
            "date:11-04-2026"
        );
        assert_eq!(
            key_of(&patch(" Minor Update - 11-04-2026", "https://b.test/2")),
            "date:11-04-2026"
        );
        assert_eq!(
            key_of(&patch("Halloween Update", "https://b.test/3")),
            "link:https://b.test/3"
        );
        assert_eq!(
            key_of(&patch("1-2", "https://b.test/4")),
            "link:https://b.test/4"
        );
    }

    #[test]
    fn only_https_links() {
        assert!(safe_link("https://store.example/news/1").is_some());
        assert!(safe_link("http://store.example/news/1").is_none());
        assert!(safe_link("javascript:alert(1)").is_none());
    }

    #[test]
    fn heroes_named_in_order_and_whole_words() {
        let mut cat = Catalog::default();
        for (id, name) in [
            (1, "Infernus"),
            (13, "Haze"),
            (7, "Wraith"),
            (18, "Mo & Krill"),
        ] {
            cat.heroes.insert(
                id,
                Hero {
                    name: name.into(),
                    ..Hero::default()
                },
            );
        }
        let ids = heroes_named(
            &cat,
            "Hazel note. Wraith: faster. Haze: slower. Mo & Krill: burrow. Infernuses",
        );
        assert_eq!(ids, vec![7, 13, 18]);
    }
}
