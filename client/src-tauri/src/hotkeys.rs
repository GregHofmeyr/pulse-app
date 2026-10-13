//! Windows global hotkeys (Ctrl+Shift+M / Ctrl+Shift+D). A global hotkey takes the combo away from
//! every other app, so they're only held while in a voice channel. (Linux uses `ipc` instead.)

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the hotkeys are held; `set` reports only real changes, so voice state events (which
/// repeat) don't re-register.
#[derive(Default)]
pub struct Held(AtomicBool);

impl Held {
    pub fn set(&self, on: bool) -> bool {
        self.0.swap(on, Ordering::SeqCst) != on
    }
}

#[cfg(windows)]
mod platform {
    use tauri::{AppHandle, Manager};
    use tauri_plugin_global_shortcut::{
        Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
    };

    fn mute() -> Shortcut {
        Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyM)
    }

    fn deafen() -> Shortcut {
        Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyD)
    }

    pub fn install(app: &tauri::App) -> tauri::Result<()> {
        app.handle().plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, sc, ev| {
                    if ev.state() == ShortcutState::Pressed {
                        if sc == &mute() {
                            crate::commands::hotkey(app, crate::ipc::Command::ToggleMute);
                        } else if sc == &deafen() {
                            crate::commands::hotkey(app, crate::ipc::Command::ToggleDeafen);
                        }
                    }
                })
                .build(),
        )?;
        app.manage(super::Held::default());
        Ok(())
    }

    pub fn in_voice(app: &AppHandle, on: bool) {
        if !app.state::<super::Held>().set(on) {
            return;
        }
        let gs = app.global_shortcut();
        for sc in [mute(), deafen()] {
            // Another app may already own these: log and carry on.
            let r = if on {
                gs.register(sc)
            } else {
                gs.unregister(sc)
            };
            if let Err(e) = r {
                tracing::warn!(shortcut = ?sc, on, error = %e, "hotkey change failed");
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn install(_app: &tauri::App) -> tauri::Result<()> {
        Ok(())
    }

    pub fn in_voice(_app: &tauri::AppHandle, _on: bool) {}
}

pub use platform::{in_voice, install};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_changes_count() {
        let h = Held::default();
        assert!(!h.set(false), "starts released");
        assert!(h.set(true));
        assert!(!h.set(true), "repeated voice state: no re-register");
        assert!(h.set(false));
    }
}
