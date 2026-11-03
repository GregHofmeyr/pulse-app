//! The `badge` command: award, reset or inspect a user's badges for testing and for hand-given
//! badges. Never exposed in the app. A running server isn't told, so no toast fires; the badge
//! shows (unseen, [NEW]) on the next profile load.

use pulse_protocol::ids::UserId;
use sqlx::SqlitePool;

use super::catalogue::{CATALOGUE, HAND_GIVEN, find, stat};
use super::{engine, stats};

/// `badge award`: the line to print.
pub async fn award(db: &SqlitePool, username: &str, key: &str, tier: u8) -> anyhow::Result<String> {
    let key = known(key)?;
    let (id, name) = user(db, username).await?;
    engine::grant(db, id, key, tier).await?;
    Ok(format!("awarded {key} tier {tier} to {name}"))
}

/// `badge reset`: all of the badge, or with `tier` everything from that tier up.
pub async fn reset(
    db: &SqlitePool,
    username: &str,
    key: &str,
    tier: Option<u8>,
) -> anyhow::Result<String> {
    let key = known(key)?;
    let (id, name) = user(db, username).await?;
    engine::revoke(db, id, key, tier).await?;
    Ok(format!("reset {key} for {name}"))
}

/// `badge stats`: each counter (`key value`), then each badge held (`key tier N`).
pub async fn show(db: &SqlitePool, username: &str) -> anyhow::Result<Vec<String>> {
    let (id, _) = user(db, username).await?;
    let mut conn = db.acquire().await?;
    let mut counters: Vec<_> = stats::all(&mut conn, id).await?.into_iter().collect();
    counters.sort();
    let held = engine::held(&mut conn, id).await?;
    Ok(counters
        .into_iter()
        .map(|(k, v)| format!("{k} {v}"))
        .chain(
            held.into_iter()
                .map(|b| format!("{} tier {}", b.key, b.tier)),
        )
        .collect())
}

/// `badge set-stat`: so the next real event crosses a threshold and unlocks with its toast.
pub async fn set_stat(
    db: &SqlitePool,
    username: &str,
    key: &str,
    value: i64,
) -> anyhow::Result<String> {
    let known = stat::ALL.contains(&key)
        || [stat::EMOJI_USES_PREFIX, stat::VENDETTA_PREFIX]
            .iter()
            .any(|p| key.strip_prefix(p).is_some_and(|rest| !rest.is_empty()));
    anyhow::ensure!(
        known,
        "unknown stat {key}; valid stats: {}, {}<emoji id>, {}<user id>",
        stat::ALL.join(", "),
        stat::EMOJI_USES_PREFIX,
        stat::VENDETTA_PREFIX
    );
    let (id, name) = user(db, username).await?;
    stats::set(&mut *db.acquire().await?, id, key, value).await?;
    Ok(format!("set {key} to {value} for {name}"))
}

/// The catalogue key, or an error listing every valid one.
fn known(key: &str) -> anyhow::Result<&'static str> {
    find(key).map(|d| d.key).ok_or_else(|| {
        let keys: Vec<_> = CATALOGUE.iter().chain(HAND_GIVEN).map(|d| d.key).collect();
        anyhow::anyhow!("unknown badge {key}; valid badges: {}", keys.join(", "))
    })
}

async fn user(db: &SqlitePool, username: &str) -> anyhow::Result<(UserId, String)> {
    let name = crate::auth::routes::normalize_username(username);
    let id: Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE username = ?")
        .bind(&name)
        .fetch_optional(db)
        .await?;
    let id = id.ok_or_else(|| anyhow::anyhow!("no user named {name}"))?;
    Ok((id.parse()?, name))
}
