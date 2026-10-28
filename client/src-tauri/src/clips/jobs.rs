//! Sending a clip: one job per clip that prepares it (the export encode, then the thumbnail),
//! uploads it (ticket, poster, the streamed PUT) and posts it (confirm, with a nonce kept from the
//! start so a retried confirm can never post twice).
//!
//! Each job lives in `<scope>/jobs/<id>/` (`job.json`, `out.mp4`, `thumb.jpg`), per account like the
//! media cache, so a restart can show it again (failed, with Retry). Every change goes to the UI
//! as the whole [`ClipJob`] (`clip://job`), progress at most 4 times a second.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pulse_protocol::ids::{ChannelId, ClipUploadId, GameId, MessageId};
use pulse_protocol::rest::{ClipUploadRequest, ConfirmClipRequest};
use serde::{Deserialize, Serialize};
use tokio::task::AbortHandle;

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

/// Progress alone goes to the UI at most this often (stage changes always go at once).
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

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
    /// The current ticket. Retry cancels it before asking for a new one.
    upload_id: Option<ClipUploadId>,
    /// The store took the file for `upload_id`: Retry confirms rather than uploading again.
    #[serde(default)]
    uploaded: bool,
    thumb_hash: Option<String>,
    /// The encoder that made `out.mp4` (for the log and bug reports).
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
    task: Option<AbortHandle>,
    cancel: CancellationFlag,
    last_emit: Option<Instant>,
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
        | ApiError::Forbidden(message) => Some(message.clone()),
        _ => None,
    }
}

