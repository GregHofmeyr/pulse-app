//! Keybinds: which key combination does what. Stored per profile as W3C key codes ("KeyM", "F13"):
//! the physical key, so a bind survives a keyboard layout switch. Windows registers them globally
//! (hotkeys.rs); on Linux the compositor binds the CLI instead (ipc.rs).

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bind {
    pub code: String,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub meta: bool,
}

/// What can be bound. `None` binds are unbound (cleared by the user).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keybinds {
    pub toggle_mute: Option<Bind>,
    pub toggle_deafen: Option<Bind>,
}

impl Default for Keybinds {
    fn default() -> Self {
        let ctrl_shift = |code: &str| Bind {
            code: code.into(),
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        };
        Self {
            toggle_mute: Some(ctrl_shift("KeyM")),
            toggle_deafen: Some(ctrl_shift("KeyD")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ToggleMute,
    ToggleDeafen,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::ToggleMute => "Toggle mute",
            Self::ToggleDeafen => "Toggle deafen",
        }
    }

    pub fn command(self) -> crate::ipc::Command {
        match self {
            Self::ToggleMute => crate::ipc::Command::ToggleMute,
            Self::ToggleDeafen => crate::ipc::Command::ToggleDeafen,
        }
    }
}

impl Keybinds {
    /// Each bound action with its bind.
    pub fn bound(&self) -> Vec<(Action, &Bind)> {
        [
            (Action::ToggleMute, &self.toggle_mute),
            (Action::ToggleDeafen, &self.toggle_deafen),
        ]
        .into_iter()
        .filter_map(|(a, b)| b.as_ref().map(|b| (a, b)))
        .collect()
    }
}

const MODIFIER_CODES: [&str; 8] = [
    "ControlLeft",
    "ControlRight",
    "ShiftLeft",
    "ShiftRight",
    "AltLeft",
    "AltRight",
    "MetaLeft",
    "MetaRight",
];

/// Keys that type something: bound globally on their own, they'd stop working in every other app.
fn types_text(code: &str) -> bool {
    code.starts_with("Key")
        || code.starts_with("Digit")
        || matches!(
            code,
            "Space"
                | "Enter"
                | "Tab"
                | "Backspace"
                | "Minus"
                | "Equal"
                | "BracketLeft"
                | "BracketRight"
                | "Backslash"
                | "Semicolon"
                | "Quote"
                | "Backquote"
                | "Comma"
                | "Period"
                | "Slash"
        )
}

/// "Ctrl+Shift+M": for messages (the UI has its own, richer labels).
pub fn label(b: &Bind) -> String {
    let key = b
        .code
        .strip_prefix("Key")
        .or_else(|| b.code.strip_prefix("Digit"))
        .unwrap_or(&b.code);
    let mut parts = Vec::new();
    if b.ctrl {
        parts.push("Ctrl");
    }
    if b.shift {
        parts.push("Shift");
    }
    if b.alt {
        parts.push("Alt");
    }
    if b.meta {
        parts.push("Win");
    }
    parts.push(key);
    parts.join("+")
}

/// Why `binds` can't be used, if anything.
pub fn validate(binds: &Keybinds) -> Result<(), String> {
    let bound = binds.bound();
    for (action, b) in &bound {
        if b.code.is_empty() || MODIFIER_CODES.contains(&b.code.as_str()) {
            return Err(format!("{}: add a key, not just modifiers", action.label()));
        }
        let modified = b.ctrl || b.alt || b.meta;
        if types_text(&b.code) && !modified {
            return Err(format!(
                "{}: {} on its own would stop that key typing in other apps; add Ctrl or Alt",
                action.label(),
                label(b)
            ));
        }
    }
    for (i, (_, b)) in bound.iter().enumerate() {
        if let Some((other, _)) = bound[..i].iter().find(|(_, o)| o == b) {
            return Err(format!(
                "{} is already used for {}",
                label(b),
                other.label().to_lowercase()
            ));
        }
    }
    Ok(())
}

const FILE: &str = "keybinds.json";

/// The profile's binds; the defaults if none are saved yet (or the file is unreadable).
pub fn load(dir: &Path) -> Keybinds {
    std::fs::read_to_string(dir.join(FILE))
        .ok()
        .and_then(|s| serde_json::from_str::<Keybinds>(&s).ok())
        .filter(|k| validate(k).is_ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, binds: &Keybinds) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(binds).map_err(std::io::Error::other)?;
    std::fs::write(dir.join(FILE), json)
}

/// The profile's binds, as the app is using them.
pub struct Store {
    dir: std::path::PathBuf,
    binds: std::sync::Mutex<Keybinds>,
}

impl Store {
    pub fn load(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            binds: std::sync::Mutex::new(load(dir)),
        }
    }

