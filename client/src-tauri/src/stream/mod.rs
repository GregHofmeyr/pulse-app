//! Screen share, client side: REST calls wrapped as Tauri commands, and small window helpers.

pub mod audio;
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
use crate::voice::VoiceManager;
use audio::{StreamAudio, StreamAudioError};

/// The picker needs room for its thumbnails: the window grows to at least this while it is open.
const PICKER_MIN_HEIGHT: f64 = 640.0;
/// One log line is capped here, so a runaway caller can't flood the file.
const LOG_LINE_MAX: usize = 4096;
/// The stream ticket can be asked for just before LiveKit's join webhook reaches the server,
/// which answers 403 until it has. Retries after the first try, and the pause between them.
const TICKET_RETRIES: usize = 3;
const TICKET_RETRY_EVERY: Duration = Duration::from_millis(500);

/// Re-announcing a live stream after a server restart (the next `Ready` lacks it): the server
/// may not know we are in voice yet (403) or still be finding its feet (5xx, 429). Retries
/// after the first try, and the pause between them.
const REANNOUNCE_RETRIES: usize = 3;
const REANNOUNCE_RETRY_EVERY: Duration = Duration::from_millis(1500);

/// `call`, tried again up to `retries` times, `every` apart, while it fails with an error
/// `retry_on` accepts.
async fn retrying<T, Fut>(
    retries: usize,
    every: Duration,
    retry_on: fn(&ApiError) -> bool,
    mut call: impl FnMut() -> Fut,
) -> Result<T, ApiError>
where
    Fut: Future<Output = Result<T, ApiError>>,
{
    let mut tried = 0;
    loop {
        match call().await {
            Err(e) if tried < retries && retry_on(&e) => {
                tried += 1;
                tokio::time::sleep(every).await;
            }
            r => return r,
        }
    }
}

/// A stream ticket, retrying a 403 a few times (see `TICKET_RETRIES`) before giving up.
pub async fn ticket_with_retry(
    api: &Api,
    token: &str,
    channel: ChannelId,
    role: StreamRole,
) -> Result<VoiceTokenResponse, ApiError> {
    retrying(
        TICKET_RETRIES,
        TICKET_RETRY_EVERY,
        |e| matches!(e, ApiError::Forbidden(_)),
        || api.stream_ticket(token, channel, role),
    )
    .await
}

/// Announces a stream we are still sending (see `REANNOUNCE_RETRIES`). Going live twice is
/// harmless: the server treats it as an update.
pub async fn reannounce_with_retry(
    api: &Api,
    token: &str,
    channel: ChannelId,
    settings: StreamSettings,
) -> Result<StreamInfo, ApiError> {
    retrying(
        REANNOUNCE_RETRIES,
        REANNOUNCE_RETRY_EVERY,
        |e| {
            matches!(
                e,
                ApiError::Forbidden(_) | ApiError::Server(_) | ApiError::RateLimited
            )
        },
        || api.go_live(token, channel, settings.clone()),
    )
    .await
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

/// After a server restart: announce our stream again, retrying while the server catches up.
#[tauri::command]
pub async fn stream_reannounce(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    settings: StreamSettings,
) -> Result<StreamInfo, ApiError> {
    let (api, token) = core.current()?;
    core.check(
        &app,
        reannounce_with_retry(&api, &token, channel_id, settings).await,
    )
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

/// `image_base64` is the plain base64 of the frame's WebP (or JPEG/PNG) bytes (no `data:` prefix).
#[tauri::command]
pub async fn stream_preview_put(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    image_base64: String,
) -> Result<(), ApiError> {
    let image = base64::engine::general_purpose::STANDARD
        .decode(image_base64.trim())
        .map_err(|_| ApiError::Rejected("preview isn't valid base64".into()))?;
    let (api, token) = core.current()?;
    core.check(&app, api.put_preview(&token, channel_id, image).await)
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

/// The voice call's mixer, looked up through the app whenever stream audio needs it.
fn app_mixer(app: &AppHandle) -> audio::MixerSource {
    let app = app.clone();
    std::sync::Arc::new(move || {
        let app = app.clone();
        Box::pin(async move { app.state::<VoiceManager>().mixer_input().await })
    })
}

/// Connects `#a` to the stream room (if it isn't) and publishes stream audio: `window_title`'s
/// app for a window share (none when no single app owns it), everything but Pulse otherwise;
/// `muted` from the first frame.
#[tauri::command]
pub async fn stream_audio_start(
    app: AppHandle,
    core: State<'_, Core>,
    audio: State<'_, StreamAudio>,
    channel_id: ChannelId,
    window_title: Option<String>,
    muted: bool,
) -> Result<(), StreamAudioError> {
    let (api, token) = core.current()?;
    let r = audio
        .ensure_connected(&api, &token, channel_id, app_mixer(&app))
        .await;
    audio_check(&app, &core, r)?;
    Ok(audio.start_publishing(window_title, muted).await?)
}

/// A dead login (401) while `#a` connects logs out, as it does for every other command.
fn audio_check(
    app: &AppHandle,
    core: &Core,
    r: Result<(), StreamAudioError>,
) -> Result<(), StreamAudioError> {
    if let Err(StreamAudioError::Api(ApiError::Unauthorized)) = &r {
        let _ = core.check::<()>(app, Err(ApiError::Unauthorized));
    }
    r
}

#[tauri::command]
pub async fn stream_audio_stop(audio: State<'_, StreamAudio>) -> Result<(), String> {
    audio.stop_publishing().await;
    Ok(())
}

/// Voice moved to `keep` (`None`: left voice): `#a` goes unless it is in that channel. Covers
/// voice ending without `leave_voice` (rejoin given up, a failed join).
#[tauri::command]
pub async fn stream_audio_leave(
    audio: State<'_, StreamAudio>,
    keep: Option<ChannelId>,
) -> Result<(), String> {
    audio.leave_unless_in(keep).await;
    Ok(())
}

#[tauri::command]
pub async fn stream_audio_mute(audio: State<'_, StreamAudio>, muted: bool) -> Result<(), String> {
    audio.set_muted(muted).await;
    Ok(())
}

/// Starts (`on`) or stops playing `streamer`'s stream audio.
#[tauri::command]
pub async fn stream_audio_watch(
    app: AppHandle,
    core: State<'_, Core>,
    audio: State<'_, StreamAudio>,
    channel_id: ChannelId,
    streamer: UserId,
    on: bool,
) -> Result<(), StreamAudioError> {
    if on {
        let (api, token) = core.current()?;
        let r = audio
            .ensure_connected(&api, &token, channel_id, app_mixer(&app))
            .await;
        audio_check(&app, &core, r)?;
    } else if audio.connected_channel().await != Some(channel_id) {
        return Ok(()); // nothing playing there
    }
    Ok(audio.watch(streamer, on).await?)
}

#[tauri::command]
pub async fn set_stream_volume(
    voice: State<'_, VoiceManager>,
    audio: State<'_, StreamAudio>,
    user_id: UserId,
    pct: u16,
) -> Result<(), String> {
    let mixer = voice.mixer_input().await;
    audio.set_volume(mixer.as_ref().map(|(m, _)| &**m), &user_id.to_string(), pct);
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
