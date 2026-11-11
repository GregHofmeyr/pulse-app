//! Starting an upload (a pending row and a signed PUT) and cancelling one.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, post};
use axum::{Json, Router};
use pulse_protocol::ids::{AttachmentId, ChannelId};
use pulse_protocol::rest::{AttachmentKind, AttachmentUpload, ChannelKind, StartAttachmentRequest};

use super::{
    MAX_BYTES, OPEN_PER_USER, URL_TTL, clean_name, kind_str, object_key, storage, stored_type,
};
use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::{begin_write, now};
use crate::error::{AppError, AppResult};
use crate::servers::routes::is_server_member;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/channels/{id}/attachments", post(start))
        .route("/attachment-uploads/{id}", delete(cancel))
}

fn bad(msg: &str) -> AppError {
    AppError::BadRequest(msg.into())
}

async fn start(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(channel): Path<ChannelId>,
    Json(req): Json<StartAttachmentRequest>,
) -> AppResult<Json<AttachmentUpload>> {
    if !s.limits.attachment_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let r2 = storage(&s)?;
    let ch = channel_for(&s.db, me, channel).await?;
    // Quotes and clips channels have their own media. A DM or group is checked by `channel_for`
    // (only its people see it); a server channel needs membership.
    match ch.kind {
        ChannelKind::Dm | ChannelKind::Group => {}
        ChannelKind::Text | ChannelKind::Voice => match ch.server_id {
            Some(server) if is_server_member(&s.db, server, me).await? => {}
            _ => return Err(AppError::Forbidden),
        },
        _ => return Err(bad("Files can't be posted here.")),
    }
    if req.temp_days.is_some() {
        return Err(bad("Temporary files aren't available yet."));
    }
    if req.size == 0 {
        return Err(bad("That file is empty."));
    }
    if req.size > MAX_BYTES {
        return Err(AppError::PayloadTooLarge(
            "Files can be up to 50 MB.".into(),
        ));
    }
    let name = clean_name(&req.name).ok_or_else(|| bad("That file needs a name."))?;
    let (content_type, kind) = stored_type(&req.content_type)?;
    crate::media::writes_ok(&s.cfg)?;

    // Sizes and length are the client's word until send checks the stored object; a plain file
    // has none.
    let media = kind != AttachmentKind::File;
    let id = AttachmentId::new();
    let key = crate::media::new_key();
    // Count and insert under the write lock, so two starts can't both take the last slot.
    let mut tx = begin_write(&s.db).await?;
    let open: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM attachments WHERE user_id = ? AND message_id IS NULL",
    )
    .bind(me.to_string())
    .fetch_one(&mut *tx)
    .await?;
    if open >= OPEN_PER_USER {
        return Err(AppError::TooManyRequests(Some(
            "Too many uploads in progress.".into(),
        )));
    }
    sqlx::query(
        "INSERT INTO attachments
             (id, key, user_id, channel_id, name, content_type, kind, size, width, height,
              duration_ms, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(&key)
    .bind(me.to_string())
    .bind(channel.to_string())
    .bind(&name)
    .bind(content_type)
    .bind(kind_str(kind))
    .bind(req.size as i64)
    .bind(req.width.filter(|_| media))
    .bind(req.height.filter(|_| media))
    .bind(req.duration_ms.filter(|_| media))
    .bind(now())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let (url, expires_at) =
        r2.upload_url(&object_key(&key, false), req.size, content_type, URL_TTL);
    Ok(Json(AttachmentUpload {
        id,
        url: Some(url.to_string()),
        content_type: content_type.into(),
        expires_at,
    }))
}

/// Drop one of my pending uploads, row then (best effort) object. A sent one is part of its
/// message; anyone else's, or one already gone, is a 404.
async fn cancel(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<AttachmentId>,
) -> AppResult<StatusCode> {
    // One statement, so a send that attaches it at the same moment either wins or finds it gone.
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "DELETE FROM attachments WHERE id = ? AND user_id = ? AND message_id IS NULL
         RETURNING key, expires_at",
    )
    .bind(id.to_string())
    .bind(me.to_string())
    .fetch_optional(&s.db)
    .await?;
    let (key, expires_at) = row.ok_or(AppError::NotFound)?;
    if let Some(r2) = &s.r2
        && let Err(e) = r2.delete(&object_key(&key, expires_at.is_some())).await
    {
        // The row is gone, so no sweep will retry this: the object stays until removed by hand.
        tracing::warn!(error = ?e, "couldn't delete a cancelled attachment upload's object");
    }
    Ok(StatusCode::NO_CONTENT)
}
