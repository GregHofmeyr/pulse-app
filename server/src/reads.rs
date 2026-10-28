//! Private read points: where each user stopped reading each channel. Never visible to others.

use pulse_protocol::gateway::ThreadSeenPoint;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{Message, ReadState};
use sqlx::{SqliteConnection, SqlitePool};

use crate::error::AppResult;

/// Start `user` at "everything so far is read" in `channel` (no-op if they already have a point).
pub async fn init_point(
    c: &mut SqliteConnection,
    user: UserId,
    channel: ChannelId,
) -> AppResult<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO read_states (user_id, channel_id, last_read_message_id)
         VALUES (?, ?, (SELECT MAX(id) FROM messages WHERE channel_id = ?))",
    )
    .bind(user.to_string())
    .bind(channel.to_string())
    .bind(channel.to_string())
    .execute(&mut *c)
    .await?;
    Ok(())
}

/// Move the point forward to `message` (never backwards). True if it moved.
pub async fn advance(
    c: &mut SqliteConnection,
    user: UserId,
    channel: ChannelId,
    message: MessageId,
) -> AppResult<bool> {
    let r = sqlx::query(
        "INSERT INTO read_states (user_id, channel_id, last_read_message_id) VALUES (?, ?, ?)
         ON CONFLICT (user_id, channel_id) DO UPDATE SET last_read_message_id = excluded.last_read_message_id
         WHERE read_states.last_read_message_id IS NULL OR excluded.last_read_message_id > read_states.last_read_message_id",
    )
    .bind(user.to_string())
    .bind(channel.to_string())
    .bind(message.to_string())
    .execute(&mut *c)
    .await?;
    Ok(r.rows_affected() > 0)
}

/// Unread = others' normal, non-deleted main-list messages after the point (replies never count);
/// mentions = your mentions in the main list after it, plus those in a quote's replies after your
/// seen point for that quote.
/// Channels without a point (never joined/initialised) are reported as fully read.
pub async fn states_for(db: &SqlitePool, me: UserId) -> AppResult<Vec<ReadState>> {
    read_states(db, me, None).await
}

/// `user`'s absolute (unread, mentions) counts in `channel`, counted exactly as [`states_for`] does
/// ((0, 0) for a channel without a read point). Events carry them so clients needn't infer them from
/// what they happen to have loaded; call it under the same `Hub::in_order` guard the event is
/// published under.
pub async fn counts_for(
    db: &SqlitePool,
    user: UserId,
    channel: ChannelId,
) -> AppResult<(u32, u32)> {
    Ok(read_states(db, user, Some(channel))
        .await?
        .first()
        .map_or((0, 0), |r| (r.unread, r.mentions)))
}

/// [`counts_for`] after a change has committed, for the event that announces it: the change has
/// landed, so a failed read is logged and gives `None` (the event is skipped) rather than failing a
/// request whose retry would find nothing left to do.
pub async fn counts_after_commit(
    db: &SqlitePool,
    user: UserId,
    channel: ChannelId,
) -> Option<(u32, u32)> {
    match counts_for(db, user, channel).await {
        Ok(c) => Some(c),
        Err(e) => {
            tracing::error!(error = ?e, %user, %channel, "couldn't read counts after a commit; event skipped");
            None
        }
    }
}

/// The reply mentions still counted for a user: a mention in a live reply of a live quote, after the
/// user's seen point for that quote. Shared by the per-channel counts and the seen-point list, so
/// the two can never disagree: only channels the user has a read state for.
/// Binds: user, channel (or NULL for all), channel.
const COUNTED_REPLY_MENTIONS: &str = "FROM mentions mn JOIN messages m ON m.id = mn.message_id
    JOIN messages root ON root.id = m.thread_root_id
    JOIN read_states rs0 ON rs0.user_id = mn.user_id AND rs0.channel_id = m.channel_id
    LEFT JOIN thread_reads tr ON tr.user_id = mn.user_id AND tr.root_id = m.thread_root_id
    WHERE mn.user_id = ? AND (? IS NULL OR m.channel_id = ?) AND m.thread_root_id IS NOT NULL
      AND m.deleted_at IS NULL AND root.deleted_at IS NULL
      AND m.id > COALESCE(tr.last_seen_id, '')";

