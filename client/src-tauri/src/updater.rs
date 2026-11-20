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
//! - The manifest is fetched here ([`fetch_manifest`]) so its status is seen: 200 → the plugin takes it
//!   (`restore_update`, which still checks the installer's signature); 204/404 → nothing newer; 401 → signed out
//!   there (the login screen asks `/updates/version` instead: [`update_signed_out`]); anything else → an error, so
//!   the check retries and an earlier offer stays.
//! - Found while running, an update installs by itself once Pulse has been left alone a while (the page decides:
//!   [`update_install_idle`]); a failed install is detected at the next launch ([`crate::update_attempt`]).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use semver::Version;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::watch;
use url::Url;

/// How long the page waits at launch for the check that started in `setup`.
pub const LAUNCH_WAIT: Duration = Duration::from_secs(2);
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

/// This build's platform, sent beside the version (`X-Pulse-Platform`): the server never asks a platform without an
/// updater to update.
pub const PLATFORM: &str = if cfg!(windows) {
    "windows"
} else if cfg!(target_os = "macos") {
    "macos"
} else {
    "linux"
};

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

/// How the server answered a manifest request, by its status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestAnswer {
    /// 2xx with a body: the manifest.
    Manifest,
    /// 204 or 404: nothing published (or nothing newer).
    NoUpdate,
    /// 401: not signed in there any more.
    SignedOut,
    /// Anything else (5xx during a deploy, 429, …): a failed check, retried, and an earlier offer stays.
    Error,
}

pub fn manifest_answer(status: u16) -> ManifestAnswer {
    match status {
        204 | 404 => ManifestAnswer::NoUpdate,
        401 => ManifestAnswer::SignedOut,
        200..=299 => ManifestAnswer::Manifest,
        _ => ManifestAnswer::Error,
    }
}

/// What [`fetch_manifest`] got.
#[derive(Debug, Clone, PartialEq)]
pub enum Fetched {
    Manifest(serde_json::Value),
    NoUpdate,
    SignedOut,
}

/// `GET <server>/updates/latest.json` with the session's token, this app's version and platform. A network error,
/// a timeout or an unexpected status is `Err` (see [`ManifestAnswer`]).
pub async fn fetch_manifest(
    server: &Url,
    token: &str,
    version: &str,
    timeout: Duration,
) -> Result<Fetched, String> {
    let endpoint = server
        .join("updates/latest.json")
        .map_err(|e| format!("bad server address: {e}"))?;
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("updater setup: {e}"))?;
    let res = client
        .get(endpoint)
        .bearer_auth(token)
        .header("X-Pulse-Version", version)
        .header("X-Pulse-Platform", PLATFORM)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("the update check failed: {}", short_error(&e, timeout)))?;
    let status = res.status().as_u16();
    match manifest_answer(status) {
        ManifestAnswer::NoUpdate => Ok(Fetched::NoUpdate),
        ManifestAnswer::SignedOut => Ok(Fetched::SignedOut),
        ManifestAnswer::Error => Err(format!(
            "the update check failed: the server answered {status}"
        )),
        ManifestAnswer::Manifest => res
            .json()
            .await
            .map(Fetched::Manifest)
            .map_err(|e| format!("the update check failed: {}", short_error(&e, timeout))),
    }
}

/// A request error in a few words (a timeout says how long it waited).
fn short_error(e: &reqwest::Error, timeout: Duration) -> String {
    if e.is_timeout() {
        format!("timed out after {} s", timeout.as_secs())
    } else {
        e.to_string()
    }
}

