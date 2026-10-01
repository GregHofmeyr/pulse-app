//! Tauri commands: the UI's only way to reach the server. Tokens stay in here.

use std::sync::Mutex;

use pulse_protocol::gateway::ClientFrame;
use pulse_protocol::ids::{ChannelId, MessageId, ServerId};
use pulse_protocol::rest::{Channel, Member, Message, Server, User};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc;

use crate::api::{Api, ApiError, check_server_url};
use crate::gateway::{ConnState, GatewayHandle, GatewayUpdate};
use crate::session::Store;
use crate::voice::devices::{AudioConfig, DeviceInfo};
use crate::voice::{AudioMode, VoiceError, VoiceManager, controls::Controls};

pub struct Core {
    pub outbox: std::sync::Arc<crate::outbox::Outbox>,
    inner: Mutex<Option<(Api, String)>>,
    gateway: Mutex<Option<GatewayHandle>>,
    store: Store,
}

impl Core {
    pub fn new(store: Store, outbox: crate::outbox::Outbox) -> Self {
        Self {
            outbox: std::sync::Arc::new(outbox),
            inner: Mutex::new(None),
            gateway: Mutex::new(None),
            store,
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
        if persist {
            self.store.save(api.base(), &token);
        }
        *self.inner.lock().unwrap() = Some((api.clone(), token.clone()));
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = GatewayHandle::spawn(api.base().to_string(), token, tx);
        // Replacing an old handle drops (and stops) it.
        *self.gateway.lock().unwrap() = Some(handle);
        tauri::async_runtime::spawn(forward(app.clone(), rx));
    }

    /// Forget the session locally (logout, or the server said our token is dead).
    fn deactivate(&self) {
        self.gateway.lock().unwrap().take();
        if let Some((api, _)) = self.inner.lock().unwrap().take() {
            self.store.clear(api.base());
        }
    }

    /// A 401 outside login means the session died server-side: drop it and tell the UI.
    pub(crate) fn check<T>(&self, app: &AppHandle, r: Result<T, ApiError>) -> Result<T, ApiError> {
        if let Err(ApiError::Unauthorized) = &r {
            self.deactivate();
            let _ = app.emit("pulse://conn", ConnState::LoggedOut);
        }
        r
    }

    pub(crate) fn send_frame(&self, f: ClientFrame) {
        if let Some(g) = self.gateway.lock().unwrap().as_ref() {
            g.send(f);
        }
    }
}

/// Gateway updates → Tauri events for the UI.
async fn forward(app: AppHandle, mut rx: mpsc::UnboundedReceiver<GatewayUpdate>) {
    while let Some(u) = rx.recv().await {
        match u {
            GatewayUpdate::Ready(r) => {
                let _ = app.emit("pulse://ready", &*r);
            }
            GatewayUpdate::Event(e) => {
                let _ = app.emit("pulse://event", &e);
            }
            GatewayUpdate::Connection(state) => {
                if state == ConnState::LoggedOut {
                    app.state::<Core>().deactivate();
                }
                if state == ConnState::Connected {
                    // Back online: send anything queued while we were away.
                    let core = app.state::<Core>();
                    if let Ok((api, token)) = core.current() {
                        let outbox = core.outbox.clone();
                        tauri::async_runtime::spawn(
                            async move { outbox.flush(&api, &token).await },
                        );
                    }
                }
                let _ = app.emit("pulse://conn", state);
            }
        }
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

/// Resume the stored session. `Ok(None)` means "show the login screen".
#[tauri::command]
pub async fn restore_session(
    app: AppHandle,
    core: State<'_, Core>,
) -> Result<Option<User>, ApiError> {
    let Some((server, token)) = core.store.load() else {
        return Ok(None);
    };
    let api = Api::new(&server);
    match api.me(&token).await {
        Ok(user) => {
            core.activate(&app, api, token, false);
            Ok(Some(user))
        }
        Err(ApiError::Unauthorized) => {
            core.store.clear(&server);
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn logout(core: State<'_, Core>) -> Result<(), ApiError> {
    let current = core.inner.lock().unwrap().clone();
    if let Some((api, token)) = current {
        let _ = api.logout(&token).await;
    }
    core.deactivate();
    Ok(())
}

/// The UI calls this on window focus / `online`: skip the backoff wait.
#[tauri::command]
pub fn gateway_reconnect_now(core: State<'_, Core>) {
    if let Some(g) = core.gateway.lock().unwrap().as_ref() {
        g.reconnect_now();
    }
}

#[tauri::command]
pub async fn list_servers(app: AppHandle, core: State<'_, Core>) -> Result<Vec<Server>, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.servers(&token).await)
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
        core.outbox.flush(&api, &token).await;
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

#[tauri::command]
pub async fn join_voice(
    app: AppHandle,
    core: State<'_, Core>,
    voice: State<'_, VoiceManager>,
    channel_id: ChannelId,
    config: AudioConfig,
) -> Result<(), VoiceError> {
    let (api, token) = core.current()?;
    let r = voice
        .join(&api, &token, channel_id, config, AudioMode::Real)
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
    voice.set_audio_config(config).await
}

#[derive(serde::Serialize)]
pub struct AudioDevices {
    inputs: Vec<DeviceInfo>,
    outputs: Vec<DeviceInfo>,
}

#[tauri::command]
pub fn list_audio_devices() -> AudioDevices {
    AudioDevices {
        inputs: crate::voice::devices::list_inputs(),
        outputs: crate::voice::devices::list_outputs(),
    }
}

#[tauri::command]
pub fn start_mic_test(
    app: AppHandle,
    mic: State<'_, crate::voice::mictest::MicTest>,
    config: AudioConfig,
) -> Result<(), VoiceError> {
    let handle = app.clone();
    mic.start(
        &config,
        std::sync::Arc::new(move |e| {
            let _ = handle.emit("voice://event", &e);
        }),
    )
}

#[tauri::command]
pub fn stop_mic_test(mic: State<'_, crate::voice::mictest::MicTest>) {
    mic.stop();
}

/// Hotkey path (IPC on Linux, global shortcut on Windows): toggle + broadcast, same as the buttons.
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