async fn read_states(
    db: &SqlitePool,
    me: UserId,
    only: Option<ChannelId>,
) -> AppResult<Vec<ReadState>> {
    let sql = format!(
        "SELECT rs.channel_id, rs.last_read_message_id,
            (SELECT COUNT(*) FROM messages m WHERE m.channel_id = rs.channel_id
               AND m.id > COALESCE(rs.last_read_message_id, '') AND m.kind = 'normal'
               AND m.deleted_at IS NULL AND m.thread_root_id IS NULL AND COALESCE(m.author_id, '') != rs.user_id),
            (SELECT COUNT(*) FROM mentions mn JOIN messages m ON m.id = mn.message_id
               WHERE mn.user_id = rs.user_id AND m.channel_id = rs.channel_id
               AND m.deleted_at IS NULL AND m.thread_root_id IS NULL
               AND m.id > COALESCE(rs.last_read_message_id, '')) + COALESCE(tm.n, 0)
         FROM read_states rs
         LEFT JOIN (
            SELECT m.channel_id AS channel_id, COUNT(*) AS n
            {COUNTED_REPLY_MENTIONS}
            GROUP BY m.channel_id
         ) tm ON tm.channel_id = rs.channel_id
         WHERE rs.user_id = ? AND (? IS NULL OR rs.channel_id = ?)"
    );
    let only = only.map(|c| c.to_string());
    let rows: Vec<(String, Option<String>, i64, i64)> = sqlx::query_as(&sql)
        .bind(me.to_string())
        .bind(&only)
        .bind(&only)
        .bind(me.to_string())
        .bind(&only)
        .bind(&only)
        .fetch_all(db)
        .await?;
    rows.into_iter()
        .map(|(c, last, unread, mentions)| {
            Ok(ReadState {
                channel_id: c.parse().map_err(anyhow::Error::from)?,
                last_read_message_id: last
                    .map(|l| l.parse())
                    .transpose()
                    .map_err(anyhow::Error::from)?,
                unread: unread as u32,
                mentions: mentions as u32,
            })
        })
        .collect()
}

/// `me`'s seen point in every quote thread where a reply mention of theirs is still counted.
pub async fn thread_seen_for(db: &SqlitePool, me: UserId) -> AppResult<Vec<ThreadSeenPoint>> {
    let sql = format!(
        "SELECT DISTINCT m.thread_root_id, tr.last_seen_id {COUNTED_REPLY_MENTIONS} ORDER BY m.thread_root_id"
    );
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(&sql)
        .bind(me.to_string())
        .bind(None::<String>)
        .bind(None::<String>)
        .fetch_all(db)
        .await?;
    rows.into_iter()
        .map(|(root, seen)| {
            Ok(ThreadSeenPoint {
                root_id: root.parse().map_err(anyhow::Error::from)?,
                last_seen_id: seen
                    .map(|l| l.parse())
                    .transpose()
                    .map_err(anyhow::Error::from)?,
            })
        })
        .collect()
}

/// The newest message of each DM/group `me` belongs to.
pub async fn latest_for(db: &SqlitePool, me: UserId) -> AppResult<Vec<Message>> {
    let ids: Vec<String> = sqlx::query_scalar::<_, Option<String>>(
        "SELECT (SELECT MAX(m.id) FROM messages m WHERE m.channel_id = cm.channel_id)
         FROM channel_members cm WHERE cm.user_id = ?",
    )
    .bind(me.to_string())
    .fetch_all(db)
    .await?
    .into_iter()
    .flatten()
    .collect();
    let mut out = Vec::new();
    for id in ids {
        if let Some(m) =
            crate::messages::routes::load(db, None, id.parse().map_err(anyhow::Error::from)?)
                .await?
        {
            out.push(m);
        }
    }
    Ok(out)
}
