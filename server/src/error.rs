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
    /// A 409 with its own `code`, for a client that acts on which conflict it was.
    #[error("{1}")]
    ConflictCode(&'static str, String),
    #[error("{0}")]
    PayloadTooLarge(String),
    /// A 429; the message is the default unless the reason is worth showing as-is.
    #[error("{}", .0.as_deref().unwrap_or("too many attempts, try again in a bit"))]
    TooManyRequests(Option<String>),
    /// A 503 whose reason is worth showing to the user as-is.
    #[error("{0}")]
    Unavailable(String),
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

/// Whether `e` failed a foreign key: a row the write refers to (its channel, message, poll...)
/// was deleted between the request's check and its write. Writes re-check their channel under the
/// write lock (`access::channel_live_in`), so this is the backstop for any write that doesn't.
fn foreign_key_failure(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        matches!(c.downcast_ref::<sqlx::Error>(),
            Some(sqlx::Error::Database(d)) if d.is_foreign_key_violation())
    })
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
        // What a write referred to went before it landed (#109): it isn't there, so a 404. Mapped
        // here, where every route's errors pass, whether they came up as `sqlx` or `anyhow` errors.
        let this = match self {
            Self::Internal(e) if foreign_key_failure(&e) => {
                tracing::info!(error = %e, "a write's target went before it landed; 404");
                Self::NotFound
            }
            other => other,
        };
        let message = this.to_string();
        let (status, body) = match &this {
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
            Self::ConflictCode(code, _) => api(StatusCode::CONFLICT, code, message),
            Self::TooManyRequests(_) => api(StatusCode::TOO_MANY_REQUESTS, "rate_limited", message),
            Self::Unavailable(_) => {
                api(StatusCode::SERVICE_UNAVAILABLE, "unavailable", message)
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A foreign key failure from a real SQLite: an insert naming a channel that isn't there.
    async fn fk_failure() -> sqlx::Error {
        use sqlx::ConnectOptions;
        use std::str::FromStr;
        let mut db = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(true)
            .connect()
            .await
            .unwrap();
        sqlx::query("CREATE TABLE channels (id TEXT PRIMARY KEY)")
            .execute(&mut db)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE messages (channel_id TEXT NOT NULL REFERENCES channels(id))")
            .execute(&mut db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO messages (channel_id) VALUES ('gone')")
            .execute(&mut db)
            .await
            .unwrap_err()
    }

    fn status(e: AppError) -> StatusCode {
        e.into_response().status()
    }

    /// A write whose channel (or message, or anything it refers to) went between its check and
    /// its write fails a foreign key: that is a 404, never a 500 (#109). Also when the error was
    /// carried up as an `anyhow` error, with context or not.
    #[tokio::test]
    async fn a_foreign_key_failure_is_a_404() {
        assert_eq!(status(fk_failure().await.into()), StatusCode::NOT_FOUND);
        let wrapped = anyhow::Error::from(fk_failure().await).context("storing the message");
        assert_eq!(status(AppError::Internal(wrapped)), StatusCode::NOT_FOUND);
        // other database errors stay a 500
        assert_eq!(
            status(sqlx::Error::RowNotFound.into()),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
