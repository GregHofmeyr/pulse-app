//! Hide what I'm playing per person (#436): `PUT|DELETE /me/activity-hidden/{user_id}`. Someone on your list never
//! gets your game activity: your "Playing" (and the Join inside it) and your Steam hours (profile card and a server's
//! board). They see you online with no game, as if you played nothing. "Listening to" isn't game activity and isn't
//! touched.
//!
//! Enforced where game activity leaves the server: the hub's per-viewer pass (`presence::Subject::shows_game_to`,
//! read by `Hub::sync_presence` and Ready's people) and the Steam hours reads. A change re-runs the pass, so a newly
//! hidden viewer's clients drop your game at once and an un-hidden one gets it back.
//!
//! Kept per person (`activity_hidden`) so it follows you to every device, sent only to you (Ready's `activity_hidden`
//! and `ActivityHiddenChanged`, always the whole list).

use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::UserId;
use sqlx::SqlitePool;

use crate::AppState;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};

/// People one person may have on the list.
pub const MAX_PEOPLE: i64 = 200;

pub fn router() -> Router<AppState> {
    Router::new().route("/me/activity-hidden/{id}", put(set).delete(clear))
}

/// `owner`'s list, in the order they were added.
pub async fn list(db: &SqlitePool, owner: UserId) -> AppResult<Vec<UserId>> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT target_id FROM activity_hidden WHERE user_id = ? ORDER BY created_at, rowid",
    )
    .bind(owner.to_string())
    .fetch_all(db)
    .await?;
    ids.iter()
        .map(|i| {
            i.parse()
                .map_err(|e| AppError::from(anyhow::Error::from(e)))
        })
        .collect()
}

/// The person named: real and not you. Also the rate limit.
async fn target(s: &AppState, me: UserId, id: &str) -> AppResult<UserId> {
    // a malformed id names no user, so it is a 404 like an unknown one
    let target: UserId = id.parse().map_err(|_| AppError::NotFound)?;
    if target == me {
        return Err(AppError::BadRequest(
            "You always see what you're playing.".into(),
        ));
    }
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM users WHERE id = ?")
        .bind(target.to_string())
        .fetch_optional(&s.db)
        .await?;
    if exists.is_none() {
        return Err(AppError::NotFound);
    }
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(Some(
            "You're changing who sees your games too fast. Try again in a minute.".into(),
        )));
    }
    Ok(target)
}

/// `PUT /me/activity-hidden/{id}`: hide your game activity from them. Answers with your whole list.
async fn set(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Vec<UserId>>> {
    let target = target(&s, me, &id).await?;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let (count, has): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(target_id = ?), 0) FROM activity_hidden WHERE user_id = ?",
    )
    .bind(target.to_string())
    .bind(me.to_string())
    .fetch_one(&mut *tx)
    .await?;
    if has == 0 && count >= MAX_PEOPLE {
        return Err(AppError::BadRequest(format!(
            "What you're playing can be hidden from up to {MAX_PEOPLE} people."
        )));
    }
    sqlx::query(
        "INSERT INTO activity_hidden (user_id, target_id, created_at) VALUES (?, ?, ?)
         ON CONFLICT (user_id, target_id) DO NOTHING",
    )
    .bind(me.to_string())
    .bind(target.to_string())
    .bind(now())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(announce(&s, me).await?))
}

/// `DELETE /me/activity-hidden/{id}`: show them your game activity again. Answers with your whole list.
async fn clear(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Vec<UserId>>> {
    let target = target(&s, me, &id).await?;
    sqlx::query("DELETE FROM activity_hidden WHERE user_id = ? AND target_id = ?")
        .bind(me.to_string())
        .bind(target.to_string())
        .execute(&s.db)
        .await?;
    Ok(Json(announce(&s, me).await?))
}

/// Tell `me`'s own sessions their list as it is now, and every viewer what they may now see of `me`'s game.
async fn announce(s: &AppState, me: UserId) -> AppResult<Vec<UserId>> {
    let list = list(&s.db, me).await?;
    s.hub
        .publish(
            &s.db,
            Event::ActivityHiddenChanged {
                user_id: me,
                list: list.clone(),
            },
        )
        .await;
    s.hub.sync_presence(&s.db, me).await;
    Ok(list)
}

/// Whether `owner` hides their game activity from `viewer` (never from themselves).
pub async fn hides(db: &SqlitePool, owner: UserId, viewer: UserId) -> sqlx::Result<bool> {
    if owner == viewer {
        return Ok(false);
    }
    let hit: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM activity_hidden WHERE user_id = ? AND target_id = ?")
            .bind(owner.to_string())
            .bind(viewer.to_string())
            .fetch_optional(db)
            .await?;
    Ok(hit.is_some())
}
