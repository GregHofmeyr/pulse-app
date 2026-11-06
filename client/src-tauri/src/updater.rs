//! App updates (#82). Tauri's updater, driven from here (the page only calls our commands): the signed-in server's
//! `/updates/latest.json`, asked with the session token, names a newer signed installer.
//!
//! - At launch the check starts in `setup` (before the page loads); the page waits at most [`LAUNCH_WAIT`] for it and
//!   shows "Updating Pulse…" when it found one.
//! - While running: after each login's first Ready and every [`RECHECK_EVERY`]; an offer goes to the page as
//!   [`UPDATE_EVENT`] ("Restart to update", never during a call: [`may_restart`]).
//! - Release Windows builds only ([`updates_enabled`]). A failed check or install logs why and leaves Pulse as it is.
//! - The token only ever goes to the server it belongs to: the endpoint is that server, and a download URL on any
//!   other origin is refused ([`same_origin`]) before anything is fetched.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use semver::Version;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::watch;
use url::Url;

/// How long the page waits at launch for the check that started in `setup`.
pub const LAUNCH_WAIT: Duration = Duration::from_secs(4);
/// How long one check (the manifest request) may take. Never the updater's own `.timeout()`: that bounds the
/// installer download too.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(4);
/// The longest the installer download may go without receiving anything before it fails.
pub const DOWNLOAD_STALL: Duration = Duration::from_secs(30);
/// While running, how often to look again.
pub const RECHECK_EVERY: Duration = Duration::from_secs(30 * 60);
/// A running check found a newer version: [`Offer`].
pub const UPDATE_EVENT: &str = "pulse://update";
/// Download progress: [`Progress`].
pub const PROGRESS_EVENT: &str = "pulse://update-progress";

/// Release Windows builds update; Linux, debug builds and `PULSE_NO_UPDATE=1` never ask.
pub fn updates_enabled() -> bool {
    cfg!(windows) && !cfg!(debug_assertions) && std::env::var_os("PULSE_NO_UPDATE").is_none()
}

/// Offer only a strictly newer version (the updater never downgrades).
pub fn should_offer(current: &Version, offered: &Version) -> bool {
    offered > current
}

/// The download comes from the server the session belongs to (same scheme, host and port).
pub fn same_origin(server: &Url, download: &Url) -> bool {
    server.scheme() == download.scheme()
        && server.host_str().is_some()
        && server.host_str() == download.host_str()
        && server.port_or_known_default() == download.port_or_known_default()
}

/// "Restart to update" while in a call (or streaming, which needs one): refused.
#[derive(Debug, PartialEq, Eq)]
pub struct RestartRefused;

impl std::fmt::Display for RestartRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Finish your call first")
    }
}

pub fn may_restart(in_call: bool) -> Result<(), RestartRefused> {
    if in_call { Err(RestartRefused) } else { Ok(()) }
}

/// A newer version the page can install.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    pub version: String,
}

/// How far the installer download is.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

/// The launch check: still running, or what it found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchCheck {
    Pending,
    Done(Option<Offer>),
}

/// What the launch check found, waiting at most `wait` for it.
pub async fn wait_launch(rx: &mut watch::Receiver<LaunchCheck>, wait: Duration) -> Option<Offer> {
    match tokio::time::timeout(wait, rx.wait_for(|c| matches!(c, LaunchCheck::Done(_)))).await {
        Ok(Ok(c)) => match &*c {
            LaunchCheck::Done(offer) => offer.clone(),
            LaunchCheck::Pending => None,
        },
        _ => None,
    }
}

/// The updater's state in the app (managed).
pub struct Updates {
    launch: watch::Sender<LaunchCheck>,
    /// The newest update a check found, ready to install.
    #[cfg(windows)]
    found: Mutex<Option<tauri_plugin_updater::Update>>,
    installing: AtomicBool,
    /// The login whose first Ready was checked already (its token): one check per login.
    checked_login: Mutex<Option<String>>,
    /// Runs just before the installer takes over and the process ends (Windows): crash marker, log flush.
    #[cfg_attr(not(windows), allow(dead_code))]
    before_exit: Arc<dyn Fn() + Send + Sync>,
}

