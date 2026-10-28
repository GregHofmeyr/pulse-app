//! Finding ffmpeg/ffprobe and spawning them.
//!
//! The Windows installer bundles BtbN's LGPL build as resources (`ffmpeg/` beside the app, see
//! `tauri.windows.conf.json` and `scripts/fetch-ffmpeg.sh`). Release builds elsewhere never send clips. Debug
//! builds on any OS also take `PULSE_DEV_FFMPEG_DIR`, a folder holding `ffmpeg` and `ffprobe`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Serialize;
use tokio::process::Command;

/// The one variable naming a dev/test ffmpeg folder (debug builds only; tests and CI use it too).
pub const DEV_DIR_ENV: &str = "PULSE_DEV_FFMPEG_DIR";

/// `CREATE_NO_WINDOW`: a console child of our GUI process would otherwise flash a console window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub const WINDOWS_ONLY: &str = "Sharing clips is Windows-only for now.";
pub const MISSING: &str = "Pulse's video tools are missing. Reinstall Pulse to share clips.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Linux,
    Other,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else {
            Os::Other
        }
    }

    fn exe(self, stem: &str) -> String {
        match self {
            Os::Windows => format!("{stem}.exe"),
            Os::Linux | Os::Other => stem.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegPaths {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

/// `ffmpeg` and `ffprobe` in `dir`, if both are files.
fn in_dir(dir: &Path, os: Os) -> Option<FfmpegPaths> {
    let paths = FfmpegPaths {
        ffmpeg: dir.join(os.exe("ffmpeg")),
        ffprobe: dir.join(os.exe("ffprobe")),
    };
    (paths.ffmpeg.is_file() && paths.ffprobe.is_file()).then_some(paths)
}

/// Where ffmpeg is: the dev folder first (when given and complete), then, on Windows, the bundled
/// `resource_dir/ffmpeg/`. Release builds elsewhere have none.
pub fn locate(
    resource_dir: Option<&Path>,
    dev_override: Option<&Path>,
    os: Os,
) -> Option<FfmpegPaths> {
    if let Some(found) = dev_override.and_then(|dir| in_dir(dir, os)) {
        return Some(found);
    }
    match os {
        Os::Windows => in_dir(&resource_dir?.join("ffmpeg"), os),
        Os::Linux | Os::Other => None,
    }
}

/// `PULSE_DEV_FFMPEG_DIR` in debug builds; always `None` in release builds.
pub fn dev_override() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var_os(DEV_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Every ffmpeg/ffprobe spawn goes through here: killed when dropped, no stdin, and no console window on Windows.
pub fn command<I, S>(program: &Path, args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClipSupport {
    pub can_send: bool,
    pub reason: Option<String>,
}

/// Whether this install can send clips, and why not.
pub fn clip_support(found: Option<&FfmpegPaths>, os: Os) -> ClipSupport {
    let reason = match (found, os) {
        (Some(_), _) => None,
        (None, Os::Windows) => Some(MISSING),
        (None, Os::Linux | Os::Other) => Some(WINDOWS_ONLY),
    };
    ClipSupport {
        can_send: reason.is_none(),
        reason: reason.map(str::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, names: &[&str]) {
        std::fs::create_dir_all(dir).unwrap();
        for n in names {
            std::fs::write(dir.join(n), b"x").unwrap();
        }
    }

    #[test]
    fn locate_finds_the_bundled_ffmpeg_on_windows() {
        let res = tempfile::tempdir().unwrap();
        touch(
            &res.path().join("ffmpeg"),
            &["ffmpeg.exe", "ffprobe.exe", "avcodec-62.dll"],
        );
        let got = locate(Some(res.path()), None, Os::Windows).expect("bundled ffmpeg");
        assert_eq!(got.ffmpeg, res.path().join("ffmpeg").join("ffmpeg.exe"));
        assert_eq!(got.ffprobe, res.path().join("ffmpeg").join("ffprobe.exe"));
    }

    #[test]
    fn release_linux_never_sends() {
        // Even a resource folder holding binaries doesn't count off Windows.
        let res = tempfile::tempdir().unwrap();
        touch(
            &res.path().join("ffmpeg"),
            &["ffmpeg", "ffprobe", "ffmpeg.exe", "ffprobe.exe"],
        );
        assert_eq!(locate(Some(res.path()), None, Os::Linux), None);
        assert_eq!(locate(None, None, Os::Linux), None);
    }

    #[test]
    fn debug_override_wins() {
        let dev = tempfile::tempdir().unwrap();
        touch(dev.path(), &["ffmpeg", "ffprobe"]);
        let got = locate(None, Some(dev.path()), Os::Linux).expect("dev ffmpeg on Linux");
        assert_eq!(
            got,
            FfmpegPaths {
                ffmpeg: dev.path().join("ffmpeg"),
                ffprobe: dev.path().join("ffprobe")
            }
        );

        // On Windows the override beats the bundled copy.
        let res = tempfile::tempdir().unwrap();
        touch(&res.path().join("ffmpeg"), &["ffmpeg.exe", "ffprobe.exe"]);
        touch(dev.path(), &["ffmpeg.exe", "ffprobe.exe"]);
        let got = locate(Some(res.path()), Some(dev.path()), Os::Windows).unwrap();
        assert_eq!(got.ffmpeg, dev.path().join("ffmpeg.exe"));
        assert_eq!(got.ffprobe, dev.path().join("ffprobe.exe"));
    }

    #[test]
    fn missing_files_mean_no_ffmpeg() {
        // Bundled folder without ffprobe.
        let res = tempfile::tempdir().unwrap();
        touch(&res.path().join("ffmpeg"), &["ffmpeg.exe"]);
        assert_eq!(locate(Some(res.path()), None, Os::Windows), None);
        // No resource dir at all.
        assert_eq!(locate(None, None, Os::Windows), None);
        // Override folder without ffmpeg, nothing bundled.
        let dev = tempfile::tempdir().unwrap();
        touch(dev.path(), &["ffprobe"]);
        assert_eq!(locate(None, Some(dev.path()), Os::Linux), None);
        // A directory named like the binary isn't the binary.
        let odd = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(odd.path().join("ffmpeg")).unwrap();
        std::fs::create_dir_all(odd.path().join("ffprobe")).unwrap();
        assert_eq!(locate(None, Some(odd.path()), Os::Linux), None);
    }

    #[test]
    fn an_incomplete_override_falls_back_to_the_bundled_copy() {
        let res = tempfile::tempdir().unwrap();
        touch(&res.path().join("ffmpeg"), &["ffmpeg.exe", "ffprobe.exe"]);
        let dev = tempfile::tempdir().unwrap();
        let got = locate(Some(res.path()), Some(dev.path()), Os::Windows).unwrap();
        assert_eq!(got.ffmpeg, res.path().join("ffmpeg").join("ffmpeg.exe"));
    }

    #[test]
    fn clip_support_explains_linux() {
        let s = clip_support(None, Os::Linux);
        assert_eq!(
            s,
            ClipSupport {
                can_send: false,
                reason: Some("Sharing clips is Windows-only for now.".into())
            }
        );
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "can_send": false, "reason": "Sharing clips is Windows-only for now." })
        );
    }

    #[test]
    fn clip_support_with_ffmpeg_can_send() {
        let p = FfmpegPaths {
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
        };
        // A Linux debug build with PULSE_DEV_FFMPEG_DIR sends too.
        for os in [Os::Windows, Os::Linux] {
            assert_eq!(
                clip_support(Some(&p), os),
                ClipSupport {
                    can_send: true,
                    reason: None
                }
            );
        }
        // Windows without the bundled folder: a broken install, not "Windows-only".
        assert_eq!(
            clip_support(None, Os::Windows),
            ClipSupport {
                can_send: false,
                reason: Some(MISSING.into())
            }
        );
    }

    #[test]
    fn command_sets_program_and_args() {
        let cmd = command(Path::new("/opt/ff/ffprobe"), ["-v", "error", "in.mp4"]);
        let std = cmd.as_std();
        assert_eq!(std.get_program(), OsStr::new("/opt/ff/ffprobe"));
        let args: Vec<_> = std.get_args().collect();
        assert_eq!(args, ["-v", "error", "in.mp4"]);
    }

    /// kill_on_drop: dropping the child (a cancelled job) kills the process instead of leaking it.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn dropping_the_child_kills_it() {
        let child = command(Path::new("/bin/sleep"), ["30"])
            .spawn()
            .expect("spawn sleep");
        let pid = child.id().expect("pid");
        drop(child);
        let stat = format!("/proc/{pid}/stat");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            // Gone, or a zombie waiting to be reaped: either way it no longer runs.
            match std::fs::read_to_string(&stat) {
                Err(_) => break,
                Ok(s)
                    if s.rsplit(')')
                        .next()
                        .is_some_and(|rest| rest.trim_start().starts_with('Z')) =>
                {
                    break;
                }
                Ok(_) => {}
            }
            assert!(
                std::time::Instant::now() < deadline,
                "sleep {pid} still running after drop"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
}
