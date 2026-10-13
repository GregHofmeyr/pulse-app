//! Global hotkeys from the keybinds (Windows; on Linux the compositor binds the CLI, see ipc.rs).
//! Always on, like Discord: people mute before joining, or tap mute to hear that their sound
//! works. Paused while a bind is being recorded, or Windows would swallow the keys being recorded.

#[cfg(windows)]
mod platform {
    use std::sync::Mutex;

    use tauri::{AppHandle, Manager};
    use tauri_plugin_global_shortcut::{
        Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
    };

    use crate::keybinds::{Action, Bind, Keybinds, label};

    /// What's registered now. The shortcut handler runs on the main thread and reads this, and
    /// registering hops to the main thread too: so this is never held while (un)registering.
    #[derive(Default)]
    struct Active(Mutex<Vec<(Shortcut, Action)>>);

    /// Serialises changes (apply / pause), so two can't interleave their (un)registering.
    #[derive(Default)]
    struct Changing(Mutex<Option<Keybinds>>);

    fn shortcut(b: &Bind) -> Option<Shortcut> {
        let code: Code = b.code.parse().ok()?;
        let mut mods = Modifiers::empty();
        mods.set(Modifiers::CONTROL, b.ctrl);
        mods.set(Modifiers::SHIFT, b.shift);
        mods.set(Modifiers::ALT, b.alt);
        mods.set(Modifiers::SUPER, b.meta);
        Some(Shortcut::new(Some(mods), code))
    }

    pub fn install(app: &tauri::App, binds: &Keybinds) -> tauri::Result<()> {
        app.handle().plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, sc, ev| {
                    if ev.state() != ShortcutState::Pressed {
                        return;
                    }
                    let action = app
                        .state::<Active>()
                        .0
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .iter()
                        .find(|(s, _)| s == sc)
                        .map(|(_, a)| *a);
                    if let Some(a) = action {
                        crate::commands::hotkey(app, a.command());
                    }
                })
                .build(),
        )?;
        app.manage(Active::default());
        app.manage(Changing::default());
        for problem in apply(app.handle(), binds) {
            tracing::warn!(%problem, "hotkey");
        }
        Ok(())
    }

    fn unregister_all(app: &AppHandle) {
        let old: Vec<_> = std::mem::take(
            &mut *app
                .state::<Active>()
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        let gs = app.global_shortcut();
        for (sc, _) in old {
            let _ = gs.unregister(sc);
        }
    }

    fn register_all(app: &AppHandle, binds: &Keybinds) -> Vec<String> {
        let gs = app.global_shortcut();
        let (mut ok, mut problems) = (Vec::new(), Vec::new());
        for (action, b) in binds.bound() {
            match shortcut(b) {
                None => problems.push(format!(
                    "{}: {} can't be a global key",
                    action.label(),
                    label(b)
                )),
                Some(sc) => match gs.register(sc) {
                    Ok(()) => ok.push((sc, action)),
                    // another app (or Windows) already owns it
                    Err(_) => problems.push(format!(
                        "{}: {} is taken by another app",
                        action.label(),
                        label(b)
                    )),
                },
            }
        }
        *app.state::<Active>()
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = ok;
        problems
    }

    /// Replace the registered binds; one message per bind that couldn't be registered.
    pub fn apply(app: &AppHandle, binds: &Keybinds) -> Vec<String> {
        let changing = app.state::<Changing>();
        let mut current = changing.0.lock().unwrap_or_else(|e| e.into_inner());
        unregister_all(app);
        *current = Some(binds.clone());
        register_all(app, binds)
    }

    /// While a bind is being recorded, release them all (Windows would swallow the keypress).
    pub fn pause(app: &AppHandle, paused: bool) {
        let changing = app.state::<Changing>();
        let current = changing.0.lock().unwrap_or_else(|e| e.into_inner());
        unregister_all(app);
        if !paused && let Some(binds) = current.as_ref() {
            for problem in register_all(app, binds) {
                tracing::warn!(%problem, "hotkey");
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use crate::keybinds::Keybinds;

    pub fn install(_app: &tauri::App, _binds: &Keybinds) -> tauri::Result<()> {
        Ok(())
    }

    pub fn apply(_app: &tauri::AppHandle, _binds: &Keybinds) -> Vec<String> {
        Vec::new()
    }

    pub fn pause(_app: &tauri::AppHandle, _paused: bool) {}
}

pub use platform::{apply, install, pause};
