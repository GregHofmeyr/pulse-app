use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use pulse_protocol::rest::ApiError;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    /// A 403 whose reason is worth showing to the user as-is.
    #[error("{0}")]
    ForbiddenMsg(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("gone")]
    Gone,
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    PayloadTooLarge(String),
    #[error("too many attempts, try again in a bit")]
    TooManyRequests,
    #[error("server storage is full, so uploads are paused for now")]
    InsufficientStorage,
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(e.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::Forbidden | Self::ForbiddenMsg(_) => (StatusCode::FORBIDDEN, "forbidden"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::Gone => (StatusCode::GONE, "gone"),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            Self::PayloadTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large"),
            Self::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            Self::TooManyRequests => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            Self::InsufficientStorage => (StatusCode::INSUFFICIENT_STORAGE, "storage_full"),
            Self::Internal(e) => {
                tracing::error!(error = ?e, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal")
            }
        };
        let message = match &self {
            Self::Internal(_) => "internal error".to_string(),
            other => other.to_string(),
        };
        (
            status,
            Json(ApiError {
                code: code.into(),
                message,
            }),
        )
            .into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
