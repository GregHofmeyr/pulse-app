//! Starting an upload (a pending row and a signed PUT), cancelling one, and the signed links to a
//! message's files.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use pulse_protocol::ids::UserId;
use pulse_protocol::ids::{AttachmentId, ChannelId, MessageId};
use pulse_protocol::rest::{
    AttachmentKind, AttachmentUpload, AttachmentUrl, ChannelKind, MultipartInfo, PartUrl,
    PartUrlsRequest, StartAttachmentRequest, UploadedPart, attachment_limits,
};
use sqlx::SqlitePool;

use super::{
    MAX_BYTES, MAX_DURATION_MS, MAX_SIDE, OPEN_PER_USER, Stored, URL_TTL, clean_name, kind_str,
    object_key, part_count, part_len, storage, stored_type,
};
use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::begin_write;
use crate::error::{AppError, AppResult};
use crate::servers::routes::is_server_member;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/channels/{id}/attachments", post(start))
        .route("/attachment-uploads/{id}", delete(cancel))
        .route("/messages/{id}/attachments/urls", get(urls))
        .route(
            "/attachment-uploads/{id}/parts",
            post(part_urls).get(uploaded_parts),
        )
        .route("/attachment-uploads/{id}/complete", post(complete))
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
    if req.size == 0 {
        return Err(bad("That file is empty."));
    }
    // Up to 50 MB a file is kept; bigger ones (up to 2 GB) are temporary sends that expire.
    let temp_days = match req.temp_days {
        None if req.size > MAX_BYTES => {
            return Err(AppError::PayloadTooLarge(
                "Files can be up to 50 MB.".into(),
            ));
        }
        None => None,
        Some(d) if !attachment_limits::TEMP_DAYS.contains(&d) => {
            return Err(bad("Temporary files expire in 1 or 7 days."));
        }
        Some(_) if req.size <= MAX_BYTES => {
            return Err(bad("Files up to 50 MB are kept: send it as a normal file."));
        }
        Some(_) if req.size > attachment_limits::TEMP_MAX_BYTES => {
            return Err(AppError::PayloadTooLarge("Files can be up to 2 GB.".into()));
        }
        Some(d) => Some(d),
    };
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
    crate::access::channel_live_in(&mut tx, channel).await?; // channel deleted since (#109)
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
    // The server-wide room for big files: everything temporary not yet deleted, sent or not.
    let now_at = chrono::Utc::now();
    let expires_at = temp_days.map(|d| crate::db::stamp(now_at + chrono::Duration::days(d.into())));
    if temp_days.is_some() {
        let live: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(size), 0) FROM attachments
             WHERE expires_at IS NOT NULL AND purged_at IS NULL",
        )
        .fetch_one(&mut *tx)
        .await?;
        // One person's unsent big files can't hold the room for everyone (starts reserve it).
        let mine: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(size), 0) FROM attachments
             WHERE user_id = ? AND message_id IS NULL AND expires_at IS NOT NULL
               AND channel_id IS NOT NULL AND created_at >= ?",
        )
        .bind(me.to_string())
        .bind(crate::db::stamp(
            now_at - chrono::Duration::hours(super::PENDING_TTL_HOURS),
        ))
        .fetch_one(&mut *tx)
        .await?;
        if mine as u64 + req.size > attachment_limits::TEMP_PENDING_PER_USER {
            return Err(AppError::TooManyRequests(Some(
                "Finish or cancel your other big uploads first.".into(),
            )));
        }
        if live as u64 + req.size > attachment_limits::TEMP_CAP_BYTES {
            return Err(AppError::Conflict(
                "This server's space for big files is full. Try again in a day or two.".into(),
            ));
        }
    }
    sqlx::query(
        "INSERT INTO attachments
             (id, key, user_id, channel_id, name, content_type, kind, size, width, height,
              duration_ms, expires_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    .bind(&expires_at)
    .bind(crate::db::stamp(now_at))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    if temp_days.is_some() {
        // A big file goes up in parts. Started after the commit (no store call under the write
        // lock); a failure takes the row back out.
        let object = object_key(&key, true);
        let upload = match r2.create_multipart(&object, content_type).await {
            Ok(u) => u,
            Err(e) => {
                sqlx::query("DELETE FROM attachments WHERE id = ?")
                    .bind(id.to_string())
                    .execute(&s.db)
                    .await?;
                return Err(e);
            }
        };
        sqlx::query("UPDATE attachments SET upload_id = ? WHERE id = ?")
            .bind(&upload)
            .bind(id.to_string())
            .execute(&s.db)
            .await?;
        return Ok(Json(AttachmentUpload {
            id,
            url: None,
            content_type: content_type.into(),
            expires_at: crate::db::stamp(
                now_at + chrono::Duration::from_std(URL_TTL).map_err(anyhow::Error::from)?,
            ),
            multipart: Some(MultipartInfo {
                part_size: attachment_limits::PART_SIZE,
                parts: part_count(req.size),
            }),
        }));
    }
    let (url, expires_at) =
        r2.upload_url(&object_key(&key, false), req.size, content_type, URL_TTL);
    Ok(Json(AttachmentUpload {
        id,
        url: Some(url.to_string()),
        content_type: content_type.into(),
        expires_at,
        multipart: None,
    }))
}

/// Drop one of my pending uploads: the object first (best effort), then the row. A sent one is part
/// of its message; anyone else's, or one already gone, is a 404.
async fn cancel(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<AttachmentId>,
) -> AppResult<StatusCode> {
    let row: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT key, expires_at, upload_id FROM attachments
         WHERE id = ? AND user_id = ? AND message_id IS NULL",
    )
    .bind(id.to_string())
    .bind(me.to_string())
    .fetch_optional(&s.db)
    .await?;
    let (key, expires_at, upload_id) = row.ok_or(AppError::NotFound)?;
    let object = object_key(&key, expires_at.is_some());
    let gone = match (&s.r2, &upload_id) {
        (None, _) => Ok(()),
        (Some(r2), Some(upload)) => match r2.abort_multipart(&object, upload).await {
            // a completed join whose bookkeeping failed leaves an object too
            Ok(()) => r2.delete(&object).await,
            Err(e) => Err(e),
        },
        (Some(r2), None) => r2.delete(&object).await,
    };
    if let Err(e) = gone {
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

/// One of my pending big uploads: its stored object, upload id and size.
async fn my_multipart(
    db: &SqlitePool,
    me: UserId,
    id: AttachmentId,
) -> AppResult<(Stored, String, u64)> {
    let row: Option<(String, Option<String>, Option<String>, i64)> = sqlx::query_as(
        "SELECT key, expires_at, upload_id, size FROM attachments
         WHERE id = ? AND user_id = ? AND message_id IS NULL",
    )
    .bind(id.to_string())
    .bind(me.to_string())
    .fetch_optional(db)
    .await?;
    let (key, expires_at, upload_id, size) = row.ok_or(AppError::NotFound)?;
    let upload = upload_id.ok_or(AppError::NotFound)?;
    Ok((
        Stored {
            key,
            temp: expires_at.is_some(),
        },
        upload,
        size as u64,
    ))
}

/// Signed PUTs for some parts of a big upload (each for exactly its length).
async fn part_urls(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<AttachmentId>,
    Json(req): Json<PartUrlsRequest>,
) -> AppResult<Json<Vec<PartUrl>>> {
    if !s.limits.attachment_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let r2 = storage(&s)?;
    let (stored, upload, size) = my_multipart(&s.db, me, id).await?;
    if req.parts.is_empty() || req.parts.len() > attachment_limits::PARTS_PER_ASK {
        return Err(bad("Ask for 1 to 16 parts at a time."));
    }
    let count = part_count(size);
    req.parts
        .iter()
        .map(|&part| {
            if part == 0 || part > count {
                return Err(bad("That part isn't in this upload."));
            }
            let len = part_len(size, part);
            let (url, _) = r2.part_url(&stored.object_key(), &upload, part, len, URL_TTL);
            Ok(PartUrl {
                part,
                url: url.to_string(),
                size: len,
            })
        })
        .collect::<AppResult<Vec<_>>>()
        .map(Json)
}

/// The parts the store has so far (a resumed upload sends only the rest).
async fn uploaded_parts(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<AttachmentId>,
) -> AppResult<Json<Vec<UploadedPart>>> {
    let r2 = storage(&s)?;
    let (stored, upload, _) = my_multipart(&s.db, me, id).await?;
    let parts = r2
        .list_parts(&stored.object_key(), &upload)
        .await?
        .ok_or_else(|| AppError::Conflict("That upload expired. Try again.".into()))?;
    Ok(Json(
        parts
            .into_iter()
            .map(|(part, _, size)| UploadedPart { part, size })
            .collect(),
    ))
}

/// Join a big upload's parts once all are stored, then check the object. After this it can be sent.
async fn complete(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<AttachmentId>,
) -> AppResult<StatusCode> {
    let r2 = storage(&s)?;
    // A retry after a lost answer: already joined (no upload id left) and whole is done.
    let joined: Option<(String, Option<String>, i64)> = sqlx::query_as(
        "SELECT key, expires_at, size FROM attachments
         WHERE id = ? AND user_id = ? AND message_id IS NULL AND upload_id IS NULL",
    )
    .bind(id.to_string())
    .bind(me.to_string())
    .fetch_optional(&s.db)
    .await?;
    if let Some((key, expires_at, size)) = joined
        && expires_at.is_some()
        && r2.head(&object_key(&key, true)).await? == Some(size as u64)
    {
        return Ok(StatusCode::NO_CONTENT);
    }
    let (stored, upload, size) = my_multipart(&s.db, me, id).await?;
    let parts = r2
        .list_parts(&stored.object_key(), &upload)
        .await?
        .ok_or_else(|| AppError::Conflict("That upload expired. Try again.".into()))?;
    let count = part_count(size);
    let whole = parts.len() == count as usize
        && parts
            .iter()
            .enumerate()
            .all(|(i, (n, _, len))| *n as usize == i + 1 && *len == part_len(size, *n));
    if !whole {
        return Err(AppError::Conflict("The upload isn't finished yet.".into()));
    }
    let etags: Vec<String> = parts.into_iter().map(|(_, etag, _)| etag).collect();
    r2.complete_multipart(&stored.object_key(), &upload, &etags)
        .await?;
    if r2.head(&stored.object_key()).await? != Some(size) {
        if let Err(e) = r2.delete(&stored.object_key()).await {
            tracing::warn!(error = ?e, "couldn't delete a mismatched upload");
        }
        sqlx::query("DELETE FROM attachments WHERE id = ? AND message_id IS NULL")
            .bind(id.to_string())
            .execute(&s.db)
            .await?;
        return Err(AppError::Conflict(
            "The upload didn't match. Try again.".into(),
        ));
    }
    // Whole now: a send may take it.
    sqlx::query("UPDATE attachments SET upload_id = NULL WHERE id = ? AND user_id = ?")
        .bind(id.to_string())
        .bind(me.to_string())
        .execute(&s.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
