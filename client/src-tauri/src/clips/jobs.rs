//! Sending a clip: one job per clip that prepares it (the export encode, then the thumbnail),
//! uploads it (ticket, poster, the streamed PUT) and posts it (confirm, with a nonce kept from the
//! start so a retried confirm can never post twice).
//!
//! Each job lives in `<scope>/jobs/<id>/` (`job.json`, `out.mp4`, `thumb.jpg`), per account like the
//! media cache, so a restart can show it again (failed, with Retry). Every change goes to the UI
//! as the whole [`ClipJob`] (`clip://job`), progress at most 4 times a second.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pulse_protocol::ids::{ChannelId, ClipUploadId, GameId, MessageId};
use pulse_protocol::rest::{ClipUploadRequest, ConfirmClipRequest};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;

use super::encode::{self, EncodeError, Preset, Runner, Span, WORKING};
use super::preview;
use super::probe::Probe;
use super::upload::{self, CancellationFlag};
use crate::api::{Api, ApiError};

/// The largest clip the server takes (200 MiB, shown as "200 MB").
pub const MAX_BYTES: u64 = 200 * 1024 * 1024;
/// The most a clip may keep, in milliseconds.
pub const MAX_KEPT_MS: u64 = 90_000;
const CAPTION_MAX: usize = 300;

pub const TOO_BIG: &str = "This clip came out over 200 MB. Try Light.";
pub const TOO_LONG: &str = "Clips can be up to 1:30. Drop a bit more.";
pub const NOTHING_KEPT: &str = "Nothing kept. Bring a piece back.";
const BAD_PIECES: &str = "Those pieces don't fit that video.";
const CAPTION_TOO_LONG: &str = "Captions can be up to 300 characters.";
const NOT_SAVED: &str = "Pulse couldn't save that clip.";
const NO_SUCH_JOB: &str = "That clip is gone.";

/// Progress alone goes to the UI at most this often (stage changes always go at once).
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
/// Sign-out spends at most this long deleting tickets; the server sweeps what is left.
const SIGN_OUT_LIMIT: Duration = Duration::from_secs(5);

/// The finished files, and what ffmpeg writes while it works (renamed when it is done, so a stop
/// half way never leaves something that looks finished).
const OUT: &str = "out.mp4";
const OUT_PART: &str = "out.mp4.part";
const THUMB: &str = "thumb.jpg";
/// ffmpeg picks the image format from the extension, so the partial poster keeps `.jpg`.
const THUMB_PART: &str = "thumb.part.jpg";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Preparing,
    Uploading,
    Posting,
    Failed,
    Done,
}

/// What the UI is told about a job (mirrored by hand as `ClipJob` in `tauri.ts`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ClipJob {
    pub id: String,
    pub channel_id: ChannelId,
    pub stage: Stage,
    /// 0 to 1 within the stage: the encode is 0 to 0.95 of Preparing, the thumbnail the rest.
    pub progress: f32,
    /// The server's or the encoder's own words; `None` for anything else (the card says "Upload
    /// failed — Retry" itself).
    pub error: Option<String>,
    pub caption: String,
    pub game_id: Option<GameId>,
    pub kept_ms: u64,
    /// The local poster once made (it is on the asset scope).
    pub thumb_path: Option<String>,
    /// Set on `Done` when the clip posted; `None` on a cancelled job's last event.
    pub message_id: Option<MessageId>,
}

/// What `send_clip` asks for: the editor's source (path and the probe it already made) and the cut.
#[derive(Debug, Clone)]
pub struct NewJob {
    pub source_path: PathBuf,
    pub probe: Probe,
    pub channel_id: ChannelId,
    pub spans: Vec<Span>,
    pub preset: Preset,
    pub caption: String,
    pub game_id: Option<GameId>,
}

/// `job.json`: the request, and how far it got.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Saved {
    source_path: PathBuf,
    /// The source's probe from the editor: preparing again after a restart never re-probes.
    probe: Probe,
    spans: Vec<Span>,
    preset: Preset,
    caption: String,
    game_id: Option<GameId>,
    /// Sent with every confirm of this clip: a confirm that landed but whose answer was lost
    /// replays the same message instead of posting again.
    nonce: String,
    channel_id: ChannelId,
    stage: Stage,
    /// The current ticket. Retry cancels it before asking for a new one; sign-out deletes it.
    upload_id: Option<ClipUploadId>,
    /// The store took the file for `upload_id`: Retry confirms rather than uploading again.
    #[serde(default)]
    uploaded: bool,
    thumb_hash: Option<String>,
    /// The encoder that made `out.mp4`. Set only once preparing finished (the poster too), so it
    /// also says the files are complete.
    encoder: Option<String>,
}

/// What a run of Pulse needs from its host: the UI event, and the asset scope for the poster.
pub struct Hooks {
    pub emit: Box<dyn Fn(&ClipJob) + Send + Sync>,
    pub allow_file: Box<dyn Fn(&Path) + Send + Sync>,
    pub forbid_file: Box<dyn Fn(&Path) + Send + Sync>,
}

struct Entry {
    saved: Saved,
    progress: f32,
    error: Option<String>,
    /// A pipeline task is running (it owns the job until it ends); `run_no` tells runs apart.
    running: bool,
    run_no: u64,
    task: Option<JoinHandle<()>>,
    cancel: CancellationFlag,
    last_emit: Option<Instant>,
    /// A progress change held back by the 4-a-second limit is due to be sent.
    trailing: bool,
}

impl Entry {
    fn new(saved: Saved) -> Self {
        Self {
            saved,
            progress: 0.0,
            error: None,
            running: false,
            run_no: 0,
            task: None,
            cancel: CancellationFlag::new(),
            last_emit: None,
            trailing: false,
        }
    }
}

struct Inner<R> {
    dir: PathBuf,
    runner: R,
    api: Api,
    token: String,
    http: reqwest::Client,
    hooks: Hooks,
    rt: tokio::runtime::Handle,
    jobs: Mutex<HashMap<String, Entry>>,
    #[cfg(test)]
    lose_confirm: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    confirm_gate: Mutex<Option<Arc<tokio::sync::Notify>>>,
}

/// The clip jobs of one signed-in session. Cloning shares them.
pub struct Jobs<R> {
    inner: Arc<Inner<R>>,
}

impl<R> Clone for Jobs<R> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Why a step stopped: `Some` is the server's or the encoder's words for the card.
type Stop = Option<String>;

fn kept_ms(spans: &[Span]) -> u64 {
    spans
        .iter()
        .map(|s| s.end_ms.saturating_sub(s.start_ms))
        .sum()
}

/// The source time of the poster frame: kept time 1.0 s, or half the kept time when that is under 2 s.
pub fn thumb_at(spans: &[Span]) -> u64 {
    let kept = kept_ms(spans);
    let mut t = if kept < 2_000 { kept / 2 } else { 1_000 };
    for s in spans {
        let len = s.end_ms - s.start_ms;
        if t < len {
            return s.start_ms + t;
        }
        t -= len;
    }
    spans.last().map_or(0, |s| s.end_ms)
}

/// The cut must be sorted, non-overlapping, inside the source, and at most 1:30.
fn check(req: &NewJob) -> Result<(), String> {
    if req.spans.is_empty() {
        return Err(NOTHING_KEPT.to_owned());
    }
    let mut end = 0;
    for s in &req.spans {
        if s.start_ms < end || s.end_ms <= s.start_ms || s.end_ms > req.probe.duration_ms {
            return Err(BAD_PIECES.to_owned());
        }
        end = s.end_ms;
    }
    if kept_ms(&req.spans) > MAX_KEPT_MS {
        return Err(TOO_LONG.to_owned());
    }
    if req.caption.trim().chars().count() > CAPTION_MAX {
        return Err(CAPTION_TOO_LONG.to_owned());
    }
    Ok(())
}

