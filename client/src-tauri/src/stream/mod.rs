//! Screen share, client side: REST calls wrapped as Tauri commands, and small window helpers.

pub mod loopback;
pub mod plan;
pub mod procs;
pub mod summer;

use std::sync::Mutex;
use std::time::Duration;

use base64::Engine;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{StreamInfo, StreamRole, StreamSettings, VoiceTokenResponse};
use tauri::{AppHandle, LogicalSize, Manager, State};

use crate::api::{Api, ApiError};
use crate::commands::Core;

/// The picker needs room for its thumbnails: the window grows to at least this while it is open.
const PICKER_MIN_HEIGHT: f64 = 640.0;
/// One log line is capped here, so a runaway caller can't flood the file.
const LOG_LINE_MAX: usize = 4096;
/// The stream ticket can be asked for just before LiveKit's join webhook reaches the server,
/// which answers 403 until it has. Retries after the first try, and the pause between them.
const TICKET_RETRIES: usize = 3;
const TICKET_RETRY_EVERY: Duration = Duration::from_millis(500);

/// A stream ticket, retrying a 403 a few times (see `TICKET_RETRIES`) before giving up.
pub async fn ticket_with_retry(
    api: &Api,
    token: &str,
    channel: ChannelId,
    role: StreamRole,
) -> Result<VoiceTokenResponse, ApiError> {
    let mut retries = 0;
    loop {
        match api.stream_ticket(token, channel, role).await {
            Err(ApiError::Forbidden(_)) if retries < TICKET_RETRIES => {
                retries += 1;
                tokio::time::sleep(TICKET_RETRY_EVERY).await;
            }
            r => return r,
        }
    }
}

/// `line` cut to at most `max` bytes, on a char boundary.
fn capped(line: &str, max: usize) -> &str {
    if line.len() <= max {
        return line;
    }
    let mut end = max;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    &line[..end]
}

/// The window's size from before the picker grew it (logical px), while the picker is open.
#[derive(Default)]
pub struct PickerMemory(Mutex<Option<LogicalSize<f64>>>);

#[tauri::command]
pub async fn stream_ticket(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    role: StreamRole,
) -> Result<VoiceTokenResponse, ApiError> {
    let (api, token) = core.current()?;
    core.check(
        &app,
        ticket_with_retry(&api, &token, channel_id, role).await,
    )
}

#[tauri::command]
pub async fn stream_go_live(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    settings: StreamSettings,
) -> Result<StreamInfo, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.go_live(&token, channel_id, settings).await)
}

#[tauri::command]
pub async fn stream_stop(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.stop_live(&token, channel_id).await)
}

#[tauri::command]
pub async fn stream_watch(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    streamer: UserId,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.watch_stream(&token, channel_id, streamer).await)
}

#[tauri::command]
pub async fn stream_unwatch(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    streamer: UserId,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.unwatch_stream(&token, channel_id, streamer).await)
}

/// `webp_base64` is the plain base64 of the WebP bytes (no `data:` prefix).
#[tauri::command]
pub async fn stream_preview_put(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    webp_base64: String,
) -> Result<(), ApiError> {
    let webp = base64::engine::general_purpose::STANDARD
        .decode(webp_base64.trim())
        .map_err(|_| ApiError::Rejected("preview isn't valid base64".into()))?;
    let (api, token) = core.current()?;
    core.check(&app, api.put_preview(&token, channel_id, webp).await)
}

/// The preview as a `data:image/webp;base64,...` URL, or `None` when there isn't one.
#[tauri::command]
pub async fn stream_preview_get(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    user_id: UserId,
) -> Result<Option<String>, ApiError> {
    let (api, token) = core.current()?;
    let bytes = core.check(&app, api.get_preview(&token, channel_id, user_id).await)?;
    Ok(bytes.map(|b| {
        format!(
            "data:image/webp;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(b)
        )
    }))
}

#[tauri::command]
pub fn stream_log(line: String) {
    tracing::info!(target: "share", "share: {}", capped(&line, LOG_LINE_MAX));
}

/// While the picker is open the window is at least `PICKER_MIN_HEIGHT` tall; closing restores the
/// size it had. Done here so the page needs no window-resize capability.
#[tauri::command]
pub fn stream_picker_window(
    app: AppHandle,
    memory: State<'_, PickerMemory>,
    open: bool,
) -> Result<(), String> {
    let win = app.get_webview_window("main").ok_or("no main window")?;
    let mut saved = memory.0.lock().unwrap();
    if open {
        let scale = win.scale_factor().map_err(|e| e.to_string())?;
        let size: LogicalSize<f64> = win
            .inner_size()
            .map_err(|e| e.to_string())?
            .to_logical(scale);
        // Opening twice must not overwrite the remembered size with the grown one.
        saved.get_or_insert(size);
        if size.height < PICKER_MIN_HEIGHT {
            win.set_size(LogicalSize::new(size.width, PICKER_MIN_HEIGHT))
                .map_err(|e| e.to_string())?;
        }
    } else if let Some(size) = saved.take() {
        win.set_size(size).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_lines_are_capped_on_a_char_boundary() {
        assert_eq!(capped("short", 4096), "short");
        let line = "é".repeat(3000); // 6000 bytes, 2 each
        let c = capped(&line, 4096);
        assert_eq!(c.len(), 4096);
        let odd = format!("a{}", "é".repeat(10));
        assert_eq!(capped(&odd, 4), "aé"); // byte 4 is mid-char: back off to 3
    }
}
