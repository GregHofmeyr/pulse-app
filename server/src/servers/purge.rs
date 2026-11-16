//! Deleting a deleted channel's messages for good in short batches (#108), for a channel's purge
//! (`servers::channels`) and a server's (`servers::deleted`). Each batch is its own write
//! transaction, so a big channel never holds the write lock for more than one batch: a send or
//! any other write anywhere goes through between two of them.
//!
//! A purge can stop part-way (a restart, a failed batch): what is left is still a deleted channel,
//! whole apart from its newest messages, which no write can reach (#109: writes re-check that their
//! channel is live in their own transaction) and nothing can restore (it is due: see
//! `servers::channels::restore`). The next run takes up from what is left.

use pulse_protocol::ids::{ChannelId, ServerId};
use pulse_protocol::rest::ChannelKind;
use sqlx::SqliteConnection;

use crate::AppState;
use crate::error::{AppError, AppResult};

/// How many messages one purge transaction deletes.
pub(crate) const BATCH: i64 = 500;

/// What is being purged, which every batch checks again inside its own transaction: the purge
/// stops (404, or 409 for a channel that became held) as soon as it isn't due any more.
pub(crate) enum Purging<'a> {
    /// A deleted channel, due by `due` (`servers::channels`).
    Channel {
        id: ChannelId,
        kind: ChannelKind,
        due: &'a str,
    },
    /// A deleted server, due by `due` (`servers::deleted`); its channels go one after another.
    Server { id: ServerId, due: &'a str },
    /// A group whose last member left (#336, `dms::purge`): deleted at once, never restorable.
    Group { id: ChannelId },
}

impl Purging<'_> {
    pub(crate) async fn check(&self, tx: &mut SqliteConnection) -> AppResult<()> {
        match self {
            Purging::Channel { id, kind, due } => {
                if !crate::servers::channels::still_due(tx, *id, due).await? {
                    return Err(AppError::NotFound);
                }
                if let Some(why) = crate::servers::channels::held(tx, *id, *kind).await? {
                    return Err(AppError::Conflict(why.into()));
                }
                Ok(())
            }
            Purging::Server { id, due } => {
                if crate::servers::deleted::still_due(tx, *id, due).await? {
                    Ok(())
                } else {
                    Err(AppError::NotFound)
                }
            }
            Purging::Group { id } => {
                let hit: Option<i64> = sqlx::query_scalar(
                    "SELECT 1 FROM channels
                     WHERE id = ? AND server_id IS NULL AND deleted_at IS NOT NULL",
                )
                .bind(id.to_string())
                .fetch_optional(&mut *tx)
                .await?;
                hit.map(|_| ()).ok_or(AppError::NotFound)
            }
        }
    }
}

