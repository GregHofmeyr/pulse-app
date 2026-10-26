//! Crashes and unexpected exits, reported on the next launch.
//!
//! Files in the profile's log dir:
//! - `running`: written at startup, removed on a clean exit. Still there at startup → the last
//!   session ended without one (native crash, force-close, power loss).
//! - `crash.json`: written by the panic hook (first panic of a session).
//! - `pending-report.json`: what startup made of the two above, plus the crashed session's log
//!   tail; kept until it is sent (or rejected, or a week old). With "Send crash reports
//!   automatically" off it is `held`: never sent on its own, only if the user picks "Send" when
//!   asked (or deleted when they pick "Don't send").

use std::future::Future;
use std::path::Path;

use pulse_protocol::rest::{CreateReportRequest, CreateReportResponse, ReportKind};
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
    /// Captured (or still waiting) while automatic sending was off: only the user's "Send" sends
    /// it. Never cleared once set, so turning the setting back on doesn't release it.
    #[serde(default)]
    pub held: bool,
}

/// What the automatic path did with last session's report.
#[derive(Debug, PartialEq)]
pub enum AutoSend {
    Nothing,
    /// Not sent: kept for the user to decide.
    Held(ReportKind),
    Sent,
    Dropped,
    Kept,
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
        held: false,
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

/// Mark the waiting report (if any) as held: from now on only the user's "Send" sends it.
pub fn hold_pending(dir: &Path) {
    let Ok(raw) = std::fs::read_to_string(dir.join(PENDING)) else {
        return;
    };
    let Ok(mut p) = serde_json::from_str::<PendingFile>(&raw) else {
        // unreadable: it could never be sent anyway
        clear_pending(dir);
        return;
    };
    if p.held {
        return;
    }
    p.held = true;
    if let Ok(body) = serde_json::to_string(&p) {
        let tmp = dir.join(format!("{PENDING}.tmp"));
        if std::fs::write(&tmp, body).is_err() || std::fs::rename(&tmp, dir.join(PENDING)).is_err()
        {
            // can't mark it held: delete it rather than risk an automatic send
            let _ = std::fs::remove_file(tmp);
            clear_pending(dir);
        }
    }
}

/// The automatic path: send last session's report only if automatic sending is on and the report
/// was never held; otherwise hold it (nothing is sent). `send` is called at most once.
pub async fn auto_send<F, Fut>(dir: &Path, now: u64, auto: bool, send: F) -> AutoSend
where
    F: FnOnce(PendingFile) -> Fut,
    Fut: Future<Output = Result<CreateReportResponse, ApiError>>,
{
    let Some(p) = load_pending(dir, now) else {
        return AutoSend::Nothing;
    };
    if !auto || p.held {
        hold_pending(dir);
        return AutoSend::Held(p.kind);
    }
    let r = send(p).await;
    match outcome(&r) {
        Outcome::Sent => {
            clear_pending(dir);
            AutoSend::Sent
        }
        Outcome::Drop => {
            clear_pending(dir);
            tracing::warn!(error = ?r.err(), "automatic report refused; dropped");
            AutoSend::Dropped
        }
        Outcome::Keep => {
            tracing::warn!(error = ?r.err(), "automatic report not sent; will retry next launch");
            AutoSend::Kept
        }
    }
}

/// The user picked "Send" for a held report. Sent → deleted. Refused → deleted (resending won't
/// change the answer) and the error says so. Not sent (network, server down, session expired) →
/// kept, and the error says that too.
pub async fn send_held<F, Fut>(dir: &Path, now: u64, send: F) -> Result<String, ApiError>
where
    F: FnOnce(PendingFile) -> Fut,
    Fut: Future<Output = Result<CreateReportResponse, ApiError>>,
{
    let Some(p) = load_pending(dir, now) else {
        return Err(ApiError::Rejected(
            "There's no saved crash report any more.".into(),
        ));
    };
    let r = send(p).await;
    match outcome(&r) {
        Outcome::Sent => {
            clear_pending(dir);
            let id = r.map(|r| r.id).unwrap_or_default();
            tracing::info!(id = %id, "held crash report sent by the user");
            Ok(super::short(&id))
        }
        Outcome::Drop => {
            clear_pending(dir);
            let e = r.err().map(|e| e.to_string()).unwrap_or_default();
            tracing::warn!(error = %e, "held crash report refused; deleted");
            Err(ApiError::Rejected(format!(
                "The server refused the report ({e}), so it was deleted."
            )))
        }
        Outcome::Keep => {
            let e = match r.err() {
                Some(ApiError::Unauthorized) => "your session has expired".to_string(),
                Some(e) => e.to_string(),
                None => String::new(),
            };
            tracing::warn!(error = %e, "held crash report not sent; kept");
            Err(ApiError::Rejected(format!(
                "Couldn't send the report ({e}). It's kept on this computer: try again, or don't send it."
            )))
        }
    }
}

/// Sent → delete. Refused (4xx) → delete too: resending won't change the answer. Otherwise keep it
/// for the next launch.
pub fn outcome<T>(r: &Result<T, ApiError>) -> Outcome {
    match r {
        Ok(_) => Outcome::Sent,
        Err(ApiError::Rejected(_) | ApiError::Forbidden(_)) => Outcome::Drop,
        Err(_) => Outcome::Keep,
    }
}

/// The request for last session's report: its kind, crash details and log, with this session's meta.
async fn request(app: &AppHandle, server: &str, p: PendingFile) -> CreateReportRequest {
    let mut meta = super::meta(app, server).await;
    meta["crash"] = p.crash.clone().unwrap_or(serde_json::Value::Null);
    CreateReportRequest {
        kind: p.kind,
        text: String::new(),
        meta,
        log: p.log,
    }
}

/// Sign-in can start before the UI listens: hold a notice until it does.
async fn emit_when_listening<S: Serialize + Clone>(app: &AppHandle, event: &str, payload: S) {
    let mut listening = app.state::<crate::commands::Core>().ui_listening();
    let _ = listening.wait_for(|l| *l).await;
    let _ = app.emit(event, payload);
}

/// After sign-in: send last session's crash/exit report, if any and if automatic sending is on.
/// Off (or held): nothing is sent; the UI asks the user instead.
pub async fn send_pending(app: AppHandle, api: crate::api::Api, token: String) {
    let dir = app.state::<super::LogDir>().0.clone();
    let auto = app.state::<super::settings::Store>().auto_send_crashes();
    let send = |p: PendingFile| {
        let (app, api) = (app.clone(), api.clone());
        async move {
            let req = request(&app, api.base(), p).await;
            super::send_with_retry(
                || api.create_report(&token, &req),
                std::time::Duration::from_secs(2),
            )
            .await
        }
    };
    match auto_send(&dir, super::unix_now(), auto, send).await {
        AutoSend::Sent => {
            tracing::info!("automatic report sent");
            emit_when_listening(&app, "pulse://auto-report-sent", ()).await;
        }
        AutoSend::Held(kind) => {
            tracing::info!(kind = kind.as_str(), "crash report held: asking the user");
            emit_when_listening(
                &app,
                "pulse://crash-report-held",
                serde_json::json!({ "kind": kind }),
            )
            .await;
        }
        AutoSend::Nothing | AutoSend::Dropped | AutoSend::Kept => {}
    }
}

/// The user picked "Send" for a held report.
#[tauri::command]
pub async fn send_held_report(app: AppHandle) -> Result<String, ApiError> {
    let (api, token) = app.state::<crate::commands::Core>().current()?;
    let dir = app.state::<super::LogDir>().0.clone();
    let send = |p: PendingFile| {
        let app = app.clone();
        let api = api.clone();
        async move {
            let req = request(&app, api.base(), p).await;
            super::send_with_retry(
                || api.create_report(&token, &req),
                std::time::Duration::from_secs(2),
            )
            .await
        }
    };
    send_held(&dir, super::unix_now(), send).await
}

/// The user picked "Don't send": delete the held report.
#[tauri::command]
pub fn discard_held_report(log_dir: tauri::State<'_, super::LogDir>) {
    clear_pending(&log_dir.0);
    tracing::info!("held crash report deleted by the user");
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

    fn sent() -> Result<CreateReportResponse, ApiError> {
        Ok(CreateReportResponse {
            id: "01JABCDEFGHJKMNPQRSTVWXYZ".into(),
        })
    }

    /// A crashed session's report waiting at `dir` (as startup leaves it).
    fn crashed(dir: &Path) {
        log(dir, "mixer underrun\n");
        mark_running(dir);
        write_panic_marker(dir, "boom", "x.rs:1");
        detect(dir, 1000);
    }

    #[tokio::test]
    async fn on_sends_the_report_and_deletes_it() {
        let d = tempfile::tempdir().unwrap();
        crashed(d.path());
        let mut got = None;
        let r = auto_send(d.path(), 1000, true, |p| {
            got = Some(p);
            async { sent() }
        })
        .await;
        assert_eq!(r, AutoSend::Sent);
        let p = got.expect("sent");
        assert_eq!(
            (p.kind, p.log.as_str()),
            (ReportKind::Crash, "mixer underrun\n")
        );
        assert_eq!(load_pending(d.path(), 1000), None);
    }

    #[tokio::test]
    async fn off_sends_nothing_and_keeps_the_report_held() {
        let d = tempfile::tempdir().unwrap();
        crashed(d.path());
        let mut called = false;
        let r = auto_send(d.path(), 1000, false, |_| {
            called = true;
            async { sent() }
        })
        .await;
        assert!(!called, "nothing may be uploaded while off");
        assert_eq!(r, AutoSend::Held(ReportKind::Crash));
        let p = load_pending(d.path(), 1000).expect("kept locally");
        assert!(p.held);
        assert_eq!(p.log, "mixer underrun\n");
    }

    #[tokio::test]
    async fn a_held_report_is_not_sent_even_after_turning_it_back_on() {
        let d = tempfile::tempdir().unwrap();
        crashed(d.path());
        auto_send(d.path(), 1000, false, |_| async { sent() }).await;
        let mut called = false;
        let r = auto_send(d.path(), 1001, true, |_| {
            called = true;
            async { sent() }
        })
        .await;
        assert!(!called);
        assert_eq!(r, AutoSend::Held(ReportKind::Crash));
        assert!(load_pending(d.path(), 1001).is_some());
    }

    #[tokio::test]
    async fn a_report_whose_automatic_send_failed_is_not_retried_after_turning_it_off() {
        let d = tempfile::tempdir().unwrap();
        crashed(d.path());
        let r = auto_send(d.path(), 1000, true, |_| async {
            Err(ApiError::Network(crate::api::CONNECT_FAILED.into()))
        })
        .await;
        assert_eq!(r, AutoSend::Kept);
        assert!(!load_pending(d.path(), 1000).unwrap().held);
        // the user turns it off before the next launch (set_report_settings holds it)
        hold_pending(d.path());
        let mut called = false;
        let r = auto_send(d.path(), 2000, true, |_| {
            called = true;
            async { sent() }
        })
        .await;
        assert!(!called);
        assert_eq!(r, AutoSend::Held(ReportKind::Crash));
    }

    #[tokio::test]
    async fn the_setting_is_read_at_send_time_too() {
        // captured while on (not held), but off by the time it would go: still not sent
        let d = tempfile::tempdir().unwrap();
        crashed(d.path());
        let mut called = false;
        auto_send(d.path(), 1000, false, |_| {
            called = true;
            async { sent() }
        })
        .await;
        assert!(!called);
    }

    #[tokio::test]
    async fn nothing_waiting_sends_nothing() {
        let d = tempfile::tempdir().unwrap();
        let mut called = false;
        let r = auto_send(d.path(), 1000, true, |_| {
            called = true;
            async { sent() }
        })
        .await;
        assert_eq!((r, called), (AutoSend::Nothing, false));
    }

    #[test]
    fn holding_keeps_the_contents_and_needs_a_report() {
        let d = tempfile::tempdir().unwrap();
        hold_pending(d.path());
        assert!(!d.path().join(PENDING).exists(), "nothing to hold: no file");
        crashed(d.path());
        let before = load_pending(d.path(), 1000).unwrap();
        hold_pending(d.path());
        let after = load_pending(d.path(), 1000).unwrap();
        assert!(after.held);
        assert_eq!(
            PendingFile {
                held: false,
                ..after
            },
            before
        );
    }

    #[test]
    fn a_report_saved_by_an_older_version_is_not_held() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join(PENDING),
            r#"{"at":1000,"kind":"crash","crash":null,"log":"x"}"#,
        )
        .unwrap();
        assert!(!load_pending(d.path(), 1000).unwrap().held);
    }

    #[tokio::test]
    async fn choosing_send_sends_it_and_deletes_it() {
        let d = tempfile::tempdir().unwrap();
        crashed(d.path());
        hold_pending(d.path());
        let mut got = None;
        let r = send_held(d.path(), 1000, |p| {
            got = Some(p);
            async { sent() }
        })
        .await;
        assert_eq!(r.unwrap(), "TVWXYZ");
        assert_eq!(got.unwrap().kind, ReportKind::Crash);
        assert!(!d.path().join(PENDING).exists());
    }

    #[tokio::test]
    async fn choosing_send_when_it_fails_keeps_it_and_says_so() {
        let d = tempfile::tempdir().unwrap();
        crashed(d.path());
        hold_pending(d.path());
        for e in [
            ApiError::Network(crate::api::CONNECT_FAILED.into()),
            ApiError::Server("502 Bad Gateway".into()),
            ApiError::Unauthorized,
        ] {
            let r = send_held(d.path(), 1000, |_| async { Err(e) }).await;
            let t = r.unwrap_err().to_string();
            assert!(t.contains("kept"), "{t}");
            let p = load_pending(d.path(), 1000).expect("kept");
            assert!(
                p.held,
                "still held: a failed choice doesn't make it automatic"
            );
        }
    }

    #[tokio::test]
    async fn choosing_send_when_the_server_refuses_it_deletes_it_and_says_so() {
        let d = tempfile::tempdir().unwrap();
        crashed(d.path());
        hold_pending(d.path());
        let r = send_held(d.path(), 1000, |_| async {
            Err(ApiError::Rejected("report too large".into()))
        })
        .await;
        let t = r.unwrap_err().to_string();
        assert!(t.contains("too large") && t.contains("deleted"), "{t}");
        assert!(!d.path().join(PENDING).exists());
    }

    #[tokio::test]
    async fn choosing_send_with_nothing_saved_sends_nothing() {
        let d = tempfile::tempdir().unwrap();
        let mut called = false;
        let r = send_held(d.path(), 1000, |_| {
            called = true;
            async { sent() }
        })
        .await;
        assert!(r.is_err());
        assert!(!called);
    }

    #[test]
    fn choosing_dont_send_deletes_it() {
        let d = tempfile::tempdir().unwrap();
        crashed(d.path());
        hold_pending(d.path());
        clear_pending(d.path());
        assert!(!d.path().join(PENDING).exists());
        assert_eq!(load_pending(d.path(), 1000), None);
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
