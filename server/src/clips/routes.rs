//! Posting a clip: an upload ticket, the client's PUT straight to storage, then confirm.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, post};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, ClipUploadId, MessageId, UserId};
use pulse_protocol::rest::{
    ChannelKind, Clip, ClipUpload, ClipUploadRequest, ConfirmClipRequest, Message, MessageKind,
};
use sqlx::SqlitePool;

use super::{
    CAPTION_MAX, CONFIRM_SLACK_MS, MAX_BYTES, MAX_DURATION_MS, MAX_SIDE, MIN_SIDE, PER_HOUR,
    URL_TTL, games, object_key, storage,
};
use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::{begin_write, now, stamp};
use crate::error::{AppError, AppResult};
use crate::media::MediaKind;
use crate::messages::{nonce, routes::load};
use crate::servers::routes::is_server_member;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/channels/{id}/clips/uploads", post(create_ticket))
        .route("/clip-uploads/{id}", delete(cancel))
        .route("/clip-uploads/{id}/confirm", post(confirm))
}

/// Clips `me` started in the last rolling hour: top-level messages in clips channels (deleted
/// ones included, since deleting removes the clip row) plus open tickets.
async fn started_this_hour(c: &mut sqlx::SqliteConnection, me: UserId) -> AppResult<i64> {
    let since = stamp(chrono::Utc::now() - chrono::Duration::hours(1));
    let posted: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages m JOIN channels c ON c.id = m.channel_id
         WHERE m.author_id = ? AND c.kind = 'clips' AND m.thread_root_id IS NULL
           AND m.created_at > ?",
    )
    .bind(me.to_string())
    .bind(&since)
    .fetch_one(&mut *c)
    .await?;
    let open: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM clip_uploads WHERE user_id = ? AND created_at > ?",
    )
    .bind(me.to_string())
    .bind(&since)
    .fetch_one(&mut *c)
    .await?;
    Ok(posted + open)
}