/// Delete every message of `channel`, newest first, `batch` per write transaction (each checking
/// `what` first, under the lock): a reply or a thread reply is always newer than what it points
/// at, so no batch leaves one pointing at a deleted message (one with an older id, from a clock
/// that went back across a restart, loses the link instead of stopping the purge; replies stay in
/// their channel). Quote lines go before their messages (the search triggers rewrite a quote's
/// entry, which the message's then drops); the message's trigger drops its search entry, and the
/// cascades take its reactions, mentions, quote, clips row, poll, pin, previews, inbox reads,
/// thread reads, Quote of the Year options and wheel spin; its files keep their channel (and lose
/// their message), so the purge's last transaction still finds them. Each batch's clip objects are
/// deleted from R2 after its commit (the clip sweeper retries a failure).
///
/// `Ok(true)` once the channel has no messages left; `Ok(false)` when it stopped after
/// `max_batches` (only tests stop early: a purge cut off by a restart).
pub(crate) async fn messages(
    s: &AppState,
    channel: ChannelId,
    what: &Purging<'_>,
    batch: i64,
    max_batches: Option<usize>,
) -> AppResult<bool> {
    let id = channel;
    let channel = channel.to_string();
    let mut done = 0;
    let mut longest = std::time::Duration::ZERO;
    let finished = |done: usize, longest: std::time::Duration| {
        if done > 0 {
            tracing::info!(channel = %id, batches = done, longest_ms = longest.as_millis() as u64,
                "purged a deleted channel's messages");
        }
        true
    };
    loop {
        if max_batches.is_some_and(|m| done >= m) {
            return Ok(false);
        }
        // Unread and mention counts others publish as absolutes span channels: in commit order.
        let order = s.hub.in_order().await;
        let mut tx = crate::db::begin_write(&s.db).await?;
        let locked = std::time::Instant::now();
        what.check(&mut tx).await?;
        let page: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM messages WHERE channel_id = ? ORDER BY id DESC LIMIT ?",
        )
        .bind(&channel)
        .bind(batch)
        .fetch_all(&mut *tx)
        .await?;
        if page.is_empty() {
            return Ok(finished(done, longest)); // dropping `tx` ends it: nothing was written
        }
        // The page as a JSON array, for `IN (SELECT value FROM json_each(?))`.
        let ids = serde_json::to_string(&page).map_err(anyhow::Error::from)?;
        const PAGE: &str = "(SELECT value FROM json_each(?1))";
        let oldest = page.last().expect("not empty");
        for col in ["reply_to_id", "thread_root_id"] {
            sqlx::query(&format!(
                "UPDATE messages SET {col} = NULL
                 WHERE channel_id = ?2 AND id < ?3 AND {col} IN {PAGE}"
            ))
            .bind(&ids)
            .bind(&channel)
            .bind(oldest)
            .execute(&mut *tx)
            .await?;
        }
        let clips: Vec<String> = sqlx::query_scalar(&format!(
            "SELECT video_key FROM clips WHERE message_id IN {PAGE}"
        ))
        .bind(&ids)
        .fetch_all(&mut *tx)
        .await?;
        sqlx::query(&format!(
            "DELETE FROM quote_lines WHERE message_id IN {PAGE}"
        ))
        .bind(&ids)
        .execute(&mut *tx)
        .await?;
        sqlx::query(&format!("DELETE FROM messages WHERE id IN {PAGE}"))
            .bind(&ids)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let held = locked.elapsed();
        drop(order);
        remove_clip_objects(s, clips);
        done += 1;
        longest = longest.max(held);
        if (page.len() as i64) < batch {
            return Ok(finished(done, longest));
        }
        // Leave the lock free for as long as the batch held it. Writers queued on `in_order` get
        // it at once, but one waiting in SQLite's busy handler only looks again after sleeping
        // (up to 100 ms between looks): back to back, the batches could starve it past the 5 s
        // busy timeout. At most half the time, then, goes to the purge.
        tokio::time::sleep(held).await;
    }
}

/// A purge's last step, once [`messages`] has emptied deleted channel `id`: in one short
/// transaction (checking `what` first), its mutes (they have no foreign key) and the channel,
/// whose cascades take its read states, voice sessions, calls, clip views and stream sessions; its
/// open clip tickets keep no channel (their confirm then refuses). Every file started in it (#57)
/// is deleted after that commit (the attachments sweep retries a failure).
pub(crate) async fn channel_row(s: &AppState, id: ChannelId, what: &Purging<'_>) -> AppResult<()> {
    // Unread and mention counts others publish as absolutes span channels: in commit order.
    let order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    what.check(&mut tx).await?;
    // Every file started in the channel, sent or not (#57): deleted after the commit.
    let files = crate::attachments::stored_in_channels(&mut tx, &[id]).await?;
    sqlx::query("DELETE FROM notification_prefs WHERE target_kind = 'channel' AND target_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    // No write reaches a deleted channel (#109), so it is empty by now; were a message left, the
    // cascade would take it (its search entry by trigger).
    sqlx::query("DELETE FROM channels WHERE id = ? AND deleted_at IS NOT NULL")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    drop(order);
    if !files.is_empty() {
        let s = s.clone();
        tokio::spawn(async move { crate::attachments::remove_objects(&s, files).await });
    }
    Ok(())
}

/// Delete clip objects from R2 in the background (the clip sweeper retries a failure).
pub(crate) fn remove_clip_objects(s: &AppState, keys: Vec<String>) {
    if keys.is_empty() {
        return;
    }
    let s = s.clone();
    tokio::spawn(async move {
        for key in keys {
            crate::messages::routes::remove_clip_object(&s, &key).await;
        }
    });
}
