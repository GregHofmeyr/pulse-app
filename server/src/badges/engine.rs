//! Turning counters into badges: after a hook commits, [`check`] evaluates the user's counters,
//! writes any new badges or tiers (and who was first to each tier) and tells the earner. The
//! admin command's [`grant`] and [`revoke`] set tiers by hand and tell nobody.

use std::collections::HashMap;

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::Badge;
use sqlx::{SqliteConnection, SqlitePool};

use super::catalogue::{CATALOGUE, evaluate, find};
use super::stats;
use crate::AppState;
use crate::db::{begin_write, now};
use crate::error::{AppError, AppResult};

/// Award whatever `user`'s counters have reached and publish one `BadgeEarned` per badge raised
/// (to the earner only). Returns the raised badges.
pub async fn check(s: &AppState, user: UserId) -> AppResult<Vec<Badge>> {
    let mut tx = begin_write(&s.db).await?;
    let current = tiers_held(&mut tx, user).await?;
    let reached = evaluate(&stats::all(&mut tx, user).await?, &current);
    let at = now();
    let mut raised = Vec::new();
    // Catalogue order, so the toasts come in a stable order.
    for def in CATALOGUE {
        let (Some(&tier), old) = (reached.get(def.key), current.get(def.key).copied()) else {
            continue;
        };
        let old = old.unwrap_or(0);
        if tier > old {
            set_tier(&mut tx, user, def.key, old, tier, &at).await?;
            raised.push(def.key);
        }
    }
    if raised.is_empty() {
        return Ok(Vec::new());
    }
    let mut held: HashMap<String, Badge> = held(&mut tx, user)
        .await?
        .into_iter()
        .map(|b| (b.key.clone(), b))
        .collect();
    tx.commit().await?;

    let raised: Vec<Badge> = raised.iter().filter_map(|k| held.remove(*k)).collect();
    for badge in &raised {
        let def = find(&badge.key).expect("raised from the catalogue");
        s.hub
            .publish(
                &s.db,
                Event::BadgeEarned {
                    user_id: user,
                    badge: badge.clone(),
                    name: def.name.into(),
                    icon: def.icon.into(),
                },
            )
            .await;
    }
    Ok(raised)
}

/// [`check`] each user, after a hook's commit. Badge bookkeeping never fails the request that
/// caused it: errors are logged.
pub async fn after(s: &AppState, users: &[UserId]) {
    for &user in users {
        if let Err(e) = check(s, user).await {
            tracing::warn!(error = ?e, %user, "badge check failed");
        }
    }
}

/// Give `user` exactly `tier` of `key` (unseen), recording them as first to any tier nobody
/// reached before. For the admin command: nobody is told.
pub async fn grant(db: &SqlitePool, user: UserId, key: &str, tier: u8) -> AppResult<()> {
    let def = find(key).ok_or_else(|| unknown(key))?;
    if tier == 0 || tier > def.max_tier() {
        return Err(AppError::BadRequest(format!(
            "{key} has tiers 1 to {}.",
            def.max_tier()
        )));
    }
    let mut tx = begin_write(db).await?;
    let old = tiers_held(&mut tx, user)
        .await?
        .get(def.key)
        .copied()
        .unwrap_or(0);
    set_tier(&mut tx, user, def.key, old, tier, &now()).await?;
    tx.commit().await?;
    Ok(())
}

/// Take `key` from `user`: all of it (`None`), or `Some(t)` keeps the tiers below `t`. Their
/// first-earner records for the removed tiers go too, so someone else can be first.
pub async fn revoke(db: &SqlitePool, user: UserId, key: &str, tier: Option<u8>) -> AppResult<()> {
    let def = find(key).ok_or_else(|| unknown(key))?;
    let keep = tier.map_or(0, |t| t.saturating_sub(1));
    let id = user.to_string();
    let mut tx = begin_write(db).await?;
    if keep == 0 {
        sqlx::query("DELETE FROM user_badges WHERE user_id = ? AND badge_key = ?")
            .bind(&id)
            .bind(def.key)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query(
            "UPDATE user_badges SET tier = MIN(tier, ?) WHERE user_id = ? AND badge_key = ?",
        )
        .bind(keep)
        .bind(&id)
        .bind(def.key)
        .execute(&mut *tx)
        .await?;
    }
    drop_firsts_above(&mut tx, user, def.key, keep).await?;
    tx.commit().await?;
    Ok(())
}

