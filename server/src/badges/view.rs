//! Badges as one viewer may see them: the catalogue (`GET /badges`) and a profile's badges and
//! showcase, both through [`mask`].

use std::collections::HashMap;

use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{
    Badge, BadgeCatalogue, BadgeDef, BadgeProgressPart, BadgeTier, BadgeUnit,
};
use sqlx::SqliteConnection;

use super::catalogue::{CATALOGUE, Def, HAND_GIVEN, Rule, Unit, find};
use super::{engine, mask, stats};
use crate::error::AppResult;

/// How many badges a showcase holds.
pub const SHOWCASE_MAX: usize = 5;

/// Every badge in trophy-case order (hand-given last), with each tier's holders and `viewer`'s
/// own progress; hidden badges `viewer` doesn't hold are masked.
pub async fn catalogue(conn: &mut SqliteConnection, viewer: UserId) -> AppResult<BadgeCatalogue> {
    let people: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&mut *conn)
        .await?;
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT badge_key, tier, COUNT(*) FROM user_badges GROUP BY badge_key, tier",
    )
    .fetch_all(&mut *conn)
    .await?;
    let viewer_held = engine::tiers_held(conn, viewer).await?;
    let viewer_stats = stats::all(conn, viewer).await?;
    let held_defs: HashMap<&'static str, u8> = viewer_held
        .iter()
        .filter_map(|(k, t)| Some((find(k)?.key, *t)))
        .collect();
    // Holding tier 3 counts toward tiers 1 and 2 as well.
    let holders = |key: &str, tier: usize| -> u32 {
        rows.iter()
            .filter(|(k, t, _)| k == key && *t >= tier as i64)
            .map(|(_, _, n)| *n)
            .sum::<i64>()
            .clamp(0, u32::MAX.into()) as u32
    };

    let badges = CATALOGUE
        .iter()
        .chain(HAND_GIVEN)
        .map(|def| {
            let masked = mask(def, &viewer_held);
            let tiers = def
                .tiers()
                .into_iter()
                .zip(def.thresholds_shown())
                .enumerate()
                .map(|(i, ((_, rarity), threshold))| BadgeTier {
                    threshold: if masked.is_some() { 0 } else { threshold },
                    rarity: rarity.to_protocol(),
                    holders: holders(def.key, i + 1),
                })
                .collect();
            let hand_given = matches!(def.rule, Rule::HandGiven { .. });
            match masked {
                Some(key) => BadgeDef {
                    key,
                    name: "???".into(),
                    description: String::new(),
                    icon: "❔".into(),
                    hidden: true,
                    hand_given: false,
                    stacking: false,
                    unit: BadgeUnit::Count,
                    tiers,
                    progress: None,
                    parts: Vec::new(),
                },
                None => BadgeDef {
                    key: def.key.into(),
                    name: def.name.into(),
                    description: def.description.into(),
                    icon: def.icon.into(),
                    hidden: def.hidden,
                    hand_given,
                    stacking: def.stacking,
                    unit: unit(def),
                    tiers,
                    progress: def.progress(&viewer_stats, &held_defs),
                    parts: def
                        .parts(&viewer_stats, &held_defs)
                        .into_iter()
                        .map(|(label, value, target)| BadgeProgressPart {
                            label: label.into(),
                            value,
                            target,
                        })
                        .collect(),
                },
            }
        })
        .collect();
    Ok(BadgeCatalogue {
        people: people.clamp(0, u32::MAX.into()) as u32,
        badges,
    })
}

/// `user`'s badges and showcase (and whether the showcase is their own choice) as `viewer` sees
/// them: hidden badges `viewer` doesn't hold are
/// masked (rarity kept; no count, date, tier or firsts) and `seen` is only ever reported to the owner.
pub async fn profile_badges(
    conn: &mut SqliteConnection,
    user: UserId,
    viewer: UserId,
) -> AppResult<(Vec<Badge>, Vec<String>, bool)> {
    let mut badges = engine::held(conn, user).await?;
    let chosen: Vec<String> = sqlx::query_scalar(
        "SELECT badge_key FROM user_badges WHERE user_id = ? AND showcase IS NOT NULL \
         ORDER BY showcase",
    )
    .bind(user.to_string())
    .fetch_all(&mut *conn)
    .await?;
    let showcase_chosen = !chosen.is_empty();
    let showcase = if !showcase_chosen {
        // The rarest five, older first among equals (`held` is oldest first; the sort is stable).
        let mut by_rarity: Vec<&Badge> = badges.iter().collect();
        by_rarity.sort_by_key(|b| std::cmp::Reverse(b.rarity));
        by_rarity
            .into_iter()
            .take(SHOWCASE_MAX)
            .map(|b| b.key.clone())
            .collect()
    } else {
        chosen
    };

    let viewer_held = if viewer == user {
        badges.iter().map(|b| (b.key.clone(), b.tier)).collect()
    } else {
        engine::tiers_held(conn, viewer).await?
    };
    let mut shown_as: HashMap<String, String> = HashMap::new();
    for b in &mut badges {
        if let Some(key) = find(&b.key).and_then(|d| mask(d, &viewer_held)) {
            shown_as.insert(std::mem::replace(&mut b.key, key.clone()), key);
            b.hidden = true;
            b.count = 0;
            // When and who first would give it away too (the showcase is already sorted).
            b.earned_at = String::new();
            b.tier = 1;
            b.first_earner = false;
            b.first_tiers.clear();
        }
        if viewer != user {
            b.seen = true;
        }
    }
    let showcase = showcase
        .into_iter()
        .map(|k| shown_as.get(&k).cloned().unwrap_or(k))
        .collect();
    Ok((badges, showcase, showcase_chosen))
}

fn unit(def: &Def) -> BadgeUnit {
    match def.unit {
        Unit::Count => BadgeUnit::Count,
        Unit::Hours => BadgeUnit::Hours,
    }
}
