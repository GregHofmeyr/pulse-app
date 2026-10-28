//! The editor's local sources: probing a file, the temp files made for it (preview, filmstrip) and
//! the in-memory list of open sources. The Tauri commands in `commands.rs` are thin wrappers.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;

use super::encode::{self, CHAIN, Encoder, StoppableRunner, WORKING, run_with_fallback};
use super::ffmpeg::FfmpegPaths;
use super::preview::{self, PreviewPlan};
use super::probe::{self, Probe};
use super::progress;
use super::upload::CancellationFlag;
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

/// One open source's editor work: a lock, so a second preview or filmstrip request waits for the
/// first, and a signal that the source closed, which stops the ffmpeg run in flight.
pub struct Work {
    lock: tokio::sync::Mutex<()>,
    closed: tokio::sync::watch::Sender<bool>,
}

impl Work {
    fn new() -> Self {
        Self {
            lock: tokio::sync::Mutex::new(()),
            closed: tokio::sync::watch::Sender::new(false),
        }
    }

    pub async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.lock.lock().await
    }

    fn close(&self) {
        self.closed.send_replace(true);
    }

    /// Resolves once the source is closed (at once if it already is).
    pub async fn closed(&self) {
        let mut rx = self.closed.subscribe();
        let _ = rx.wait_for(|closed| *closed).await;
    }
}

/// The open sources of this run, by id, each with its work. One map, so work exists exactly while
/// its source is open: nothing is left behind for an id that was never opened or is closed.
#[derive(Default)]
pub struct Sources {
    map: Mutex<HashMap<String, (Entry, Arc<Work>)>>,
}

impl Sources {
    pub fn insert(&self, id: String, e: Entry) {
        self.map
            .lock()
            .unwrap()
            .insert(id, (e, Arc::new(Work::new())));
    }

    pub fn get(&self, id: &str) -> Option<Entry> {
        self.map.lock().unwrap().get(id).map(|(e, _)| e.clone())
    }

    pub fn update(&self, id: &str, f: impl FnOnce(&mut Entry)) {
        if let Some((e, _)) = self.map.lock().unwrap().get_mut(id) {
            f(e);
        }
    }

    /// Close a source: its work in flight stops.
    pub fn remove(&self, id: &str) -> Option<Entry> {
        let (e, work) = self.map.lock().unwrap().remove(id)?;
        work.close();
        Some(e)
    }

    /// Close every source (sign-out, quitting); the caller deletes the folders. Each comes with
    /// its work, to wait on (`Work::lock` is free once its run has stopped).
    pub fn drain(&self) -> Vec<(Entry, Arc<Work>)> {
        let all: Vec<_> = self.map.lock().unwrap().drain().map(|(_, v)| v).collect();
        for (_, work) in &all {
            work.close();
        }
        all
    }

    pub fn ids(&self) -> HashSet<String> {
        self.map.lock().unwrap().keys().cloned().collect()
    }

    /// The work of an open source; `None` once it is closed (or for an id never opened).
    pub fn lock_for(&self, id: &str) -> Option<Arc<Work>> {
        self.map.lock().unwrap().get(id).map(|(_, w)| w.clone())
    }
}

