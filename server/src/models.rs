//! Downloadable models (#77): the app fetches the low-delay DeepFilterNet3 model ("Strong (low delay)") on first use,
//! for signed-in people only, like `/updates`.
//!
//! `just publish-models` (deploy/publish-models.sh) puts it in `cfg.updates_dir/models` (`/data/updates/models`).
//! Only the exact names in [`MODELS`] are served: nothing else in that dir, and nothing outside it.
use axum::Router;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

/// DeepFilterNet3's low-delay model, as libDF ships it (the app checks its SHA-256).
pub const LOW_DELAY_MODEL: &str = "DeepFilterNet3_ll_onnx.tar.gz";

/// Every name the route serves.
const MODELS: &[&str] = &[LOW_DELAY_MODEL];

pub fn router() -> Router<AppState> {
    Router::new().route("/models/{name}", get(fetch))
}

/// An exact allowlist, checked before any path is built.
pub fn model_name_ok(name: &str) -> bool {
    MODELS.contains(&name)
}

async fn fetch(
    State(s): State<AppState>,
    AuthUser(_): AuthUser,
    Path(name): Path<String>,
    req: Request,
) -> AppResult<Response> {
    if !model_name_ok(&name) {
        return Err(AppError::NotFound);
    }
    let path = s.cfg.updates_dir.join("models").join(&name);
    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return Err(AppError::NotFound);
    }
    // Streamed from disk (36 MB), with range requests.
    let Ok(res) = ServeFile::new(path).oneshot(req).await;
    let mut res = res.into_response();
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    Ok(res)
}
