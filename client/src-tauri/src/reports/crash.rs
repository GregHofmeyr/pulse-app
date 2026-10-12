//! Crashes and unexpected exits, reported on the next launch.
//!
//! Files in the profile's log dir:
//! - `running`: written at startup, removed on a clean exit. Still there at startup → the last
//!   session ended without one (native crash, force-close, power loss).
//! - `crash.json`: written by the panic hook (first panic of a session).
//! - `pending-report.json`: what startup made of the two above, plus the crashed session's log
//!   tail; kept until it is sent (or rejected, or a week old).

use std::path::Path;

use pulse_protocol::rest::{CreateReportRequest, ReportKind};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::api::ApiError;

const RUNNING: &str = "running";
const CRASH: &str = "crash.json";
const PENDING: &str = "pending-report.json";
pub const MAX_AGE_SECS: u64 = 7 * 86_400;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingFile {
    pub at: u64,
    pub kind: ReportKind,
    pub crash: Option<serde_json::Value>,
    pub log: String,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Sent,
    Keep,
    Drop,
}

pub fn write_panic_marker(dir: &Path, message: &str, location: &str) {
    let path = dir.join(CRASH);
    if path.exists() {
        return; // the first panic of a session is the interesting one
    }
    let body = serde_json::json!({
        "at": super::unix_now(),
        "message": message,
        "location": location,
        "thread": std::thread::current().name().unwrap_or("?"),
    });
    let _ = std::fs::write(path, body.to_string()); // in a panic hook: never fail loudly
}

pub fn mark_running(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(dir.join(RUNNING), super::unix_now().to_string());
}

pub fn clear_running(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(RUNNING));
}

/// At startup, before this session logs anything: turn last session's markers into a pending
/// report carrying its log tail. A newer one replaces an older unsent one.
pub fn detect(dir: &Path, now: u64) {
    let crash: Option<serde_json::Value> = std::fs::read_to_string(dir.join(CRASH))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let ran = dir.join(RUNNING).exists();
    let kind = match (&crash, ran) {
        (Some(_), _) => ReportKind::Crash,
        (None, true) => ReportKind::UnexpectedExit,
        (None, false) => return,
    };
    let pending = PendingFile {
        at: now,
        kind,
        crash,
        log: super::tail::log_tail(dir, super::TAIL_BYTES),
    };
    if let Ok(body) = serde_json::to_string(&pending) {
        let _ = std::fs::write(dir.join(PENDING), body);
    }
    let _ = std::fs::remove_file(dir.join(CRASH));
    clear_running(dir);
}

pub fn load_pending(dir: &Path, now: u64) -> Option<PendingFile> {
    let p: PendingFile =
        serde_json::from_str(&std::fs::read_to_string(dir.join(PENDING)).ok()?).ok()?;
    if now.saturating_sub(p.at) > MAX_AGE_SECS {
        clear_pending(dir);
        return None;
    }
    Some(p)
}

pub fn clear_pending(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(PENDING));
}

/// Sent → delete. Refused (4xx) → delete too: resending won't change the answer. Otherwise keep it
/// for the next launch.
pub fn outcome<T>(r: &Result<T, ApiError>) -> Outcome {
    match r {
        Ok(_) => Outcome::Sent,
        Err(ApiError::Rejected(_)) => Outcome::Drop,
        Err(_) => Outcome::Keep,
    }
}

