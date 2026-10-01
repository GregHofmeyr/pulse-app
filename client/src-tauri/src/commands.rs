//! Tauri commands: the UI's only way to reach the server. Tokens stay in here.

use std::sync::Mutex;

use pulse_protocol::gateway::ClientFrame;
use pulse_protocol::ids::{ChannelId, MessageId, ServerId};
use pulse_protocol::rest::{Channel, Member, Message, SendMessageRequest, Server, User};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc;

use crate::api::{Api, ApiError, check_server_url};
use crate::gateway::{ConnState, GatewayHandle, GatewayUpdate};
use crate::session::Store;

pub struct Core {
    inner: Mutex<Option<(Api, String)>>,
    gateway: Mutex<Option<GatewayHandle>>,
    store: Store,
}

impl Core {
    pub fn new(store: Store) -> Self {
        Self {
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
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    content: String,
    reply_to_id: Option<MessageId>,
    nonce: Option<String>,
) -> Result<Message, ApiError> {
    let (api, token) = core.current()?;
    let body = SendMessageRequest {
        content,
        reply_to_id,
        nonce,
    };
    core.check(&app, api.send_message(&token, channel_id, &body).await)
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
