//! Sending a message with files (#57): each staged file is uploaded straight to storage on a URL
//! the server signs, then the message is sent with their ids. The UI's pending row resolves on the
//! same `pulse://outbox` event as a plain send; `attachments://progress` drives its bar.
//!
//! A send that fails keeps what it uploaded, so trying again (the same nonce) uploads only the
//! rest. Nothing here is persisted: a restart forgets it, and the server sweeps the uploads.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pulse_protocol::ids::{AttachmentId, ChannelId, MessageId};
use pulse_protocol::rest::{
    AttachmentUpload, Message, MultipartInfo, SendMessageRequest, StartAttachmentRequest,
};
use serde::Serialize;

use super::{Staged, Stager};
use crate::api::{Api, ApiError};
use crate::clips::upload::{self, CancellationFlag, UploadError};

/// What the UI asks to send.
#[derive(Clone, Debug)]
pub struct Outgoing {
    pub channel: ChannelId,
    pub content: String,
    pub reply_to: Option<MessageId>,
    pub thread_root: Option<MessageId>,
    pub nonce: String,
    pub staged: Vec<String>,
    pub temp_days: Option<u8>,
}

/// `attachments://progress`: bytes stored so far over all of a send's files.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Progress {
    pub nonce: String,
    pub sent: u64,
    pub total: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("{0}")]
    Api(#[from] ApiError),
    #[error("{0}")]
    Upload(#[from] UploadError),
    #[error("That file is no longer queued. Add it again.")]
    NotStaged,
}

impl SendError {
    pub fn cancelled(&self) -> bool {
        matches!(self, Self::Upload(UploadError::Cancelled))
    }
}

/// One send in progress or failed: its cancel switch and the files already stored.
#[derive(Default)]
struct Job {
    cancel: CancellationFlag,
    /// staged id → its upload, once the store has the whole file.
    stored: Mutex<HashMap<String, AttachmentId>>,
    /// staged id → a big file's upload still in parts (a retry resumes it).
    started: Mutex<HashMap<String, AttachmentUpload>>,
}

/// The sends of this run, by nonce.
#[derive(Default)]
pub struct Jobs {
    jobs: Mutex<HashMap<String, Arc<Job>>>,
}

/// Files bigger than this are temporary sends, in parts.
const MAX_BYTES: u64 = pulse_protocol::rest::attachment_limits::MAX_BYTES;
/// How many times a refused or dropped message send is tried before the row shows Failed.
const SEND_TRIES: u32 = 3;

impl Jobs {
    fn job(&self, nonce: &str) -> Arc<Job> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|p| p.into_inner());
        let job = jobs.entry(nonce.to_string()).or_default();
        // A retry after a cancel starts with a fresh switch (the stored files stay).
        if job.cancel.is_cancelled() {
            let stored = std::mem::take(&mut *job.stored.lock().unwrap_or_else(|p| p.into_inner()));
            let started =
                std::mem::take(&mut *job.started.lock().unwrap_or_else(|p| p.into_inner()));
            *job = Arc::new(Job {
                cancel: CancellationFlag::new(),
                stored: Mutex::new(stored),
                started: Mutex::new(started),
            });
        }
        job.clone()
    }

    /// Stop a send: its upload in flight ends now. Its stored files are cancelled on the server.
    pub fn cancel(&self, nonce: &str) -> Vec<AttachmentId> {
        let job = self
            .jobs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(nonce);
        match job {
            Some(job) => {
                job.cancel.cancel();
                let stored = job.stored.lock().unwrap_or_else(|p| p.into_inner());
                let started = job.started.lock().unwrap_or_else(|p| p.into_inner());
                stored
                    .values()
                    .copied()
                    .chain(started.values().map(|t| t.id))
                    .collect()
            }
            None => vec![],
        }
    }

    fn done(&self, nonce: &str) {
        self.jobs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(nonce);
    }

    /// Upload what isn't stored yet, then send the message. On success the staged files are
    /// forgotten (and our converted copies deleted).
    pub async fn run(
        &self,
        api: &Api,
        token: &str,
        stager: &Stager,
        out: &Outgoing,
        on_progress: impl Fn(Progress) + Send + Sync + 'static,
    ) -> Result<Message, SendError> {
        let job = self.job(&out.nonce);
        let files: Vec<Staged> = out
            .staged
            .iter()
            .map(|id| stager.get(id).ok_or(SendError::NotStaged))
            .collect::<Result<_, _>>()?;
        let total: u64 = files.iter().map(|f| f.size).sum();
        let on_progress = Arc::new(on_progress);
        let done_bytes = Arc::new(AtomicU64::new(0));
        let report = {
            let (nonce, on_progress, done_bytes) =
                (out.nonce.clone(), on_progress.clone(), done_bytes.clone());
            move |now: u64| {
                on_progress(Progress {
                    nonce: nonce.clone(),
                    sent: done_bytes.load(Ordering::SeqCst) + now,
                    total,
                })
            }
        };
        let http = upload::client();
        let mut ids = Vec::with_capacity(files.len());
        for f in &files {
            let already = job
                .stored
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&f.id)
                .copied();
            let id = match already {
                Some(id) => id,
                None => {
                    // A big file's upload started by an earlier try is resumed, not restarted.
                    let resumed = job
                        .started
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .get(&f.id)
                        .cloned();
                    let ticket = match resumed {
                        Some(t) => t,
                        None => {
                            api.start_attachment(
                                token,
                                out.channel,
                                &StartAttachmentRequest {
                                    name: f.name.clone(),
                                    size: f.size,
                                    content_type: f.content_type.clone(),
                                    width: f.width,
                                    height: f.height,
                                    duration_ms: None,
                                    temp_days: if f.size > MAX_BYTES { out.temp_days } else { None },
                                },
                            )
                            .await?
                        }
                    };
                    let r = report.clone();
                    match (&ticket.multipart, &ticket.url) {
                        (Some(info), _) => {
                            job.started
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .insert(f.id.clone(), ticket.clone());
                            let parts = upload_parts(api, token, &http, f, ticket.id, info, &job.cancel, r).await;
                            if let Err(SendError::Api(_)) = &parts {
                                // the upload is gone on the server (expired or swept): start over next time
                                job.started.lock().unwrap_or_else(|p| p.into_inner()).remove(&f.id);
                            }
                            parts?;
                            job.started.lock().unwrap_or_else(|p| p.into_inner()).remove(&f.id);
                        }
                        (None, Some(url)) => {
                            upload::put_file(
                                &http,
                                url,
                                &f.path,
                                &ticket.content_type,
                                move |sent, _| r(sent),
                                job.cancel.clone(),
                            )
                            .await?;
                        }
                        (None, None) => {
                            return Err(SendError::Api(ApiError::Rejected(
                                "That file can't be uploaded by this version of Pulse.".into(),
                            )));
                        }
                    }
                    job.stored
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(f.id.clone(), ticket.id);
                    ticket.id
                }
            };
            done_bytes.fetch_add(f.size, Ordering::SeqCst);
            report(0);
            ids.push(id);
        }
        let body = SendMessageRequest {
            content: out.content.clone(),
            reply_to_id: out.reply_to,
            nonce: Some(out.nonce.clone()),
            thread_root_id: out.thread_root,
            gif: None,
            attachments: ids,
        };
        let mut tries = 0;
        let msg = loop {
            tries += 1;
            match api.send_message(token, out.channel, &body).await {
                Ok(m) => break m,
                // The nonce makes a repeat safe: a send that landed comes back as it was.
                Err(e) if crate::outbox::retryable(&e) && tries < SEND_TRIES => {
                    tokio::time::sleep(Duration::from_secs(tries as u64)).await;
                }
                Err(e) => {
                    // The server found an upload missing or wrong: upload them all again next time.
                    if matches!(e, ApiError::Rejected(_) | ApiError::Told { .. }) {
                        let forgotten: Vec<AttachmentId> = job
                            .stored
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .drain()
                            .map(|(_, id)| id)
                            .collect();
                        for id in forgotten {
                            // Best effort: the sweep removes whatever is left.
                            let _ = api.cancel_attachment(token, id).await;
                        }
                    }
                    return Err(e.into());
                }
            }
        };
        self.done(&out.nonce);
        for id in &out.staged {
            stager.remove(id);
        }
        Ok(msg)
    }
}