/// Delete a job's folder. ffmpeg killed a moment ago may still hold `out.mp4` on Windows, so a
/// failure is retried briefly.
fn remove_folder(dir: &Path) {
    for attempt in 0..10 {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => return,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) if attempt == 9 => {
                tracing::warn!(error = %e, "clips: job folder not removed");
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

impl<R: Runner + Send + Sync + 'static> Jobs<R> {
    /// The jobs saved in `dir` (`<scope>/jobs`). One that was preparing, uploading or posting when
    /// Pulse stopped is failed now, for the person to Retry; its nonce is kept, so a confirm that
    /// did land is replayed, not posted twice. Call from inside the async runtime.
    pub fn load(dir: PathBuf, runner: R, api: Api, token: String, hooks: Hooks) -> Self {
        let inner = Inner {
            dir,
            runner,
            api,
            token,
            http: upload::client(),
            hooks,
            rt: tokio::runtime::Handle::current(),
            jobs: Mutex::new(HashMap::new()),
            #[cfg(test)]
            lose_confirm: Default::default(),
        };
        let mut jobs = HashMap::new();
        for d in std::fs::read_dir(&inner.dir)
            .into_iter()
            .flatten()
            .flatten()
        {
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
                inner.persist(&id, &saved);
            }
            let thumb = d.path().join("thumb.jpg");
            if thumb.is_file() {
                (inner.hooks.allow_file)(&thumb);
            }
            jobs.insert(
                id,
                Entry {
                    saved,
                    progress: 0.0,
                    error: None,
                    running: false,
                    run_no: 0,
                    task: None,
                    cancel: CancellationFlag::new(),
                    last_emit: None,
                },
            );
        }
        *inner.jobs.lock().unwrap() = jobs;
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn token(&self) -> &str {
        &self.inner.token
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
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        if !self.inner.persist(&id, &saved) {
            remove_folder(&dir);
            return Err("Pulse couldn't save that clip to send.".to_owned());
        }
        let job = {
            let mut jobs = self.inner.jobs.lock().unwrap();
            let e = jobs.entry(id.clone()).or_insert(Entry {
                saved,
                progress: 0.0,
                error: None,
                running: false,
                run_no: 0,
                task: None,
                cancel: CancellationFlag::new(),
                last_emit: None,
            });
            self.inner.view(&id, e)
        };
        (self.inner.hooks.emit)(&job);
        self.start(&id);
        Ok(job)
    }

    /// Try a failed job again: confirm if the file is up, upload if it is made, else prepare.
    pub fn retry(&self, id: &str) -> Result<ClipJob, String> {
        let job = {
            let mut jobs = self.inner.jobs.lock().unwrap();
            let e = jobs.get_mut(id).ok_or("That clip isn't being sent.")?;
            if e.running {
                return Ok(self.inner.view(id, e));
            }
            let made = self.inner.dir.join(id).join("out.mp4").is_file();
            e.saved.stage = if e.saved.uploaded && e.saved.upload_id.is_some() {
                Stage::Posting
            } else if made {
                Stage::Uploading
            } else {
                Stage::Preparing
            };
            e.saved.uploaded &= e.saved.stage == Stage::Posting;
            e.error = None;
            e.progress = 0.0;
            self.inner.persist(id, &e.saved);
            e.last_emit = Some(Instant::now());
            self.inner.view(id, e)
        };
        (self.inner.hooks.emit)(&job);
        self.start(id);
        Ok(job)
    }

    /// Stop a job and remove every trace of it: ffmpeg is killed, the upload aborted, the ticket (and
    /// any object) deleted on the server, the folder deleted. The UI gets `Done` with no message.
    pub async fn cancel(&self, id: &str) -> Result<(), String> {
        let Some(e) = self.inner.jobs.lock().unwrap().remove(id) else {
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

    /// Sign-out: cancel every job (tickets included), then delete the whole jobs folder.
    pub async fn cancel_all(&self) {
        let ids: Vec<String> = self.inner.jobs.lock().unwrap().keys().cloned().collect();
        for id in ids {
            let _ = self.cancel(&id).await;
        }
        self.abandon();
    }

    /// Stop every job and delete the jobs folder, with no server calls (the session is gone).
    pub fn abandon(&self) {
        for (_, e) in self.inner.jobs.lock().unwrap().drain() {
            e.cancel.cancel();
            if let Some(t) = e.task {
                t.abort();
            }
        }
        let dir = self.inner.dir.clone();
        self.inner.rt.spawn_blocking(move || remove_folder(&dir));
    }

    /// Stop the running work but keep everything on disk (the session paused, not signed out).
    pub fn stop(&self) {
        for e in self.inner.jobs.lock().unwrap().values_mut() {
            e.cancel.cancel();
            if let Some(t) = e.task.take() {
                t.abort();
            }
            e.running = false;
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
        if let Some(e) = self.inner.jobs.lock().unwrap().get_mut(id)
            && e.running
            && e.run_no == run_no
        {
            e.task = Some(task.abort_handle());
        }
    }

    /// Tests: the next confirm lands on the server, but its answer is "lost" on the way back.
    #[cfg(test)]
    pub fn lose_next_confirm_response(&self) {
        self.inner
            .lose_confirm
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl<R: Runner + Send + Sync + 'static> Inner<R> {
    fn view(&self, id: &str, e: &Entry) -> ClipJob {
        let thumb = self.dir.join(id).join("thumb.jpg");
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

    /// Write `job.json` (through a temp file, so a crash never leaves half of one).
    fn persist(&self, id: &str, saved: &Saved) -> bool {
        let dir = self.dir.join(id);
        let tmp = dir.join("job.json.tmp");
        let ok = serde_json::to_vec(saved)
            .map_err(std::io::Error::other)
            .and_then(|raw| std::fs::write(&tmp, raw))
            .and_then(|()| std::fs::rename(&tmp, dir.join("job.json")));
        if let Err(e) = &ok {
            tracing::warn!(error = %e, "clips: job not saved");
        }
        ok.is_ok()
    }

    fn snapshot(&self, id: &str, run_no: u64) -> Option<Saved> {
        let jobs = self.jobs.lock().unwrap();
        jobs.get(id)
            .filter(|e| e.running && e.run_no == run_no)
            .map(|e| e.saved.clone())
    }

    /// Change this run's job (ignored once the job was cancelled or another run owns it), save it
    /// when `save`, and tell the UI (progress alone at most 4 times a second).
    fn update(&self, id: &str, run_no: u64, save: bool, f: impl FnOnce(&mut Entry)) -> bool {
        let job = {
            let mut jobs = self.jobs.lock().unwrap();
            let Some(e) = jobs.get_mut(id).filter(|e| e.running && e.run_no == run_no) else {
                return false;
            };
            let (stage, progress) = (e.saved.stage, e.progress);
            f(e);
            if save {
                self.persist(id, &e.saved);
            }
            let only_progress = e.saved.stage == stage && e.progress != progress && !save;
            let now = Instant::now();
            if only_progress && e.last_emit.is_some_and(|t| now - t < PROGRESS_EVERY) {
                return true;
            }
            e.last_emit = Some(now);
            self.view(id, e)
        };
        (self.hooks.emit)(&job);
        true
    }

    fn progress(&self, id: &str, run_no: u64, p: f32) {
        self.update(id, run_no, false, |e| {
            if p > e.progress {
                e.progress = p;
            }
        });
    }

    fn to_stage(&self, id: &str, run_no: u64, stage: Stage, f: impl FnOnce(&mut Saved)) -> bool {
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
        (self.hooks.forbid_file)(&dir.join("thumb.jpg"));
        let _ = self.rt.spawn_blocking(move || remove_folder(&dir)).await;
        e.saved.stage = Stage::Done;
        e.progress = 1.0;
        e.error = None;
        let mut job = self.view(id, &e);
        job.thumb_path = None;
        job.message_id = message;
        (self.hooks.emit)(&job);
    }

    /// Encode the kept pieces (0 to 0.95), then make the poster (to 1.0).
    async fn prepare(&self, id: &str, run_no: u64, s: Saved) -> Result<(), Stop> {
        let dir = self.dir.join(id);
        let (out, thumb) = (dir.join("out.mp4"), dir.join("thumb.jpg"));
        let _ = std::fs::remove_file(&out);
        let kept = kept_ms(&s.spans);
        let build = |enc| encode::args(&s.probe, &s.source_path, &s.spans, s.preset, enc, &out);
        let failed = |e: EncodeError| {
            let EncodeError::Failed { stderr_tail } = &e;
            tracing::warn!(stderr = %stderr_tail, "clips: export failed");
            let _ = std::fs::remove_file(&out);
            Some(e.to_string())
        };
        let enc = WORKING
            .run(&self.runner, kept, build, |f| {
                self.progress(id, run_no, f * 0.95)
            })
            .await
            .map_err(failed)?;
        let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        if size == 0 {
            return Err(failed(EncodeError::Failed {
                stderr_tail: "no output".into(),
            }));
        }
        if size > MAX_BYTES {
            let _ = std::fs::remove_file(&out);
            return Err(Some(TOO_BIG.to_owned()));
        }

        let mut args = encode::head();
        args.extend(preview::thumb_args(
            &s.source_path.to_string_lossy(),
            thumb_at(&s.spans),
            &thumb.to_string_lossy(),
        ));
        let made = self.runner.run(args, &mut |_| {}).await;
        if made.is_err() || !thumb.is_file() {
            let stderr_tail = made.err().map(|f| f.stderr_tail).unwrap_or_default();
            return Err(failed(EncodeError::Failed { stderr_tail }));
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
        let out = dir.join("out.mp4");
        let Ok(meta) = std::fs::metadata(&out) else {
            // the file went (cleaned up by hand?): make it again
            self.to_stage(id, run_no, Stage::Preparing, |_| {});
            return Ok(());
        };
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
                    size: meta.len(),
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

        let poster = std::fs::read(dir.join("thumb.jpg")).map_err(|_| None)?;
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

    /// Confirm with the job's nonce. A refusal that means the upload is no good (the ticket expired,
    /// the file didn't arrive) sends Retry back to uploading with a new ticket.
    async fn post(&self, id: &str, run_no: u64, s: Saved) -> Result<MessageId, Stop> {
        let (Some(ticket), Some(thumb_hash)) = (s.upload_id, s.thumb_hash.clone()) else {
            self.update(id, run_no, true, |e| e.saved.uploaded = false);
            return Err(None);
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
            Err(e @ (ApiError::Rejected(_) | ApiError::Forbidden(_))) => {
                self.update(id, run_no, true, |e| e.saved.uploaded = false);
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

    /// Stands in for ffmpeg: the export writes `out_len` bytes (reporting progress first), the
    /// thumbnail run writes a unique image.
    struct FakeRunner {
        out_len: u64,
    }

    impl Runner for FakeRunner {
        async fn run(
            &self,
            args: Vec<String>,
            on_line: &mut (dyn FnMut(&str) + Send),
        ) -> Result<(), RunFailed> {
            let out = PathBuf::from(args.last().unwrap());
            if out.file_name().is_some_and(|n| n == "thumb.jpg") {
                std::fs::write(&out, unique_png()).unwrap();
                return Ok(());
            }
            for us in [250_000, 500_000, 1_000_000] {
                on_line(&format!("out_time_us={us}"));
            }
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
        fn jobs(&self, out_len: u64) -> Jobs<FakeRunner> {
            let events = self.events.clone();
            Jobs::load(
                self.dir(),
                FakeRunner { out_len },
                self.api.clone(),
                self.token.clone(),
                Hooks {
                    emit: Box::new(move |j| events.lock().unwrap().push(j.clone())),
                    allow_file: Box::new(|_| {}),
                    forbid_file: Box::new(|_| {}),
                },
            )
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
        let jobs = r.jobs(3 * 1024 * 1024);
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
        let jobs = r.jobs(1024);
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
        let jobs = r.jobs(3 * 1024 * 1024);
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
        let jobs = r.jobs(1024);
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

        let jobs = r.jobs(1024);
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
        let jobs = r.jobs(2 * 1024 * 1024);
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
        let jobs = r.jobs(2 * 1024 * 1024);
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
        let jobs = r.jobs(1024 * 1024);
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
        let jobs = r.jobs(1024 * 1024);
        jobs.retry(&job.id).unwrap();
        let done = r.until(&job.id, |j| j.stage == Stage::Done).await;
        assert_eq!(done.message_id, Some(first[0].id));
        assert_eq!(r.clips().await.len(), 1);
        assert_eq!(created(&mut rx).await, 1);
    }

    #[tokio::test]
    async fn an_oversized_output_fails_before_upload() {
        let r = rig().await;
        let jobs = r.jobs(MAX_BYTES + 1);
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
        let jobs = r.jobs(1024);
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