/// `GET <server>/updates/version`, no session (#460): the published version, `None` when nothing is published.
pub async fn latest_version(server: &Url, timeout: Duration) -> Result<Option<String>, String> {
    let url = server
        .join("updates/version")
        .map_err(|e| format!("bad server address: {e}"))?;
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("updater setup: {e}"))?;
    let res = client
        .get(url)
        .header("X-Pulse-Version", env!("CARGO_PKG_VERSION"))
        .header("X-Pulse-Platform", PLATFORM)
        .send()
        .await
        .map_err(|e| format!("the version check failed: {}", short_error(&e, timeout)))?;
    match res.status().as_u16() {
        204 | 404 => Ok(None),
        200 => res
            .json::<pulse_protocol::rest::LatestVersion>()
            .await
            .map(|v| Some(v.version))
            .map_err(|e| format!("the version check failed: {e}")),
        s => Err(format!("the version check failed: the server answered {s}")),
    }
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
    /// The profile's data dir: where an install attempt is noted ([`crate::update_attempt`]).
    #[cfg_attr(not(windows), allow(dead_code))]
    dir: std::path::PathBuf,
    /// The last install failed (found at this launch): no installing by itself for a while, "Couldn't update" shows.
    failure: Mutex<Option<crate::update_attempt::Failure>>,
    /// The server the stored login belongs to, read at launch: the "Couldn't update" download link's, while signed out.
    stored_server: Mutex<Option<String>>,
}

impl Updates {
    pub fn new(
        handover: Arc<Handover>,
        dir: std::path::PathBuf,
        failure: Option<crate::update_attempt::Failure>,
    ) -> Self {
        Self {
            launch: LaunchHandoff::default(),
            #[cfg(windows)]
            found: Mutex::new(None),
            installing: AtomicBool::new(false),
            retrying: AtomicBool::new(false),
            checked_login: Mutex::new(None),
            handover,
            dir,
            failure: Mutex::new(failure),
            stored_server: Mutex::new(None),
        }
    }

