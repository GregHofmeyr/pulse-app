//! Tauri commands: the UI's only way to reach the server. Tokens stay in here.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use pulse_protocol::gateway::ClientFrame;
use pulse_protocol::ids::{ChannelId, GameId, LineId, MessageId, QotyEventId, ServerId, UserId};
use pulse_protocol::rest::{
    Channel, ClipFacets, ClipStorage, ClipUrl, ClipView, CustomEmoji, EditClipRequest,
    EditQuoteRequest, Game, Leaderboard, LinkSpeakerRequest, MediaInfo, Member, Message,
    MuteTarget, PostQuoteRequest, Profile, QotyEvent, RecoveryCodesResponse, Server,
    SetMuteRequest, StartQotyRequest, TwoFactorEnabledResponse, TwoFactorStatus,
    UpdateProfileRequest, User,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{mpsc, watch};

use crate::api::{
    Api, ApiError, CropRect, LoginOutcome, MfaOutcome, ProfileMediaKind, check_server_url,
};
use crate::gateway::{Boot, ConnState, GatewayHandle, GatewayUpdate};
use crate::qr::QrMatrix;
use crate::session::Store;
use crate::stream::audio::StreamAudio;
use crate::voice::devices::{AudioConfig, DeviceInfo};
use crate::voice::{AudioMode, VoiceError, VoiceManager, controls::Controls};

type ClipJobs = crate::clips::jobs::Jobs<crate::clips::encode::FfmpegRunner>;
const SIGN_IN_FOR_CLIPS: &str = "Sign in to share clips.";
const NOT_CONNECTED: &str = "Pulse is still connecting. Try again in a moment.";
/// How long a clip command waits for the session's jobs to load (the first Ready).
const CLIP_JOBS_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// Signed in: where, with which token, and as whom (`None` until login or the first Ready says).
#[derive(Clone)]
struct Session {
    api: Api,
    token: String,
    me: Option<UserId>,
    /// False once this session has ended: a media fetch it started must not land afterwards.
    alive: Arc<AtomicBool>,
}

/// Runs on every sign-out: clears what the webview cached (see `media::clear_webview_cache`).
pub type SignedOut = Box<dyn Fn() + Send + Sync>;

/// A password login that still needs its second step: where, and the ticket to send with the
/// code. Never logged or shown. A wrong code leaves it in place (the server counts the tries);
/// success, an expired ticket, going back (`cancel_login`), `register` and `logout` clear it; a
/// new login replaces it.
#[derive(Default)]
struct PendingMfa(Mutex<Option<(Api, String)>>);

impl PendingMfa {
    fn set(&self, api: Api, ticket: String) {
        *self.0.lock().unwrap() = Some((api, ticket));
    }

    fn get(&self) -> Option<(Api, String)> {
        self.0.lock().unwrap().clone()
    }

    fn clear(&self) {
        self.0.lock().unwrap().take();
    }

    /// Clear it only if it is still `ticket`'s (a newer login may have replaced it while this
    /// one's second step was in flight); returns whether it was.
    fn take_if(&self, ticket: &str) -> bool {
        let mut p = self.0.lock().unwrap();
        let mine = p.as_ref().is_some_and(|(_, t)| t == ticket);
        if mine {
            p.take();
        }
        mine
    }
}

pub struct Core {
    pub outbox: std::sync::Arc<crate::outbox::Outbox>,
    inner: Mutex<Option<Session>>,
    /// Retries queued messages every 15 s while signed in (a failed send isn't left waiting for
    /// the next send or reconnect).
    retry: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    gateway: Mutex<Option<GatewayHandle>>,
    /// Last gateway state (bug reports include it).
    conn: Mutex<ConnState>,
    store: Store,
    /// The first gateway session's verdict on the stored login (restore_session waits on it).
    boot: watch::Sender<Boot>,
    /// The UI is listening for `pulse://…` events: the gateway may start before it is, so
    /// `forward` holds everything until then (the first Ready must not be emitted into the void).
    ui_attached: watch::Sender<bool>,
    /// setup and restore_session may both try to start the stored session: one at a time.
    starting: Mutex<()>,
    /// The latest session's server and account in this run (kept across `suspend`, reset by a
    /// sign-out): signing in as anyone else clears the webview's cache first.
    last: Mutex<Option<(String, Option<UserId>)>>,
    /// Where each account's `pulse-media` cache lives (`media::scope_dir`).
    media_root: std::path::PathBuf,
    /// The clip editor's open sources, and where their temp files go (`<app local data>/clips`).
    pub(crate) clip_sources: crate::clips::editor::Sources,
    clips_root: std::path::PathBuf,
    /// The signed-in account's clip jobs, once its Ready named it (`start_clip_jobs`).
    clip_jobs: watch::Sender<Option<ClipJobs>>,
    /// Takes a path off the asset scope for good (`forbid_file`); see `release_source`.
    forbid_asset: Box<dyn Fn(&std::path::Path) + Send + Sync>,
    signed_out: SignedOut,
    mfa: PendingMfa,
}

impl Core {
    pub fn new(
        store: Store,
        outbox: crate::outbox::Outbox,
        media_root: std::path::PathBuf,
        signed_out: SignedOut,
    ) -> Self {
        Self {
            outbox: std::sync::Arc::new(outbox),
            inner: Mutex::new(None),
            retry: Mutex::new(None),
            gateway: Mutex::new(None),
            conn: Mutex::new(ConnState::Connecting),
            store,
            boot: watch::channel(Boot::starting()).0,
            ui_attached: watch::channel(false).0,
            starting: Mutex::new(()),
            last: Mutex::new(None),
            media_root,
            clip_sources: Default::default(),
            clips_root: std::path::PathBuf::new(),
            clip_jobs: watch::channel(None).0,
            forbid_asset: Box::new(|_| {}),
            signed_out,
            mfa: PendingMfa::default(),
        }
    }

    /// Where the clip editor keeps its temp files (`<root>/<account scope>/sources/<id>/`).
    pub fn with_clips(
        mut self,
        root: std::path::PathBuf,
        forbid_asset: Box<dyn Fn(&std::path::Path) + Send + Sync>,
    ) -> Self {
        self.clips_root = root;
        self.forbid_asset = forbid_asset;
        self
    }

    /// A source is done (closed, or the account signed out): its temp files are deleted and the
    /// webview can no longer load them. Only the per-id preview and filmstrip are forbidden, never the
    /// user's own file: forbidding is permanent, and opening the same video again must work.
    fn release_source(&self, e: crate::clips::editor::Entry) {
        for p in [&e.preview, &e.filmstrip].into_iter().flatten() {
            (self.forbid_asset)(p);
        }
        crate::clips::editor::remove_dir_later(e.dir);
    }

    /// This account's clips folder (the same scoping as the media cache); `None` when signed out.
    pub(crate) fn clips_scope(&self) -> Option<std::path::PathBuf> {
        let s = self.inner.lock().unwrap().clone()?;
        let me = s.me?;
        Some(crate::media::scope_dir(&self.clips_root, s.api.base(), &me))
    }

    /// A login or registration begins: whatever second step was pending is superseded (even if
    /// this attempt goes no further), then the address is checked.
    fn start_login(&self, server_url: &str) -> Result<Api, ApiError> {
        self.mfa.clear();
        // the checked, normalised address is the one used for requests and the gateway
        Ok(Api::new(&check_server_url(server_url)?))
    }

    /// The login screen went back from the code step to the password.
    fn cancel_login(&self) {
        self.mfa.clear();
    }

    pub(crate) fn current(&self) -> Result<(Api, String), ApiError> {
        self.inner
            .lock()
            .unwrap()
            .clone()
            .map(|s| (s.api, s.token))
            .ok_or(ApiError::Unauthorized)
    }

    /// What `pulse-media` fetches with, and the cache it may use: `None` when signed out.
    pub(crate) fn media_session(&self) -> Option<crate::media::Session> {
        let s = self.inner.lock().unwrap().clone()?;
        let cache =
            s.me.as_ref()
                .map(|me| crate::media::scope_dir(&self.media_root, s.api.base(), me));
        Some(crate::media::Session {
            api: s.api,
            token: s.token,
            cache,
            alive: s.alive,
        })
    }

    /// Ready named the account behind `token`; ignored if that session has ended since.
    fn learn_me(&self, token: &str, me: UserId) {
        if let Some(s) = self.inner.lock().unwrap().as_mut()
            && s.token == token
        {
            s.me = Some(me);
            // Folders no open source owns (a sign-out in the middle of a job, a crash) go.
            if !self.clips_root.as_os_str().is_empty() {
                let scope = crate::media::scope_dir(&self.clips_root, s.api.base(), &me);
                let open = self.clip_sources.ids();
                tauri::async_runtime::spawn_blocking(move || {
                    crate::clips::editor::clear_stale_sources(&scope, &open);
                });
            }
            *self.last.lock().unwrap() = Some((s.api.base().to_string(), Some(me)));
        }
    }

    /// The session `token` is ready: load its account's clip jobs from disk (once per session; a
    /// reconnect's Ready keeps them). Jobs that were running when Pulse stopped come back failed.
    async fn start_clip_jobs(&self, app: &AppHandle, token: &str) {
        use crate::clips::{encode::FfmpegRunner, ffmpeg::FfmpegPaths, jobs};
        if self.clips_root.as_os_str().is_empty()
            || self
                .clip_jobs
                .borrow()
                .as_ref()
                .is_some_and(|j| j.token() == token)
        {
            return;
        }
        let Some(s) = self
            .inner
            .lock()
            .unwrap()
            .clone()
            .filter(|s| s.token == token)
        else {
            return;
        };
        let Some(dir) = self.clip_jobs_dir(&s) else {
            return;
        };
        // Without ffmpeg (a broken install) a job can still upload and post; preparing fails.
        let paths = ffmpeg_paths(app).unwrap_or(FfmpegPaths {
            ffmpeg: Default::default(),
            ffprobe: Default::default(),
        });
        let (emitter, allow, forbid) = (app.clone(), app.clone(), app.clone());
        let hooks = jobs::Hooks {
            emit: Box::new(move |job| {
                let _ = emitter.emit("clip://job", job);
            }),
            allow_file: Box::new(move |p| {
                let _ = allow.asset_protocol_scope().allow_file(p);
            }),
            forbid_file: Box::new(move |p| {
                let _ = forbid.asset_protocol_scope().forbid_file(p);
            }),
        };
        let loaded = jobs::Jobs::load(dir, FfmpegRunner(paths), s.api, s.token, hooks).await;
        // what's left from last time arrives like any other job update
        for job in self.install_clip_jobs(token, loaded).unwrap_or_default() {
            let _ = app.emit("clip://job", job);
        }
    }

    /// Install `loaded` as session `token`'s clip jobs and return what they hold, unless that
    /// session ended (or another Ready installed them) while the disk was read: then they're
    /// stopped. Checked and swapped under `inner`, the lock `forget` takes before it clears the
    /// jobs, so a sign-out either finds them installed (and discards them) or they never are.
    fn install_clip_jobs(
        &self,
        token: &str,
        loaded: ClipJobs,
    ) -> Option<Vec<crate::clips::jobs::ClipJob>> {
        let inner = self.inner.lock().unwrap();
        let current = inner.as_ref().is_some_and(|s| s.token == token);
        let taken = self
            .clip_jobs
            .borrow()
            .as_ref()
            .is_some_and(|j| j.token() == token);
        if !current || taken {
            drop(inner);
            loaded.stop();
            return None;
        }
        let found = loaded.list();
        let old = self.clip_jobs.send_replace(Some(loaded));
        drop(inner);
        if let Some(old) = old {
            old.stop();
        }
        Some(found)
    }

    /// Quitting: stop the clip jobs and close every editor source, then wait until their ffmpeg
    /// runs have been dropped (and so killed). The runtime's tasks are never dropped at exit, and
    /// neither OS kills a child with its parent. Files stay for next time (a paused job comes back
    /// failed; stale source folders are cleared at the next sign-in).
    pub async fn stop_clips(&self) {
        let jobs = self.clip_jobs.send_replace(None);
        let work: Vec<_> = self
            .clip_sources
            .drain()
            .into_iter()
            .map(|(_, w)| w)
            .collect();
        let jobs_done = async {
            if let Some(j) = jobs {
                j.shutdown().await;
            }
        };
        // a preview or filmstrip holds its source's lock until its ffmpeg run is dropped
        let editor_done = async {
            for w in &work {
                drop(w.lock().await);
            }
        };
        tokio::join!(jobs_done, editor_done);
    }

    /// `<clips scope>/jobs` of this session's account, once known.
    fn clip_jobs_dir(&self, s: &Session) -> Option<std::path::PathBuf> {
        if self.clips_root.as_os_str().is_empty() {
            return None;
        }
        let me = s.me?;
        Some(crate::media::scope_dir(&self.clips_root, s.api.base(), &me).join("jobs"))
    }

    /// Signing out: stop the clip jobs, then (the returned future) delete their tickets, while
    /// `s`'s token still works, and their folder, whether they were loaded or not.
    fn discard_clip_jobs(
        &self,
        s: &Session,
    ) -> Option<impl std::future::Future<Output = ()> + Send + use<>> {
        if let Some(jobs) = self.clip_jobs.send_replace(None) {
            jobs.stop();
        }
        let dir = self.clip_jobs_dir(s)?;
        Some(crate::clips::jobs::discard_all(
            dir,
            s.api.clone(),
            s.token.clone(),
        ))
    }

    /// This session's clip jobs, waiting for them to load (on the first Ready). The error is the
    /// reason to show: signed out, or not connected yet.
    async fn clip_jobs(&self) -> Result<ClipJobs, String> {
        self.clip_jobs_within(CLIP_JOBS_WAIT).await
    }

    async fn clip_jobs_within(&self, wait: std::time::Duration) -> Result<ClipJobs, String> {
        let (_, token) = self.current().map_err(|_| SIGN_IN_FOR_CLIPS.to_owned())?;
        let mut rx = self.clip_jobs.subscribe();
        let ready = rx.wait_for(|j| j.as_ref().is_some_and(|j| j.token() == token));
        match tokio::time::timeout(wait, ready).await {
            Ok(Ok(j)) => j.clone().ok_or_else(|| NOT_CONNECTED.to_owned()),
            _ => Err(NOT_CONNECTED.to_owned()),
        }
    }

    /// Cancel a clip job. Signed out is fine (sign-out deleted it); not connected yet is an error
    /// to show, since nothing was cancelled.
    async fn cancel_clip_job(&self, id: &str, wait: std::time::Duration) -> Result<(), String> {
        match self.clip_jobs_within(wait).await {
            Ok(jobs) => jobs.cancel(id).await,
            Err(e) if e == SIGN_IN_FOR_CLIPS => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Remember the session and (re)start the live connection.
    fn activate(
        &self,
        app: &AppHandle,
        api: Api,
        token: String,
        me: Option<UserId>,
        persist: bool,
    ) {
        self.boot.send_replace(Boot::starting());
        crate::logging::session_start(); // report log tails start here
        if persist {
            self.store.save(api.base(), &token);
        }
        self.begin(api.clone(), token.clone(), me);
        let (tx, rx) = mpsc::unbounded_channel();
        tauri::async_runtime::spawn(crate::reports::crash::send_pending(
            app.clone(),
            api.clone(),
            token.clone(),
        ));
        let declare: crate::gateway::AfterReady = {
            let app = app.clone();
            std::sync::Arc::new(move || {
                // try_state: the gateway may start before voice is set up
                app.try_state::<VoiceManager>()
                    .map(|v| after_ready_frames(v.controls()))
                    .unwrap_or_default()
            })
        };
        let handle = GatewayHandle::spawn_full(
            api.base().to_string(),
            token.clone(),
            tx,
            crate::gateway::Tuning::default(),
            declare,
        );
        // Replacing an old handle drops (and stops) it.
        *self.gateway.lock().unwrap() = Some(handle);
        tauri::async_runtime::spawn(forward(app.clone(), rx, token));
        let app2 = app.clone();
        let timer = tauri::async_runtime::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(15));
            loop {
                tick.tick().await;
                let core = app2.state::<Core>();
                if core.outbox.pending_len() == 0 {
                    continue;
                }
                if let Ok((api, token)) = core.current() {
                    core.outbox.flush(&api, &token, true).await;
                }
            }
        });
        if let Some(old) = self.retry.lock().unwrap().replace(timer) {
            old.abort();
        }
    }

    /// The session part of `activate`. Unless this is provably the same account on the same
    /// server as the run's previous session, the webview's cache is cleared first: it outlives
    /// restarts and a failed restore, and its images say `immutable`.
    fn begin(&self, api: Api, token: String, me: Option<UserId>) {
        let next = (api.base().to_string(), me);
        let same = matches!(
            self.last.lock().unwrap().replace(next.clone()),
            Some(prev) if prev == next && me.is_some()
        );
        if !same {
            (self.signed_out)();
        }
        let mut inner = self.inner.lock().unwrap();
        // the same token (a webview reload) is the same session: its fetches end with it
        let alive = match inner.take() {
            Some(old) if old.token == token => old.alive,
            old => {
                if let Some(old) = old {
                    old.alive.store(false, Ordering::SeqCst);
                }
                Arc::new(AtomicBool::new(true))
            }
        };
        *inner = Some(Session {
            api,
            token,
            me,
            alive,
        });
    }

    /// Startup couldn't restore the stored session. `refused`: its token is dead, so it is
    /// signed out (the caller clears the store); otherwise it stays for the next launch.
    fn give_up_restoring(&self, refused: bool) {
        self.suspend();
        if refused {
            self.forget();
        }
    }

    /// Start the stored session's live connection, once (setup and restore_session both ask; the
    /// first wins). `restart`: the webview reloaded, so a running session starts a fresh gateway
    /// (the new page needs its own Ready). The stored server, or `None` when nobody is signed in.
    pub(crate) fn start_stored(&self, app: &AppHandle, restart: bool) -> Option<String> {
        let _one = self.starting.lock().unwrap();
        let running = self.inner.lock().unwrap().clone();
        if let Some(s) = running {
            let server = s.api.base().to_string();
            if restart {
                self.activate(app, s.api, s.token, s.me, false);
            }
            return Some(server);
        }
        let (api, token) = self.load_stored()?;
        let server = api.base().to_string();
        self.activate(app, api, token, None, false);
        Some(server)
    }

    /// The stored session to restore, if there is one. Its server address goes through the same
    /// check as a login's and is used as checked (normalised). An address the check refuses (a
    /// hand-edited store, or one saved by an older version) can't be restored: the stored session
    /// is cleared, as for a dead token, and startup shows the login screen.
    fn load_stored(&self) -> Option<(Api, String)> {
        let (stored, token) = self.store.load()?;
        let server = match check_server_url(&stored) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "stored session's server address refused; signed out");
                self.store.clear(&stored);
                return None;
            }
        };
        if server != stored {
            // Re-key it under the address used from now on, so sign-out clears the right entry.
            self.store.clear(&stored);
            self.store.save(&server, &token);
        }
        Some((Api::new(&server), token))
    }

    /// The stored session's server didn't answer at startup: stop trying, but keep it stored
    /// (the next launch tries again, as before).
    fn suspend(&self) {
        self.gateway.lock().unwrap().take();
        if let Some(t) = self.retry.lock().unwrap().take() {
            t.abort();
        }
        if let Some(s) = self.inner.lock().unwrap().take() {
            s.alive.store(false, Ordering::SeqCst);
        }
        // after `inner`, as in `forget`: `install_clip_jobs` checks the session under that lock,
        // so jobs installed before this are stopped here, and any later install is refused
        if let Some(jobs) = self.clip_jobs.send_replace(None) {
            jobs.stop();
        }
    }

    /// Turns true once the UI listens for `pulse://…` events (anything emitted earlier is lost).
    pub(crate) fn ui_listening(&self) -> watch::Receiver<bool> {
        self.ui_attached.subscribe()
    }

    /// Forget the session locally (logout, or the server said our token is dead).
    fn deactivate(&self, app: &AppHandle) {
        self.gateway.lock().unwrap().take();
        if let Some(t) = self.retry.lock().unwrap().take() {
            t.abort();
        }
        // Signed out: nothing from this session may keep running or be sent under the next account.
        self.outbox.clear();
        app.state::<VoiceManager>().forget_tokens();
        app.state::<crate::voice::mictest::MicTest>().stop();
        let app = app.clone();
        tauri::async_runtime::spawn(async move { app.state::<VoiceManager>().leave().await });
        self.forget_stored();
    }

    /// `forget`, and the stored session with it (the next launch starts at the login screen).
    /// A paused session (`suspend`) has no live session to name its server, but is still
    /// stored: whatever is stored goes.
    fn forget_stored(&self) {
        let server = match self.forget() {
            Some(api) => Some(api.base().to_string()),
            None => self.store.load().map(|(server, _)| server),
        };
        if let Some(server) = server {
            self.store.clear(&server);
        }
    }

    /// Drop the session and what it cached on this machine: its media cache scope, and the
    /// webview's copies. The next login (even the same account) fetches afresh. The session's
    /// server, if there was one.
    fn forget(&self) -> Option<Api> {
        let session = self.inner.lock().unwrap().take();
        if let Some(s) = &session {
            s.alive.store(false, Ordering::SeqCst);
        }
        let last = self.last.lock().unwrap().take();
        match session.as_ref().and_then(|s| self.discard_clip_jobs(s)) {
            Some(discard) => {
                tauri::async_runtime::spawn(discard);
            }
            None => {
                if let Some(jobs) = self.clip_jobs.send_replace(None) {
                    jobs.stop();
                }
                // a paused session (its token isn't usable here): the folder at least goes, the
                // server sweeps the tickets
                if let Some((base, Some(me))) = last
                    && !self.clips_root.as_os_str().is_empty()
                {
                    let dir = crate::media::scope_dir(&self.clips_root, &base, &me).join("jobs");
                    tauri::async_runtime::spawn_blocking(move || {
                        let _ = std::fs::remove_dir_all(dir);
                    });
                }
            }
        }
        for (e, _) in self.clip_sources.drain() {
            self.release_source(e);
        }
        (self.signed_out)();
        let s = session?;
        if let Some(me) = &s.me {
            let scope = crate::media::scope_dir(&self.media_root, s.api.base(), me);
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(e) = std::fs::remove_dir_all(&scope)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!(error = %e, "media: cache not removed");
                }
            });
        }
        Some(s.api)
    }

    /// A 401 outside login means the session died server-side: drop it and tell the UI.
    pub(crate) fn check<T>(&self, app: &AppHandle, r: Result<T, ApiError>) -> Result<T, ApiError> {
        if let Err(ApiError::Unauthorized) = &r {
            self.deactivate(app);
            let _ = app.emit("pulse://conn", ConnState::LoggedOut);
        }
        r
    }

    pub(crate) fn conn(&self) -> ConnState {
        *self.conn.lock().unwrap()
    }

    pub(crate) fn send_frame(&self, f: ClientFrame) {
        if let Some(g) = self.gateway.lock().unwrap().as_ref() {
            g.send(f);
        }
    }

    /// Check the live connection now (window focused, network back).
    pub(crate) fn probe_gateway(&self) {
        if let Some(g) = self.gateway.lock().unwrap().as_ref() {
            g.probe();
        }
    }
}

