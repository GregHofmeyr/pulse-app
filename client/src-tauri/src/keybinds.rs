//! Keybinds: which key combination does what. Stored per profile as W3C key codes ("KeyM", "F13"):
//! the physical key, so a bind survives a keyboard layout switch. Windows listens for them globally
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

/// What can be bound; `None` = unbound. Nothing is bound by default: everyone picks their own.
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

/// The lone-modifier keys and how they read on their own.
const MODIFIERS: [(&str, &str); 8] = [
    ("ControlLeft", "Left Ctrl"),
    ("ControlRight", "Right Ctrl"),
    ("ShiftLeft", "Left Shift"),
    ("ShiftRight", "Right Shift"),
    ("AltLeft", "Left Alt"),
    ("AltRight", "Right Alt"),
    ("MetaLeft", "Left Win"),
    ("MetaRight", "Right Win"),
];

impl Bind {
    /// A lone modifier doesn't also need itself held: Right Ctrl recorded as "Ctrl + Right Ctrl"
    /// becomes plain Right Ctrl.
    pub fn normalized(mut self) -> Bind {
        match self.code.trim_end_matches("Left").trim_end_matches("Right") {
            "Control" => self.ctrl = false,
            "Shift" => self.shift = false,
            "Alt" => self.alt = false,
            "Meta" => self.meta = false,
            _ => {}
        }
        self
    }
}

impl Keybinds {
    fn normalized(self) -> Keybinds {
        Keybinds {
            toggle_mute: self.toggle_mute.map(Bind::normalized),
            toggle_deafen: self.toggle_deafen.map(Bind::normalized),
        }
    }
}

/// "Ctrl+Shift+M", "Right Ctrl", "Mouse 3": for messages (the UI has its own, richer labels).
pub fn label(b: &Bind) -> String {
    let key = if let Some((_, name)) = MODIFIERS.iter().find(|(c, _)| *c == b.code) {
        (*name).to_string()
    } else if let Some(n) = b.code.strip_prefix("Mouse") {
        format!("Mouse {n}")
    } else {
        b.code
            .strip_prefix("Key")
            .or_else(|| b.code.strip_prefix("Digit"))
            .unwrap_or(&b.code)
            .to_string()
    };
    let mut parts = Vec::new();
    if b.ctrl {
        parts.push("Ctrl".to_string());
    }
    if b.shift {
        parts.push("Shift".to_string());
    }
    if b.alt {
        parts.push("Alt".to_string());
    }
    if b.meta {
        parts.push("Win".to_string());
    }
    parts.push(key);
    parts.join("+")
}

