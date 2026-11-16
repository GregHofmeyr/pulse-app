//! Edit history (#301): an edit keeps the text it replaced, so "(edited)" can show the earlier versions with their
//! times. Anyone who can see the message can read them (it was all said in the chat); a deleted message has none (its
//! delete takes them in the same transaction). Each message keeps its newest [`MAX_EDIT_VERSIONS`].

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use pulse_protocol::ids::MessageId;
use pulse_protocol::rest::{MAX_EDIT_VERSIONS, MessageVersion};
use sqlx::SqliteConnection;

use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

pub fn router() -> Router<AppState> {
    Router::new().route("/messages/{id}/edits", get(list))
}

/// Inside an edit's transaction, before its write: keep the text being replaced (and when it was written) unless the
/// edit leaves it as it was, then drop the oldest beyond the cap. Returns whether a version was kept.
pub(crate) async fn record(
    tx: &mut SqliteConnection,
    id: MessageId,
    new_content: &str,
    replaced_at: &str,
) -> AppResult<bool> {
    let before: Option<(String, String)> = sqlx::query_as(
        "SELECT content, COALESCE(edited_at, created_at) FROM messages WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(id.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    let Some((content, written_at)) = before else {
        return Ok(false);
    };
    if content == new_content {
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO message_edits (message_id, content, written_at, replaced_at) VALUES (?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(&content)
    .bind(&written_at)
    .bind(replaced_at)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM message_edits WHERE message_id = ?1 AND id NOT IN
         (SELECT id FROM message_edits WHERE message_id = ?1 ORDER BY id DESC LIMIT ?2)",
    )
    .bind(id.to_string())
    .bind(MAX_EDIT_VERSIONS)
    .execute(&mut *tx)
    .await?;
    Ok(true)
}

/// Inside a delete's transaction: the message's history goes with it.
pub(crate) async fn on_delete(tx: &mut SqliteConnection, id: MessageId) -> AppResult<()> {
    sqlx::query("DELETE FROM message_edits WHERE message_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// `GET /messages/{id}/edits`: the message's versions, newest first, the current one first of all (as written by its
/// last edit). 404 for a message you can't see, or a deleted one.
async fn list(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<Json<Vec<MessageVersion>>> {
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT channel_id, content, COALESCE(edited_at, created_at) FROM messages
         WHERE id = ? AND deleted_at IS NULL AND kind = 'normal'",
    )
    .bind(id.to_string())
    .fetch_optional(&s.db)
    .await?;
    let (channel, content, written_at) = row.ok_or(AppError::NotFound)?;
    channel_for(&s.db, me, channel.parse().map_err(anyhow::Error::from)?).await?;
    let earlier: Vec<(String, String)> = sqlx::query_as(
        "SELECT content, written_at FROM message_edits WHERE message_id = ? ORDER BY id DESC LIMIT ?",
    )
    .bind(id.to_string())
    .bind(MAX_EDIT_VERSIONS)
    .fetch_all(&s.db)
    .await?;
    let mut versions = vec![MessageVersion {
        content,
        written_at,
    }];
    versions.extend(
        earlier
            .into_iter()
            .map(|(content, written_at)| MessageVersion {
                content,
                written_at,
            }),
    );
    Ok(Json(versions))
}
