//! Server rail order (#185): the order you dragged the servers into, kept per person (`server_order`) so it follows
//! you to every device, and sent only to you (Ready and `ServerOrderChanged`). Servers you never placed follow the
//! placed ones in their usual order; that's the client's to draw.

use std::collections::HashSet;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::put;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::ServerOrderRequest;
use sqlx::SqlitePool;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

/// The most servers one order may name: far more than any rail holds.
pub const MAX_ORDER: usize = 500;

pub fn router() -> Router<AppState> {
    Router::new().route("/me/server-order", put(set))
}

/// `user`'s server order: the live servers they placed, first to last.
pub async fn of(db: &SqlitePool, user: UserId) -> AppResult<Vec<ServerId>> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT o.server_id FROM server_order o JOIN servers s ON s.id = o.server_id
         WHERE o.user_id = ? AND s.deleted_at IS NULL ORDER BY o.position, o.server_id",
    )
    .bind(user.to_string())
    .fetch_all(db)
    .await?;
    ids.into_iter()
        .map(|id| Ok(id.parse().map_err(anyhow::Error::from)?))
        .collect()
}

/// Save your server order (replacing the old one) and tell your own sessions. A server deleted since your client
/// last looked is left out rather than refused, so a drag never fails over it.
async fn set(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(req): Json<ServerOrderRequest>,
) -> AppResult<StatusCode> {
    if req.server_ids.len() > MAX_ORDER {
        return Err(AppError::BadRequest("that's too many servers".into()));
    }
    let mut seen = HashSet::new();
    if !req.server_ids.iter().all(|id| seen.insert(*id)) {
        return Err(AppError::BadRequest(
            "each server can only be in the order once".into(),
        ));
    }
    if !s.limits.layout_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(Some(
            "You're moving things too fast. Try again in a minute.".into(),
        )));
    }
    let live: HashSet<String> =
        sqlx::query_scalar::<_, String>("SELECT id FROM servers WHERE deleted_at IS NULL")
            .fetch_all(&s.db)
            .await?
            .into_iter()
            .collect();

    let _order = s.hub.in_order().await;
    let mut tx = s.db.begin().await?;
    sqlx::query("DELETE FROM server_order WHERE user_id = ?")
        .bind(me.to_string())
        .execute(&mut *tx)
        .await?;
    let mut position = 0_i64;
    for id in req.server_ids.iter().map(ToString::to_string) {
        if !live.contains(&id) {
            continue;
        }
        sqlx::query("INSERT INTO server_order (user_id, server_id, position) VALUES (?, ?, ?)")
            .bind(me.to_string())
            .bind(&id)
            .bind(position)
            .execute(&mut *tx)
            .await?;
        position += 1;
    }
    tx.commit().await?;
    let server_ids = of(&s.db, me).await?;
    s.hub
        .publish(
            &s.db,
            Event::ServerOrderChanged {
                user_id: me,
                server_ids,
            },
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}