/// Gateway updates → Tauri events for the UI.
async fn forward(app: AppHandle, mut rx: mpsc::UnboundedReceiver<GatewayUpdate>, token: String) {
    let mut attached = app.state::<Core>().ui_listening();
    let _ = attached.wait_for(|a| *a).await;
    while let Some(u) = rx.recv().await {
        match &u {
            GatewayUpdate::Ready(r) => {
                app.state::<Core>().learn_me(&token, r.me.id);
                // the jobs load on their own: Ready (and what follows) never waits for the disk
                let (a, t) = (app.clone(), token.clone());
                tauri::async_runtime::spawn(async move {
                    a.state::<Core>().start_clip_jobs(&a, &t).await;
                });
                let _ = app.emit("pulse://ready", &**r);
            }
            GatewayUpdate::Event(e) => {
                let _ = app.emit("pulse://event", e);
            }
            GatewayUpdate::Connection(state) => {
                let state = *state;
                *app.state::<Core>().conn.lock().unwrap() = state;
                tracing::info!(?state, "gateway");
                if state == ConnState::LoggedOut {
                    app.state::<Core>().deactivate(&app);
                }
                if state == ConnState::Connected {
                    // Back online: send anything queued while we were away.
                    let core = app.state::<Core>();
                    if let Ok((api, token)) = core.current() {
                        let outbox = core.outbox.clone();
                        tauri::async_runtime::spawn(async move {
                            outbox.flush(&api, &token, true).await
                        });
                    }
                }
                let _ = app.emit("pulse://conn", state);
            }
        }
        // after the emit: restore_session returns only once the UI has the Ready
        app.state::<Core>()
            .boot
            .send_modify(|b| b.observe(&u, std::time::Instant::now()));
    }
}

