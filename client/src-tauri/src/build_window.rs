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

use pulse_protocol::gateway::Event;
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
pub struct BuildWindow {
    inner: Mutex<Inner>,
    /// Held by an open from its look at the window to the end of its build (#639): two opens at once would otherwise
    /// both find no window, and the second's build, refused for the label the first just took, would clear the
    /// first's build ("This build is gone.").
    opening: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct Inner {
    generation: u64,
    /// Sign-outs so far: an open that started before one never shows (or keeps) its window.
    signouts: u64,
    snapshot: Option<BuildSnapshot>,
    /// The generation of the window that is up and not closing: one an open can show another build in.
    live: Option<u64>,
}

/// What [`BuildWindow::switch`] did.
#[derive(Debug, PartialEq)]
pub enum Switch {
    /// The live window shows the new snapshot now (a copy of it, should that window turn out to be gone).
    Done(Box<BuildSnapshot>),
    /// Signed out since the open began: nothing stored.
    SignedOut,
    /// No live window (none, or it's closing): a new one must be built for the snapshot, handed back.
    NoWindow(Box<BuildSnapshot>),
}

/// Taken when an open starts, before it waits for the old window to go ([`BuildWindow::begin`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket(u64);

impl BuildWindow {
    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// An open starts: what it checks against before showing anything.
    pub fn begin(&self) -> Ticket {
        Ticket(self.inner().signouts)
    }

    /// A window opens on `snapshot` (replacing whatever was shown); its generation. None when a sign-out came since
    /// `ticket` (nothing is stored).
    pub fn open(&self, ticket: Ticket, snapshot: BuildSnapshot) -> Option<u64> {
        let mut i = self.inner();
        if i.signouts != ticket.0 {
            return None;
        }
        i.generation += 1;
        i.snapshot = Some(snapshot);
        Some(i.generation)
    }

    /// The window of `generation` is built and up: whether it still has its build. A close (the build deleted or out
    /// of reach) while it was being built found no window to destroy but cleared the snapshot, and nothing else
    /// clears it while an open holds the lock; then it must go, not show "This build is gone."
    pub fn built(&self, generation: u64) -> bool {
        let mut i = self.inner();
        if i.generation != generation || i.snapshot.is_none() {
            return false;
        }
        i.live = Some(generation);
        true
    }

    /// The live window takes `snapshot` in place (same window, same generation: nothing moves on screen), or says
    /// why it can't.
    pub fn switch(&self, ticket: Ticket, snapshot: BuildSnapshot) -> Switch {
        let mut i = self.inner();
        if i.signouts != ticket.0 {
            return Switch::SignedOut;
        }
        if i.live != Some(i.generation) {
            return Switch::NoWindow(Box::new(snapshot));
        }
        i.snapshot = Some(snapshot.clone());
        Switch::Done(Box::new(snapshot))
    }

    /// Whether a sign-out came since `ticket` (a window built for it must go).
    pub fn signed_out_since(&self, ticket: Ticket) -> bool {
        self.inner().signouts != ticket.0
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
        if i.live == Some(generation) {
            i.live = None;
        }
    }

    /// The window of `generation` is on its way out (its close button): an open builds a new one rather than show
    /// its build in this one.
    pub fn closing(&self, generation: u64) {
        let mut i = self.inner();
        if i.live == Some(generation) {
            i.live = None;
        }
    }

    /// Nothing on show (sign-out, or the build is gone).
    pub fn clear(&self) {
        let mut i = self.inner();
        i.snapshot = None;
        i.live = None;
    }