async fn create_ticket(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(channel): Path<ChannelId>,
    Json(req): Json<ClipUploadRequest>,
) -> AppResult<Json<ClipUpload>> {
    let r2 = storage(&s)?;
    let ch = channel_for(&s.db, me, channel).await?;
    if ch.kind != ChannelKind::Clips {
        return Err(AppError::BadRequest("this isn't a clips channel".into()));
    }
    match ch.server_id {
        Some(server) if is_server_member(&s.db, server, me).await? => {}
        _ => return Err(AppError::Forbidden),
    }
    if req.size == 0 {
        return Err(AppError::BadRequest("That clip is empty.".into()));
    }
    if req.size > MAX_BYTES {
        return Err(AppError::PayloadTooLarge(
            "Clips can be up to 200 MB.".into(),
        ));
    }
    if req.duration_ms == 0 || req.duration_ms > MAX_DURATION_MS {
        return Err(AppError::BadRequest("Clips can be up to 1:30.".into()));
    }
    crate::media::writes_ok(&s.cfg)?;

    let id = ClipUploadId::new();
    let key = crate::media::new_key();
    // Count and insert under the write lock, so two tickets can't both take the last slot.
    let mut tx = begin_write(&s.db).await?;
    if started_this_hour(&mut tx, me).await? >= PER_HOUR {
        return Err(AppError::TooManyRequests(Some(format!(
            "That's {PER_HOUR} clips this hour. Try again later."
        ))));
    }
    sqlx::query(
        "INSERT INTO clip_uploads (id, key, user_id, channel_id, size, duration_ms, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(&key)
    .bind(me.to_string())
    .bind(channel.to_string())
    .bind(req.size as i64)
    .bind(req.duration_ms)
    .bind(now())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let content_type = "video/mp4";
    let (url, expires_at) = r2.upload_url(&object_key(&key), req.size, content_type, URL_TTL);
    Ok(Json(ClipUpload {
        id,
        url: url.to_string(),
        content_type: content_type.into(),
        expires_at,
    }))
}

/// One of my tickets: `(key, channel, size)`. Anyone else's is a 404.
async fn my_ticket(
    db: &SqlitePool,
    me: UserId,
    id: ClipUploadId,
) -> AppResult<(String, Option<String>, i64)> {
    sqlx::query_as("SELECT key, channel_id, size FROM clip_uploads WHERE id = ? AND user_id = ?")
        .bind(id.to_string())
        .bind(me.to_string())
        .fetch_optional(db)
        .await?
        .ok_or(AppError::NotFound)
}

/// Drop the ticket and (best effort) its object.
async fn discard(s: &AppState, id: ClipUploadId, key: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM clip_uploads WHERE id = ?")
        .bind(id.to_string())
        .execute(&s.db)
        .await?;
    if let Some(r2) = &s.r2
        && let Err(e) = r2.delete(&object_key(key)).await
    {
        // The sweeper retries what a failed delete leaves behind.
        tracing::warn!(error = ?e, "couldn't delete a cancelled clip upload's object");
    }
    Ok(())
}

async fn cancel(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ClipUploadId>,
) -> AppResult<StatusCode> {
    let (key, _, _) = my_ticket(&s.db, me, id).await?;
    discard(&s, id, &key).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn bad(msg: &str) -> AppError {
    AppError::BadRequest(msg.into())
}

async fn confirm(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ClipUploadId>,
    Json(req): Json<ConfirmClipRequest>,
) -> AppResult<Json<Message>> {
    let r2 = storage(&s)?;
    nonce::check(req.nonce.as_deref())?;
    // A retry of a confirm that already landed gets the same clip back (even with no ticket).
    if let Some(existing) = prior(&s, me, req.nonce.as_deref()).await? {
        return replay(&s, me, id, existing).await;
    }

    let (key, channel, ticket_size) = match my_ticket(&s.db, me, id).await {
        Ok(t) => t,
        // A twin of this request may have committed (spending the ticket) since the check above.
        Err(AppError::NotFound) => {
            return match prior(&s, me, req.nonce.as_deref()).await? {
                Some(existing) => replay(&s, me, id, existing).await,
                None => Err(AppError::NotFound),
            };
        }
        Err(e) => return Err(e),
    };
    let channel: ChannelId = channel
        .ok_or(AppError::NotFound)?
        .parse()
        .map_err(anyhow::Error::from)?;
    let ch = channel_for(&s.db, me, channel).await?;
    if ch.kind != ChannelKind::Clips {
        return Err(bad("this isn't a clips channel"));
    }
    let server = ch.server_id.ok_or(AppError::NotFound)?;

    let size = r2
        .head(&object_key(&key))
        .await?
        .ok_or_else(|| AppError::Conflict("The upload didn't arrive. Try again.".into()))?;
    if size as i64 != ticket_size {
        discard(&s, id, &key).await?;
        return Err(bad("The upload didn't match. Try again."));
    }
    if req.duration_ms == 0 || req.duration_ms > MAX_DURATION_MS + CONFIRM_SLACK_MS {
        return Err(bad("Clips can be up to 1:30."));
    }
    let side = MIN_SIDE..=MAX_SIDE;
    if !side.contains(&req.width) || !side.contains(&req.height) {
        return Err(bad("That clip's size isn't supported."));
    }
    let thumb: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM media WHERE hash = ? AND kind = 'clip_thumb' AND uploaded_by = ?",
    )
    .bind(&req.thumb_hash)
    .bind(me.to_string())
    .fetch_optional(&s.db)
    .await?;
    if thumb.is_none() {
        return Err(bad("The preview image is missing. Try again."));
    }
    if let Some(game) = req.game_id
        && !games::in_server(&s.db, server, game).await?
    {
        return Err(bad("That game isn't in this server."));
    }
    let caption = req.caption.trim().to_string();
    if caption.chars().count() > CAPTION_MAX {
        return Err(bad("Captions can be up to 300 characters."));
    }
    if !is_server_member(&s.db, server, me).await? {
        return Err(AppError::Forbidden);
    }
    crate::media::writes_ok(&s.cfg)?;

    let msg = Message {
        id: MessageId::new(),
        channel_id: channel,
        author_id: Some(me),
        kind: MessageKind::Normal,
        content: caption,
        reply_to_id: None,
        created_at: now(),
        edited_at: None,
        deleted: false,
        mentions: vec![],
        reactions: vec![],
        thread_root_id: None,
        quote: None,
        clip: Some(Clip {
            thumb_hash: req.thumb_hash.clone(),
            duration_ms: req.duration_ms,
            width: req.width,
            height: req.height,
            size,
            game_id: req.game_id,
            reply_count: 0,
            last_reply_id: None,
            replies_seen: None,
        }),
        qoty_card: None,
    };
    // As in `send`: publishes (and the absolute unread counts) go out in commit order.
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    if let Some(existing) = nonce::existing(&mut tx, me, req.nonce.as_deref()).await? {
        tx.rollback().await?;
        return replay(&s, me, id, existing.to_string()).await;
    }
    // The ticket may have been cancelled or swept since the early check.
    let live = sqlx::query("DELETE FROM clip_uploads WHERE id = ? AND user_id = ?")
        .bind(id.to_string())
        .bind(me.to_string())
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if live == 0 {
        return Err(AppError::NotFound);
    }
    crate::media::put_remote(
        &mut tx,
        MediaKind::Clip,
        &key,
        size,
        req.width,
        req.height,
        me,
    )
    .await?;
    sqlx::query(
        "INSERT INTO messages (id, channel_id, author_id, kind, content, created_at, nonce)
         VALUES (?, ?, ?, 'normal', ?, ?, ?)",
    )
    .bind(msg.id.to_string())
    .bind(channel.to_string())
    .bind(me.to_string())
    .bind(&msg.content)
    .bind(&msg.created_at)
    .bind(&req.nonce)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO clips (message_id, video_key, thumb_hash, duration_ms, width, height, game_id)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(msg.id.to_string())
    .bind(&key)
    .bind(&req.thumb_hash)
    .bind(req.duration_ms)
    .bind(req.width)
    .bind(req.height)
    .bind(req.game_id.map(|g| g.to_string()))
    .execute(&mut *tx)
    .await?;
    let moved = crate::reads::advance(&mut tx, me, channel, msg.id).await?;
    tx.commit().await?;

    s.hub
        .publish(
            &s.db,
            Event::MessageCreated {
                message: msg.clone(),
                nonce: req.nonce,
            },
        )
        .await;
    if moved
        && let Some((unread, mentions)) =
            crate::reads::counts_after_commit(&s.db, me, channel).await
    {
        s.hub
            .publish(
                &s.db,
                Event::ReadStateUpdated {
                    user_id: me,
                    channel_id: channel,
                    last_read_message_id: Some(msg.id),
                    mentions: Some(mentions),
                    unread: Some(unread),
                },
            )
            .await;
    }
    Ok(Json(msg))
}

/// The id of my message that used this nonce, if any.
async fn prior(s: &AppState, me: UserId, nonce: Option<&str>) -> AppResult<Option<String>> {
    let Some(n) = nonce else { return Ok(None) };
    Ok(
        sqlx::query_scalar("SELECT id FROM messages WHERE author_id = ? AND nonce = ?")
            .bind(me.to_string())
            .bind(n)
            .fetch_optional(&s.db)
            .await?,
    )
}

/// The clip an earlier confirm with this nonce posted (nothing new is published). The nonce must
/// belong to one of my clips, and to this ticket's channel while the ticket still exists.
async fn replay(
    s: &AppState,
    me: UserId,
    ticket: ClipUploadId,
    id: String,
) -> AppResult<Json<Message>> {
    let (channel, is_clip): (String, bool) = sqlx::query_as(
        "SELECT channel_id, EXISTS (SELECT 1 FROM clips WHERE message_id = messages.id)
         FROM messages WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(&s.db)
    .await?;
    let ticket_channel: Option<Option<String>> =
        sqlx::query_scalar("SELECT channel_id FROM clip_uploads WHERE id = ? AND user_id = ?")
            .bind(ticket.to_string())
            .bind(me.to_string())
            .fetch_optional(&s.db)
            .await?;
    let other_channel = ticket_channel.is_some_and(|c| c.as_deref() != Some(channel.as_str()));
    if !is_clip || other_channel {
        return Err(AppError::Conflict("That request was already used.".into()));
    }
    let id: MessageId = id.parse().map_err(anyhow::Error::from)?;
    load(&s.db, Some(me), id)
        .await?
        .map(Json)
        .ok_or(AppError::NotFound)
}
