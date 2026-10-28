use std::str::FromStr;

use anyhow::Context;

use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};
use sqlx::{ConnectOptions, Connection, Sqlite, Transaction};

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
    // Migrations get their own connection with foreign keys OFF: rebuilding a table (0008 does)
    // means dropping it, and with them on that would cascade-delete everything pointing at it.
    // Each migration that rebuilds a table guards its own children (0008 refuses on a violation
    // in them, so that is the hard stop). A boot that applied something also checks the whole
    // database, but only logs: a legacy orphan in an unrelated table must not brick every boot.
    // (Run `PRAGMA foreign_key_check` on a backup of production before deploying a migration.)
    let mut migrator = opts.clone().foreign_keys(false).connect().await?;
    let tracked: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(&mut migrator)
    .await?;
    let before: i64 = if tracked == 0 {
        0
    } else {
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&mut migrator)
            .await?
    };
    sqlx::migrate!("./migrations").run(&mut migrator).await?;
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&mut migrator)
        .await?;
    if after > before {
        let broken: Vec<(String, Option<i64>, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(&mut migrator)
                .await?;
        if !broken.is_empty() {
            let mut tables: Vec<&str> = broken.iter().map(|b| b.0.as_str()).collect();
            tables.sort_unstable();
            tables.dedup();
            tracing::error!(
                rows = broken.len(),
                tables = %tables.join(", "),
                "foreign key violations after migrating (left as they are)"
            );
        }
    }
    // The pool opens before the migration connection closes: an in-memory database lives only
    // while a connection to it is open.
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await?;
    migrator.close().await?;
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
    stamp(chrono::Utc::now())
}

/// A time as stored: RFC 3339 UTC with millisecond precision, so stored times compare as strings.
pub fn stamp(t: chrono::DateTime<chrono::Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