/// Why `binds` can't be used, if anything. Binds pass through (the key keeps working in every
/// app), so nearly anything goes: only Esc (it cancels recording) and left/right click are out.
pub fn validate(binds: &Keybinds) -> Result<(), String> {
    let bound = binds.bound();
    for (action, b) in &bound {
        let (name, key) = (action.label(), label(b));
        match b.code.as_str() {
            "" => return Err(format!("{name}: add a key")),
            "Escape" => return Err(format!("{name}: Esc is used to cancel; pick another key")),
            "Mouse1" | "Mouse2" => {
                return Err(format!("{name}: left and right click can't be binds"));
            }
            code if !crate::bindmatch::is_known(code) => {
                return Err(format!("{name}: {key} isn't a key Pulse knows"));
            }
            _ => {}
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
    let k = match serde_json::from_str::<Keybinds>(&text) {
        Ok(k) => k.normalized(),
        Err(e) => {
            tracing::warn!(error = %e, "keybinds file unreadable; nothing bound");
            return Keybinds::default();
        }
    };
    // a bind this version can't use drops only itself (v1 allowed keys v2 doesn't know)
    let usable = |b: Option<Bind>| {
        b.filter(|b| {
            let alone = Keybinds {
                toggle_mute: Some(b.clone()),
                toggle_deafen: None,
            };
            validate(&alone)
                .inspect_err(|e| tracing::warn!(error = %e, "keybind dropped"))
                .is_ok()
        })
    };
    let mut k = Keybinds {
        toggle_mute: usable(k.toggle_mute),
        toggle_deafen: usable(k.toggle_deafen),
    };
    if validate(&k).is_err() {
        k.toggle_deafen = None; // both on one bind: the first keeps it
    }
    k
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
    /// Why binds aren't working (the Windows key hooks couldn't start), if they aren't.
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

/// What a settings change did: the binds now in use, and why they aren't working, if they aren't.
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

/// Validate, save and use `binds`. An invalid set is refused (nothing changes).
#[tauri::command]
pub fn set_keybinds(
    app: tauri::AppHandle,
    store: tauri::State<'_, Store>,
    binds: Keybinds,
) -> Result<Applied, String> {
    let binds = binds.normalized();
    validate(&binds)?;
    save(&store.dir, &binds).map_err(|e| format!("couldn't save keybinds: {e}"))?;
    *store.binds.lock().unwrap_or_else(|e| e.into_inner()) = binds.clone();
    let problems = crate::hotkeys::apply(&app, &binds);
    tracing::info!(?problems, "keybinds changed");
    store.set_problems(problems);
    Ok(store.applied())
}

/// Recording a bind: binds don't fire meanwhile (pressing the new key mustn't toggle anything).
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

    /// Binds pass through (the key keeps working everywhere), so anything goes: binding a key
    /// you type with is the user's call.
    #[test]
    fn any_key_may_stand_alone() {
        for (code, mods) in [
            ("PageDown", ""),
            ("KeyM", ""),
            ("Space", ""),
            ("Numpad3", ""),
            ("F13", ""),
            ("ControlRight", ""),
            ("MetaLeft", ""),
            ("Mouse3", ""),
            ("Mouse5", ""),
            ("KeyM", "w"),
        ] {
            assert!(check(b(code, mods)).is_ok(), "{code}+{mods}");
        }
    }

    #[test]
    fn escape_and_left_right_click_are_refused() {
        for code in ["Escape", "Mouse1", "Mouse2"] {
            let e = check(b(code, "")).unwrap_err();
            assert!(e.contains("Esc") || e.contains("click"), "{code}: {e}");
        }
    }

    #[test]
    fn unknown_codes_are_refused() {
        for code in ["Banana", "Mouse6"] {
            assert!(check(b(code, "")).is_err(), "{code}");
        }
    }

    #[test]
    fn a_lone_modifier_drops_its_own_flag() {
        assert!(!b("ControlRight", "c").normalized().ctrl);
        assert!(!b("MetaLeft", "w").normalized().meta);
        assert!(b("KeyM", "c").normalized().ctrl);
    }

    /// v1 files (combos with modifiers) keep working after the update.
    #[test]
    fn old_binds_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let v1 = Keybinds {
            toggle_mute: Some(b("KeyM", "cs")),
            toggle_deafen: Some(b("F13", "ca")),
        };
        save(dir.path(), &v1).unwrap();
        assert_eq!(load(dir.path()), v1);
    }

    /// One bind this version can't use (v1 allowed MediaPlay) mustn't cost the other one.
    #[test]
    fn an_unusable_bind_only_drops_itself() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(FILE),
            r#"{"toggle_mute":{"code":"MediaPlay"},"toggle_deafen":{"code":"F13"}}"#,
        )
        .unwrap();
        let k = load(dir.path());
        assert_eq!(k.toggle_mute, None);
        assert_eq!(k.toggle_deafen, Some(b("F13", "")));
        // two actions on one bind: the first keeps it
        std::fs::write(
            dir.path().join(FILE),
            r#"{"toggle_mute":{"code":"F13"},"toggle_deafen":{"code":"F13"}}"#,
        )
        .unwrap();
        let k = load(dir.path());
        assert_eq!((k.toggle_mute, k.toggle_deafen), (Some(b("F13", "")), None));
    }

    #[test]
    fn labels() {
        assert_eq!(label(&b("Mouse3", "")), "Mouse 3");
        assert_eq!(label(&b("ControlRight", "")), "Right Ctrl");
        assert_eq!(label(&b("ShiftLeft", "")), "Left Shift");
        assert_eq!(label(&b("KeyM", "cs")), "Ctrl+Shift+M");
        assert_eq!(label(&b("Mouse4", "c")), "Ctrl+Mouse 4");
    }

    #[test]
    fn a_bind_needs_a_real_key() {
        assert!(check(b("", "c")).unwrap_err().contains("add a key"));
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
    fn startup_problems_are_kept_for_the_settings_pane() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::load(dir.path());
        store.set_problems(vec![
            "Keybinds couldn't start (access denied); restart Pulse to try again".into(),
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
            r#"{"toggle_mute":{"code":"Escape"},"toggle_deafen":null}"#,
        )
        .unwrap();
        assert_eq!(load(dir.path()), Keybinds::default());
        // a missing action reads as unbound (an older file)
        std::fs::write(dir.path().join(FILE), r#"{"toggle_mute":{"code":"F13"}}"#).unwrap();
        assert_eq!(load(dir.path()).toggle_deafen, None);
    }
}
