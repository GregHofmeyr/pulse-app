//! Client nonces: an outbox retry of a post that already landed must not post twice.

use pulse_protocol::ids::{MessageId, UserId};
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

/// The message `me` already posted with this nonce. Call under the write lock, so that
/// check-then-insert is atomic; the unique index is the backstop.
pub(crate) async fn existing(
    tx: &mut SqliteConnection,
    me: UserId,
    nonce: Option<&str>,
) -> AppResult<Option<MessageId>> {
    let Some(nonce) = nonce else {
        return Ok(None);
    };
    let id: Option<String> =
        sqlx::query_scalar("SELECT id FROM messages WHERE author_id = ? AND nonce = ?")
            .bind(me.to_string())
            .bind(nonce)
            .fetch_optional(tx)
            .await?;
    id.map(|id| id.parse().map_err(|e| anyhow::Error::from(e).into()))
        .transpose()
}
