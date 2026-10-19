//! Tauri commands: the UI's only way to reach the server. Tokens stay in here.

use std::sync::Mutex;

use pulse_protocol::gateway::ClientFrame;
use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{Channel, Member, Message, MuteTarget, Server, SetMuteRequest, User};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{mpsc, watch};

use crate::api::{Api, ApiError, check_server_url};
use crate::gateway::{Boot, ConnState, GatewayHandle, GatewayUpdate};
use crate::session::Store;
use crate::voice::devices::{AudioConfig, DeviceInfo};
use crate::voice::{AudioMode, VoiceError, VoiceManager, controls::Controls};

pub struct Core {
    pub outbox: std::sync::Arc<crate::outbox::Outbox>,
    inner: Mutex<Option<(Api, String)>>,
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
}

impl Core {
    pub fn new(store: Store, outbox: crate::outbox::Outbox) -> Self {
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
        }
    }

    pub(crate) fn current(&self) -> Result<(Api, String), ApiError> {
        self.inner
            .lock()
            .unwrap()
            .clone()
            .ok_or(ApiError::Unauthorized)
    }

    /// Remember the session and (re)start the live connection.
    fn activate(&self, app: &AppHandle, api: Api, token: String, persist: bool) {
        self.boot.send_replace(Boot::starting());
        if persist {
            self.store.save(api.base(), &token);
        }
        *self.inner.lock().unwrap() = Some((api.clone(), token.clone()));
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
            token,
            tx,
            crate::gateway::Tuning::default(),
            declare,
        );
        // Replacing an old handle drops (and stops) it.
        *self.gateway.lock().unwrap() = Some(handle);
        tauri::async_runtime::spawn(forward(app.clone(), rx));
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

    /// Start the stored session's live connection, once (setup and restore_session both ask; the
    /// first wins). `restart`: the webview reloaded, so a running session starts a fresh gateway
    /// (the new page needs its own Ready). The stored server, or `None` when nobody is signed in.
    pub(crate) fn start_stored(&self, app: &AppHandle, restart: bool) -> Option<String> {
        let _one = self.starting.lock().unwrap();
        let running = self.inner.lock().unwrap().clone();
        if let Some((api, token)) = running {
            let server = api.base().to_string();
            if restart {
                self.activate(app, api, token, false);
            }
            return Some(server);
        }
        let (server, token) = self.store.load()?;
        self.activate(app, Api::new(&server), token, false);
        Some(server)
    }

    /// The stored session's server didn't answer at startup: stop trying, but keep it stored
    /// (the next launch tries again, as before).
    fn suspend(&self) {
        self.gateway.lock().unwrap().take();
        if let Some(t) = self.retry.lock().unwrap().take() {
            t.abort();
        }
        self.inner.lock().unwrap().take();
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
        if let Some((api, _)) = self.inner.lock().unwrap().take() {
            self.store.clear(api.base());
        }
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
async fn forward(app: AppHandle, mut rx: mpsc::UnboundedReceiver<GatewayUpdate>) {
    let mut attached = app.state::<Core>().ui_listening();
    let _ = attached.wait_for(|a| *a).await;
    while let Some(u) = rx.recv().await {
        match &u {
            GatewayUpdate::Ready(r) => {
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
    core.activate(&app, api, s.token, true);
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
    core.activate(&app, api, s.token, true);
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
            core.suspend();
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
    let current = core.inner.lock().unwrap().clone();
    if let Some((api, token)) = current {
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
    channel_id: ChannelId,
    config: AudioConfig,
) -> Result<(), VoiceError> {
    let (api, token) = core.current()?;
    tracing::info!(%channel_id, input = ?config.input, output = ?config.output, gain = config.input_gain_pct, "join_voice");
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
pub async fn leave_voice(voice: State<'_, VoiceManager>) -> Result<(), VoiceError> {
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
