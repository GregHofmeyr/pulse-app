use std::str::FromStr;

use anyhow::Context;

use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};
use sqlx::{Sqlite, Transaction};

/// Open (creating if needed) the database in WAL mode with FKs on, and run migrations.
pub async fn connect(url: &str) -> anyhow::Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        // Safe from corruption in WAL; an OS crash or power cut (not a process crash) can roll back
        // the last commits. One fsync per checkpoint instead of one per commit.
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// Start a transaction that will write. `BEGIN IMMEDIATE` takes the write lock up front, so a
/// handler that reads and then writes queues behind other writers (busy_timeout) instead of
/// failing at once with SQLITE_BUSY when a write commits between its read and its write.
pub async fn begin_write(db: &SqlitePool) -> sqlx::Result<Transaction<'static, Sqlite>> {
    db.begin_with("BEGIN IMMEDIATE").await
}

/// Consistent snapshot of the live database into a new file at `path` (`VACUUM INTO`:
/// safe while the server is running). Fails if `path` already exists.
pub async fn backup(db: &SqlitePool, path: &std::path::Path) -> anyhow::Result<()> {
    anyhow::ensure!(!path.exists(), "{} already exists", path.display());
    let p = path.to_str().context("backup path must be UTF-8")?;
    sqlx::query("VACUUM INTO ?").bind(p).execute(db).await?;
    Ok(())
}

/// Current time as RFC 3339 UTC with millisecond precision (sorts lexically).
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