/// What `login` and `login_mfa` tell the UI.
#[derive(Debug, serde::Serialize, PartialEq)]
#[serde(tag = "status", content = "user", rename_all = "snake_case")]
pub enum LoginResult {
    Ready(User),
    /// Password accepted; ask for the 6-digit code (or a recovery code), then `login_mfa`.
    MfaRequired,
    /// The ticket is gone (`login_mfa` only): back to the password.
    Expired,
}

#[tauri::command]
pub async fn login(
    app: AppHandle,
    core: State<'_, Core>,
    server_url: String,
    username: String,
    password: String,
) -> Result<LoginResult, ApiError> {
    let api = core.start_login(&server_url)?;
    match api.login(&username, &password).await? {
        LoginOutcome::Session(s) => {
            core.activate(&app, api, s.token, Some(s.user.id), true);
            Ok(LoginResult::Ready(s.user))
        }
        LoginOutcome::MfaRequired { ticket } => {
            core.mfa.set(api, ticket);
            Ok(LoginResult::MfaRequired)
        }
    }
}

/// The second login step, for the ticket `login` stored. No ticket (or a dead one) is `Expired`.
#[tauri::command]
pub async fn login_mfa(
    app: AppHandle,
    core: State<'_, Core>,
    code: String,
) -> Result<LoginResult, ApiError> {
    let Some((api, ticket)) = core.mfa.get() else {
        return Ok(LoginResult::Expired);
    };
    match api.login_mfa(&ticket, &code).await? {
        MfaOutcome::Session(s) if core.mfa.take_if(&ticket) => {
            core.activate(&app, api, s.token, Some(s.user.id), true);
            Ok(LoginResult::Ready(s.user))
        }
        MfaOutcome::Session(s) => {
            // The user went back or started another login meanwhile: that one wins, and this
            // session is ended rather than left signed in somewhere unseen.
            let _ = api.logout(&s.token).await;
            Ok(LoginResult::Expired)
        }
        MfaOutcome::Expired => {
            core.mfa.take_if(&ticket);
            Ok(LoginResult::Expired)
        }
    }
}

/// The login screen's Back from the code step: drop the pending ticket now.
#[tauri::command]
pub fn cancel_login(core: State<'_, Core>) {
    core.cancel_login();
}

#[tauri::command]
pub async fn register(
    app: AppHandle,
    core: State<'_, Core>,
    server_url: String,
    invite_code: String,
    username: String,
    password: String,
) -> Result<User, ApiError> {
    let api = core.start_login(&server_url)?;
    let s = api.register(&invite_code, &username, &password).await?;
    core.activate(&app, api, s.token, Some(s.user.id), true);
    Ok(s.user)
}

/// Longest startup waits for the first gateway session to decide (connect 10 s + Ready 10 s).
const BOOT_WAIT: std::time::Duration = std::time::Duration::from_secs(25);

/// What `restore_session` found.
#[derive(Debug, serde::Serialize, PartialEq)]
#[serde(tag = "status", content = "user", rename_all = "snake_case")]
pub enum Restored {
    /// Signed in and connected.
    Ready(User),
    /// Nothing stored, or the session is dead: show the login screen.
    LoggedOut,
    /// Signed in, but the server turned the connection away (too many devices / attempts). The
    /// gateway keeps retrying; the app opens when Ready arrives.
    Refused,
}

/// Resume the stored session.
#[tauri::command]
pub async fn restore_session(
    app: AppHandle,
    core: State<'_, Core>,
    voice: State<'_, VoiceManager>,
) -> Result<Restored, ApiError> {
    crate::startup::mark("restore_begin");
    // Runs once per page load: a reload (renderer crash) can't leave the old page's meters
    // counted or its status panel's 1 s link polling running.
    voice.reset_page_views();
    // The UI listens before it calls this. Already attached = the webview reloaded.
    let reload = core.ui_attached.send_replace(true);
    let Some(server) = core.start_stored(&app, reload) else {
        return Ok(Restored::LoggedOut);
    };
    let mut boot = core.boot.subscribe();
    let decided = tokio::time::timeout(BOOT_WAIT, boot.wait_for(|b| !b.is_waiting()))
        .await
        .ok()
        .and_then(|r| r.ok().map(|b| b.clone()));
    crate::startup::mark("restore_end");
    match decided {
        Some(Boot::Ready(user)) => Ok(Restored::Ready(user)),
        Some(Boot::LoggedOut) => Ok(Restored::LoggedOut), // forward() already cleared the stored session
        // Turned away (too many devices / attempts) but signed in: leave the session and its
        // gateway running. The UI sees the refusal in `pulse://conn`, shows it, and opens the
        // app when Ready arrives.
        Some(Boot::Refused(_)) => Ok(Restored::Refused),
        Some(Boot::UpdateRequired) => {
            // the update screen sits inside the signed-in shell: it needs the user (rare path)
            let (api, token) = core.current()?;
            api.me(&token)
                .await
                .map(Restored::Ready)
                .map_err(|e| e.while_restoring(&server))
        }
        _ => {
            let why = match core.current() {
                Ok((api, token)) => unrestored_reason(&api, &token).await,
                Err(_) => ApiError::Network(crate::api::CONNECT_FAILED.into()),
            };
            core.give_up_restoring(matches!(why, ApiError::Unauthorized));
            match why {
                ApiError::Unauthorized => {
                    core.store.clear(&server);
                    Ok(Restored::LoggedOut)
                }
                e => Err(e.while_restoring(&server)),
            }
        }
    }
}

/// Longest startup asks `/me` why a stored session never reached Ready.
const PROBE_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Startup's stored session never reached Ready: ask `/me` once why (off the happy path), so a
/// reachable server that's struggling says so instead of "didn't answer". Unauthorized = the
/// token is dead; Server = it's having trouble; anything else stays "couldn't connect".
async fn unrestored_reason(api: &Api, token: &str) -> ApiError {
    match tokio::time::timeout(PROBE_WAIT, api.me(token)).await {
        Ok(Err(e @ (ApiError::Unauthorized | ApiError::Server(_)))) => e,
        _ => ApiError::Network(crate::api::CONNECT_FAILED.into()),
    }
}

#[tauri::command]
pub async fn logout(app: AppHandle, core: State<'_, Core>) -> Result<(), ApiError> {
    core.mfa.clear();
    let session = core.inner.lock().unwrap().clone();
    if let Some(discard) = session.as_ref().and_then(|s| core.discard_clip_jobs(s)) {
        // while the token still works, so their tickets go too (a few seconds at most)
        discard.await;
    }
    if let Ok((api, token)) = core.current() {
        let _ = api.logout(&token).await;
    }
    core.deactivate(&app);
    Ok(())
}

/// Settings → Advanced: sign out every other device; this one stays signed in.
#[tauri::command]
pub async fn logout_others(app: AppHandle, core: State<'_, Core>) -> Result<u32, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.logout_others(&token).await)
}

/// Settings → Advanced: change the password (`code` when two-factor is on); every other device
/// is signed out. The passwords are passed straight on: never stored or logged.
#[tauri::command]
pub async fn change_password(
    app: AppHandle,
    core: State<'_, Core>,
    current_password: String,
    new_password: String,
    code: Option<String>,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    let code = code.as_deref().map(str::trim).filter(|c| !c.is_empty());
    core.check(
        &app,
        api.change_password(&token, &current_password, &new_password, code)
            .await,
    )
}

/// Settings → Advanced: is two-factor on, and can this server do it at all.
#[tauri::command]
pub async fn two_factor_status(
    app: AppHandle,
    core: State<'_, Core>,
) -> Result<TwoFactorStatus, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.two_factor_status(&token).await)
}

/// What the setup screen shows: the secret for typing in, and the QR code as a module matrix
/// (the `otpauth://` URI stays here). Not `Debug`: it holds the secret.
#[derive(serde::Serialize)]
pub struct TwoFactorSetup {
    pub secret: String,
    pub qr: QrMatrix,
}