/// Delete a source's temp folder, off the async threads. Missing is fine; a failure is retried
/// briefly (a just-killed ffmpeg may still hold a file on Windows).
pub fn remove_dir_later(dir: PathBuf) {
    tauri::async_runtime::spawn_blocking(move || super::jobs::remove_folder(&dir));
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
    let run = ffmpeg::output(ffmpeg::command(ffprobe, probe::args(&text(path))));
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
/// (`encode::WORKING`, which remembers the encoder that works); a Remux has just the one run.
/// Closing the source (`closed` resolves) stops ffmpeg: it is killed and waited for before this returns.
pub async fn make_preview(
    paths: &FfmpegPaths,
    entry: &Entry,
    closed: impl Future<Output = ()>,
    mut on_percent: impl FnMut(u8) + Send,
) -> Result<PathBuf, String> {
    let out = entry.dir.join("preview.mp4");
    std::fs::create_dir_all(&entry.dir).map_err(|_| NO_PREVIEW.to_owned())?;
    let (src, dst) = (text(&entry.path), text(&out));
    let copy_audio = preview::can_copy_audio(&entry.probe);
    let encode_preview = entry.plan == PreviewPlan::Encode;
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
    let stop = CancellationFlag::new();
    let runner = StoppableRunner {
        paths: paths.clone(),
        stop: stop.clone(),
    };
    let kept_ms = entry.probe.duration_ms;
    let run = async {
        if encode_preview {
            WORKING.run(&runner, kept_ms, build, on_progress).await
        } else {
            // A Remux is a stream copy: no encoder is involved (the encoder argument is unused), so one run, and
            // nothing to remember.
            run_with_fallback(&runner, &CHAIN[..1], kept_ms, build, on_progress).await
        }
    };
    tokio::pin!(run);
    let done = tokio::select! {
        done = &mut run => Some(done),
        () = closed => None,
    };
    let Some(done) = done else {
        // closed: ffmpeg is killed and waited for, so the partial file can go
        stop.cancel();
        let _ = run.await;
        let _ = std::fs::remove_file(&out);
        return Err(NO_PREVIEW.to_owned());
    };
    if let Err(e) = done {
        tracing::warn!(error = ?e, "clips: preview not made");
        let _ = std::fs::remove_file(&out);
        return Err(NO_PREVIEW.to_owned());
    }
    Ok(out)
}

/// Make `<dir>/filmstrip.jpg`. Closing the source (`closed` resolves) stops ffmpeg: it is killed
/// and waited for before this returns (and so is one that runs past the timeout).
pub async fn make_filmstrip(
    ffmpeg_exe: &Path,
    entry: &Entry,
    closed: impl Future<Output = ()>,
) -> Result<PathBuf, String> {
    let out = entry.dir.join("filmstrip.jpg");
    std::fs::create_dir_all(&entry.dir).map_err(|_| NO_FILMSTRIP.to_owned())?;
    let args = preview::filmstrip_args(&text(&entry.path), entry.probe.duration_ms, &text(&out));
    let cmd = ffmpeg::command(
        ffmpeg_exe,
        ["-v", "error"]
            .into_iter()
            .chain(args.iter().map(String::as_str)),
    );
    let stop = async {
        tokio::select! {
            () = closed => {}
            () = tokio::time::sleep(FILMSTRIP_TIMEOUT) => {}
        }
    };
    match ffmpeg::status_until(cmd, stop).await {
        Some(Ok(s)) if s.success() && out.is_file() => Ok(out),
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

    #[tokio::test]
    async fn one_source_cannot_be_worked_on_twice_at_once() {
        let s = Sources::default();
        s.insert("a".into(), entry(Path::new("/t/a")));
        s.insert("b".into(), entry(Path::new("/t/b")));
        let first = s.lock_for("a").unwrap();
        let guard = first.lock().await;
        // the same source gets the same lock, so the second request has to wait
        let again = s.lock_for("a").unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), again.lock())
                .await
                .is_err()
        );
        // another source is independent
        let b = s.lock_for("b").unwrap();
        let _b = b.lock().await;
        drop(guard);
        let _a = again.lock().await;
    }

    #[test]
    fn only_open_sources_have_work() {
        let s = Sources::default();
        // an id never opened, or already closed, gets nothing (and leaves nothing behind)
        assert!(s.lock_for("nope").is_none());
        s.insert("a".into(), entry(Path::new("/t/a")));
        assert!(s.lock_for("a").is_some());
        assert!(s.remove("a").is_some());
        assert!(s.lock_for("a").is_none());
        assert!(s.ids().is_empty());
    }

    #[tokio::test]
    async fn closing_a_source_signals_its_work() {
        let s = Sources::default();
        s.insert("a".into(), entry(Path::new("/t/a")));
        s.insert("b".into(), entry(Path::new("/t/b")));
        let (a, b) = (s.lock_for("a").unwrap(), s.lock_for("b").unwrap());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), a.closed())
                .await
                .is_err(),
            "open: not closed"
        );
        s.remove("a");
        tokio::time::timeout(Duration::from_secs(1), a.closed())
            .await
            .expect("closed");
        // sign-out closes the rest
        assert_eq!(s.drain().len(), 1);
        tokio::time::timeout(Duration::from_secs(1), b.closed())
            .await
            .expect("closed by the drain");
    }

    /// A stand-in ffmpeg that notes its pid and sleeps for 30 s.
    #[cfg(target_os = "linux")]
    fn sleeper(dir: &Path) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let pid = dir.join("pid");
        let exe = dir.join("ffmpeg");
        std::fs::write(
            &exe,
            format!("#!/bin/sh\necho $$ > '{}'\nexec sleep 30\n", pid.display()),
        )
        .unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        (exe, pid)
    }

    /// Waits for the sleeper to start, then closes source `id`; returns its pid.
    #[cfg(target_os = "linux")]
    async fn close_once_running(s: &Sources, id: &str, pid: &Path) -> u32 {
        for _ in 0..500 {
            if let Some(p) = std::fs::read_to_string(pid)
                .ok()
                .and_then(|t| t.trim().parse().ok())
            {
                s.remove(id);
                return p;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the stand-in ffmpeg never started");
    }

    /// Already exited and waited for (not running, not even a zombie) the moment the run returned:
    /// on Windows a dying ffmpeg still holds its files, so they can only be deleted after this.
    #[cfg(target_os = "linux")]
    fn assert_exited(pid: u32) {
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "ffmpeg {pid} hadn't exited when the run returned"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn closing_the_editor_stops_its_filmstrip() {
        let dir = tempfile::tempdir().unwrap();
        let (exe, pid) = sleeper(dir.path());
        let e = entry(&dir.path().join("src"));
        let s = Sources::default();
        s.insert("a".into(), e.clone());
        let work = s.lock_for("a").unwrap();
        let run = make_filmstrip(&exe, &e, work.closed());
        let (made, pid) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(run, close_once_running(&s, "a", &pid))
        })
        .await
        .expect("the filmstrip stops when its source closes");
        assert_eq!(made, Err(NO_FILMSTRIP.to_owned()));
        assert_exited(pid);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn closing_the_editor_stops_its_preview() {
        let dir = tempfile::tempdir().unwrap();
        let (exe, pid) = sleeper(dir.path());
        let mut e = entry(&dir.path().join("src"));
        e.plan = PreviewPlan::Remux;
        let s = Sources::default();
        s.insert("a".into(), e.clone());
        let work = s.lock_for("a").unwrap();
        let paths = FfmpegPaths {
            ffmpeg: exe.clone(),
            ffprobe: exe,
        };
        let run = make_preview(&paths, &e, work.closed(), |_| {});
        let (made, pid) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(run, close_once_running(&s, "a", &pid))
        })
        .await
        .expect("the preview stops when its source closes");
        assert_eq!(made, Err(NO_PREVIEW.to_owned()));
        assert!(!dir.path().join("src/preview.mp4").exists());
        assert_exited(pid);
    }

    #[test]
    fn ids_are_16_hex_digits_and_differ() {
        let (a, b) = (new_id(), new_id());
        assert_eq!(a.len(), 16);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
