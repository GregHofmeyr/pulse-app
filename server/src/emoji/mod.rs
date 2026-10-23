//! Per-server custom emoji (soft-deleted, so old messages keep their `:name:`) and the
//! per-user "frequently used" ranking.

pub mod routes;

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::CustomEmoji;
use sqlx::{SqliteConnection, SqlitePool};

use crate::AppState;
use crate::error::{AppError, AppResult};

pub const MAX_PER_SERVER: i64 = 100;
/// How many "frequently used" emoji Ready and [`Event::FrequentEmoji`] carry: 3 rows of 9.
pub const FREQUENT: usize = 27;
const HALF_LIFE_DAYS: f64 = 14.0;

type Row = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    Option<String>,
    String,
);

// A deleted emoji has no media_hash, so the join yields no hashes at all.
const SELECT: &str = "SELECT e.id, e.server_id, e.name, e.media_hash, m.still_hash, e.animated,
                             e.added_by, e.created_at
                      FROM custom_emoji e LEFT JOIN media m ON m.hash = e.media_hash";

fn from_row(
    (id, server_id, name, hash, still_hash, animated, added_by, created_at): Row,
) -> AppResult<CustomEmoji> {
    Ok(CustomEmoji {
        id,
        server_id: server_id.parse::<ServerId>().map_err(anyhow::Error::from)?,
        name,
        hash,
        still_hash,
        animated: animated != 0,
        added_by: added_by
            .map(|u| u.parse::<UserId>())
            .transpose()
            .map_err(anyhow::Error::from)?,
        created_at,
    })
}

/// 2-32 characters of letters, digits and `_`, exactly as given.
pub fn clean_name(name: &str) -> AppResult<&str> {
    let ok = (2..=32).contains(&name.len())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !ok {
        return Err(AppError::BadRequest(
            "emoji names are 2-32 letters, numbers or underscores".into(),
        ));
    }
    Ok(name)
}

/// The key an emoji is stored under, for reactions and usage alike. U+FE0F only asks for emoji
/// presentation, so `❤` and `❤️` are one emoji; `custom:<id>` keys are kept as they are.
pub fn canonical_key(key: &str) -> String {
    if key.starts_with("custom:") {
        key.to_string()
    } else {
        key.replace('\u{FE0F}', "")
    }
}

/// Every server's emoji, deleted ones included (without a hash).
pub async fn for_ready(db: &SqlitePool) -> AppResult<Vec<CustomEmoji>> {
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} ORDER BY e.id"))
        .fetch_all(db)
        .await?;
    rows.into_iter().map(from_row).collect()
}

/// One emoji by id, deleted or not.
pub async fn load(db: &SqlitePool, id: &str) -> AppResult<Option<CustomEmoji>> {
    let row: Option<Row> = sqlx::query_as(&format!("{SELECT} WHERE e.id = ?"))
        .bind(id)
        .fetch_optional(db)
        .await?;
    row.map(from_row).transpose()
}

/// Count a use of each emoji for `user`.
pub async fn record_use(
    conn: &mut SqliteConnection,
    user: UserId,
    emojis: &[String],
) -> AppResult<()> {
    record_use_at(conn, user, emojis, Utc::now()).await
}

/// [`record_use`] with the clock injected, so tests can age scores.
pub async fn record_use_at(
    conn: &mut SqliteConnection,
    user: UserId,
    emojis: &[String],
    now: DateTime<Utc>,
) -> AppResult<()> {
    let at = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut seen = HashSet::new();
    for emoji in emojis {
        let emoji = &canonical_key(emoji);
        // `❤` and `❤️` in one write are one use.
        if !seen.insert(emoji.clone()) {
            continue;
        }
        let prev: Option<(f64, String)> = sqlx::query_as(
            "SELECT score, last_used_at FROM emoji_usage WHERE user_id = ? AND emoji = ?",
        )
        .bind(user.to_string())
        .bind(emoji)
        .fetch_optional(&mut *conn)
        .await?;
        let score = match prev {
            Some((score, last)) => decayed(score, &last, now) + 1.0,
            None => 1.0,
        };
        sqlx::query(
            "INSERT INTO emoji_usage (user_id, emoji, score, last_used_at) VALUES (?, ?, ?, ?)
             ON CONFLICT (user_id, emoji) DO UPDATE SET score = excluded.score,
                                                         last_used_at = excluded.last_used_at",
        )
        .bind(user.to_string())
        .bind(emoji)
        .bind(score)
        .bind(&at)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

fn decayed(score: f64, last_used_at: &str, now: DateTime<Utc>) -> f64 {
    let Ok(last) = DateTime::parse_from_rfc3339(last_used_at) else {
        return score;
    };
    let days = (now - last.with_timezone(&Utc)).num_milliseconds().max(0) as f64 / 86_400_000.0;
    score * 0.5f64.powf(days / HALF_LIFE_DAYS)
}

/// The `n` highest-scoring emoji `user` can use right now, best first, scores decayed to now.
/// Custom emoji that were deleted, or whose server `user` isn't in, are skipped and the next ones
/// down fill their places. (The same "usable" rule as `messages::reactions::live_keys`.)
pub async fn frequent(db: &SqlitePool, user: UserId, n: usize) -> AppResult<Vec<String>> {
    let now = Utc::now();
    let rows: Vec<(String, f64, String)> = sqlx::query_as(
        "SELECT u.emoji, u.score, u.last_used_at FROM emoji_usage u
         WHERE u.user_id = ?1
           AND (substr(u.emoji, 1, 7) != 'custom:' OR EXISTS (
                SELECT 1 FROM custom_emoji e
                JOIN server_members m ON m.server_id = e.server_id AND m.user_id = ?1
                WHERE e.id = substr(u.emoji, 8) AND e.deleted_at IS NULL))",
    )
    .bind(user.to_string())
    .fetch_all(db)
    .await?;
    let mut scored: Vec<(String, f64, String)> = rows
        .into_iter()
        .map(|(e, s, last)| {
            let s = decayed(s, &last, now);
            (e, s, last)
        })
        .collect();
    // Ties go to the more recent use, then the name, so the order is stable.
    scored.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| a.0.cmp(&b.0))
    });
    Ok(scored.into_iter().take(n).map(|(e, ..)| e).collect())
}

/// Tell `user`'s own sessions their new frequently used list, after a committed write that
/// counted their emoji use. Call it under [`crate::gateway::hub::Hub::in_order`] so two writes'
/// lists can't arrive stale-last. Best effort: the write stands whether or not this goes out.
pub async fn publish_frequent(s: &AppState, user: UserId) {
    match frequent(&s.db, user, FREQUENT).await {
        Ok(frequent_emoji) => {
            let event = Event::FrequentEmoji {
                user_id: user,
                frequent_emoji,
            };
            s.hub.publish(&s.db, event).await;
        }
        Err(e) => tracing::error!(error = ?e, %user, "frequent emoji not published"),
    }
}
