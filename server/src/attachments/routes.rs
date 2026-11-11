//! Starting an upload (a pending row and a signed PUT), cancelling one, and the signed links to a
//! message's files.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use pulse_protocol::ids::{AttachmentId, ChannelId, MessageId};
use pulse_protocol::rest::{
    AttachmentKind, AttachmentUpload, AttachmentUrl, ChannelKind, StartAttachmentRequest,
};

use super::{
    MAX_BYTES, MAX_DURATION_MS, MAX_SIDE, OPEN_PER_USER, URL_TTL, clean_name, kind_str, object_key,
    storage, stored_type,
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
        .route("/messages/{id}/attachments/urls", get(urls))
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
    // The client's word for how a picture or video is shaped: only kept when plausible.
    let side = |v: Option<u32>| v.filter(|v| (1..=MAX_SIDE).contains(v));
    let length = req.duration_ms.filter(|d| *d <= MAX_DURATION_MS);
    let (content_type, kind) = stored_type(&req.content_type)?;
    crate::media::writes_ok(&s.cfg)?;

    // Sizes and length are the client's word until send checks the stored object; a plain file
    // has none.
    let media = kind != AttachmentKind::File;
    let id = AttachmentId::new();
    let key = crate::media::new_key();
    // Count and insert under the write lock, so two starts can't both take the last slot.
    let mut tx = begin_write(&s.db).await?;
    // Only uploads a send could still take count: not one whose channel is gone, nor one past
    // its time (the sweep removes those).
    let open: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM attachments WHERE user_id = ? AND message_id IS NULL
           AND channel_id IS NOT NULL AND created_at >= ?",
    )
    .bind(me.to_string())
    .bind(crate::db::stamp(
        chrono::Utc::now() - chrono::Duration::hours(super::PENDING_TTL_HOURS),
    ))
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
    .bind(side(req.width).filter(|_| media))
    .bind(side(req.height).filter(|_| media))
    .bind(length.filter(|_| media))
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

/// Drop one of my pending uploads: the object first (best effort), then the row. A sent one is part
/// of its message; anyone else's, or one already gone, is a 404.
async fn cancel(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<AttachmentId>,
) -> AppResult<StatusCode> {
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT key, expires_at FROM attachments WHERE id = ? AND user_id = ? AND message_id IS NULL",
    )
    .bind(id.to_string())
    .bind(me.to_string())
    .fetch_optional(&s.db)
    .await?;
    let (key, expires_at) = row.ok_or(AppError::NotFound)?;
    if let Some(r2) = &s.r2
        && let Err(e) = r2.delete(&object_key(&key, expires_at.is_some())).await
    {
        // The row stays: the sweep deletes unsent uploads (object and row) once they're old.
        tracing::warn!(error = ?e, "couldn't delete a cancelled attachment upload's object");
        return Ok(StatusCode::NO_CONTENT);
    }
    // Guarded: a send that took it meanwhile keeps it (its object is then gone: the sender sees a
    // file that won't load, which only a cancel racing its own send can cause).
    sqlx::query("DELETE FROM attachments WHERE id = ? AND message_id IS NULL")
        .bind(id.to_string())
        .execute(&s.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Signed links to every file of a message, in order; an expired temporary file has none. Every
/// call signs again (links are cached client-side until shortly before they expire).
async fn urls(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<Json<Vec<AttachmentUrl>>> {
    if !s.limits.clip_url_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let r2 = storage(&s)?;
    let channel: String =
        sqlx::query_scalar("SELECT channel_id FROM messages WHERE id = ? AND deleted_at IS NULL")
            .bind(id.to_string())
            .fetch_optional(&s.db)
            .await?
            .ok_or(AppError::NotFound)?;
    let ch = channel_for(&s.db, me, channel.parse().map_err(anyhow::Error::from)?).await?;
    if let Some(server) = ch.server_id
        && !is_server_member(&s.db, server, me).await?
    {
        return Err(AppError::Forbidden);
    }
    let rows: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id, key, expires_at, purged_at FROM attachments WHERE message_id = ?
         ORDER BY position",
    )
    .bind(id.to_string())
    .fetch_all(&s.db)
    .await?;
    let now = crate::db::now();
    rows.into_iter()
        .map(|(att, key, expires_at, purged_at)| {
            let expired =
                purged_at.is_some() || expires_at.as_deref().is_some_and(|e| e <= now.as_str());
            let (url, at) = if expired {
                (None, None)
            } else {
                let (url, at) = r2.download_url(&object_key(&key, expires_at.is_some()), URL_TTL);
                (Some(url.to_string()), Some(at))
            };
            Ok(AttachmentUrl {
                id: att.parse().map_err(anyhow::Error::from)?,
                url,
                expires_at: at,
            })
        })
        .collect::<AppResult<Vec<_>>>()
        .map(Json)
}
