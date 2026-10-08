//! Conversation management: groups, read points, mutes, closing.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::ChannelId;
use pulse_protocol::rest::MarkReadRequest;

use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

pub fn router() -> Router<AppState> {
    Router::new().route("/channels/{id}/read", post(mark_read))
}

async fn mark_read(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<MarkReadRequest>,
) -> AppResult<StatusCode> {
    channel_for(&s.db, me, id).await?;
    let in_channel: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM messages WHERE id = ? AND channel_id = ?")
            .bind(req.message_id.to_string())
            .bind(id.to_string())
            .fetch_optional(&s.db)
            .await?;
    if in_channel.is_none() {
        return Err(AppError::BadRequest("message not in this channel".into()));
    }
    if crate::reads::advance(&s.db, me, id, req.message_id).await? {
        s.hub
            .publish(
                &s.db,
                Event::ReadStateUpdated {
                    user_id: me,
                    channel_id: id,
                    last_read_message_id: Some(req.message_id),
                },
            )
            .await;
    }
    Ok(StatusCode::NO_CONTENT)
}
