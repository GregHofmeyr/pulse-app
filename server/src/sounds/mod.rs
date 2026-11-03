//! Per-server soundboard sounds. Unlike emoji they are hard-deleted: nothing refers to a sound by
//! name, so there is no history to keep.

pub mod routes;

use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::Sound;
use sqlx::SqlitePool;

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
