//! Global hotkeys from the keybinds (Windows; on Linux the compositor binds the CLI, see ipc.rs).
//! Always on, like Discord: people mute before joining, or tap mute to hear that their sound
//! works. Paused while a bind is being recorded, or Windows would swallow the keys being recorded.

use std::sync::Mutex;

use crate::keybinds::{Action, Bind, Keybinds, label};

/// The OS side: turn a bind into a key and (un)register it.
pub trait Registrar {
    type Key: Clone + PartialEq;
    fn key(&self, b: &Bind) -> Option<Self::Key>;
    /// False if it couldn't be registered (another app owns it).
    fn register(&self, k: &Self::Key) -> bool;
    fn unregister(&self, k: &Self::Key);
}

/// Which binds are registered, and the rules for changing them.
pub struct Hotkeys<K> {
    /// What's registered now. Read by the shortcut handler (Windows: main thread); registering
    /// also hops to the main thread, so this is never held while (un)registering.
    active: Mutex<Vec<(K, Action)>>,
    /// The binds wanted, and whether we're paused. Serialises changes.
    wanted: Mutex<(Option<Keybinds>, bool)>,
}

impl<K: Clone + PartialEq> Default for Hotkeys<K> {
    fn default() -> Self {
        Self {
            active: Mutex::new(Vec::new()),
            wanted: Mutex::new((None, false)),
        }
    }
}

impl<K: Clone + PartialEq> Hotkeys<K> {
    /// The action a pressed key is bound to.
    pub fn action_for(&self, k: &K) -> Option<Action> {
        let active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        active
            .iter()
            .find(|(a, _)| a == k)
            .map(|(_, action)| *action)
    }

    fn unregister_all(&self, r: &impl Registrar<Key = K>) {
        let old = std::mem::take(&mut *self.active.lock().unwrap_or_else(|e| e.into_inner()));
        for (k, _) in old {
            r.unregister(&k);
        }
    }

    fn register_all(&self, r: &impl Registrar<Key = K>, binds: &Keybinds) -> Vec<String> {
        let (mut ok, mut problems) = (Vec::new(), Vec::new());
        for (action, b) in binds.bound() {
            match r.key(b) {
                Some(k) if r.register(&k) => ok.push((k, action)),
                Some(_) => problems.push(format!(
                    "{}: {} is taken by another app",
                    action.label(),
                    label(b)
                )),
                None => problems.push(format!(
                    "{}: {} can't be used as a global key",
                    action.label(),
                    label(b)
                )),
            }
        }
        *self.active.lock().unwrap_or_else(|e| e.into_inner()) = ok;
        problems
    }

    /// Use `binds` from now on; one message per bind that couldn't be registered. While paused
    /// (recording) they're registered on unpause instead.
    pub fn apply(&self, r: &impl Registrar<Key = K>, binds: &Keybinds) -> Vec<String> {
        let mut wanted = self.wanted.lock().unwrap_or_else(|e| e.into_inner());
        wanted.0 = Some(binds.clone());
        self.unregister_all(r);
        if wanted.1 {
            return Vec::new();
        }
        self.register_all(r, binds)
    }

    pub fn pause(&self, r: &impl Registrar<Key = K>, paused: bool) -> Vec<String> {
        let mut wanted = self.wanted.lock().unwrap_or_else(|e| e.into_inner());
        if wanted.1 == paused {
            return Vec::new();
        }
        wanted.1 = paused;
        self.unregister_all(r);
        match (&wanted.0, paused) {
            (Some(binds), false) => self.register_all(r, binds),
            _ => Vec::new(),
        }
    }
}

#[cfg(windows)]
mod platform {
    use tauri::{AppHandle, Manager};
    use tauri_plugin_global_shortcut::{
        Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
    };

    use super::{Hotkeys, Registrar};
    use crate::keybinds::{Bind, Keybinds};

