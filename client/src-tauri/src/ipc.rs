//! Linux hotkeys: Wayland apps can't grab global keys, so a Hyprland bind runs
//! `pulse-app --toggle-mute`, which pokes the running app over an owner-only Unix socket.

#[cfg(unix)]
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    ToggleMute,
    ToggleDeafen,
}

impl Command {
    pub fn parse(line: &str) -> Option<Self> {
        match line.trim() {
            "toggle-mute" => Some(Self::ToggleMute),
            "toggle-deafen" => Some(Self::ToggleDeafen),
            _ => None,
        }
    }

    pub fn from_args(args: &[String]) -> Option<Self> {
        args.iter()
            .skip(1)
            .find_map(|a| Self::parse(a.trim_start_matches("--")))
    }

    #[cfg(unix)]
    fn wire(self) -> &'static str {
        match self {
            Self::ToggleMute => "toggle-mute\n",
            Self::ToggleDeafen => "toggle-deafen\n",
        }
    }
}

/// `$XDG_RUNTIME_DIR/pulse-app[-<profile>].sock` (per-user tmpfs), else
/// `<temp>/pulse-app-<uid>/pulse-app[-<profile>].sock`, inside a private directory (see
/// `private_dir`). Per profile: a second profile must not take over the first one's hotkeys.
#[cfg(unix)]
pub fn socket_path() -> std::io::Result<PathBuf> {
    socket_path_in(
        std::env::var_os("XDG_RUNTIME_DIR"),
        &std::env::temp_dir(),
        current_uid(),
        std::env::var("PULSE_PROFILE").ok().as_deref(),
    )
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(unix)]
fn socket_path_in(
    runtime_dir: Option<std::ffi::OsString>,
    temp: &Path,
    uid: u32,
    profile: Option<&str>,
) -> std::io::Result<PathBuf> {
    let name = format!(
        "pulse-app{}.sock",
        crate::session::profile_name(profile)
            .map(|p| format!("-{p}"))
            .unwrap_or_default()
    );
    match runtime_dir {
        Some(dir) => Ok(PathBuf::from(dir).join(name)),
        // Must be the same for the app and the CLI, so per-user, not per-process.
        None => {
            let dir = temp.join(format!("pulse-app-{uid}"));
            private_dir(&dir, uid)?;
            Ok(dir.join(name))
        }
    }
}

/// Creates `dir` with mode `0700`, or accepts it if it already is a real directory (not a
/// symlink) owned by `uid` with no group or other permissions. The shared temp directory is
/// writable by everyone, so anything else may be someone else's.
#[cfg(unix)]
fn private_dir(dir: &Path, uid: u32) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return Err(e),
        _ => {}
    }
    // lstat: a symlink is refused, not followed
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "the socket directory isn't a private directory of ours",
        ));
    }
    Ok(())
}

/// Errors unless `path` is a socket owned by `uid`: the CLI must not talk to someone else's.
#[cfg(unix)]
fn check_socket(path: &Path, uid: u32) -> std::io::Result<()> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.file_type().is_socket() || meta.uid() != uid {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "the socket isn't ours",
        ));
    }
    Ok(())
}

#[cfg(unix)]
pub fn serve(
    path: &Path,
    handler: impl Fn(Command) + Send + Sync + 'static,
) -> std::io::Result<tokio::task::JoinHandle<()>> {
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::AsyncBufReadExt;

    let _ = std::fs::remove_file(path); // stale socket from a crash
    let listener = tokio::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    let handler = std::sync::Arc::new(handler);
    Ok(tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let handler = handler.clone();
            tokio::spawn(async move {
                let mut lines = tokio::io::BufReader::new(stream).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(c) = Command::parse(&line) {
                        handler(c);
                    }
                }
            });
        }
    }))
}

