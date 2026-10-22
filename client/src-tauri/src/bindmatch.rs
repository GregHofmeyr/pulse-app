//! Which bind a key or mouse-button press fires. Pure (no OS calls), so it's tested on every
//! platform; on Windows the low-level hooks in hotkeys.rs feed it.

use std::time::{Duration, Instant};

use crate::keybinds::{Action, Bind, Keybinds};

/// Mouse buttons that can be bound: wheel click and the two side buttons. (Mouse1/Mouse2, left and
/// right click, are known so validation can name them, but can't be bound.)
pub const MOUSE_CODES: [&str; 3] = ["Mouse3", "Mouse4", "Mouse5"];

/// PC set-1 scan code + extended flag (as Windows low-level hooks report them) → W3C `code`, the
/// physical key, same as the webview records.
#[rustfmt::skip]
pub const KEYS: &[(u16, bool, &str)] = &[
    (0x01, false, "Escape"),
    (0x02, false, "Digit1"), (0x03, false, "Digit2"), (0x04, false, "Digit3"), (0x05, false, "Digit4"),
    (0x06, false, "Digit5"), (0x07, false, "Digit6"), (0x08, false, "Digit7"), (0x09, false, "Digit8"),
    (0x0A, false, "Digit9"), (0x0B, false, "Digit0"), (0x0C, false, "Minus"), (0x0D, false, "Equal"),
    (0x0E, false, "Backspace"), (0x0F, false, "Tab"),
    (0x10, false, "KeyQ"), (0x11, false, "KeyW"), (0x12, false, "KeyE"), (0x13, false, "KeyR"),
    (0x14, false, "KeyT"), (0x15, false, "KeyY"), (0x16, false, "KeyU"), (0x17, false, "KeyI"),
    (0x18, false, "KeyO"), (0x19, false, "KeyP"), (0x1A, false, "BracketLeft"), (0x1B, false, "BracketRight"),
    (0x1C, false, "Enter"), (0x1C, true, "NumpadEnter"),
    (0x1D, false, "ControlLeft"), (0x1D, true, "ControlRight"),
    (0x1E, false, "KeyA"), (0x1F, false, "KeyS"), (0x20, false, "KeyD"), (0x21, false, "KeyF"),
    (0x22, false, "KeyG"), (0x23, false, "KeyH"), (0x24, false, "KeyJ"), (0x25, false, "KeyK"),
    (0x26, false, "KeyL"), (0x27, false, "Semicolon"), (0x28, false, "Quote"), (0x29, false, "Backquote"),
    (0x2A, false, "ShiftLeft"), (0x2B, false, "Backslash"),
    (0x2C, false, "KeyZ"), (0x2D, false, "KeyX"), (0x2E, false, "KeyC"), (0x2F, false, "KeyV"),
    (0x30, false, "KeyB"), (0x31, false, "KeyN"), (0x32, false, "KeyM"), (0x33, false, "Comma"),
    (0x34, false, "Period"), (0x35, false, "Slash"), (0x35, true, "NumpadDivide"),
    (0x36, false, "ShiftRight"), (0x36, true, "ShiftRight"),
    (0x37, false, "NumpadMultiply"), (0x37, true, "PrintScreen"),
    (0x38, false, "AltLeft"), (0x38, true, "AltRight"),
    (0x39, false, "Space"), (0x3A, false, "CapsLock"),
    (0x3B, false, "F1"), (0x3C, false, "F2"), (0x3D, false, "F3"), (0x3E, false, "F4"),
    (0x3F, false, "F5"), (0x40, false, "F6"), (0x41, false, "F7"), (0x42, false, "F8"),
    (0x43, false, "F9"), (0x44, false, "F10"),
    (0x45, false, "Pause"), (0x45, true, "NumLock"), (0x46, false, "ScrollLock"),
    (0x46, true, "Pause"), // Ctrl+Pause arrives as Break (E0 46)
    (0x47, false, "Numpad7"), (0x47, true, "Home"),
    (0x48, false, "Numpad8"), (0x48, true, "ArrowUp"),
    (0x49, false, "Numpad9"), (0x49, true, "PageUp"),
    (0x4A, false, "NumpadSubtract"),
    (0x4B, false, "Numpad4"), (0x4B, true, "ArrowLeft"),
    (0x4C, false, "Numpad5"),
    (0x4D, false, "Numpad6"), (0x4D, true, "ArrowRight"),
    (0x4E, false, "NumpadAdd"),
    (0x4F, false, "Numpad1"), (0x4F, true, "End"),
    (0x50, false, "Numpad2"), (0x50, true, "ArrowDown"),
    (0x51, false, "Numpad3"), (0x51, true, "PageDown"),
    (0x52, false, "Numpad0"), (0x52, true, "Insert"),
    (0x53, false, "NumpadDecimal"), (0x53, true, "Delete"),
    (0x56, false, "IntlBackslash"), (0x59, false, "NumpadEqual"),
    (0x69, true, "BrowserForward"), (0x6A, true, "BrowserBack"), (0x57, false, "F11"), (0x58, false, "F12"),
    (0x5B, true, "MetaLeft"), (0x5C, true, "MetaRight"), (0x5D, true, "ContextMenu"),
    (0x64, false, "F13"), (0x65, false, "F14"), (0x66, false, "F15"), (0x67, false, "F16"),
    (0x68, false, "F17"), (0x69, false, "F18"), (0x6A, false, "F19"), (0x6B, false, "F20"),
    (0x6C, false, "F21"), (0x6D, false, "F22"), (0x6E, false, "F23"), (0x76, false, "F24"),
    (0x10, true, "MediaTrackPrevious"), (0x19, true, "MediaTrackNext"), (0x20, true, "AudioVolumeMute"),
    (0x22, true, "MediaPlayPause"), (0x24, true, "MediaStop"),
    (0x2E, true, "AudioVolumeDown"), (0x30, true, "AudioVolumeUp"),
];

