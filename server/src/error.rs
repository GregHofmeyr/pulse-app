use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use pulse_protocol::rest::{ApiError, MfaRequired};

/// A login ticket that can't leak through `{:?}`.
pub struct Ticket(pub String);

impl std::fmt::Debug for Ticket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Ticket(..)")
    }
}

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
    /// Password right, second factor still needed: carries the login ticket. Answered as a 403
    /// with an `MfaRequired` body; never logged or shown in `Display`.
    #[error("two-factor code required")]
    MfaRequired(Ticket),
    #[error("gone")]
    Gone,
    #[error("unsupported media type")]
    UnsupportedMediaType,
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
        let api = |status, code: &str, message: String| {
            (
                status,
                serde_json::json!(ApiError {
                    code: code.into(),
                    message
                }),
            )
        };
        let message = self.to_string();
        let (status, body) = match &self {
            Self::NotFound => api(StatusCode::NOT_FOUND, "not_found", message),
            Self::Forbidden | Self::ForbiddenMsg(_) => {
                api(StatusCode::FORBIDDEN, "forbidden", message)
            }
            Self::Unauthorized => api(StatusCode::UNAUTHORIZED, "unauthorized", message),
            Self::MfaRequired(t) => (
                StatusCode::FORBIDDEN,
                serde_json::json!(MfaRequired {
                    code: "mfa_required".into(),
                    message: "Enter the 6-digit code from your authenticator app. If Pulse doesn't ask for one, update it.".into(),
                    ticket: t.0.clone(),
                }),
            ),
            Self::Gone => api(StatusCode::GONE, "gone", message),
            Self::UnsupportedMediaType => api(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                message,
            ),
            Self::BadRequest(_) => api(StatusCode::BAD_REQUEST, "bad_request", message),
            Self::PayloadTooLarge(_) => {
                api(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large", message)
            }
            Self::Conflict(_) => api(StatusCode::CONFLICT, "conflict", message),
            Self::TooManyRequests => api(StatusCode::TOO_MANY_REQUESTS, "rate_limited", message),
            Self::InsufficientStorage => {
                api(StatusCode::INSUFFICIENT_STORAGE, "storage_full", message)
            }
            Self::Internal(e) => {
                tracing::error!(error = ?e, "internal error");
                api(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "internal error".into(),
                )
            }
        };
        (status, Json(body)).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
