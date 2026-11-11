//! The composer's file commands (#57): queue picked, dropped and pasted files.

use std::time::{Duration, Instant};

use pulse_protocol::ids::{AttachmentId, ChannelId, MessageId};
use pulse_protocol::rest::AttachmentUrl;
use tauri::{AppHandle, Emitter, Manager, State};

use super::send::{Outgoing, Progress, SendError};
use super::{StageResult, Staged};
use crate::api::ApiError;
use crate::commands::Core;
use crate::outbox::Status;

/// At most four progress events a second per send.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// Queue files by path (picked or dropped). One result per path, in order: a folder or an
/// unreadable file says why instead.
#[tauri::command]
pub async fn stage_attachment_paths(
    core: State<'_, Core>,
    paths: Vec<String>,
) -> Result<Vec<StageResult>, String> {
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let path = std::path::PathBuf::from(&p);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or(p);
        // Converting an image takes a moment: block_in_place lets this worker's other tasks move
        // to another thread meanwhile.
        let stager = &core.attachments;
        let r = tokio::task::block_in_place(|| stager.stage_path(&path));
        out.push(match r {
            Ok(file) => StageResult::Staged { file },
            Err(error) => StageResult::Refused {
                folder: (error == super::FOLDER).then(|| path.to_string_lossy().into_owned()),
                name,
                error,
            },
        });
    }
    Ok(out)
}

/// Queue a pasted file: the request body is its bytes, the `x-name` header its name.
#[tauri::command]
pub async fn stage_attachment_bytes(
    core: State<'_, Core>,
    request: tauri::ipc::Request<'_>,
) -> Result<Staged, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected the file's bytes".into());
    };
    let name = request
        .headers()
        .get("x-name")
        .and_then(|v| v.to_str().ok())
        .map(percent_decode)
        .unwrap_or_else(|| "pasted".into());
    let stager = &core.attachments;
    tokio::task::block_in_place(|| stager.stage_bytes(&name, bytes))
}

/// `encodeURIComponent` undone (header values are ASCII; names needn't be).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(hex) = std::str::from_utf8(&b[i + 1..i + 3])
            && let Ok(v) = u8::from_str_radix(hex, 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// "Zip and send": zip a dropped or picked folder and queue the zip.
#[tauri::command]
pub async fn zip_attachment_folder(core: State<'_, Core>, path: String) -> Result<Staged, String> {
    let stager = &core.attachments;
    tokio::task::block_in_place(|| stager.stage_folder(std::path::Path::new(&path)))
}

/// Forget a queued file (its chip was removed).
#[tauri::command]
pub fn unstage_attachment(core: State<'_, Core>, id: String) {
    core.attachments.remove(&id);
}

/// The system file dialog, any files, several at once. Empty when cancelled.
#[tauri::command]
pub async fn pick_attachment_files(app: AppHandle) -> Vec<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app.dialog().file();
    if let Some(win) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&win);
    }
    dialog.pick_files(move |f| {
        let _ = tx.send(f);
    });
    rx.await
        .ok()
        .flatten()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|f| f.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// Send a message with queued files: uploads them (`attachments://progress` events), then sends
/// it. Returns at once; the outcome is a `pulse://outbox` event for `nonce`, like any send.
/// Calling it again with the same nonce after a failure uploads only what isn't stored yet.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn send_with_attachments(
    app: AppHandle,
    channel_id: ChannelId,
    content: String,
    reply_to_id: Option<MessageId>,
    thread_root_id: Option<MessageId>,
    nonce: String,
    staged: Vec<String>,
    temp_days: Option<u8>,
) -> Result<(), ApiError> {
    let out = Outgoing {
        channel: channel_id,
        content,
        reply_to: reply_to_id,
        thread_root: thread_root_id,
        nonce,
        staged,
        temp_days,
    };
    tauri::async_runtime::spawn(async move {
        let core = app.state::<Core>();
        let result = match core.current() {
            Ok((api, token)) => {
                let emitter = app.clone();
                let throttle = std::sync::Mutex::new(None::<Instant>);
                let on_progress = move |p: Progress| {
                    let mut last = throttle.lock().unwrap_or_else(|e| e.into_inner());
                    let due = last.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY);
                    if due || p.sent >= p.total {
                        *last = Some(Instant::now());
                        let _ = emitter.emit("attachments://progress", &p);
                    }
                };
                core.attachment_sends
                    .run(&api, &token, &core.attachments, &out, on_progress)
                    .await
            }
            Err(e) => Err(e.into()),
        };
        match result {
            Ok(m) => core.outbox.report(&out.nonce, Status::Sent, Some(&m)),
            Err(e) => {
                if let SendError::Api(ApiError::Unauthorized) = e {
                    let _ = core.check::<()>(&app, Err(ApiError::Unauthorized));
                }
                if !e.cancelled() {
                    tracing::warn!(error = %e, "a message with files didn't send");
                    let _ = app.emit(
                        "attachments://failed",
                        serde_json::json!({ "nonce": out.nonce, "error": e.to_string() }),
                    );
                }
                core.outbox.report(&out.nonce, Status::Failed, None);
            }
        }
    });
    Ok(())
}

