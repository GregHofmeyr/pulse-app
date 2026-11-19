//! App updates (#82). Tauri's updater, driven from here (the page only calls our commands): the signed-in server's
//! `/updates/latest.json`, asked with the session token, names a newer signed installer.
//!
//! - At launch the check starts in `setup` (before the page loads); the page waits at most [`LAUNCH_WAIT`] for it and
//!   shows "Updating Pulse…" when it found one. A check still running then carries on (up to [`CHECK_TIMEOUT`]), and
//!   what it finds is offered like a running check's (#536).
//! - While running: after each login's first Ready and every [`RECHECK_EVERY`]; an offer goes to the page as
//!   [`UPDATE_EVENT`] ("Restart to update", never during a call: [`may_restart`]; in a call the page says the update
//!   is ready and applies when you restart). A failed check tries again sooner ([`retry_delay`]).
//! - Closing the window quits Pulse (there's no tray), so the next launch's check is the one that installs.
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
/// How long one background check (the manifest request: at launch, after login, periodic) may take. The page never
/// waits for it longer than [`LAUNCH_WAIT`], so a slow connection still gets its answer, just later (#536). Never the
/// updater's own `.timeout()`: that bounds the installer download too.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// How long Settings' "Check for updates" (and the "update required" screen) waits: someone is watching.
pub const MANUAL_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
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

/// After the `failures`-th failed check in a row, how long until the next try (#536): 1, 2, 4, 8 and 16 minutes, then
/// `None` (the periodic check, every [`RECHECK_EVERY`], carries on from there).
pub fn retry_delay(failures: u32) -> Option<Duration> {
    match failures {
        1..=5 => Some(Duration::from_secs(60 << (failures - 1))),
        _ => None,
    }
}

/// After the `attempt`-th failed download, how long until the next (#536): a dropped connection gets two more tries
/// (after 2 s, then 10 s); anything else (a bad signature, a full disk) fails at once.
pub fn download_retry_delay(attempt: u32, transient: bool) -> Option<Duration> {
    match (transient, attempt) {
        (true, 1) => Some(Duration::from_secs(2)),
        (true, 2) => Some(Duration::from_secs(10)),
        _ => None,
    }
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

/// The launch check's result between the core and the page (#536). The page waits at most [`LAUNCH_WAIT`]; a check
/// that finishes after it gave up is offered as a running offer instead of being dropped. One lock orders the two, so
/// a result landing just as the page gives up goes to exactly one of them.
pub struct LaunchHandoff {
    tx: watch::Sender<LaunchCheck>,
    page_gave_up: Mutex<bool>,
}

impl Default for LaunchHandoff {
    fn default() -> Self {
        Self {
            tx: watch::channel(LaunchCheck::Pending).0,
            page_gave_up: Mutex::new(false),
        }
    }
}

impl LaunchHandoff {
    /// The page's ask: what the launch check found, waiting at most `wait`. `None` = start normally (and if the check
    /// is still running, what it finds goes out as a running offer).
    pub async fn page_wait(&self, wait: Duration) -> Option<Offer> {
        let mut rx = self.tx.subscribe();
        // what it found is read again under the lock below, so a result landing just now isn't missed
        let _ = wait_launch(&mut rx, wait).await;
        let mut gave_up = self.page_gave_up.lock().unwrap();
        match &*self.tx.borrow() {
            LaunchCheck::Done(offer) => offer.clone(),
            LaunchCheck::Pending => {
                *gave_up = true;
                None
            }
        }
    }

    /// The check's result: kept for the page (`send_replace`, never `send`: `send` drops the value while nobody is
    /// subscribed, #460). Returns the offer to send as a running one when the page stopped waiting already.
    pub fn publish(&self, offer: Option<Offer>) -> Option<Offer> {
        let gave_up = self.page_gave_up.lock().unwrap();
        self.tx.send_replace(LaunchCheck::Done(offer.clone()));
        if *gave_up { offer } else { None }
    }

    #[cfg(test)]
    fn subscribe(&self) -> watch::Receiver<LaunchCheck> {
        self.tx.subscribe()
    }
}

/// Around the installer taking over (Windows): the plugin runs [`Handover::before_exit`] just before it starts the
/// installer and ends the process (the crash marker is cleared, the log closed). If starting the installer fails, the
/// install comes back with an error and Pulse carries on, so [`Handover::install_failed`] undoes that (#233).
pub struct Handover {
    exit: Box<dyn Fn() + Send + Sync>,
    resume: Box<dyn Fn() + Send + Sync>,
    handed_over: AtomicBool,
}

impl Handover {
    pub fn new(
        exit: impl Fn() + Send + Sync + 'static,
        resume: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            exit: Box::new(exit),
            resume: Box::new(resume),
            handed_over: AtomicBool::new(false),
        }
    }

    /// The plugin's `on_before_exit`.
    pub fn before_exit(&self) {
        self.handed_over.store(true, Ordering::SeqCst);
        (self.exit)();
    }

    /// The install came back (it only does when it failed): undo [`Self::before_exit`] if it ran.
    pub fn install_failed(&self) {
        if self.handed_over.swap(false, Ordering::SeqCst) {
            (self.resume)();
        }
    }
}

