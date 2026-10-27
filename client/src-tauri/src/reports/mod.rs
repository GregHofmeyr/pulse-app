//! Bug and crash reports: gather diagnostics, send them, or save them to a file to pass on by hand.

pub mod crash;
pub mod settings;
pub mod tail;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pulse_protocol::rest::{CreateReportRequest, CreateReportResponse, ReportKind};
use tauri::{AppHandle, Manager};

use crate::api::ApiError;
use crate::commands::Core;
use crate::voice::VoiceManager;

/// The profile's log dir (reports read their log tail from it; crash markers live there).
pub struct LogDir(pub PathBuf);

/// How much of the log a report carries.
pub const TAIL_BYTES: usize = 256 * 1024;

/// Try, and on a network or server error try once more after `delay`. Refusals (4xx, 429) aren't
/// retried: sending the same thing again won't change the answer.
pub async fn send_with_retry<F, Fut>(
    mut attempt: F,
    delay: Duration,
) -> Result<CreateReportResponse, ApiError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<CreateReportResponse, ApiError>>,
{
    // Only when it certainly didn't arrive: a retry after a timeout could file the report twice.
    match attempt().await {
        Err(e) if e.never_arrived() => {
            tokio::time::sleep(delay).await;
            attempt().await
        }
        r => r,
    }
}

pub fn short(id: &str) -> String {
    id[id.len().saturating_sub(6)..].to_string()
}

/// The saved-file form of a report: readable as-is in Notepad / WhatsApp.
pub fn file_text(kind: ReportKind, text: &str, meta: &serde_json::Value, log: &str) -> String {
    let title = match kind {
        ReportKind::Bug => "Pulse bug report",
        ReportKind::Crash => "Pulse crash report",
        ReportKind::UnexpectedExit => "Pulse unexpected-exit report",
    };
    format!(
        "{title}\n\n{}\n\n{}\n\n----- app log -----\n{log}",
        if text.trim().is_empty() {
            "(no text)"
        } else {
            text.trim()
        },
        serde_json::to_string_pretty(meta).unwrap_or_default(),
    )
}

