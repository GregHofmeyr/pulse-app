use axum::extract::{DefaultBodyLimit, State};
use axum::routing::post;
use axum::{Json, Router};
use pulse_protocol::rest::{CreateReportRequest, CreateReportResponse};

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

/// Room for a 512 KB log plus text and meta; everything else keeps axum's default.
const BODY_LIMIT: usize = 1024 * 1024;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/reports", post(create))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
}

async fn create(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(req): Json<CreateReportRequest>,
) -> AppResult<Json<CreateReportResponse>> {
    if !s.limits.report_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    let id = super::store::insert(&s.db, me, &req).await?;
    tracing::info!(%me, kind = req.kind.as_str(), %id, "report received");
    Ok(Json(CreateReportResponse { id }))
}
