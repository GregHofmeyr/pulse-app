//! The page's process dying (or hanging) leaves a dead window while the Rust core, voice
//! included, carries on. Reload the page instead; if the whole browser process died (WebView2
//! only), the webview can't be reloaded, so Pulse restarts. A page that keeps dying isn't reloaded
//! forever: past `MAX_RELOADS` within `RELOAD_WINDOW` it is left as it is, and a restart that
//! failed again within `RESTART_GUARD` isn't repeated. The policy is plain logic; applying it is
//! per platform.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub const MAX_RELOADS: usize = 3;
pub const RELOAD_WINDOW: Duration = Duration::from_secs(60);
pub const RESTART_GUARD: Duration = Duration::from_secs(60);
/// Set for the restarted process: the unix time the last browser-process restart happened.
pub const RESTARTED_ENV: &str = "PULSE_WEBVIEW_RESTARTED_AT";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The page's renderer exited (crash, out of memory, killed).
    RendererGone,
    /// The page's renderer stopped responding.
    RendererHung,
    /// The browser process itself exited: the webview is unusable.
    BrowserGone,
    /// A helper process (GPU, utility, a subframe…): the webview recovers by itself.
    Helper,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    Nothing,
    Reload,
    Restart,
    /// Failing again and again: stop trying, so it doesn't spin.
    GiveUp,
}

#[derive(Debug, Default)]
pub struct CrashPolicy {
    reloads: VecDeque<Instant>,
    /// Unix seconds of the restart that started this process, if one did.
    restarted_at: Option<u64>,
}

impl CrashPolicy {
    pub fn new(restarted_at: Option<u64>) -> Self {
        Self {
            reloads: VecDeque::new(),
            restarted_at,
        }
    }

    pub fn failed(&mut self, failure: Failure, now: Instant, unix_now: u64) -> Recovery {
        match failure {
            Failure::Helper => Recovery::Nothing,
            Failure::RendererGone | Failure::RendererHung => {
                while self
                    .reloads
                    .front()
                    .is_some_and(|t| now.duration_since(*t) >= RELOAD_WINDOW)
                {
                    self.reloads.pop_front();
                }
                if self.reloads.len() >= MAX_RELOADS {
                    return Recovery::GiveUp;
                }
                self.reloads.push_back(now);
                Recovery::Reload
            }
            Failure::BrowserGone => match self.restarted_at {
                Some(at) if unix_now.saturating_sub(at) < RESTART_GUARD.as_secs() => {
                    Recovery::GiveUp
                }
                _ => Recovery::Restart,
            },
        }
    }
}

