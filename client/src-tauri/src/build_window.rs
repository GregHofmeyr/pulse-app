//! The build pop-out (#620, spec §4.4): one build in a small window of its own, always on top by default.
//!
//! It is a second window on the app's own page (`index.html?window=build`), so it could reach every app command
//! but for the IPC gate (ipc_gate.rs): it may call only [`build_window_snapshot`], [`build_window_image_urls`] and
//! [`set_build_window_on_top`]. Everything it shows is handed over here by the main window, for one build: the
//! snapshot the main window opened it with (or a newer one of the same build), and fresh links to that build's
//! screenshots, which the core fetches for the stored build only. It gets no capability (no events), no session
//! token and nothing in its address but `window=build`; it may only load the app's own page.

use std::sync::Mutex;
use std::time::Duration;

use pulse_protocol::ids::{AttachmentId, MessageId};
use pulse_protocol::rest::{AttachmentUrl, BuildLink};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::api::ApiError;
use crate::commands::Core;
use crate::embed_window::WebviewData;
use crate::ipc_gate::BUILD_LABEL;

/// A screenshot as the pop-out lays it out (its link comes from [`build_window_image_urls`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotShot {
    pub id: AttachmentId,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// What the pop-out shows of a build: title, media and notes, never its comments. Made by the main window from the
/// build it has (`snapshotOf` in the page).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildSnapshot {
    pub message_id: MessageId,
    pub title: String,
    pub game_name: Option<String>,
    pub game_guessed: bool,
    pub author_name: String,
    pub saved_at: String,
    pub updated_at: Option<String>,
    pub notes: Option<String>,
    pub link: Option<BuildLink>,
    pub screenshots: Vec<SnapshotShot>,
    pub archived: bool,
    pub quick_drop: bool,
}

/// The pop-out's build. `generation` counts opens, so a replaced window's late "closed" can't clear its successor.
#[derive(Default)]
pub struct BuildWindow(Mutex<Inner>);

#[derive(Default)]
struct Inner {
    generation: u64,
    snapshot: Option<BuildSnapshot>,
}

impl BuildWindow {
    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A window opens on `snapshot` (replacing whatever was shown); its generation.
    pub fn open(&self, snapshot: BuildSnapshot) -> u64 {
        let mut i = self.inner();
        i.generation += 1;
        i.snapshot = Some(snapshot);
        i.generation
    }

    /// A newer copy of the build on show; another build's is ignored (a different build page can't take the window
    /// over). Whether it was taken.
    pub fn update(&self, snapshot: BuildSnapshot) -> bool {
        let mut i = self.inner();
        match &mut i.snapshot {
            Some(s) if s.message_id == snapshot.message_id => {
                *s = snapshot;
                true
            }
            _ => false,
        }
    }

    /// Whether the window shows `id` (and so closes for it).
    pub fn shows(&self, id: MessageId) -> bool {
        self.inner()
            .snapshot
            .as_ref()
            .is_some_and(|s| s.message_id == id)
    }

    /// The window of `generation` closed: its snapshot goes, unless a newer window has replaced it.
    pub fn closed(&self, generation: u64) {
        let mut i = self.inner();
        if i.generation == generation {
            i.snapshot = None;
        }
    }

    /// Nothing on show (sign-out).
    pub fn clear(&self) {
        self.inner().snapshot = None;
    }

    pub fn snapshot(&self) -> Option<BuildSnapshot> {
        self.inner().snapshot.clone()
    }

    /// Which build's screenshots the pop-out may have links to, when it asks for `asked`: only the stored build's,
    /// never an id the page names otherwise. Its screenshots' ids come with it.
    pub fn image_target(&self, asked: MessageId) -> Result<(MessageId, Vec<AttachmentId>), String> {
        match &self.inner().snapshot {
            Some(s) if s.message_id == asked => {
                Ok((s.message_id, s.screenshots.iter().map(|x| x.id).collect()))
            }
            _ => Err(NOT_THIS_BUILD.to_string()),
        }
    }
}

