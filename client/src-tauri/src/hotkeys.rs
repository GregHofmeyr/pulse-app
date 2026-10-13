//! Windows global hotkeys (Ctrl+Shift+M / Ctrl+Shift+D). A global hotkey takes the combo away from
//! every other app, so they're only held while in a voice channel. (Linux uses `ipc` instead.)

use std::sync::Mutex;

/// Whether the hotkeys are held. Only real changes register/unregister (voice state events
/// repeat), and the flag is locked across the change, so events racing in from several threads
/// can't leave the keys registered while the flag says released.
#[derive(Default)]
pub struct Held(Mutex<bool>);

impl Held {
    pub fn apply(&self, on: bool, change: impl FnOnce(bool)) {
        let mut held = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if *held != on {
            change(on);
            *held = on;
        }
    }

    pub fn is_held(&self) -> bool {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
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

    /// Applied on the main thread (where registering happens anyway): a caller never waits on
    /// it, so no thread can hold the flag while waiting for the main thread to register.
    pub fn in_voice(app: &AppHandle, on: bool) {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || in_voice_now(&handle, on));
    }

    fn in_voice_now(app: &AppHandle, on: bool) {
        app.state::<super::Held>().apply(on, |on| {
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
        });
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
        let calls = std::sync::Mutex::new(Vec::new());
        let rec = |on| calls.lock().unwrap().push(on);
        h.apply(false, rec);
        h.apply(true, rec);
        h.apply(true, rec); // repeated voice state: no re-register
        h.apply(false, rec);
        assert_eq!(*calls.lock().unwrap(), vec![true, false]);
    }

    /// Voice state events come from several threads: the keys actually registered must always end
    /// up matching the flag (keys held while out of voice is exactly what this module prevents).
    #[test]
    fn registration_never_drifts_from_the_flag() {
        let h = std::sync::Arc::new(Held::default());
        let registered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let (h, registered) = (h.clone(), registered.clone());
                std::thread::spawn(move || {
                    for i in 0..500 {
                        h.apply((i + t) % 2 == 0, |on| {
                            std::thread::yield_now(); // registering is slow (main thread hop)
                            registered.store(on, std::sync::atomic::Ordering::SeqCst);
                        });
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(
            registered.load(std::sync::atomic::Ordering::SeqCst),
            h.is_held()
        );
    }
}