/// What a check found becomes the update to install. Finding nothing newer (the server withdrew it, or it's signed
/// out there) clears an earlier one; a failed check leaves it as it was (#233).
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn keep_found<T: Clone>(
    slot: &Mutex<Option<T>>,
    checked: Result<Option<T>, String>,
) -> Result<Option<T>, String> {
    if let Ok(found) = &checked {
        *slot.lock().unwrap() = found.clone();
    }
    checked
}

/// The updater's state in the app (managed).
pub struct Updates {
    launch: LaunchHandoff,
    /// The newest update a check found, ready to install.
    #[cfg(windows)]
    found: Mutex<Option<tauri_plugin_updater::Update>>,
    installing: AtomicBool,
    /// A failed check's retries are scheduled ([`check_with_retries`]): another failing check needn't start more.
    retrying: AtomicBool,
    /// The login checked already (its token), by the launch check or its first Ready: one check per login.
    checked_login: Mutex<Option<String>>,
    /// Around the installer taking over and the process ending (Windows): crash marker, log.
    #[cfg_attr(not(windows), allow(dead_code))]
    handover: Arc<Handover>,
}

impl Updates {
    pub fn new(handover: Arc<Handover>) -> Self {
        Self {
            launch: LaunchHandoff::default(),
            #[cfg(windows)]
            found: Mutex::new(None),
            installing: AtomicBool::new(false),
            retrying: AtomicBool::new(false),
            checked_login: Mutex::new(None),
            handover,
        }
    }

    /// The first check for the login holding `token`: true once per login (the launch check with the stored login
    /// and that login's first Ready would otherwise both ask, #233).
    fn claim_login(&self, token: &str) -> bool {
        let mut checked = self.checked_login.lock().unwrap();
        if checked.as_deref() == Some(token) {
            return false;
        }
        *checked = Some(token.to_string());
        true
    }

    /// That check failed: the login's first Ready may try again.
    fn release_login(&self, token: &str) {
        let mut checked = self.checked_login.lock().unwrap();
        if checked.as_deref() == Some(token) {
            *checked = None;
        }
    }

    /// Ask `server` (with `token`) whether there's a newer version; remembers it for [`update_install`].
    async fn check(
        &self,
        app: &AppHandle,
        server: &str,
        token: &str,
        timeout: Duration,
    ) -> Result<Option<Offer>, String> {
        if !updates_enabled() {
            return Ok(None);
        }
        #[cfg(windows)]
        {
            let started = std::time::Instant::now();
            let found = keep_found(
                &self.found,
                check_server(app, server, token, self.handover.clone(), timeout).await,
            )?;
            let took_ms = started.elapsed().as_millis() as u64;
            match &found {
                Some(u) => {
                    tracing::info!(version = %u.version, took_ms, "update: a newer version is available")
                }
                None => tracing::info!(took_ms, "update: up to date"),
            }
            Ok(found.map(|u| Offer { version: u.version }))
        }
        #[cfg(not(windows))]
        {
            let _ = (app, server, token, timeout);
            Ok(None)
        }
    }
}