/// This process's restart time, read from [`RESTARTED_ENV`].
pub fn restarted_at() -> Option<u64> {
    std::env::var(RESTARTED_ENV).ok()?.parse().ok()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn recover(
    app: &tauri::AppHandle,
    policy: &std::sync::Mutex<CrashPolicy>,
    failure: Failure,
    reload: impl FnOnce(),
) {
    let recovery = policy
        .lock()
        .unwrap()
        .failed(failure, Instant::now(), unix_now());
    match recovery {
        Recovery::Nothing => tracing::warn!(
            ?failure,
            "webview helper process failed; it recovers by itself"
        ),
        Recovery::Reload => {
            tracing::error!(?failure, "web renderer failed; reloading");
            reload();
        }
        Recovery::GiveUp => {
            tracing::error!(?failure, "web renderer keeps failing; not reloading again")
        }
        Recovery::Restart => {
            tracing::error!(?failure, "webview browser process exited; restarting Pulse");
            // SAFETY: the process is about to exit; the new one inherits the variable, and
            // nothing else reads or writes the environment meanwhile.
            unsafe { std::env::set_var(RESTARTED_ENV, unix_now().to_string()) };
            // Through the normal exit, so voice leaves cleanly and the single-instance lock is
            // released before the new process looks for it.
            app.request_restart();
        }
    }
}

/// WebKitGTK: reload when the page's web process terminates.
#[cfg(target_os = "linux")]
pub fn install(win: &tauri::WebviewWindow) -> tauri::Result<()> {
    use std::sync::{Arc, Mutex};
    let policy = Arc::new(Mutex::new(CrashPolicy::new(restarted_at())));
    use tauri::Manager;
    let app = win.app_handle().clone();
    win.with_webview(move |wv| {
        use webkit2gtk::WebViewExt;
        wv.inner()
            .connect_web_process_terminated(move |view, reason| {
                tracing::error!(?reason, "web renderer died");
                recover(&app, &policy, Failure::RendererGone, || view.reload());
            });
    })
}

/// WebView2: reload on a renderer failure, restart if the browser process is gone.
#[cfg(windows)]
pub fn install(win: &tauri::WebviewWindow) -> tauri::Result<()> {
    use std::sync::{Arc, Mutex};
    let policy = Arc::new(Mutex::new(CrashPolicy::new(restarted_at())));
    use tauri::Manager;
    let app = win.app_handle().clone();
    win.with_webview(move |wv| {
        use webview2_com::Microsoft::Web::WebView2::Win32::*;
        use webview2_com::ProcessFailedEventHandler;
        let handler = ProcessFailedEventHandler::create(Box::new(move |core, args| {
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
            if let Some(args) = &args {
                // SAFETY: a plain COM getter on the event's args, on the webview's thread.
                unsafe { args.ProcessFailedKind(&mut kind)? };
            }
            let failure = match kind {
                COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => Failure::RendererGone,
                COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => {
                    Failure::RendererHung
                }
                COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => Failure::BrowserGone,
                _ => Failure::Helper,
            };
            tracing::error!(kind = kind.0, "webview process failed");
            recover(&app, &policy, failure, || {
                if let Some(core) = &core {
                    // SAFETY: Reload on the webview that raised the event, on its own thread.
                    if let Err(e) = unsafe { core.Reload() } {
                        tracing::error!(error = %e, "webview reload failed");
                    }
                }
            });
            Ok(())
        }));
        let mut token = 0i64;
        // SAFETY: plain COM calls on the webview's own thread (with_webview runs us there).
        let added = unsafe {
            wv.controller()
                .CoreWebView2()
                .and_then(|core| core.add_ProcessFailed(&handler, &mut token))
        };
        if let Err(e) = added {
            tracing::warn!(error = %e, "webview crash recovery not installed");
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_renderer_that_dies_is_reloaded() {
        let mut p = CrashPolicy::new(None);
        assert_eq!(
            p.failed(Failure::RendererGone, Instant::now(), 1000),
            Recovery::Reload
        );
    }

    #[test]
    fn a_hung_renderer_is_reloaded() {
        let mut p = CrashPolicy::new(None);
        assert_eq!(
            p.failed(Failure::RendererHung, Instant::now(), 1000),
            Recovery::Reload
        );
    }

    #[test]
    fn a_helper_process_failing_needs_nothing() {
        let mut p = CrashPolicy::new(None);
        assert_eq!(
            p.failed(Failure::Helper, Instant::now(), 1000),
            Recovery::Nothing
        );
    }

    #[test]
    fn a_page_that_keeps_dying_is_given_up_on() {
        let mut p = CrashPolicy::new(None);
        let t0 = Instant::now();
        for i in 0..MAX_RELOADS as u64 {
            let now = t0 + Duration::from_secs(i);
            assert_eq!(p.failed(Failure::RendererGone, now, 1000), Recovery::Reload);
        }
        let now = t0 + Duration::from_secs(MAX_RELOADS as u64);
        assert_eq!(p.failed(Failure::RendererHung, now, 1000), Recovery::GiveUp);
    }

    #[test]
    fn reloads_long_ago_dont_count() {
        let mut p = CrashPolicy::new(None);
        let t0 = Instant::now();
        for _ in 0..MAX_RELOADS {
            p.failed(Failure::RendererGone, t0, 1000);
        }
        let later = t0 + RELOAD_WINDOW;
        assert_eq!(
            p.failed(Failure::RendererGone, later, 1000),
            Recovery::Reload
        );
    }

    #[test]
    fn a_dead_browser_process_restarts_pulse() {
        let mut p = CrashPolicy::new(None);
        assert_eq!(
            p.failed(Failure::BrowserGone, Instant::now(), 1000),
            Recovery::Restart
        );
    }

    #[test]
    fn a_restart_that_fails_again_soon_isnt_repeated() {
        let mut p = CrashPolicy::new(Some(1000));
        assert_eq!(
            p.failed(Failure::BrowserGone, Instant::now(), 1030),
            Recovery::GiveUp
        );
    }

    #[test]
    fn a_restart_long_ago_doesnt_stop_another() {
        let mut p = CrashPolicy::new(Some(1000));
        let now = 1000 + RESTART_GUARD.as_secs();
        assert_eq!(
            p.failed(Failure::BrowserGone, Instant::now(), now),
            Recovery::Restart
        );
    }

    #[test]
    fn the_restart_time_is_read_from_the_environment() {
        // SAFETY: test-only; no other test reads this variable.
        unsafe { std::env::set_var(RESTARTED_ENV, "1234") };
        assert_eq!(restarted_at(), Some(1234));
        unsafe { std::env::set_var(RESTARTED_ENV, "nonsense") };
        assert_eq!(restarted_at(), None);
        unsafe { std::env::remove_var(RESTARTED_ENV) };
    }
}
