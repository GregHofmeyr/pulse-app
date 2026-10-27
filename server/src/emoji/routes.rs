use axum::extract::{Path, State};
use axum::routing::{patch, post};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ServerId, next_ulid};
use pulse_protocol::rest::{CreateEmojiRequest, CustomEmoji, RenameEmojiRequest};

use super::{MAX_PER_SERVER, clean_name, load};
use crate::AppState;
use crate::auth::AuthUser;
use crate::db::{begin_write, now};
use crate::error::{AppError, AppResult};
use crate::limits::caps;
use crate::media::{MediaKind, valid_hash};
use crate::servers::routes::is_server_member;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/servers/{id}/emoji", post(create))
        .route("/emoji/{id}", patch(rename).delete(delete))
}

fn upload_missing() -> AppError {
    AppError::BadRequest("that upload wasn't found; upload the image again".into())
}

fn taken(e: sqlx::Error) -> AppError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => {
            AppError::Conflict("this server already has an emoji with that name".into())
        }
        // The media row went between the check and the insert.
        sqlx::Error::Database(d) if d.is_foreign_key_violation() => upload_missing(),
        _ => e.into(),
    }
}

/// The emoji, if `me` may change it: members of its server only. Deleted or missing ones are
/// gone (404) for everyone.
async fn editable(
    s: &AppState,
    me: pulse_protocol::ids::UserId,
    id: &str,
) -> AppResult<CustomEmoji> {
    let emoji = load(&s.db, id).await?.ok_or(AppError::NotFound)?;
    if emoji.hash.is_none() {
        return Err(AppError::NotFound);
    }
    if !is_server_member(&s.db, emoji.server_id, me).await? {
        return Err(AppError::Forbidden);
    }
    Ok(emoji)
}

async fn create(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(server): Path<ServerId>,
    Json(req): Json<CreateEmojiRequest>,
) -> AppResult<Json<CustomEmoji>> {
    if !s.limits.emoji_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    crate::media::writes_ok(&s.cfg)?;
    if !is_server_member(&s.db, server, me).await? {
        return Err(AppError::Forbidden);
    }
    let name = clean_name(&req.name)?;
    if !valid_hash(&req.media_hash) {
        return Err(upload_missing());
    }
    let id = next_ulid().to_string();
    // Checked under the write lock, so cleanup can't remove the media before the insert.
    let mut tx = begin_write(&s.db).await?;
    let media: Option<(i64,)> =
        sqlx::query_as("SELECT frames FROM media WHERE hash = ? AND kind = ?")
            .bind(&req.media_hash)
            .bind(MediaKind::Emoji.as_str())
            .fetch_optional(&mut *tx)
            .await?;
    let Some((frames,)) = media else {
        return Err(upload_missing());
    };
    let live: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM custom_emoji WHERE server_id = ? AND deleted_at IS NULL",
    )
    .bind(server.to_string())
    .fetch_one(&mut *tx)
    .await?;
    if live >= MAX_PER_SERVER {
        return Err(AppError::BadRequest(format!(
            "this server has {MAX_PER_SERVER} emoji, the most it can hold; delete one first"
        )));
    }
    // Deleted emoji keep their row (old messages keep their `:name:`), so create-and-delete
    // would otherwise grow the table without bound.
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM custom_emoji WHERE server_id = ?")
        .bind(server.to_string())
        .fetch_one(&mut *tx)
        .await?;
    if rows >= caps::EMOJI_ROWS_PER_SERVER {
        return Err(AppError::BadRequest(format!(
            "this server has had {} emoji, the most it can ever add",
            caps::EMOJI_ROWS_PER_SERVER
        )));
    }
    sqlx::query(
        "INSERT INTO custom_emoji (id, server_id, name, media_hash, animated, added_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(server.to_string())
    .bind(name)
    .bind(&req.media_hash)
    .bind(i64::from(frames > 1))
    .bind(me.to_string())
    .bind(now())
    .execute(&mut *tx)
    .await
    .map_err(taken)?;
    tx.commit().await?;
    let emoji = load(&s.db, &id).await?.ok_or(AppError::NotFound)?;
    s.hub
        .publish(
            &s.db,
            Event::EmojiCreated {
                emoji: emoji.clone(),
            },
        )
        .await;
    Ok(Json(emoji))
}

async fn rename(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
    Json(req): Json<RenameEmojiRequest>,
) -> AppResult<Json<CustomEmoji>> {
    if !s.limits.emoji_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    let name = clean_name(&req.name)?;
    editable(&s, me, &id).await?;
    let changed =
        sqlx::query("UPDATE custom_emoji SET name = ? WHERE id = ? AND deleted_at IS NULL")
            .bind(name)
            .bind(&id)
            .execute(&s.db)
            .await
            .map_err(taken)?
            .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    let emoji = load(&s.db, &id).await?.ok_or(AppError::NotFound)?;
    s.hub
        .publish(
            &s.db,
            Event::EmojiUpdated {
                emoji: emoji.clone(),
            },
        )
        .await;
    Ok(Json(emoji))
}

async fn delete(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<CustomEmoji>> {
    if !s.limits.emoji_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    editable(&s, me, &id).await?;
    // Dropping the media reference lets cleanup reclaim the file; the row keeps its name.
    let changed = sqlx::query(
        "UPDATE custom_emoji SET deleted_at = ?, media_hash = NULL WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(now())
    .bind(&id)
    .execute(&s.db)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    let emoji = load(&s.db, &id).await?.ok_or(AppError::NotFound)?;
    s.hub
        .publish(
            &s.db,
            Event::EmojiDeleted {
                emoji: emoji.clone(),
            },
        )
        .await;
    Ok(Json(emoji))
}
