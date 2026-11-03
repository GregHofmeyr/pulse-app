use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{patch, post};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ServerId, UserId, next_ulid};
use pulse_protocol::rest::{CreateSoundRequest, Sound, UpdateSoundRequest};
use pulse_protocol::sound::{MAX_BYTES, clean_name, parse_ogg_opus, valid_emoji};

use super::{bad_volume, for_server, load};
use crate::AppState;
use crate::auth::AuthUser;
use crate::db::{begin_write, now};
use crate::error::{AppError, AppResult};
use crate::media::{MediaKind, path_for, valid_hash};
use crate::servers::routes::is_server_member;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/servers/{id}/sounds", post(create).get(list))
        .route("/sounds/{id}", patch(update).delete(delete))
}

const VOLUME: std::ops::RangeInclusive<u16> = 10..=200;

fn upload_missing() -> AppError {
    AppError::BadRequest("that upload wasn't found; upload the sound again".into())
}

fn bad_name() -> AppError {
    AppError::BadRequest("sound names are 1-32 characters".into())
}

fn bad_emoji() -> AppError {
    AppError::BadRequest("a sound's emoji must be a single emoji".into())
}

/// The sound, if `me` may change it: members of its server only.
async fn editable(s: &AppState, me: UserId, id: &str) -> AppResult<Sound> {
    let sound = load(&s.db, id).await?.ok_or(AppError::NotFound)?;
    if !is_server_member(&s.db, sound.server_id, me).await? {
        return Err(AppError::Forbidden);
    }
    Ok(sound)
}

/// The stored file's length, read from the file itself rather than taken from the client.
async fn stored_duration(s: &AppState, hash: &str) -> AppResult<u32> {
    let path = path_for(&s.cfg.media_dir, hash).ok_or_else(upload_missing)?;
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) if b.len() <= MAX_BYTES => b,
        Ok(_) => return Err(upload_missing()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(upload_missing()),
        Err(e) => return Err(anyhow::Error::from(e).context("read sound").into()),
    };
    let parsed = parse_ogg_opus(&bytes).map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(parsed.duration_ms)
}

async fn list(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(server): Path<ServerId>,
) -> AppResult<Json<Vec<Sound>>> {
    if !is_server_member(&s.db, server, me).await? {
        return Err(AppError::Forbidden);
    }
    Ok(Json(for_server(&s.db, server).await?))
}

async fn create(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(server): Path<ServerId>,
    Json(req): Json<CreateSoundRequest>,
) -> AppResult<(StatusCode, Json<Sound>)> {
    if !s.limits.sound_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    crate::media::writes_ok(&s.cfg)?;
    if !is_server_member(&s.db, server, me).await? {
        return Err(AppError::Forbidden);
    }
    let name = clean_name(&req.name).ok_or_else(bad_name)?;
    if let Some(emoji) = &req.emoji
        && !valid_emoji(emoji)
    {
        return Err(bad_emoji());
    }
    if !VOLUME.contains(&req.volume) {
        return Err(bad_volume());
    }
    if !valid_hash(&req.media_hash) {
        return Err(upload_missing());
    }
    let id = next_ulid().to_string();
    // Checked under the write lock, so cleanup can't remove the media before the insert.
    let mut tx = begin_write(&s.db).await?;
    let is_sound: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM media WHERE hash = ? AND kind = ?")
            .bind(&req.media_hash)
            .bind(MediaKind::Sound.as_str())
            .fetch_optional(&mut *tx)
            .await?;
    if is_sound.is_none() {
        return Err(upload_missing());
    }
    let duration_ms = stored_duration(&s, &req.media_hash).await?;
    sqlx::query(
        "INSERT INTO sounds (id, server_id, name, emoji, media_hash, volume, duration_ms,
                             added_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(server.to_string())
    .bind(&name)
    .bind(&req.emoji)
    .bind(&req.media_hash)
    .bind(i64::from(req.volume))
    .bind(i64::from(duration_ms))
    .bind(me.to_string())
    .bind(now())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let sound = load(&s.db, &id).await?.ok_or(AppError::NotFound)?;
    s.hub
        .publish(
            &s.db,
            Event::SoundCreated {
                sound: sound.clone(),
            },
        )
        .await;
    Ok((StatusCode::CREATED, Json(sound)))
}

async fn update(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateSoundRequest>,
) -> AppResult<Json<Sound>> {
    if !s.limits.sound_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let old = editable(&s, me, &id).await?;
    let name = match &req.name {
        Some(n) => clean_name(n).ok_or_else(bad_name)?,
        None => old.name,
    };
    let emoji = match req.emoji {
        Some(Some(e)) if !valid_emoji(&e) => return Err(bad_emoji()),
        Some(e) => e,
        None => old.emoji,
    };
    let volume = match req.volume {
        Some(v) if !VOLUME.contains(&v) => return Err(bad_volume()),
        Some(v) => v,
        None => old.volume,
    };
    let changed = sqlx::query("UPDATE sounds SET name = ?, emoji = ?, volume = ? WHERE id = ?")
        .bind(&name)
        .bind(&emoji)
        .bind(i64::from(volume))
        .bind(&id)
        .execute(&s.db)
        .await?
        .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    let sound = load(&s.db, &id).await?.ok_or(AppError::NotFound)?;
    s.hub
        .publish(
            &s.db,
            Event::SoundUpdated {
                sound: sound.clone(),
            },
        )
        .await;
    Ok(Json(sound))
}

async fn delete(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    if !s.limits.sound_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let sound = editable(&s, me, &id).await?;
    // Favourites and usage go with it (cascade); the file waits for media cleanup.
    let changed = sqlx::query("DELETE FROM sounds WHERE id = ?")
        .bind(&id)
        .execute(&s.db)
        .await?
        .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    s.hub
        .publish(
            &s.db,
            Event::SoundDeleted {
                server_id: sound.server_id,
                sound_id: id,
            },
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}