/// "ABCDEFGHIJ" -> "ABCD EFGH IJ": easier to read off a screen and type into an authenticator.
fn group_secret(secret: &str) -> String {
    secret
        .as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

#[tauri::command]
pub async fn two_factor_setup(
    app: AppHandle,
    core: State<'_, Core>,
    password: String,
) -> Result<TwoFactorSetup, ApiError> {
    let (api, token) = core.current()?;
    let r = core.check(&app, api.two_factor_setup(&token, &password).await)?;
    let qr = crate::qr::qr_matrix(&r.otpauth_uri).ok_or_else(|| {
        ApiError::Rejected("Couldn't draw the QR code; type the key in instead.".into())
    })?;
    Ok(TwoFactorSetup {
        secret: group_secret(&r.secret),
        qr,
    })
}

/// Turns two-factor on; the response holds the recovery codes, shown once.
#[tauri::command]
pub async fn two_factor_enable(
    app: AppHandle,
    core: State<'_, Core>,
    code: String,
) -> Result<TwoFactorEnabledResponse, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.two_factor_enable(&token, &code).await)
}

#[tauri::command]
pub async fn two_factor_disable(
    app: AppHandle,
    core: State<'_, Core>,
    code: String,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.two_factor_disable(&token, &code).await)
}

#[tauri::command]
pub async fn two_factor_regenerate(
    app: AppHandle,
    core: State<'_, Core>,
    code: String,
) -> Result<RecoveryCodesResponse, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.two_factor_regenerate(&token, &code).await)
}

/// The UI calls this when the network comes back (`online`); window focus probes from Rust.
#[tauri::command]
pub fn gateway_probe(core: State<'_, Core>) {
    core.probe_gateway();
}

#[tauri::command]
pub async fn list_servers(app: AppHandle, core: State<'_, Core>) -> Result<Vec<Server>, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.servers(&token).await)
}

#[tauri::command]
pub async fn create_server(
    app: AppHandle,
    core: State<'_, Core>,
    name: String,
) -> Result<Server, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.create_server(&token, &name).await)
}

#[tauri::command]
pub async fn create_channel(
    app: AppHandle,
    core: State<'_, Core>,
    server_id: ServerId,
    kind: pulse_protocol::rest::ChannelKind,
    name: String,
) -> Result<Channel, ApiError> {
    let (api, token) = core.current()?;
    core.check(
        &app,
        api.create_channel(&token, server_id, kind, &name).await,
    )
}

#[tauri::command]
pub async fn join_server(
    app: AppHandle,
    core: State<'_, Core>,
    server_id: ServerId,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.join_server(&token, server_id).await)
}

#[tauri::command]
pub async fn list_channels(
    app: AppHandle,
    core: State<'_, Core>,
    server_id: ServerId,
) -> Result<Vec<Channel>, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.channels(&token, server_id).await)
}

#[tauri::command]
pub async fn list_members(
    app: AppHandle,
    core: State<'_, Core>,
    server_id: ServerId,
) -> Result<Vec<Member>, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.members(&token, server_id).await)
}

#[tauri::command]
pub async fn list_messages(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    before: Option<MessageId>,
) -> Result<Vec<Message>, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.messages(&token, channel_id, before).await)
}

#[tauri::command]
pub async fn send_message(
    core: State<'_, Core>,
    channel_id: ChannelId,
    content: String,
    reply_to_id: Option<MessageId>,
    nonce: String,
) -> Result<(), ApiError> {
    // Queue first, then try: offline sends survive until the connection is back (or fail visibly).
    core.outbox
        .enqueue(nonce, channel_id, content, reply_to_id, None);
    if let Ok((api, token)) = core.current() {
        core.outbox.flush(&api, &token, false).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn edit_message(
    app: AppHandle,
    core: State<'_, Core>,
    message_id: MessageId,
    content: String,
) -> Result<Message, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.edit_message(&token, message_id, &content).await)
}

#[tauri::command]
pub async fn delete_message(
    app: AppHandle,
    core: State<'_, Core>,
    message_id: MessageId,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.delete_message(&token, message_id).await)
}

/// Tell others you're typing (the UI throttles to 1 per 3 s).
#[tauri::command]
pub fn send_typing(core: State<'_, Core>, channel_id: pulse_protocol::ids::ChannelId) {
    core.send_frame(ClientFrame::Typing { channel_id });
}

// ---------- voice ----------

fn flags(c: Controls) -> pulse_protocol::gateway::VoiceFlags {
    pulse_protocol::gateway::VoiceFlags {
        muted: c.muted,
        deafened: c.deafened,
    }
}

/// Sent after every gateway Ready: the server forgets declared flags on restart, and a toggle
/// made while offline never reached it.
fn after_ready_frames(c: Controls) -> Vec<ClientFrame> {
    vec![ClientFrame::VoiceState { flags: flags(c) }]
}

#[tauri::command]
pub async fn join_voice(
    app: AppHandle,
    core: State<'_, Core>,
    voice: State<'_, VoiceManager>,
    stream_audio: State<'_, StreamAudio>,
    channel_id: ChannelId,
    config: AudioConfig,
) -> Result<(), VoiceError> {
    let (api, token) = core.current()?;
    tracing::info!(%channel_id, input = ?config.input, output = ?config.output, gain = config.input_gain_pct, "join_voice");
    // a switch leaves the last channel's stream room (and ends our stream audio there)
    stream_audio.leave().await;
    let r = voice
        .join(
            &api,
            &token,
            channel_id,
            config.normalized(),
            AudioMode::Real,
        )
        .await;
    if let Err(VoiceError::Api(ApiError::Unauthorized)) = &r {
        let _ = core.check::<()>(&app, Err(ApiError::Unauthorized));
    }
    r?;
    // Tell everyone our current mute/deafen state (it persists across channels).
    core.send_frame(ClientFrame::VoiceState {
        flags: flags(voice.controls()),
    });
    Ok(())
}

#[tauri::command]
pub async fn leave_voice(
    voice: State<'_, VoiceManager>,
    stream_audio: State<'_, StreamAudio>,
) -> Result<(), VoiceError> {
    stream_audio.leave().await;
    voice.leave().await;
    Ok(())
}

/// Hovering a voice channel (a click usually follows within 150-400 ms).
#[tauri::command]
pub async fn prefetch_voice(
    core: State<'_, Core>,
    voice: State<'_, VoiceManager>,
    channel_id: ChannelId,
) -> Result<(), VoiceError> {
    if let Ok((api, token)) = core.current() {
        voice.prefetch(&api, &token, channel_id);
    }
    Ok(())
}

#[tauri::command]
pub async fn toggle_mute(
    core: State<'_, Core>,
    voice: State<'_, VoiceManager>,
) -> Result<Controls, VoiceError> {
    let c = voice.toggle_mute().await;
    core.send_frame(ClientFrame::VoiceState { flags: flags(c) });
    Ok(c)
}

#[tauri::command]
pub async fn toggle_deafen(
    core: State<'_, Core>,
    voice: State<'_, VoiceManager>,
) -> Result<Controls, VoiceError> {
    let c = voice.toggle_deafen().await;
    core.send_frame(ClientFrame::VoiceState { flags: flags(c) });
    Ok(c)
}

#[tauri::command]
pub async fn set_peer_volume(
    voice: State<'_, VoiceManager>,
    user_id: String,
    percent: u16,
) -> Result<(), VoiceError> {
    voice.set_peer_volume(user_id, percent).await;
    Ok(())
}

#[tauri::command]
pub async fn set_audio_config(
    voice: State<'_, VoiceManager>,
    config: AudioConfig,
) -> Result<(), VoiceError> {
    voice.set_audio_config(config.normalized()).await
}

#[derive(serde::Serialize)]
pub struct AudioDevices {
    inputs: Vec<DeviceInfo>,
    outputs: Vec<DeviceInfo>,
}

/// async: runs on the runtime, not the UI thread (device enumeration can block).
#[tauri::command]
pub async fn list_audio_devices() -> AudioDevices {
    AudioDevices {
        inputs: crate::voice::devices::list_inputs(),
        outputs: crate::voice::devices::list_outputs(),
    }
}

/// async: MicTest spawns tokio tasks, which need a runtime (sync commands run on the UI thread).
#[tauri::command]
pub async fn start_mic_test(
    app: AppHandle,
    mic: State<'_, crate::voice::mictest::MicTest>,
    config: AudioConfig,
) -> Result<(), VoiceError> {
    let handle = app.clone();
    // opening devices can block for seconds (a Bluetooth headset): let the runtime move other
    // work off this worker meanwhile
    tokio::task::block_in_place(|| {
        mic.start(
            &config.normalized(),
            std::sync::Arc::new(move |e| {
                let _ = handle.emit("voice://event", &e);
            }),
        )
    })
}

#[tauri::command]
pub async fn stop_mic_test(
    mic: State<'_, crate::voice::mictest::MicTest>,
) -> Result<(), VoiceError> {
    mic.stop();
    Ok(())
}

/// A voice meter appeared in (`on`) or left the UI: in a call, levels are sent only while one shows.
#[tauri::command]
pub fn watch_voice_meters(voice: State<'_, VoiceManager>, on: bool) {
    voice.watch_meters(on);
}

/// The status panel opened (`on`) or closed: ping, jitter and loss are sent only while it is open.
#[tauri::command]
pub fn watch_voice_link(voice: State<'_, VoiceManager>, on: bool) {
    voice.watch_link(on);
}

/// Hotkey path (IPC on Linux, keyboard/mouse hooks on Windows): toggle + broadcast, same as the buttons.
pub fn hotkey(app: &AppHandle, c: crate::ipc::Command) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let voice = app.state::<VoiceManager>();
        let controls = match c {
            crate::ipc::Command::ToggleMute => voice.toggle_mute().await,
            crate::ipc::Command::ToggleDeafen => voice.toggle_deafen().await,
        };
        app.state::<Core>().send_frame(ClientFrame::VoiceState {
            flags: flags(controls),
        });
    });
}

static SOUND_LOG_BUDGET: std::sync::Mutex<Option<crate::logging::LineBudget>> =
    std::sync::Mutex::new(None);

/// UI sound, played natively (see sounds.rs for why not in the webview).
#[tauri::command]
pub async fn play_sound(
    name: String,
    set: Option<String>,
    volume: Option<f32>,
    output: Option<String>,
) {
    let set = set.as_deref().unwrap_or(crate::sounds::DEFAULT_SET);
    // with `window focus`, shows whether a message sound was asked for (a name from a fixed list);
    // budgeted, so a page loop can't flood the log
    crate::logging::log_budgeted(
        &SOUND_LOG_BUDGET,
        20,
        2.0,
        || tracing::info!(name = %crate::logging::one_line(&name, 32), "sound"),
        |n| tracing::warn!("sound: {n} lines dropped (too many)"),
    );
    crate::sounds::play(set, &name, crate::sounds::gain(volume), output);
}

#[tauri::command]
pub async fn create_dm(
    app: AppHandle,
    core: State<'_, Core>,
    user_ids: Vec<UserId>,
) -> Result<Channel, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.create_dm(&token, user_ids).await)
}

#[tauri::command]
pub async fn add_members(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    user_ids: Vec<UserId>,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.add_members(&token, channel_id, user_ids).await)
}

