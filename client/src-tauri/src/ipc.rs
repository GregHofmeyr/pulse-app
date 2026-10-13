//! Linux hotkeys: Wayland apps can't grab global keys, so a Hyprland bind runs
//! `pulse-app --toggle-mute`, which pokes the running app over an owner-only Unix socket.

#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;

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

/// `$XDG_RUNTIME_DIR/pulse-app[-<profile>].sock` (per-user tmpfs), else a uid-scoped temp path.
/// Per profile: a second profile must not take over the first one's hotkeys.
pub fn socket_path() -> PathBuf {
    socket_path_for(
        std::env::var_os("XDG_RUNTIME_DIR"),
        &std::env::var("USER").unwrap_or_else(|_| "user".into()),
        std::env::var("PULSE_PROFILE").ok().as_deref(),
    )
}

fn socket_path_for(
    runtime_dir: Option<std::ffi::OsString>,
    user: &str,
    profile: Option<&str>,
) -> PathBuf {
    let suffix = crate::session::profile_name(profile)
        .map(|p| format!("-{p}"))
        .unwrap_or_default();
    match runtime_dir {
        Some(dir) => PathBuf::from(dir).join(format!("pulse-app{suffix}.sock")),
        // Must be the same for the app and the CLI, so per-user, not per-process.
        None => std::env::temp_dir().join(format!("pulse-app-{user}{suffix}.sock")),
    }
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
    let mut s = std::os::unix::net::UnixStream::connect(path)?;
    s.write_all(c.wire().as_bytes())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[test]
    fn each_profile_has_its_own_socket() {
        let run = Some(std::ffi::OsString::from("/run/user/1000"));
        assert_eq!(
            socket_path_for(run.clone(), "greg", None),
            PathBuf::from("/run/user/1000/pulse-app.sock")
        );
        assert_eq!(
            socket_path_for(run, "greg", Some("b")),
            PathBuf::from("/run/user/1000/pulse-app-b.sock")
        );
        assert_eq!(
            socket_path_for(None, "greg", Some("../x")),
            std::env::temp_dir().join("pulse-app-greg-x.sock"),
            "cleaned like the profile dirs"
        );
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
