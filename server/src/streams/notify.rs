//! "Notify me when they go live" (#60): `PUT|DELETE /me/live-notify/{id}`. The list is saved per
//! account (so every device knows it), told to your own sessions only, and is in your Ready. The
//! person on it is never told. Clients decide the notification itself when a `StreamStarted` arrives.

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::put;
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::UserId;
use sqlx::SqlitePool;

use crate::AppState;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};

pub fn router() -> Router<AppState> {
    Router::new().route("/me/live-notify/{id}", put(follow).delete(unfollow))
}

/// The people `owner` gets a notification for, in the order they were added; for their Ready only.
pub async fn list(db: &SqlitePool, owner: UserId) -> anyhow::Result<Vec<UserId>> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT target_id FROM live_notify WHERE owner_id = ? ORDER BY created_at, rowid",
    )
    .bind(owner.to_string())
    .fetch_all(db)
    .await?;
    Ok(ids.iter().map(|i| i.parse()).collect::<Result<_, _>>()?)
}

/// The person named: real and not you.
async fn target(s: &AppState, me: UserId, id: &str) -> AppResult<UserId> {
    // A malformed id names no user, so it is a 404 like an unknown one.
    let target: UserId = id.parse().map_err(|_| AppError::NotFound)?;
    if target == me {
        return Err(AppError::BadRequest(
            "You can't be notified about your own stream.".into(),
        ));
    }
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM users WHERE id = ?")
        .bind(target.to_string())
        .fetch_optional(&s.db)
        .await?;
    if exists.is_none() {
        return Err(AppError::NotFound);
    }
    if !s.limits.live_notify_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(Some(
            "You're changing live notifications too fast. Try again in a minute.".into(),
        )));
    }
    Ok(target)
}

async fn follow(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let target = target(&s, me, &id).await?;
    sqlx::query(
        "INSERT INTO live_notify (owner_id, target_id, created_at) VALUES (?, ?, ?) \
         ON CONFLICT (owner_id, target_id) DO NOTHING",
    )
    .bind(me.to_string())
    .bind(target.to_string())
    .bind(now())
    .execute(&s.db)
    .await?;
    changed(&s, me, target, true).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn unfollow(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let target = target(&s, me, &id).await?;
    sqlx::query("DELETE FROM live_notify WHERE owner_id = ? AND target_id = ?")
        .bind(me.to_string())
        .bind(target.to_string())
        .execute(&s.db)
        .await?;
    changed(&s, me, target, false).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn changed(s: &AppState, me: UserId, target: UserId, on: bool) {
    s.hub
        .publish(
            &s.db,
            Event::LiveNotifyChanged {
                user_id: me,
                target_id: target,
                on,
            },
        )
        .await;
}