    struct Plugin<'a>(&'a AppHandle);

    impl Registrar for Plugin<'_> {
        type Key = Shortcut;

        fn key(&self, b: &Bind) -> Option<Shortcut> {
            let code: Code = b.code.parse().ok()?;
            let mut mods = Modifiers::empty();
            mods.set(Modifiers::CONTROL, b.ctrl);
            mods.set(Modifiers::SHIFT, b.shift);
            mods.set(Modifiers::ALT, b.alt);
            mods.set(Modifiers::SUPER, b.meta);
            Some(Shortcut::new(Some(mods), code))
        }

        fn register(&self, k: &Shortcut) -> bool {
            self.0.global_shortcut().register(*k).is_ok()
        }

        fn unregister(&self, k: &Shortcut) {
            let _ = self.0.global_shortcut().unregister(*k);
        }
    }

    type State = Hotkeys<Shortcut>;

    /// Returns the binds that couldn't be registered.
    pub fn install(app: &tauri::App, binds: &Keybinds) -> tauri::Result<Vec<String>> {
        app.handle().plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, sc, ev| {
                    if ev.state() != ShortcutState::Pressed {
                        return;
                    }
                    if let Some(a) = app.state::<State>().action_for(sc) {
                        crate::commands::hotkey(app, a.command());
                    }
                })
                .build(),
        )?;
        app.manage(State::default());
        Ok(apply(app.handle(), binds))
    }

    pub fn apply(app: &AppHandle, binds: &Keybinds) -> Vec<String> {
        app.state::<State>().apply(&Plugin(app), binds)
    }

    pub fn pause(app: &AppHandle, paused: bool) -> Vec<String> {
        app.state::<State>().pause(&Plugin(app), paused)
    }
}

#[cfg(not(windows))]
mod platform {
    use crate::keybinds::Keybinds;

    pub fn install(_app: &tauri::App, _binds: &Keybinds) -> tauri::Result<Vec<String>> {
        Ok(Vec::new())
    }

    pub fn apply(_app: &tauri::AppHandle, _binds: &Keybinds) -> Vec<String> {
        Vec::new()
    }

    pub fn pause(_app: &tauri::AppHandle, _paused: bool) -> Vec<String> {
        Vec::new()
    }
}

pub use platform::{apply, install, pause};

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Records what's registered; `taken` keys belong to another app.
    #[derive(Default)]
    struct Fake {
        registered: RefCell<Vec<String>>,
        taken: Vec<String>,
    }

    impl Registrar for Fake {
        type Key = String;
        fn key(&self, b: &Bind) -> Option<String> {
            Some(label(b))
        }
        fn register(&self, k: &String) -> bool {
            if self.taken.contains(k) {
                return false;
            }
            self.registered.borrow_mut().push(k.clone());
            true
        }
        fn unregister(&self, k: &String) {
            self.registered.borrow_mut().retain(|x| x != k);
        }
    }

    fn f13() -> Keybinds {
        Keybinds {
            toggle_mute: Some(Bind {
                code: "F13".into(),
                ctrl: false,
                shift: false,
                alt: false,
                meta: false,
            }),
            toggle_deafen: None,
        }
    }

    #[test]
    fn applying_replaces_the_registered_binds() {
        let (h, r) = (Hotkeys::default(), Fake::default());
        h.apply(&r, &Keybinds::default());
        assert_eq!(*r.registered.borrow(), ["Ctrl+Shift+M", "Ctrl+Shift+D"]);
        h.apply(&r, &f13());
        assert_eq!(*r.registered.borrow(), ["F13"]);
        assert_eq!(h.action_for(&"F13".to_string()), Some(Action::ToggleMute));
        assert_eq!(h.action_for(&"Ctrl+Shift+M".to_string()), None);
    }

    #[test]
    fn a_bind_another_app_owns_is_reported_and_the_rest_still_work() {
        let h = Hotkeys::default();
        let r = Fake {
            taken: vec!["Ctrl+Shift+M".into()],
            ..Default::default()
        };
        let problems = h.apply(&r, &Keybinds::default());
        assert_eq!(
            problems,
            ["Toggle mute: Ctrl+Shift+M is taken by another app"]
        );
        assert_eq!(*r.registered.borrow(), ["Ctrl+Shift+D"]);
    }

    /// Recording: everything's released, a new bind saved meanwhile isn't registered until the
    /// recording ends (it would swallow the next keypress), then the newest binds come back.
    #[test]
    fn while_paused_nothing_is_registered_and_unpause_brings_the_latest_back() {
        let (h, r) = (Hotkeys::default(), Fake::default());
        h.apply(&r, &Keybinds::default());
        h.pause(&r, true);
        assert!(r.registered.borrow().is_empty());
        h.apply(&r, &f13());
        assert!(r.registered.borrow().is_empty(), "still recording");
        h.pause(&r, false);
        assert_eq!(*r.registered.borrow(), ["F13"]);
        h.pause(&r, false); // a second unpause changes nothing
        assert_eq!(*r.registered.borrow(), ["F13"]);
    }
}
