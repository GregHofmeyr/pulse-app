use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use pulse_protocol::ids::UserId;

use crate::AppState;
use crate::error::AppError;

/// The authenticated caller, from `Authorization: Bearer <token>`.
#[derive(Clone, Copy, Debug)]
pub struct AuthUser(pub UserId);

/// The raw bearer token (for logout).
pub struct BearerToken(pub String);

fn bearer(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::to_string)
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer(parts).ok_or(AppError::Unauthorized)?;
        super::session::authenticate(&state.db, &token)
            .await?
            .map(AuthUser)
            .ok_or(AppError::Unauthorized)
    }
}

impl FromRequestParts<AppState> for BearerToken {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _: &AppState) -> Result<Self, Self::Rejection> {
        bearer(parts).map(BearerToken).ok_or(AppError::Unauthorized)
    }
}
