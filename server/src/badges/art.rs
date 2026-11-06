//! Badge art (#225): a picture for a badge, stored only on the server. The admin command puts it
//! through the media pipeline (validated, the first frame fitted within 256 px, re-encoded as a
//! still WebP) and maps the badge key to its hash; the catalogue then carries the hash to clients,
//! which draw it inside the rarity ring instead of the emoji. The pictures never live in the repo.

use std::collections::HashMap;
use std::path::Path;

use sqlx::{SqliteConnection, SqlitePool};

use crate::media::{self, MediaKind, process};

/// Give badge `key` the picture in `bytes`, replacing any it had; returns the picture's hash. The
/// one it replaces is left for the media sweep.
pub async fn set(db: &SqlitePool, dir: &Path, key: &str, bytes: &[u8]) -> anyhow::Result<String> {
    let key = super::admin::known(key)?;
    let bytes = bytes.to_vec();
    let processed = tokio::task::spawn_blocking(move || process::badge_art(&bytes)).await??;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| anyhow::anyhow!("create {}: {e}", dir.display()))?;
    let info = media::put_by(db, dir, MediaKind::BadgeArt, &processed, None).await?;
    sqlx::query(
        "INSERT INTO badge_art (badge_key, media_hash, updated_at) VALUES (?, ?, ?)
         ON CONFLICT (badge_key) DO UPDATE SET media_hash = excluded.media_hash,
                                               updated_at = excluded.updated_at",
    )
    .bind(key)
    .bind(&info.hash)
    .bind(crate::db::now())
    .execute(db)
    .await?;
    Ok(info.hash)
}

/// Take badge `key`'s picture away (back to its emoji); whether it had one.
pub async fn clear(db: &SqlitePool, key: &str) -> anyhow::Result<bool> {
    let key = super::admin::known(key)?;
    let r = sqlx::query("DELETE FROM badge_art WHERE badge_key = ?")
        .bind(key)
        .execute(db)
        .await?;
    Ok(r.rows_affected() > 0)
}

/// Every badge's picture hash, by badge key.
pub async fn all(conn: &mut SqliteConnection) -> sqlx::Result<HashMap<String, String>> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT badge_key, media_hash FROM badge_art")
        .fetch_all(conn)
        .await?;
    Ok(rows.into_iter().collect())
}