impl Updates {
    pub fn new(before_exit: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self {
            launch: watch::channel(LaunchCheck::Pending).0,
            #[cfg(windows)]
            found: Mutex::new(None),
            installing: AtomicBool::new(false),
            checked_login: Mutex::new(None),
            before_exit,
        }
    }

    /// Ask `server` (with `token`) whether there's a newer version; remembers it for [`update_install`].
    async fn check(
        &self,
        app: &AppHandle,
        server: &str,
        token: &str,
    ) -> Result<Option<Offer>, String> {
        if !updates_enabled() {
            return Ok(None);
        }
        #[cfg(windows)]
        {
            let found = check_server(app, server, token, self.before_exit.clone()).await?;
            let offer = found.as_ref().map(|u| Offer {
                version: u.version.clone(),
            });
            if let Some(u) = found {
                tracing::info!(version = %u.version, "update: a newer version is available");
                *self.found.lock().unwrap() = Some(u);
            }
            Ok(offer)
        }
        #[cfg(not(windows))]
        {
            let _ = (app, server, token);
            Ok(None)
        }
    }
}

/// One check against `server`: `Ok(None)` = nothing newer (or no manifest, or not signed in there any more).
#[cfg(windows)]
async fn check_server(
    app: &AppHandle,
    server: &str,
    token: &str,
    before_exit: Arc<dyn Fn() + Send + Sync>,
) -> Result<Option<tauri_plugin_updater::Update>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let base = Url::parse(server).map_err(|e| format!("bad server address: {e}"))?;
    let endpoint = base
        .join("updates/latest.json")
        .map_err(|e| format!("bad server address: {e}"))?;
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .and_then(|b| b.header("Authorization", format!("Bearer {token}")))
        .map_err(|e| format!("updater setup: {e}"))?
        .version_comparator(|current, release| should_offer(&current, &release.version))
        // A download that stalls fails (and Pulse carries on) instead of leaving "Updating Pulse…" up for good; a
        // per-read limit, so a slow but moving download of any size still finishes.
        .configure_client(|c| c.read_timeout(DOWNLOAD_STALL))
        .on_before_exit(move || before_exit())
        .build()
        .map_err(|e| format!("updater setup: {e}"))?;
    let found = match tokio::time::timeout(CHECK_TIMEOUT, updater.check()).await {
        Err(_) => return Err("the update check timed out".into()),
        // a non-2xx answer (401: signed out there, 404: nothing published) ends as "release not found"
        Ok(Err(tauri_plugin_updater::Error::ReleaseNotFound)) => {
            tracing::info!("update: the server offers no update");
            return Ok(None);
        }
        Ok(Err(e)) => return Err(format!("the update check failed: {e}")),
        Ok(Ok(found)) => found,
    };
    match found {
        Some(u) if !same_origin(&base, &u.download_url) => Err(format!(
            "refusing an update from another host ({})",
            u.download_url.host_str().unwrap_or("none")
        )),
        found => Ok(found),
    }
}

/// The signed-in session the core holds now: (server, token).
fn current_session(app: &AppHandle) -> Option<(String, String)> {
    app.try_state::<crate::commands::Core>()
        .and_then(|c| c.current().ok())
        .map(|(api, token)| (api.base().to_string(), token))
}

/// At startup (from `setup`): check against the stored session, so the page usually has the answer by the time it
/// asks. Signed out = nothing to ask.
pub fn start_launch_check(app: &AppHandle, store: crate::session::Store) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let updates = app.state::<Updates>();
        let offer = if updates_enabled() {
            // keychain reads block: not on the runtime's threads
            let stored = tauri::async_runtime::spawn_blocking(move || store.load())
                .await
                .ok()
                .flatten();
            match stored {
                None => None,
                Some((server, token)) => match updates.check(&app, &server, &token).await {
                    Ok(offer) => offer,
                    Err(e) => {
                        tracing::warn!(error = %e, "update: launch check failed; starting this version");
                        None
                    }
                },
            }
        } else {
            None
        };
        let _ = updates.launch.send(LaunchCheck::Done(offer));
    });
}

