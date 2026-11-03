//! Private nicknames (#48): a name you give someone that only you see. They live in one table
//! keyed by owner, reach the owner's Ready and the owner's own sessions, and nothing else reads
//! them.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::put;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{PrivateNickname, SetNicknameRequest};
use sqlx::SqlitePool;

use super::names;
use crate::AppState;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};

pub fn router() -> Router<AppState> {
    Router::new().route("/me/private-nicknames/{id}", put(set))
}

/// The names `owner` gave people, for their Ready only.
pub async fn of_owner(db: &SqlitePool, owner: UserId) -> AppResult<Vec<PrivateNickname>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT target_id, nickname FROM private_nicknames WHERE owner_id = ? ORDER BY target_id",
    )
    .bind(owner.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|(id, nickname)| {
            Ok(PrivateNickname {
                user_id: id.parse().map_err(anyhow::Error::from)?,
                nickname,
            })
        })
        .collect()
}

pub(crate) fn too_fast() -> AppError {
    AppError::TooManyRequests(Some(
        "You're renaming too fast. Try again in a minute.".into(),
    ))
}

/// Set (or, blank, clear) your private nickname for someone, then tell your own sessions.
async fn set(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
    Json(req): Json<SetNicknameRequest>,
) -> AppResult<StatusCode> {
    // A malformed id names no user, so it is a 404 like an unknown one.
    let target: UserId = id.parse().map_err(|_| AppError::NotFound)?;
    if target == me {
        return Err(AppError::BadRequest(
            "Pick your own name in Settings → Profile.".into(),
        ));
    }
    let nickname = names::clean_nickname(req.nickname.as_deref())?;
    if !s.limits.rename_user.hit(&me.to_string()) {
        return Err(too_fast());
    }
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM users WHERE id = ?")
        .bind(target.to_string())
        .fetch_optional(&s.db)
        .await?;
    if exists.is_none() {
        return Err(AppError::NotFound);
    }

    let _order = s.hub.in_order().await;
    match &nickname {
        Some(n) => {
            sqlx::query(
                "INSERT INTO private_nicknames (owner_id, target_id, nickname, updated_at) \
                 VALUES (?, ?, ?, ?) ON CONFLICT (owner_id, target_id) \
                 DO UPDATE SET nickname = excluded.nickname, updated_at = excluded.updated_at",
            )
            .bind(me.to_string())
            .bind(target.to_string())
            .bind(n)
            .bind(now())
            .execute(&s.db)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM private_nicknames WHERE owner_id = ? AND target_id = ?")
                .bind(me.to_string())
                .bind(target.to_string())
                .execute(&s.db)
                .await?;
        }
    }
    s.hub
        .publish(
            &s.db,
            Event::PrivateNicknameChanged {
                user_id: me,
                target_id: target,
                nickname,
            },
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}