    /// The last install failed not long ago: install only when asked ("Restart to update"), never by itself.
    pub fn backing_off(&self) -> bool {
        let now = crate::reports::unix_now();
        self.failure
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|f| now < f.backoff_until)
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

/// One check against `server`: `Ok(None)` = nothing newer (or no manifest, or not signed in there any more). The
/// manifest is fetched here so a server error is an error ([`fetch_manifest`]); the plugin then takes it as if it had
/// fetched it, and checks the installer's signature as always.
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
    let version = app.package_info().version.to_string();
    let manifest = match fetch_manifest(&base, token, &version, timeout).await? {
        Fetched::Manifest(m) => m,
        Fetched::NoUpdate => {
            tracing::info!("update: the server offers no update");
            return Ok(None);
        }
        Fetched::SignedOut => {
            tracing::info!(
                "update: signed out on the server; the login screen asks for the version only"
            );
            return Ok(None);
        }
    };
    let endpoint = base
        .join("updates/latest.json")
        .map_err(|e| format!("bad server address: {e}"))?;
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        // the installer download goes with the same headers
        .and_then(|b| b.header("Authorization", format!("Bearer {token}")))
        .and_then(|b| b.header("X-Pulse-Version", version.clone()))
        .and_then(|b| b.header("X-Pulse-Platform", PLATFORM))
        .map_err(|e| format!("updater setup: {e}"))?
        .version_comparator(|current, release| should_offer(&current, &release.version))
        // A download that stalls fails (and Pulse carries on) instead of leaving "Updating Pulse…" up for good; a
        // per-read limit, so a slow but moving download of any size still finishes.
        .configure_client(|c| c.read_timeout(DOWNLOAD_STALL))
        .on_before_exit(move || handover.before_exit())
        .build()
        .map_err(|e| format!("updater setup: {e}"))?;
    // the plugin's own parsing, target choice and version comparison; the signature is checked on download/install
    let found = updater
        .restore_update(manifest)
        .map_err(|e| format!("the update manifest was refused: {e}"))?;
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
/// asks. Signed out = nothing to ask. `stored`: the stored login, read once in `setup` for this and the gateway.
pub fn start_launch_check(
    app: &AppHandle,
    stored: tokio::sync::oneshot::Receiver<Option<(String, String)>>,
) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let updates = app.state::<Updates>();
        let mut failed = false;
        let started = std::time::Instant::now();
        let stored = stored.await.ok().flatten();
        *updates.stored_server.lock().unwrap() = stored.as_ref().map(|(s, _)| s.clone());
        let offer = if updates_enabled() {
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
        crate::startup::mark("launch_check_done");
        tracing::info!(
            took_ms = started.elapsed().as_millis() as u64,
            found = offer.is_some(),
            failed,
            "update: launch check finished"
        );
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
pub async fn update_launch(
    app: AppHandle,
    updates: tauri::State<'_, Updates>,
) -> Result<Option<Offer>, String> {
    let offer = updates.launch.page_wait(LAUNCH_WAIT).await;
    crate::startup::mark("launch_wait_done");
    if let Some(o) = &offer
        && updates.backing_off()
    {
        // #460: the last install failed; don't download it again at every launch. "Restart to update" still can.
        tracing::info!(version = %o.version, "update: the last install failed; offering it instead of installing at launch");
        let _ = app.emit(UPDATE_EVENT, Some(o));
        return Ok(None);
    }
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

/// Signed out (or the session expired), on the login screen: is a newer version published on `server`? Asks
/// `/updates/version` (no session); the update itself needs a sign-in. `None`: nothing newer, or this build doesn't
/// update itself. A failed ask is `None` too (the login screen just doesn't say anything).
#[tauri::command]
pub async fn update_signed_out(server: String) -> Result<Option<Offer>, String> {
    if !updates_enabled() {
        return Ok(None);
    }
    let Ok(server) = crate::api::check_server_url(&server) else {
        return Ok(None);
    };
    let Ok(base) = Url::parse(&server) else {
        return Ok(None);
    };
    let current = Version::parse(env!("CARGO_PKG_VERSION")).map_err(|e| e.to_string())?;
    match latest_version(&base, MANUAL_CHECK_TIMEOUT).await {
        Ok(Some(v)) => match Version::parse(&v) {
            Ok(latest) if should_offer(&current, &latest) => {
                tracing::info!(version = %v, "update: one is published; signing in gets it");
                Ok(Some(Offer { version: v }))
            }
            _ => Ok(None),
        },
        Ok(None) => Ok(None),
        Err(e) => {
            tracing::info!(error = %e, "update: couldn't ask for the published version");
            Ok(None)
        }
    }
}

/// The last install failed (found at this launch, #460): what "Couldn't update" says.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedUpdate {
    pub version: String,
    /// Where to download it by hand: the server's public Windows download.
    pub download_url: Option<String>,
}

/// The public installer download on `server`.
pub fn download_url(server: &str) -> Option<String> {
    Url::parse(server)
        .ok()?
        .join("download/windows")
        .ok()
        .map(|u| u.to_string())
}

/// What "Couldn't update" shows, once per launch that found the last install failed (`None` after the first ask, or
/// when nothing failed).
#[tauri::command]
pub fn update_failed(app: AppHandle) -> Option<FailedUpdate> {
    let updates = app.state::<Updates>();
    let f = updates.failure.lock().unwrap().clone()?;
    if !f.fresh {
        return None;
    }
    let server = current_session(&app)
        .map(|(s, _)| s)
        .or_else(|| updates.stored_server.lock().unwrap().clone());
    Some(FailedUpdate {
        version: f.attempt.to,
        download_url: server.as_deref().and_then(download_url),
    })
}

/// After sign-in (beside last session's crash report): a fresh failed install is reported as a crash would be, only
/// with "Send crash reports automatically" on, once.
pub async fn report_failed_install(app: AppHandle, api: crate::api::Api, token: String) {
    let Some(updates) = app.try_state::<Updates>() else {
        return;
    };
    let attempt = {
        let mut failure = updates.failure.lock().unwrap();
        match failure.as_mut() {
            Some(f) if f.fresh => {
                if !app
                    .state::<crate::reports::settings::Store>()
                    .auto_send_crashes()
                {
                    tracing::info!(
                        "update: not reporting the failed install (automatic reports are off)"
                    );
                    return;
                }
                f.fresh = false; // one report, even if it fails to send
                f.attempt.clone()
            }
            _ => return,
        }
    };
    let dir = app.state::<crate::reports::LogDir>().0.clone();
    // file I/O: not on an async worker
    let log = tokio::task::spawn_blocking(move || {
        crate::reports::tail::log_tail(&dir, crate::reports::TAIL_BYTES)
    })
    .await
    .unwrap_or_default();
    let mut meta = crate::reports::meta(&app, api.base()).await;
    meta["update_failed"] =
        serde_json::json!({ "from": attempt.from, "to": attempt.to, "at": attempt.at });
    let req = pulse_protocol::rest::CreateReportRequest {
        kind: pulse_protocol::rest::ReportKind::Bug,
        text: format!(
            "Automatic report: the update to {} didn't install; Pulse is still on {}.",
            attempt.to,
            env!("CARGO_PKG_VERSION")
        ),
        meta,
        log,
    };
    match api.create_report(&token, &req).await {
        Ok(r) => tracing::info!(id = %r.id, "update: the failed install was reported"),
        Err(e) => tracing::warn!(error = %e, "update: couldn't report the failed install"),
    }
}

/// Download (with [`PROGRESS_EVENT`]s) and install the update a check found. On Windows the installer takes over and
/// the process ends here. `restart`: asked for from the title bar, refused during a call.
#[tauri::command]
pub async fn update_install(app: AppHandle, restart: bool) -> Result<(), String> {
    install(&app, restart, "asked").await
}

/// The page saw Pulse left alone long enough with an update waiting (#460): install it, unless something's going on
/// the core can see (a call, a clip still uploading) or the last install failed not long ago.
#[tauri::command]
pub async fn update_install_idle(app: AppHandle) -> Result<(), String> {
    if app.state::<Updates>().backing_off() {
        return Err("the last update didn't install; not trying again by itself yet".into());
    }
    if app.state::<crate::commands::Core>().clips_busy() {
        tracing::info!("update: not installing while idle, a clip is uploading");
        return Err("a clip is still uploading".into());
    }
    install(&app, true, "idle").await
}

async fn install(app: &AppHandle, restart: bool, why: &'static str) -> Result<(), String> {
    if restart {
        let voice = app.state::<crate::voice::VoiceManager>();
        let in_call = voice.current_channel().await.is_some() || voice.rejoining().is_some();
        may_restart(in_call)
            .inspect_err(|_| tracing::info!(why, "update: restart refused, in a call"))
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
        tracing::info!(version = %update.version, restart, why, "update: downloading");
        let result = download_and_install(app, &update, &updates.dir).await;
        if result.is_err() {
            // the installer didn't start after all: crash marker and log back, if they were already handed over
            updates.handover.install_failed();
            crate::update_attempt::clear(&updates.dir);
        }
        updates.installing.store(false, Ordering::SeqCst);
        result.inspect_err(|e| tracing::warn!(error = %e, version = %update.version, "update: install failed; staying on this version"))
    }
    // Only the Windows app updates itself (its checks never offer anything elsewhere).
    #[cfg(not(windows))]
    {
        let _ = why;
        Err("there's no update to install".into())
    }
}

#[cfg(windows)]
async fn download_and_install(
    app: &AppHandle,
    update: &tauri_plugin_updater::Update,
    dir: &std::path::Path,
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
    // #460: the next launch checks it got here (Pulse ends at the handover, so a failed install is otherwise silent)
    crate::update_attempt::record(
        dir,
        env!("CARGO_PKG_VERSION"),
        &update.version,
        crate::reports::unix_now(),
    );
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
        let updates = Updates::new(Arc::new(counting_handover().0), Default::default(), None);
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

    /// One HTTP answer from a loopback stub; gives back the request it got (lower case).
    async fn stub(answer: &'static str) -> (Url, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
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
            sock.write_all(answer.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&req).to_lowercase()
        });
        (u(&format!("http://{addr}/")), task)
    }

