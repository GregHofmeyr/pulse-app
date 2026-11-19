//! Server settings › Overview (#55): a server's icon and its name, beside its banner (`banner.rs`).
//!
//! `PUT /servers/{id}/icon` sets the icon to an upload from `POST /media/avatar` (an avatar's square crop, size
//! limits and still), or removes it (`hash: null`). `PUT /servers/{id}/name` renames it. Any of its members may, as
//! with the banner (the v1 trust model). Its members hear `ServerUpdated` (#80: servers are private). An icon it
//! replaces is left to the unused-media sweep.

use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::ServerId;
use pulse_protocol::rest::{RenameServerRequest, Server, SetServerIconRequest};

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::servers::banner::{Picture, set_picture};
use crate::servers::routes::{clean_name, load_server, server_live_in, visible_to};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/servers/{id}/icon", put(set_icon))
        .route("/servers/{id}/name", put(rename))
}

/// `PUT /servers/{id}/icon`: as the banner's route. Any member; 404 for a deleted or
/// unknown server or one you aren't in (#80), 400 for a hash that isn't a live avatar upload.
async fn set_icon(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
    Json(req): Json<SetServerIconRequest>,
) -> AppResult<Json<Server>> {
    set_picture(&s, me, id, req.hash, Picture::Icon)
        .await
        .map(Json)
}

/// `PUT /servers/{id}/name`: any member; 404 for a deleted or unknown server or one you
/// aren't in (#80), 400 for a name that is blank or longer than 64 characters. The same name again says nothing.
async fn rename(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
    Json(req): Json<RenameServerRequest>,
) -> AppResult<Json<Server>> {
    if !s.limits.rename_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    visible_to(&s.db, id, me).await?;
    let name = clean_name(&req.name)?;
    crate::media::writes_ok(&s.cfg)?;
    // Hold the order guard so two quick renames publish in commit order.
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    server_live_in(&mut tx, id).await?; // deleted since the check above
    let current: String = sqlx::query_scalar("SELECT name FROM servers WHERE id = ?")
        .bind(id.to_string())
        .fetch_one(&mut *tx)
        .await?;
    if current == name {
        drop(tx);
        return Ok(Json(load_server(&s.db, id).await?));
    }
    sqlx::query("UPDATE servers SET name = ? WHERE id = ?")
        .bind(&name)
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let server = load_server(&s.db, id).await?;
    tracing::info!(server = %id, "server renamed");
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