/// Every badge `user` holds, unmasked, oldest first. `first_earner` and `first_tiers` come from
/// `badge_firsts` (the old `user_badges.first_earner` column is no longer read).
pub async fn held(conn: &mut SqliteConnection, user: UserId) -> AppResult<Vec<Badge>> {
    let id = user.to_string();
    let rows: Vec<(String, String, i64, bool)> = sqlx::query_as(
        "SELECT badge_key, earned_at, tier, seen FROM user_badges WHERE user_id = ? \
         ORDER BY earned_at, badge_key",
    )
    .bind(&id)
    .fetch_all(&mut *conn)
    .await?;
    let firsts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT badge_key, tier FROM badge_firsts WHERE user_id = ? ORDER BY badge_key, tier",
    )
    .bind(&id)
    .fetch_all(&mut *conn)
    .await?;
    let stats = stats::all(conn, user).await?;

    Ok(rows
        .into_iter()
        .map(|(key, earned_at, tier, seen)| {
            let tier = tier.clamp(0, u8::MAX.into()) as u8;
            let def = find(&key);
            let first_tiers: Vec<u8> = firsts
                .iter()
                .filter(|(k, _)| *k == key)
                .map(|(_, t)| (*t).clamp(0, u8::MAX.into()) as u8)
                .collect();
            let count = def
                .filter(|d| d.stacking)
                .and_then(|d| stats.get(d.stat()?))
                .map_or(0, |v| (*v).clamp(0, u32::MAX.into()) as u32);
            Badge {
                first_earner: first_tiers.contains(&tier),
                rarity: def
                    .map(|d| d.rarity_at(tier).to_protocol())
                    .unwrap_or_default(),
                key,
                earned_at,
                tier,
                first_tiers,
                count,
                hidden: false,
                seen,
            }
        })
        .collect())
}

/// The tier of each badge `user` holds.
async fn tiers_held(conn: &mut SqliteConnection, user: UserId) -> AppResult<HashMap<String, u8>> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT badge_key, tier FROM user_badges WHERE user_id = ?")
            .bind(user.to_string())
            .fetch_all(conn)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(k, t)| (k, t.clamp(0, u8::MAX.into()) as u8))
        .collect())
}

/// Move `user` from tier `old` to `new` of `key`, unseen and earned `at`. Every tier newly reached
/// that nobody reached before is theirs first (the primary key settles races); going down drops
/// their first-earner records above `new`.
async fn set_tier(
    conn: &mut SqliteConnection,
    user: UserId,
    key: &str,
    old: u8,
    new: u8,
    at: &str,
) -> AppResult<()> {
    let id = user.to_string();
    sqlx::query(
        "INSERT INTO user_badges (user_id, badge_key, earned_at, tier, seen) VALUES (?, ?, ?, ?, 0) \
         ON CONFLICT (user_id, badge_key) DO UPDATE \
         SET tier = excluded.tier, earned_at = excluded.earned_at, seen = 0",
    )
    .bind(&id)
    .bind(key)
    .bind(at)
    .bind(new)
    .execute(&mut *conn)
    .await?;
    for tier in old + 1..=new {
        sqlx::query(
            "INSERT OR IGNORE INTO badge_firsts (badge_key, tier, user_id, earned_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(key)
        .bind(tier)
        .bind(&id)
        .bind(at)
        .execute(&mut *conn)
        .await?;
    }
    drop_firsts_above(conn, user, key, new).await
}

async fn drop_firsts_above(
    conn: &mut SqliteConnection,
    user: UserId,
    key: &str,
    tier: u8,
) -> AppResult<()> {
    sqlx::query("DELETE FROM badge_firsts WHERE user_id = ? AND badge_key = ? AND tier > ?")
        .bind(user.to_string())
        .bind(key)
        .bind(tier)
        .execute(conn)
        .await?;
    Ok(())
}

fn unknown(key: &str) -> AppError {
    AppError::BadRequest(format!("Unknown badge {key}."))
}