/// A running check: an offer goes to the page ("Restart to update").
async fn check_running(app: &AppHandle, why: &str) {
    let Some((server, token)) = current_session(app) else {
        return;
    };
    let updates = app.state::<Updates>();
    if updates.installing.load(Ordering::SeqCst) {
        return;
    }
    match updates.check(app, &server, &token).await {
        Ok(Some(offer)) => {
            let _ = app.emit(UPDATE_EVENT, &offer);
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, why, "update: check failed"),
    }
}

/// The gateway's Ready (commands::forward): the first one of each login checks (a login after a signed-out start,
/// or another account).
pub fn after_ready(app: &AppHandle, token: &str) {
    if !updates_enabled() {
        return;
    }
    let Some(updates) = app.try_state::<Updates>() else {
        return;
    };
    {
        let mut checked = updates.checked_login.lock().unwrap();
        if checked.as_deref() == Some(token) {
            return;
        }
        *checked = Some(token.to_string());
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move { check_running(&app, "login").await });
}

/// Every [`RECHECK_EVERY`] while Pulse runs.
pub fn spawn_periodic(app: &AppHandle) {
    if !updates_enabled() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick =
            tokio::time::interval_at(tokio::time::Instant::now() + RECHECK_EVERY, RECHECK_EVERY);
        loop {
            tick.tick().await;
            check_running(&app, "periodic").await;
        }
    });
}

/// What the launch check found (waits at most [`LAUNCH_WAIT`] for it); `None` = start normally.
#[tauri::command]
pub async fn update_launch(updates: tauri::State<'_, Updates>) -> Result<Option<Offer>, String> {
    let mut rx = updates.launch.subscribe();
    Ok(wait_launch(&mut rx, LAUNCH_WAIT).await)
}

/// A fresh check against the signed-in server (the forced "update required" screen); `None` = nothing newer yet.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<Option<Offer>, String> {
    let Some((server, token)) = current_session(&app) else {
        return Ok(None);
    };
    app.state::<Updates>()
        .check(&app, &server, &token)
        .await
        .inspect_err(|e| tracing::warn!(error = %e, "update: check failed"))
}

/// Download (with [`PROGRESS_EVENT`]s) and install the update a check found. On Windows the installer takes over and
/// the process ends here. `restart`: asked for from the title bar, refused during a call.
#[tauri::command]
pub async fn update_install(app: AppHandle, restart: bool) -> Result<(), String> {
    if restart {
        let voice = app.state::<crate::voice::VoiceManager>();
        let in_call = voice.current_channel().await.is_some() || voice.rejoining().is_some();
        may_restart(in_call).map_err(|e| e.to_string())?;
    }
    #[cfg(windows)]
    {
        let updates = app.state::<Updates>();
        let Some(update) = updates.found.lock().unwrap().clone() else {
            return Err("there's no update to install".into());
        };
        if updates.installing.swap(true, Ordering::SeqCst) {
            return Err("already updating".into());
        }
        tracing::info!(version = %update.version, restart, "update: downloading");
        let result = download_and_install(&app, &update).await;
        updates.installing.store(false, Ordering::SeqCst);
        result.inspect_err(|e| tracing::warn!(error = %e, version = %update.version, "update: install failed; staying on this version"))
    }
    // Only the Windows app updates itself (its checks never offer anything elsewhere).
    #[cfg(not(windows))]
    Err("there's no update to install".into())
}