/// The server's own words in a refusal, if it gave any.
fn server_words(e: &ApiError) -> Stop {
    match e {
        ApiError::Told { message, .. }
        | ApiError::Rejected(message)
        | ApiError::Forbidden(message)
        | ApiError::UploadGone(message)
        | ApiError::PosterGone(message) => Some(message.clone()),
        _ => None,
    }
}

/// Whether preparing finished: the encoder is recorded and both files are there.
fn prepared(job_dir: &Path, s: &Saved) -> bool {
    s.encoder.is_some() && job_dir.join(OUT).is_file() && job_dir.join(THUMB).is_file()
}

/// Delete a folder. ffmpeg killed a moment ago may still hold `out.mp4.part` on Windows, so a
/// failure is retried briefly. Blocking: call off the async threads.
pub(crate) fn remove_folder(dir: &Path) {
    for attempt in 0..10 {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => return,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) if attempt == 9 => {
                tracing::warn!(error = %e, "clips: folder not removed");
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Write `<job_dir>/job.json` through a synced temp file, so neither a crash nor a power cut
/// leaves half of one.
fn persist(job_dir: &Path, saved: &Saved) -> bool {
    let tmp = job_dir.join("job.json.tmp");
    let ok = serde_json::to_vec(saved)
        .map_err(std::io::Error::other)
        .and_then(|raw| {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&raw)?;
            f.sync_all()
        })
        .and_then(|()| std::fs::rename(&tmp, job_dir.join("job.json")));
    if let Err(e) = &ok {
        tracing::warn!(error = %e, "clips: job not saved");
    }
    ok.is_ok()
}

/// Read the saved jobs of `dir`, failing the ones that were running (saved so), and deleting what
/// can't be read. Blocking.
fn read_saved(dir: &Path) -> Vec<(String, Saved)> {
    let mut out = Vec::new();
    for d in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let id = d.file_name().to_string_lossy().into_owned();
        let saved = std::fs::read(d.path().join("job.json"))
            .ok()
            .and_then(|raw| serde_json::from_slice::<Saved>(&raw).ok());
        let Some(mut saved) = saved.filter(|s| s.stage != Stage::Done) else {
            // unreadable, or done but not yet deleted: nothing to show
            remove_folder(&d.path());
            continue;
        };
        if saved.stage != Stage::Failed {
            saved.stage = Stage::Failed;
            persist(&d.path(), &saved);
        }
        out.push((id, saved));
    }
    out
}

/// Sign-out: delete every saved job's ticket on the server (all at once, for at most a few
/// seconds; the server sweeps whatever is left), then the jobs folder itself. Works whether or not
/// the jobs were ever loaded; stop a loaded [`Jobs`] first.
pub async fn discard_all(dir: PathBuf, api: Api, token: String) {
    let scan = dir.clone();
    let tickets: Vec<ClipUploadId> = tokio::task::spawn_blocking(move || {
        std::fs::read_dir(&scan)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|d| std::fs::read(d.path().join("job.json")).ok())
            .filter_map(|raw| serde_json::from_slice::<Saved>(&raw).ok())
            .filter_map(|s| s.upload_id)
            .collect()
    })
    .await
    .unwrap_or_default();
    let cancels = tickets.into_iter().map(|t| {
        let (api, token) = (&api, token.as_str());
        async move {
            if let Err(e) = api.cancel_clip_upload(token, t).await {
                tracing::info!(error = %e, "clips: ticket not cancelled at sign-out");
            }
        }
    });
    if tokio::time::timeout(SIGN_OUT_LIMIT, futures::future::join_all(cancels))
        .await
        .is_err()
    {
        tracing::info!("clips: sign-out stopped waiting for ticket deletes");
    }
    let _ = tokio::task::spawn_blocking(move || remove_folder(&dir)).await;
}

