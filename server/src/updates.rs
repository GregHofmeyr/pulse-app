//! App updates (#82): the updater manifest and the signed Windows installer, for signed-in people only.
//!
//! `just publish-update` (deploy/publish-update.sh) fills `cfg.updates_dir` (`/data/updates`): `latest.json` and
//! `Pulse_<X.Y.Z>_x64-setup.exe`, the current version and the one before. The app asks with its session token, so
//! only friends can see or download a build. No manifest = 404, which the updater takes as "no update".
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

pub fn router() -> Router<AppState> {
    Router::new().route("/updates/{file}", get(fetch))
}

/// Only an installer name `publish-update` writes: `Pulse_<X.Y.Z>_x64-setup.exe`, digits only. Checked before any
/// path is built, so nothing else in the dir (`latest.json.tmp`, `.sig`, `.incoming/`) and nothing outside it is served.
pub fn installer_name_ok(name: &str) -> bool {
    let Some(v) = name
        .strip_prefix("Pulse_")
        .and_then(|r| r.strip_suffix("_x64-setup.exe"))
    else {
        return false;
    };
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()))
}

async fn fetch(
    State(s): State<AppState>,
    AuthUser(_): AuthUser,
    Path(file): Path<String>,
    req: Request,
) -> AppResult<Response> {
    if file == "latest.json" {
        let path = s.cfg.updates_dir.join("latest.json");
        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(AppError::NotFound),
            Err(e) => return Err(anyhow::anyhow!("read {}: {e}", path.display()).into()),
        };
        return Ok((
            [
                (header::CONTENT_TYPE, "application/json"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response());
    }
    if !installer_name_ok(&file) {
        return Err(AppError::NotFound);
    }
    let path = s.cfg.updates_dir.join(&file);
    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return Err(AppError::NotFound);
    }
    // Streamed from disk (an ~80 MB installer never sits in memory), with range requests.
    let Ok(res) = ServeFile::new(path).oneshot(req).await;
    let mut res = res.into_response();
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    Ok(res)
}
