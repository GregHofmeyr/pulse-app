//! The media store: processed uploads on disk, content-addressed by SHA-256, one `media` row each.

pub mod process;
pub mod routes;

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::MediaInfo;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::error::{AppError, AppResult};
pub use process::Processed;

pub const CONTENT_TYPE: &str = "image/webp";
/// Below this much free space, uploads still work but the logs say so.
const WARN_FREE_BYTES: u64 = 5 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Emoji,
}

impl MediaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Emoji => "emoji",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "emoji" => Some(Self::Emoji),
            _ => None,
        }
    }
}

/// `dir/ab/abcdef….webp`, sharded by the hash's first two characters.
pub fn path_for(dir: &Path, hash: &str) -> PathBuf {
    dir.join(&hash[..2]).join(format!("{hash}.webp"))
}

/// What a stored hash looks like; anything else never reaches the filesystem.
pub fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Store a processed upload (its still first, then the file pointing at it) and describe it.
/// Storing the same bytes again reuses the existing file and row.
pub async fn put(
    db: &SqlitePool,
    dir: &Path,
    kind: MediaKind,
    meta: &Processed,
    by: UserId,
) -> AppResult<MediaInfo> {
    let mut still_hash = None;
    if let Some(still) = &meta.still {
        still_hash = Some(put_one(db, dir, kind, still, meta, 1, None, by).await?);
    }
    let hash = put_one(
        db,
        dir,
        kind,
        &meta.webp,
        meta,
        meta.frames,
        still_hash.as_deref(),
        by,
    )
    .await?;
    Ok(MediaInfo {
        hash,
        content_type: CONTENT_TYPE.into(),
        width: meta.width,
        height: meta.height,
        frames: meta.frames,
        animated: meta.frames > 1,
        still_hash,
    })
}

#[allow(clippy::too_many_arguments)]
async fn put_one(
    db: &SqlitePool,
    dir: &Path,
    kind: MediaKind,
    bytes: &[u8],
    meta: &Processed,
    frames: u32,
    still_hash: Option<&str>,
    by: UserId,
) -> AppResult<String> {
    let hash = hex::encode(Sha256::digest(bytes));
    let path = path_for(dir, &hash);
    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
        let shard = path.parent().expect("sharded path");
        tokio::fs::create_dir_all(shard)
            .await
            .with_context(|| format!("create {}", shard.display()))?;
        // Write then rename, so a crash never leaves a truncated file under a real hash.
        let tmp = shard.join(format!("{hash}.{:016x}.tmp", rand::random::<u64>()));
        tokio::fs::write(&tmp, bytes)
            .await
            .with_context(|| format!("write {}", tmp.display()))?;
        tokio::fs::rename(&tmp, &path)
            .await
            .with_context(|| format!("rename to {}", path.display()))?;
    }
    // A re-upload restarts the cleanup grace period, so a hash the client was just given is
    // never swept from under it.
    sqlx::query(
        "INSERT INTO media (hash, kind, content_type, size, width, height, frames, still_hash,
                            uploaded_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (hash) DO UPDATE SET created_at = excluded.created_at",
    )
    .bind(&hash)
    .bind(kind.as_str())
    .bind(CONTENT_TYPE)
    .bind(bytes.len() as i64)
    .bind(meta.width)
    .bind(meta.height)
    .bind(frames)
    .bind(still_hash)
    .bind(by.to_string())
    .bind(crate::db::now())
    .execute(db)
    .await?;
    Ok(hash)
}

/// Refuse uploads when the media filesystem has less than `min_free` bytes free.
pub fn disk_ok(dir: &Path, min_free: u64) -> AppResult<()> {
    #[cfg(unix)]
    {
        let st = rustix::fs::statvfs(dir).with_context(|| format!("statvfs {}", dir.display()))?;
        let free = st.f_bavail.saturating_mul(st.f_frsize);
        if free < min_free {
            tracing::error!(free, dir = %dir.display(), "media disk full, refusing uploads");
            return Err(AppError::InsufficientStorage);
        }
        if free < WARN_FREE_BYTES {
            tracing::warn!(free, dir = %dir.display(), "media disk nearly full");
        }
    }
    #[cfg(not(unix))]
    let _ = (dir, min_free);
    Ok(())
}

/// Delete media nothing uses that is older than `grace`: rows first, then files. A still goes
/// with its parent. Returns how many were deleted.
pub async fn cleanup(db: &SqlitePool, dir: &Path, grace: Duration) -> AppResult<u64> {
    let cutoff = chrono::Utc::now()
        - chrono::Duration::from_std(grace).context("cleanup grace out of range")?;
    let cutoff = cutoff.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let gone: Vec<String> = sqlx::query_scalar(
        "DELETE FROM media
         WHERE created_at <= ?1
           AND hash NOT IN (SELECT media_hash FROM custom_emoji
                         WHERE media_hash IS NOT NULL)
           AND hash NOT IN (SELECT p.still_hash FROM media p
                            WHERE p.still_hash IS NOT NULL
                              AND (p.created_at > ?1
                                   OR p.hash IN (SELECT media_hash FROM custom_emoji
                                                  WHERE media_hash IS NOT NULL)))
         RETURNING hash",
    )
    .bind(&cutoff)
    .fetch_all(db)
    .await?;
    for hash in &gone {
        let path = path_for(dir, hash);
        match tokio::fs::remove_file(&path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(error = %e, path = %path.display(), "media file not removed"),
        }
    }
    Ok(gone.len() as u64)
}

/// Hourly: drop media unused for a day (an upload awaiting its emoji survives the gap).
pub fn spawn_cleanup(db: SqlitePool, dir: PathBuf) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            match cleanup(&db, &dir, Duration::from_secs(24 * 3600)).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(n, "unused media cleaned up"),
                Err(e) => tracing::warn!(error = %e, "media cleanup failed"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_round_trips() {
        assert_eq!(
            MediaKind::parse(MediaKind::Emoji.as_str()),
            Some(MediaKind::Emoji)
        );
        assert_eq!(MediaKind::parse("avatar"), None);
    }

    #[test]
    fn paths_are_sharded_and_hashes_strict() {
        let h = "ab".repeat(32);
        assert_eq!(
            path_for(Path::new("/m"), &h),
            Path::new("/m/ab").join(format!("{h}.webp"))
        );
        assert!(valid_hash(&h));
        assert!(!valid_hash(&"AB".repeat(32)));
        assert!(!valid_hash("../etc/passwd"));
        assert!(!valid_hash(&"a".repeat(63)));
    }
}
