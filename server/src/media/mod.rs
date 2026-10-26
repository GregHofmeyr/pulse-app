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

use crate::error::AppResult;
pub use process::Processed;

pub const CONTENT_TYPE: &str = "image/webp";
/// Below this much free space, uploads still work but the logs say so.
#[cfg(unix)]
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

/// `dir/ab/abcdef….webp`, sharded by the hash's first two characters. `None` unless `hash` is a
/// stored-hash shape (64 lowercase hex), so nothing else can name a path.
pub fn path_for(dir: &Path, hash: &str) -> Option<PathBuf> {
    valid_hash(hash).then(|| dir.join(&hash[..2]).join(format!("{hash}.webp")))
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
    let path = path_for(dir, &hash).expect("a SHA-256 hex digest is a valid hash");
    ensure_file(&path, &hash, bytes).await?;
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
    // Again after the row exists: a cleanup that deleted this row just before our insert may have
    // unlinked the file after the first check (it does so inside its transaction, so by now it is
    // done). A row must never outlive its file.
    ensure_file(&path, &hash, bytes).await?;
    Ok(hash)
}

/// Write `bytes` at `path` unless it is already there (which also restores a file that went missing).
async fn ensure_file(path: &Path, hash: &str, bytes: &[u8]) -> AppResult<()> {
    if tokio::fs::try_exists(path).await.unwrap_or(false) {
        return Ok(());
    }
    let shard = path.parent().expect("sharded path");
    tokio::fs::create_dir_all(shard)
        .await
        .with_context(|| format!("create {}", shard.display()))?;
    // Write then rename, so a crash never leaves a truncated file under a real hash.
    let tmp = shard.join(format!("{hash}.{:016x}.tmp", rand::random::<u64>()));
    tokio::fs::write(&tmp, bytes)
        .await
        .with_context(|| format!("write {}", tmp.display()))?;
    tokio::fs::rename(&tmp, path)
        .await
        .with_context(|| format!("rename to {}", path.display()))?;
    Ok(())
}

/// Refuse uploads when the media filesystem has less than `min_free` bytes free.
pub fn disk_ok(dir: &Path, min_free: u64) -> AppResult<()> {
    #[cfg(unix)]
    {
        let st = rustix::fs::statvfs(dir).with_context(|| format!("statvfs {}", dir.display()))?;
        let free = st.f_bavail.saturating_mul(st.f_frsize);
        if free < min_free {
            tracing::error!(free, dir = %dir.display(), "media disk full, refusing uploads");
            return Err(crate::error::AppError::InsufficientStorage);
        }
        if free < WARN_FREE_BYTES {
            tracing::warn!(free, dir = %dir.display(), "media disk nearly full");
        }
    }
    #[cfg(not(unix))]
    let _ = (dir, min_free);
    Ok(())
}

/// The directory holding the SQLite file named by `db_url`; `None` for in-memory databases.
fn db_dir(db_url: &str) -> Option<PathBuf> {
    let rest = db_url
        .strip_prefix("sqlite://")
        .or_else(|| db_url.strip_prefix("sqlite:"))
        .unwrap_or(db_url);
    let path = rest.split('?').next().unwrap_or(rest);
    if path.is_empty() || path == ":memory:" {
        return None;
    }
    match Path::new(path).parent() {
        Some(p) if !p.as_os_str().is_empty() => Some(p.to_path_buf()),
        _ => Some(PathBuf::from(".")),
    }
}

/// `dir`, or its nearest ancestor that exists (the media dir is created on first upload).
fn existing_ancestor(dir: &Path) -> &Path {
    dir.ancestors().find(|d| d.exists()).unwrap_or(dir)
}

/// The same free-space check for a write that grows the database (a message, a report, an
/// emoji): refuses when either the database's directory or the media directory is low.
/// Usually these are one volume; they are both checked in case they are not.
pub fn writes_ok(cfg: &crate::config::Config) -> AppResult<()> {
    if let Some(dir) = db_dir(&cfg.db_url) {
        disk_ok(existing_ancestor(&dir), cfg.media_min_free_bytes)?;
    }
    disk_ok(existing_ancestor(&cfg.media_dir), cfg.media_min_free_bytes)
}

/// Delete media nothing uses that is older than `grace`: rows, then their files, in one
/// transaction. A re-upload's insert waits for the commit, so it sees the files gone and rewrites
/// them (see `put_one`). A still goes with its parent. Returns how many were deleted.
pub async fn cleanup(db: &SqlitePool, dir: &Path, grace: Duration) -> AppResult<u64> {
    let cutoff = chrono::Utc::now()
        - chrono::Duration::from_std(grace).context("cleanup grace out of range")?;
    let cutoff = cutoff.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut tx = db.begin().await?;
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
    .fetch_all(&mut *tx)
    .await?;
    for hash in &gone {
        let Some(path) = path_for(dir, hash) else {
            tracing::warn!(hash, "media row with a malformed hash; no file removed");
            continue;
        };
        match tokio::fs::remove_file(&path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(error = %e, path = %path.display(), "media file not removed"),
        }
    }
    tx.commit().await?;
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
    fn db_dir_from_url() {
        assert_eq!(db_dir("sqlite:///data/pulse.db"), Some("/data".into()));
        assert_eq!(
            db_dir("sqlite:///data/pulse.db?mode=rwc"),
            Some("/data".into())
        );
        assert_eq!(db_dir("sqlite://pulse.db"), Some(".".into()));
        assert_eq!(db_dir("sqlite::memory:"), None);
    }

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
            Some(Path::new("/m/ab").join(format!("{h}.webp")))
        );
        // Short, odd or non-ASCII input is refused, never sliced.
        for bad in ["", "a", "ab", "../etc/passwd", "é", &"é".repeat(32)] {
            assert_eq!(path_for(Path::new("/m"), bad), None, "{bad:?}");
        }
        assert_eq!(path_for(Path::new("/m"), &"AB".repeat(32)), None);
        assert!(valid_hash(&h));
        assert!(!valid_hash(&"AB".repeat(32)));
        assert!(!valid_hash("../etc/passwd"));
        assert!(!valid_hash(&"a".repeat(63)));
    }
}
