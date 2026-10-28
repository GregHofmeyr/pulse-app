//! The editor's local sources: probing a file, the temp files made for it (preview, filmstrip) and
//! the in-memory list of open sources. The Tauri commands in `commands.rs` are thin wrappers.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;

use super::encode::{self, CHAIN, Encoder, FfmpegRunner, run_with_fallback};
use super::ffmpeg::FfmpegPaths;
use super::preview::{self, PreviewPlan};
use super::probe::{self, Probe};
use super::progress;
use super::{ffmpeg, game};

pub const UNREADABLE: &str = "Pulse couldn't read that video.";
pub const NO_PREVIEW: &str = "Pulse couldn't prepare a preview of that video.";
pub const NO_FILMSTRIP: &str = "Pulse couldn't read the frames of that video.";

const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const FILMSTRIP_TIMEOUT: Duration = Duration::from_secs(120);

/// What `open_clip` returns (mirrored by hand as `ClipSource` in `tauri.ts`).
#[derive(Debug, Clone, Serialize)]
pub struct ClipSource {
    pub id: String,
    pub path: String,
    pub file_name: String,
    pub size: u64,
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub video_codec: String,
    pub game_guess: Option<String>,
    pub preview: PreviewPlan,
}

#[derive(Debug, Clone, Serialize)]
pub struct Filmstrip {
    pub path: String,
    pub tiles: u32,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    /// `<scope>/sources/<id>/`: the preview and the filmstrip go here.
    pub dir: PathBuf,
    pub probe: Probe,
    pub plan: PreviewPlan,
    pub preview: Option<PathBuf>,
    pub filmstrip: Option<PathBuf>,
}

/// The open sources of this run, by id.
#[derive(Default)]
pub struct Sources {
    map: Mutex<HashMap<String, Entry>>,
    /// One async lock per source: a second preview or filmstrip request waits for the first.
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Sources {
    pub fn insert(&self, id: String, e: Entry) {
        self.map.lock().unwrap().insert(id, e);
    }

    pub fn get(&self, id: &str) -> Option<Entry> {
        self.map.lock().unwrap().get(id).cloned()
    }

    pub fn update(&self, id: &str, f: impl FnOnce(&mut Entry)) {
        if let Some(e) = self.map.lock().unwrap().get_mut(id) {
            f(e);
        }
    }

    pub fn remove(&self, id: &str) -> Option<Entry> {
        self.locks.lock().unwrap().remove(id);
        self.map.lock().unwrap().remove(id)
    }

    /// Forget every source (sign-out); the caller deletes the folders.
    pub fn drain(&self) -> Vec<Entry> {
        self.locks.lock().unwrap().clear();
        self.map.lock().unwrap().drain().map(|(_, e)| e).collect()
    }

    pub fn ids(&self) -> HashSet<String> {
        self.map.lock().unwrap().keys().cloned().collect()
    }

    /// The lock that serialises the preview and filmstrip work of one source.
    pub fn lock_for(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .unwrap()
            .entry(id.to_owned())
            .or_default()
            .clone()
    }
}

/// Delete a source's temp folder, off the async threads. Missing is fine.
pub fn remove_dir_later(dir: PathBuf) {
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = std::fs::remove_dir_all(&dir)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(error = %e, "clips: temp files not removed");
        }
    });
}

/// What the editor may open: an absolute path to a regular file with a video extension (the picker's
/// list). Checked before ffprobe ever sees the path.
pub fn validate_source_path(path: &Path) -> Result<(), String> {
    const EXTENSIONS: [&str; 5] = ["mp4", "mov", "mkv", "webm", "m4v"];
    let ext = extension(path).to_ascii_lowercase();
    if path.is_absolute() && EXTENSIONS.contains(&ext.as_str()) && path.is_file() {
        Ok(())
    } else {
        Err(UNREADABLE.to_owned())
    }
}