    /// Signed out: nothing on show, and an open under way is refused ([`Self::open`], [`Self::signed_out_since`]).
    pub fn sign_out(&self) {
        let mut i = self.inner();
        i.signouts += 1;
        i.snapshot = None;
        i.live = None;
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

/// The build a gateway event says is gone (deleted), if any: a backstop for the main window's own closing, which a
/// reload of its page or a delete while it was away would miss.
pub fn gone_in(e: &Event) -> Option<MessageId> {
    match e {
        Event::MessageDeleted {
            message_id,
            thread_root_id: None,
            ..
        } => Some(*message_id),
        Event::MessageUpdated { message }
            if message.deleted && message.thread_root_id.is_none() =>
        {
            Some(message.id)
        }
        _ => None,
    }
}

/// Whether the server's answer for the build's screenshots says it's gone or out of reach (403, 404).
pub fn means_gone(e: &ApiError) -> bool {
    match e {
        ApiError::Forbidden(_) => true,
        // a 404 comes back as the server's "not found"
        ApiError::Rejected(m) => m.eq_ignore_ascii_case("not found"),
        _ => false,
    }
}

/// Whether a fresh fetch of the build on show (after each Ready: deletes while disconnected send no event) says it's
/// gone: deleted, no longer a build, or refused as [`means_gone`].
pub fn gone_after_recheck(r: &Result<pulse_protocol::rest::Message, ApiError>) -> bool {
    match r {
        Ok(m) => m.deleted || m.build.is_none(),
        Err(e) => means_gone(e),
    }
}

/// After each Ready (#620): the build on show is fetched again, and the pop-out closes if it's gone. A delete while
/// disconnected sends no event, and the main window's page may have been reloaded since.
pub fn recheck_after_ready(app: &AppHandle) {
    let Some(id) = app
        .try_state::<BuildWindow>()
        .and_then(|w| w.snapshot())
        .map(|s| s.message_id)
    else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let core = app.state::<Core>();
        let Ok((api, token)) = core.current() else {
            return;
        };
        let r = core.check(&app, api.get_build(&token, id).await);
        if gone_after_recheck(&r) && app.state::<BuildWindow>().shows(id) {
            close(&app);
        }
    });
}

const NOT_THIS_BUILD: &str = "That build isn't in this window.";

/// What an open does with the pop-out window: the app's ([`TauriPopout`]) or a test's.
pub(crate) trait Popout {
    /// The window is there (it may be closing: its label stays taken till the event loop has destroyed it).
    fn exists(&self) -> bool;
    fn destroy(&self);
    /// Builds the window for the snapshot of `generation`. Fails while the label is still taken.
    async fn build(&self, title: &str, on_top: bool, generation: u64) -> Result<(), String>;
    fn bring_forward(&self, title: &str);
}

/// Opens the pop-out on `snapshot` (see [`open_build_window`]).
pub(crate) async fn open_in(
    p: &impl Popout,
    win: &BuildWindow,
    ticket: Ticket,
    snapshot: BuildSnapshot,
    on_top: bool,
) -> Result<(), String> {
    // one open at a time, from the look at the window to the end of its build (#639)
    let _one = win.opening.lock().await;
    let title = snapshot.title.clone();
    // a window already up shows this build in place, and comes forward: no second window, nothing moves
    let snapshot = match win.switch(ticket, snapshot) {
        Switch::Done(_) if p.exists() => {
            p.bring_forward(&title);
            return Ok(());
        }
        // it went meanwhile (its late "closed" may clear the snapshot just stored): a new one, for this copy
        Switch::Done(s) => *s,
        Switch::SignedOut => return Err(SIGNED_OUT.into()),
        Switch::NoWindow(s) => *s,
    };
    // a window on its way out: wait till it's gone
    if p.exists() {
        p.destroy();
        // The label is freed once the event loop has handled the destroy.
        for _ in 0..40 {
            if !p.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    let generation = win.open(ticket, snapshot).ok_or(SIGNED_OUT)?;
    if let Err(e) = p.build(&title, on_top, generation).await {
        win.closed(generation);
        return Err(e);
    }
    // signed out while it was being built (the sign-out's close found no window yet): it goes now
    if win.signed_out_since(ticket) {
        p.destroy();
        win.clear();
        return Err(SIGNED_OUT.into());
    }
    // closed while it was being built (its build deleted): likewise
    if !win.built(generation) {
        p.destroy();
        return Err(CLOSED.into());
    }
    Ok(())
}

/// The app's pop-out window.
struct TauriPopout<'a> {
    app: &'a AppHandle,
    data: &'a WebviewData,
}

impl Popout for TauriPopout<'_> {
    fn exists(&self) -> bool {
        self.app.get_webview_window(BUILD_LABEL).is_some()
    }

    fn destroy(&self) {
        destroy(self.app);
    }

    async fn build(&self, title: &str, on_top: bool, generation: u64) -> Result<(), String> {
        let w = WebviewWindowBuilder::new(
            self.app,
            BUILD_LABEL,
            WebviewUrl::App("index.html?window=build".into()),
        )
        .title(title)
        .inner_size(420.0, 640.0)
        .min_inner_size(300.0, 320.0)
        .always_on_top(on_top)
        // the main window's profile: its theme and settings (no credentials are kept there; the token stays in the
        // core)
        .data_directory(self.data.0.clone())
        // the app's own page only; a link (target=_blank, or a page trying to leave) opens in the browser
        .on_navigation(crate::links::build_navigation)
        .on_new_window(|u, _| crate::links::new_window(u))
        .build()
        .map_err(|e| e.to_string())?;
        let h = self.app.clone();
        w.on_window_event(move |e| match e {
            tauri::WindowEvent::CloseRequested { .. } => {
                h.state::<BuildWindow>().closing(generation)
            }
            tauri::WindowEvent::Destroyed => h.state::<BuildWindow>().closed(generation),
            _ => {}
        });
        Ok(())
    }

    fn bring_forward(&self, title: &str) {
        if let Some(w) = self.app.get_webview_window(BUILD_LABEL) {
            let _ = w.set_title(title);
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
}

/// Pop a build out (main window only). A pop-out already open shows it instead, in place, and comes forward (it keeps
/// its on-top state: `on_top` is for a new window).
///
/// Async: building a window from a synchronous command can deadlock on Windows.
#[tauri::command]
pub async fn open_build_window(
    app: AppHandle,
    core: State<'_, Core>,
    data: State<'_, WebviewData>,
    win: State<'_, BuildWindow>,
    snapshot: BuildSnapshot,
    on_top: bool,
) -> Result<(), String> {
    // before any wait: a sign-out from here on means this open shows nothing
    let ticket = win.begin();
    core.current().map_err(|e| e.to_string())?;
    let p = TauriPopout {
        app: &app,
        data: &data,
    };
    open_in(&p, &win, ticket, snapshot, on_top).await
}

const SIGNED_OUT: &str = "Signed out.";

/// Closed (its build deleted or out of reach) while it was being built.
const CLOSED: &str = "The pop-out was closed.";

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

/// Signed out: the pop-out closes, whatever it shows, and an open under way is refused.
pub fn signed_out(app: &AppHandle) {
    if let Some(win) = app.try_state::<BuildWindow>() {
        win.sign_out();
    }
    destroy(app);
}

/// Close the pop-out, whatever it shows.
pub fn close(app: &AppHandle) {
    if let Some(win) = app.try_state::<BuildWindow>() {
        win.clear();
    }
    destroy(app);
}

/// A gateway event (the core's backstop, #620): the build on show was deleted, so the pop-out closes, whatever the
/// main window's page knows.
pub fn note_event(app: &AppHandle, e: &Event) {
    let Some(id) = gone_in(e) else { return };
    if app.try_state::<BuildWindow>().is_some_and(|w| w.shows(id)) {
        close(app);
    }
}

fn destroy(app: &AppHandle) {
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
    match core.check(&app, api.attachment_urls(&token, id).await) {
        Ok(urls) => Ok(only_shots(urls, &shots)),
        Err(e) => {
            // deleted, or out of reach (left the server): the pop-out closes, if it still shows that build
            if means_gone(&e) && win.shows(id) {
                close(&app);
            }
            Err(e)
        }
    }
}

/// The pop-out's always-on-top switch (pop-out only). Whether the window can be kept on top here: false under
/// Wayland, whose GTK backend ignores keep-above (there is no protocol for it), so nothing is done and the pop-out
/// shows no switch (QPMKJE: a switch that does nothing). The pop-out asks once as it opens, re-stating its opening
/// state (on top), so this command doubles as that question rather than the gate letting it call another.
#[tauri::command]
pub fn set_build_window_on_top(app: AppHandle, on: bool) -> Result<bool, String> {
    let w = app
        .get_webview_window(BUILD_LABEL)
        .ok_or("The pop-out is closed.")?;
    if !can_keep_on_top(&w)? {
        return Ok(false);
    }
    w.set_always_on_top(on).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Whether a GDK display of this type name honours keep-above (every backend but Wayland's).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn keeps_above(display_type: &str) -> bool {
    display_type != "GdkWaylandDisplay"
}

/// Whether `w` can be kept on top: on Linux, by its GDK display's backend (read on the main thread, as GTK needs).
#[cfg(target_os = "linux")]
fn can_keep_on_top(w: &tauri::WebviewWindow) -> Result<bool, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let win = w.clone();
    w.run_on_main_thread(move || {
        use gtk::prelude::*;
        let ok = win
            .gtk_window()
            .map(|g| keeps_above(g.display().type_().name()))
            .unwrap_or(true);
        let _ = tx.send(ok);
    })
    .map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())
}

#[cfg(not(target_os = "linux"))]
fn can_keep_on_top(_: &tauri::WebviewWindow) -> Result<bool, String> {
    Ok(true)
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
    fn only_wayland_cannot_keep_the_pop_out_on_top() {
        assert!(!keeps_above("GdkWaylandDisplay"));
        assert!(keeps_above("GdkX11Display"));
        assert!(keeps_above("GdkWin32Display"));
    }

    #[test]
    fn image_urls_are_only_for_the_snapshots_build() {
        let w = BuildWindow::default();
        let (a, shot) = (MessageId::new(), AttachmentId::new());
        assert!(w.image_target(a).is_err(), "no snapshot: nothing");
        w.open(w.begin(), snap(a, "Frost mage", &[shot]));
        assert_eq!(w.image_target(a), Ok((a, vec![shot])));
    }

    #[test]
    fn a_forged_build_id_is_refused() {
        let w = BuildWindow::default();
        let (a, forged) = (MessageId::new(), MessageId::new());
        w.open(w.begin(), snap(a, "Frost mage", &[AttachmentId::new()]));
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
        let g1 = w.open(w.begin(), snap(a, "Frost mage", &[])).unwrap();
        let g2 = w.open(w.begin(), snap(b, "Bleed rogue", &[])).unwrap();
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
        w.open(w.begin(), snap(a, "Frost mage", &[]));
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
        w.open(w.begin(), snap(a, "Frost mage", &[]));
        assert!(w.shows(a));
        assert!(!w.shows(b));
        w.clear();
        assert!(!w.shows(a));
        assert!(w.image_target(a).is_err(), "signed out: nothing");
    }

    #[test]
    fn a_sign_out_while_opening_refuses_the_open() {
        let w = BuildWindow::default();
        let a = MessageId::new();
        // the open starts, then waits for the old window to go; the sign-out lands meanwhile
        let t = w.begin();
        w.sign_out();
        assert_eq!(w.open(t, snap(a, "Frost mage", &[])), None);
        assert!(
            w.snapshot().is_none(),
            "the old account's build is never stored"
        );
        assert!(w.image_target(a).is_err());
        // an open after the sign-out works
        let t = w.begin();
        assert!(w.open(t, snap(a, "Frost mage", &[])).is_some());
    }

    #[test]
    fn a_sign_out_after_the_window_was_built_is_noticed() {
        let w = BuildWindow::default();
        let t = w.begin();
        w.open(t, snap(MessageId::new(), "Frost mage", &[]))
            .unwrap();
        assert!(!w.signed_out_since(t));
        w.sign_out();
        assert!(w.signed_out_since(t), "the window it built must go");
        assert!(w.snapshot().is_none());
    }

    /// A pop-out window as Tauri keeps one: building takes a while, its label is taken from the start of the build
    /// (though the window can't be found till it's done), and a destroyed window is there, label and all, till the
    /// event loop gets to it; then its "destroyed" handler runs.
    #[derive(Clone)]
    struct FakePopout(std::sync::Arc<Mutex<Fake>>, std::sync::Arc<BuildWindow>);

    #[derive(Default)]
    struct Fake {
        /// The generation of the window that can be found, if any.
        shown: Option<u64>,
        /// The label is taken (a window is being built, is shown or is being destroyed).
        taken: bool,
        builds: u32,
        /// Brought forward with these titles (reused, not rebuilt).
        forwards: Vec<String>,
        /// How long the event loop takes to destroy a window (60 ms when unset).
        destroy_delay: Option<Duration>,
        /// A window gone from the app whose "destroyed" handler hasn't run yet: it runs as the next look finds no
        /// window (the race with an open's look, made certain).
        destroyed_pending: Option<u64>,
    }

    impl FakePopout {
        fn new(win: &std::sync::Arc<BuildWindow>) -> Self {
            Self(Default::default(), win.clone())
        }
        fn state(&self) -> std::sync::MutexGuard<'_, Fake> {
            self.0.lock().unwrap()
        }
        /// The user closes the window (its close button): it's on its way out but still there.
        fn user_closes(&self) {
            let generation = self.state().shown.unwrap();
            self.1.closing(generation);
            self.destroy();
        }
    }

    impl Popout for FakePopout {
        fn exists(&self) -> bool {
            let (shown, pending) = {
                let mut s = self.state();
                (s.shown, s.destroyed_pending.take())
            };
            if let Some(generation) = pending {
                self.1.closed(generation);
            }
            shown.is_some()
        }
        fn destroy(&self) {
            let (Some(generation), delay) = ({
                let s = self.state();
                (s.shown, s.destroy_delay)
            }) else {
                return;
            };
            let me = self.clone();
            tokio::spawn(async move {
                tokio::time::sleep(delay.unwrap_or(Duration::from_millis(60))).await;
                {
                    let mut s = me.state();
                    if s.shown == Some(generation) {
                        s.shown = None;
                        s.taken = false;
                    }
                }
                me.1.closed(generation);
            });
        }
        async fn build(&self, _title: &str, _on_top: bool, generation: u64) -> Result<(), String> {
            {
                let mut s = self.state();
                if s.taken {
                    return Err("a webview with label `build` already exists".into());
                }
                s.taken = true;
                s.builds += 1;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
            self.state().shown = Some(generation);
            Ok(())
        }
        fn bring_forward(&self, title: &str) {
            self.state().forwards.push(title.into());
        }
    }

    /// Lets the fake's pending destroys run.
    async fn settle() {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    #[tokio::test(start_paused = true)]
    async fn two_pop_outs_at_once_leave_one_window_on_the_second_build() {
        let win = std::sync::Arc::new(BuildWindow::default());
        let p = FakePopout::new(&win);
        let (a, b) = (MessageId::new(), MessageId::new());
        let (ra, rb) = tokio::join!(
            open_in(&p, &win, win.begin(), snap(a, "Frost mage", &[]), true),
            open_in(&p, &win, win.begin(), snap(b, "Bleed rogue", &[]), true),
        );
        settle().await;
        assert_eq!((ra, rb), (Ok(()), Ok(())));
        assert_eq!(
            win.snapshot().map(|s| s.message_id),
            Some(b),
            "the window shows the second build, not \"This build is gone.\""
        );
        assert!(p.exists());
        assert_eq!(p.state().builds, 1, "one window, reused");
        assert_eq!(p.state().forwards, ["Bleed rogue"]);
    }

    #[tokio::test(start_paused = true)]
    async fn popping_out_the_same_build_twice_brings_its_window_forward() {
        let win = std::sync::Arc::new(BuildWindow::default());
        let p = FakePopout::new(&win);
        let a = MessageId::new();
        let (ra, rb) = tokio::join!(
            open_in(&p, &win, win.begin(), snap(a, "Frost mage", &[]), true),
            open_in(&p, &win, win.begin(), snap(a, "Frost mage", &[]), true),
        );
        settle().await;
        assert_eq!((ra, rb), (Ok(()), Ok(())));
        assert_eq!(win.snapshot().map(|s| s.message_id), Some(a));
        assert_eq!(p.state().builds, 1);
        assert_eq!(p.state().forwards, ["Frost mage"]);
        // once open, again: still the one window
        open_in(&p, &win, win.begin(), snap(a, "Frost mage", &[]), true)
            .await
            .unwrap();
        assert_eq!(p.state().builds, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn another_build_switches_the_open_window_in_place() {
        let win = std::sync::Arc::new(BuildWindow::default());
        let p = FakePopout::new(&win);
        let (a, b) = (MessageId::new(), MessageId::new());
        open_in(&p, &win, win.begin(), snap(a, "Frost mage", &[]), true)
            .await
            .unwrap();
        open_in(&p, &win, win.begin(), snap(b, "Bleed rogue", &[]), true)
            .await
            .unwrap();
        settle().await;
        assert_eq!(win.snapshot().map(|s| s.message_id), Some(b));
        assert_eq!(p.state().builds, 1, "the window stays where it is");
        assert!(
            win.image_target(a).is_err(),
            "the old build is out of reach"
        );
        // closing it later clears the build it now shows
        p.destroy();
        settle().await;
        assert!(win.snapshot().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn popping_out_while_the_window_is_closing_opens_a_new_one() {
        let win = std::sync::Arc::new(BuildWindow::default());
        let p = FakePopout::new(&win);
        let (a, b) = (MessageId::new(), MessageId::new());
        open_in(&p, &win, win.begin(), snap(a, "Frost mage", &[]), true)
            .await
            .unwrap();
        // closed by its button, and Pop out pressed before it's gone
        p.user_closes();
        open_in(&p, &win, win.begin(), snap(b, "Bleed rogue", &[]), true)
            .await
            .unwrap();
        settle().await;
        assert!(p.exists(), "a window is up");
        assert_eq!(p.state().builds, 2, "a new window, not the closing one");
        assert_eq!(
            win.snapshot().map(|s| s.message_id),
            Some(b),
            "the old window's late close leaves the new one's build"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_sign_out_while_waiting_for_another_open_refuses_it() {
        let win = std::sync::Arc::new(BuildWindow::default());
        let p = FakePopout::new(&win);
        let (a, b) = (MessageId::new(), MessageId::new());
        let (ta, tb) = (win.begin(), win.begin());
        let first = open_in(&p, &win, ta, snap(a, "Frost mage", &[]), true);
        let second = async {
            tokio::task::yield_now().await;
            win.sign_out();
            open_in(&p, &win, tb, snap(b, "Bleed rogue", &[]), true).await
        };
        let (_, rb) = tokio::join!(first, second);
        assert_eq!(rb, Err(SIGNED_OUT.to_string()));
        assert!(
            win.snapshot().is_none(),
            "the old account's build is never stored"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_sign_out_while_the_window_is_being_built_takes_it_and_says_so() {
        let win = std::sync::Arc::new(BuildWindow::default());
        let p = FakePopout::new(&win);
        let (r, _) = tokio::join!(
            open_in(
                &p,
                &win,
                win.begin(),
                snap(MessageId::new(), "Frost mage", &[]),
                true
            ),
            async {
                tokio::time::sleep(Duration::from_millis(10)).await;
                win.sign_out();
            }
        );
        settle().await;
        assert_eq!(r, Err(SIGNED_OUT.to_string()));
        assert!(!p.exists());
        assert!(win.snapshot().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn a_close_while_the_window_is_being_built_takes_the_new_window_too() {
        let win = std::sync::Arc::new(BuildWindow::default());
        let p = FakePopout::new(&win);
        let a = MessageId::new();
        // the build is deleted (close: no window to find yet) while its pop-out is being built
        let (r, _) = tokio::join!(
            open_in(&p, &win, win.begin(), snap(a, "Frost mage", &[]), true),
            async {
                tokio::time::sleep(Duration::from_millis(10)).await;
                win.clear();
            }
        );
        settle().await;
        assert_eq!(r, Err(CLOSED.to_string()));
        assert!(!p.exists(), "no window left on \"This build is gone.\"");
        assert!(win.snapshot().is_none());
        // the next Pop out works
        open_in(&p, &win, win.begin(), snap(a, "Frost mage", &[]), true)
            .await
            .unwrap();
        assert!(p.exists());
        assert_eq!(win.snapshot().map(|s| s.message_id), Some(a));
    }

    #[tokio::test(start_paused = true)]
    async fn a_late_destroyed_between_the_switch_and_the_look_still_opens_the_build() {
        let win = std::sync::Arc::new(BuildWindow::default());
        let p = FakePopout::new(&win);
        let (a, b) = (MessageId::new(), MessageId::new());
        let g1 = open_in(&p, &win, win.begin(), snap(a, "Frost mage", &[]), true)
            .await
            .map(|_| p.state().shown.unwrap())
            .unwrap();
        // the window is gone from the app, its "destroyed" not yet handled: it lands as the open looks
        {
            let mut s = p.state();
            s.shown = None;
            s.taken = false;
            s.destroyed_pending = Some(g1);
        }
        let r = open_in(&p, &win, win.begin(), snap(b, "Bleed rogue", &[]), true).await;
        settle().await;
        assert_eq!(r, Ok(()), "not \"Signed out.\"");
        assert!(p.exists());
        assert_eq!(p.state().builds, 2);
        assert_eq!(win.snapshot().map(|s| s.message_id), Some(b));
    }

    #[tokio::test(start_paused = true)]
    async fn a_window_slow_to_go_fails_the_open_cleanly() {
        let win = std::sync::Arc::new(BuildWindow::default());
        let p = FakePopout::new(&win);
        let (a, b) = (MessageId::new(), MessageId::new());
        open_in(&p, &win, win.begin(), snap(a, "Frost mage", &[]), true)
            .await
            .unwrap();
        // closing, and the event loop takes longer than the open waits (1 s)
        p.state().destroy_delay = Some(Duration::from_secs(3));
        p.user_closes();
        let r = tokio::time::timeout(
            Duration::from_secs(2),
            open_in(&p, &win, win.begin(), snap(b, "Bleed rogue", &[]), true),
        )
        .await
        .expect("the open gives up rather than hang");
        assert!(r.is_err(), "the label is still taken: {r:?}");
        assert!(
            win.snapshot().is_none(),
            "nothing stored for a window that never came"
        );
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(!p.exists());
        assert!(win.snapshot().is_none());
        // and the next Pop out works
        open_in(&p, &win, win.begin(), snap(b, "Bleed rogue", &[]), true)
            .await
            .unwrap();
        assert_eq!(win.snapshot().map(|s| s.message_id), Some(b));
    }

    fn deleted(id: MessageId, thread_root_id: Option<MessageId>) -> Event {
        Event::MessageDeleted {
            channel_id: pulse_protocol::ids::ChannelId::new(),
            message_id: id,
            author_id: None,
            mentions: vec![],
            thread_root_id,
        }
    }

    #[test]
    fn a_deleted_build_is_gone_in_the_gateway_event() {
        let id = MessageId::new();
        assert_eq!(gone_in(&deleted(id, None)), Some(id));
        // a comment deleted under a build isn't the build
        assert_eq!(gone_in(&deleted(id, Some(MessageId::new()))), None);
        let msg: pulse_protocol::rest::Message = serde_json::from_value(serde_json::json!({
            "id": id, "channel_id": pulse_protocol::ids::ChannelId::new(), "author_id": null, "kind": "normal",
            "content": "", "reply_to_id": null, "created_at": "2026-11-25T10:00:00Z", "edited_at": null,
            "deleted": true, "mentions": [], "reactions": [], "thread_root_id": null
        }))
        .unwrap();
        assert_eq!(
            gone_in(&Event::MessageUpdated {
                message: msg.clone()
            }),
            Some(id)
        );
        let kept = pulse_protocol::rest::Message {
            deleted: false,
            ..msg.clone()
        };
        assert_eq!(gone_in(&Event::MessageUpdated { message: kept }), None);
        let reply = pulse_protocol::rest::Message {
            thread_root_id: Some(MessageId::new()),
            ..msg
        };
        assert_eq!(gone_in(&Event::MessageUpdated { message: reply }), None);
    }

    #[test]
    fn a_403_or_404_for_its_screenshots_means_the_build_is_gone() {
        assert!(means_gone(&ApiError::Forbidden("forbidden".into())));
        assert!(means_gone(&ApiError::Rejected("not found".into())));
        assert!(means_gone(&ApiError::Rejected("Not found".into())));
        assert!(!means_gone(&ApiError::Network("offline".into())));
        assert!(!means_gone(&ApiError::Server("502".into())));
        assert!(!means_gone(&ApiError::Unauthorized));
        assert!(!means_gone(&ApiError::RateLimited));
    }

    #[test]
    fn a_recheck_after_ready_finds_a_build_gone_meanwhile() {
        let msg: pulse_protocol::rest::Message = serde_json::from_value(serde_json::json!({
            "id": MessageId::new(), "channel_id": pulse_protocol::ids::ChannelId::new(), "author_id": null,
            "kind": "normal", "content": "", "reply_to_id": null, "created_at": "2026-11-25T10:00:00Z",
            "edited_at": null, "deleted": false, "mentions": [], "reactions": [], "thread_root_id": null
        }))
        .unwrap();
        assert!(gone_after_recheck(&Ok(msg.clone())), "no longer a build");
        assert!(gone_after_recheck(&Ok(pulse_protocol::rest::Message {
            deleted: true,
            ..msg.clone()
        })));
        assert!(gone_after_recheck(&Err(ApiError::Rejected(
            "not found".into()
        ))));
        assert!(gone_after_recheck(&Err(ApiError::Forbidden(
            "forbidden".into()
        ))));
        assert!(!gone_after_recheck(&Err(ApiError::Network(
            "offline".into()
        ))));
    }
}
