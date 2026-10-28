use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::Profile;

use super::load_profile;
use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

pub fn router() -> Router<AppState> {
    Router::new().route("/users/{id}/profile", get(profile))
}

async fn profile(
    State(s): State<AppState>,
    AuthUser(_): AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Profile>> {
    // A malformed id names no user, so it is a 404 like an unknown one.
    let id: UserId = id.parse().map_err(|_| AppError::NotFound)?;
    Ok(Json(
        load_profile(&s.db, id).await?.ok_or(AppError::NotFound)?,
    ))
}
