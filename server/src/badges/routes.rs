//! Badge routes: the catalogue, the caller's showcase and [NEW] state, and the two things only
//! the client can see: talking into a muted mic (The Doos) and finding the logo egg.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::Utc;
use pulse_protocol::rest::{BadgeCatalogue, EggRequest, SetShowcaseRequest};

use super::catalogue::stat;
use super::time::{doos_counts, sast_day};
use super::view::{self, SHOWCASE_MAX};
use super::{engine, stats};
use crate::AppState;
use crate::access::load_channel;
use crate::auth::AuthUser;
use crate::db::begin_write;
use crate::error::{AppError, AppResult};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/badges", get(catalogue))
        .route("/me/badges/showcase", put(set_showcase))
        .route("/me/badges/seen", post(seen))
        .route("/me/badges/muted-talk", post(muted_talk))
        .route("/me/badges/egg", post(egg))
}

/// `GET /badges`: every badge as the caller may see it (see [`view::catalogue`]).
async fn catalogue(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
) -> AppResult<Json<BadgeCatalogue>> {
    let mut tx = s.db.begin().await?;
    let c = view::catalogue(&mut tx, me).await?;
    tx.commit().await?;
    Ok(Json(c))
}

/// `PUT /me/badges/showcase`: up to five distinct badges the caller holds, in order. Empty clears
/// the choice, so the profile falls back to the rarest five.
async fn set_showcase(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(req): Json<SetShowcaseRequest>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    if req.keys.len() > SHOWCASE_MAX {
        return Err(AppError::BadRequest(
            "A showcase holds up to 5 badges.".into(),
        ));
    }
    if req
        .keys
        .iter()
        .enumerate()
        .any(|(i, k)| req.keys[..i].contains(k))
    {
        return Err(AppError::BadRequest(
            "A badge can be in the showcase once.".into(),
        ));
    }
    let mut tx = begin_write(&s.db).await?;
    let held = engine::tiers_held(&mut tx, me).await?;
    if !req.keys.iter().all(|k| held.get(k).is_some_and(|t| *t > 0)) {
        return Err(AppError::BadRequest(
            "You can only show badges you have.".into(),
        ));
    }
    let id = me.to_string();
    sqlx::query("UPDATE user_badges SET showcase = NULL WHERE user_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    for (i, key) in req.keys.iter().enumerate() {
        sqlx::query("UPDATE user_badges SET showcase = ? WHERE user_id = ? AND badge_key = ?")
            .bind(i as i64)
            .bind(&id)
            .bind(key)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /me/badges/seen`: the caller opened their trophy case; nothing of theirs is [NEW] now.
async fn seen(State(s): State<AppState>, AuthUser(me): AuthUser) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    sqlx::query("UPDATE user_badges SET seen = 1 WHERE user_id = ? AND seen = 0")
        .bind(me.to_string())
        .execute(&s.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /me/badges/muted-talk`: the "you're muted" detector fired. Counts only in a server voice
/// channel the caller is in right now, at most once a minute and five times a South African day
/// (see [`doos_counts`]); anything else is quietly not counted.
async fn muted_talk(State(s): State<AppState>, AuthUser(me): AuthUser) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let Some((channel, _)) = s.voice.room_flags(me) else {
        return Ok(StatusCode::NO_CONTENT);
    };
    if !load_channel(&s.db, channel)
        .await?
        .is_some_and(|c| c.server_id.is_some())
    {
        return Ok(StatusCode::NO_CONTENT);
    }
    let now = Utc::now();
    let (now_unix, today) = (now.timestamp(), sast_day(now));
    let mut tx = begin_write(&s.db).await?;
    let last_at = stats::get(&mut tx, me, stat::DOOS_LAST_AT).await?;
    let day = stats::get(&mut tx, me, stat::DOOS_DAY).await?;
    let day_count = stats::get(&mut tx, me, stat::DOOS_DAY_COUNT).await?;
    if !doos_counts(now_unix, last_at, day, day_count) {
        return Ok(StatusCode::NO_CONTENT);
    }
    let day_count = if day == today { day_count + 1 } else { 1 };
    stats::set(&mut tx, me, stat::DOOS_LAST_AT, now_unix).await?;
    stats::set(&mut tx, me, stat::DOOS_DAY, today).await?;
    stats::set(&mut tx, me, stat::DOOS_DAY_COUNT, day_count).await?;
    stats::add(&mut tx, me, stat::DOOS, 1).await?;
    tx.commit().await?;
    engine::after(&s, &[me]).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /me/badges/egg`: the client found a hidden egg. Only `"logo"` exists.
async fn egg(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(req): Json<EggRequest>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    if req.egg != "logo" {
        return Err(AppError::BadRequest("no such egg".into()));
    }
    let mut conn = s.db.acquire().await?;
    stats::set_max(&mut conn, me, stat::LOGO_EGG, 1).await?;
    drop(conn);
    engine::after(&s, &[me]).await;
    Ok(StatusCode::NO_CONTENT)
}
