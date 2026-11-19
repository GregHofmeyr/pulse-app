//! A server's banner (#475): what its invite cards show over its chip, and the invite page too.
//!
//! `PUT /servers/{id}/banner` sets it to an upload from `POST /media/banner` (the profile banner's pipeline: the same
//! crop, size limits, still and storage), or removes it (`hash: null`). Its owner only. Everyone hears
//! `ServerUpdated` (servers are public). The upload it replaces is left to the unused-media sweep.

use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::ServerId;
use pulse_protocol::rest::{Server, SetServerBannerRequest};

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::servers::routes::{load_server, server_exists, server_live_in};

pub fn router() -> Router<AppState> {
    Router::new().route("/servers/{id}/banner", put(set_banner))
}

/// `PUT /servers/{id}/banner`: 403 for anyone but the owner, 404 for a deleted or unknown server, 400 for a hash that
/// isn't a live banner upload (it is fine when it is the banner already).
async fn set_banner(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
    Json(req): Json<SetServerBannerRequest>,
) -> AppResult<Json<Server>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    server_exists(&s.db, id).await?;
    if req.hash.is_some() {
        crate::media::writes_ok(&s.cfg)?;
    }
    // Hold the order guard so two quick changes publish in commit order.
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    server_live_in(&mut tx, id).await?; // deleted since the check above
    let (owner, current): (String, Option<String>) =
        sqlx::query_as("SELECT created_by, banner_hash FROM servers WHERE id = ?")
            .bind(id.to_string())
            .fetch_one(&mut *tx)
            .await?;
    if owner != me.to_string() {
        return Err(AppError::ForbiddenMsg(
            "Only the server's owner can change its banner.".into(),
        ));
    }
    if current == req.hash {
        drop(tx);
        return Ok(Json(load_server(&s.db, id).await?));
    }
    if let Some(hash) = &req.hash {
        let found: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM media WHERE hash = ? AND kind = 'banner'")
                .bind(hash)
                .fetch_optional(&mut *tx)
                .await?;
        if found.is_none() {
            return Err(AppError::BadRequest(
                "That picture has expired. Upload it again.".into(),
            ));
        }
    }
    sqlx::query("UPDATE servers SET banner_hash = ? WHERE id = ?")
        .bind(&req.hash)
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let server = load_server(&s.db, id).await?;
    tracing::info!(server = %id, set = req.hash.is_some(), "server banner changed");
    s.hub
        .publish(
            &s.db,
            Event::ServerUpdated {
                server: server.clone(),
            },
        )
        .await;
    Ok(Json(server))
}
