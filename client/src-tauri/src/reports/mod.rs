//! Bug and crash reports: gather diagnostics, send them, or save them to a file to pass on by hand.

pub mod crash;
pub mod tail;

use std::future::Future;
use std::path::PathBuf;
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
    match attempt().await {
        Err(ApiError::Network(_) | ApiError::Server(_)) => {
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

/// `pulse-report-YYYYMMDD-HHMMSS.txt` in UTC, from the unix time (no date crate in the client).
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

#[tauri::command]
pub async fn submit_report(app: AppHandle, text: String) -> Result<String, ApiError> {
    let (api, token) = app.state::<Core>().current()?;
    let req = CreateReportRequest {
        kind: ReportKind::Bug,
        text,
        meta: meta(&app, api.base()).await,
        log: tail::log_tail(&app.state::<LogDir>().0, TAIL_BYTES),
    };
    let r = send_with_retry(|| api.create_report(&token, &req), Duration::from_secs(2)).await;
    let r = app.state::<Core>().check(&app, r)?;
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
    let log = tail::log_tail(&app.state::<LogDir>().0, TAIL_BYTES);
    let path = save_dir(&app).join(file_name(unix_now()));
    std::fs::write(&path, file_text(ReportKind::Bug, &text, &meta, &log))
        .map_err(|e| e.to_string())?;
    tracing::info!(path = %path.display(), "bug report saved");
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn ok() -> Result<CreateReportResponse, ApiError> {
        Ok(CreateReportResponse {
            id: "01JABCDEFGHJKMNPQRSTVWXYZ".into(),
        })
    }

    #[tokio::test]
    async fn a_network_error_is_retried_once() {
        let calls = AtomicU32::new(0);
        let r = send_with_retry(
            || {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    if n == 0 {
                        Err(ApiError::Network("blip".into()))
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
                async { Err(ApiError::Server("502".into())) }
            },
            Duration::from_millis(1),
        )
        .await;
        assert!(matches!(r, Err(ApiError::Server(_))));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
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