/// After sign-in: send last session's crash/exit report, if any.
pub async fn send_pending(app: AppHandle, api: crate::api::Api, token: String) {
    let dir = app.state::<super::LogDir>().0.clone();
    let Some(p) = load_pending(&dir, super::unix_now()) else {
        return;
    };
    let mut meta = super::meta(&app, api.base()).await;
    meta["crash"] = p.crash.clone().unwrap_or(serde_json::Value::Null);
    let req = CreateReportRequest {
        kind: p.kind,
        text: String::new(),
        meta,
        log: p.log,
    };
    let r = super::send_with_retry(
        || api.create_report(&token, &req),
        std::time::Duration::from_secs(2),
    )
    .await;
    match outcome(&r) {
        Outcome::Sent => {
            clear_pending(&dir);
            tracing::info!(kind = req.kind.as_str(), "automatic report sent");
            let _ = app.emit("pulse://auto-report-sent", ());
        }
        Outcome::Drop => {
            clear_pending(&dir);
            tracing::warn!(error = ?r.err(), "automatic report refused; dropped");
        }
        Outcome::Keep => {
            tracing::warn!(error = ?r.err(), "automatic report not sent; will retry next launch")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(dir: &Path, text: &str) {
        std::fs::write(dir.join("pulse.2026-10-12.log"), text).unwrap();
    }

    #[test]
    fn clean_exit_leaves_nothing_to_report() {
        let d = tempfile::tempdir().unwrap();
        mark_running(d.path());
        clear_running(d.path());
        detect(d.path(), 1000);
        assert_eq!(load_pending(d.path(), 1000), None);
    }

    #[test]
    fn a_leftover_running_marker_is_an_unexpected_exit_with_the_old_log() {
        let d = tempfile::tempdir().unwrap();
        log(d.path(), "joining voice\n");
        mark_running(d.path());
        detect(d.path(), 1000);
        let p = load_pending(d.path(), 1000).unwrap();
        assert_eq!(p.kind, ReportKind::UnexpectedExit);
        assert_eq!(p.log, "joining voice\n");
        assert!(!d.path().join(RUNNING).exists(), "markers are consumed");
    }

    #[test]
    fn a_panic_marker_wins_over_running_and_the_first_panic_is_kept() {
        let d = tempfile::tempdir().unwrap();
        mark_running(d.path());
        write_panic_marker(d.path(), "index out of bounds", "src/voice/mixer.rs:88");
        write_panic_marker(d.path(), "second panic", "elsewhere.rs:1");
        detect(d.path(), 1000);
        let p = load_pending(d.path(), 1000).unwrap();
        assert_eq!(p.kind, ReportKind::Crash);
        let c = p.crash.unwrap();
        assert_eq!(c["message"], "index out of bounds");
        assert_eq!(c["location"], "src/voice/mixer.rs:88");
    }

    #[test]
    fn panic_marker_write_ignores_a_missing_dir() {
        let d = tempfile::tempdir().unwrap();
        write_panic_marker(&d.path().join("gone/away"), "boom", "x.rs:1"); // must not panic
    }

    #[test]
    fn a_week_old_pending_report_is_dropped() {
        let d = tempfile::tempdir().unwrap();
        mark_running(d.path());
        detect(d.path(), 1000);
        assert!(d.path().join(PENDING).exists());
        assert_eq!(load_pending(d.path(), 1000 + MAX_AGE_SECS + 1), None);
        assert!(!d.path().join(PENDING).exists());
    }

    #[test]
    fn markers_are_per_dir() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        mark_running(a.path());
        mark_running(b.path());
        clear_running(b.path()); // profile B exits cleanly
        detect(a.path(), 1000);
        detect(b.path(), 1000);
        assert!(load_pending(a.path(), 1000).is_some());
        assert!(load_pending(b.path(), 1000).is_none());
    }

    #[test]
    fn pending_dropped_on_rejection_kept_on_network_error() {
        let ok: Result<(), ApiError> = Ok(());
        assert_eq!(outcome(&ok), Outcome::Sent);
        assert_eq!(
            outcome::<()>(&Err(ApiError::Rejected("too large".into()))),
            Outcome::Drop
        );
        assert_eq!(
            outcome::<()>(&Err(ApiError::Network("down".into()))),
            Outcome::Keep
        );
        assert_eq!(
            outcome::<()>(&Err(ApiError::Server("502".into()))),
            Outcome::Keep
        );
        assert_eq!(outcome::<()>(&Err(ApiError::RateLimited)), Outcome::Keep);
        assert_eq!(outcome::<()>(&Err(ApiError::Unauthorized)), Outcome::Keep);
    }
}