#[cfg(windows)]
async fn download_and_install(
    app: &AppHandle,
    update: &tauri_plugin_updater::Update,
) -> Result<(), String> {
    let mut downloaded = 0u64;
    let mut last_emit: Option<std::time::Instant> = None;
    let bytes = update
        .download(
            |chunk, total| {
                downloaded += chunk as u64;
                // a few updates a second are plenty for a progress bar
                let due = last_emit.is_none_or(|t| t.elapsed() >= Duration::from_millis(100));
                if due || total == Some(downloaded) {
                    last_emit = Some(std::time::Instant::now());
                    let _ = app.emit(PROGRESS_EVENT, Progress { downloaded, total });
                }
            },
            || tracing::info!("update: downloaded, verifying the signature"),
        )
        .await
        .map_err(|e| format!("the download failed: {e}"))?;
    tracing::info!(version = %update.version, "update: installing; Pulse will close and reopen");
    // The installer's exit skips RunEvent::Exit: wind down now, so no ffmpeg holds the files it replaces.
    crate::wind_down(app).await;
    update
        .install(bytes)
        .map_err(|e| format!("the install failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }
    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn offers_only_newer() {
        assert!(should_offer(&v("0.2.0"), &v("0.2.1")));
        assert!(!should_offer(&v("0.2.0"), &v("0.2.0")));
        assert!(!should_offer(&v("0.2.0"), &v("0.1.9")));
        assert!(should_offer(&v("0.9.0"), &v("0.10.0")));
        assert!(should_offer(&v("0.2.9"), &v("1.0.0")));
    }

    #[test]
    fn refuses_foreign_download_url() {
        let server = u("https://pulsechat.co.za");
        assert!(same_origin(
            &server,
            &u("https://pulsechat.co.za/updates/Pulse_0.2.1_x64-setup.exe")
        ));
        assert!(same_origin(
            &server,
            &u("https://pulsechat.co.za:443/updates/Pulse_0.2.1_x64-setup.exe")
        ));
        for bad in [
            "https://github.com/GregHofmeyr/pulse-app/releases/download/v0.2.1/Pulse_0.2.1_x64-setup.exe",
            "http://pulsechat.co.za/updates/Pulse_0.2.1_x64-setup.exe",
            "https://pulsechat.co.za.evil.example/updates/Pulse_0.2.1_x64-setup.exe",
            "https://pulsechat.co.za:8443/updates/Pulse_0.2.1_x64-setup.exe",
            "https://voice.pulsechat.co.za/updates/Pulse_0.2.1_x64-setup.exe",
        ] {
            assert!(!same_origin(&server, &u(bad)), "{bad}");
        }
    }

    #[test]
    fn restart_refused_in_call() {
        assert_eq!(may_restart(true), Err(RestartRefused));
        assert_eq!(may_restart(false), Ok(()));
        assert_eq!(RestartRefused.to_string(), "Finish your call first");
    }

    #[tokio::test(start_paused = true)]
    async fn launch_wait_gives_up() {
        let (_tx, mut rx) = tokio::sync::watch::channel(LaunchCheck::Pending);
        let started = tokio::time::Instant::now();
        assert_eq!(wait_launch(&mut rx, LAUNCH_WAIT).await, None);
        assert_eq!(started.elapsed(), LAUNCH_WAIT);

        let offer = Offer {
            version: "0.2.1".into(),
        };
        let (_tx, mut rx) = tokio::sync::watch::channel(LaunchCheck::Done(Some(offer.clone())));
        let started = tokio::time::Instant::now();
        assert_eq!(wait_launch(&mut rx, LAUNCH_WAIT).await, Some(offer.clone()));
        assert_eq!(started.elapsed(), std::time::Duration::ZERO);

        // finishing while the page waits hands it over then
        let (tx, mut rx) = tokio::sync::watch::channel(LaunchCheck::Pending);
        let o = offer.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            tx.send(LaunchCheck::Done(Some(o))).unwrap();
        });
        assert_eq!(wait_launch(&mut rx, LAUNCH_WAIT).await, Some(offer));
    }

    #[test]
    fn offer_is_camel_case_json() {
        let j = serde_json::to_string(&Offer {
            version: "0.2.1".into(),
        })
        .unwrap();
        assert_eq!(j, r#"{"version":"0.2.1"}"#);
    }

    #[cfg(not(windows))]
    #[test]
    fn disabled_on_linux_and_debug() {
        assert!(!updates_enabled());
    }

    /// release-windows runs these with `--release`: there, updating is on (unless a test machine turned it off).
    #[cfg(all(windows, not(debug_assertions)))]
    #[test]
    fn enabled_in_windows_release() {
        assert_eq!(
            updates_enabled(),
            std::env::var_os("PULSE_NO_UPDATE").is_none()
        );
    }
}