#[tauri::command]
pub async fn remove_member(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    user_id: UserId,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.remove_member(&token, channel_id, user_id).await)
}

#[tauri::command]
pub async fn rename_channel(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    name: Option<String>,
) -> Result<Channel, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.rename_channel(&token, channel_id, name).await)
}

#[tauri::command]
pub async fn mark_read(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    message_id: MessageId,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.mark_read(&token, channel_id, message_id).await)
}

#[tauri::command]
pub async fn set_mute(
    app: AppHandle,
    core: State<'_, Core>,
    target_kind: MuteTarget,
    target_id: String,
    until: Option<String>,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    let req = SetMuteRequest {
        target_kind,
        target_id,
        until,
    };
    core.check(&app, api.set_mute(&token, &req).await)
}

#[tauri::command]
pub async fn clear_mute(
    app: AppHandle,
    core: State<'_, Core>,
    target_kind: MuteTarget,
    target_id: String,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.clear_mute(&token, target_kind, &target_id).await)
}

#[tauri::command]
pub async fn close_conversation(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.close_conversation(&token, channel_id).await)
}

// ---------- emoji + reactions ----------

/// The UI passes the file as a raw `Uint8Array` (a JSON number array of 2 MB is slow).
#[tauri::command]
pub async fn upload_emoji_media(
    app: AppHandle,
    core: State<'_, Core>,
    request: tauri::ipc::Request<'_>,
) -> Result<MediaInfo, ApiError> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(ApiError::Rejected("expected the file's bytes".into()));
    };
    let (api, token) = core.current()?;
    core.check(&app, api.upload_emoji_media(&token, bytes.clone()).await)
}

// ---------- Profiles ----------

#[tauri::command]
pub async fn get_profile(
    app: AppHandle,
    core: State<'_, Core>,
    user_id: UserId,
) -> Result<Profile, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.profile(&token, user_id).await)
}

#[tauri::command]
pub async fn update_profile(
    app: AppHandle,
    core: State<'_, Core>,
    request: UpdateProfileRequest,
) -> Result<Profile, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.update_profile(&token, &request).await)
}

/// The `x-pulse-crop` header is `x,y,w,h`; absent means no crop.
fn parse_crop(header: Option<&str>) -> Result<Option<CropRect>, ApiError> {
    let Some(h) = header else { return Ok(None) };
    let bad = || ApiError::Rejected("bad crop".into());
    let n = h
        .split(',')
        .map(|p| p.trim().parse::<u32>().map_err(|_| bad()))
        .collect::<Result<Vec<_>, _>>()?;
    let [x, y, w, h] = n[..] else {
        return Err(bad());
    };
    Ok(Some(CropRect { x, y, w, h }))
}

/// The raw file is the body, as for `upload_emoji_media`; the crop rides in `x-pulse-crop`.
async fn upload_profile_media(
    app: AppHandle,
    core: State<'_, Core>,
    request: tauri::ipc::Request<'_>,
    kind: ProfileMediaKind,
) -> Result<MediaInfo, ApiError> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(ApiError::Rejected("expected the file's bytes".into()));
    };
    let crop = parse_crop(
        request
            .headers()
            .get("x-pulse-crop")
            .map(|v| {
                v.to_str()
                    .map_err(|_| ApiError::Rejected("bad crop".into()))
            })
            .transpose()?,
    )?;
    let (api, token) = core.current()?;
    core.check(
        &app,
        api.upload_profile_media(&token, kind, bytes.clone(), crop)
            .await,
    )
}

#[tauri::command]
pub async fn upload_avatar_media(
    app: AppHandle,
    core: State<'_, Core>,
    request: tauri::ipc::Request<'_>,
) -> Result<MediaInfo, ApiError> {
    upload_profile_media(app, core, request, ProfileMediaKind::Avatar).await
}

#[tauri::command]
pub async fn upload_banner_media(
    app: AppHandle,
    core: State<'_, Core>,
    request: tauri::ipc::Request<'_>,
) -> Result<MediaInfo, ApiError> {
    upload_profile_media(app, core, request, ProfileMediaKind::Banner).await
}

#[tauri::command]
pub async fn create_emoji(
    app: AppHandle,
    core: State<'_, Core>,
    server_id: ServerId,
    name: String,
    media_hash: String,
) -> Result<CustomEmoji, ApiError> {
    let (api, token) = core.current()?;
    core.check(
        &app,
        api.create_emoji(&token, server_id, &name, &media_hash)
            .await,
    )
}

#[tauri::command]
pub async fn rename_emoji(
    app: AppHandle,
    core: State<'_, Core>,
    id: String,
    name: String,
) -> Result<CustomEmoji, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.rename_emoji(&token, &id, &name).await)
}

#[tauri::command]
pub async fn delete_emoji(
    app: AppHandle,
    core: State<'_, Core>,
    id: String,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.delete_emoji(&token, &id).await)
}

/// `emoji`: a Unicode emoji or `custom:<id>`.
#[tauri::command]
pub async fn add_reaction(
    app: AppHandle,
    core: State<'_, Core>,
    message_id: MessageId,
    emoji: String,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.add_reaction(&token, message_id, &emoji).await)
}

#[tauri::command]
pub async fn remove_reaction(
    app: AppHandle,
    core: State<'_, Core>,
    message_id: MessageId,
    emoji: String,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.remove_reaction(&token, message_id, &emoji).await)
}

#[tauri::command]
pub async fn list_reactors(
    app: AppHandle,
    core: State<'_, Core>,
    message_id: MessageId,
    emoji: String,
) -> Result<Vec<UserId>, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.reactors(&token, message_id, &emoji).await)
}

// ---------- Quotes ----------

/// The raw file is the request body, as for `upload_emoji_media`.
#[tauri::command]
pub async fn upload_quote_media(
    app: AppHandle,
    core: State<'_, Core>,
    request: tauri::ipc::Request<'_>,
) -> Result<MediaInfo, ApiError> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(ApiError::Rejected("expected the file's bytes".into()));
    };
    let (api, token) = core.current()?;
    core.check(&app, api.upload_quote_media(&token, bytes.clone()).await)
}

/// The bytes of the image among files dropped on a quote line (Tauri gives the page only their
/// paths), for `upload_quote_media`: the first image by name, at most the size limit. None of them
/// an image says so in the server's words.
#[tauri::command]
pub async fn read_quote_image(paths: Vec<String>) -> Result<tauri::ipc::Response, ApiError> {
    let paths: Vec<std::path::PathBuf> = paths.into_iter().map(Into::into).collect();
    let bytes = tokio::task::spawn_blocking(move || {
        let path = crate::media::first_image(&paths)
            .ok_or_else(|| ApiError::Rejected(crate::media::NOT_AN_IMAGE.into()))?;
        crate::media::read_quote_image(path)
    })
    .await
    .map_err(|_| ApiError::Rejected(crate::media::UNREADABLE_IMAGE.into()))??;
    Ok(tauri::ipc::Response::new(bytes))
}

/// A reply in a quote's thread: queued in the outbox like `send_message` (retry, failed state),
/// carrying the quote it answers (and an inline reply-to inside that thread, if any). Confirmation
/// arrives as a `pulse://outbox` event whose message has `thread_root_id` set.
#[tauri::command]
pub async fn send_reply(
    core: State<'_, Core>,
    channel_id: ChannelId,
    root_id: MessageId,
    content: String,
    reply_to_id: Option<MessageId>,
    nonce: String,
) -> Result<(), ApiError> {
    core.outbox
        .enqueue(nonce, channel_id, content, reply_to_id, Some(root_id));
    if let Ok((api, token)) = core.current() {
        core.outbox.flush(&api, &token, false).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn list_messages_around(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    message_id: MessageId,
) -> Result<Option<Vec<Message>>, ApiError> {
    let (api, token) = core.current()?;
    core.check(
        &app,
        api.messages_around(&token, channel_id, message_id).await,
    )
}

#[tauri::command]
pub async fn list_messages_after(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    message_id: MessageId,
) -> Result<Vec<Message>, ApiError> {
    let (api, token) = core.current()?;
    core.check(
        &app,
        api.messages_after(&token, channel_id, message_id).await,
    )
}

#[tauri::command]
pub async fn post_quote(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    body: PostQuoteRequest,
) -> Result<Message, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.post_quote(&token, channel_id, &body).await)
}

#[tauri::command]
pub async fn edit_quote(
    app: AppHandle,
    core: State<'_, Core>,
    message_id: MessageId,
    body: EditQuoteRequest,
) -> Result<Message, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.edit_quote(&token, message_id, &body).await)
}

// ---------- Clips ----------

#[tauri::command]
pub async fn add_game(
    app: AppHandle,
    core: State<'_, Core>,
    server_id: ServerId,
    name: String,
) -> Result<Game, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.add_game(&token, server_id, &name).await)
}

#[tauri::command]
pub async fn edit_clip(
    app: AppHandle,
    core: State<'_, Core>,
    message_id: MessageId,
    body: EditClipRequest,
) -> Result<Message, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.edit_clip(&token, message_id, &body).await)
}

#[tauri::command]
pub async fn clip_url(
    app: AppHandle,
    core: State<'_, Core>,
    message_id: MessageId,
) -> Result<ClipUrl, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.clip_url(&token, message_id).await)
}

/// The clip player's full screen: the window is frameless, so the page's element fullscreen alone
/// only fills the window. The UI calls this on every `fullscreenchange`. Returns whether the window
/// was fullscreen before, so the UI can put it back as it found it.
#[tauri::command]
pub fn set_fullscreen(app: AppHandle, on: bool) -> Result<bool, String> {
    let win = app
        .get_webview_window("main")
        .ok_or_else(|| "no main window".to_owned())?;
    let was = win.is_fullscreen().map_err(|e| e.to_string())?;
    win.set_fullscreen(on).map_err(|e| e.to_string())?;
    Ok(was)
}

#[tauri::command]
pub async fn list_clips(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    before: Option<MessageId>,
    game: Option<GameId>,
    author: Option<UserId>,
) -> Result<Vec<Message>, ApiError> {
    let (api, token) = core.current()?;
    core.check(
        &app,
        api.list_clips(&token, channel_id, before, game, author)
            .await,
    )
}

#[tauri::command]
pub async fn clip_facets(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
) -> Result<ClipFacets, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.clip_facets(&token, channel_id).await)
}

#[tauri::command]
pub async fn set_clip_view(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    view: ClipView,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.set_clip_view(&token, channel_id, view).await)
}

#[tauri::command]
pub async fn clip_storage(
    app: AppHandle,
    core: State<'_, Core>,
    server_id: ServerId,
) -> Result<ClipStorage, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.clip_storage(&token, server_id).await)
}

/// Whether this install can send clips (ffmpeg found), and the reason shown when it can't.
#[tauri::command]
pub async fn clip_support(app: AppHandle) -> crate::clips::ffmpeg::ClipSupport {
    use crate::clips::ffmpeg;
    let resource_dir = app.path().resource_dir().ok();
    let found = ffmpeg::locate(
        resource_dir.as_deref(),
        ffmpeg::dev_override().as_deref(),
        ffmpeg::Os::current(),
    );
    ffmpeg::clip_support(found.as_ref(), ffmpeg::Os::current())
}

