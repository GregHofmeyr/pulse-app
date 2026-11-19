use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get};
use axum::{Json, Router};
use pulse_protocol::rest::{Gif, GifConfig, GifFavouritesQuery, gif_limits};

use super::{clean, valid_id};
use crate::AppState;
use crate::auth::AuthUser;
use crate::db::{begin_write, now};
use crate::error::{AppError, AppResult};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/gifs/config", get(config))
        .route("/gifs/favourites", get(favourites).post(add_favourite))
        .route("/gifs/favourites/{id}", delete(remove_favourite))
}

fn limited(s: &AppState, me: impl ToString) -> AppResult<()> {
    if s.limits.gif_user.hit(&me.to_string()) {
        Ok(())
    } else {
        Err(AppError::TooManyRequests(None))
    }
}

/// The key the app searches GIPHY with (GIPHY wants requests straight from the app), only for signed-in people.
async fn config(State(s): State<AppState>, AuthUser(me): AuthUser) -> AppResult<Json<GifConfig>> {
    limited(&s, me)?;
    Ok(Json(GifConfig {
        api_key: s.cfg.giphy.api_key.clone(),
        rating: s.cfg.giphy.rating.clone(),
    }))
}

/// Your favourites, newest first: favourite GIFs, or with `with_stickers` your favourite stickers too (#498), each
/// marked. Plain, it's GIFs only, as older apps (which draw every favourite as a GIF) expect.
async fn favourites(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Query(q): Query<GifFavouritesQuery>,
) -> AppResult<Json<Vec<Gif>>> {
    let rows: Vec<(String, String, i64, i64, bool)> = sqlx::query_as(
        "SELECT gif_id, title, width, height, sticker FROM gif_favourites WHERE user_id = ? AND (sticker = 0 OR ?)
         ORDER BY created_at DESC, rowid DESC",
    )
    .bind(me.to_string())
    .bind(q.with_stickers)
    .fetch_all(&s.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, title, width, height, sticker)| Gif {
                id,
                title,
                width: u32::try_from(width).unwrap_or(1),
                height: u32::try_from(height).unwrap_or(1),
                sticker,
            })
            .collect(),
    ))
}

/// Star a GIF, or a sticker (`sticker`, #498: its own list, with its own cap). Starring one already starred changes
/// nothing.
async fn add_favourite(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(gif): Json<Gif>,
) -> AppResult<StatusCode> {
    limited(&s, me)?;
    let gif = clean(gif)?;
    let mut tx = begin_write(&s.db).await?;
    // Counted under the write lock, so two stars at once can't both slip under the cap.
    let (count, have): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(sticker = ?), 0), COALESCE(SUM(gif_id = ?), 0) FROM gif_favourites WHERE user_id = ?",
    )
    .bind(gif.sticker)
    .bind(&gif.id)
    .bind(me.to_string())
    .fetch_one(&mut *tx)
    .await?;
    if have > 0 {
        return Ok(StatusCode::NO_CONTENT);
    }
    if count >= gif_limits::FAVOURITES_PER_PERSON {
        return Err(AppError::BadRequest(format!(
            "You can keep {} favourite {}. Unstar one to make room.",
            gif_limits::FAVOURITES_PER_PERSON,
            if gif.sticker { "stickers" } else { "GIFs" }
        )));
    }
    sqlx::query(
        "INSERT INTO gif_favourites (user_id, gif_id, title, width, height, sticker, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(me.to_string())
    .bind(&gif.id)
    .bind(&gif.title)
    .bind(i64::from(gif.width))
    .bind(i64::from(gif.height))
    .bind(gif.sticker)
    .bind(now())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Unstar a GIF or a sticker. Unstarring one that isn't starred changes nothing.
async fn remove_favourite(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    limited(&s, me)?;
    if !valid_id(&id) {
        return Err(AppError::BadRequest("that isn't a GIPHY GIF id".into()));
    }
    sqlx::query("DELETE FROM gif_favourites WHERE user_id = ? AND gif_id = ?")
        .bind(me.to_string())
        .bind(&id)
        .execute(&s.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
