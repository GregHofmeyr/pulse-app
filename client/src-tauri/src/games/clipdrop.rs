//! Auto clip drop (#382): a recording NVIDIA (ShadowPlay / the NVIDIA App: `Videos\<Game>\`) or Game Bar
//! (`Videos\Captures`) just saved is offered to a clips channel (`pulse://clip-drop`; the page asks "Share to #clips?"
//! and opens the usual clip editor with it). Settings → Chat → Clip drop ([`ClipDropSettings`], on by default), its own
//! feature: it runs with Game activity off, never starts game detection and shares nothing until you share a clip.
//! Windows only: elsewhere nothing runs.
//!
//! No file watcher: every [`POLL`] the watched folders are listed, their top level and one level of subfolders (the
//! per-game ones), nothing deeper. A folder's files when it's first read are old ([`Tracker`]): only a video that
//! appears after that, and whose size then holds for [`SETTLE`] (seen on two polls at least), is offered, once.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// How often the folders are listed (owner: cheap, ~10 s).
pub const POLL: Duration = Duration::from_secs(10);
/// How long a new file's size must hold before it counts as written.
pub const SETTLE: Duration = Duration::from_secs(4);

/// The videos offered: what the clip picker takes.
const VIDEO_EXTS: &[&str] = &["mp4", "mov", "mkv", "webm", "m4v"];

/// Whether `path` names a finished-looking video: one of [`VIDEO_EXTS`] (so `x.mp4.tmp` and `.part` files never count)
/// and not a hidden or temp name (`.x.mp4`, `~x.mp4`).
pub fn is_video(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.starts_with('.') || name.starts_with('~') {
        return false;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTS.iter().any(|v| e.eq_ignore_ascii_case(v)))
}

/// A video file in a watched folder and its size now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub size: u64,
}

/// The videos in `root` and its direct subfolders; `None` when `root` can't be read (missing, a drive unplugged).
pub fn scan(root: &Path) -> Option<Vec<Entry>> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<Entry>, path: PathBuf, md: std::fs::Metadata| {
        if md.is_file() && is_video(&path) {
            out.push(Entry {
                path,
                size: md.len(),
            });
        }
    };
    for e in std::fs::read_dir(root).ok()?.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            let Ok(sub) = std::fs::read_dir(e.path()) else {
                continue;
            };
            for f in sub.flatten() {
                if let Ok(md) = f.metadata() {
                    push(&mut out, f.path(), md);
                }
            }
        } else if let Ok(md) = e.metadata() {
            push(&mut out, e.path(), md);
        }
    }
    Some(out)
}

/// What the polls have seen (pure: fed listings and a clock). A folder's first readable listing is old news; so is
/// one read again after it couldn't be (a drive back: its files aren't all new).
#[derive(Debug)]
pub struct Tracker {
    settle: Duration,
    /// The folders read on the last poll.
    roots: HashSet<PathBuf>,
    /// Files there at a folder's first read, or already offered.
    known: HashSet<PathBuf>,
    /// New files not yet offered: their size and since when it's held.
    growing: HashMap<PathBuf, (u64, Instant)>,
}

impl Tracker {
    pub fn new(settle: Duration) -> Self {
        Tracker {
            settle,
            roots: HashSet::new(),
            known: HashSet::new(),
            growing: HashMap::new(),
        }
    }

    /// One poll: each watched folder with its listing (`None`: unreadable now). Returns the files just finished, each
    /// once, in listing order.
    pub fn step(
        &mut self,
        listing: &[(PathBuf, Option<Vec<Entry>>)],
        now: Instant,
    ) -> Vec<PathBuf> {
        let mut done = Vec::new();
        let mut present = HashSet::new();
        let mut roots = HashSet::new();
        for (root, entries) in listing {
            let Some(entries) = entries else { continue };
            let first = !self.roots.contains(root);
            roots.insert(root.clone());
            for e in entries {
                if !present.insert(e.path.clone()) || !is_video(&e.path) {
                    continue; // listed twice (a folder inside another watched one), or not ours
                }
                if first {
                    self.known.insert(e.path.clone());
                    continue;
                }
                if self.known.contains(&e.path) {
                    continue;
                }
                match self.growing.get_mut(&e.path) {
                    Some((size, since)) if *size == e.size => {
                        if e.size > 0 && now.saturating_duration_since(*since) >= self.settle {
                            self.growing.remove(&e.path);
                            self.known.insert(e.path.clone());
                            done.push(e.path.clone());
                        }
                    }
                    Some(held) => *held = (e.size, now),
                    None => {
                        self.growing.insert(e.path.clone(), (e.size, now));
                    }
                }
            }
        }
        // gone files are forgotten (memory stays the size of the folders); so are folders not read this time
        self.known.retain(|p| present.contains(p));
        self.growing.retain(|p, _| present.contains(p));
        self.roots = roots;
        done
    }
}