/// ffmpeg and ffprobe, or why clips can't be edited here.
fn ffmpeg_paths(app: &AppHandle) -> Result<crate::clips::ffmpeg::FfmpegPaths, String> {
    use crate::clips::ffmpeg;
    let resource_dir = app.path().resource_dir().ok();
    let os = ffmpeg::Os::current();
    let found = ffmpeg::locate(
        resource_dir.as_deref(),
        ffmpeg::dev_override().as_deref(),
        os,
    );
    found.ok_or_else(|| {
        ffmpeg::clip_support(None, os)
            .reason
            .unwrap_or_else(|| ffmpeg::MISSING.to_owned())
    })
}

/// Let the webview load this one file through the asset protocol.
fn allow_asset(app: &AppHandle, path: &std::path::Path) -> Result<(), String> {
    app.asset_protocol_scope()
        .allow_file(path)
        .map_err(|e| e.to_string())
}

/// Look at a video file and open it as an editor source (allowing the file on the asset scope).
#[tauri::command]
pub async fn open_clip(
    app: AppHandle,
    core: State<'_, Core>,
    path: String,
) -> Result<crate::clips::editor::ClipSource, String> {
    use crate::clips::{editor, preview};
    let path = std::path::PathBuf::from(path);
    editor::validate_source_path(&path)?;
    let tools = ffmpeg_paths(&app)?;
    let scope = core
        .clips_scope()
        .ok_or_else(|| SIGN_IN_FOR_CLIPS.to_owned())?;
    let probe = editor::probe_file(&tools.ffprobe, &path).await?;
    let plan = preview::plan(&probe, &editor::extension(&path));
    let id = editor::new_id();
    let source = editor::describe(&id, &path, &probe, plan);
    allow_asset(&app, &path)?;
    core.clip_sources.insert(
        id.clone(),
        editor::Entry {
            dir: scope.join("sources").join(&id),
            path,
            probe,
            plan,
            preview: None,
            filmstrip: None,
        },
    );
    Ok(source)
}

/// The system file dialog, filtered to videos. `None` when cancelled.
#[tauri::command]
pub async fn pick_clip_file(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app
        .dialog()
        .file()
        .add_filter("Videos", &["mp4", "mov", "mkv", "webm", "m4v"]);
    if let Some(win) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&win);
    }
    dialog.pick_file(move |f| {
        let _ = tx.send(f);
    });
    let picked = rx.await.ok().flatten()?.into_path().ok()?;
    Some(picked.to_string_lossy().into_owned())
}

/// The file the editor plays: the source itself (Direct), or a remuxed or re-encoded copy made now
/// (`clip://preview` events `{ source, percent }` while it runs). Its path is on the asset scope.
#[tauri::command]
pub async fn prepare_preview(
    app: AppHandle,
    core: State<'_, Core>,
    source: String,
) -> Result<String, String> {
    use crate::clips::{editor, preview::PreviewPlan};
    // One job per source: a second request waits, then finds the first one's file.
    let work = core
        .clip_sources
        .lock_for(&source)
        .ok_or_else(|| editor::NO_PREVIEW.to_owned())?;
    let _one_at_a_time = work.lock().await;
    let entry = core
        .clip_sources
        .get(&source)
        .ok_or_else(|| editor::NO_PREVIEW.to_owned())?;
    if entry.plan == PreviewPlan::Direct {
        return Ok(entry.path.to_string_lossy().into_owned());
    }
    if let Some(done) = entry.preview.as_ref().filter(|p| p.is_file()) {
        return Ok(done.to_string_lossy().into_owned());
    }
    let tools = ffmpeg_paths(&app)?;
    let id = source.clone();
    let out = editor::make_preview(&tools, &entry, work.closed(), |percent| {
        let _ = app.emit(
            "clip://preview",
            serde_json::json!({ "source": id, "percent": percent }),
        );
    })
    .await;
    if core.clip_sources.get(&source).is_none() {
        // closed or signed out while (or just as) ffmpeg ran: nothing owns these files now, and
        // ffmpeg has let go of them
        editor::remove_dir_later(entry.dir);
        return Err(editor::NO_PREVIEW.to_owned());
    }
    let out = out?;
    allow_asset(&app, &out)?;
    core.clip_sources
        .update(&source, |e| e.preview = Some(out.clone()));
    Ok(out.to_string_lossy().into_owned())
}

/// The scrubber's sprite: 24 keyframe tiles in one JPEG, on the asset scope.
#[tauri::command]
pub async fn clip_filmstrip(
    app: AppHandle,
    core: State<'_, Core>,
    source: String,
) -> Result<crate::clips::editor::Filmstrip, String> {
    use crate::clips::{editor, preview::FILMSTRIP_TILES};
    let work = core
        .clip_sources
        .lock_for(&source)
        .ok_or_else(|| editor::NO_FILMSTRIP.to_owned())?;
    let _one_at_a_time = work.lock().await;
    let entry = core
        .clip_sources
        .get(&source)
        .ok_or_else(|| editor::NO_FILMSTRIP.to_owned())?;
    let path = match entry.filmstrip.clone().filter(|p| p.is_file()) {
        Some(done) => done,
        None => {
            let tools = ffmpeg_paths(&app)?;
            let out = editor::make_filmstrip(&tools.ffmpeg, &entry, work.closed()).await;
            if core.clip_sources.get(&source).is_none() {
                // closed while (or just as) ffmpeg ran: nothing owns these files now
                editor::remove_dir_later(entry.dir);
                return Err(editor::NO_FILMSTRIP.to_owned());
            }
            let out = out?;
            allow_asset(&app, &out)?;
            core.clip_sources
                .update(&source, |e| e.filmstrip = Some(out.clone()));
            out
        }
    };
    Ok(editor::Filmstrip {
        path: path.to_string_lossy().into_owned(),
        tiles: FILMSTRIP_TILES,
    })
}

/// The editor is done with this source: its preview and filmstrip files are deleted.
#[tauri::command]
pub fn close_clip_source(core: State<'_, Core>, source: String) {
    if let Some(e) = core.clip_sources.remove(&source) {
        core.release_source(e);
    }
}

/// What the editor's Trim & send asks for (`spans` are in source milliseconds).
#[derive(serde::Deserialize)]
pub struct SendClip {
    source: String,
    channel_id: ChannelId,
    spans: Vec<crate::clips::encode::Span>,
    preset: crate::clips::encode::Preset,
    caption: String,
    game_id: Option<GameId>,
}

/// Start sending a clip of an open editor source (its path is copied, so closing the editor is fine).
/// Progress arrives as `clip://job` events (the whole job).
#[tauri::command]
pub async fn send_clip(
    core: State<'_, Core>,
    request: SendClip,
) -> Result<crate::clips::jobs::ClipJob, String> {
    let source = core
        .clip_sources
        .get(&request.source)
        .ok_or_else(|| crate::clips::editor::UNREADABLE.to_owned())?;
    let jobs = core.clip_jobs().await?;
    jobs.send(crate::clips::jobs::NewJob {
        source_path: source.path,
        probe: source.probe,
        channel_id: request.channel_id,
        spans: request.spans,
        preset: request.preset,
        caption: request.caption,
        game_id: request.game_id,
    })
}

#[tauri::command]
pub async fn retry_clip(
    core: State<'_, Core>,
    id: String,
) -> Result<crate::clips::jobs::ClipJob, String> {
    core.clip_jobs().await?.retry(&id)
}

#[tauri::command]
pub async fn cancel_clip(core: State<'_, Core>, id: String) -> Result<(), String> {
    core.cancel_clip_job(&id, CLIP_JOBS_WAIT).await
}

/// The clips being sent (after a restart, the ones left over, failed). Waits for them to load.
#[tauri::command]
pub async fn clip_jobs(core: State<'_, Core>) -> Result<Vec<crate::clips::jobs::ClipJob>, String> {
    Ok(core.clip_jobs().await.map(|j| j.list()).unwrap_or_default())
}

#[tauri::command]
pub async fn list_replies(
    app: AppHandle,
    core: State<'_, Core>,
    message_id: MessageId,
    before: Option<MessageId>,
) -> Result<Vec<Message>, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.replies(&token, message_id, before).await)
}

#[tauri::command]
pub async fn mark_replies_seen(
    app: AppHandle,
    core: State<'_, Core>,
    root_id: MessageId,
    message_id: MessageId,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(
        &app,
        api.mark_replies_seen(&token, root_id, message_id).await,
    )
}

#[tauri::command]
pub async fn give_award(
    app: AppHandle,
    core: State<'_, Core>,
    line_id: LineId,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.give_award(&token, line_id).await)
}

#[tauri::command]
pub async fn take_award(
    app: AppHandle,
    core: State<'_, Core>,
    line_id: LineId,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.take_award(&token, line_id).await)
}

#[tauri::command]
pub async fn link_speaker(
    app: AppHandle,
    core: State<'_, Core>,
    server_id: ServerId,
    body: LinkSpeakerRequest,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.link_speaker(&token, server_id, &body).await)
}

#[tauri::command]
pub async fn unlink_speaker(
    app: AppHandle,
    core: State<'_, Core>,
    server_id: ServerId,
    name_key: String,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.unlink_speaker(&token, server_id, &name_key).await)
}

#[tauri::command]
pub async fn leaderboard(
    app: AppHandle,
    core: State<'_, Core>,
    server_id: ServerId,
    year: Option<i32>,
) -> Result<Leaderboard, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.leaderboard(&token, server_id, year).await)
}

#[tauri::command]
pub async fn search_quotes(
    app: AppHandle,
    core: State<'_, Core>,
    server_id: ServerId,
    q: String,
) -> Result<Vec<Message>, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.search_quotes(&token, server_id, &q).await)
}

#[tauri::command]
pub async fn start_qoty(
    app: AppHandle,
    core: State<'_, Core>,
    server_id: ServerId,
    body: StartQotyRequest,
) -> Result<QotyEvent, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.start_qoty(&token, server_id, &body).await)
}

#[tauri::command]
pub async fn vote_qoty(
    app: AppHandle,
    core: State<'_, Core>,
    event_id: QotyEventId,
    message_id: MessageId,
) -> Result<QotyEvent, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.vote_qoty(&token, event_id, message_id).await)
}

#[tauri::command]
pub async fn unvote_qoty(
    app: AppHandle,
    core: State<'_, Core>,
    event_id: QotyEventId,
) -> Result<QotyEvent, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.unvote_qoty(&token, event_id).await)
}

#[tauri::command]
pub async fn add_qoty_option(
    app: AppHandle,
    core: State<'_, Core>,
    event_id: QotyEventId,
    message_id: MessageId,
) -> Result<QotyEvent, ApiError> {
    let (api, token) = core.current()?;
    core.check(
        &app,
        api.add_qoty_option(&token, event_id, message_id).await,
    )
}

#[tauri::command]
pub async fn end_qoty(
    app: AppHandle,
    core: State<'_, Core>,
    event_id: QotyEventId,
) -> Result<QotyEvent, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.end_qoty(&token, event_id).await)
}

