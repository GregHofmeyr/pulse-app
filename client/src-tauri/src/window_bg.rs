//! The main window's background colour follows the UI theme (#42): what shows before the page paints and in resize
//! gaps. The page sends its theme's `--bg-0` (`set_window_background`); it is kept per profile as `window-bg` (a
//! `#rrggbb` line) so the next start opens on the right colour instead of flashing Pulse Dark.

use std::path::{Path, PathBuf};

use tauri::window::Color;

const FILE: &str = "window-bg";

/// Pulse Dark's `--bg-0`: no file, or one that can't be read.
pub const DEFAULT: Color = Color(0x14, 0x15, 0x19, 0xff);

/// The profile's data directory, where the colour is kept.
pub struct Dir(pub PathBuf);

/// `#rrggbb` (any case) as an opaque colour; anything else is `None`.
pub fn parse(s: &str) -> Option<Color> {
    let hex = s.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(Color(byte(0)?, byte(2)?, byte(4)?, 0xff))
}

/// The saved colour, or [`DEFAULT`].
pub fn load(dir: &Path) -> Color {
    std::fs::read_to_string(dir.join(FILE))
        .ok()
        .and_then(|s| parse(&s))
        .unwrap_or(DEFAULT)
}

/// Keeps `color` for the next start; no write when it is already the saved one (every start sends it).
pub fn save(dir: &Path, color: Color) -> std::io::Result<()> {
    if load(dir) == color && dir.join(FILE).exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    let Color(r, g, b, _) = color;
    std::fs::write(dir.join(FILE), format!("#{r:02x}{g:02x}{b:02x}\n"))
}

/// The page's theme changed (or it started): colour the window now and remember it.
#[tauri::command]
pub fn set_window_background(
    app: tauri::AppHandle,
    dir: tauri::State<'_, Dir>,
    color: String,
) -> Result<(), String> {
    use tauri::Manager;
    let c = parse(&color).ok_or_else(|| format!("not a #rrggbb colour: {color}"))?;
    // Saved first: the next start uses it even if colouring this window fails.
    let saved = save(&dir.0, c).map_err(|e| e.to_string());
    if let Some(win) = app.get_webview_window("main") {
        win.set_background_color(Some(c))
            .map_err(|e| e.to_string())?;
    }
    saved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colours_only() {
        assert_eq!(parse("#e9eaed"), Some(Color(0xe9, 0xea, 0xed, 0xff)));
        assert_eq!(parse("#FFFFFF\n"), Some(Color(0xff, 0xff, 0xff, 0xff)));
        for bad in [
            "",
            "#fff",
            "e9eaed",
            "#e9eaeg",
            "#e9eaed00",
            "rgb(0,0,0)",
            "#é9eaed",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn missing_or_bad_file_is_pulse_dark() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), DEFAULT);
        std::fs::write(dir.path().join(FILE), "nonsense").unwrap();
        assert_eq!(load(dir.path()), DEFAULT);
    }

    #[test]
    fn saved_colour_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("profile");
        let light = Color(0xe9, 0xea, 0xed, 0xff);
        save(&sub, light).unwrap();
        assert_eq!(load(&sub), light);
        assert_eq!(
            std::fs::read_to_string(sub.join(FILE)).unwrap(),
            "#e9eaed\n"
        );
        // Pulse Dark is saved too (switching back must stick, though it equals the default)
        save(&sub, DEFAULT).unwrap();
        assert_eq!(
            std::fs::read_to_string(sub.join(FILE)).unwrap(),
            "#141519\n"
        );
    }
}