/// `pulse://clip-drop`: a new recording to offer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ClipOffer {
    pub path: String,
    /// Its file name, for the toast.
    pub name: String,
}

impl ClipOffer {
    fn of(path: &Path) -> Self {
        ClipOffer {
            path: path.to_string_lossy().into_owned(),
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }
}

/// The default folders under the user's Videos: NVIDIA's per-game folders are its subfolders, Game Bar's `Captures`
/// is one too (listed on its own so it stays watched when NVIDIA's folder is moved elsewhere).
pub fn default_folders(videos: Option<PathBuf>) -> Vec<PathBuf> {
    videos
        .map(|v| vec![v.join("Captures"), v])
        .unwrap_or_default()
}

/// The folders as given, without blanks and repeats (same path, any case: Windows paths).
pub fn tidy_folders(folders: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    folders
        .into_iter()
        .filter(|p| !p.as_os_str().is_empty())
        .filter(|p| seen.insert(p.to_string_lossy().to_lowercase()))
        .collect()
}

/// Settings → Chat → Clip drop, as the page saves it (`set_clip_drop`): the switch and the folders it watches.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ClipDropSettings {
    pub on: bool,
    /// `None`: the defaults under Videos ([`default_folders`]).
    pub folders: Option<Vec<String>>,
}

impl Default for ClipDropSettings {
    fn default() -> Self {
        ClipDropSettings {
            on: true,
            folders: None,
        }
    }
}

impl ClipDropSettings {
    /// The folders to watch: the chosen ones, else the defaults under `videos`.
    pub fn watched(&self, videos: Option<PathBuf>) -> Vec<PathBuf> {
        match &self.folders {
            Some(f) => f.iter().map(PathBuf::from).collect(),
            None => default_folders(videos),
        }
    }
}

type Offer = Arc<dyn Fn(ClipOffer) + Send + Sync>;

struct Inner {
    folders: Vec<PathBuf>,
    poller: Option<tauri::async_runtime::JoinHandle<()>>,
}

/// The clip drop poller and the folders it lists. Managed Tauri state.
pub struct ClipDrop {
    inner: Arc<Mutex<Inner>>,
    offer: Offer,
    supported: bool,
    every: Duration,
    settle: Duration,
}

impl ClipDrop {
    /// Nothing runs until the page sends its switches, and never outside Windows.
    pub fn new(offer: Offer) -> Self {
        Self::with_timing(offer, cfg!(windows), POLL, SETTLE)
    }

