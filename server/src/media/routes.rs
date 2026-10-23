use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use pulse_protocol::rest::MediaInfo;

use super::{CONTENT_TYPE, MediaKind, process};
use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

/// Bodies a little over the processing limit still reach it, so they get its clear message.
const BODY_LIMIT: usize = process::MAX_INPUT + 64 * 1024;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/media/emoji",
            post(upload_emoji).layer(DefaultBodyLimit::max(BODY_LIMIT)),
        )
        .route("/media/{hash}", get(fetch))
}

async fn upload_emoji(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    body: Bytes,
) -> AppResult<Json<MediaInfo>> {
    if !s.limits.upload_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    let dir = &s.cfg.media_dir;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| anyhow::anyhow!("create {}: {e}", dir.display()))?;
    super::disk_ok(dir, s.cfg.media_min_free_bytes)?;
    let processed = tokio::task::spawn_blocking(move || process::emoji(&body))
        .await
        .map_err(|e| anyhow::anyhow!("emoji processing panicked: {e}"))?
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let info = super::put(&s.db, dir, MediaKind::Emoji, &processed, me).await?;
    tracing::info!(%me, hash = %info.hash, frames = info.frames, "emoji media stored");
    Ok(Json(info))
}

async fn fetch(
    State(s): State<AppState>,
    AuthUser(_): AuthUser,
    Path(hash): Path<String>,
) -> AppResult<impl IntoResponse> {
    if !super::valid_hash(&hash) {
        return Err(AppError::NotFound);
    }
    let bytes = match tokio::fs::read(super::path_for(&s.cfg.media_dir, &hash)).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(AppError::NotFound),
        Err(e) => return Err(anyhow::Error::from(e).context("read media").into()),
    };
    Ok((
        [
            (header::CONTENT_TYPE, CONTENT_TYPE),
            // Content-addressed: the bytes behind a hash never change.
            (
                header::CACHE_CONTROL,
                "private, max-age=31536000, immutable",
            ),
        ],
        bytes,
    ))
}