    #[test]
    fn manifest_status_mapping() {
        use ManifestAnswer::*;
        assert_eq!(manifest_answer(200), Manifest);
        assert_eq!(manifest_answer(204), NoUpdate);
        assert_eq!(manifest_answer(404), NoUpdate);
        assert_eq!(manifest_answer(401), SignedOut);
        for s in [500, 502, 503, 429, 400, 403, 301] {
            assert_eq!(manifest_answer(s), Error, "{s}");
        }
    }

    /// #460: the manifest request carries the token, version and platform, and a server error is an error (retried,
    /// an earlier offer kept), never "no update".
    #[tokio::test]
    async fn fetch_manifest_tells_errors_from_no_update() {
        let body = r#"{"version":"9.9.9","platforms":{}}"#;
        let ok: &'static str = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .into_boxed_str(),
        );
        let (server, req) = stub(ok).await;
        let got = fetch_manifest(&server, "tok-1", "0.2.18", CHECK_TIMEOUT).await;
        assert_eq!(
            got,
            Ok(Fetched::Manifest(serde_json::from_str(body).unwrap()))
        );
        let req = req.await.unwrap();
        assert!(req.starts_with("get /updates/latest.json "), "{req}");
        assert!(req.contains("authorization: bearer tok-1"), "{req}");
        assert!(req.contains("x-pulse-version: 0.2.18"), "{req}");
        assert!(
            req.contains(&format!("x-pulse-platform: {PLATFORM}")),
            "{req}"
        );