/// Parts of a big file in flight at once.
const PARALLEL_PARTS: usize = 3;
/// How many times one part is tried before the send fails (a retry of the send resumes).
const PART_TRIES: u32 = 3;

/// Upload a big file's parts (only those the store doesn't have yet), then join them. `report`
/// gets this file's stored bytes so far.
#[allow(clippy::too_many_arguments)]
async fn upload_parts(
    api: &Api,
    token: &str,
    http: &reqwest::Client,
    file: &Staged,
    id: AttachmentId,
    info: &MultipartInfo,
    cancel: &CancellationFlag,
    report: impl Fn(u64) + Clone + Send + Sync + 'static,
) -> Result<(), SendError> {
    use futures::{StreamExt, TryStreamExt};
    let have = api.attachment_parts(token, id).await?;
    let done_bytes: u64 = have.iter().map(|p| p.size).sum();
    let done: std::collections::HashSet<u16> = have.iter().map(|p| p.part).collect();
    let todo: Vec<u16> = (1..=info.parts).filter(|p| !done.contains(p)).collect();
    // bytes stored per part (whole parts) plus what each part in flight has sent
    let flying: Arc<Mutex<HashMap<u16, u64>>> = Arc::default();
    let finished = Arc::new(AtomicU64::new(done_bytes));
    report(done_bytes);
    for batch in todo.chunks(pulse_protocol::rest::attachment_limits::PARTS_PER_ASK) {
        let urls = api.attachment_part_urls(token, id, batch).await?;
        futures::stream::iter(urls)
            .map(|pu| {
                let (flying, finished, report, cancel) =
                    (flying.clone(), finished.clone(), report.clone(), cancel.clone());
                let path = file.path.clone();
                let offset = (pu.part as u64 - 1) * info.part_size;
                async move {
                    let mut tries = 0;
                    loop {
                        tries += 1;
                        let progress = {
                            let (flying, finished, report) =
                                (flying.clone(), finished.clone(), report.clone());
                            move |sent: u64, _| {
                                let mut f = flying.lock().unwrap_or_else(|p| p.into_inner());
                                f.insert(pu.part, sent);
                                report(finished.load(Ordering::SeqCst) + f.values().sum::<u64>());
                            }
                        };
                        let r = upload::put_slice(
                            http,
                            &pu.url,
                            &path,
                            offset,
                            pu.size,
                            None,
                            progress,
                            cancel.clone(),
                            upload::STALL_AFTER,
                        )
                        .await;
                        flying
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .remove(&pu.part);
                        match r {
                            Ok(_) => {
                                finished.fetch_add(pu.size, Ordering::SeqCst);
                                return Ok::<(), SendError>(());
                            }
                            Err(UploadError::Cancelled) => return Err(UploadError::Cancelled.into()),
                            Err(e) if tries >= PART_TRIES => return Err(e.into()),
                            Err(_) => tokio::time::sleep(Duration::from_secs(tries as u64)).await,
                        }
                    }
                }
            })
            .buffer_unordered(PARALLEL_PARTS)
            .try_collect::<Vec<()>>()
            .await?;
    }
    api.complete_attachment(token, id).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_server::testing;
    use std::sync::Mutex as StdMutex;

    struct Setup {
        app: testing::TestApp,
        api: Api,
        token: String,
        channel: ChannelId,
        stager: Stager,
        dir: tempfile::TempDir,
    }

    async fn setup() -> Setup {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let channel = testing::general(&app, &token, s.id).await.id;
        let api = Api::new(&format!("http://{}", app.addr));
        let dir = tempfile::tempdir().unwrap();
        let stager = Stager::new(dir.path().join("staging"));
        Setup {
            app,
            api,
            token,
            channel,
            stager,
            dir,
        }
    }

    fn png() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(40, 20, image::Rgba([9, 9, 9, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn stage(s: &Setup) -> Vec<String> {
        let shot = s.dir.path().join("shot.png");
        std::fs::write(&shot, png()).unwrap();
        let notes = s.dir.path().join("notes.txt");
        std::fs::write(&notes, b"remember the milk").unwrap();
        vec![
            s.stager.stage_path(&shot).unwrap().id,
            s.stager.stage_path(&notes).unwrap().id,
        ]
    }

    fn outgoing(s: &Setup, staged: Vec<String>, content: &str) -> Outgoing {
        Outgoing {
            channel: s.channel,
            content: content.into(),
            reply_to: None,
            thread_root: None,
            nonce: "n-1".into(),
            staged,
            temp_days: None,
        }
    }

    #[tokio::test]
    async fn uploads_each_file_then_sends_with_ids_in_order() {
        let s = setup().await;
        let staged = stage(&s);
        let out = outgoing(&s, staged.clone(), "");
        let seen = Arc::new(StdMutex::new(Vec::<Progress>::new()));
        let log = seen.clone();
        let jobs = Jobs::default();
        let m = jobs
            .run(&s.api, &s.token, &s.stager, &out, move |p| {
                log.lock().unwrap().push(p)
            })
            .await
            .unwrap();
        let names: Vec<&str> = m.attachments.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["shot.webp", "notes.txt"]);
        assert_eq!(m.content, "📎 2 files");
        assert!(m.caption_is_fallback);
        assert_eq!(
            (m.attachments[0].width, m.attachments[0].height),
            (Some(40), Some(20))
        );
        // progress ends at the total
        let last = seen.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.sent, last.total);
        assert!(last.total > 0);
        // staged files are let go after a send
        assert!(staged.iter().all(|id| s.stager.get(id).is_none()));
    }

    #[tokio::test]
    async fn a_failed_put_reports_failed_and_retry_skips_done_files() {
        let s = setup().await;
        let staged = stage(&s);
        let out = outgoing(&s, staged, "hello");
        let jobs = Jobs::default();
        let fake = s.app.s3.as_ref().unwrap();
        fake.set_down(true);
        let err = jobs.run(&s.api, &s.token, &s.stager, &out, |_| {}).await;
        assert!(err.is_err());
        fake.set_down(false);
        let m = jobs
            .run(&s.api, &s.token, &s.stager, &out, |_| {})
            .await
            .unwrap();
        assert_eq!(m.content, "hello");
        assert_eq!(m.attachments.len(), 2);
        // one upload per file in all: the failed one was started again, nothing twice
        let puts = fake
            .requests()
            .iter()
            .filter(|(method, _)| method == reqwest::Method::PUT)
            .count();
        assert_eq!(puts, 2, "{:?}", fake.requests());
    }

    /// A file just over the normal limit: four parts.
    fn stage_big(s: &Setup) -> String {
        let p = s.dir.path().join("world.zip");
        let f = std::fs::File::create(&p).unwrap();
        f.set_len(MAX_BYTES + 1).unwrap();
        s.stager.stage_path(&p).unwrap().id
    }

    fn part_puts(s: &Setup) -> usize {
        s.app
            .s3
            .as_ref()
            .unwrap()
            .requests()
            .iter()
            .filter(|(method, _)| method == reqwest::Method::PUT)
            .count()
    }

    #[tokio::test]
    async fn a_big_file_goes_up_in_parts_and_expires() {
        let s = setup().await;
        let mut out = outgoing(&s, vec![stage_big(&s)], "the world");
        out.temp_days = Some(1);
        let seen = Arc::new(StdMutex::new(Vec::<Progress>::new()));
        let log = seen.clone();
        let m = Jobs::default()
            .run(&s.api, &s.token, &s.stager, &out, move |p| log.lock().unwrap().push(p))
            .await
            .unwrap();
        assert_eq!(m.attachments[0].size, MAX_BYTES + 1);
        assert!(m.attachments[0].expires_at.is_some());
        assert_eq!(part_puts(&s), 4);
        let last = seen.lock().unwrap().last().cloned().unwrap();
        assert_eq!((last.sent, last.total), (MAX_BYTES + 1, MAX_BYTES + 1));
    }

    #[tokio::test]
    async fn a_retry_resumes_a_big_upload_without_sending_stored_parts_again() {
        let s = setup().await;
        let staged = stage_big(&s);
        let mut out = outgoing(&s, vec![staged.clone()], "");
        out.temp_days = Some(7);
        let file = s.stager.get(&staged).unwrap();
        // an earlier try started the upload and stored part 1, then the connection dropped
        let ticket = s
            .api
            .start_attachment(
                &s.token,
                s.channel,
                &StartAttachmentRequest {
                    name: file.name.clone(),
                    size: file.size,
                    content_type: file.content_type.clone(),
                    width: None,
                    height: None,
                    duration_ms: None,
                    temp_days: Some(7),
                },
            )
            .await
            .unwrap();
        let urls = s
            .api
            .attachment_part_urls(&s.token, ticket.id, &[1])
            .await
            .unwrap();
        let part_size = ticket.multipart.as_ref().unwrap().part_size;
        upload::put_slice(
            &upload::client(),
            &urls[0].url,
            &file.path,
            0,
            part_size,
            None,
            |_, _| {},
            CancellationFlag::new(),
            upload::STALL_AFTER,
        )
        .await
        .unwrap();
        let jobs = Jobs::default();
        jobs.job("n-1")
            .started
            .lock()
            .unwrap()
            .insert(staged, ticket);
        let m = jobs.run(&s.api, &s.token, &s.stager, &out, |_| {}).await.unwrap();
        assert_eq!(m.attachments.len(), 1);
        // part 1 once (the earlier try), then parts 2 to 4
        assert_eq!(part_puts(&s), 4);
    }

    #[tokio::test]
    async fn a_file_no_longer_staged_is_refused_before_anything_uploads() {
        let s = setup().await;
        let mut staged = stage(&s);
        staged.push("gone".into());
        let out = outgoing(&s, staged, "");
        let err = Jobs::default()
            .run(&s.api, &s.token, &s.stager, &out, |_| {})
            .await;
        assert!(matches!(err, Err(SendError::NotStaged)));
        assert!(s.app.s3.as_ref().unwrap().keys().is_empty());
    }
}