pub fn code_for_scan(scan: u32, extended: bool) -> Option<&'static str> {
    KEYS.iter()
        .find(|(s, e, _)| u32::from(*s) == scan && *e == extended)
        .map(|(_, _, c)| *c)
}

/// What a low-level keyboard event is, from its virtual-key code, scan code and extended flag.
/// `vk_to_scan` (MapVirtualKeyW with MAPVK_VK_TO_VSC_EX on Windows) covers injected keys that carry
/// only a virtual-key code.
pub fn key_code(
    vk: u32,
    scan: u32,
    extended: bool,
    vk_to_scan: impl FnOnce(u32) -> u32,
) -> Option<&'static str> {
    /// Unicode text sent by apps (SendInput KEYEVENTF_UNICODE): the scan field holds a character.
    const VK_PACKET: u32 = 0xE7;
    if vk == VK_PACKET {
        return None;
    }
    if scan != 0 {
        return code_for_scan(scan, extended);
    }
    let v = vk_to_scan(vk);
    match v & 0xFF00 {
        0xE100 => Some("Pause"), // the only E1-prefixed key
        prefix => code_for_scan(v & 0xFF, prefix == 0xE000),
    }
}

/// A code Pulse can see: a key from the table or a mouse button.
pub fn is_known(code: &str) -> bool {
    KEYS.iter().any(|(_, _, c)| *c == code)
        || MOUSE_CODES.contains(&code)
        || matches!(code, "Mouse1" | "Mouse2")
}

/// Windows' first auto-repeat comes up to ~1 s after the press (the slowest "repeat delay"); a
/// "down" for a held key later than this is a fresh press whose key-up we missed (lock screen,
/// UAC, an admin window).
const REPEAT_WINDOW: Duration = Duration::from_millis(1500);

