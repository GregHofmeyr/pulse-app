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
/// Panic messages can be huge (`assert_eq!` on a big value); the report keeps the start.
pub const MAX_MESSAGE_BYTES: usize = 2048;
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
    let message = if message.len() > MAX_MESSAGE_BYTES {
        let mut cut = MAX_MESSAGE_BYTES;
        while !message.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}…", &message[..cut])
    } else {
        message.to_string()
    };
    let body = serde_json::json!({
        "at": super::unix_now(),
        "message": message,
        "location": location,
        "thread": std::thread::current().name().unwrap_or("?"),
    });
    let _ = std::fs::write(path, body.to_string()); // in a panic hook: never fail loudly
}

pub fn mark_running(dir: &Path) {
    write_running(dir, super::unix_now(), env!("CARGO_PKG_VERSION"));
}

fn write_running(dir: &Path, at: u64, version: &str) {
    let _ = std::fs::create_dir_all(dir);
    let body = serde_json::json!({ "at": at, "version": version });
    let _ = std::fs::write(dir.join(RUNNING), body.to_string());
}

/// When the machine last booted (unix seconds), if we can tell.
pub fn boot_time() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string("/proc/stat").ok()?;
        stat.lines()
            .find_map(|l| l.strip_prefix("btime "))
            .and_then(|v| v.trim().parse().ok())
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetTickCount64() -> u64;
        }
        // SAFETY: no arguments, no preconditions; milliseconds since boot.
        let up_ms = unsafe { GetTickCount64() };
        Some(super::unix_now().saturating_sub(up_ms / 1000))
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        None
    }
}

/// A `running` marker left by a session that ended without a clean exit, unless the cause is
/// innocent: a different app version (the installer closed the old one) or a reboot since (Windows
/// shutdown / logoff kills apps without an exit event). Unreadable markers count as innocent.
fn ended_unexpectedly(dir: &Path, version: &str, booted_at: Option<u64>) -> bool {
    let Ok(raw) = std::fs::read_to_string(dir.join(RUNNING)) else {
        return false;
    };
    let Ok(m) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    let at = m["at"].as_u64().unwrap_or(0);
    m["version"].as_str() == Some(version) && booted_at.is_none_or(|b| at >= b)
}

pub fn clear_running(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(RUNNING));
}

/// At startup, before this session logs anything: turn last session's markers into a pending
/// report carrying its log tail. A newer one replaces an older unsent one.
pub fn detect(dir: &Path, now: u64) {
    detect_with(dir, now, env!("CARGO_PKG_VERSION"), boot_time());
}

/// `detect` with the running version and boot time passed in (tests).
pub fn detect_with(dir: &Path, now: u64, version: &str, booted_at: Option<u64>) {
    let crash: Option<serde_json::Value> = std::fs::read_to_string(dir.join(CRASH))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let ran = ended_unexpectedly(dir, version, booted_at);
    let kind = match (&crash, ran) {
        (Some(_), _) => ReportKind::Crash,
        (None, true) => ReportKind::UnexpectedExit,
        (None, false) => {
            clear_running(dir); // a stale marker from an upgrade or reboot
            return;
        }
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
    fn a_huge_panic_message_is_cut_to_fit_the_report() {
        // e.g. assert_eq! on a big Vec: the report's meta must stay under the server's 16 KB
        let d = tempfile::tempdir().unwrap();
        let msg = "é".repeat(10_000); // 20 KB, multi-byte chars
        write_panic_marker(d.path(), &msg, "x.rs:1");
        detect(d.path(), 1000);
        let p = load_pending(d.path(), 1000).unwrap();
        let kept = p.crash.unwrap()["message"].as_str().unwrap().to_string();
        assert!(kept.len() <= MAX_MESSAGE_BYTES + 4, "{} bytes", kept.len());
        assert!(kept.ends_with('…'));
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
    fn an_upgrade_is_not_an_unexpected_exit() {
        // the installer closes the old version without a clean exit
        let d = tempfile::tempdir().unwrap();
        write_running(d.path(), 1000, "0.0.9");
        detect_with(d.path(), 2000, "0.1.0", None);
        assert_eq!(load_pending(d.path(), 2000), None);
        assert!(
            !d.path().join(RUNNING).exists(),
            "the stale marker is cleared"
        );
    }

    #[test]
    fn a_reboot_since_is_not_an_unexpected_exit() {
        // Windows shutdown / logoff with Pulse open kills it without a clean exit
        let d = tempfile::tempdir().unwrap();
        write_running(d.path(), 1000, "0.1.0");
        detect_with(d.path(), 3000, "0.1.0", Some(2000));
        assert_eq!(load_pending(d.path(), 3000), None);

        write_running(d.path(), 2500, "0.1.0");
        detect_with(d.path(), 3000, "0.1.0", Some(2000));
        assert_eq!(
            load_pending(d.path(), 3000).map(|p| p.kind),
            Some(ReportKind::UnexpectedExit)
        );
    }

    #[test]
    fn a_crash_is_reported_even_across_an_upgrade() {
        let d = tempfile::tempdir().unwrap();
        write_running(d.path(), 1000, "0.0.9");
        write_panic_marker(d.path(), "boom", "x.rs:1");
        detect_with(d.path(), 2000, "0.1.0", Some(1500));
        assert_eq!(
            load_pending(d.path(), 2000).map(|p| p.kind),
            Some(ReportKind::Crash)
        );
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