/// Delete source folders under `<scope>/sources/` that no open source owns (left by a sign-out in the
/// middle of a job, or a crash). Returns how many were removed.
pub fn clear_stale_sources(scope: &Path, keep: &HashSet<String>) -> usize {
    let Ok(dirs) = std::fs::read_dir(scope.join("sources")) else {
        return 0;
    };
    let mut removed = 0;
    for d in dirs.flatten() {
        if keep.contains(d.file_name().to_string_lossy().as_ref()) {
            continue;
        }
        if std::fs::remove_dir_all(d.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

pub fn new_id() -> String {
    format!("{:016x}", rand::random::<u64>())
}

fn text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Run ffprobe on `path` and read the answer.
pub async fn probe_file(ffprobe: &Path, path: &Path) -> Result<Probe, String> {
    let size = std::fs::metadata(path)
        .map_err(|_| UNREADABLE.to_owned())?
        .len();
    let run = ffmpeg::command(ffprobe, probe::args(&text(path))).output();
    let out = match tokio::time::timeout(PROBE_TIMEOUT, run).await {
        Ok(Ok(out)) if out.status.success() => out,
        _ => return Err(UNREADABLE.to_owned()),
    };
    probe::parse(&String::from_utf8_lossy(&out.stdout), size).map_err(|e| e.to_string())
}

pub fn describe(id: &str, path: &Path, p: &Probe, plan: PreviewPlan) -> ClipSource {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    ClipSource {
        id: id.to_owned(),
        path: text(path),
        file_name,
        size: p.size,
        duration_ms: p.duration_ms,
        width: p.width,
        height: p.height,
        fps: p.fps,
        video_codec: p.video_codec.clone(),
        game_guess: game::guess(&text(path), p.title.as_deref()),
        preview: plan,
    }
}

pub fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Make `<dir>/preview.mp4` for a Remux or Encode source. Encode goes down the shared encoder chain
/// (`encode::run_with_fallback`); a Remux has just the one run.
pub async fn make_preview(
    paths: &FfmpegPaths,
    entry: &Entry,
    mut on_percent: impl FnMut(u8) + Send,
) -> Result<PathBuf, String> {
    let out = entry.dir.join("preview.mp4");
    std::fs::create_dir_all(&entry.dir).map_err(|_| NO_PREVIEW.to_owned())?;
    let (src, dst) = (text(&entry.path), text(&out));
    let copy_audio = preview::can_copy_audio(&entry.probe);
    let encoders: &[Encoder] = if entry.plan == PreviewPlan::Encode {
        &CHAIN
    } else {
        &CHAIN[..1]
    };
    let build = |enc: Encoder| {
        let mut a = encode::head();
        a.extend(preview::args(
            entry.plan,
            &src,
            &dst,
            enc.name(),
            copy_audio,
        ));
        a
    };
    let mut last = 0u8;
    let on_progress = |fraction: f32| {
        let pct = progress::percent(fraction);
        if pct != last {
            last = pct;
            on_percent(pct);
        }
    };
    let runner = FfmpegRunner(paths.clone());
    let done = run_with_fallback(
        &runner,
        encoders,
        entry.probe.duration_ms,
        build,
        on_progress,
    )
    .await;
    if let Err(e) = done {
        tracing::warn!(error = ?e, "clips: preview not made");
        let _ = std::fs::remove_file(&out);
        return Err(NO_PREVIEW.to_owned());
    }
    Ok(out)
}

/// Make `<dir>/filmstrip.jpg`.
pub async fn make_filmstrip(ffmpeg_exe: &Path, entry: &Entry) -> Result<PathBuf, String> {
    let out = entry.dir.join("filmstrip.jpg");
    std::fs::create_dir_all(&entry.dir).map_err(|_| NO_FILMSTRIP.to_owned())?;
    let args = preview::filmstrip_args(&text(&entry.path), entry.probe.duration_ms, &text(&out));
    let run = ffmpeg::command(
        ffmpeg_exe,
        ["-v", "error"]
            .into_iter()
            .chain(args.iter().map(String::as_str)),
    )
    .output();
    match tokio::time::timeout(FILMSTRIP_TIMEOUT, run).await {
        Ok(Ok(o)) if o.status.success() && out.is_file() => Ok(out),
        _ => Err(NO_FILMSTRIP.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clips::probe::AudioStream;

    fn entry(dir: &Path) -> Entry {
        Entry {
            path: "/v/a.mp4".into(),
            dir: dir.into(),
            probe: Probe {
                duration_ms: 1000,
                width: 1,
                height: 1,
                fps: 1.0,
                video_codec: "h264".into(),
                audio: vec![AudioStream {
                    index: 1,
                    codec: "aac".into(),
                }],
                format_name: String::new(),
                title: None,
                size: 1,
            },
            plan: PreviewPlan::Direct,
            preview: None,
            filmstrip: None,
        }
    }

    #[test]
    fn sources_come_and_go() {
        let s = Sources::default();
        s.insert("a".into(), entry(Path::new("/t/a")));
        s.update("a", |e| e.preview = Some("/t/a/preview.mp4".into()));
        assert_eq!(
            s.get("a").unwrap().preview,
            Some(PathBuf::from("/t/a/preview.mp4"))
        );
        assert!(s.remove("a").is_some());
        assert!(s.get("a").is_none() && s.remove("a").is_none());
        s.insert("b".into(), entry(Path::new("/t/b")));
        assert_eq!(s.drain().len(), 1);
        assert!(s.get("b").is_none());
    }

    #[test]
    fn describe_guesses_the_game_and_names_the_file() {
        let e = entry(Path::new("/t"));
        let c = describe(
            "id",
            Path::new("/home/x/Videos/Valorant/a.mp4"),
            &e.probe,
            e.plan,
        );
        assert_eq!(c.file_name, "a.mp4");
        assert_eq!(c.game_guess.as_deref(), Some("Valorant"));
        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(json["preview"], "direct");
        assert_eq!(json["duration_ms"], 1000);
    }

    #[test]
    fn only_video_files_by_extension_are_valid_sources() {
        let dir = tempfile::tempdir().unwrap();
        let mk = |n: &str| {
            let p = dir.path().join(n);
            std::fs::write(&p, b"x").unwrap();
            p
        };
        for ok in ["a.mp4", "b.MOV", "c.mkv", "d.WebM", "e.m4v"] {
            assert!(validate_source_path(&mk(ok)).is_ok(), "{ok}");
        }
        for bad in ["id_rsa", "a.mp3", "a.m3u8", "a.txt", "mp4"] {
            assert_eq!(
                validate_source_path(&mk(bad)),
                Err(UNREADABLE.to_owned()),
                "{bad}"
            );
        }
        // missing, relative, and a directory named like a video
        assert!(validate_source_path(&dir.path().join("none.mp4")).is_err());
        assert!(validate_source_path(Path::new("a.mp4")).is_err());
        std::fs::create_dir(dir.path().join("dir.mp4")).unwrap();
        assert!(validate_source_path(&dir.path().join("dir.mp4")).is_err());
    }

    #[test]
    fn stale_source_folders_are_cleared_but_open_ones_stay() {
        let scope = tempfile::tempdir().unwrap();
        for id in ["open", "stale1", "stale2"] {
            let d = scope.path().join("sources").join(id);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("preview.mp4"), b"x").unwrap();
        }
        let keep: HashSet<String> = ["open".to_owned()].into();
        assert_eq!(clear_stale_sources(scope.path(), &keep), 2);
        assert!(scope.path().join("sources/open/preview.mp4").is_file());
        assert!(!scope.path().join("sources/stale1").exists());
        // no sources folder at all is fine
        assert_eq!(clear_stale_sources(&scope.path().join("nope"), &keep), 0);
    }

    #[test]
    fn one_source_cannot_be_worked_on_twice_at_once() {
        let s = Sources::default();
        let first = s.lock_for("a");
        let guard = first.try_lock().expect("free");
        // the same source gets the same lock, so the second request has to wait
        assert!(s.lock_for("a").try_lock().is_err());
        // another source is independent
        assert!(s.lock_for("b").try_lock().is_ok());
        drop(guard);
        assert!(s.lock_for("a").try_lock().is_ok());
        assert_eq!(s.ids().len(), 0);
    }

    #[test]
    fn ids_are_16_hex_digits_and_differ() {
        let (a, b) = (new_id(), new_id());
        assert_eq!(a.len(), 16);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
