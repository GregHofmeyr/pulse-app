//! `POST /messages/{id}/embeds/hide`: the sender removes a preview (or all of a message's), for
//! everyone. Hidden is kept per link, through edits. `POST /messages/{id}/embeds/show` (#94): the
//! sender brings back the ones whose link is still in the message.

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::MessageId;
use pulse_protocol::rest::{HideEmbedRequest, Message};

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::messages::routes::{load, own_message};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/messages/{id}/embeds/hide", post(hide))
        .route("/messages/{id}/embeds/show", post(show))
}

async fn hide(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
    Json(req): Json<HideEmbedRequest>,
) -> AppResult<Json<Message>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let (_, ch) = own_message(&s.db, me, id).await?;
    // Stored links are normalised; a link that isn't one has no preview.
    let url = match req.url.as_deref() {
        Some(u) => Some(super::links::normalise(u).ok_or(AppError::NotFound)?),
        None => None,
    };
    // In order with the unfurl's and edits' publishes, so the last update out is the newest.
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    crate::access::channel_live_in(&mut tx, ch.id).await?; // channel deleted since (#109)
    crate::access::open_in(&mut tx, ch.id).await?; // archived (#303): previews stay as they are, like the text
    // The message must still be live: a delete that committed since the check above wins.
    let hidden = sqlx::query(
        "UPDATE link_embeds SET hidden = 1
         WHERE message_id = ? AND (? IS NULL OR url = ?)
           AND EXISTS (SELECT 1 FROM messages WHERE id = ? AND deleted_at IS NULL)",
    )
    .bind(id.to_string())
    .bind(&url)
    .bind(&url)
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if hidden == 0 {
        return Err(AppError::NotFound);
    }
    tx.commit().await?;
    publish(&s, id).await
}

/// Who may and how often: as for [`hide`]. A 404 when nothing hidden is left to show.
async fn show(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<Json<Message>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let (_, ch) = own_message(&s.db, me, id).await?;
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    // Read in the write: an edit can't change the links between here and the UPDATE.
    let content: Option<String> =
        sqlx::query_scalar("SELECT content FROM messages WHERE id = ? AND deleted_at IS NULL")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    let Some(content) = content else {
        return Err(AppError::NotFound); // deleted since the check above
    };
    crate::access::channel_live_in(&mut tx, ch.id).await?; // channel deleted since (#109)
    crate::access::open_in(&mut tx, ch.id).await?; // archived (#303): read-only
    // Only links still in the message: one edited out stays hidden in case it comes back.
    let links = super::links::extract(&content);
    let mut shown = 0;
    for url in &links {
        shown += sqlx::query(
            "UPDATE link_embeds SET hidden = 0 WHERE message_id = ? AND url = ? AND hidden = 1",
        )
        .bind(id.to_string())
        .bind(url)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    if shown == 0 {
        return Err(AppError::NotFound);
    }
    tx.commit().await?;
    publish(&s, id).await
}

/// The message as everyone now sees it, out as `MessageUpdated` and back to the caller.
async fn publish(s: &AppState, id: MessageId) -> AppResult<Json<Message>> {
    // Quotes and clips never have previews, so the neutral view is the caller's too.
    let message = load(&s.db, None, id).await?.ok_or(AppError::NotFound)?;
    s.hub
        .publish(
            &s.db,
            Event::MessageUpdated {
                message: message.clone(),
            },
        )
        .await;
    Ok(Json(message))
}
