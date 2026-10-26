//! Session token storage. Preferred: the OS keychain (Windows Credential Manager / Secret Service).
//! Fallback when no keychain exists (e.g. Hyprland without a Secret Service): an owner-only
//! `session.json` in the app data dir — mode 0600 on Unix; on Windows a protected DACL that grants
//! only the current user, applied when the file is created.
//! The token never crosses into the webview either way.

use std::path::{Path, PathBuf};

use keyring::Entry;

const SERVICE: &str = "pulse-app";
const LAST_SERVER: &str = "__last_server__";

/// Where this instance keeps its session. `PULSE_PROFILE=b` gives a second, separate login on the
/// same machine (testing with two accounts). Only the profile's alphanumerics are used.
fn clean_profile(profile: Option<&str>) -> String {
    profile
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// The profile's name as used in paths, or `None` for the default profile.
pub fn profile_name(profile: Option<&str>) -> Option<String> {
    Some(clean_profile(profile)).filter(|p| !p.is_empty())
}

/// A second launch of the default profile focuses the running window instead of starting another
/// copy. Named profiles (testing several users on one machine) may run side by side.
pub fn single_instance(profile: Option<&str>) -> bool {
    clean_profile(profile).is_empty()
}

pub fn profile_dir(base: &Path, profile: Option<&str>) -> PathBuf {
    let clean = clean_profile(profile);
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
        prefer_keychain(&self.dir, keychain::load(&self.service))
    }

    pub fn clear(&self, server_url: &str) {
        keychain::clear(&self.service, server_url);
        file::clear(&self.dir);
    }
}

