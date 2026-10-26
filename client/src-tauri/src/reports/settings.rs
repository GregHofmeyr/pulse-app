//! "Send crash reports automatically": a per-profile switch, stored as `reports.json` beside
//! `keybinds.json`. On by default (a missing file or field = on), so nothing changes for anyone
//! who never touches it. Off: a crash or unexpected exit is still saved locally, but it is held,
//! and only goes out if the user picks "Send" when asked on the next launch.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const FILE: &str = "reports.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportSettings {
    #[serde(default = "on")]
    pub auto_send_crashes: bool,
}

fn on() -> bool {
    true
}

impl Default for ReportSettings {
    fn default() -> Self {
        Self {
            auto_send_crashes: true,
        }
    }
}

/// No file (never changed) → on. A file that's there but unreadable → off: it only exists
/// because the user changed the setting, and guessing "send" could upload what they said not to.
pub fn load(dir: &Path) -> ReportSettings {
    let Ok(text) = std::fs::read_to_string(dir.join(FILE)) else {
        return ReportSettings::default();
    };
    match serde_json::from_str::<ReportSettings>(&text) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "report settings unreadable; not sending automatically");
            ReportSettings {
                auto_send_crashes: false,
            }
        }
    }
}

pub fn save(dir: &Path, s: &ReportSettings) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(s).map_err(std::io::Error::other)?;
    // write then rename: a crash mid-write can't leave a half file
    let tmp = dir.join(format!("{FILE}.tmp"));
    std::fs::write(&tmp, json)?;
    std::fs::rename(tmp, dir.join(FILE))
}

/// The profile's report settings, as the app is using them.
pub struct Store {
    dir: PathBuf,
    /// Held across a save, so two changes can't interleave their file writes.
    saved: Mutex<ReportSettings>,
}

impl Store {
    pub fn load(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            saved: Mutex::new(load(dir)),
        }
    }

    pub fn get(&self) -> ReportSettings {
        self.saved.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn auto_send_crashes(&self) -> bool {
        self.get().auto_send_crashes
    }

    /// Save, then use. A failed save changes nothing.
    pub fn set(&self, next: ReportSettings) -> std::io::Result<()> {
        let mut saved = self.saved.lock().unwrap_or_else(|e| e.into_inner());
        save(&self.dir, &next)?;
        *saved = next;
        Ok(())
    }
}

#[tauri::command]
pub fn get_report_settings(store: tauri::State<'_, Store>) -> ReportSettings {
    store.get()
}

/// Turning it off also holds a report already waiting to go (one whose automatic send failed and
/// would be retried): it is then only sent if the user picks "Send". Turning it back on doesn't
/// release a held report.
#[tauri::command]
pub fn set_report_settings(
    store: tauri::State<'_, Store>,
    log_dir: tauri::State<'_, super::LogDir>,
    settings: ReportSettings,
) -> Result<ReportSettings, String> {
    store
        .set(settings.clone())
        .map_err(|e| format!("couldn't save the setting: {e}"))?;
    if !settings.auto_send_crashes {
        super::crash::hold_pending(&log_dir.0);
    }
    tracing::info!(
        auto_send_crashes = settings.auto_send_crashes,
        "report settings changed"
    );
    Ok(store.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_by_default() {
        let d = tempfile::tempdir().unwrap();
        assert!(load(d.path()).auto_send_crashes);
        assert!(Store::load(d.path()).auto_send_crashes());
    }

    #[test]
    fn a_missing_field_is_on() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(FILE), "{}").unwrap();
        assert!(load(d.path()).auto_send_crashes);
    }

    #[test]
    fn off_survives_a_restart() {
        let d = tempfile::tempdir().unwrap();
        Store::load(d.path())
            .set(ReportSettings {
                auto_send_crashes: false,
            })
            .unwrap();
        assert!(!Store::load(d.path()).auto_send_crashes());
        assert!(!d.path().join(format!("{FILE}.tmp")).exists());
    }

    #[test]
    fn back_on_survives_a_restart() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::load(d.path());
        s.set(ReportSettings {
            auto_send_crashes: false,
        })
        .unwrap();
        s.set(ReportSettings {
            auto_send_crashes: true,
        })
        .unwrap();
        assert!(Store::load(d.path()).auto_send_crashes());
    }

    #[test]
    fn an_unreadable_file_means_off() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(FILE), "{ not json").unwrap();
        assert!(!load(d.path()).auto_send_crashes);
    }

    #[test]
    fn settings_are_per_profile() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        Store::load(a.path())
            .set(ReportSettings {
                auto_send_crashes: false,
            })
            .unwrap();
        assert!(!Store::load(a.path()).auto_send_crashes());
        assert!(Store::load(b.path()).auto_send_crashes());
    }
}