    pub fn get(&self) -> Keybinds {
        self.binds.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// What a settings change did: the binds now in use, and any that couldn't be registered.
#[derive(Serialize)]
pub struct Applied {
    pub binds: Keybinds,
    pub problems: Vec<String>,
}

#[tauri::command]
pub fn get_keybinds(store: tauri::State<'_, Store>) -> Keybinds {
    store.get()
}

/// Validate, save and register `binds`. An invalid set is refused (nothing changes); a bind
/// another app owns is saved anyway (it may free up) and reported.
#[tauri::command]
pub fn set_keybinds(
    app: tauri::AppHandle,
    store: tauri::State<'_, Store>,
    binds: Keybinds,
) -> Result<Applied, String> {
    validate(&binds)?;
    save(&store.dir, &binds).map_err(|e| format!("couldn't save keybinds: {e}"))?;
    *store.binds.lock().unwrap_or_else(|e| e.into_inner()) = binds.clone();
    let problems = crate::hotkeys::apply(&app, &binds);
    tracing::info!(?problems, "keybinds changed");
    Ok(Applied { binds, problems })
}

/// Recording a bind: release the global hotkeys meanwhile (Windows would swallow the keypress).
#[tauri::command]
pub fn pause_keybinds(app: tauri::AppHandle, paused: bool) {
    crate::hotkeys::pause(&app, paused);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind(code: &str, ctrl: bool, shift: bool) -> Option<Bind> {
        Some(Bind {
            code: code.into(),
            ctrl,
            shift,
            alt: false,
            meta: false,
        })
    }

    #[test]
    fn the_defaults_are_ctrl_shift_m_and_d() {
        let k = Keybinds::default();
        assert_eq!(label(k.toggle_mute.as_ref().unwrap()), "Ctrl+Shift+M");
        assert_eq!(label(k.toggle_deafen.as_ref().unwrap()), "Ctrl+Shift+D");
        assert!(validate(&k).is_ok());
    }

    #[test]
    fn function_keys_may_stand_alone_but_typing_keys_may_not() {
        let k = Keybinds {
            toggle_mute: bind("F13", false, false),
            toggle_deafen: bind("Pause", false, false),
        };
        assert!(validate(&k).is_ok());
        let k = Keybinds {
            toggle_mute: bind("KeyM", false, false),
            toggle_deafen: None,
        };
        let e = validate(&k).unwrap_err();
        assert!(e.contains("Toggle mute") && e.contains("M"), "{e}");
        // Shift alone still types (a capital): not enough
        let k = Keybinds {
            toggle_mute: bind("KeyM", false, true),
            toggle_deafen: None,
        };
        assert!(validate(&k).is_err());
    }

    #[test]
    fn a_bind_needs_a_real_key() {
        let k = Keybinds {
            toggle_mute: bind("ControlLeft", true, false),
            toggle_deafen: None,
        };
        assert!(validate(&k).unwrap_err().contains("not just modifiers"));
    }

    #[test]
    fn two_actions_cant_share_a_bind() {
        let k = Keybinds {
            toggle_mute: bind("KeyM", true, true),
            toggle_deafen: bind("KeyM", true, true),
        };
        assert_eq!(
            validate(&k).unwrap_err(),
            "Ctrl+Shift+M is already used for toggle mute"
        );
    }

    #[test]
    fn unbound_actions_are_fine() {
        let k = Keybinds {
            toggle_mute: None,
            toggle_deafen: None,
        };
        assert!(validate(&k).is_ok());
        assert!(k.bound().is_empty());
    }

    #[test]
    fn binds_are_saved_per_profile_and_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Keybinds::default(), "nothing saved yet");
        let mine = Keybinds {
            toggle_mute: bind("F13", false, false),
            toggle_deafen: None,
        };
        save(dir.path(), &mine).unwrap();
        assert_eq!(load(dir.path()), mine);
        // a broken file falls back to the defaults instead of leaving nothing bound
        std::fs::write(dir.path().join(FILE), "{nope").unwrap();
        assert_eq!(load(dir.path()), Keybinds::default());
    }
}
