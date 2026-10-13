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

/// What can be bound; `None` = unbound. Nothing is bound by default (Greg 2026-10-13): a global
/// bind takes its keys away from every app, so everyone picks their own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keybinds {
    pub toggle_mute: Option<Bind>,
    pub toggle_deafen: Option<Bind>,
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

/// Keys Windows global hotkeys can register (global-hotkey 0.8's `key_to_vk`), minus the ones it
/// maps to a different key (NumpadEqual → E, MediaPause → Pause).
fn registrable(code: &str) -> bool {
    let named = [
        "Backquote",
        "Backslash",
        "BracketLeft",
        "BracketRight",
        "Comma",
        "Equal",
        "Minus",
        "Period",
        "Quote",
        "Semicolon",
        "Slash",
        "Space",
        "Tab",
        "Enter",
        "Backspace",
        "Escape",
        "CapsLock",
        "NumLock",
        "ScrollLock",
        "PrintScreen",
        "Pause",
        "Insert",
        "Delete",
        "Home",
        "End",
        "PageUp",
        "PageDown",
        "ArrowUp",
        "ArrowDown",
        "ArrowLeft",
        "ArrowRight",
        "NumpadAdd",
        "NumpadSubtract",
        "NumpadMultiply",
        "NumpadDivide",
        "NumpadDecimal",
        "NumpadEnter",
        "MediaPlay",
        "MediaPlayPause",
        "MediaStop",
        "MediaTrackNext",
        "MediaTrackPrevious",
        "AudioVolumeUp",
        "AudioVolumeDown",
        "AudioVolumeMute",
    ];
    let one_char = |prefix: &str, ok: fn(char) -> bool| {
        code.strip_prefix(prefix)
            .is_some_and(|r| r.len() == 1 && r.chars().all(ok))
    };
    let f_key = code
        .strip_prefix('F')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n));
    named.contains(&code)
        || one_char("Key", |c| c.is_ascii_uppercase())
        || one_char("Digit", |c| c.is_ascii_digit())
        || one_char("Numpad", |c| c.is_ascii_digit())
        || f_key
}

