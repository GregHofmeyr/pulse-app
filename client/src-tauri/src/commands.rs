//! Tauri commands: the UI's only way to reach the server. Tokens stay in here.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use pulse_protocol::gateway::ClientFrame;
use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{
    Channel, CustomEmoji, MediaInfo, Member, Message, MuteTarget, Server, SetMuteRequest, User,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{mpsc, watch};

use crate::api::{Api, ApiError, check_server_url};
use crate::gateway::{Boot, ConnState, GatewayHandle, GatewayUpdate};
use crate::session::Store;
use crate::stream::audio::StreamAudio;
use crate::voice::devices::{AudioConfig, DeviceInfo};
use crate::voice::{AudioMode, VoiceError, VoiceManager, controls::Controls};

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
    signed_out: SignedOut,
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
            signed_out,
        }
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
            *self.last.lock().unwrap() = Some((s.api.base().to_string(), Some(me)));
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
        let (server, token) = self.store.load()?;
        self.activate(app, Api::new(&server), token, None, false);
        Some(server)
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
        if let Some(api) = self.forget() {
            self.store.clear(api.base());
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
        self.last.lock().unwrap().take();
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

#[tauri::command]
pub async fn login(
    app: AppHandle,
    core: State<'_, Core>,
    server_url: String,
    username: String,
    password: String,
) -> Result<User, ApiError> {
    check_server_url(&server_url)?;
    let api = Api::new(&server_url);
    let s = api.login(&username, &password).await?;
    core.activate(&app, api, s.token, Some(s.user.id), true);
    Ok(s.user)
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
    check_server_url(&server_url)?;
    let api = Api::new(&server_url);
    let s = api.register(&invite_code, &username, &password).await?;
    core.activate(&app, api, s.token, Some(s.user.id), true);
    Ok(s.user)
}

/// Longest startup waits for the first gateway session to decide (connect 10 s + Ready 10 s).
const BOOT_WAIT: std::time::Duration = std::time::Duration::from_secs(25);

/// Resume the stored session. `Ok(None)` means "show the login screen".
#[tauri::command]
pub async fn restore_session(
    app: AppHandle,
    core: State<'_, Core>,
    voice: State<'_, VoiceManager>,
) -> Result<Option<User>, ApiError> {
    crate::startup::mark("restore_begin");
    // Runs once per page load: a reload (renderer crash) can't leave the old page's meters counted.
    voice.reset_meters();
    // The UI listens before it calls this. Already attached = the webview reloaded.
    let reload = core.ui_attached.send_replace(true);
    let Some(server) = core.start_stored(&app, reload) else {
        return Ok(None);
    };
    let mut boot = core.boot.subscribe();
    let decided = tokio::time::timeout(BOOT_WAIT, boot.wait_for(|b| !b.is_waiting()))
        .await
        .ok()
        .and_then(|r| r.ok().map(|b| b.clone()));
    crate::startup::mark("restore_end");
    match decided {
        Some(Boot::Ready(user)) => Ok(Some(user)),
        Some(Boot::LoggedOut) => Ok(None), // forward() already cleared the stored session
        Some(Boot::UpdateRequired) => {
            // the update screen sits inside the signed-in shell: it needs the user (rare path)
            let (api, token) = core.current()?;
            api.me(&token)
                .await
                .map(Some)
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
                    Ok(None)
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
    if let Ok((api, token)) = core.current() {
        let _ = api.logout(&token).await;
    }
    core.deactivate(&app);
    Ok(())
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
    core.outbox.enqueue(nonce, channel_id, content, reply_to_id);
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

/// UI sound, played natively (see sounds.rs for why not in the webview).
#[tauri::command]
pub async fn play_sound(name: String, output: Option<String>) {
    crate::sounds::play(&name, output);
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

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn a_stored_login_refused_at_startup_is_signed_out() {
        let root = tempfile::tempdir().unwrap();
        let (core, cleared) = signed_in(root.path(), "https://pulse.example.com", None);
        core.give_up_restoring(true);
        assert_eq!(cleared.load(Ordering::SeqCst), 1);
        assert!(core.media_session().is_none());
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