/// Replace `dir/name` whole: write a temp file beside it, flush it to disk, then rename it over
/// the old one. A crash or power loss leaves the old file or the new one, never half of one (the
/// rename itself isn't synced: right after a power loss it may be the old one).
pub fn write_atomic(dir: &Path, name: &str, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = dir.join(format!("{name}.tmp"));
    let r = std::fs::File::create(&tmp)
        .and_then(|mut f| {
            f.write_all(contents)?;
            f.sync_all()
        })
        .and_then(|()| std::fs::rename(&tmp, dir.join(name)));
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

/// `pulse-report-YYYYMMDD-HHMMSS.txt` in UTC, from the unix time (no date crate in the client).
/// Write to `dir/name`, or `name-2`, `-3`… if taken: two saves in the same second keep both.
pub fn write_new(dir: &Path, name: &str, contents: &str) -> std::io::Result<PathBuf> {
    use std::io::Write;
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    for n in 1..100 {
        let candidate = match n {
            1 => name.to_string(),
            n if ext.is_empty() => format!("{stem}-{n}"),
            n => format!("{stem}-{n}.{ext}"),
        };
        let path = dir.join(candidate);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                f.write_all(contents.as_bytes())?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("too many reports saved this second"))
}

pub fn file_name(unix_secs: u64) -> String {
    let (days, secs) = (unix_secs / 86_400, unix_secs % 86_400);
    // civil-from-days (Howard Hinnant), valid for 1970+
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "pulse-report-{y:04}{m:02}{d:02}-{:02}{:02}{:02}.txt",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Everything a report carries besides the text and log.
pub async fn meta(app: &AppHandle, server: &str) -> serde_json::Value {
    let commit = option_env!("PULSE_COMMIT")
        .map(|c| &c[..c.len().min(7)])
        .unwrap_or("dev");
    serde_json::json!({
        "app_version": env!("CARGO_PKG_VERSION"),
        "commit": commit,
        "os": os_info::get().to_string(),
        "server": server,
        "gateway": format!("{:?}", app.state::<Core>().conn()),
        "voice": app.state::<VoiceManager>().diagnostics().await,
    })
}

/// Where "Save report" writes: Downloads, else Desktop, else the log dir.
pub fn save_dir(app: &AppHandle) -> PathBuf {
    let p = app.path();
    p.download_dir()
        .or_else(|_| p.desktop_dir())
        .unwrap_or_else(|_| app.state::<LogDir>().0.clone())
}

/// A 401 here means the session expired. Say that (not "wrong password"), and don't sign out yet:
/// that would close the report and lose the text before it can be saved.
fn expired_or(e: ApiError) -> ApiError {
    match e {
        ApiError::Unauthorized => ApiError::Rejected(
            "Your session has expired, so this can't be sent. Save the report instead, then log in again."
                .into(),
        ),
        other => other,
    }
}

/// The log tail, read on the blocking pool (file I/O doesn't belong on an async worker).
async fn read_tail(app: &AppHandle) -> String {
    let dir = app.state::<LogDir>().0.clone();
    tokio::task::spawn_blocking(move || tail::log_tail(&dir, TAIL_BYTES))
        .await
        .unwrap_or_default()
}

#[tauri::command]
pub async fn submit_report(app: AppHandle, text: String) -> Result<String, ApiError> {
    let (api, token) = app.state::<Core>().current()?;
    let req = CreateReportRequest {
        kind: ReportKind::Bug,
        text,
        meta: meta(&app, api.base()).await,
        log: read_tail(&app).await,
    };
    let r = match send_with_retry(|| api.create_report(&token, &req), Duration::from_secs(2)).await
    {
        Err(ApiError::Unauthorized) => return Err(expired_or(ApiError::Unauthorized)),
        r => app.state::<Core>().check(&app, r)?,
    };
    tracing::info!(id = %r.id, "bug report sent");
    Ok(short(&r.id))
}

#[tauri::command]
pub async fn save_report(
    app: AppHandle,
    text: String,
    server_url: Option<String>,
) -> Result<String, String> {
    let server = match server_url {
        Some(s) => s,
        None => app
            .state::<Core>()
            .current()
            .map(|(a, _)| a.base().to_string())
            .unwrap_or_default(),
    };
    let meta = meta(&app, &server).await;
    let log = read_tail(&app).await;
    let path = write_new(
        &save_dir(&app),
        &file_name(unix_now()),
        &file_text(ReportKind::Bug, &text, &meta, &log),
    )
    .map_err(|e| e.to_string())?;
    tracing::info!(path = %crate::logging::redact_home(&path), "bug report saved");
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_twice_in_a_second_keeps_both() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_new(dir.path(), "pulse-report-x.txt", "first").unwrap();
        let b = write_new(dir.path(), "pulse-report-x.txt", "second").unwrap();
        assert_ne!(a, b);
        assert_eq!(b.file_name().unwrap(), "pulse-report-x-2.txt");
        assert_eq!(std::fs::read_to_string(a).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(b).unwrap(), "second");
    }

    #[test]
    fn an_expired_session_says_so_and_points_at_saving() {
        let e = expired_or(ApiError::Unauthorized);
        let t = e.to_string();
        assert!(t.contains("expired") && t.contains("Save"), "{t}");
        assert!(!t.contains("password"), "{t}");
        assert!(matches!(
            expired_or(ApiError::RateLimited),
            ApiError::RateLimited
        ));
    }
    use std::sync::atomic::{AtomicU32, Ordering};

    fn ok() -> Result<CreateReportResponse, ApiError> {
        Ok(CreateReportResponse {
            id: "01JABCDEFGHJKMNPQRSTVWXYZ".into(),
        })
    }

    #[tokio::test]
    async fn a_connection_failure_is_retried_once() {
        let calls = AtomicU32::new(0);
        let r = send_with_retry(
            || {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    if n == 0 {
                        Err(ApiError::Network(crate::api::CONNECT_FAILED.into()))
                    } else {
                        ok()
                    }
                }
            },
            Duration::from_millis(1),
        )
        .await;
        assert!(r.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn gives_up_after_the_second_failure() {
        let calls = AtomicU32::new(0);
        let r = send_with_retry(
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Err(ApiError::Server("502 Bad Gateway".into())) }
            },
            Duration::from_millis(1),
        )
        .await;
        assert!(matches!(r, Err(ApiError::Server(_))));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// A timeout or a 500 may come after the server stored the report: retrying would file it twice.
    #[tokio::test]
    async fn failures_that_may_have_arrived_are_not_retried() {
        for e in [
            ApiError::Network("it took too long to answer".into()),
            ApiError::Server("500 Internal Server Error".into()),
            ApiError::Server("504 Gateway Timeout".into()),
        ] {
            let calls = AtomicU32::new(0);
            let e2 = e.to_string();
            let r = send_with_retry(
                || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let e = match &e {
                        ApiError::Network(t) => ApiError::Network(t.clone()),
                        ApiError::Server(t) => ApiError::Server(t.clone()),
                        _ => unreachable!(),
                    };
                    async { Err(e) }
                },
                Duration::from_millis(1),
            )
            .await;
            assert!(r.is_err());
            assert_eq!(calls.load(Ordering::SeqCst), 1, "{e2}");
        }
    }

    #[tokio::test]
    async fn a_refusal_is_not_retried() {
        let calls = AtomicU32::new(0);
        let r = send_with_retry(
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Err(ApiError::Rejected("say what happened".into())) }
            },
            Duration::from_millis(1),
        )
        .await;
        assert!(matches!(r, Err(ApiError::Rejected(_))));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn file_text_has_the_text_details_and_log() {
        let meta = serde_json::json!({ "app_version": "0.1.0", "os": "Windows 11" });
        let t = file_text(ReportKind::Bug, "voice cut out", &meta, "log line\n");
        assert!(t.starts_with("Pulse bug report\n"), "{t}");
        assert!(t.contains("voice cut out"));
        assert!(t.contains("\"os\": \"Windows 11\""));
        assert!(t.ends_with("----- app log -----\nlog line\n"), "{t}");
    }

    #[test]
    fn short_is_the_last_six() {
        assert_eq!(short("01JABCDEFGHJKMNPQRSTVWXYZ"), "TVWXYZ");
    }

    #[test]
    fn file_name_is_utc_date_and_time() {
        // 2026-10-12 13:25:07 UTC
        assert_eq!(file_name(1_791_811_507), "pulse-report-20261012-132507.txt");
    }
}
