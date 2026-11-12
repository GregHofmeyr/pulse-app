//! `POST /calls/{channel}` (start or join, ringing only on a start) and `POST /calls/{channel}/decline`.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use pulse_protocol::ids::ChannelId;
use pulse_protocol::rest::StartCallResponse;

use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/calls/{id}", post(start))
        .route("/calls/{id}/decline", post(decline))
}

async fn start(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
) -> AppResult<Json<StartCallResponse>> {
    let ch = channel_for(&s.db, me, id).await?;
    if !super::is_conversation(&ch) {
        return Err(AppError::BadRequest(
            "calls are for DMs and groups; join a server voice channel instead".into(),
        ));
    }
    if !crate::voice::routes::may_join(&s.db, me, &ch).await? {
        return Err(AppError::NotFound);
    }
    // Notes to self (#308): a DM with nobody else in it has nobody to call.
    if crate::dms::routes::members_of(&s.db, ch.id)
        .await?
        .iter()
        .all(|u| *u == me)
    {
        return Err(AppError::BadRequest(
            "there's nobody else here to call".into(),
        ));
    }
    if !s.limits.call_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    Ok(Json(super::start(&s, me, &ch).await?))
}

async fn decline(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
) -> AppResult<StatusCode> {
    let ch = channel_for(&s.db, me, id).await?;
    if !super::is_conversation(&ch) {
        return Err(AppError::BadRequest("only DM and group calls ring".into()));
    }
    super::decline(&s, me, ch.id).await;
    Ok(StatusCode::NO_CONTENT)
}