/// One check against `server`: `Ok(None)` = nothing newer (or no manifest, or not signed in there any more).
#[cfg(windows)]
async fn check_server<R: tauri::Runtime>(
    app: &AppHandle<R>,
    server: &str,
    token: &str,
    handover: Arc<Handover>,
    timeout: Duration,
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
        // the server logs it with each check (#460); the updater's own User-Agent names only the plugin
        .and_then(|b| b.header("X-Pulse-Version", app.package_info().version.to_string()))
        .map_err(|e| format!("updater setup: {e}"))?
        .version_comparator(|current, release| should_offer(&current, &release.version))
        // A download that stalls fails (and Pulse carries on) instead of leaving "Updating Pulse…" up for good; a
        // per-read limit, so a slow but moving download of any size still finishes.
        .configure_client(|c| c.read_timeout(DOWNLOAD_STALL))
        .on_before_exit(move || handover.before_exit())
        .build()
        .map_err(|e| format!("updater setup: {e}"))?;
    let found = match tokio::time::timeout(timeout, updater.check()).await {
        Err(_) => {
            return Err(format!(
                "the update check timed out after {} s",
                timeout.as_secs()
            ));
        }
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
        let mut failed = false;
        let offer = if updates_enabled() {
            // keychain reads block: not on the runtime's threads
            let stored = tauri::async_runtime::spawn_blocking(move || store.load())
                .await
                .ok()
                .flatten();
            match stored {
                None => {
                    tracing::info!("update: signed out at launch; the first login checks");
                    None
                }
                Some((server, token)) => {
                    // this login's first Ready needn't ask again (unless this check fails)
                    updates.claim_login(&token);
                    tracing::info!("update: launch check started");
                    match updates.check(&app, &server, &token, CHECK_TIMEOUT).await {
                        Ok(offer) => offer,
                        Err(e) => {
                            updates.release_login(&token);
                            tracing::warn!(error = %e, "update: launch check failed; starting this version");
                            failed = true;
                            None
                        }
                    }
                }
            }
        } else {
            None
        };
        if let Some(late) = updates.launch.publish(offer) {
            // #536: the page stopped waiting and started this version; offer it as "Restart to update"
            tracing::info!(version = %late.version, "update: the launch check finished after the page stopped waiting; offering it while running");
            let _ = app.emit(UPDATE_EVENT, Some(&late));
        }
        if failed {
            // #536: its login's Ready may have come and gone already; don't wait the full RECHECK_EVERY
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                check_with_retries(&app, "launch retry", 1).await
            });
        }
    });
}

/// What a running check came to.
#[derive(Debug)]
enum Ran {
    /// Not asked: why (signed out, installing).
    Skipped(&'static str),
    Checked,
    Failed,
}

/// A running check: an offer goes to the page ("Restart to update"); finding nothing any more takes it away (`null`).
async fn check_running(app: &AppHandle, why: &str) -> Ran {
    let Some((server, token)) = current_session(app) else {
        return Ran::Skipped("signed out");
    };
    let updates = app.state::<Updates>();
    if updates.installing.load(Ordering::SeqCst) {
        return Ran::Skipped("installing");
    }
    match updates.check(app, &server, &token, CHECK_TIMEOUT).await {
        Ok(offer) => {
            let _ = app.emit(UPDATE_EVENT, &offer);
            Ran::Checked
        }
        Err(e) => {
            tracing::warn!(error = %e, why, "update: check failed");
            Ran::Failed
        }
    }
}

/// A running check that, when it fails, tries again after [`retry_delay`] instead of waiting for the next periodic
/// one (#536). `failed`: how many failed already (the launch check's failure counts; it waits before its first try).
/// One set of retries at a time: a check that would start beside them leaves it to them.
async fn check_with_retries(app: &AppHandle, why: &'static str, mut failed: u32) {
    let updates = app.state::<Updates>();
    if updates.retrying.swap(true, Ordering::SeqCst) {
        tracing::info!(
            why,
            "update: retries are already scheduled; not checking now"
        );
        return;
    }
    loop {
        if failed > 0 {
            let Some(wait) = retry_delay(failed) else {
                tracing::warn!(
                    failed,
                    "update: still failing; the next periodic check tries again"
                );
                break;
            };
            tracing::info!(
                failed,
                wait_s = wait.as_secs(),
                why,
                "update: trying again soon"
            );
            tokio::time::sleep(wait).await;
        }
        match check_running(app, why).await {
            Ran::Failed => failed += 1,
            Ran::Skipped(reason) => {
                tracing::info!(reason, why, "update: check skipped");
                break;
            }
            Ran::Checked => break,
        }
    }
    updates.retrying.store(false, Ordering::SeqCst);
}

/// The gateway's Ready (commands::forward): the first one of each login checks (a login after a signed-out start,
/// or another account), unless the launch check already asked for that login.
pub fn after_ready(app: &AppHandle, token: &str) {
    if !updates_enabled() {
        return;
    }
    let Some(updates) = app.try_state::<Updates>() else {
        return;
    };
    if !updates.claim_login(token) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move { check_with_retries(&app, "login", 0).await });
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
            check_with_retries(&app, "periodic", 0).await;
        }
    });
}