#[tauri::command]
pub async fn reopen_qoty(
    app: AppHandle,
    core: State<'_, Core>,
    event_id: QotyEventId,
    minutes: Option<u32>,
) -> Result<QotyEvent, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.reopen_qoty(&token, event_id, minutes).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_header_parses() {
        assert_eq!(
            parse_crop(Some("1,2,3,4")).unwrap(),
            Some(CropRect {
                x: 1,
                y: 2,
                w: 3,
                h: 4
            })
        );
        assert_eq!(parse_crop(None).unwrap(), None);
        assert!(
            matches!(parse_crop(Some("1,2,x,4")), Err(ApiError::Rejected(m)) if m == "bad crop")
        );
        assert!(parse_crop(Some("1,2,3")).is_err());
        assert!(parse_crop(Some("1,2,3,4,5")).is_err());
    }

    #[test]
    fn a_pending_login_is_replaced_by_the_next_and_cleared_on_demand() {
        let p = PendingMfa::default();
        assert!(
            p.get().is_none(),
            "login_mfa with nothing pending is expired"
        );
        p.set(Api::new("https://a.example.com"), "t1".into());
        p.set(Api::new("https://b.example.com"), "t2".into());
        let (api, ticket) = p.get().unwrap();
        assert_eq!(
            (api.base(), ticket.as_str()),
            ("https://b.example.com", "t2")
        );
        assert!(
            p.get().is_some(),
            "reading it does not spend it: a wrong code retries"
        );
        p.clear();
        assert!(p.get().is_none());
    }

    #[test]
    fn a_pending_login_is_only_taken_by_its_own_ticket() {
        let p = PendingMfa::default();
        assert!(!p.take_if("t1"), "nothing pending");
        p.set(Api::new("https://a.example.com"), "t1".into());
        p.set(Api::new("https://a.example.com"), "t2".into());
        // A second step that finishes after a newer login started must not clear the newer one.
        assert!(!p.take_if("t1"));
        assert_eq!(p.get().unwrap().1, "t2");
        assert!(p.take_if("t2"));
        assert!(p.get().is_none());
    }

    #[test]
    fn going_back_from_the_code_step_drops_the_ticket() {
        let root = tempfile::tempdir().unwrap();
        let (core, _) = signed_in(root.path(), "https://pulse.example.com", None);
        core.mfa
            .set(Api::new("https://pulse.example.com"), "t1".into());
        core.cancel_login();
        assert!(core.mfa.get().is_none());
    }

    #[test]
    fn a_new_login_drops_the_old_ticket_even_with_a_bad_server_url() {
        let root = tempfile::tempdir().unwrap();
        let (core, _) = signed_in(root.path(), "https://pulse.example.com", None);
        core.mfa
            .set(Api::new("https://pulse.example.com"), "t1".into());
        assert!(core.start_login("not a url").is_err());
        assert!(core.mfa.get().is_none());
        core.mfa
            .set(Api::new("https://pulse.example.com"), "t1".into());
        let api = core.start_login("https://other.example.com").unwrap();
        assert_eq!(api.base(), "https://other.example.com");
        assert!(core.mfa.get().is_none());
    }

    #[test]
    fn login_results_tell_the_ui_apart() {
        let j = |r: LoginResult| serde_json::to_string(&r).unwrap();
        assert_eq!(j(LoginResult::MfaRequired), r#"{"status":"mfa_required"}"#);
        assert_eq!(j(LoginResult::Expired), r#"{"status":"expired"}"#);
    }

    #[test]
    fn the_secret_is_shown_in_groups_of_four() {
        assert_eq!(group_secret("ABCDEFGHIJKL"), "ABCD EFGH IJKL");
        assert_eq!(group_secret("ABCDEFGHIJ"), "ABCD EFGH IJ");
        assert_eq!(group_secret(""), "");
    }

    #[test]
    fn restore_results_tell_the_ui_apart() {
        let j = |r: Restored| serde_json::to_string(&r).unwrap();
        assert_eq!(j(Restored::LoggedOut), r#"{"status":"logged_out"}"#);
        assert_eq!(j(Restored::Refused), r#"{"status":"refused"}"#);
        let u = User {
            id: UserId::new(),
            username: "alex".into(),
            avatar_hash: None,
            avatar_still: None,
            style: Default::default(),
        };
        assert!(j(Restored::Ready(u)).starts_with(r#"{"status":"ready","user":{"#));
    }
    use pulse_server::testing;

    /// R6: after every Ready the client declares its current mute/deafen, whatever they are.
    #[test]
    fn after_ready_declares_the_current_flags() {
        let mut c = Controls::default();
        c.toggle_deafen(); // deafen implies mute
        let frames = after_ready_frames(c);
        assert_eq!(frames.len(), 1);
        let json = serde_json::to_value(&frames[0]).unwrap();
        let want = serde_json::to_value(ClientFrame::VoiceState {
            flags: pulse_protocol::gateway::VoiceFlags {
                muted: true,
                deafened: true,
            },
        })
        .unwrap();
        assert_eq!(json, want);
    }

    /// Cancel while signed in but before the jobs loaded says so; signed out it is a no-op.
    #[tokio::test]
    async fn cancelling_a_clip_before_connecting_says_so() {
        let root = tempfile::tempdir().unwrap();
        let (core, _) = signed_in(
            root.path(),
            "https://pulse.example.com",
            Some(UserId::new()),
        );
        let wait = std::time::Duration::from_millis(50);
        assert_eq!(
            core.cancel_clip_job("0123456789abcdef", wait).await,
            Err(NOT_CONNECTED.to_owned())
        );
        core.forget();
        assert_eq!(core.cancel_clip_job("0123456789abcdef", wait).await, Ok(()));
    }

    /// Signing out deletes the account's clip jobs and their tickets even when they were never
    /// loaded (no Ready yet, or a paused session).
    #[tokio::test]
    async fn sign_out_deletes_clip_jobs_that_were_never_loaded() {
        use pulse_protocol::rest::{ChannelKind, ClipUploadRequest};
        let app = testing::spawn().await;
        let (me, token) = testing::register(&app, "alex").await;
        let server = testing::create_server(&app, &token, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let channel = api
            .create_channel(&token, server.id, ChannelKind::Clips, "clips")
            .await
            .unwrap();
        let ticket = api
            .clip_upload(
                &token,
                channel.id,
                &ClipUploadRequest {
                    size: 10,
                    duration_ms: 1000,
                },
            )
            .await
            .unwrap();

        let root = tempfile::tempdir().unwrap();
        let core = Core::new(
            Store::new(root.path().join("session"), Some("unit-test")),
            crate::outbox::Outbox::new(std::sync::Arc::new(|_: &str, _, _: Option<&Message>| {})),
            root.path().join("media"),
            Box::new(|| {}),
        )
        .with_clips(root.path().join("clips"), Box::new(|_| {}));
        core.begin(api.clone(), token.clone(), Some(me));
        let jobs =
            crate::media::scope_dir(&root.path().join("clips"), api.base(), &me).join("jobs");
        let job = jobs.join("0123456789abcdef");
        std::fs::create_dir_all(&job).unwrap();
        let saved = serde_json::json!({
            "source_path": "/v/a.mp4",
            "probe": { "duration_ms": 5000, "width": 1280, "height": 720, "fps": 60.0,
                       "video_codec": "h264", "audio": [], "format_name": "mp4",
                       "title": null, "size": 1 },
            "spans": [{ "start_ms": 0, "end_ms": 1000 }],
            "preset": "light", "caption": "", "game_id": null, "nonce": "n",
            "channel_id": channel.id, "stage": "failed", "upload_id": ticket.id,
            "uploaded": false, "thumb_hash": null, "encoder": null,
        });
        std::fs::write(job.join("job.json"), saved.to_string()).unwrap();
        std::fs::write(job.join("out.mp4"), b"x").unwrap();

        core.forget();
        for _ in 0..500 {
            if !jobs.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(!jobs.exists(), "the jobs folder goes");
        assert!(
            api.cancel_clip_upload(&token, ticket.id).await.is_err(),
            "its ticket was deleted"
        );

        // a session paused at startup (its server didn't answer), then signed out
        std::fs::create_dir_all(&job).unwrap();
        std::fs::write(job.join("job.json"), saved.to_string()).unwrap();
        core.begin(api.clone(), token.clone(), Some(me));
        core.suspend();
        core.forget();
        for _ in 0..500 {
            if !jobs.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(!jobs.exists(), "the jobs folder goes after a pause too");
    }

    fn no_hooks() -> crate::clips::jobs::Hooks {
        crate::clips::jobs::Hooks {
            emit: Box::new(|_| {}),
            allow_file: Box::new(|_| {}),
            forbid_file: Box::new(|_| {}),
        }
    }

    async fn load_jobs(dir: &std::path::Path, token: &str) -> ClipJobs {
        use crate::clips::{encode::FfmpegRunner, ffmpeg::FfmpegPaths};
        let paths = FfmpegPaths {
            ffmpeg: Default::default(),
            ffprobe: Default::default(),
        };
        ClipJobs::load(
            dir.to_path_buf(),
            FfmpegRunner(paths),
            Api::new("https://pulse.example.com"),
            token.to_owned(),
            no_hooks(),
        )
        .await
    }

    /// Jobs loaded for a session that signed out before they were installed stay out (the
    /// install is checked under the lock sign-out takes first); a live session's go in, once.
    #[tokio::test]
    async fn clip_jobs_of_a_signed_out_session_are_never_installed() {
        let root = tempfile::tempdir().unwrap();
        let me = UserId::new();
        let (core, _) = signed_in(root.path(), "https://pulse.example.com", Some(me));
        let dir = root.path().join("jobs");
        let loaded = load_jobs(&dir, "t1").await;
        core.forget(); // signed out between the load and the install
        assert!(core.install_clip_jobs("t1", loaded).is_none());
        assert!(core.clip_jobs.borrow().is_none());

        core.begin(Api::new("https://pulse.example.com"), "t2".into(), Some(me));
        let loaded = load_jobs(&dir, "t2").await;
        assert_eq!(core.install_clip_jobs("t2", loaded), Some(vec![]));
        let again = load_jobs(&dir, "t2").await;
        assert!(
            core.install_clip_jobs("t2", again).is_none(),
            "already loaded"
        );
        assert!(core.clip_jobs.borrow().is_some());
        // an older session's load never replaces the current one's
        assert!(
            core.install_clip_jobs("t1", load_jobs(&dir, "t1").await)
                .is_none()
        );
    }

    /// A pause (startup's server didn't answer) stops the session's installed jobs, and a load
    /// that lands afterwards stays out, as after a sign-out.
    #[tokio::test]
    async fn clip_jobs_of_a_paused_session_are_stopped_and_never_installed() {
        let root = tempfile::tempdir().unwrap();
        let (core, _) = signed_in(
            root.path(),
            "https://pulse.example.com",
            Some(UserId::new()),
        );
        let dir = root.path().join("jobs");
        assert!(
            core.install_clip_jobs("t1", load_jobs(&dir, "t1").await)
                .is_some()
        );
        let late = load_jobs(&dir, "t1").await;
        core.suspend();
        assert!(core.clip_jobs.borrow().is_none());
        assert!(core.install_clip_jobs("t1", late).is_none());
        assert!(core.clip_jobs.borrow().is_none());
    }

    /// Quitting stops clip work: the jobs go, every editor source closes, and it returns once
    /// the editor's work in flight has let go.
    #[tokio::test]
    async fn quitting_stops_clip_work() {
        let root = tempfile::tempdir().unwrap();
        let me = UserId::new();
        let (core, _) = signed_in(root.path(), "https://pulse.example.com", Some(me));
        assert!(
            core.install_clip_jobs("t1", load_jobs(&root.path().join("jobs"), "t1").await)
                .is_some()
        );
        core.clip_sources.insert(
            "a".into(),
            crate::clips::editor::Entry {
                path: "/v/a.mp4".into(),
                dir: root.path().join("sources/a"),
                probe: crate::clips::probe::Probe {
                    duration_ms: 1000,
                    width: 1,
                    height: 1,
                    fps: 1.0,
                    video_codec: "h264".into(),
                    audio: vec![],
                    format_name: String::new(),
                    title: None,
                    size: 1,
                },
                plan: crate::clips::preview::PreviewPlan::Encode,
                preview: None,
                filmstrip: None,
            },
        );
        // a preview in flight: it holds the source's lock until its source closes
        let work = core.clip_sources.lock_for("a").unwrap();
        let (held_tx, held_rx) = tokio::sync::oneshot::channel();
        let let_go = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = let_go.clone();
        tokio::spawn(async move {
            let _one_at_a_time = work.lock().await;
            let _ = held_tx.send(());
            work.closed().await;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await; // ffmpeg being dropped
            flag.store(true, Ordering::SeqCst);
        });
        held_rx.await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), core.stop_clips())
            .await
            .expect("quitting doesn't hang");
        assert!(
            let_go.load(Ordering::SeqCst),
            "returned only after the preview let go"
        );
        assert!(core.clip_jobs.borrow().is_none());
        assert!(core.clip_sources.ids().is_empty());
    }

    /// A Core with a sign-out hook that counts, signed in as `me` on `server`.
    fn signed_in(
        root: &std::path::Path,
        server: &str,
        me: Option<UserId>,
    ) -> (Core, Arc<std::sync::atomic::AtomicUsize>) {
        let cleared = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = cleared.clone();
        let core = Core::new(
            Store::new(root.join("session"), Some("unit-test")),
            crate::outbox::Outbox::new(std::sync::Arc::new(|_: &str, _, _: Option<&Message>| {})),
            root.join("media"),
            Box::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
            }),
        );
        core.begin(Api::new(server), "t1".into(), me);
        // the run's first sign-in clears too (nothing proves the webview's cache is this
        // account's); the tests count from here
        assert_eq!(cleared.swap(0, Ordering::SeqCst), 1);
        (core, cleared)
    }

    /// A stored server address the login check would refuse (a hand-edited store, or one saved
    /// by an older version) is not restored: the stored session is cleared, as for a dead token.
    #[test]
    fn a_stored_server_address_is_rechecked_before_restoring() {
        let root = tempfile::tempdir().unwrap();
        let (core, _) = signed_in(root.path(), "https://pulse.example.com", None);
        let dir = root.path().join("session");
        for bad in [
            "http://pulse.example.com",
            "https://user:pw@pulse.example.com",
            "https://pulse.example.com/?x=1",
            "file:///etc/passwd",
            "not a url",
        ] {
            crate::session::file::save(&dir, bad, "tok").unwrap();
            assert!(core.load_stored().is_none(), "{bad} was restored");
            assert!(
                !dir.join(crate::session::file::NAME).exists(),
                "{bad}: the stored session is cleared"
            );
        }
        crate::session::file::save(&dir, "https://pulse.example.com", "tok").unwrap();
        let (api, token) = core.load_stored().expect("a good address restores");
        assert_eq!(
            (api.base(), token.as_str()),
            ("https://pulse.example.com", "tok")
        );
        assert!(dir.join(crate::session::file::NAME).exists());
    }

    #[test]
    fn a_stored_login_refused_at_startup_is_signed_out() {
        let root = tempfile::tempdir().unwrap();
        let (core, cleared) = signed_in(root.path(), "https://pulse.example.com", None);
        core.give_up_restoring(true);
        assert_eq!(cleared.load(Ordering::SeqCst), 1);
        assert!(core.media_session().is_none());
    }

    /// Startup couldn't reach the stored session's server (it is paused, kept for the next
    /// launch), and the user signs out while waiting: the stored session goes too.
    #[test]
    fn signing_out_of_a_paused_session_forgets_the_stored_session() {
        let root = tempfile::tempdir().unwrap();
        let server = "https://pulse.example.com";
        let (core, _) = signed_in(root.path(), server, Some(UserId::new()));
        let dir = root.path().join("session");
        crate::session::file::save(&dir, server, "t1").unwrap();
        core.give_up_restoring(false);
        assert!(core.media_session().is_none(), "paused: no live session");
        assert!(
            core.store.load().is_some(),
            "still stored for the next launch"
        );
        core.forget_stored();
        assert!(core.store.load().is_none(), "the stored session is gone");
        assert!(!dir.join(crate::session::file::NAME).exists());
    }

    #[test]
    fn signing_in_as_someone_else_after_a_failed_restore_clears_first() {
        let root = tempfile::tempdir().unwrap();
        let server = "https://pulse.example.com";
        let (core, cleared) = signed_in(root.path(), server, None);
        // the server didn't answer: nobody signed out, nothing cleared
        core.give_up_restoring(false);
        assert_eq!(cleared.load(Ordering::SeqCst), 0);
        // whoever logs in next may not be who that was
        core.begin(Api::new(server), "t2".into(), Some(UserId::new()));
        assert_eq!(cleared.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn only_a_different_account_or_server_clears_on_sign_in() {
        let root = tempfile::tempdir().unwrap();
        let server = "https://pulse.example.com";
        let alex = UserId::new();
        let (core, cleared) = signed_in(root.path(), server, Some(alex));
        let count = || cleared.load(Ordering::SeqCst);
        // the webview reloaded: same session again
        core.begin(Api::new(server), "t1".into(), Some(alex));
        assert_eq!(count(), 0);
        core.begin(
            Api::new("https://other.example.com"),
            "t3".into(),
            Some(alex),
        );
        assert_eq!(count(), 1);
        core.begin(
            Api::new("https://other.example.com"),
            "t4".into(),
            Some(UserId::new()),
        );
        assert_eq!(count(), 2);
    }

    #[test]
    fn a_reload_keeps_the_session_so_a_sign_out_still_ends_its_fetches() {
        let root = tempfile::tempdir().unwrap();
        let server = "https://pulse.example.com";
        let alex = UserId::new();
        let (core, _) = signed_in(root.path(), server, Some(alex));
        let before_reload = core.media_session().unwrap();
        core.begin(Api::new(server), "t1".into(), Some(alex));
        assert!(before_reload.alive.load(Ordering::SeqCst));
        core.forget();
        assert!(!before_reload.alive.load(Ordering::SeqCst));
    }

    #[test]
    fn signing_out_ends_the_media_session_it_handed_out() {
        let root = tempfile::tempdir().unwrap();
        let (core, _) = signed_in(
            root.path(),
            "https://pulse.example.com",
            Some(UserId::new()),
        );
        let media = core.media_session().unwrap();
        assert!(media.alive.load(Ordering::SeqCst));
        core.forget();
        assert!(!media.alive.load(Ordering::SeqCst));
    }

    #[test]
    fn signing_out_clears_the_webview_and_this_accounts_media_cache() {
        let root = tempfile::tempdir().unwrap();
        let server = "https://pulse.example.com";
        let alex = UserId::new();
        let sam = UserId::new();
        let (core, cleared) = signed_in(root.path(), server, Some(alex));
        let scope = core.media_session().unwrap().cache.unwrap();
        assert_eq!(
            scope,
            crate::media::scope_dir(&root.path().join("media"), server, &alex)
        );
        let other = crate::media::scope_dir(&root.path().join("media"), server, &sam);
        for dir in [&scope, &other] {
            std::fs::create_dir_all(dir.join("ab")).unwrap();
            std::fs::write(dir.join("ab").join("x.webp"), b"x").unwrap();
        }

        assert_eq!(
            core.forget().map(|a| a.base().to_string()).as_deref(),
            Some(server)
        );
        assert_eq!(cleared.load(Ordering::SeqCst), 1);
        // removed off the async workers
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while scope.exists() && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!scope.exists(), "this account's cache is deleted");
        assert!(other.exists(), "another account's is not");
        assert!(core.media_session().is_none());

        // already signed out (a second 401): the webview is cleared again, nothing else to do
        assert!(core.forget().is_none());
        assert_eq!(cleared.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn media_is_cached_only_once_we_know_who_is_signed_in() {
        let root = tempfile::tempdir().unwrap();
        let (core, _) = signed_in(root.path(), "https://pulse.example.com", None);
        assert!(core.media_session().unwrap().cache.is_none());
        let alex = UserId::new();
        // a Ready from a session that has ended since names nobody
        core.learn_me("old token", alex);
        assert!(core.media_session().unwrap().cache.is_none());
        core.learn_me("t1", alex);
        assert!(core.media_session().unwrap().cache.is_some());
    }

    /// A server that answers every request with 503 (a proxy in front of a Pulse that's down).
    async fn always_503() -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("localhost:0").await.unwrap();
        let base = format!("http://localhost:{}", l.local_addr().unwrap().port());
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = s.read(&mut buf).await;
                    let _ = s
                        .write_all(b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\n\r\n")
                        .await;
                });
            }
        });
        base
    }

    #[tokio::test]
    async fn unrestored_with_a_refused_token_is_unauthorized() {
        let app = testing::spawn().await;
        let api = Api::new(&format!("http://{}", app.addr));
        let why = unrestored_reason(&api, "garbage").await;
        assert!(matches!(why, ApiError::Unauthorized), "{why:?}");
    }

    #[tokio::test]
    async fn unrestored_with_a_struggling_server_says_so() {
        let api = Api::new(&always_503().await);
        let why = unrestored_reason(&api, "t").await;
        assert!(
            matches!(&why, ApiError::Server(s) if s.starts_with("503")),
            "{why:?}"
        );
    }

    #[tokio::test]
    async fn unrestored_with_no_server_is_connect_failed() {
        let api = Api::new("http://localhost:9");
        let why = unrestored_reason(&api, "t").await;
        assert!(
            matches!(&why, ApiError::Network(t) if t == crate::api::CONNECT_FAILED),
            "{why:?}"
        );
    }

    /// `/me` works but the gateway never got to Ready: still "couldn't connect" (controller ruling).
    #[tokio::test]
    async fn unrestored_with_a_working_rest_api_is_connect_failed() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let why = unrestored_reason(&api, &token).await;
        assert!(
            matches!(&why, ApiError::Network(t) if t == crate::api::CONNECT_FAILED),
            "{why:?}"
        );
    }
}