/// The keychain's session wins. A session.json beside it is a leftover (from a time without a
/// keychain, or an older version) that would never be read again, so it is removed.
fn prefer_keychain(
    dir: &Path,
    from_keychain: Option<(String, String)>,
) -> Option<(String, String)> {
    match from_keychain {
        Some(session) => {
            file::clear(dir);
            Some(session)
        }
        None => file::load(dir),
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
        write_owner_only(&path, &body)
    }

    #[cfg(not(windows))]
    fn write_owner_only(path: &Path, body: &[u8]) -> std::io::Result<()> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        use std::io::Write;
        opts.open(path)?.write_all(body)?;
        // mode() only applies on create; tighten an existing file too.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    /// Written to a fresh temp file that is created with the owner-only DACL (so it is never
    /// readable through the folder's inherited permissions, not even briefly), then renamed over
    /// the target. A rename on the same volume keeps the moved file's DACL, so this also replaces
    /// an older, inherited-permission session.json.
    #[cfg(windows)]
    fn write_owner_only(path: &Path, body: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        let tmp = path.with_extension("json.tmp");
        // A leftover from a crash: CREATE_NEW below must create the file (an existing file would
        // keep its own DACL).
        let _ = std::fs::remove_file(&tmp);
        let written = win::create_owner_only(&tmp).and_then(|mut f| {
            f.write_all(body)?;
            f.sync_all()
        });
        let renamed = written.and_then(|()| std::fs::rename(&tmp, path));
        if renamed.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        renamed
    }

    pub fn load(dir: &Path) -> Option<(String, String)> {
        let path = dir.join(NAME);
        let body = std::fs::read(&path).ok()?;
        // Versions before the owner-only DACL left the file with the folder's inherited
        // permissions; tighten it in place the first time it is read (even if it won't parse).
        #[cfg(windows)]
        if let Err(e) = win::restrict(&path) {
            tracing::warn!(error = %e, "could not restrict session file permissions");
        }
        let s: Saved = serde_json::from_slice(&body).ok()?;
        Some((s.server, s.token))
    }

    pub fn clear(dir: &Path) {
        let _ = std::fs::remove_file(dir.join(NAME));
        // a temp file a crash left mid-save (Windows writes through one)
        let _ = std::fs::remove_file(dir.join(NAME).with_extension("json.tmp"));
    }

    /// Windows: an owner-only file means a protected DACL (no inherited entries) with a single
    /// entry granting the current user full access. Nobody else, SYSTEM and Administrators
    /// included, gets an entry (an administrator can still take ownership; nothing on the machine
    /// stops that).
    #[cfg(windows)]
    pub(crate) mod win {
        use std::fs::File;
        use std::io;
        use std::os::windows::ffi::OsStrExt;
        use std::os::windows::io::FromRawHandle;
        use std::path::Path;

        use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
        use windows::Win32::Security::Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
        };
        use windows::Win32::Security::{
            ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, GetTokenInformation,
            PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES,
            TOKEN_QUERY, TOKEN_USER, TokenUser,
        };
        use windows::Win32::Storage::FileSystem::{
            CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
            FILE_SHARE_NONE,
        };
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        use windows::core::{BOOL, HSTRING, PCWSTR, PWSTR};

        fn wide(path: &Path) -> Vec<u16> {
            path.as_os_str().encode_wide().chain([0]).collect()
        }

        /// The current user's SID, from the process token (the user, not a group, even when
        /// elevated).
        pub struct UserSid {
            // TOKEN_USER plus the SID it points into; u64 keeps the pointer field aligned.
            buf: Vec<u64>,
        }

        impl UserSid {
            pub fn current() -> io::Result<Self> {
                unsafe {
                    let mut token = HANDLE::default();
                    OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
                    let mut len = 0u32;
                    // First call only reports the size (and fails with "insufficient buffer").
                    let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
                    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
                    let got = GetTokenInformation(
                        token,
                        TokenUser,
                        Some(buf.as_mut_ptr().cast()),
                        len,
                        &mut len,
                    );
                    let _ = CloseHandle(token);
                    got?;
                    Ok(Self { buf })
                }
            }

            pub fn psid(&self) -> PSID {
                unsafe { (*(self.buf.as_ptr() as *const TOKEN_USER)).User.Sid }
            }

            fn to_sddl(&self) -> io::Result<String> {
                unsafe {
                    let mut s = PWSTR::null();
                    ConvertSidToStringSidW(self.psid(), &mut s)?;
                    let out = s.to_string();
                    LocalFree(Some(HLOCAL(s.0.cast())));
                    out.map_err(io::Error::other)
                }
            }
        }

        /// `D:P(A;;FA;;;<sid>)`: protected DACL, one allow-ACE, FILE_ALL_ACCESS, current user.
        struct OwnerOnly(PSECURITY_DESCRIPTOR);

        impl OwnerOnly {
            fn new() -> io::Result<Self> {
                let sddl = format!("D:P(A;;FA;;;{})", UserSid::current()?.to_sddl()?);
                let mut sd = PSECURITY_DESCRIPTOR::default();
                unsafe {
                    ConvertStringSecurityDescriptorToSecurityDescriptorW(
                        &HSTRING::from(sddl),
                        SDDL_REVISION_1,
                        &mut sd,
                        None,
                    )?;
                }
                Ok(Self(sd))
            }
        }

        impl Drop for OwnerOnly {
            fn drop(&mut self) {
                unsafe {
                    LocalFree(Some(HLOCAL(self.0.0)));
                }
            }
        }

        /// Creates `path` (it must not exist) with the owner-only DACL applied at creation.
        pub fn create_owner_only(path: &Path) -> io::Result<File> {
            let sd = OwnerOnly::new()?;
            let attrs = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd.0.0,
                bInheritHandle: false.into(),
            };
            let name = wide(path);
            unsafe {
                let h = CreateFileW(
                    PCWSTR(name.as_ptr()),
                    (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
                    FILE_SHARE_NONE,
                    Some(&attrs),
                    CREATE_NEW,
                    FILE_ATTRIBUTE_NORMAL,
                    None,
                )?;
                Ok(File::from_raw_handle(h.0))
            }
        }

        /// Replaces an existing file's DACL with the owner-only one (protected: inherited entries
        /// are dropped).
        pub fn restrict(path: &Path) -> io::Result<()> {
            let sd = OwnerOnly::new()?;
            let name = wide(path);
            unsafe {
                let (mut present, mut defaulted) = (BOOL::default(), BOOL::default());
                let mut dacl: *mut ACL = std::ptr::null_mut();
                GetSecurityDescriptorDacl(sd.0, &mut present, &mut dacl, &mut defaulted)?;
                SetNamedSecurityInfoW(
                    PCWSTR(name.as_ptr()),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    None,
                    None,
                    Some(dacl),
                    None,
                )
                .ok()?;
            }
            Ok(())
        }
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

    /// A session.json left beside a working keychain is never read again, so it is removed
    /// rather than left on disk (for Windows: with an older version's inherited permissions).
    #[test]
    fn a_working_keychain_removes_a_leftover_session_file() {
        let dir = tempfile::tempdir().unwrap();
        file::save(dir.path(), "http://old", "stale").unwrap();
        let from_keychain = Some(("http://x".to_string(), "tok".to_string()));
        assert_eq!(
            prefer_keychain(dir.path(), from_keychain.clone()),
            from_keychain
        );
        assert!(!dir.path().join(file::NAME).exists());

        // no keychain entry: the file is the session
        file::save(dir.path(), "http://x", "tok").unwrap();
        assert_eq!(prefer_keychain(dir.path(), None), from_keychain);
        assert!(dir.path().join(file::NAME).exists());
    }

    /// The file's DACL is protected (nothing inherited from the folder) and holds exactly one
    /// ACE: full access for the current user.
    #[cfg(windows)]
    fn assert_owner_only(path: &std::path::Path) {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Foundation::{HLOCAL, LocalFree};
        use windows::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
        use windows::Win32::Security::{
            ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
            DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation,
            GetSecurityDescriptorControl, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
        };
        use windows::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
        use windows::core::PCWSTR;

        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let me = file::win::UserSid::current().unwrap();
        unsafe {
            let mut dacl: *mut ACL = std::ptr::null_mut();
            let mut sd = PSECURITY_DESCRIPTOR::default();
            GetNamedSecurityInfoW(
                PCWSTR(wide.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut dacl),
                None,
                &mut sd,
            )
            .ok()
            .unwrap();

            let (mut control, mut revision) = (0u16, 0u32);
            GetSecurityDescriptorControl(sd, &mut control, &mut revision).unwrap();
            assert_ne!(control & SE_DACL_PROTECTED.0, 0, "DACL must not inherit");

            assert!(!dacl.is_null(), "a NULL DACL grants everyone access");
            let mut size = ACL_SIZE_INFORMATION::default();
            GetAclInformation(
                dacl,
                &mut size as *mut _ as *mut _,
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
            .unwrap();
            assert_eq!(size.AceCount, 1, "exactly one ACE");

            let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
            GetAce(dacl, 0, &mut ace).unwrap();
            let ace = &*(ace as *const ACCESS_ALLOWED_ACE);
            assert_eq!(ace.Header.AceType, 0, "ACCESS_ALLOWED_ACE_TYPE");
            assert_eq!(ace.Mask, FILE_ALL_ACCESS.0);
            let sid = PSID(&ace.SidStart as *const u32 as *mut _);
            EqualSid(sid, me.psid()).expect("the ACE is for the current user");

            LocalFree(Some(HLOCAL(sd.0)));
        }
    }

    #[cfg(windows)]
    #[test]
    fn file_fallback_is_owner_only_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        file::save(dir.path(), "http://x", "tok").unwrap();
        assert_owner_only(&dir.path().join(file::NAME));
        // saving again (rename over the existing file) keeps it owner-only
        file::save(dir.path(), "http://x", "tok2").unwrap();
        assert_owner_only(&dir.path().join(file::NAME));
        assert_eq!(
            file::load(dir.path()),
            Some(("http://x".into(), "tok2".into()))
        );
    }

    /// Versions before this one wrote session.json with the folder's inherited permissions.
    #[cfg(windows)]
    #[test]
    fn older_inherited_session_file_is_tightened() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(file::NAME);
        std::fs::write(&path, br#"{"server":"http://x","token":"tok"}"#).unwrap();
        assert_eq!(
            file::load(dir.path()),
            Some(("http://x".into(), "tok".into()))
        );
        assert_owner_only(&path);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(file::NAME);
        std::fs::write(&path, b"old").unwrap();
        file::save(dir.path(), "http://x", "tok").unwrap();
        assert_owner_only(&path);

        // a corrupt one is tightened too, even though it can't be used
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(file::NAME);
        std::fs::write(&path, b"not json").unwrap();
        assert_eq!(file::load(dir.path()), None);
        assert_owner_only(&path);
    }
}

#[cfg(test)]
mod profile_tests {
    use super::*;

    #[test]
    fn only_the_default_profile_is_single_instance() {
        // friends run one Pulse; extra profiles are for testing several users on one machine
        assert!(single_instance(None));
        assert!(single_instance(Some("")));
        assert!(
            single_instance(Some("../")),
            "nothing left after cleaning = the default"
        );
        assert!(!single_instance(Some("b")));
    }

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