/// What the launch check found (waits at most [`LAUNCH_WAIT`] for it); `None` = start normally.
#[tauri::command]
pub async fn update_launch(updates: tauri::State<'_, Updates>) -> Result<Option<Offer>, String> {
    let offer = updates.launch.page_wait(LAUNCH_WAIT).await;
    match &offer {
        Some(o) => tracing::info!(version = %o.version, "update: installing at launch"),
        None if updates_enabled() => tracing::info!("update: starting this version"),
        None => {}
    }
    Ok(offer)
}

/// What Settings shows about updates (#460): this app's version, whether it updates itself ([`updates_enabled`]), and
/// whether its updates come from a package manager instead (Linux).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub enabled: bool,
    pub package_manager: bool,
}

/// [`UpdateInfo`] for Settings' "Check for updates".
#[tauri::command]
pub fn update_info(app: AppHandle) -> UpdateInfo {
    UpdateInfo {
        version: app.package_info().version.to_string(),
        enabled: updates_enabled(),
        package_manager: !cfg!(windows),
    }
}

/// A fresh check against the signed-in server (the forced "update required" screen, and Settings' "Check for
/// updates"); `None` = nothing newer yet.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<Option<Offer>, String> {
    let Some((server, token)) = current_session(&app) else {
        return Ok(None);
    };
    app.state::<Updates>()
        .check(&app, &server, &token, MANUAL_CHECK_TIMEOUT)
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
        may_restart(in_call)
            .inspect_err(|_| tracing::info!("update: restart refused, in a call"))
            .map_err(|e| e.to_string())?;
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
        if result.is_err() {
            // the installer didn't start after all: crash marker and log back, if they were already handed over
            updates.handover.install_failed();
        }
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
    let mut attempt = 0;
    let bytes = loop {
        attempt += 1;
        let mut downloaded = 0u64;
        let mut last_emit: Option<std::time::Instant> = None;
        let got = update
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
            .await;
        match got {
            Ok(bytes) => break bytes,
            Err(e) => {
                // #536: a dropped connection tries again; a bad signature or the like doesn't
                let transient = matches!(
                    e,
                    tauri_plugin_updater::Error::Reqwest(_)
                        | tauri_plugin_updater::Error::Network(_)
                        | tauri_plugin_updater::Error::Io(_)
                );
                let Some(wait) = download_retry_delay(attempt, transient) else {
                    return Err(format!("the download failed: {e}"));
                };
                tracing::warn!(error = %e, attempt, wait_s = wait.as_secs(), "update: download failed; trying again");
                tokio::time::sleep(wait).await;
            }
        }
    };
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

    #[tokio::test(start_paused = true)]
    async fn launch_result_kept_when_it_beats_the_page() {
        // #460: the check finished (no one subscribed yet) before the page asked
        let handoff = LaunchHandoff::default();
        let offer = Offer {
            version: "0.2.13".into(),
        };
        assert_eq!(
            handoff.publish(Some(offer.clone())),
            None,
            "the page still asks"
        );
        let started = tokio::time::Instant::now();
        assert_eq!(handoff.page_wait(LAUNCH_WAIT).await, Some(offer.clone()));
        assert_eq!(started.elapsed(), std::time::Duration::ZERO);
        let mut rx = handoff.subscribe();
        assert_eq!(wait_launch(&mut rx, LAUNCH_WAIT).await, Some(offer));
    }

    /// #536: a launch check slower than LAUNCH_WAIT isn't dropped: the page starts this version, and what the check
    /// finds afterwards is offered as a running offer.
    #[tokio::test(start_paused = true)]
    async fn a_late_launch_check_is_offered_while_running() {
        let handoff = LaunchHandoff::default();
        let started = tokio::time::Instant::now();
        assert_eq!(handoff.page_wait(LAUNCH_WAIT).await, None);
        assert_eq!(started.elapsed(), LAUNCH_WAIT);
        let offer = Offer {
            version: "0.2.16".into(),
        };
        assert_eq!(handoff.publish(Some(offer.clone())), Some(offer));
        // finding nothing late is nothing to offer
        let handoff = LaunchHandoff::default();
        assert_eq!(handoff.page_wait(LAUNCH_WAIT).await, None);
        assert_eq!(handoff.publish(None), None);
    }

    /// #536: finishing while the page waits goes to the page only (installed at launch), never also as a running offer.
    #[tokio::test(start_paused = true)]
    async fn a_launch_check_inside_the_wait_goes_to_the_page_only() {
        let handoff = Arc::new(LaunchHandoff::default());
        let offer = Offer {
            version: "0.2.16".into(),
        };
        let (h, o) = (handoff.clone(), offer.clone());
        let checker = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            h.publish(Some(o))
        });
        assert_eq!(handoff.page_wait(LAUNCH_WAIT).await, Some(offer));
        assert_eq!(checker.await.unwrap(), None);
    }

    #[test]
    fn failed_checks_back_off_then_leave_it_to_the_periodic_check() {
        let mins: Vec<u64> = (1..=5)
            .map(|n| retry_delay(n).unwrap().as_secs() / 60)
            .collect();
        assert_eq!(mins, [1, 2, 4, 8, 16]);
        assert_eq!(retry_delay(0), None);
        assert_eq!(retry_delay(6), None);
        let total: Duration = (1..=5).filter_map(retry_delay).sum();
        assert!(
            total <= RECHECK_EVERY + Duration::from_secs(60),
            "{total:?}"
        );
    }

    #[test]
    fn only_a_dropped_download_is_retried() {
        assert_eq!(download_retry_delay(1, true), Some(Duration::from_secs(2)));
        assert_eq!(download_retry_delay(2, true), Some(Duration::from_secs(10)));
        assert_eq!(download_retry_delay(3, true), None, "three tries in all");
        assert_eq!(
            download_retry_delay(1, false),
            None,
            "a bad signature fails at once"
        );
    }

    #[test]
    fn background_checks_outlast_the_launch_wait() {
        assert!(CHECK_TIMEOUT > LAUNCH_WAIT);
        assert!(MANUAL_CHECK_TIMEOUT <= CHECK_TIMEOUT);
    }

    #[test]
    fn offer_is_camel_case_json() {
        let j = serde_json::to_string(&Offer {
            version: "0.2.1".into(),
        })
        .unwrap();
        assert_eq!(j, r#"{"version":"0.2.1"}"#);
    }

    #[test]
    fn update_info_is_camel_case_json() {
        let j = serde_json::to_string(&UpdateInfo {
            version: "0.2.15".into(),
            enabled: false,
            package_manager: true,
        })
        .unwrap();
        assert_eq!(
            j,
            r#"{"version":"0.2.15","enabled":false,"packageManager":true}"#
        );
    }

    fn counting_handover() -> (
        Handover,
        Arc<std::sync::atomic::AtomicU32>,
        Arc<std::sync::atomic::AtomicU32>,
    ) {
        use std::sync::atomic::AtomicU32;
        let (exits, resumes) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicU32::new(0)));
        let (e, r) = (exits.clone(), resumes.clone());
        let h = Handover::new(
            move || {
                e.fetch_add(1, Ordering::SeqCst);
            },
            move || {
                r.fetch_add(1, Ordering::SeqCst);
            },
        );
        (h, exits, resumes)
    }

    /// #233: the crash marker and the log are handed over just before the installer starts; if starting it fails,
    /// they come back. A download that fails never handed anything over, so there's nothing to undo.
    #[test]
    fn a_failed_handover_is_undone() {
        let (h, exits, resumes) = counting_handover();
        h.install_failed(); // failed before the plugin's hook ran (a download error)
        assert_eq!(resumes.load(Ordering::SeqCst), 0);
        h.before_exit();
        assert_eq!(exits.load(Ordering::SeqCst), 1);
        h.install_failed();
        assert_eq!(resumes.load(Ordering::SeqCst), 1);
        h.install_failed(); // once per handover
        assert_eq!(resumes.load(Ordering::SeqCst), 1);
        h.before_exit();
        h.install_failed();
        assert_eq!(resumes.load(Ordering::SeqCst), 2);
    }

    /// #233: a check that finds nothing clears an earlier offer (the server withdrew it); a failed one keeps it.
    #[test]
    fn finding_nothing_withdraws_an_earlier_offer() {
        let slot = Mutex::new(None);
        let o = |v: &str| Offer { version: v.into() };
        assert_eq!(
            keep_found(&slot, Ok(Some(o("0.2.1")))),
            Ok(Some(o("0.2.1")))
        );
        assert_eq!(*slot.lock().unwrap(), Some(o("0.2.1")));
        assert!(keep_found(&slot, Err::<Option<Offer>, _>("offline".into())).is_err());
        assert_eq!(
            *slot.lock().unwrap(),
            Some(o("0.2.1")),
            "a failed check keeps it"
        );
        assert_eq!(
            keep_found(&slot, Ok(Some(o("0.2.2")))),
            Ok(Some(o("0.2.2")))
        );
        assert_eq!(keep_found(&slot, Ok(None)), Ok(None));
        assert_eq!(*slot.lock().unwrap(), None, "withdrawn");
    }

    /// #233: one check per login: the launch check's login doesn't ask again at its first Ready, unless it failed.
    #[test]
    fn one_check_per_login() {
        let updates = Updates::new(Arc::new(counting_handover().0));
        assert!(updates.claim_login("t1"), "the launch check asks");
        assert!(!updates.claim_login("t1"), "its first Ready doesn't");
        assert!(updates.claim_login("t2"), "another login does");
        updates.release_login("t1"); // not the current one: no effect
        assert!(!updates.claim_login("t2"));
        updates.release_login("t2"); // its check failed
        assert!(updates.claim_login("t2"), "so the first Ready tries again");
    }

    /// #233: an expired or revoked session gets 401 from `/updates/latest.json`; that's "no update" (`Ok(None)`, so
    /// no Updating screen), not an error, and the session's token was sent. Windows only (the plugin is a Windows
    /// dependency); release-windows runs it.
    #[cfg(windows)]
    #[tokio::test]
    async fn unauthorized_check_is_no_update() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let stub = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut req = Vec::new();
            let mut buf = [0u8; 4096];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = sock.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&buf[..n]);
            }
            sock.write_all(
                b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
            )
            .await
            .unwrap();
            String::from_utf8_lossy(&req).to_lowercase()
        });

        let mut ctx = tauri::test::mock_context(tauri::test::noop_assets());
        ctx.config_mut().plugins.0.insert(
            "updater".into(),
            serde_json::json!({
                "pubkey": "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEMyMDA4NkU3MzgzM0MxMEYKUldRUHdUTTQ1NFlBd2g0T1RTd0VGS29ldWpZSGU0M1BPSGlCNSs3ZUxFZWxJQkZ3OHBzUnlNb0gK",
                "endpoints": [],
                // the stub is plain http on loopback (release builds refuse http otherwise)
                "dangerousInsecureTransportProtocol": true
            }),
        );
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .build(ctx)
            .unwrap();
        let (h, exits, _) = counting_handover();
        let found = check_server(
            app.handle(),
            &format!("http://{addr}/"),
            "tok-401",
            Arc::new(h),
            CHECK_TIMEOUT,
        )
        .await;
        assert!(matches!(found, Ok(None)), "401 is no update");
        let req = stub.await.unwrap();
        assert!(req.starts_with("get /updates/latest.json "), "{req}");
        assert!(req.contains("authorization: bearer tok-401"), "{req}");
        assert_eq!(exits.load(Ordering::SeqCst), 0);
    }

    #[cfg(not(windows))]
    #[test]
    fn disabled_on_linux_and_debug() {
        assert!(!updates_enabled());
    }

    /// The Windows CI tests run these in the `release-test` profile (debug_assertions off): there, updating is on
    /// (unless a test machine turned it off).
    #[cfg(all(windows, not(debug_assertions)))]
    #[test]
    fn enabled_in_windows_release() {
        assert_eq!(
            updates_enabled(),
            std::env::var_os("PULSE_NO_UPDATE").is_none()
        );
    }
}