impl<R: Runner + Send + Sync + 'static> Jobs<R> {
    /// The jobs saved in `dir` (`<scope>/jobs`). One that was preparing, uploading or posting when
    /// Pulse stopped is failed now, for the person to Retry; its nonce is kept, so a confirm that
    /// did land is replayed, not posted twice.
    pub async fn load(dir: PathBuf, runner: R, api: Api, token: String, hooks: Hooks) -> Self {
        let scan = dir.clone();
        let saved = tokio::task::spawn_blocking(move || read_saved(&scan))
            .await
            .unwrap_or_default();
        let mut jobs = HashMap::new();
        for (id, s) in saved {
            let thumb = dir.join(&id).join(THUMB);
            if thumb.is_file() {
                (hooks.allow_file)(&thumb);
            }
            jobs.insert(id, Entry::new(s));
        }
        Self {
            inner: Arc::new(Inner {
                dir,
                runner,
                api,
                token,
                http: upload::client(),
                hooks,
                rt: tokio::runtime::Handle::current(),
                jobs: Mutex::new(jobs),
                #[cfg(test)]
                lose_confirm: Default::default(),
                #[cfg(test)]
                confirm_gate: Default::default(),
            }),
        }
    }

    pub fn token(&self) -> &str {
        &self.inner.token
    }

    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }

    pub fn list(&self) -> Vec<ClipJob> {
        let jobs = self.inner.jobs.lock().unwrap();
        let mut out: Vec<ClipJob> = jobs.iter().map(|(id, e)| self.inner.view(id, e)).collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Save a new job and start it.
    pub fn send(&self, req: NewJob) -> Result<ClipJob, String> {
        check(&req)?;
        let id = super::editor::new_id();
        let saved = Saved {
            source_path: req.source_path,
            probe: req.probe,
            spans: req.spans,
            preset: req.preset,
            caption: req.caption.trim().to_owned(),
            game_id: req.game_id,
            nonce: format!("{:032x}", rand::random::<u128>()),
            channel_id: req.channel_id,
            stage: Stage::Preparing,
            upload_id: None,
            uploaded: false,
            thumb_hash: None,
            encoder: None,
        };
        let dir = self.inner.dir.join(&id);
        if std::fs::create_dir_all(&dir).is_err() || !persist(&dir, &saved) {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(NOT_SAVED.to_owned());
        }
        let job = {
            let mut jobs = self.inner.jobs.lock().unwrap();
            let e = jobs.entry(id.clone()).or_insert(Entry::new(saved));
            self.inner.view(&id, e)
        };
        (self.inner.hooks.emit)(&job);
        self.start(&id);
        Ok(job)
    }

    /// Try a failed job again: confirm if the file is up, upload if preparing finished, else
    /// prepare again.
    pub fn retry(&self, id: &str) -> Result<ClipJob, String> {
        let job_dir = self.inner.dir.join(id);
        let (job, saved) = {
            let mut jobs = self.inner.jobs.lock().unwrap();
            let e = jobs.get_mut(id).ok_or(NO_SUCH_JOB)?;
            if e.running {
                return Ok(self.inner.view(id, e));
            }
            e.saved.stage = if e.saved.uploaded && e.saved.upload_id.is_some() {
                Stage::Posting
            } else if prepared(&job_dir, &e.saved) {
                Stage::Uploading
            } else {
                Stage::Preparing
            };
            e.saved.uploaded &= e.saved.stage == Stage::Posting;
            e.error = None;
            e.progress = 0.0;
            e.last_emit = Some(Instant::now());
            (self.inner.view(id, e), e.saved.clone())
        };
        persist(&job_dir, &saved);
        (self.inner.hooks.emit)(&job);
        self.start(id);
        Ok(job)
    }

    /// Stop a job and remove every trace of it: ffmpeg is killed, the upload aborted, the ticket (and
    /// any object) deleted on the server, the folder deleted. The UI gets `Done` with no message.
    /// A confirm already on its way is waited for: if it posts, the job ends posted.
    pub async fn cancel(&self, id: &str) -> Result<(), String> {
        let confirming = {
            let mut jobs = self.inner.jobs.lock().unwrap();
            match jobs.get_mut(id) {
                Some(e) if e.running && e.saved.stage == Stage::Posting => e.task.take(),
                _ => None,
            }
        };
        if let Some(run) = confirming {
            let _ = run.await;
        }
        let Some(e) = self.inner.jobs.lock().unwrap().remove(id) else {
            // posted (or cancelled) meanwhile
            return Ok(());
        };
        e.cancel.cancel();
        if let Some(t) = &e.task {
            // dropping the run kills ffmpeg (`kill_on_drop`) and the PUT's connection
            t.abort();
        }
        if let Some(ticket) = e.saved.upload_id
            && let Err(err) = self
                .inner
                .api
                .cancel_clip_upload(&self.inner.token, ticket)
                .await
        {
            // gone already (404), or the server is unreachable: the sweeper takes it after 24 h
            tracing::info!(error = %err, "clips: ticket not cancelled");
        }
        self.inner.finish(id, e, None).await;
        Ok(())
    }

    /// Stop every running job and forget them all; the files stay (sign-out deletes them with
    /// [`discard_all`]; a paused session keeps them for next time).
    pub fn stop(&self) {
        for (_, e) in self.inner.jobs.lock().unwrap().drain() {
            e.cancel.cancel();
            if let Some(t) = e.task {
                t.abort();
            }
        }
    }

    /// Quitting: stop every running job and wait until each task has ended. An aborted task
    /// drops its ffmpeg child, which kills it, so none is left running after Pulse exits (the
    /// runtime's tasks are never dropped at exit). The files stay, as with [`Jobs::stop`].
    pub async fn shutdown(&self) {
        let tasks: Vec<JoinHandle<()>> = self
            .inner
            .jobs
            .lock()
            .unwrap()
            .drain()
            .filter_map(|(_, e)| {
                e.cancel.cancel();
                e.task
            })
            .collect();
        for t in tasks {
            t.abort();
            let _ = t.await;
        }
    }

    fn start(&self, id: &str) {
        let run_no = {
            let mut jobs = self.inner.jobs.lock().unwrap();
            let Some(e) = jobs.get_mut(id) else { return };
            e.run_no += 1;
            e.running = true;
            e.cancel = CancellationFlag::new();
            e.run_no
        };
        let task = self
            .inner
            .rt
            .spawn(self.inner.clone().run(id.to_owned(), run_no));
        let mut jobs = self.inner.jobs.lock().unwrap();
        match jobs.get_mut(id) {
            Some(e) if e.running && e.run_no == run_no => e.task = Some(task),
            // already over: nothing to keep
            _ => drop(task),
        }
    }

    /// Tests: the next confirm lands on the server, but its answer is "lost" on the way back.
    #[cfg(test)]
    pub fn lose_next_confirm_response(&self) {
        self.inner
            .lose_confirm
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Tests: every confirm waits for a `notify_one` on the returned gate before it is sent.
    #[cfg(test)]
    pub fn hold_confirms(&self) -> Arc<tokio::sync::Notify> {
        let gate = Arc::new(tokio::sync::Notify::new());
        *self.inner.confirm_gate.lock().unwrap() = Some(gate.clone());
        gate
    }
}

impl<R: Runner + Send + Sync + 'static> Inner<R> {
    fn view(&self, id: &str, e: &Entry) -> ClipJob {
        let thumb = self.dir.join(id).join(THUMB);
        ClipJob {
            id: id.to_owned(),
            channel_id: e.saved.channel_id,
            stage: e.saved.stage,
            progress: e.progress,
            error: e.error.clone(),
            caption: e.saved.caption.clone(),
            game_id: e.saved.game_id,
            kept_ms: kept_ms(&e.saved.spans),
            thumb_path: thumb
                .is_file()
                .then(|| thumb.to_string_lossy().into_owned()),
            message_id: None,
        }
    }

    fn snapshot(&self, id: &str, run_no: u64) -> Option<Saved> {
        let jobs = self.jobs.lock().unwrap();
        jobs.get(id)
            .filter(|e| e.running && e.run_no == run_no)
            .map(|e| e.saved.clone())
    }

    /// Change this run's job (ignored once the job was cancelled or another run owns it), save it
    /// when `save`, and tell the UI. Progress alone goes at most 4 times a second; a change held
    /// back is sent when its window ends.
    fn update(
        self: &Arc<Self>,
        id: &str,
        run_no: u64,
        save: bool,
        f: impl FnOnce(&mut Entry),
    ) -> bool {
        let (job, saved) = {
            let mut jobs = self.jobs.lock().unwrap();
            let Some(e) = jobs.get_mut(id).filter(|e| e.running && e.run_no == run_no) else {
                return false;
            };
            let (stage, progress) = (e.saved.stage, e.progress);
            f(e);
            let only_progress = !save && e.saved.stage == stage;
            if only_progress && e.progress == progress {
                return true;
            }
            let now = Instant::now();
            if only_progress && let Some(last) = e.last_emit.filter(|t| now - *t < PROGRESS_EVERY) {
                if !e.trailing {
                    e.trailing = true;
                    self.rt
                        .spawn(self.clone().trailing(id.to_owned(), last + PROGRESS_EVERY));
                }
                return true;
            }
            e.last_emit = Some(now);
            e.trailing = false;
            (self.view(id, e), save.then(|| e.saved.clone()))
        };
        if let Some(saved) = saved {
            persist(&self.dir.join(id), &saved);
        }
        (self.hooks.emit)(&job);
        true
    }

    /// The progress change held back by the rate limit, sent when its window ends (unless
    /// something was sent since).
    async fn trailing(self: Arc<Self>, id: String, at: Instant) {
        tokio::time::sleep_until(at.into()).await;
        let job = {
            let mut jobs = self.jobs.lock().unwrap();
            let Some(e) = jobs.get_mut(&id).filter(|e| e.trailing) else {
                return;
            };
            e.trailing = false;
            e.last_emit = Some(Instant::now());
            self.view(&id, e)
        };
        (self.hooks.emit)(&job);
    }

    fn progress(self: &Arc<Self>, id: &str, run_no: u64, p: f32) {
        self.update(id, run_no, false, |e| {
            if p > e.progress {
                e.progress = p;
            }
        });
    }

    fn to_stage(
        self: &Arc<Self>,
        id: &str,
        run_no: u64,
        stage: Stage,
        f: impl FnOnce(&mut Saved),
    ) -> bool {
        self.update(id, run_no, true, |e| {
            f(&mut e.saved);
            e.saved.stage = stage;
            e.progress = 0.0;
        })
    }

    async fn run(self: Arc<Self>, id: String, run_no: u64) {
        let res = loop {
            let Some(saved) = self.snapshot(&id, run_no) else {
                return;
            };
            let step = match saved.stage {
                Stage::Preparing => self.prepare(&id, run_no, saved).await,
                Stage::Uploading => self.upload(&id, run_no, saved).await,
                Stage::Posting => match self.post(&id, run_no, saved).await {
                    Ok(message) => break Ok(message),
                    Err(stop) => Err(stop),
                },
                Stage::Failed | Stage::Done => return,
            };
            if let Err(stop) = step {
                break Err(stop);
            }
        };
        match res {
            Ok(message) => {
                let e = {
                    let mut jobs = self.jobs.lock().unwrap();
                    match jobs.get(&id) {
                        Some(e) if e.running && e.run_no == run_no => jobs.remove(&id),
                        _ => None,
                    }
                };
                if let Some(e) = e {
                    self.finish(&id, e, Some(message)).await;
                }
            }
            Err(error) => {
                self.update(&id, run_no, true, |e| {
                    e.saved.stage = Stage::Failed;
                    e.error = error;
                    e.running = false;
                    e.task = None;
                });
            }
        }
    }

    /// The job is over (posted, or cancelled): its folder goes and the UI gets its last event.
    async fn finish(&self, id: &str, mut e: Entry, message: Option<MessageId>) {
        let dir = self.dir.join(id);
        (self.hooks.forbid_file)(&dir.join(THUMB));
        let _ = self.rt.spawn_blocking(move || remove_folder(&dir)).await;
        e.saved.stage = Stage::Done;
        e.progress = 1.0;
        e.error = None;
        let mut job = self.view(id, &e);
        job.thumb_path = None;
        job.message_id = message;
        (self.hooks.emit)(&job);
    }

    /// Encode the kept pieces (0 to 0.95), then make the poster (to 1.0). Both are written under a
    /// temporary name and renamed once complete.
    async fn prepare(self: &Arc<Self>, id: &str, run_no: u64, s: Saved) -> Result<(), Stop> {
        let dir = self.dir.join(id);
        let (out, out_part) = (dir.join(OUT), dir.join(OUT_PART));
        let (thumb, thumb_part) = (dir.join(THUMB), dir.join(THUMB_PART));
        for f in [&out, &out_part, &thumb_part] {
            let _ = std::fs::remove_file(f);
        }
        if s.encoder.is_some() {
            self.update(id, run_no, true, |e| e.saved.encoder = None);
        }
        let kept = kept_ms(&s.spans);
        let build =
            |enc| encode::args(&s.probe, &s.source_path, &s.spans, s.preset, enc, &out_part);
        let failed = |e: EncodeError| {
            let EncodeError::Failed { stderr_tail } = &e;
            tracing::warn!(stderr = %stderr_tail, "clips: export failed");
            let _ = std::fs::remove_file(&out_part);
            let _ = std::fs::remove_file(&thumb_part);
            Some(e.to_string())
        };
        let enc = WORKING
            .run(&self.runner, kept, build, |f| {
                self.progress(id, run_no, f * 0.95)
            })
            .await
            .map_err(failed)?;
        let size = std::fs::metadata(&out_part).map(|m| m.len()).unwrap_or(0);
        if size == 0 {
            return Err(failed(EncodeError::Failed {
                stderr_tail: "no output".into(),
            }));
        }
        if size > MAX_BYTES {
            let _ = std::fs::remove_file(&out_part);
            return Err(Some(TOO_BIG.to_owned()));
        }

        let mut args = encode::head();
        args.extend(preview::thumb_args(
            &s.source_path.to_string_lossy(),
            thumb_at(&s.spans),
            &thumb_part.to_string_lossy(),
        ));
        let made = self.runner.run(args, &mut |_| {}).await;
        if made.is_err() || !thumb_part.is_file() {
            let stderr_tail = made.err().map(|f| f.stderr_tail).unwrap_or_default();
            return Err(failed(EncodeError::Failed { stderr_tail }));
        }
        let renamed =
            std::fs::rename(&out_part, &out).and_then(|()| std::fs::rename(&thumb_part, &thumb));
        if let Err(e) = renamed {
            return Err(failed(EncodeError::Failed {
                stderr_tail: e.to_string(),
            }));
        }
        (self.hooks.allow_file)(&thumb);
        self.progress(id, run_no, 1.0);
        self.to_stage(id, run_no, Stage::Uploading, |s| {
            s.encoder = Some(enc.name().to_owned());
            s.uploaded = false;
        });
        Ok(())
    }

    /// A new ticket (the old one cancelled first), the poster, then the file.
    async fn upload(self: &Arc<Self>, id: &str, run_no: u64, s: Saved) -> Result<(), Stop> {
        let dir = self.dir.join(id);
        let out = dir.join(OUT);
        if !prepared(&dir, &s) {
            // not (or no longer) complete: make it again
            self.to_stage(id, run_no, Stage::Preparing, |_| {});
            return Ok(());
        }
        let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        let (api, token) = (&self.api, self.token.as_str());
        if let Some(old) = s.upload_id {
            if let Err(e) = api.cancel_clip_upload(token, old).await {
                tracing::info!(error = %e, "clips: old ticket not cancelled");
            }
            self.update(id, run_no, true, |e| e.saved.upload_id = None);
        }
        let ticket = api
            .clip_upload(
                token,
                s.channel_id,
                &ClipUploadRequest {
                    size,
                    duration_ms: kept_ms(&s.spans) as u32,
                },
            )
            .await
            .map_err(|e| server_words(&e))?;
        if !self.update(id, run_no, true, |e| e.saved.upload_id = Some(ticket.id)) {
            // cancelled while asking: the ticket is ours to drop
            let _ = api.cancel_clip_upload(token, ticket.id).await;
            return Ok(());
        }

        let poster = std::fs::read(dir.join(THUMB)).map_err(|_| None)?;
        let thumb = api
            .upload_clip_thumb(token, poster)
            .await
            .map_err(|e| server_words(&e))?;
        self.update(id, run_no, true, |e| e.saved.thumb_hash = Some(thumb.hash));

        let cancel = {
            let jobs = self.jobs.lock().unwrap();
            match jobs.get(id) {
                Some(e) => e.cancel.clone(),
                None => return Ok(()),
            }
        };
        let (me, job) = (self.clone(), id.to_owned());
        let res = upload::put_file(
            &self.http,
            &ticket.url,
            &out,
            &ticket.content_type,
            move |sent, total| me.progress(&job, run_no, sent as f32 / total.max(1) as f32),
            cancel,
        )
        .await;
        if let Err(e) = res {
            tracing::info!(error = %e, "clips: upload failed");
            return Err(None);
        }
        self.to_stage(id, run_no, Stage::Posting, |s| s.uploaded = true);
        Ok(())
    }

    /// Confirm with the job's nonce. When the ticket or its upload is gone (404, 409: expired,
    /// didn't arrive) Retry uploads again with a new ticket; any other refusal keeps the upload, so
    /// Retry confirms again.
    async fn post(self: &Arc<Self>, id: &str, run_no: u64, s: Saved) -> Result<MessageId, Stop> {
        let Some(ticket) = s.upload_id else {
            self.update(id, run_no, true, |e| e.saved.uploaded = false);
            return Err(None);
        };
        let thumb_hash = match s.thumb_hash.clone() {
            Some(h) => h,
            // the server lost the poster (or never had it): send it again, the video is fine
            None => {
                let poster = std::fs::read(self.dir.join(id).join(THUMB)).map_err(|_| None)?;
                let h = self
                    .api
                    .upload_clip_thumb(&self.token, poster)
                    .await
                    .map_err(|e| server_words(&e))?
                    .hash;
                self.update(id, run_no, true, |e| e.saved.thumb_hash = Some(h.clone()));
                h
            }
        };
        let (width, height) = encode::target_size(s.probe.width, s.probe.height, s.preset);
        let body = ConfirmClipRequest {
            caption: s.caption.clone(),
            game_id: s.game_id,
            duration_ms: kept_ms(&s.spans) as u32,
            width,
            height,
            thumb_hash,
            nonce: Some(s.nonce.clone()),
        };
        #[cfg(test)]
        {
            let gate = self.confirm_gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.notified().await;
            }
        }
        #[allow(unused_mut)]
        let mut res = self.api.confirm_clip(&self.token, ticket, &body).await;
        #[cfg(test)]
        if res.is_ok()
            && self
                .lose_confirm
                .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            res = Err(ApiError::Network("the answer was lost".into()));
        }
        match res {
            Ok(m) => Ok(m.id),
            Err(e @ ApiError::UploadGone(_)) => {
                self.update(id, run_no, true, |e| e.saved.uploaded = false);
                Err(server_words(&e))
            }
            Err(e @ ApiError::PosterGone(_)) => {
                self.update(id, run_no, true, |e| e.saved.thumb_hash = None);
                Err(server_words(&e))
            }
            Err(e) => Err(server_words(&e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Api;
    use crate::clips::encode::{Preset, RunFailed, Runner, Span};
    use crate::clips::probe::{AudioStream, Probe};
    use pulse_protocol::gateway::{Event, ServerFrame};
    use pulse_protocol::rest::{Channel, ChannelKind, ClipUploadRequest, Game};
    use pulse_server::testing;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// A decodable image no other call produces (the media store keeps the first uploader of
    /// identical bytes, and confirm wants the poster uploaded by me).
    fn unique_png() -> Vec<u8> {
        static N: AtomicU32 = AtomicU32::new(1);
        let n = N.fetch_add(1, Ordering::SeqCst) + std::process::id();
        let px = image::Rgba([
            (n * 53 % 251) as u8,
            (n * 97 % 241) as u8,
            (n * 29 % 239) as u8,
            255,
        ]);
        let mut img = image::RgbaImage::from_pixel(64, 36, px);
        img.put_pixel(n % 64, n / 64 % 36, image::Rgba([n as u8, 1, 2, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    /// Stands in for ffmpeg: the export writes `out_len` bytes (reporting `steps` progress lines
    /// first, then pausing `pause`), the thumbnail run writes a unique image. The first `fail`
    /// exports fail half way.
    struct FakeRunner {
        out_len: u64,
        steps: u64,
        pause: Duration,
        fail: AtomicU32,
        /// Exports in flight: an export that is dropped (its ffmpeg killed) counts down too.
        live: Arc<AtomicU32>,
    }

    impl FakeRunner {
        fn new(out_len: u64) -> Self {
            Self {
                out_len,
                steps: 3,
                pause: Duration::ZERO,
                fail: AtomicU32::new(0),
                live: Default::default(),
            }
        }
    }

    struct Live(Arc<AtomicU32>);
    impl Drop for Live {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl Runner for FakeRunner {
        async fn run(
            &self,
            args: Vec<String>,
            on_line: &mut (dyn FnMut(&str) + Send),
        ) -> Result<(), RunFailed> {
            let out = PathBuf::from(args.last().unwrap());
            if out.to_string_lossy().ends_with(".jpg") {
                std::fs::write(&out, unique_png()).unwrap();
                return Ok(());
            }
            self.live.fetch_add(1, Ordering::SeqCst);
            let _live = Live(self.live.clone());
            // the clip keeps 12.5 s: the steps go up to half of it (each block as ffmpeg prints it: frames
            // written, then the output time)
            for i in 1..=self.steps {
                on_line(&format!("frame={}", 375 * i / self.steps));
                on_line(&format!("out_time_us={}", 6_250_000 * i / self.steps));
            }
            if self
                .fail
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
            {
                std::fs::write(&out, b"half a clip").unwrap();
                return Err(RunFailed {
                    stderr_tail: "boom".into(),
                });
            }
            tokio::time::sleep(self.pause).await;
            let f = std::fs::File::create(&out).unwrap();
            if self.out_len > 64 * 1024 {
                // sparse: the size is what counts
                f.set_len(self.out_len).unwrap();
            } else {
                drop(f);
                let bytes: Vec<u8> = (0..self.out_len).map(|i| (i * 7 % 251) as u8).collect();
                std::fs::write(&out, bytes).unwrap();
            }
            on_line("progress=end");
            Ok(())
        }
    }

    struct Rig {
        app: testing::TestApp,
        api: Api,
        token: String,
        me: pulse_protocol::ids::UserId,
        channel: Channel,
        game: Game,
        root: tempfile::TempDir,
        source: PathBuf,
        events: Arc<Mutex<Vec<ClipJob>>>,
    }

    async fn rig() -> Rig {
        let app = testing::spawn().await;
        let (me, token) = testing::register(&app, "alex").await;
        let server = testing::create_server(&app, &token, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let channel = api
            .create_channel(&token, server.id, ChannelKind::Clips, "clips")
            .await
            .unwrap();
        let game = api.add_game(&token, server.id, "Valorant").await.unwrap();
        let root = tempfile::tempdir().unwrap();
        let source = root
            .path()
            .join("Valorant 2026.10.28 - 20.01.02.03.DVR.mp4");
        std::fs::write(&source, b"not really a video").unwrap();
        Rig {
            app,
            api,
            token,
            me,
            channel,
            game,
            root,
            source,
            events: Default::default(),
        }
    }

    fn probe() -> Probe {
        Probe {
            duration_ms: 40_000,
            width: 2560,
            height: 1440,
            fps: 60.0,
            video_codec: "h264".into(),
            audio: vec![AudioStream {
                index: 1,
                codec: "aac".into(),
            }],
            format_name: "mov,mp4,m4a,3gp,3g2,mj2".into(),
            title: None,
            size: 1_000_000,
        }
    }

    impl Rig {
        fn dir(&self) -> PathBuf {
            self.root.path().join("jobs")
        }

        /// `Jobs` as a fresh start of Pulse would build it.
        async fn jobs(&self, out_len: u64) -> Jobs<FakeRunner> {
            self.jobs_with(FakeRunner::new(out_len)).await
        }

        async fn jobs_with(&self, runner: FakeRunner) -> Jobs<FakeRunner> {
            let events = self.events.clone();
            Jobs::load(
                self.dir(),
                runner,
                self.api.clone(),
                self.token.clone(),
                Hooks {
                    emit: Box::new(move |j| events.lock().unwrap().push(j.clone())),
                    allow_file: Box::new(|_| {}),
                    forbid_file: Box::new(|_| {}),
                },
            )
            .await
        }

        /// PUTs the store has seen.
        fn puts(&self) -> usize {
            self.s3()
                .requests()
                .iter()
                .filter(|(m, _)| m.as_str() == "PUT")
                .count()
        }

        fn request(&self) -> NewJob {
            NewJob {
                source_path: self.source.clone(),
                probe: probe(),
                channel_id: self.channel.id,
                spans: vec![
                    Span {
                        start_ms: 2_000,
                        end_ms: 9_000,
                    },
                    Span {
                        start_ms: 20_000,
                        end_ms: 25_500,
                    },
                ],
                preset: Preset::Light,
                caption: "  ace on B  ".into(),
                game_id: Some(self.game.id),
            }
        }

        /// The last event of job `id` once it satisfies `f` (10 s at most).
        async fn until(&self, id: &str, f: impl Fn(&ClipJob) -> bool) -> ClipJob {
            for _ in 0..1000 {
                let last = self
                    .events
                    .lock()
                    .unwrap()
                    .iter()
                    .rev()
                    .find(|j| j.id == id)
                    .cloned();
                if let Some(j) = last.filter(|j| f(j)) {
                    return j;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!(
                "job {id} never got there: {:?}",
                self.events.lock().unwrap()
            );
        }

        fn stages(&self, id: &str) -> Vec<Stage> {
            let mut out: Vec<Stage> = Vec::new();
            for j in self.events.lock().unwrap().iter().filter(|j| j.id == id) {
                if out.last() != Some(&j.stage) {
                    out.push(j.stage);
                }
            }
            out
        }

        async fn clips(&self) -> Vec<pulse_protocol::rest::Message> {
            self.api
                .list_clips(&self.token, self.channel.id, None, None, None)
                .await
                .unwrap()
        }

        fn s3(&self) -> &pulse_server::testing::fake_s3::FakeS3 {
            self.app.s3.as_ref().unwrap()
        }

        fn saved(&self, id: &str) -> serde_json::Value {
            let raw = std::fs::read(self.dir().join(id).join("job.json")).unwrap();
            serde_json::from_slice(&raw).unwrap()
        }

        /// Whether the server still has this ticket (cancelling a gone one is a 404).
        async fn ticket_alive(&self, id: &str) -> bool {
            let id = id.parse().unwrap();
            self.api.cancel_clip_upload(&self.token, id).await.is_ok()
        }
    }

    fn is_dir_gone(p: &Path) -> bool {
        !p.exists()
    }

    #[tokio::test]
    async fn a_job_goes_preparing_uploading_posting_done() {
        let r = rig().await;
        let jobs = r.jobs(3 * 1024 * 1024).await;
        let job = jobs.send(r.request()).unwrap();
        assert_eq!(job.stage, Stage::Preparing);
        assert_eq!(job.kept_ms, 12_500);
        assert_eq!(job.caption, "ace on B");
        let done = r.until(&job.id, |j| j.stage == Stage::Done).await;
        assert_eq!(
            r.stages(&job.id),
            vec![
                Stage::Preparing,
                Stage::Uploading,
                Stage::Posting,
                Stage::Done
            ]
        );
        // progress only goes up within a stage
        let events = r.events.lock().unwrap().clone();
        for w in events.windows(2) {
            if w[0].stage == w[1].stage && w[0].stage != Stage::Done {
                assert!(w[1].progress >= w[0].progress, "{events:?}");
            }
        }
        let clips = r.clips().await;
        assert_eq!(clips.len(), 1);
        assert_eq!(done.message_id, Some(clips[0].id));
        assert_eq!(clips[0].content, "ace on B");
        let clip = clips[0].clip.as_ref().unwrap();
        assert_eq!(clip.game_id, Some(r.game.id));
        assert_eq!(clip.duration_ms, 12_500);
        assert_eq!((clip.width, clip.height), (1280, 720));
        assert_eq!(clip.size, 3 * 1024 * 1024);
        assert!(is_dir_gone(&r.dir().join(&job.id)));
        assert!(jobs.list().is_empty());
    }

    #[tokio::test]
    async fn a_ticket_refusal_fails_with_the_servers_words() {
        let r = rig().await;
        for _ in 0..20 {
            r.api
                .clip_upload(
                    &r.token,
                    r.channel.id,
                    &ClipUploadRequest {
                        size: 10,
                        duration_ms: 1000,
                    },
                )
                .await
                .unwrap();
        }
        let jobs = r.jobs(1024).await;
        let job = jobs.send(r.request()).unwrap();
        let failed = r.until(&job.id, |j| j.stage == Stage::Failed).await;
        assert_eq!(
            failed.error.as_deref(),
            Some("That's 20 clips this hour. Try again later.")
        );
        assert_eq!(jobs.list()[0].stage, Stage::Failed);
        assert!(r.clips().await.is_empty());
    }

    #[tokio::test]
    async fn cancel_mid_upload_leaves_nothing() {
        let r = rig().await;
        r.s3().set_delay(Duration::from_secs(30));
        let jobs = r.jobs(3 * 1024 * 1024).await;
        let job = jobs.send(r.request()).unwrap();
        r.until(&job.id, |j| j.stage == Stage::Uploading).await;
        // wait for the ticket to be taken
        let mut ticket = None;
        for _ in 0..500 {
            ticket = r.saved(&job.id)["upload_id"].as_str().map(str::to_owned);
            if ticket.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let ticket = ticket.expect("a ticket");
        tokio::time::sleep(Duration::from_millis(200)).await;
        // the PUT in flight stays held; the server's own delete on cancel needn't wait
        r.s3().set_delay(Duration::ZERO);

        jobs.cancel(&job.id).await.unwrap();
        let last = r.until(&job.id, |j| j.stage == Stage::Done).await;
        assert_eq!(last.message_id, None);
        assert!(is_dir_gone(&r.dir().join(&job.id)));
        assert!(jobs.list().is_empty());
        assert!(!r.ticket_alive(&ticket).await, "the ticket is deleted");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(r.s3().keys().is_empty(), "{:?}", r.s3().keys());
        assert!(r.clips().await.is_empty());
    }

    #[tokio::test]
    async fn restart_turns_live_jobs_into_failed() {
        let r = rig().await;
        let jobs = r.jobs(1024).await;
        let job = jobs.send(r.request()).unwrap();
        r.until(&job.id, |j| j.stage == Stage::Done).await;

        // a job that was uploading when Pulse stopped
        for (id, stage) in [
            ("aaaa", "uploading"),
            ("bbbb", "posting"),
            ("cccc", "preparing"),
        ] {
            let d = r.dir().join(id);
            std::fs::create_dir_all(&d).unwrap();
            let saved = serde_json::json!({
                "source_path": r.source,
                "probe": probe(),
                "spans": [{ "start_ms": 0, "end_ms": 5000 }],
                "preset": "sharp",
                "caption": "clutch",
                "game_id": null,
                "nonce": format!("nonce-{id}"),
                "channel_id": r.channel.id,
                "stage": stage,
                "upload_id": null,
                "uploaded": false,
                "thumb_hash": null,
                "encoder": null,
            });
            std::fs::write(d.join("job.json"), saved.to_string()).unwrap();
        }
        std::fs::write(r.dir().join("bbbb").join("thumb.jpg"), unique_png()).unwrap();

        let jobs = r.jobs(1024).await;
        let mut list = jobs.list();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(list.len(), 3);
        for j in &list {
            assert_eq!(j.stage, Stage::Failed, "{j:?}");
            assert_eq!(
                j.error, None,
                "the card says \"Upload failed — Retry\" itself"
            );
            assert_eq!(j.caption, "clutch");
            assert_eq!(j.kept_ms, 5000);
            assert_eq!(j.channel_id, r.channel.id);
        }
        assert!(list[0].thumb_path.is_none());
        assert!(
            list[1]
                .thumb_path
                .as_deref()
                .unwrap()
                .ends_with("thumb.jpg")
        );
        // saved as failed, so the next start agrees
        assert_eq!(r.saved("aaaa")["stage"], "failed");
        assert_eq!(r.saved("aaaa")["nonce"], "nonce-aaaa");
    }

    #[tokio::test]
    async fn retry_after_a_failed_upload_posts_once() {
        let r = rig().await;
        r.s3().set_down(true);
        let jobs = r.jobs(2 * 1024 * 1024).await;
        let job = jobs.send(r.request()).unwrap();
        r.until(&job.id, |j| j.stage == Stage::Failed).await;
        let old_ticket = r.saved(&job.id)["upload_id"]
            .as_str()
            .expect("the ticket is kept for Retry to cancel")
            .to_owned();
        assert_eq!(r.saved(&job.id)["uploaded"], false);
        r.s3().set_down(false);
        drop(jobs);

        // Pulse restarts, then Retry
        let jobs = r.jobs(2 * 1024 * 1024).await;
        assert_eq!(jobs.list()[0].stage, Stage::Failed);
        let mut rx = r.app.hub.register(r.me, String::new()).rx;
        jobs.retry(&job.id).unwrap();
        let done = r.until(&job.id, |j| j.stage == Stage::Done).await;
        // the encode wasn't redone: out.mp4 was still there
        assert_eq!(
            r.stages(&job.id)
                .iter()
                .filter(|s| **s == Stage::Preparing)
                .count(),
            1
        );
        let clips = r.clips().await;
        assert_eq!(clips.len(), 1);
        assert_eq!(done.message_id, Some(clips[0].id));
        assert!(
            !r.ticket_alive(&old_ticket).await,
            "the old ticket is deleted"
        );
        assert_eq!(created(&mut rx).await, 1);
    }

    #[tokio::test]
    async fn a_lost_confirm_response_never_double_posts() {
        let r = rig().await;
        let jobs = r.jobs(1024 * 1024).await;
        jobs.lose_next_confirm_response();
        let mut rx = r.app.hub.register(r.me, String::new()).rx;
        let job = jobs.send(r.request()).unwrap();
        let failed = r.until(&job.id, |j| j.stage == Stage::Failed).await;
        assert_eq!(
            failed.error, None,
            "a lost response isn't the server's words"
        );
        assert_eq!(r.saved(&job.id)["uploaded"], true);
        // the clip did post
        let first = r.clips().await;
        assert_eq!(first.len(), 1);
        drop(jobs);

        // Retry after a restart: confirm again with the same nonce
        let jobs = r.jobs(1024 * 1024).await;
        jobs.retry(&job.id).unwrap();
        let done = r.until(&job.id, |j| j.stage == Stage::Done).await;
        assert_eq!(done.message_id, Some(first[0].id));
        assert_eq!(r.clips().await.len(), 1);
        assert_eq!(created(&mut rx).await, 1);
    }

    #[tokio::test]
    async fn an_oversized_output_fails_before_upload() {
        let r = rig().await;
        let jobs = r.jobs(MAX_BYTES + 1).await;
        let job = jobs.send(r.request()).unwrap();
        let failed = r.until(&job.id, |j| j.stage == Stage::Failed).await;
        assert_eq!(
            failed.error.as_deref(),
            Some("This clip came out over 200 MB. Try Light.")
        );
        assert!(!r.stages(&job.id).contains(&Stage::Uploading));
        assert!(
            r.saved(&job.id)["upload_id"].is_null(),
            "no ticket asked for"
        );
        assert!(!r.dir().join(&job.id).join("out.mp4").exists());
        assert!(r.s3().keys().is_empty());
    }

    #[tokio::test]
    async fn bad_requests_are_refused_up_front() {
        let r = rig().await;
        let jobs = r.jobs(1024).await;
        let mut req = r.request();
        req.spans.clear();
        assert_eq!(
            jobs.send(req).unwrap_err(),
            "Nothing kept. Bring a piece back."
        );
        let mut req = r.request();
        req.spans = vec![Span {
            start_ms: 0,
            end_ms: 90_001,
        }];
        req.probe.duration_ms = 120_000;
        assert_eq!(
            jobs.send(req).unwrap_err(),
            "Clips can be up to 1:30. Drop a bit more."
        );
        let mut req = r.request();
        req.spans = vec![Span {
            start_ms: 5_000,
            end_ms: 41_000,
        }];
        assert!(jobs.send(req).is_err(), "past the end of the source");
        let mut req = r.request();
        req.spans.reverse();
        assert!(jobs.send(req).is_err(), "out of order");
        assert!(jobs.list().is_empty());
        assert!(!r.dir().exists() || std::fs::read_dir(r.dir()).unwrap().next().is_none());
    }

    /// Save a job as a stop in the middle of preparing leaves it (restart material).
    fn save_job(r: &Rig, id: &str, stage: &str, encoder: Option<&str>) -> PathBuf {
        let d = r.dir().join(id);
        std::fs::create_dir_all(&d).unwrap();
        let saved = serde_json::json!({
            "source_path": r.source,
            "probe": probe(),
            "spans": [{ "start_ms": 0, "end_ms": 5000 }],
            "preset": "light",
            "caption": "clutch",
            "game_id": null,
            "nonce": format!("nonce-{id}"),
            "channel_id": r.channel.id,
            "stage": stage,
            "upload_id": null,
            "uploaded": false,
            "thumb_hash": null,
            "encoder": encoder,
        });
        std::fs::write(d.join("job.json"), saved.to_string()).unwrap();
        d
    }

    #[tokio::test]
    async fn retry_after_a_stop_mid_prepare_prepares_again() {
        let r = rig().await;
        // stopped mid-encode: a partial export, no poster
        let d = save_job(&r, "aaaa", "preparing", None);
        std::fs::write(d.join("out.mp4.part"), b"half").unwrap();
        std::fs::write(d.join("out.mp4"), b"half, from an older build").unwrap();
        // stopped making the poster: the export is whole, the poster isn't
        let d = save_job(&r, "bbbb", "preparing", None);
        std::fs::write(d.join("out.mp4"), vec![1u8; 4096]).unwrap();
        std::fs::write(d.join("thumb.part.jpg"), b"half a jpeg").unwrap();
        // finished files but no encoder recorded: not trusted either
        let d = save_job(&r, "cccc", "uploading", None);
        std::fs::write(d.join("out.mp4"), vec![1u8; 4096]).unwrap();
        std::fs::write(d.join("thumb.jpg"), unique_png()).unwrap();

        let jobs = r.jobs(2 * 1024 * 1024).await;
        for id in ["aaaa", "bbbb", "cccc"] {
            let again = jobs.retry(id).unwrap();
            assert_eq!(again.stage, Stage::Preparing, "{id}");
            let done = r.until(id, |j| j.stage == Stage::Done).await;
            assert!(done.message_id.is_some());
        }
        let clips = r.clips().await;
        assert_eq!(clips.len(), 3);
        for c in clips {
            assert_eq!(c.clip.unwrap().size, 2 * 1024 * 1024, "the fresh export");
        }
        assert_eq!(r.puts(), 3);
    }

    #[tokio::test]
    async fn retry_after_an_encoder_failure_prepares_again() {
        let r = rig().await;
        let runner = FakeRunner::new(1024 * 1024);
        runner.fail.store(1, Ordering::SeqCst);
        let jobs = r.jobs_with(runner).await;
        let job = jobs.send(r.request()).unwrap();
        let failed = r.until(&job.id, |j| j.stage == Stage::Failed).await;
        assert_eq!(
            failed.error.as_deref(),
            Some("Pulse couldn't prepare that clip.")
        );
        let d = r.dir().join(&job.id);
        assert!(!d.join("out.mp4").exists() && !d.join("out.mp4.part").exists());
        assert_eq!(jobs.retry(&job.id).unwrap().stage, Stage::Preparing);
        r.until(&job.id, |j| j.stage == Stage::Done).await;
        assert_eq!(r.clips().await.len(), 1);
    }

    #[tokio::test]
    async fn cancel_while_posting_ends_as_posted() {
        let r = rig().await;
        let jobs = r.jobs(1024 * 1024).await;
        let gate = jobs.hold_confirms();
        let job = jobs.send(r.request()).unwrap();
        r.until(&job.id, |j| j.stage == Stage::Posting).await;
        let (j2, id) = (jobs.clone(), job.id.clone());
        let cancel = tokio::spawn(async move { j2.cancel(&id).await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        gate.notify_one();
        cancel.await.unwrap().unwrap();
        let last = r.until(&job.id, |j| j.stage == Stage::Done).await;
        let clips = r.clips().await;
        assert_eq!(clips.len(), 1);
        assert_eq!(
            last.message_id,
            Some(clips[0].id),
            "it posted, so it ends posted"
        );
        assert_eq!(r.s3().keys().len(), 1, "the clip keeps its video");
        assert!(jobs.list().is_empty());
    }

    #[tokio::test]
    async fn a_gone_upload_on_confirm_uploads_again_on_retry() {
        let r = rig().await;
        let jobs = r.jobs(1024 * 1024).await;
        let gate = jobs.hold_confirms();
        let job = jobs.send(r.request()).unwrap();
        r.until(&job.id, |j| j.stage == Stage::Posting).await;
        // the ticket is gone before the confirm arrives (expired and swept, say)
        let ticket = r.saved(&job.id)["upload_id"].as_str().unwrap().to_owned();
        r.api
            .cancel_clip_upload(&r.token, ticket.parse().unwrap())
            .await
            .unwrap();
        gate.notify_one();
        let failed = r.until(&job.id, |j| j.stage == Stage::Failed).await;
        assert!(failed.error.is_some(), "the server's words");
        assert_eq!(r.saved(&job.id)["uploaded"], false);

        gate.notify_one();
        jobs.retry(&job.id).unwrap();
        r.until(&job.id, |j| j.stage == Stage::Done).await;
        assert_eq!(r.puts(), 2, "uploaded again with a new ticket");
        assert_eq!(r.clips().await.len(), 1);
    }

    #[tokio::test]
    async fn a_missing_poster_on_confirm_sends_the_poster_again() {
        let r = rig().await;
        // an uploaded job (restart material) whose poster the server doesn't have
        let d = save_job(&r, "pppp", "posting", Some("libx264"));
        let video = vec![5u8; 1000];
        std::fs::write(d.join("out.mp4"), &video).unwrap();
        std::fs::write(d.join("thumb.jpg"), unique_png()).unwrap();
        let ticket = r
            .api
            .clip_upload(
                &r.token,
                r.channel.id,
                &ClipUploadRequest {
                    size: 1000,
                    duration_ms: 5000,
                },
            )
            .await
            .unwrap();
        let put = reqwest::Client::new()
            .put(&ticket.url)
            .header("content-type", &ticket.content_type)
            .body(video)
            .send()
            .await
            .unwrap();
        assert!(put.status().is_success());
        let mut saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(d.join("job.json")).unwrap()).unwrap();
        saved["upload_id"] = serde_json::json!(ticket.id);
        saved["uploaded"] = serde_json::json!(true);
        saved["thumb_hash"] = serde_json::json!("0".repeat(64));
        std::fs::write(d.join("job.json"), saved.to_string()).unwrap();

        let jobs = r.jobs(1024).await;
        assert_eq!(jobs.retry("pppp").unwrap().stage, Stage::Posting);
        let failed = r.until("pppp", |j| j.stage == Stage::Failed).await;
        assert_eq!(
            failed.error.as_deref(),
            Some("The preview image is missing. Try again.")
        );
        assert!(r.saved("pppp")["thumb_hash"].is_null());
        assert_eq!(r.saved("pppp")["uploaded"], true);

        assert_eq!(jobs.retry("pppp").unwrap().stage, Stage::Posting);
        r.until("pppp", |j| j.stage == Stage::Done).await;
        assert_eq!(r.clips().await.len(), 1);
        assert_eq!(r.puts(), 1, "the video wasn't sent again");
    }

    #[tokio::test]
    async fn a_mismatched_upload_on_confirm_uploads_again_at_once() {
        let r = rig().await;
        let jobs = r.jobs(1024 * 1024).await;
        let gate = jobs.hold_confirms();
        let job = jobs.send(r.request()).unwrap();
        r.until(&job.id, |j| j.stage == Stage::Posting).await;
        // the stored object isn't the size the ticket says
        let key = r.s3().keys().pop().unwrap();
        r.s3().put_raw(&key, vec![0u8; 10]);
        gate.notify_one();
        let failed = r.until(&job.id, |j| j.stage == Stage::Failed).await;
        assert_eq!(
            failed.error.as_deref(),
            Some("The upload didn't match. Try again.")
        );
        assert_eq!(r.saved(&job.id)["uploaded"], false);
        gate.notify_one();
        assert_eq!(jobs.retry(&job.id).unwrap().stage, Stage::Uploading);
        r.until(&job.id, |j| j.stage == Stage::Done).await;
        assert_eq!(r.clips().await.len(), 1);
    }

    #[tokio::test]
    async fn a_refused_confirm_keeps_the_upload() {
        let r = rig().await;
        // a game of another server
        let other = testing::create_server(&r.app, &r.token, "Other").await;
        let stranger = r.api.add_game(&r.token, other.id, "Tetris").await.unwrap();
        let jobs = r.jobs(1024 * 1024).await;
        let mut req = r.request();
        req.game_id = Some(stranger.id);
        let job = jobs.send(req).unwrap();
        let failed = r.until(&job.id, |j| j.stage == Stage::Failed).await;
        assert_eq!(
            failed.error.as_deref(),
            Some("That game isn't in this server.")
        );
        assert_eq!(r.saved(&job.id)["uploaded"], true);
        jobs.retry(&job.id).unwrap();
        r.until(&job.id, |j| j.stage == Stage::Posting).await;
        r.until(&job.id, |j| j.stage == Stage::Failed).await;
        assert_eq!(r.puts(), 1, "Retry confirms again, it doesn't upload again");
    }

    #[tokio::test]
    async fn a_store_outage_on_confirm_keeps_the_upload() {
        let r = rig().await;
        let jobs = r.jobs(1024 * 1024).await;
        let gate = jobs.hold_confirms();
        let job = jobs.send(r.request()).unwrap();
        r.until(&job.id, |j| j.stage == Stage::Posting).await;
        r.s3().set_down(true);
        gate.notify_one();
        let failed = r.until(&job.id, |j| j.stage == Stage::Failed).await;
        assert_eq!(failed.error, None);
        assert_eq!(r.saved(&job.id)["uploaded"], true);
        r.s3().set_down(false);
        gate.notify_one();
        jobs.retry(&job.id).unwrap();
        r.until(&job.id, |j| j.stage == Stage::Done).await;
        assert_eq!(r.puts(), 1);
        assert_eq!(r.clips().await.len(), 1);
    }

    #[tokio::test]
    async fn progress_is_rate_limited_and_the_last_value_still_arrives() {
        let r = rig().await;
        let mut runner = FakeRunner::new(1024);
        runner.steps = 50;
        runner.pause = Duration::from_millis(700);
        let jobs = r.jobs_with(runner).await;
        let job = jobs.send(r.request()).unwrap();
        r.until(&job.id, |j| j.stage == Stage::Uploading).await;
        let preparing: Vec<f32> = r
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|j| j.id == job.id && j.stage == Stage::Preparing)
            .map(|j| j.progress)
            .collect();
        // 50 progress lines in a burst: the first, then the last of the burst once its window
        // ends (during the pause), then the poster's 1.0
        assert!(preparing.len() <= 5, "{preparing:?}");
        let last_of_burst = 0.95 * 0.5;
        let at = preparing
            .iter()
            .position(|p| (p - last_of_burst).abs() < 1e-4)
            .unwrap_or_else(|| panic!("the held-back value never arrived: {preparing:?}"));
        assert_eq!(preparing[at + 1], 1.0);
    }

    #[tokio::test]
    async fn sign_out_deletes_tickets_and_the_folder() {
        let r = rig().await;
        r.s3().set_delay(Duration::from_secs(30));
        let jobs = r.jobs(3 * 1024 * 1024).await;
        let job = jobs.send(r.request()).unwrap();
        r.until(&job.id, |j| j.stage == Stage::Uploading).await;
        let mut ticket = None;
        for _ in 0..500 {
            ticket = r.saved(&job.id)["upload_id"].as_str().map(str::to_owned);
            if ticket.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let ticket = ticket.expect("a ticket");
        r.s3().set_delay(Duration::ZERO);
        // and a failed job from an earlier run that was never loaded, holding a ticket
        let d = save_job(&r, "ffff", "failed", None);
        let old = r
            .api
            .clip_upload(
                &r.token,
                r.channel.id,
                &ClipUploadRequest {
                    size: 10,
                    duration_ms: 1000,
                },
            )
            .await
            .unwrap();
        let mut saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(d.join("job.json")).unwrap()).unwrap();
        saved["upload_id"] = serde_json::json!(old.id);
        std::fs::write(d.join("job.json"), saved.to_string()).unwrap();

        let started = std::time::Instant::now();
        jobs.stop();
        discard_all(r.dir(), r.api.clone(), r.token.clone()).await;
        assert!(started.elapsed() < Duration::from_secs(6));
        assert!(!r.dir().exists());
        assert!(!r.ticket_alive(&ticket).await);
        assert!(!r.ticket_alive(&old.id.to_string()).await);
        assert!(jobs.list().is_empty());
    }

    /// Quitting: the running export's ffmpeg is gone by the time `shutdown` returns (`stop` only
    /// asks the tasks to end).
    #[tokio::test]
    async fn shutting_down_lets_go_of_running_exports() {
        let r = rig().await;
        let mut runner = FakeRunner::new(1024);
        runner.pause = Duration::from_secs(30);
        let live = runner.live.clone();
        let jobs = r.jobs_with(runner).await;
        jobs.send(r.request()).unwrap();
        for _ in 0..500 {
            if live.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(live.load(Ordering::SeqCst), 1, "exporting");
        tokio::time::timeout(Duration::from_secs(2), jobs.shutdown())
            .await
            .expect("shutdown waits for the tasks, not the exports");
        assert_eq!(live.load(Ordering::SeqCst), 0, "the export was dropped");
        assert!(jobs.list().is_empty());
    }

    #[test]
    fn the_thumbnail_is_at_one_second_of_kept_time() {
        let spans = [
            Span {
                start_ms: 500,
                end_ms: 900,
            },
            Span {
                start_ms: 3_000,
                end_ms: 10_000,
            },
        ];
        // 400 ms in the first piece, then 600 ms into the second
        assert_eq!(thumb_at(&spans), 3_600);
        // under 2 s kept: half way
        let short = [Span {
            start_ms: 1_000,
            end_ms: 2_500,
        }];
        assert_eq!(thumb_at(&short), 1_750);
    }

    async fn created(rx: &mut tokio::sync::mpsc::Receiver<ServerFrame>) -> usize {
        let mut n = 0;
        while let Ok(Some(f)) = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await {
            if matches!(f, ServerFrame::Event(Event::MessageCreated { .. })) {
                n += 1;
            }
        }
        n
    }
}