        for (answer, want) in [
            (
                "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                Ok(Fetched::NoUpdate),
            ),
            (
                "HTTP/1.1 204 No Content\r\nconnection: close\r\n\r\n",
                Ok(Fetched::NoUpdate),
            ),
            (
                "HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                Ok(Fetched::SignedOut),
            ),
        ] {
            let (server, _req) = stub(answer).await;
            assert_eq!(
                fetch_manifest(&server, "t", "0.2.18", CHECK_TIMEOUT).await,
                want,
                "{answer}"
            );
        }
        let (server, _req) = stub(
            "HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await;
        let e = fetch_manifest(&server, "t", "0.2.18", CHECK_TIMEOUT)
            .await
            .unwrap_err();
        assert!(e.contains("503"), "{e}");
        // nobody there
        let gone = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            u(&format!("http://{}/", l.local_addr().unwrap()))
        };
        assert!(
            fetch_manifest(&gone, "t", "0.2.18", CHECK_TIMEOUT)
                .await
                .is_err()
        );
    }

    /// The signed-out check (#460): the version only, no token sent.
    #[tokio::test]
    async fn latest_version_needs_no_token() {
        let (server, req) = stub(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 20\r\nconnection: close\r\n\r\n{\"version\":\"0.2.19\"}",
        )
        .await;
        assert_eq!(
            latest_version(&server, CHECK_TIMEOUT).await,
            Ok(Some("0.2.19".into()))
        );
        let req = req.await.unwrap();
        assert!(req.starts_with("get /updates/version "), "{req}");
        assert!(!req.contains("authorization"), "{req}");
        let (server, _) = stub("HTTP/1.1 204 No Content\r\nconnection: close\r\n\r\n").await;
        assert_eq!(latest_version(&server, CHECK_TIMEOUT).await, Ok(None));
        let (server, _) =
            stub("HTTP/1.1 500 Oops\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
        assert!(latest_version(&server, CHECK_TIMEOUT).await.is_err());
    }

    #[test]
    fn download_link_is_the_public_windows_download() {
        assert_eq!(
            download_url("https://example.invalid").as_deref(),
            Some("https://example.invalid/download/windows")
        );
        assert_eq!(download_url("not a url"), None);
    }

    /// #460: after a failed install, nothing installs by itself until the back-off ends.
    #[test]
    fn a_failed_install_backs_off() {
        let now = crate::reports::unix_now();
        let failure = |until| crate::update_attempt::Failure {
            attempt: crate::update_attempt::Attempt {
                from: "0.2.18".into(),
                to: "0.2.19".into(),
                at: now - 10,
                failed_at: Some(now),
            },
            fresh: true,
            backoff_until: until,
        };
        let h = || Arc::new(counting_handover().0);
        assert!(!Updates::new(h(), Default::default(), None).backing_off());
        assert!(Updates::new(h(), Default::default(), Some(failure(now + 60))).backing_off());
        assert!(!Updates::new(h(), Default::default(), Some(failure(now - 1))).backing_off());
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
