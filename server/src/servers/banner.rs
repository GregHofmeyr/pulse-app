//! A server's banner (#475): what its invite cards show over its chip, and the invite page too.
//!
//! `PUT /servers/{id}/banner` sets it to an upload from `POST /media/banner` (the profile banner's pipeline: the same
//! crop, size limits, still and storage), or removes it (`hash: null`). Its owner only. Its members hear
//! `ServerUpdated` (#80: servers are private). The upload it replaces is left to the unused-media sweep.

use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{Server, SetServerBannerRequest};

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::servers::routes::{load_server, server_live_in, visible_to};

pub fn router() -> Router<AppState> {
    Router::new().route("/servers/{id}/banner", put(set_banner))
}

/// `PUT /servers/{id}/banner`: 403 for a member who isn't the owner, 404 for a deleted or unknown server or one you
/// aren't in (#80), 400 for a hash that
/// isn't a live banner upload (it is fine when it is the banner already).
async fn set_banner(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
    Json(req): Json<SetServerBannerRequest>,
) -> AppResult<Json<Server>> {
    set_picture(&s, me, id, req.hash, Picture::Banner)
        .await
        .map(Json)
}

/// Which of a server's pictures [`set_picture`] changes: its column and the upload kind it takes.
#[derive(Clone, Copy)]
pub(crate) enum Picture {
    /// From `POST /media/banner` (#475).
    Banner,
    /// From `POST /media/avatar`, the same square crop as an avatar (#55).
    Icon,
}

impl Picture {
    fn column(self) -> &'static str {
        match self {
            Self::Banner => "banner_hash",
            Self::Icon => "icon_hash",
        }
    }
    fn kind(self) -> &'static str {
        match self {
            Self::Banner => "banner",
            Self::Icon => "avatar",
        }
    }
    fn noun(self) -> &'static str {
        match self {
            Self::Banner => "banner",
            Self::Icon => "icon",
        }
    }
}

/// Set or remove one of a server's pictures, as its owner, and tell its members (`ServerUpdated`). The same rules
/// for both: 404 unless you're in the live server, 403 unless you own it, 400 for a hash that isn't a live upload of
/// the right kind (fine when it is the picture already: nothing changes and nothing is said).
pub(crate) async fn set_picture(
    s: &AppState,
    me: UserId,
    id: ServerId,
    hash: Option<String>,
    pic: Picture,
) -> AppResult<Server> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    visible_to(&s.db, id, me).await?;
    if hash.is_some() {
        crate::media::writes_ok(&s.cfg)?;
    }
    // Hold the order guard so two quick changes publish in commit order.
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    server_live_in(&mut tx, id).await?; // deleted since the check above
    let (owner, current): (String, Option<String>) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT created_by, {} FROM servers WHERE id = ?",
        pic.column()
    )))
    .bind(id.to_string())
    .fetch_one(&mut *tx)
    .await?;
    if owner != me.to_string() {
        return Err(AppError::ForbiddenMsg(format!(
            "Only the server's owner can change its {}.",
            pic.noun()
        )));
    }
    if current == hash {
        drop(tx);
        return load_server(&s.db, id).await;
    }
    if let Some(hash) = &hash {
        let found: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM media WHERE hash = ? AND kind = ?")
                .bind(hash)
                .bind(pic.kind())
                .fetch_optional(&mut *tx)
                .await?;
        if found.is_none() {
            return Err(AppError::BadRequest(
                "That picture has expired. Upload it again.".into(),
            ));
        }
    }
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE servers SET {} = ? WHERE id = ?",
        pic.column()
    )))
    .bind(&hash)
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let server = load_server(&s.db, id).await?;
    tracing::info!(server = %id, set = hash.is_some(), picture = pic.noun(), "server picture changed");
    s.hub
        .publish(
            &s.db,
            Event::ServerUpdated {
                server: server.clone(),
            },
        )
        .await;
    Ok(server)
}
