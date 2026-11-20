//! App updates (#82): the updater manifest and the signed Windows installer, for signed-in people only.
//!
//! `just publish-update` (deploy/publish-update.sh) fills `cfg.updates_dir` (`/data/updates`): `latest.json` and
//! `Pulse_<X.Y.Z>_x64-setup.exe`, the current version and the one before. The app asks with its session token, so
//! only friends can see or download a build. No manifest = 404, which the updater takes as "no update".
//!
//! `GET /updates/version` needs no session: the published version only (no installer URL or signature), so a
//! signed-out app can say an update is waiting for when it signs in.
use axum::Router;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::limits::{ClientIp, rate_key};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/updates/version", get(latest_version))
        .route("/updates/{file}", get(fetch))
        .layer(axum::middleware::from_fn(log_check))
}

/// The header the app's updater sends its own version in (#460). Older apps send none: their
/// updater's `User-Agent` names only the updater plugin.
pub const VERSION_HEADER: &str = "x-pulse-version";

/// The header apps from 0.2.18 send their platform in (`windows`, `linux`, `macos`), beside [`VERSION_HEADER`]: only
/// the Windows app updates itself, so the others are never turned away to update.
pub const PLATFORM_HEADER: &str = "x-pulse-platform";

/// A header's value for the log: printable ASCII only, at most 64 characters, `-` when absent.
fn header_for_log(headers: &HeaderMap, name: impl header::AsHeaderName) -> String {
    match headers.get(name).and_then(|v| v.to_str().ok()) {
        Some(v) if !v.is_empty() => v
            .chars()
            .filter(|c| c.is_ascii_graphic() || *c == ' ')
            .take(64)
            .collect(),
        _ => "-".into(),
    }
}

/// Every manifest request is logged at info (#460), refused ones (401: signed out there) included, so a
/// failing app's checks can be seen: the app's version, its updater's `User-Agent` and the status. Never
/// the token.
async fn log_check(req: Request, next: Next) -> Response {
    if req.uri().path() != "/updates/latest.json" {
        return next.run(req).await;
    }
    let version = header_for_log(req.headers(), VERSION_HEADER);
    let agent = header_for_log(req.headers(), header::USER_AGENT);
    let res = next.run(req).await;
    tracing::info!(
        app_version = %version,
        user_agent = %agent,
        status = res.status().as_u16(),
        "update check"
    );
    res
}

/// The published version, for anyone: `{"version": "X.Y.Z"}`, or 204 when nothing is published (or the manifest
/// names no version). Never the manifest itself: its installer URL and signature stay behind sign-in.
async fn latest_version(State(s): State<AppState>, ClientIp(ip): ClientIp) -> Response {
    if !s.limits.version_ip.hit(&rate_key(ip).to_string()) {
        return AppError::TooManyRequests(None).into_response();
    }
    let path = s.cfg.updates_dir.join("latest.json");
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return axum::http::StatusCode::NO_CONTENT.into_response();
        }
        Err(e) => {
            return AppError::from(anyhow::anyhow!("read {}: {e}", path.display())).into_response();
        }
    };
    match crate::app_versions::manifest_version(&bytes) {
        Some(version) => (
            [(header::CACHE_CONTROL, "no-store")],
            axum::Json(pulse_protocol::rest::LatestVersion { version }),
        )
            .into_response(),
        None => axum::http::StatusCode::NO_CONTENT.into_response(),
    }
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
    AuthUser(me): AuthUser,
    Path(file): Path<String>,
    req: Request,
) -> AppResult<Response> {
    if file == "latest.json" {
        use crate::app_versions::{CheckResult, header_version, manifest_version};
        // who asked on which version, and what they were told, outlasts the log (#534)
        let version = header_version(req.headers());
        let path = s.cfg.updates_dir.join("latest.json");
        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                spawn_record(&s, me, version, CheckResult::NoManifest);
                return Err(AppError::NotFound);
            }
            Err(e) => return Err(anyhow::anyhow!("read {}: {e}", path.display()).into()),
        };
        let result = CheckResult::of(version.as_deref(), manifest_version(&bytes).as_deref());
        spawn_record(&s, me, version, result);
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

/// Note the check off the request path (#534): the app's answer never waits on a SQLite write.
fn spawn_record(
    s: &AppState,
    me: pulse_protocol::ids::UserId,
    version: Option<String>,
    result: crate::app_versions::CheckResult,
) {
    let db = s.db.clone();
    tokio::spawn(async move {
        crate::app_versions::record_check(&db, me, version.as_deref(), result).await
    });
}
