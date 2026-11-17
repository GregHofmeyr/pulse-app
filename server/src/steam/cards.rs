//! Achievement cards (#379): a person's new unlocks in a game, and their 100 % of it, as system lines in the
//! Achievements channel of every server they're in that has one. The game's schema and global rarity are cached per
//! app for a day; icons and art are fetched by the server (clients never reach Valve) and kept from the media sweep.

use std::collections::HashMap;

use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{AchievementCard, AchievementEntry, EmbedThumb};

use super::api::Schema;
use crate::AppState;
use crate::db::now;
use crate::error::AppResult;
use crate::messages::system;

/// Unlocks shown on one card; the rest are "and N more".
pub const SHOWN: usize = 5;
/// How long a game's schema and rarity are trusted.
pub const APP_CACHE_TTL: chrono::Duration = chrono::Duration::hours(24);

/// A game's name, achievements and global rarity: from the cache when it's under a day old, else from Steam (two
/// calls), else whatever the cache has, however old.
pub async fn app_info(s: &AppState, app: u32) -> (Schema, HashMap<String, f32>) {
    let cached: Option<(String, String, String)> =
        sqlx::query_as("SELECT schema, rarity, fetched_at FROM steam_app_cache WHERE app_id = ?")
            .bind(app)
            .fetch_optional(&s.db)
            .await
            .ok()
            .flatten();
    let parse = |(schema, rarity, _): &(String, String, String)| {
        Some((
            serde_json::from_str::<Schema>(schema).ok()?,
            serde_json::from_str::<HashMap<String, f32>>(rarity).ok()?,
        ))
    };
    let fresh = cached.as_ref().is_some_and(|(_, _, at)| {
        chrono::DateTime::parse_from_rfc3339(at)
            .is_ok_and(|t| chrono::Utc::now() - t.to_utc() < APP_CACHE_TTL)
    });
    if fresh && let Some(hit) = cached.as_ref().and_then(parse) {
        return hit;
    }
    let api = s.steam.api();
    match api.schema(app).await {
        Ok(schema) => {
            let rarity = api.global_percentages(app).await.unwrap_or_default();
            let stored = (
                serde_json::to_string(&schema),
                serde_json::to_string(&rarity),
            );
            if let (Ok(sj), Ok(rj)) = stored
                && let Err(e) = sqlx::query(
                    "INSERT INTO steam_app_cache (app_id, name, schema, rarity, fetched_at) VALUES (?, ?, ?, ?, ?)
                     ON CONFLICT (app_id) DO UPDATE SET name = excluded.name, schema = excluded.schema,
                         rarity = excluded.rarity, fetched_at = excluded.fetched_at",
                )
                .bind(app)
                .bind(&schema.game)
                .bind(sj)
                .bind(rj)
                .bind(now())
                .execute(&s.db)
                .await
            {
                tracing::warn!(error = %e, "Steam app cache not stored");
            }
            (schema, rarity)
        }
        Err(e) => {
            tracing::debug!(?e, app, "Steam schema unavailable");
            cached.as_ref().and_then(parse).unwrap_or_default()
        }
    }
}

/// The text channels picked as Achievements channels in the servers `user` is in (live servers and channels), less
/// those of servers where a member is someone `user` hides their games from (#438).
pub async fn channels_for(s: &AppState, user: UserId) -> AppResult<Vec<ChannelId>> {
    let ids: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT a.channel_id FROM server_achievement_channels a
         JOIN server_members m ON m.server_id = a.server_id AND m.user_id = ?1
         JOIN servers sv ON sv.id = a.server_id AND sv.deleted_at IS NULL
         JOIN channels c ON c.id = a.channel_id AND c.deleted_at IS NULL AND c.archived_at IS NULL AND c.kind = 'text'
         WHERE NOT {}
         ORDER BY a.server_id",
        crate::activity_hidden::hides_in_server("?1", "a.server_id")
    )))
    .bind(user.to_string())
    .fetch_all(&s.db)
    .await?;
    Ok(ids.into_iter().filter_map(|i| i.parse().ok()).collect())
}

/// A picture for a card, kept from the media sweep.
async fn card_thumb(s: &AppState, user: UserId, url: &str) -> Option<EmbedThumb> {
    let t = s
        .embeds
        .remote_thumb(&s.db, &s.cfg.media_dir, user, url)
        .await?;
    keep(s, &t).await;
    Some(t)
}

async fn keep(s: &AppState, t: &EmbedThumb) {
    if let Err(e) = sqlx::query("INSERT OR IGNORE INTO steam_card_media (hash) VALUES (?)")
        .bind(&t.hash)
        .execute(&s.db)
        .await
    {
        tracing::warn!(error = %e, "card media not kept");
    }
}

async fn display_name(s: &AppState, user: UserId) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT CASE WHEN display_name != '' THEN display_name ELSE username END FROM users WHERE id = ?",
    )
    .bind(user.to_string())
    .fetch_optional(&s.db)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "Someone".into())
}

/// "12.5 %", "0.4 %", "<0.1 %".
pub fn percent_text(p: f32) -> String {
    if p < 0.1 {
        "<0.1 %".into()
    } else {
        format!("{} %", (p * 10.0).round() / 10.0)
    }
}

