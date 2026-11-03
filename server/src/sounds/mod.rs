//! Per-server soundboard sounds. Unlike emoji they are hard-deleted: nothing refers to a sound by
//! name, so there is no history to keep.

pub mod routes;

use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::Sound;
use sqlx::{SqliteConnection, SqlitePool};

use crate::AppState;
use crate::emoji::decayed;
use crate::error::{AppError, AppResult};

/// How many "frequently played" sounds Ready and `Event::FrequentSounds` carry.
pub const FREQUENT: usize = 9;

type Row = (
    String,
    String,
    String,
    Option<String>,
    String,
    i64,
    i64,
    Option<String>,
    String,
    i64,
);

const SELECT: &str = "SELECT id, server_id, name, emoji, media_hash, volume, duration_ms,
                             added_by, created_at, plays
                      FROM sounds";

fn from_row(
    (id, server_id, name, emoji, media_hash, volume, duration_ms, added_by, created_at, plays): Row,
) -> AppResult<Sound> {
    Ok(Sound {
        id,
        server_id: server_id.parse::<ServerId>().map_err(anyhow::Error::from)?,
        name,
        emoji,
        media_hash,
        volume: u16::try_from(volume).map_err(anyhow::Error::from)?,
        duration_ms: u32::try_from(duration_ms).map_err(anyhow::Error::from)?,
        added_by: added_by
            .map(|u| u.parse::<UserId>())
            .transpose()
            .map_err(anyhow::Error::from)?,
        created_at,
        plays: u32::try_from(plays.max(0)).unwrap_or(u32::MAX),
    })
}

/// Every server's sounds, oldest first.
pub async fn for_ready(db: &SqlitePool) -> AppResult<Vec<Sound>> {
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} ORDER BY id"))
        .fetch_all(db)
        .await?;
    rows.into_iter().map(from_row).collect()
}

/// One server's sounds, oldest first.
pub async fn for_server(db: &SqlitePool, server: ServerId) -> AppResult<Vec<Sound>> {
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} WHERE server_id = ? ORDER BY id"))
        .bind(server.to_string())
        .fetch_all(db)
        .await?;
    rows.into_iter().map(from_row).collect()
}

/// One sound by id.
pub async fn load(db: &SqlitePool, id: &str) -> AppResult<Option<Sound>> {
    let row: Option<Row> = sqlx::query_as(&format!("{SELECT} WHERE id = ?"))
        .bind(id)
        .fetch_optional(db)
        .await?;
    row.map(from_row).transpose()
}

pub(crate) fn bad_volume() -> AppError {
    AppError::BadRequest("volume is between 10% and 200%".into())
}

/// Count a play of `sound` for `user`: bump the sound's total and decay-and-add their personal
/// score (same 14-day half-life as emoji).
pub async fn record_play(conn: &mut SqliteConnection, user: UserId, sound: &str) -> AppResult<()> {
    record_play_at(conn, user, sound, chrono::Utc::now()).await
}

/// [`record_play`] with the clock injected.
pub async fn record_play_at(
    conn: &mut SqliteConnection,
    user: UserId,
    sound: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> AppResult<()> {
    let prev: Option<(f64, String)> = sqlx::query_as(
        "SELECT score, last_used_at FROM sound_usage WHERE user_id = ? AND sound_id = ?",
    )
    .bind(user.to_string())
    .bind(sound)
    .fetch_optional(&mut *conn)
    .await?;
    let score = match prev {
        Some((score, last)) => decayed(score, &last, now) + 1.0,
        None => 1.0,
    };
    sqlx::query(
        "INSERT INTO sound_usage (user_id, sound_id, score, last_used_at) VALUES (?, ?, ?, ?)
         ON CONFLICT (user_id, sound_id) DO UPDATE SET score = excluded.score,
                                                       last_used_at = excluded.last_used_at",
    )
    .bind(user.to_string())
    .bind(sound)
    .bind(score)
    .bind(crate::db::stamp(now))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The `n` sounds `user` plays most (scores decayed to now), best first. Sounds of servers they
/// are not in are skipped and the next ones fill their places.
pub async fn frequent(db: &SqlitePool, user: UserId, n: usize) -> AppResult<Vec<String>> {
    let now = chrono::Utc::now();
    let rows: Vec<(String, f64, String)> = sqlx::query_as(
        "SELECT u.sound_id, u.score, u.last_used_at FROM sound_usage u
         JOIN sounds s ON s.id = u.sound_id
         JOIN server_members m ON m.server_id = s.server_id AND m.user_id = ?1
         WHERE u.user_id = ?1",
    )
    .bind(user.to_string())
    .fetch_all(db)
    .await?;
    let mut scored: Vec<(String, f64, String)> = rows
        .into_iter()
        .map(|(id, s, last)| (id, decayed(s, &last, now), last))
        .collect();
    // Ties go to the more recent play, then the id, so the order is stable.
    scored.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| a.0.cmp(&b.0))
    });
    Ok(scored.into_iter().take(n).map(|(id, ..)| id).collect())
}

/// `user`'s starred sound ids, oldest star first.
pub async fn favourites(db: &SqlitePool, user: UserId) -> AppResult<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT sound_id FROM sound_favourites WHERE user_id = ? ORDER BY created_at, sound_id",
    )
    .bind(user.to_string())
    .fetch_all(db)
    .await?)
}

/// Tell `user`'s own sessions their new frequently played list. Call under `Hub::in_order` so
/// two plays' lists can't arrive stale-last. Best effort: the play stands either way.
pub async fn publish_frequent(s: &AppState, user: UserId) {
    match frequent(&s.db, user, FREQUENT).await {
        Ok(frequent) => {
            let event = pulse_protocol::gateway::Event::FrequentSounds {
                user_id: user,
                frequent,
            };
            s.hub.publish(&s.db, event).await;
        }
        Err(e) => tracing::error!(error = ?e, %user, "frequent sounds not published"),
    }
}

/// Tell `user`'s own sessions their new favourites (same ordering rule as above).
pub async fn publish_favourites(s: &AppState, user: UserId) {
    match favourites(&s.db, user).await {
        Ok(favourites) => {
            let event = pulse_protocol::gateway::Event::SoundFavourites {
                user_id: user,
                favourites,
            };
            s.hub.publish(&s.db, event).await;
        }
        Err(e) => tracing::error!(error = ?e, %user, "sound favourites not published"),
    }
}
