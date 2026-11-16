//! A group whose last member leaves (#336). The leave marks it deleted (`channels.deleted_at`,
//! which the access rule honours: from then on no read or write reaches it) in its own
//! transaction, then its messages are purged in short batches (`servers::purge`), as Recently
//! deleted purges a server channel, so a long-lived group never holds the write lock for the
//! whole delete. Nobody can bring it back (it has no members, and adding one needs a member), so
//! it is purged at once rather than kept. A purge cut off part-way (a restart) is finished by
//! [`purge_emptied`] on the next sweep; until then the group is still deleted and counts nowhere
//! (every listing and count goes through a membership or a server).

use pulse_protocol::ids::ChannelId;
use sqlx::SqliteConnection;

use crate::AppState;
use crate::error::{AppError, AppResult};
use crate::servers::purge::{BATCH, Purging};

/// Mark group `id` deleted, inside the leave's transaction once its last member is out.
pub(crate) async fn mark_emptied(tx: &mut SqliteConnection, id: ChannelId) -> AppResult<()> {
    sqlx::query("UPDATE channels SET deleted_at = ? WHERE id = ? AND deleted_at IS NULL")
        .bind(crate::db::now())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Emptied group `id` and everything in it, for good: its messages `batch` per transaction
/// (stopping after `max_batches` if given: only tests stop early), then its row and files
/// (`servers::purge::channel_row`). 404 if it isn't an emptied group (any more).
async fn purge_group(
    s: &AppState,
    id: ChannelId,
    batch: i64,
    max_batches: Option<usize>,
) -> AppResult<()> {
    let what = Purging::Group { id };
    if !crate::servers::purge::messages(s, id, &what, batch, max_batches).await? {
        return Ok(()); // cut off (tests)
    }
    crate::servers::purge::channel_row(s, id, &what).await
}

/// How long the last leave waits for its group's purge before answering (clients give up on a
/// request after 15 s): a small group is gone when it answers, a big one shortly after.
const LEAVE_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Purge emptied group `id` as its own task (a client that hangs up doesn't cut it off; a restart
/// can, and the next sweep finishes it), waiting for it at most [`LEAVE_WAIT`]. A failure is
/// logged, not returned: the leave stands either way.
pub(crate) async fn after_leave(s: &AppState, id: ChannelId) {
    let task = {
        let s = s.clone();
        tokio::spawn(async move {
            match purge_group(&s, id, BATCH, None).await {
                // a sweep purged it first
                Ok(()) | Err(AppError::NotFound) => {}
                Err(e) => {
                    tracing::warn!(error = ?e, channel = %id, "couldn't purge an emptied group; the sweep will")
                }
            }
        })
    };
    let _ = tokio::time::timeout(LEAVE_WAIT, task).await;
}

/// Purge every emptied group still there (a purge cut off by a restart, or one that failed).
/// Returns how many went.
pub async fn purge_emptied(s: &AppState) -> AppResult<u32> {
    purge_emptied_batched(s, BATCH).await
}

/// [`purge_emptied`] deleting at most `batch` messages per transaction.
pub async fn purge_emptied_batched(s: &AppState, batch: i64) -> AppResult<u32> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM channels WHERE server_id IS NULL AND deleted_at IS NOT NULL
         ORDER BY deleted_at",
    )
    .fetch_all(&s.db)
    .await?;
    let mut purged = 0;
    for id in ids {
        let id: ChannelId = id.parse().map_err(anyhow::Error::from)?;
        match purge_group(s, id, batch, None).await {
            Ok(()) => purged += 1,
            Err(AppError::NotFound) => {} // purged in the meantime
            // One group that fails doesn't keep the rest (it's tried again next sweep).
            Err(e) => tracing::warn!(error = ?e, channel = %id, "couldn't purge an emptied group"),
        }
    }
    Ok(purged)
}

/// How often the sweeper looks for emptied groups left part-purged.
const SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(3600);

/// Run [`purge_emptied`] now (finishing what a restart cut off) and then hourly. Call after
/// binding the listener.
pub fn spawn_sweeper(s: AppState) {
    tokio::spawn(async move {
        loop {
            match purge_emptied(&s).await {
                Ok(n) if n > 0 => tracing::info!(groups = n, "emptied groups purged"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = ?e, "emptied group sweep failed"),
            }
            tokio::time::sleep(SWEEP_EVERY).await;
        }
    });
}

/// The last member `last` of group `id` leaving as the leave route does, with its purge cut off
/// after `batches` batches of `batch` messages, as a restart would cut it off: for tests.
#[cfg(feature = "testing")]
pub async fn empty_partly(
    s: &AppState,
    id: ChannelId,
    last: pulse_protocol::ids::UserId,
    batch: i64,
    batches: usize,
) -> AppResult<()> {
    let mut tx = crate::db::begin_write(&s.db).await?;
    let removed = sqlx::query("DELETE FROM channel_members WHERE channel_id = ? AND user_id = ?")
        .bind(id.to_string())
        .bind(last.to_string())
        .execute(&mut *tx)
        .await?
        .rows_affected();
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channel_members WHERE channel_id = ?")
        .bind(id.to_string())
        .fetch_one(&mut *tx)
        .await?;
    if removed == 0 || left > 0 {
        return Err(AppError::NotFound);
    }
    mark_emptied(&mut tx, id).await?;
    tx.commit().await?;
    purge_group(s, id, batch, Some(batches)).await
}
