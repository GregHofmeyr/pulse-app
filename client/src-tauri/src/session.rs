//! Session token storage. Preferred: the OS keychain (Windows Credential Manager / Secret Service).
//! Fallback when no keychain exists (e.g. Hyprland without a Secret Service): an owner-only
//! (0600) `session.json` in the app data dir — the same protection Discord gives its token.
//! The token never crosses into the webview either way.

use std::path::{Path, PathBuf};

use keyring::Entry;

const SERVICE: &str = "pulse-app";
const LAST_SERVER: &str = "__last_server__";

/// Where this instance keeps its session. `PULSE_PROFILE=b` gives a second, separate login on the
/// same machine (testing with two accounts). Only the profile's alphanumerics are used.
pub fn profile_dir(base: &Path, profile: Option<&str>) -> PathBuf {
    let clean: String = profile
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if clean.is_empty() {
        base.to_path_buf()
    } else {
        base.join("profiles").join(clean)
    }
}

/// Keychain service name, per profile (Windows always has a keychain, so without this a second
/// profile would overwrite the first one's login).
pub fn keychain_service(profile: Option<&str>) -> String {
    let clean: String = profile
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if clean.is_empty() {
        SERVICE.to_string()
    } else {
        format!("{SERVICE}-{clean}")
    }
}

pub struct Store {
    dir: PathBuf,
    service: String,
}

impl Store {
    pub fn new(dir: PathBuf, profile: Option<&str>) -> Self {
        Self {
            dir,
            service: keychain_service(profile),
        }
    }

    pub fn save(&self, server_url: &str, token: &str) {
        if keychain::save(&self.service, server_url, token).is_ok() {
            file::clear(&self.dir);
            return;
        }
        if let Err(e) = file::save(&self.dir, server_url, token) {
            tracing::warn!(error = %e, "could not persist session");
        }
    }

    pub fn load(&self) -> Option<(String, String)> {
        keychain::load(&self.service).or_else(|| file::load(&self.dir))
    }

    pub fn clear(&self, server_url: &str) {
        keychain::clear(&self.service, server_url);
        file::clear(&self.dir);
    }
}

mod keychain {
    use super::*;

    fn entry(service: &str, account: &str) -> keyring::Result<Entry> {
        Entry::new(service, account)
    }

    pub fn save(service: &str, server_url: &str, token: &str) -> keyring::Result<()> {
        entry(service, server_url)?.set_password(token)?;
        entry(service, LAST_SERVER)?.set_password(server_url)
    }

    pub fn load(service: &str) -> Option<(String, String)> {
        let server = entry(service, LAST_SERVER).ok()?.get_password().ok()?;
        let token = entry(service, &server).ok()?.get_password().ok()?;
        Some((server, token))
    }

    pub fn clear(service: &str, server_url: &str) {
        if let Ok(e) = entry(service, server_url) {
            let _ = e.delete_credential();
        }
    }
}

pub(crate) mod file {
    use super::*;

    pub const NAME: &str = "session.json";

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Saved {
        server: String,
        token: String,
    }

    pub fn save(dir: &Path, server: &str, token: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(NAME);
        let body = serde_json::to_vec(&Saved {
            server: server.into(),
            token: token.into(),
        })?;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        use std::io::Write;
        opts.open(&path)?.write_all(&body)?;
        // mode() only applies on create; tighten an existing file too.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    pub fn load(dir: &Path) -> Option<(String, String)> {
        let s: Saved = serde_json::from_slice(&std::fs::read(dir.join(NAME)).ok()?).ok()?;
        Some((s.server, s.token))
    }

    pub fn clear(dir: &Path) {
        let _ = std::fs::remove_file(dir.join(NAME));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_fallback_roundtrip_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(file::load(dir.path()), None);
        file::save(dir.path(), "http://localhost:7890", "tok").unwrap();
        assert_eq!(
            file::load(dir.path()),
            Some(("http://localhost:7890".into(), "tok".into()))
        );
        file::clear(dir.path());
        assert_eq!(file::load(dir.path()), None);
    }

    #[cfg(unix)]
    #[test]
    fn file_fallback_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        file::save(dir.path(), "http://x", "tok").unwrap();
        let mode = std::fs::metadata(dir.path().join(file::NAME))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[cfg(test)]
mod profile_tests {
    use super::*;

    #[test]
    fn profile_dir_separates_sessions() {
        let base = std::path::Path::new("/data/app.pulse.client");
        assert_eq!(profile_dir(base, None), base.to_path_buf());
        assert_eq!(
            profile_dir(base, Some("b")),
            base.join("profiles").join("b")
        );
        // nothing path-like escapes the data dir
        assert_eq!(
            profile_dir(base, Some("../../etc")),
            base.join("profiles").join("etc")
        );
        assert_eq!(profile_dir(base, Some("")), base.to_path_buf());
    }

    /// Windows always has a keychain, so the profile must be part of the keychain entry or a
    /// second profile overwrites the first one's login.
    #[test]
    fn keychain_service_is_per_profile() {
        assert_eq!(keychain_service(None), "pulse-app");
        assert_eq!(keychain_service(Some("b")), "pulse-app-b");
        assert_eq!(keychain_service(Some("")), "pulse-app");
        assert_eq!(keychain_service(Some("../x")), "pulse-app-x");
    }
}
