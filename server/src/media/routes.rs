use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use pulse_protocol::rest::MediaInfo;
use tokio::sync::Semaphore;

use super::{CONTENT_TYPE, MediaKind, process};
use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

/// Bodies a little over the processing limit still reach it, so they get its clear message.
const BODY_LIMIT: usize = process::MAX_INPUT + 64 * 1024;

/// At most this many uploads processed at once: decoding and resizing is the costly part, so this
/// bounds the CPU/RAM a burst of uploads can take.
const PROCESS_PARALLELISM: usize = 2;
static PROCESS_SLOTS: std::sync::LazyLock<Arc<Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(Semaphore::new(PROCESS_PARALLELISM)));

/// Run CPU-bound image work (emoji uploads, Go Live previews) on the blocking pool, sharing the
/// same [`PROCESS_PARALLELISM`] slots.
pub(crate) async fn run_processing<T: Send + 'static>(
    job: impl FnOnce() -> T + Send + 'static,
) -> AppResult<T> {
    run_in_slot(PROCESS_SLOTS.clone(), job).await
}

/// Run `job` on the blocking pool once one of `slots` is free. The slot moves into the task, so it
/// is held while the work runs even if the client goes away.
async fn run_in_slot<T: Send + 'static>(
    slots: Arc<Semaphore>,
    job: impl FnOnce() -> T + Send + 'static,
) -> AppResult<T> {
    let slot = slots
        .acquire_owned()
        .await
        .map_err(|e| anyhow::anyhow!("processing slots closed: {e}"))?;
    tokio::task::spawn_blocking(move || {
        let _slot = slot;
        job()
    })
    .await
    .map_err(|e| anyhow::anyhow!("image processing panicked: {e}").into())
}

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
    body: Result<Bytes, BytesRejection>,
) -> AppResult<Json<MediaInfo>> {
    // Past the body limit axum would answer in plain text; keep to the API's JSON error shape.
    let body = body.map_err(|e| {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            AppError::PayloadTooLarge(process::ProcessError::TooBig.to_string())
        } else {
            AppError::BadRequest("the upload couldn't be read".into())
        }
    })?;
    if !s.limits.upload_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    let dir = &s.cfg.media_dir;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| anyhow::anyhow!("create {}: {e}", dir.display()))?;
    super::disk_ok(dir, s.cfg.media_min_free_bytes)?;
    let processed = run_processing(move || process::emoji(&body))
        .await?
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
    let path = super::path_for(&s.cfg.media_dir, &hash).ok_or(AppError::NotFound)?;
    let bytes = match tokio::fs::read(path).await {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;

    #[tokio::test]
    async fn a_third_job_waits_while_two_run() {
        let slots = Arc::new(Semaphore::new(PROCESS_PARALLELISM));
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let started = Arc::new(AtomicUsize::new(0));
        let jobs: Vec<_> = (0..3)
            .map(|_| {
                let (slots, gate, started) = (slots.clone(), gate.clone(), started.clone());
                tokio::spawn(run_in_slot(slots, move || {
                    started.fetch_add(1, Ordering::SeqCst);
                    let (open, cv) = &*gate;
                    // Bounded, so a failing run ends instead of hanging on blocked threads.
                    let _ =
                        cv.wait_timeout_while(open.lock().unwrap(), Duration::from_secs(5), |o| {
                            !*o
                        });
                }))
            })
            .collect();
        // Wait for the first two to start (bounded), then give a third time to wrongly follow.
        for _ in 0..500 {
            if started.load(Ordering::SeqCst) >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        let running = started.load(Ordering::SeqCst);
        // Release first, so a failed assertion doesn't leave blocked threads hanging the runtime.
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        assert_eq!(running, 2, "the third must wait");
        for j in jobs {
            j.await.unwrap().unwrap();
        }
        assert_eq!(started.load(Ordering::SeqCst), 3);
    }
}