/// Stop sending a message with files; what it stored is dropped on the server.
#[tauri::command]
pub async fn cancel_attachment_send(core: State<'_, Core>, nonce: String) -> Result<(), ApiError> {
    let stored = core.attachment_sends.cancel(&nonce);
    if let Ok((api, token)) = core.current() {
        for id in stored {
            // Best effort: the server sweeps whatever is left.
            let _ = api.cancel_attachment(&token, id).await;
        }
    }
    Ok(())
}

/// Short-lived links to a message's files (the UI caches them like clip links).
#[tauri::command]
pub async fn attachment_urls(
    app: AppHandle,
    core: State<'_, Core>,
    message_id: MessageId,
) -> Result<Vec<AttachmentUrl>, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.attachment_urls(&token, message_id).await)
}

/// Download one file of a message to where the system Save dialog says, suggesting its name.
/// `false` when the dialog is cancelled. Pulse never opens what it saves.
#[tauri::command]
pub async fn save_attachment(
    app: AppHandle,
    core: State<'_, Core>,
    message_id: MessageId,
    attachment_id: AttachmentId,
    name: String,
) -> Result<bool, String> {
    use crate::clips::download;
    let (api, token) = core.current().map_err(|e| e.to_string())?;
    let link = async || -> Result<String, String> {
        let urls = core
            .check(&app, api.attachment_urls(&token, message_id).await)
            .map_err(|e| e.to_string())?;
        urls.into_iter()
            .find(|u| u.id == attachment_id)
            .and_then(|u| u.url)
            .ok_or_else(|| GONE.to_string())
    };
    // asked first: a file that is gone (or expired) says so before any dialog
    let first = link().await?;
    let Some(dest) = ask_save_path(&app, safe_name(&name)).await else {
        return Ok(false);
    };
    let http = crate::clips::upload::client();
    match download::save_url(&http, &first, &dest).await {
        // the link lasts an hour; a dialog left open longer gets a fresh one
        Err(download::DownloadError::Rejected(403)) => {
            let fresh = link().await?;
            download::save_url(&http, &fresh, &dest).await
        }
        other => other,
    }
    .map_err(|e| e.to_string())?;
    Ok(true)
}

const GONE: &str = "That file isn't available any more.";

/// A name safe to suggest in a Save dialog: the server cleaned it already; this is the last line
/// (no path, nothing Windows refuses).
fn safe_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').trim_start_matches('.');
    if cleaned.is_empty() {
        "file".into()
    } else {
        cleaned.into()
    }
}

/// The system Save dialog over the main window, suggesting `name`; `None` when cancelled.
async fn ask_save_path(app: &AppHandle, name: String) -> Option<std::path::PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app.dialog().file().set_file_name(name);
    if let Some(win) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&win);
    }
    dialog.save_file(move |f| {
        let _ = tx.send(f);
    });
    rx.await.ok().flatten().and_then(|f| f.into_path().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_names_are_safe() {
        assert_eq!(safe_name("report.pdf"), "report.pdf");
        assert_eq!(safe_name("../../x.exe"), "x.exe");
        assert_eq!(safe_name("a:b?.txt"), "a_b_.txt");
        assert_eq!(safe_name("..."), "file");
        assert_eq!(safe_name("name. "), "name");
    }

    #[test]
    fn names_from_the_header_are_decoded() {
        assert_eq!(percent_decode("caf%C3%A9.png"), "café.png");
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("plain"), "plain");
        assert_eq!(percent_decode("100%"), "100%");
    }
}