/// Only the links to `shots` (the snapshot's screenshots), in the server's order.
pub fn only_shots(urls: Vec<AttachmentUrl>, shots: &[AttachmentId]) -> Vec<AttachmentUrl> {
    urls.into_iter().filter(|u| shots.contains(&u.id)).collect()
}

const NOT_THIS_BUILD: &str = "That build isn't in this window.";

/// Pop a build out (main window only), replacing a pop-out already open.
///
/// Async: building a window from a synchronous command can deadlock on Windows.
#[tauri::command]
pub async fn open_build_window(
    app: AppHandle,
    data: State<'_, WebviewData>,
    win: State<'_, BuildWindow>,
    snapshot: BuildSnapshot,
    on_top: bool,
) -> Result<(), String> {
    if let Some(old) = app.get_webview_window(BUILD_LABEL) {
        old.destroy().map_err(|e| e.to_string())?;
        // The label is freed once the event loop has handled the destroy.
        for _ in 0..40 {
            if app.get_webview_window(BUILD_LABEL).is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    let title = snapshot.title.clone();
    let generation = win.open(snapshot);
    let w = WebviewWindowBuilder::new(
        &app,
        BUILD_LABEL,
        WebviewUrl::App("index.html?window=build".into()),
    )
    .title(title)
    .inner_size(420.0, 640.0)
    .min_inner_size(300.0, 320.0)
    .always_on_top(on_top)
    // the main window's profile: its theme and settings (no credentials are kept there; the token stays in the core)
    .data_directory(data.0.clone())
    // the app's own page only; a link (target=_blank, or a page trying to leave) opens in the browser
    .on_navigation(crate::links::build_navigation)
    .on_new_window(|u, _| crate::links::new_window(u))
    .build();
    let w = match w {
        Ok(w) => w,
        Err(e) => {
            win.closed(generation);
            return Err(e.to_string());
        }
    };
    let h = app.clone();
    w.on_window_event(move |e| {
        if let tauri::WindowEvent::Destroyed = e {
            h.state::<BuildWindow>().closed(generation);
        }
    });
    Ok(())
}

/// A newer copy of the build on show (main window only): the pop-out picks it up on its next look. Another build's
/// copy is ignored.
#[tauri::command]
pub fn update_build_window(win: State<'_, BuildWindow>, snapshot: BuildSnapshot) {
    win.update(snapshot);
}

/// Close the pop-out if it shows `message_id` (main window only): the build was deleted or is out of reach.
#[tauri::command]
pub async fn close_build_window(
    app: AppHandle,
    win: State<'_, BuildWindow>,
    message_id: MessageId,
) -> Result<bool, String> {
    if !win.shows(message_id) {
        return Ok(false);
    }
    close(&app);
    Ok(true)
}

/// Close the pop-out, whatever it shows (sign-out).
pub fn close(app: &AppHandle) {
    if let Some(win) = app.try_state::<BuildWindow>() {
        win.clear();
    }
    if let Some(w) = app.get_webview_window(BUILD_LABEL) {
        let _ = w.destroy();
    }
}

/// The pop-out's build (pop-out only); none once it's gone.
#[tauri::command]
pub fn build_window_snapshot(win: State<'_, BuildWindow>) -> Option<BuildSnapshot> {
    win.snapshot()
}

/// Fresh links to the pop-out's build's screenshots (pop-out only; they last an hour, so it asks again before they
/// lapse). `message_id` must be the build on show: any other id is refused, and only that build's screenshots come
/// back.
#[tauri::command]
pub async fn build_window_image_urls(
    app: AppHandle,
    core: State<'_, Core>,
    win: State<'_, BuildWindow>,
    message_id: MessageId,
) -> Result<Vec<AttachmentUrl>, ApiError> {
    let (id, shots) = win.image_target(message_id).map_err(ApiError::Forbidden)?;
    let (api, token) = core.current()?;
    let urls = core.check(&app, api.attachment_urls(&token, id).await)?;
    Ok(only_shots(urls, &shots))
}

/// The pop-out's always-on-top switch (pop-out only).
#[tauri::command]
pub fn set_build_window_on_top(app: AppHandle, on: bool) -> Result<(), String> {
    let w = app
        .get_webview_window(BUILD_LABEL)
        .ok_or("The pop-out is closed.")?;
    w.set_always_on_top(on).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(id: MessageId, title: &str, shots: &[AttachmentId]) -> BuildSnapshot {
        BuildSnapshot {
            message_id: id,
            title: title.into(),
            game_name: None,
            game_guessed: false,
            author_name: "alex".into(),
            saved_at: "2026-11-25T10:00:00Z".into(),
            updated_at: None,
            notes: None,
            link: None,
            screenshots: shots
                .iter()
                .map(|&id| SnapshotShot {
                    id,
                    width: Some(16),
                    height: Some(9),
                })
                .collect(),
            archived: false,
            quick_drop: false,
        }
    }

    fn url(id: AttachmentId) -> AttachmentUrl {
        AttachmentUrl {
            id,
            url: Some(format!("https://example.r2.cloudflarestorage.com/{id}")),
            expires_at: None,
        }
    }

    #[test]
    fn image_urls_are_only_for_the_snapshots_build() {
        let w = BuildWindow::default();
        let (a, shot) = (MessageId::new(), AttachmentId::new());
        assert!(w.image_target(a).is_err(), "no snapshot: nothing");
        w.open(snap(a, "Frost mage", &[shot]));
        assert_eq!(w.image_target(a), Ok((a, vec![shot])));
    }

    #[test]
    fn a_forged_build_id_is_refused() {
        let w = BuildWindow::default();
        let (a, forged) = (MessageId::new(), MessageId::new());
        w.open(snap(a, "Frost mage", &[AttachmentId::new()]));
        assert_eq!(w.image_target(forged), Err(NOT_THIS_BUILD.to_string()));
    }

    #[test]
    fn only_the_snapshots_screenshots_come_back() {
        let (s1, s2, other) = (
            AttachmentId::new(),
            AttachmentId::new(),
            AttachmentId::new(),
        );
        let got = only_shots(vec![url(s1), url(other), url(s2)], &[s1, s2]);
        let ids: Vec<_> = got.iter().map(|u| u.id).collect();
        assert_eq!(ids, [s1, s2]);
    }

    #[test]
    fn a_new_snapshot_replaces_the_old() {
        let w = BuildWindow::default();
        let (a, b) = (MessageId::new(), MessageId::new());
        let g1 = w.open(snap(a, "Frost mage", &[]));
        let g2 = w.open(snap(b, "Bleed rogue", &[]));
        assert_ne!(g1, g2);
        assert_eq!(w.snapshot().unwrap().message_id, b);
        assert!(w.image_target(a).is_err(), "the old build is out of reach");
        // the replaced window's late close leaves the new one alone
        w.closed(g1);
        assert_eq!(w.snapshot().unwrap().message_id, b);
        w.closed(g2);
        assert!(w.snapshot().is_none());
    }

    #[test]
    fn an_update_of_the_same_build_is_taken_and_another_builds_is_ignored() {
        let w = BuildWindow::default();
        let (a, b) = (MessageId::new(), MessageId::new());
        assert!(!w.update(snap(a, "Frost mage", &[])), "nothing open");
        assert!(w.snapshot().is_none());
        w.open(snap(a, "Frost mage", &[]));
        assert!(w.update(snap(a, "Frost mage v2", &[])));
        assert_eq!(w.snapshot().unwrap().title, "Frost mage v2");
        assert!(!w.update(snap(b, "Bleed rogue", &[])));
        assert_eq!(w.snapshot().unwrap().message_id, a);
    }

    #[test]
    fn close_only_closes_the_matching_build() {
        let w = BuildWindow::default();
        let (a, b) = (MessageId::new(), MessageId::new());
        assert!(!w.shows(a));
        w.open(snap(a, "Frost mage", &[]));
        assert!(w.shows(a));
        assert!(!w.shows(b));
        w.clear();
        assert!(!w.shows(a));
        assert!(w.image_target(a).is_err(), "signed out: nothing");
    }
}