/// The modifier keys, by code.
pub const MODIFIER_CODES: [&str; 8] = [
    "ControlLeft",
    "ControlRight",
    "ShiftLeft",
    "ShiftRight",
    "AltLeft",
    "AltRight",
    "MetaLeft",
    "MetaRight",
];

/// Turns key/button downs and ups into the actions they fire. Binds never swallow anything: this
/// only decides, the caller always passes the event on.
#[derive(Default)]
pub struct Matcher {
    binds: Vec<(Action, Bind)>,
    /// Held keys/buttons and when each last went down (or repeated).
    held: Vec<(&'static str, Instant)>,
    paused: bool,
}

impl Matcher {
    pub fn set_binds(&mut self, binds: &Keybinds) {
        self.binds = binds
            .bound()
            .into_iter()
            .map(|(a, b)| (a, b.clone()))
            .collect();
    }

    /// Paused (a bind is being recorded) fires nothing but keeps tracking held keys, so a key still
    /// held when recording ends doesn't fire on its next auto-repeat.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn down(&mut self, code: &'static str, at: Instant) -> Vec<Action> {
        if let Some((_, last)) = self.held.iter_mut().find(|(c, _)| *c == code) {
            let repeat = at.saturating_duration_since(*last) < REPEAT_WINDOW;
            *last = at;
            if repeat {
                return Vec::new();
            }
        } else {
            self.held.push((code, at));
        }
        if self.paused {
            return Vec::new();
        }
        let held = |side: [&str; 2]| {
            self.held
                .iter()
                .any(|(c, _)| *c != code && side.contains(c))
        };
        let (ctrl, shift, alt, meta) = (
            held(["ControlLeft", "ControlRight"]),
            held(["ShiftLeft", "ShiftRight"]),
            held(["AltLeft", "AltRight"]),
            held(["MetaLeft", "MetaRight"]),
        );
        let matching: Vec<_> = self
            .binds
            .iter()
            .filter(|(_, b)| {
                b.code == code
                    && (!b.ctrl || ctrl)
                    && (!b.shift || shift)
                    && (!b.alt || alt)
                    && (!b.meta || meta)
            })
            .collect();
        let mods = |b: &Bind| {
            [b.ctrl, b.shift, b.alt, b.meta]
                .iter()
                .filter(|m| **m)
                .count()
        };
        let best = matching.iter().map(|(_, b)| mods(b)).max();
        matching
            .iter()
            .filter(|(_, b)| Some(mods(b)) == best)
            .map(|(a, _)| *a)
            .collect()
    }

    /// Drop held modifiers the OS says are up: their key-ups went to the secure desktop (UAC,
    /// Ctrl+Alt+Del, Ctrl+Shift+Esc) and would otherwise turn a plain key into a combo.
    pub fn forget_released(&mut self, still_down: impl Fn(&str) -> bool) {
        self.held
            .retain(|(c, _)| !MODIFIER_CODES.contains(c) || still_down(c));
    }

