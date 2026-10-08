//! Private read points: where each user stopped reading each channel. Never visible to others.

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
    db: &SqlitePool,
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
    .execute(db)
    .await?;
    Ok(r.rows_affected() > 0)
}

/// Unread = others' normal, non-deleted messages after the point; mentions = your mentions after it.
/// Channels without a point (never joined/initialised) are reported as fully read.
pub async fn states_for(db: &SqlitePool, me: UserId) -> AppResult<Vec<ReadState>> {
    let rows: Vec<(String, Option<String>, i64, i64)> = sqlx::query_as(
        "SELECT rs.channel_id, rs.last_read_message_id,
            (SELECT COUNT(*) FROM messages m WHERE m.channel_id = rs.channel_id
               AND m.id > COALESCE(rs.last_read_message_id, '') AND m.kind = 'normal'
               AND m.deleted_at IS NULL AND COALESCE(m.author_id, '') != rs.user_id),
            (SELECT COUNT(*) FROM mentions mn JOIN messages m ON m.id = mn.message_id
               WHERE mn.user_id = rs.user_id AND m.channel_id = rs.channel_id
               AND m.id > COALESCE(rs.last_read_message_id, '') AND m.deleted_at IS NULL)
         FROM read_states rs WHERE rs.user_id = ?",
    )
    .bind(me.to_string())
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
            crate::messages::routes::load(db, id.parse().map_err(anyhow::Error::from)?).await?
        {
            out.push(m);
        }
    }
    Ok(out)
}