/// CLI side: deliver one command to the running app. Errors if the app isn't running.
#[cfg(unix)]
pub fn send(path: &Path, c: Command) -> std::io::Result<()> {
    use std::io::Write;
    check_socket(path, current_uid())?;
    let mut s = std::os::unix::net::UnixStream::connect(path)?;
    s.write_all(c.wire().as_bytes())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn mode(p: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::symlink_metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn each_profile_has_its_own_socket() {
        let temp = tempfile::tempdir().unwrap();
        let uid = current_uid();
        let run = Some(std::ffi::OsString::from("/run/user/1000"));
        assert_eq!(
            socket_path_in(run.clone(), temp.path(), uid, None).unwrap(),
            PathBuf::from("/run/user/1000/pulse-app.sock")
        );
        assert_eq!(
            socket_path_in(run, temp.path(), uid, Some("b")).unwrap(),
            PathBuf::from("/run/user/1000/pulse-app-b.sock")
        );
        assert_eq!(
            socket_path_in(None, temp.path(), uid, Some("../x")).unwrap(),
            temp.path()
                .join(format!("pulse-app-{uid}"))
                .join("pulse-app-x.sock"),
            "cleaned like the profile dirs"
        );
    }

    #[test]
    fn fallback_socket_is_in_a_private_dir_of_ours() {
        use std::os::unix::fs::MetadataExt;
        let temp = tempfile::tempdir().unwrap();
        let uid = current_uid();
        let path = socket_path_in(None, temp.path(), uid, None).unwrap();
        let dir = path.parent().unwrap();
        assert_eq!(dir, temp.path().join(format!("pulse-app-{uid}")));
        let meta = std::fs::symlink_metadata(dir).unwrap();
        assert!(meta.is_dir());
        assert_eq!(meta.uid(), uid);
        assert_eq!(mode(dir), 0o700);
        // a second call (the CLI, or a restart) accepts the directory it made
        assert_eq!(socket_path_in(None, temp.path(), uid, None).unwrap(), path);
    }

    #[test]
    fn fallback_refuses_an_open_dir() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let uid = current_uid();
        let dir = temp.path().join(format!("pulse-app-{uid}"));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(socket_path_in(None, temp.path(), uid, None).is_err());
        assert_eq!(mode(&dir), 0o777, "left alone, not fixed up");
    }

    #[test]
    fn fallback_refuses_a_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let uid = current_uid();
        let target = temp.path().join("elsewhere");
        std::fs::create_dir(&target).unwrap(); // 0700 by umask or not: a symlink is refused anyway
        std::os::unix::fs::symlink(&target, temp.path().join(format!("pulse-app-{uid}"))).unwrap();
        assert!(socket_path_in(None, temp.path(), uid, None).is_err());
    }

    #[test]
    fn fallback_refuses_a_dir_owned_by_someone_else() {
        // Can't chown without root: ask for a uid that isn't ours instead.
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let other = current_uid().wrapping_add(1);
        let dir = temp.path().join(format!("pulse-app-{other}"));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(private_dir(&dir, current_uid()).is_ok());
        assert!(private_dir(&dir, other).is_err());
    }

    #[tokio::test]
    async fn send_refuses_a_socket_that_isnt_ours() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pulse-app.sock");
        let _server = serve(&path, |_| {}).unwrap();
        assert!(check_socket(&path, current_uid()).is_ok());
        assert!(check_socket(&path, current_uid().wrapping_add(1)).is_err());
        let file = dir.path().join("plain");
        std::fs::write(&file, b"").unwrap();
        assert!(check_socket(&file, current_uid()).is_err(), "not a socket");
    }

    #[tokio::test]
    async fn socket_roundtrip_dispatches_command() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pulse-app.sock");
        let got = Arc::new(Mutex::new(Vec::new()));
        let g = got.clone();
        let _server = serve(&path, move |c| g.lock().unwrap().push(c)).unwrap();
        send(&path, Command::ToggleMute).unwrap();
        send(&path, Command::ToggleDeafen).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            *got.lock().unwrap(),
            vec![Command::ToggleMute, Command::ToggleDeafen]
        );
    }

    #[tokio::test]
    async fn unknown_command_ignored() {
        assert_eq!(Command::parse("toggle-mute"), Some(Command::ToggleMute));
        assert_eq!(
            Command::parse("  toggle-deafen \n"),
            Some(Command::ToggleDeafen)
        );
        assert_eq!(Command::parse("rm -rf /"), None);
        assert_eq!(
            Command::from_args(&["pulse-app".into(), "--toggle-mute".into()]),
            Some(Command::ToggleMute)
        );
        assert_eq!(Command::from_args(&["pulse-app".into()]), None);
    }

    #[tokio::test]
    async fn socket_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pulse-app.sock");
        let _server = serve(&path, |_| {}).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn send_without_app_running_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(send(&dir.path().join("nope.sock"), Command::ToggleMute).is_err());
    }
}