/// Letters, digits and punctuation: these type text.
fn types_text(code: &str) -> bool {
    code.starts_with("Key")
        || code.starts_with("Digit")
        || matches!(
            code,
            "Space"
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

/// Keys every app relies on beyond typing: editing, moving around, the numpad, the locks.
fn everyday(code: &str) -> bool {
    code.starts_with("Numpad")
        || code.starts_with("Arrow")
        || matches!(
            code,
            "Enter"
                | "Tab"
                | "Backspace"
                | "Escape"
                | "Insert"
                | "Delete"
                | "Home"
                | "End"
                | "PageUp"
                | "PageDown"
                | "CapsLock"
                | "NumLock"
                | "ScrollLock"
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

/// Why `binds` can't be used, if anything. A global hotkey takes its combination away from every
/// other app, so the rules keep binds off anything people type or rely on elsewhere.
pub fn validate(binds: &Keybinds) -> Result<(), String> {
    let bound = binds.bound();
    for (action, b) in &bound {
        let (name, key) = (action.label(), label(b));
        if b.code.is_empty() || MODIFIER_CODES.contains(&b.code.as_str()) {
            return Err(format!("{name}: add a key, not just modifiers"));
        }
        if !registrable(&b.code) || (b.code == "Pause" && b.ctrl) {
            return Err(format!(
                "{name}: {key} can't be used as a global key; try another"
            ));
        }
        if b.meta {
            return Err(format!(
                "{name}: Windows keeps most Win-key shortcuts for itself; use Ctrl, Alt or Shift"
            ));
        }
        if types_text(&b.code) || everyday(&b.code) {
            if !(b.ctrl || b.alt) {
                return Err(format!(
                    "{name}: {key} would stop working in every other app; add Ctrl or Alt"
                ));
            }
            // AltGr is Ctrl+Alt to Windows: AltGr+Q types @ on German keyboards
            if b.ctrl && b.alt && !b.shift && types_text(&b.code) {
                return Err(format!(
                    "{name}: Ctrl+Alt is AltGr on many keyboards (AltGr+Q types @); add Shift"
                ));
            }
            // Ctrl+C, Ctrl+V, Ctrl+1…: shortcuts every app uses
            if b.ctrl && !b.alt && !b.shift && types_text(&b.code) {
                return Err(format!(
                    "{name}: {key} is a shortcut other apps use; add Shift or Alt"
                ));
            }
        }
    }
    for (i, (a, x)) in bound.iter().enumerate() {
        if let Some((b, _)) = bound[i + 1..].iter().find(|(_, y)| y == x) {
            return Err(format!(
                "{} can't be both {} and {}",
                label(x),
                a.label().to_lowercase(),
                b.label().to_lowercase()
            ));
        }
    }
    Ok(())
}

const FILE: &str = "keybinds.json";

/// The profile's binds; none if nothing is saved yet (or the file is unusable).
pub fn load(dir: &Path) -> Keybinds {
    let Ok(text) = std::fs::read_to_string(dir.join(FILE)) else {
        return Keybinds::default();
    };
    match serde_json::from_str::<Keybinds>(&text)
        .map_err(|e| e.to_string())
        .and_then(|k| validate(&k).map(|_| k))
    {
        Ok(k) => k,
        Err(e) => {
            tracing::warn!(error = %e, "keybinds file unusable; nothing bound");
            Keybinds::default()
        }
    }
}

pub fn save(dir: &Path, binds: &Keybinds) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(binds).map_err(std::io::Error::other)?;
    // write then rename: a crash mid-write can't leave a half file (which would reset the binds)
    let tmp = dir.join(format!("{FILE}.tmp"));
    std::fs::write(&tmp, json)?;
    std::fs::rename(tmp, dir.join(FILE))
}

/// The profile's binds, as the app is using them.
pub struct Store {
    dir: std::path::PathBuf,
    binds: std::sync::Mutex<Keybinds>,
    /// Binds that couldn't be registered (another app owns them), from the last apply.
    problems: std::sync::Mutex<Vec<String>>,
}

impl Store {
    pub fn load(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            binds: std::sync::Mutex::new(load(dir)),
            problems: Default::default(),
        }
    }

    pub fn set_problems(&self, problems: Vec<String>) {
        *self.problems.lock().unwrap_or_else(|e| e.into_inner()) = problems;
    }

    /// What the settings pane shows: the binds, what's wrong with them, and whether the user has
    /// ever chosen (if not, the app nudges them to).
    pub fn applied(&self) -> Applied {
        Applied {
            binds: self.get(),
            problems: self
                .problems
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
            chosen: self.dir.join(FILE).exists(),
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
    pub chosen: bool,
}

#[tauri::command]
pub fn get_keybinds(store: tauri::State<'_, Store>) -> Applied {
    store.applied()
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
    store.set_problems(problems);
    Ok(store.applied())
}

/// Recording a bind: release the global hotkeys meanwhile (Windows would swallow the keypress).
#[tauri::command]
pub fn pause_keybinds(app: tauri::AppHandle, paused: bool) {
    for problem in crate::hotkeys::pause(&app, paused) {
        tracing::warn!(%problem, "hotkey");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `mods`: c = Ctrl, s = Shift, a = Alt, w = Win.
    fn b(code: &str, mods: &str) -> Bind {
        Bind {
            code: code.into(),
            ctrl: mods.contains('c'),
            shift: mods.contains('s'),
            alt: mods.contains('a'),
            meta: mods.contains('w'),
        }
    }

    fn check(bind: Bind) -> Result<(), String> {
        validate(&Keybinds {
            toggle_mute: Some(bind),
            toggle_deafen: None,
        })
    }

    /// Greg 2026-10-13: no defaults (a global bind takes its keys from every app); everyone
    /// picks their own, nudged on first run until they've chosen.
    #[test]
    fn nothing_is_bound_until_the_user_chooses() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::load(dir.path());
        assert!(store.get().bound().is_empty());
        assert!(!store.applied().chosen, "first run: nudge");
        save(dir.path(), &Keybinds::default()).unwrap(); // dismissing the nudge saves 'none'
        assert!(Store::load(dir.path()).applied().chosen);
    }

    #[test]
    fn keys_that_dont_type_may_stand_alone() {
        for code in ["F13", "F1", "Pause", "MediaPlayPause", "AudioVolumeMute"] {
            assert!(check(b(code, "")).is_ok(), "{code}");
        }
    }

    /// A global hotkey takes its key away from every app: anything that types or moves the
    /// cursor needs a modifier (NumpadEnter is Enter to Windows; the numpad types digits).
    #[test]
    fn keys_that_type_or_navigate_need_a_modifier() {
        for code in [
            "KeyM",
            "Digit1",
            "Space",
            "Enter",
            "NumpadEnter",
            "Numpad5",
            "NumpadAdd",
            "ArrowUp",
            "Home",
            "Delete",
            "Insert",
            "Backquote",
            "Escape",
            "CapsLock",
        ] {
            let e = check(b(code, "")).unwrap_err();
            assert!(
                e.contains("Toggle mute") && e.contains("add"),
                "{code}: {e}"
            );
        }
        assert!(
            check(b("KeyM", "s")).is_err(),
            "Shift+M still types a capital"
        );
        assert!(check(b("Numpad5", "a")).is_ok());
    }

    #[test]
    fn ctrl_letter_alone_is_refused_but_with_shift_is_fine() {
        // Ctrl+C / Ctrl+V… would stop working in every app
        assert!(check(b("KeyC", "c")).is_err());
        assert!(check(b("KeyC", "cs")).is_ok());
        assert!(check(b("KeyM", "a")).is_ok(), "Alt+M");
    }

    /// AltGr is Ctrl+Alt to Windows: AltGr+Q types @ on German layouts.
    #[test]
    fn ctrl_alt_with_a_typing_key_is_refused() {
        let e = check(b("KeyQ", "ca")).unwrap_err();
        assert!(e.contains("AltGr"), "{e}");
        assert!(
            check(b("KeyQ", "cas")).is_ok(),
            "Ctrl+Alt+Shift+Q types nothing"
        );
    }

    #[test]
    fn the_windows_key_is_refused() {
        assert!(check(b("KeyM", "w")).unwrap_err().contains("Windows"));
        assert!(check(b("KeyM", "cw")).is_err());
    }

    /// Keys Windows hotkeys can't register (or that act as another key) are refused up front,
    /// not saved and then reported as "taken" on every launch.
    #[test]
    fn keys_windows_hotkeys_cant_use_are_refused() {
        for code in [
            "IntlBackslash",
            "IntlRo",
            "ContextMenu",
            "BrowserBack",
            "F25",
            "NumpadEqual",
            "MediaPause",
        ] {
            let e = check(b(code, "c")).unwrap_err();
            assert!(e.contains("can't be used"), "{code}: {e}");
        }
        assert!(
            check(b("Pause", "c")).is_err(),
            "Ctrl+Pause is a different key (Break) to Windows"
        );
    }

    #[test]
    fn a_bind_needs_a_real_key() {
        assert!(
            check(b("ControlLeft", "c"))
                .unwrap_err()
                .contains("not just modifiers")
        );
    }

    #[test]
    fn two_actions_cant_share_a_bind() {
        // whichever was changed, the message names both
        let k = Keybinds {
            toggle_mute: Some(b("KeyD", "cs")),
            toggle_deafen: Some(b("KeyD", "cs")),
        };
        assert_eq!(
            validate(&k).unwrap_err(),
            "Ctrl+Shift+D can't be both toggle mute and toggle deafen"
        );
    }

    /// Teams owns Ctrl+Shift+M: the settings pane must say so after a restart too, not show the
    /// bind as if it worked.
    #[test]
    fn registration_problems_are_kept_for_the_settings_pane() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::load(dir.path());
        store.set_problems(vec![
            "Toggle mute: Ctrl+Shift+M is taken by another app".into(),
        ]);
        let shown = store.applied();
        assert_eq!(shown.problems.len(), 1);
        assert_eq!(shown.binds, Keybinds::default());
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
            toggle_mute: Some(b("F13", "")),
            toggle_deafen: None,
        };
        save(dir.path(), &mine).unwrap();
        assert_eq!(load(dir.path()), mine);
        // a broken file means nothing bound (and a warning in the log)
        std::fs::write(dir.path().join(FILE), "{nope").unwrap();
        assert_eq!(load(dir.path()), Keybinds::default());
        // so does one that parses but can't be used
        std::fs::write(
            dir.path().join(FILE),
            r#"{"toggle_mute":{"code":"KeyM"},"toggle_deafen":null}"#,
        )
        .unwrap();
        assert_eq!(load(dir.path()), Keybinds::default());
        // a missing action reads as unbound (an older file)
        std::fs::write(dir.path().join(FILE), r#"{"toggle_mute":{"code":"F13"}}"#).unwrap();
        assert_eq!(load(dir.path()).toggle_deafen, None);
    }
}
