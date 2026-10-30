//! Client nonces: an outbox retry of a post that already landed must not post twice.

use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use sqlx::SqliteConnection;

use crate::error::{AppError, AppResult};

/// Client nonces are UUIDs (36 bytes); anything much longer isn't one of ours.
const MAX_NONCE: usize = 64;

pub(crate) fn check(nonce: Option<&str>) -> AppResult<()> {
    if nonce.is_some_and(|n| n.is_empty() || n.len() > MAX_NONCE) {
        return Err(AppError::BadRequest(format!(
            "nonce must be 1-{MAX_NONCE} bytes"
        )));
    }
    Ok(())
}

/// The message `me` already posted to `channel` with this nonce. Call under the write lock, so
/// that check-then-insert is atomic; the unique index is the backstop.
///
/// A nonce names one send, so a retry gets the original whatever content it carries (the outbox
/// never changes a queued message). The same nonce on another channel can't be a retry of it:
/// that is a 409, and the other channel's message isn't handed back. The index is per author, not
/// per channel, so posting it there isn't possible either.
pub(crate) async fn existing(
    tx: &mut SqliteConnection,
    me: UserId,
    channel: ChannelId,
    nonce: Option<&str>,
) -> AppResult<Option<MessageId>> {
    let Some(nonce) = nonce else {
        return Ok(None);
    };
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT id, channel_id FROM messages WHERE author_id = ? AND nonce = ?")
            .bind(me.to_string())
            .bind(nonce)
            .fetch_optional(tx)
            .await?;
    let Some((id, posted_in)) = row else {
        return Ok(None);
    };
    if posted_in != channel.to_string() {
        return Err(AppError::Conflict("That request was already used.".into()));
    }
    Ok(Some(id.parse().map_err(anyhow::Error::from)?))
}
