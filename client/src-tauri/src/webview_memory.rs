//! WebView2 memory: drop to the Low memory target while Pulse is minimised or has sat in the
//! background a while, and back to Normal on focus. This is Microsoft's advice for inactive
//! webviews. It is never combined with TrySuspend, which would stop the page handling events
//! (message sounds, unread counts). The policy is plain logic; only applying it is Windows-only.

use std::time::Duration;

/// Unfocused (but not minimised) this long counts as "in the background".
pub const BACKGROUND_AFTER: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Normal,
    Low,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Nothing,
    /// Apply this target now.
    Set(Target),
    /// Call `timer_fired(generation)` after `BACKGROUND_AFTER`.
    Arm(u64),
}

/// Which target the webview should have. Every focus change starts a new generation, so a timer
/// armed before the user came back can't lower memory under them.
#[derive(Debug, Default)]
pub struct MemPolicy {
    generation: u64,
    low: bool,
}

impl MemPolicy {
    pub fn focused(&mut self) -> Action {
        self.generation += 1;
        if !self.low {
            return Action::Nothing;
        }
        self.low = false;
        Action::Set(Target::Normal)
    }

    pub fn unfocused(&mut self) -> Action {
        self.generation += 1;
        if self.low {
            Action::Nothing
        } else {
            Action::Arm(self.generation)
        }
    }

    pub fn minimised(&mut self) -> Action {
        self.generation += 1;
        self.go_low()
    }

    pub fn timer_fired(&mut self, generation: u64) -> Action {
        if generation == self.generation {
            self.go_low()
        } else {
            Action::Nothing
        }
    }

    fn go_low(&mut self) -> Action {
        if self.low {
            return Action::Nothing;
        }
        self.low = true;
        Action::Set(Target::Low)
    }
}

/// Follow the main window's focus and minimise state.
#[cfg(windows)]
pub fn install(win: &tauri::WebviewWindow) {
    use std::sync::{Arc, Mutex};
    let policy = Arc::new(Mutex::new(MemPolicy::default()));
    let w = win.clone();
    win.on_window_event(move |e| {
        let action = match e {
            tauri::WindowEvent::Focused(true) => policy.lock().unwrap().focused(),
            tauri::WindowEvent::Focused(false) => policy.lock().unwrap().unfocused(),
            // Windows reports a minimised window as resized to 0 x 0.
            tauri::WindowEvent::Resized(size) if size.width == 0 && size.height == 0 => {
                policy.lock().unwrap().minimised()
            }
            _ => return,
        };
        apply(&w, &policy, action);
    });
}

#[cfg(windows)]
fn apply(
    win: &tauri::WebviewWindow,
    policy: &std::sync::Arc<std::sync::Mutex<MemPolicy>>,
    action: Action,
) {
    match action {
        Action::Nothing => {}
        Action::Set(target) => set_target(win, target),
        Action::Arm(generation) => {
            let (win, policy) = (win.clone(), policy.clone());
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(BACKGROUND_AFTER).await;
                let next = policy.lock().unwrap().timer_fired(generation);
                apply(&win, &policy, next);
            });
        }
    }
}

#[cfg(windows)]
fn set_target(win: &tauri::WebviewWindow, target: Target) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
        ICoreWebView2_19,
    };
    use windows_core::Interface;
    let level = match target {
        Target::Low => COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
        Target::Normal => COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    };
    let sent = win.with_webview(move |wv| {
        // SAFETY: plain COM calls on the webview's own thread (with_webview runs us there).
        let set = unsafe {
            wv.controller()
                .CoreWebView2()
                .and_then(|core| core.cast::<ICoreWebView2_19>())
                .and_then(|v19| v19.SetMemoryUsageTargetLevel(level))
        };
        match set {
            Ok(()) => tracing::info!(?target, "webview memory target"),
            // ICoreWebView2_19 needs WebView2 Runtime 114+; older runtimes just stay on Normal.
            Err(e) => tracing::warn!(?target, error = %e, "webview memory target not set"),
        }
    });
    if let Err(e) = sent {
        tracing::warn!(error = %e, "webview memory target: webview unavailable");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimising_goes_low_at_once_and_focus_restores_normal() {
        let mut p = MemPolicy::default();
        assert_eq!(p.minimised(), Action::Set(Target::Low));
        assert_eq!(p.minimised(), Action::Nothing, "already low");
        assert_eq!(p.unfocused(), Action::Nothing, "already low");
        assert_eq!(p.focused(), Action::Set(Target::Normal));
        assert_eq!(p.focused(), Action::Nothing, "already normal");
    }

    #[test]
    fn the_background_goes_low_only_when_the_timer_fires() {
        let mut p = MemPolicy::default();
        let Action::Arm(g) = p.unfocused() else {
            panic!("expected a timer")
        };
        assert_eq!(p.timer_fired(g), Action::Set(Target::Low));
        assert_eq!(p.focused(), Action::Set(Target::Normal));
    }

    #[test]
    fn coming_back_before_the_timer_cancels_it() {
        let mut p = MemPolicy::default();
        let Action::Arm(g) = p.unfocused() else {
            panic!("expected a timer")
        };
        assert_eq!(p.focused(), Action::Nothing);
        assert_eq!(p.timer_fired(g), Action::Nothing, "stale timer");
        let Action::Arm(g2) = p.unfocused() else {
            panic!("expected a timer")
        };
        assert_eq!(
            p.timer_fired(g),
            Action::Nothing,
            "the first timer stays stale"
        );
        assert_eq!(p.timer_fired(g2), Action::Set(Target::Low));
    }

    #[test]
    fn minimising_after_losing_focus_does_not_wait_for_the_timer() {
        let mut p = MemPolicy::default();
        let Action::Arm(g) = p.unfocused() else {
            panic!("expected a timer")
        };
        assert_eq!(p.minimised(), Action::Set(Target::Low));
        assert_eq!(p.timer_fired(g), Action::Nothing);
    }
}