/// The fallback text older clients show for `card`.
pub fn content(name: &str, card: &AchievementCard) -> String {
    if card.complete {
        return format!(
            "💯 {name} completed {} ({}/{})",
            card.game, card.unlocked, card.total
        );
    }
    let first = card.achievements.first().map(|a| {
        let rarity = a
            .global_percent
            .map(|p| format!(" ({} of players)", percent_text(p)))
            .unwrap_or_default();
        format!("{}{rarity}", a.name)
    });
    let more = card.achievements.len().saturating_sub(1) as u32 + card.more;
    match (first, more) {
        (Some(f), 0) => format!("🏆 {name} unlocked {f} in {}", card.game),
        (Some(f), n) => format!("🏆 {name} unlocked {f} and {n} more in {}", card.game),
        (None, _) => format!("🏆 {name} unlocked achievements in {}", card.game),
    }
}

/// `user` unlocked `new` (API names) in `app` and is now at `unlocked` of `total`; `completed`: that made 100 %.
/// Recorded for the awards whatever happens; a card goes to each Achievements channel (none: nothing is fetched).
#[allow(clippy::too_many_arguments)]
pub async fn announce(
    s: &AppState,
    user: UserId,
    app: u32,
    game_hint: Option<&str>,
    new: &[(String, i64)],
    unlocked: u32,
    total: u32,
    completed: bool,
) -> AppResult<()> {
    if new.is_empty() && !completed {
        return Ok(());
    }
    let (schema, rarity) = app_info(s, app).await;
    let game = schema
        .game
        .as_deref()
        .or(game_hint)
        .and_then(|g| super::one_line(g, 120))
        .unwrap_or_else(|| format!("App {app}"));
    let at = |t: i64| {
        chrono::DateTime::from_timestamp(t, 0)
            .map(crate::db::stamp)
            .unwrap_or_else(now)
    };
    // the awards' record first: it doesn't depend on any channel
    for (api_name, time) in new {
        let name = schema
            .achievements
            .iter()
            .find(|a| &a.api_name == api_name)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| api_name.clone());
        sqlx::query(
            "INSERT INTO steam_unlocks (user_id, app_id, api_name, name, global_percent, unlocked_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(user.to_string())
        .bind(app)
        .bind(api_name)
        .bind(super::one_line(&name, 200).unwrap_or_default())
        .bind(rarity.get(api_name).map(|p| f64::from(*p)))
        .bind(at(*time))
        .execute(&s.db)
        .await?;
    }
    if completed {
        sqlx::query(
            "INSERT INTO steam_unlocks (user_id, app_id, api_name, name, global_percent, unlocked_at) VALUES (?, ?, NULL, ?, NULL, ?)",
        )
        .bind(user.to_string())
        .bind(app)
        .bind(&game)
        .bind(now())
        .execute(&s.db)
        .await?;
    }
    let channels = channels_for(s, user).await?;
    if channels.is_empty() {
        return Ok(());
    }
    let name = display_name(s, user).await;
    let mut cards = Vec::new();
    if !new.is_empty() {
        let mut achievements = Vec::new();
        for (api_name, _) in new.iter().take(SHOWN) {
            let def = schema.achievements.iter().find(|a| &a.api_name == api_name);
            let icon = match def.and_then(|d| d.icon.as_deref()) {
                Some(url) => card_thumb(s, user, url).await,
                None => None,
            };
            achievements.push(AchievementEntry {
                name: def
                    .and_then(|d| super::one_line(&d.name, 200))
                    .unwrap_or_else(|| api_name.clone()),
                description: def
                    .and_then(|d| d.description.as_deref())
                    .and_then(|d| super::one_line(d, 300)),
                icon,
                global_percent: rarity.get(api_name).copied(),
            });
        }
        cards.push(AchievementCard {
            user_id: user,
            app_id: app,
            game: game.clone(),
            achievements,
            more: new.len().saturating_sub(SHOWN) as u32,
            unlocked,
            total,
            complete: false,
            art: None,
        });
    }
    if completed {
        let art = s
            .embeds
            .steam_art(
                &s.db,
                &s.cfg.media_dir,
                user,
                &s.limits.listening_art_user,
                app,
            )
            .await
            .and_then(|a| a.header.or(a.hero));
        if let Some(t) = &art {
            keep(s, t).await;
        }
        cards.push(AchievementCard {
            user_id: user,
            app_id: app,
            game: game.clone(),
            achievements: vec![],
            more: 0,
            unlocked,
            total,
            complete: true,
            art,
        });
    }
    for card in &cards {
        let payload = system::achievement_payload(card)?;
        let text = content(&name, card);
        for ch in &channels {
            if let Err(e) = system::post(s, *ch, text.clone(), payload.clone()).await {
                tracing::warn!(error = ?e, "achievement card not posted");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(n: usize, more: u32, complete: bool) -> AchievementCard {
        AchievementCard {
            user_id: UserId::new(),
            app_id: 10,
            game: "Space Game".into(),
            achievements: (0..n)
                .map(|i| AchievementEntry {
                    name: format!("A{i}"),
                    description: None,
                    icon: None,
                    global_percent: (i == 0).then_some(12.34),
                })
                .collect(),
            more,
            unlocked: 3,
            total: 10,
            complete,
            art: None,
        }
    }

    #[test]
    fn fallback_text() {
        assert_eq!(
            content("alex", &card(1, 0, false)),
            "🏆 alex unlocked A0 (12.3 % of players) in Space Game"
        );
        assert_eq!(
            content("alex", &card(3, 2, false)),
            "🏆 alex unlocked A0 (12.3 % of players) and 4 more in Space Game"
        );
        assert_eq!(
            content("alex", &card(0, 0, true)),
            "💯 alex completed Space Game (3/10)"
        );
        assert_eq!(percent_text(0.04), "<0.1 %");
        assert_eq!(percent_text(50.0), "50 %");
    }
}
