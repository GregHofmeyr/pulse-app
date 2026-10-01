//! Tauri commands: the UI's only way to reach the server. Tokens stay in here.

use std::sync::Mutex;

use pulse_protocol::rest::{Server, User};
use tauri::State;

use crate::api::{Api, ApiError};
use crate::session;

#[derive(Default)]
pub struct Core {
    inner: Mutex<Option<(Api, String)>>,
}

impl Core {
    fn current(&self) -> Result<(Api, String), ApiError> {
        self.inner
            .lock()
            .unwrap()
            .clone()
            .ok_or(ApiError::Unauthorized)
    }

    fn set(&self, api: Api, token: String) {
        session::save(api.base(), &token);
        *self.inner.lock().unwrap() = Some((api, token));
    }
}

#[tauri::command]
pub async fn login(
    core: State<'_, Core>,
    server_url: String,
    username: String,
    password: String,
) -> Result<User, ApiError> {
    let api = Api::new(&server_url);
    let s = api.login(&username, &password).await?;
    core.set(api, s.token);
    Ok(s.user)
}

#[tauri::command]
pub async fn register(
    core: State<'_, Core>,
    server_url: String,
    invite_code: String,
    username: String,
    password: String,
) -> Result<User, ApiError> {
    let api = Api::new(&server_url);
    let s = api.register(&invite_code, &username, &password).await?;
    core.set(api, s.token);
    Ok(s.user)
}

/// Resume the stored session. `Ok(None)` means "show the login screen".
#[tauri::command]
pub async fn restore_session(core: State<'_, Core>) -> Result<Option<User>, ApiError> {
    let Some((server, token)) = session::load() else {
        return Ok(None);
    };
    let api = Api::new(&server);
    match api.me(&token).await {
        Ok(user) => {
            *core.inner.lock().unwrap() = Some((api, token));
            Ok(Some(user))
        }
        Err(ApiError::Unauthorized) => {
            session::clear(&server);
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn logout(core: State<'_, Core>) -> Result<(), ApiError> {
    let current = core.inner.lock().unwrap().take();
    if let Some((api, token)) = current {
        let _ = api.logout(&token).await;
        session::clear(api.base());
    }
    Ok(())
}

#[tauri::command]
pub async fn list_servers(core: State<'_, Core>) -> Result<Vec<Server>, ApiError> {
    let (api, token) = core.current()?;
    api.servers(&token).await
}