    fn with_timing(offer: Offer, supported: bool, every: Duration, settle: Duration) -> Self {
        ClipDrop {
            inner: Arc::new(Mutex::new(Inner {
                folders: Vec::new(),
                poller: None,
            })),
            offer,
            supported,
            every,
            settle,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// On (Clip drop's switch, whatever Game activity says) starts the poller if none runs; off stops it. The folders
    /// are read on the next poll (one newly added starts as old news).
    pub fn set(&self, on: bool, folders: Vec<PathBuf>) {
        let mut g = self.lock();
        g.folders = tidy_folders(folders);
        if !on || !self.supported {
            if let Some(p) = g.poller.take() {
                p.abort();
            }
        } else if g.poller.is_none() {
            g.poller = Some(self.spawn());
        }
    }

    /// Whether a poller runs right now.
    pub fn watching(&self) -> bool {
        self.lock().poller.is_some()
    }

    fn spawn(&self) -> tauri::async_runtime::JoinHandle<()> {
        let (inner, offer, every) = (self.inner.clone(), self.offer.clone(), self.every);
        let mut tracker = Tracker::new(self.settle);
        tauri::async_runtime::spawn(async move {
            let mut tick = tokio::time::interval(every);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let folders = inner
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .folders
                    .clone();
                // off the async workers: listing folders blocks (a sleeping drive can take a moment)
                let Ok(listing) = tokio::task::spawn_blocking(move || {
                    folders
                        .into_iter()
                        .map(|f| {
                            let l = scan(&f);
                            (f, l)
                        })
                        .collect::<Vec<_>>()
                })
                .await
                else {
                    return;
                };
                for path in tracker.step(&listing, Instant::now()) {
                    offer(ClipOffer::of(&path));
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(path: &str, size: u64) -> Entry {
        Entry {
            path: PathBuf::from(path),
            size,
        }
    }

    fn root(r: &str, entries: Vec<Entry>) -> (PathBuf, Option<Vec<Entry>>) {
        (PathBuf::from(r), Some(entries))
    }

    #[test]
    fn only_videos_count_never_partial_or_temp_files() {
        for yes in ["a/clip.mp4", "a/Clip.MKV", "a/x.mov", "a/x.webm", "a/x.m4v"] {
            assert!(is_video(Path::new(yes)), "{yes}");
        }
        for no in [
            "a/clip.mp4.tmp",
            "a/clip.tmp",
            "a/clip.part",
            "a/.clip.mp4",
            "a/~clip.mp4",
            "a/notes.txt",
            "a/mp4",
        ] {
            assert!(!is_video(Path::new(no)), "{no}");
        }
    }

    #[test]
    fn files_there_at_the_first_read_are_never_offered() {
        let mut t = Tracker::new(SETTLE);
        let t0 = Instant::now();
        let old = vec![root(
            "V",
            vec![e("V/old.mp4", 100), e("V/Game/older.mp4", 5)],
        )];
        assert!(t.step(&old, t0).is_empty());
        assert!(t.step(&old, t0 + POLL).is_empty());
        assert!(t.step(&old, t0 + POLL * 2).is_empty());
    }

    #[test]
    fn a_new_file_is_offered_once_its_size_holds() {
        let mut t = Tracker::new(SETTLE);
        let t0 = Instant::now();
        t.step(&[root("V", vec![])], t0);
        // still being written: it grows between polls
        assert!(
            t.step(&[root("V", vec![e("V/Game/new.mp4", 10)])], t0 + POLL)
                .is_empty()
        );
        assert!(
            t.step(&[root("V", vec![e("V/Game/new.mp4", 20)])], t0 + POLL * 2)
                .is_empty()
        );
        // the same size a poll later: done
        assert_eq!(
            t.step(&[root("V", vec![e("V/Game/new.mp4", 20)])], t0 + POLL * 3),
            vec![PathBuf::from("V/Game/new.mp4")]
        );
        // and never again
        assert!(
            t.step(&[root("V", vec![e("V/Game/new.mp4", 20)])], t0 + POLL * 4)
                .is_empty()
        );
    }

    #[test]
    fn the_size_must_hold_for_the_settle_time_and_empty_files_wait() {
        let mut t = Tracker::new(SETTLE);
        let t0 = Instant::now();
        t.step(&[root("V", vec![])], t0);
        let l = [root("V", vec![e("V/a.mp4", 7), e("V/empty.mp4", 0)])];
        assert!(t.step(&l, t0 + Duration::from_secs(1)).is_empty());
        assert!(
            t.step(&l, t0 + Duration::from_secs(3)).is_empty(),
            "held 2 s only"
        );
        assert_eq!(
            t.step(&l, t0 + Duration::from_secs(5)),
            vec![PathBuf::from("V/a.mp4")]
        );
        assert!(
            t.step(&l, t0 + Duration::from_secs(60)).is_empty(),
            "a 0-byte file isn't a recording yet"
        );
    }

    #[test]
    fn a_folder_added_later_or_back_after_an_unplug_starts_as_old_news() {
        let mut t = Tracker::new(SETTLE);
        let t0 = Instant::now();
        t.step(&[root("V", vec![])], t0);
        let d = vec![e("D/Game/a.mp4", 9)];
        // a newly watched folder: its files are old
        let both = [root("V", vec![]), root("D", d.clone())];
        assert!(t.step(&both, t0 + POLL).is_empty());
        assert!(t.step(&both, t0 + POLL * 2).is_empty());
        // the drive goes away, then comes back with the same files (and one made meanwhile): none of them is new
        let gone = [root("V", vec![]), (PathBuf::from("D"), None)];
        assert!(t.step(&gone, t0 + POLL * 3).is_empty());
        let back = [
            root("V", vec![]),
            root("D", vec![d[0].clone(), e("D/b.mp4", 3)]),
        ];
        assert!(t.step(&back, t0 + POLL * 4).is_empty());
        assert!(t.step(&back, t0 + POLL * 5).is_empty());
    }

    #[test]
    fn a_file_in_two_watched_folders_is_offered_once() {
        let mut t = Tracker::new(SETTLE);
        let t0 = Instant::now();
        let l0 = [root("V/Captures", vec![]), root("V", vec![])];
        t.step(&l0, t0);
        let f = e("V/Captures/c.mp4", 4);
        let l = [
            root("V/Captures", vec![f.clone()]),
            root("V", vec![f.clone()]),
        ];
        assert!(t.step(&l, t0 + POLL).is_empty());
        assert_eq!(
            t.step(&l, t0 + POLL * 2),
            vec![PathBuf::from("V/Captures/c.mp4")]
        );
    }

    #[test]
    fn scan_reads_the_top_level_and_one_level_of_subfolders() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        std::fs::write(r.join("top.mp4"), b"12").unwrap();
        std::fs::write(r.join("top.mp4.tmp"), b"1").unwrap();
        std::fs::create_dir_all(r.join("Game/deeper")).unwrap();
        std::fs::write(r.join("Game/clip.mkv"), b"123").unwrap();
        std::fs::write(r.join("Game/deeper/no.mp4"), b"1").unwrap();
        let mut got = scan(r).unwrap();
        got.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(
            got,
            vec![
                Entry {
                    path: r.join("Game/clip.mkv"),
                    size: 3
                },
                Entry {
                    path: r.join("top.mp4"),
                    size: 2
                },
            ]
        );
        assert_eq!(scan(&r.join("missing")), None);
    }

    #[test]
    fn folders_default_under_videos_and_repeats_go() {
        let v = PathBuf::from("C:/Users/alex/Videos");
        assert_eq!(
            default_folders(Some(v.clone())),
            vec![v.join("Captures"), v.clone()]
        );
        assert!(default_folders(None).is_empty());
        assert_eq!(
            tidy_folders([
                v.clone(),
                PathBuf::new(),
                PathBuf::from("c:/users/ALEX/videos"),
                PathBuf::from("D:/Clips")
            ]),
            vec![v, PathBuf::from("D:/Clips")]
        );
    }

    #[test]
    fn settings_default_on_and_watch_the_chosen_folders_or_the_defaults() {
        let v = PathBuf::from("C:/Users/alex/Videos");
        let s: ClipDropSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, ClipDropSettings::default());
        assert!(s.on);
        assert_eq!(s.watched(Some(v.clone())), default_folders(Some(v.clone())));
        let s: ClipDropSettings =
            serde_json::from_str(r#"{"on":false,"folders":["D:/Clips"]}"#).unwrap();
        assert!(!s.on);
        assert_eq!(s.watched(Some(v)), vec![PathBuf::from("D:/Clips")]);
    }

    #[tokio::test]
    async fn the_poller_offers_a_new_recording_and_stops_when_off() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path().to_path_buf();
        std::fs::write(r.join("old.mp4"), b"old").unwrap();
        let offers = Arc::new(Mutex::new(Vec::<ClipOffer>::new()));
        let o = offers.clone();
        let cd = ClipDrop::with_timing(
            Arc::new(move |c| o.lock().unwrap().push(c)),
            true,
            Duration::from_millis(20),
            Duration::from_millis(30),
        );
        cd.set(true, vec![r.clone()]);
        assert!(cd.watching());
        tokio::time::sleep(Duration::from_millis(50)).await; // the first poll has read the folder
        std::fs::create_dir_all(r.join("Game")).unwrap();
        std::fs::write(r.join("Game/new.mp4"), b"recording").unwrap();
        for _ in 0..200 {
            if !offers.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let got = offers.lock().unwrap().clone();
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].name, "new.mp4");

        cd.set(false, vec![r.clone()]);
        assert!(!cd.watching());
        std::fs::write(r.join("later.mp4"), b"x").unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(offers.lock().unwrap().len(), 1, "nothing after off");
    }

    #[test]
    fn nothing_runs_where_unsupported() {
        let cd = ClipDrop::with_timing(Arc::new(|_| panic!("no offers")), false, POLL, SETTLE);
        cd.set(true, vec![PathBuf::from("V")]);
        assert!(!cd.watching());
    }
}
