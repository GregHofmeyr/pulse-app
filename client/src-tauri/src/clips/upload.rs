//! The PUT of a clip's file to its presigned URL: streamed from disk in 1 MiB chunks (a clip can
//! be 200 MiB; none of it is held in memory), with progress, cancel and a stall watchdog.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::sync::Notify;
use tokio::time::Instant;

const CHUNK: usize = 1024 * 1024;
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
/// No chunk taken by the connection (and no answer after the last one) for this long.
pub const STALL_AFTER: Duration = Duration::from_secs(120);

#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    #[error("the upload stopped moving")]
    Stalled,
    #[error("cancelled")]
    Cancelled,
    /// The store answered with a non-2xx status (an expired or tampered URL is a 403).
    #[error("the upload was refused ({0})")]
    Rejected(u16),
    #[error("couldn't read the clip: {0}")]
    File(String),
    #[error("can't reach the upload server: {0}")]
    Network(String),
}

/// Shared "stop now" switch for one upload. Cloning shares the switch.
#[derive(Clone, Default)]
pub struct CancellationFlag(Arc<(AtomicBool, Notify)>);

impl CancellationFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.0.store(true, Ordering::SeqCst);
        self.0.1.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.0.load(Ordering::SeqCst)
    }

    async fn cancelled(&self) {
        loop {
            let waiting = self.0.1.notified();
            if self.is_cancelled() {
                return;
            }
            waiting.await;
        }
    }
}

/// A client for the PUT: connects fast, but has no total timeout (the watchdog is the limit).
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("http client")
}

/// PUT `path` to `url`. `on_progress(sent, total)` runs at most every 250 ms; it stays below
/// `total` (99%) until the store answers 2xx, then reports `total` once.
pub async fn put_file(
    http: &reqwest::Client,
    url: &str,
    path: &Path,
    content_type: &str,
    on_progress: impl Fn(u64, u64) + Send + Sync + 'static,
    cancel: CancellationFlag,
) -> Result<(), UploadError> {
    put_file_with_stall(
        http,
        url,
        path,
        content_type,
        on_progress,
        cancel,
        STALL_AFTER,
    )
    .await
}

/// [`put_file`] with the stall limit chosen (tests use a short one).
pub async fn put_file_with_stall(
    http: &reqwest::Client,
    url: &str,
    path: &Path,
    content_type: &str,
    on_progress: impl Fn(u64, u64) + Send + Sync + 'static,
    cancel: CancellationFlag,
    stall: Duration,
) -> Result<(), UploadError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| UploadError::File(e.to_string()))?;
    let total = file
        .metadata()
        .await
        .map_err(|e| UploadError::File(e.to_string()))?
        .len();

    let started = Instant::now();
    // When the connection last took a chunk, in ms since `started`.
    let last_move = Arc::new(AtomicU64::new(0));
    let on_progress = Arc::new(on_progress);
    struct State {
        file: tokio::fs::File,
        sent: u64,
        last_report: Option<Instant>,
    }
    let state = State {
        file,
        sent: 0,
        last_report: None,
    };
    let (moved, report, stop) = (last_move.clone(), on_progress.clone(), cancel.clone());
    let stream = futures::stream::unfold(state, move |mut st| {
        let (moved, report, stop) = (moved.clone(), report.clone(), stop.clone());
        async move {
            if stop.is_cancelled() || st.sent >= total {
                return None;
            }
            let want = CHUNK.min((total - st.sent) as usize);
            let mut buf = vec![0u8; want];
            let mut filled = 0;
            while filled < want {
                match st.file.read(&mut buf[filled..]).await {
                    Ok(0) => break,
                    Ok(n) => filled += n,
                    Err(e) => return Some((Err(e), st)),
                }
            }
            if filled == 0 {
                // shorter than it was a moment ago: end the body early, the length check fails
                return None;
            }
            buf.truncate(filled);
            st.sent += filled as u64;
            moved.store(started.elapsed().as_millis() as u64, Ordering::SeqCst);
            let now = Instant::now();
            let due = st.last_report.is_none_or(|t| now - t >= PROGRESS_EVERY);
            if due || st.sent >= total {
                st.last_report = Some(now);
                // The connection taking the last chunk isn't the store acknowledging it: stay
                // under 100% until the answer arrives.
                report(st.sent.min(total - total / 100), total);
            }
            Some((Ok::<Vec<u8>, std::io::Error>(buf), st))
        }
    });

    let send = http
        .put(url)
        .header(reqwest::header::CONTENT_TYPE, content_type)
        .header(reqwest::header::CONTENT_LENGTH, total)
        .body(reqwest::Body::wrap_stream(stream))
        .send();

    let watchdog = async {
        loop {
            let since = started
                .elapsed()
                .saturating_sub(Duration::from_millis(last_move.load(Ordering::SeqCst)));
            if since >= stall {
                return;
            }
            tokio::time::sleep(stall - since).await;
        }
    };

    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(UploadError::Cancelled),
        () = watchdog => Err(UploadError::Stalled),
        res = send => match res {
            Ok(resp) if resp.status().is_success() => {
                on_progress(total, total);
                Ok(())
            }
            Ok(resp) => Err(UploadError::Rejected(resp.status().as_u16())),
            Err(e) => Err(UploadError::Network(crate::api::failure_text(e))),
        },
    }
}