    pub fn up(&mut self, code: &str) {
        self.held.retain(|(c, _)| *c != code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybinds::{Action, Bind, Keybinds};
    use std::time::{Duration, Instant};

    fn b(code: &str, mods: &str) -> Bind {
        Bind {
            code: code.into(),
            ctrl: mods.contains('c'),
            shift: mods.contains('s'),
            alt: mods.contains('a'),
            meta: mods.contains('w'),
        }
    }

    fn m(mute: Option<Bind>, deafen: Option<Bind>) -> Matcher {
        let mut m = Matcher::default();
        m.set_binds(&Keybinds {
            toggle_mute: mute,
            toggle_deafen: deafen,
        });
        m
    }

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    use Action::{ToggleDeafen as D, ToggleMute as M};

    #[test]
    fn scan_codes_map_to_physical_keys() {
        assert_eq!(code_for_scan(0x1D, false), Some("ControlLeft"));
        assert_eq!(code_for_scan(0x1D, true), Some("ControlRight"));
        assert_eq!(code_for_scan(0x51, true), Some("PageDown"));
        assert_eq!(code_for_scan(0x51, false), Some("Numpad3"));
        assert_eq!(code_for_scan(0x32, false), Some("KeyM"));
        assert_eq!(code_for_scan(0x5B, true), Some("MetaLeft"));
        assert_eq!(code_for_scan(0x36, false), Some("ShiftRight"));
        assert_eq!(code_for_scan(0x1C, true), Some("NumpadEnter"));
        assert_eq!(code_for_scan(0x64, false), Some("F13"));
        assert_eq!(code_for_scan(0x45, true), Some("NumLock"));
        assert_eq!(code_for_scan(0x45, false), Some("Pause"));
    }

    #[test]
    fn every_table_code_is_known_and_strangers_arent() {
        for (_, _, code) in KEYS {
            assert!(is_known(code), "{code}");
        }
        for code in MOUSE_CODES {
            assert!(is_known(code));
        }
        assert!(is_known("Mouse1") && is_known("Mouse2"));
        assert!(!is_known("Banana") && !is_known("Mouse6") && !is_known(""));
    }

    #[test]
    fn page_down_alone_fires_mute_once_while_held() {
        let mut m = m(Some(b("PageDown", "")), None);
        let t0 = Instant::now();
        assert_eq!(m.down("PageDown", t0), vec![M]);
        assert_eq!(m.down("PageDown", at(t0, 400)), vec![]);
        assert_eq!(m.down("PageDown", at(t0, 800)), vec![]);
        m.up("PageDown");
        assert_eq!(m.down("PageDown", at(t0, 900)), vec![M]);
    }

    #[test]
    fn a_missed_key_up_doesnt_eat_the_next_press() {
        let mut m = m(Some(b("PageDown", "")), None);
        let t0 = Instant::now();
        assert_eq!(m.down("PageDown", t0), vec![M]);
        assert_eq!(m.down("PageDown", at(t0, 5000)), vec![M]);
    }

    #[test]
    fn shift_page_down_still_mutes() {
        let mut m = m(Some(b("PageDown", "")), None);
        let t0 = Instant::now();
        m.down("ShiftLeft", t0);
        assert_eq!(m.down("PageDown", t0), vec![M]);
    }

    #[test]
    fn right_ctrl_alone_fires_and_left_ctrl_doesnt() {
        let mut m = m(None, Some(b("ControlRight", "")));
        let t0 = Instant::now();
        assert_eq!(m.down("ControlLeft", t0), vec![]);
        assert_eq!(m.down("ControlRight", t0), vec![D]);
    }

    #[test]
    fn right_ctrl_then_c_deafens_once() {
        let mut m = m(None, Some(b("ControlRight", "")));
        let t0 = Instant::now();
        assert_eq!(m.down("ControlRight", t0), vec![D]);
        assert_eq!(m.down("KeyC", t0), vec![]);
    }

    #[test]
    fn a_lone_modifier_bind_also_counts_as_ctrl_for_combos() {
        let mut m = m(Some(b("KeyM", "cs")), Some(b("ControlRight", "")));
        let t0 = Instant::now();
        assert_eq!(m.down("ControlRight", t0), vec![D]);
        assert_eq!(m.down("ShiftLeft", t0), vec![]);
        assert_eq!(m.down("KeyM", t0), vec![M]);
    }

    #[test]
    fn the_most_specific_bind_wins() {
        let mut m = m(Some(b("KeyM", "")), Some(b("KeyM", "c")));
        let t0 = Instant::now();
        m.down("ControlLeft", t0);
        assert_eq!(m.down("KeyM", t0), vec![D]);
        m.up("KeyM");
        m.up("ControlLeft");
        assert_eq!(m.down("KeyM", t0), vec![M]);
    }

    #[test]
    fn equal_matches_both_fire() {
        let mut m = m(Some(b("KeyM", "c")), Some(b("KeyM", "s")));
        let t0 = Instant::now();
        m.down("ControlLeft", t0);
        m.down("ShiftLeft", t0);
        assert_eq!(m.down("KeyM", t0), vec![M, D]);
    }

    #[test]
    fn mouse_buttons_fire() {
        let mut m = m(Some(b("Mouse3", "")), Some(b("Mouse4", "c")));
        let t0 = Instant::now();
        assert_eq!(m.down("Mouse3", t0), vec![M]);
        assert_eq!(m.down("Mouse4", t0), vec![]);
        m.up("Mouse4");
        m.down("ControlRight", t0);
        assert_eq!(m.down("Mouse4", t0), vec![D]);
    }

    #[test]
    fn paused_fires_nothing() {
        let mut m = m(Some(b("PageDown", "")), None);
        m.set_paused(true);
        assert_eq!(m.down("PageDown", Instant::now()), vec![]);
    }

    #[test]
    fn a_key_held_through_unpause_doesnt_fire() {
        let mut m = m(Some(b("PageDown", "")), None);
        let t0 = Instant::now();
        m.set_paused(true);
        m.down("PageDown", t0);
        m.set_paused(false);
        assert_eq!(m.down("PageDown", at(t0, 300)), vec![]);
    }

    #[test]
    fn new_binds_replace_old() {
        let mut m = m(Some(b("PageDown", "")), None);
        m.set_binds(&Keybinds::default());
        assert_eq!(m.down("PageDown", Instant::now()), vec![]);
    }
    #[test]
    fn unicode_packets_and_vk_only_keys() {
        // VK_PACKET: scanCode holds a character, not a key: never a bind
        assert_eq!(key_code(0xE7, 0x32, false, |_| unreachable!()), None);
        // some injected keys carry only a virtual-key code: map it (0xE0 prefix = extended)
        assert_eq!(key_code(0x22, 0, false, |_| 0xE051), Some("PageDown"));
        assert_eq!(
            key_code(0x4D, 0x32, false, |_| unreachable!()),
            Some("KeyM")
        );
    }

    #[test]
    fn a_slow_auto_repeat_is_still_a_repeat() {
        // Windows' longest repeat delay is ~1 s
        let mut m = m(Some(b("PageDown", "")), None);
        let t0 = Instant::now();
        assert_eq!(m.down("PageDown", t0), vec![M]);
        assert_eq!(m.down("PageDown", at(t0, 1100)), vec![]);
    }

    #[test]
    fn modifiers_released_behind_our_back_are_forgotten() {
        // Ctrl+Shift+Esc / UAC: the key-ups went to the secure desktop
        let mut m = m(Some(b("KeyM", "cs")), None);
        let t0 = Instant::now();
        m.down("ControlLeft", t0);
        m.down("ShiftLeft", t0);
        m.forget_released(|_| false);
        assert_eq!(m.down("KeyM", t0), vec![]);
        // still held per the OS: kept
        m.down("ControlLeft", at(t0, 5000));
        m.down("ShiftLeft", at(t0, 5000));
        m.up("KeyM");
        m.forget_released(|_| true);
        assert_eq!(m.down("KeyM", at(t0, 6000)), vec![M]);
    }
    #[test]
    fn ctrl_pause_and_vk_only_pause_are_pause() {
        // Ctrl+Pause sends E0 46 (Break); the browser records it as Pause
        assert_eq!(code_for_scan(0x46, true), Some("Pause"));
        // MAPVK_VK_TO_VSC_EX gives Pause as E1 1D
        assert_eq!(key_code(0x13, 0, false, |_| 0xE11D), Some("Pause"));
    }

    #[test]
    fn browser_keys_and_numpad_equal() {
        assert_eq!(code_for_scan(0x6A, true), Some("BrowserBack"));
        assert_eq!(code_for_scan(0x69, true), Some("BrowserForward"));
        assert_eq!(code_for_scan(0x59, false), Some("NumpadEqual"));
    }
}
