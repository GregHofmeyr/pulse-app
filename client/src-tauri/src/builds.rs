//! Builds channels (#617): browsing a channel's builds, their per-game counts, archiving and deleting them, and
//! searching builds across servers. Saving one (#618): its screenshots go up first (as a message's files do), then
//! the build is posted with their ids.

use std::time::{Duration, Instant};

use pulse_protocol::ids::{AttachmentId, ChannelId, GameId, MessageId};
use pulse_protocol::rest::{
    ArchiveBuildsRequest, BuildFacets, BuildsBatchResult, BulkArchiveRequest, BulkArchiveResult,
    CreateBuildRequest, DeleteBuildsRequest, FavouriteBuildsRequest, Message,
    SetBuildArchivedRequest, UpdateBuildRequest,
};
use tauri::{AppHandle, Emitter, State};

use crate::api::{Api, ApiError};
use crate::attachments::Stager;
use crate::attachments::send::{Jobs, Progress, SendError};
use crate::commands::Core;

impl Api {
    /// A page of a builds channel's builds (30), newest first by when they were saved; `before` pages back. Archived
    /// ones only with `archived`; `game` narrows to one game, `favourite` to your starred ones, `q` to builds with those
    /// words.
    #[allow(clippy::too_many_arguments)]
    pub async fn list_builds(
        &self,
        token: &str,
        channel: ChannelId,
        game: Option<GameId>,
        archived: bool,
        favourite: bool,
        q: Option<&str>,
        before: Option<MessageId>,
    ) -> Result<Vec<Message>, ApiError> {
        let mut req = self
            .http
            .get(self.url(&["channels", &channel.to_string(), "builds"])?)
            .bearer_auth(token);
        if let Some(g) = game {
            req = req.query(&[("game", g.to_string())]);
        }
        if archived {
            req = req.query(&[("archived", "true")]);
        }
        if favourite {
            req = req.query(&[("favourite", "true")]);
        }
        if let Some(q) = q.map(str::trim).filter(|q| !q.is_empty()) {
            req = req.query(&[("q", q)]);
        }
        if let Some(b) = before {
            req = req.query(&[("before", b.to_string())]);
        }
        Self::clip_parse(req.send().await).await
    }

    pub async fn create_build(
        &self,
        token: &str,
        channel: ChannelId,
        body: &CreateBuildRequest,
    ) -> Result<Message, ApiError> {
        Self::clip_parse(
            self.http
                .post(self.url(&["channels", &channel.to_string(), "builds"])?)
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    /// The author's edit, replacing every field (`body.attachments` in their final order).
    pub async fn update_build(
        &self,
        token: &str,
        id: MessageId,
        body: &UpdateBuildRequest,
    ) -> Result<Message, ApiError> {
        Self::clip_parse(
            self.http
                .put(self.url(&["builds", &id.to_string()])?)
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    pub async fn get_build(&self, token: &str, id: MessageId) -> Result<Message, ApiError> {
        Self::clip_parse(
            self.http
                .get(self.url(&["builds", &id.to_string()])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// The channel's builds counted per game (archived ones too with `archived`).
    pub async fn build_facets(
        &self,
        token: &str,
        channel: ChannelId,
        archived: bool,
    ) -> Result<BuildFacets, ApiError> {
        let mut req = self
            .http
            .get(self.url(&["channels", &channel.to_string(), "builds", "facets"])?)
            .bearer_auth(token);
        if archived {
            req = req.query(&[("archived", "true")]);
        }
        Self::clip_parse(req.send().await).await
    }

    pub async fn set_build_archived(
        &self,
        token: &str,
        id: MessageId,
        archived: bool,
    ) -> Result<Message, ApiError> {
        Self::clip_parse(
            self.http
                .put(self.url(&["builds", &id.to_string(), "archived"])?)
                .bearer_auth(token)
                .json(&SetBuildArchivedRequest { archived })
                .send()
                .await,
        )
        .await
    }

    /// Archives (or with `dry_run`, only lists) the builds saved before a time, of one game or all, that the caller
    /// may archive.
    pub async fn bulk_archive_builds(
        &self,
        token: &str,
        channel: ChannelId,
        body: &BulkArchiveRequest,
    ) -> Result<BulkArchiveResult, ApiError> {
        Self::clip_parse(
            self.http
                .post(self.url(&["channels", &channel.to_string(), "builds", "archive"])?)
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    /// Archives (or restores) up to 100 of a channel's builds in one go; the ones the caller may not change are
    /// skipped (multi-select).
    pub async fn archive_many_builds(
        &self,
        token: &str,
        channel: ChannelId,
        body: &ArchiveBuildsRequest,
    ) -> Result<BuildsBatchResult, ApiError> {
        Self::clip_parse(
            self.http
                .post(self.url(&["channels", &channel.to_string(), "builds", "archive-many"])?)
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    /// Deletes up to 100 of a channel's builds in one go; the ones the caller may not delete are skipped.
    pub async fn delete_many_builds(
        &self,
        token: &str,
        channel: ChannelId,
        body: &DeleteBuildsRequest,
    ) -> Result<BuildsBatchResult, ApiError> {
        Self::clip_parse(
            self.http
                .post(self.url(&["channels", &channel.to_string(), "builds", "delete-many"])?)
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    /// Stars (or unstars) a build for the caller (#617: favourites are each person's own).
    pub async fn set_build_favourite(
        &self,
        token: &str,
        id: MessageId,
        favourite: bool,
    ) -> Result<(), ApiError> {
        let url = self.url(&["builds", &id.to_string(), "favourite"])?;
        let req = if favourite {
            self.http.put(url)
        } else {
            self.http.delete(url)
        };
        Self::clip_ok_empty(req.bearer_auth(token).send().await).await
    }

    /// Stars (or unstars) up to 100 of a channel's builds for the caller.
    pub async fn favourite_many_builds(
        &self,
        token: &str,
        channel: ChannelId,
        body: &FavouriteBuildsRequest,
    ) -> Result<BuildsBatchResult, ApiError> {
        Self::clip_parse(
            self.http
                .post(self.url(&["channels", &channel.to_string(), "builds", "favourite-many"])?)
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    pub async fn delete_build(&self, token: &str, id: MessageId) -> Result<(), ApiError> {
        Self::clip_ok_empty(
            self.http
                .delete(self.url(&["builds", &id.to_string()])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// Builds across every builds channel the caller can read with these words, of one game (its normalised name)
    /// when given: current ones first, then archived.
    pub async fn search_builds(
        &self,
        token: &str,
        q: &str,
        game: Option<&str>,
    ) -> Result<Vec<Message>, ApiError> {
        let mut req = self
            .http
            .get(self.url(&["builds", "search"])?)
            .bearer_auth(token);
        if !q.trim().is_empty() {
            req = req.query(&[("q", q.trim())]);
        }
        if let Some(g) = game.filter(|g| !g.is_empty()) {
            req = req.query(&[("game", g)]);
        }
        Self::clip_parse(req.send().await).await
    }
}

/// One place in an edit's screenshot order: a file the build already has, or a staged one to upload.
enum Shot {
    Kept(AttachmentId),
    Staged(String),
}

/// `id:<attachment id>` or `staged:<staged id>`, in order.
fn parse_order(order: &[String]) -> Result<Vec<Shot>, SendError> {
    order
        .iter()
        .map(|item| match item.split_once(':') {
            Some(("id", id)) => id.parse().map(Shot::Kept).map_err(|_| {
                SendError::Api(ApiError::Rejected("That screenshot isn't known.".into()))
            }),
            Some(("staged", id)) if !id.is_empty() => Ok(Shot::Staged(id.to_string())),
            _ => Err(SendError::Api(ApiError::Rejected(
                "That screenshot isn't known.".into(),
            ))),
        })
        .collect()
}

/// How many times a save's post is tried when it fails in a way worth retrying (as a message send's is).
const POST_TRIES: u32 = 3;

/// `post` tried up to `POST_TRIES` times while it fails retryably (`outbox::retryable`), waiting `wait(try)` between.
/// The nonce makes a repeated create safe; an update just writes the same fields again.
async fn with_retries<T, F, Fut>(mut post: F, wait: impl Fn(u32) -> Duration) -> Result<T, ApiError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, ApiError>>,
{
    let mut tries = 0;
    loop {
        tries += 1;
        match post().await {
            Err(e) if crate::outbox::retryable(&e) && tries < POST_TRIES => {
                tokio::time::sleep(wait(tries)).await
            }
            r => return r,
        }
    }
}

/// A save's post, retried; refused (`Rejected`/`Told`), its uploads are dropped so the next try sends them again.
async fn posted<F, Fut>(
    jobs: &Jobs,
    api: &Api,
    token: &str,
    nonce: &str,
    post: F,
) -> Result<Message, SendError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Message, ApiError>>,
{
    match with_retries(post, |t| Duration::from_secs(t as u64)).await {
        Ok(m) => Ok(m),
        Err(e) => {
            if matches!(e, ApiError::Rejected(_) | ApiError::Told { .. }) {
                jobs.forget_stored(api, token, nonce).await;
            }
            Err(e.into())
        }
    }
}

/// A save given up (its form cancelled, or quick add closed after it failed): what it uploaded is dropped on the
/// server and the send forgotten. The caller unstages its staged files.
pub async fn release_build_save(jobs: &Jobs, api: &Api, token: &str, nonce: &str) {
    for id in jobs.cancel(nonce) {
        // Best effort: the server sweeps whatever is left.
        let _ = api.cancel_attachment(token, id).await;
    }
}

/// A new build: `staged` screenshots uploaded in order under its nonce (after any ids it already lists), then posted.
/// A failed post keeps the uploads, so trying again with the same nonce uploads nothing twice.
#[allow(clippy::too_many_arguments)]
pub async fn save_new_build(
    jobs: &Jobs,
    api: &Api,
    token: &str,
    stager: &Stager,
    channel: ChannelId,
    mut body: CreateBuildRequest,
    staged: &[String],
    on_progress: impl Fn(Progress) + Send + Sync + 'static,
) -> Result<Message, SendError> {
    let nonce = body
        .nonce
        .clone()
        .unwrap_or_else(|| format!("build:{}", MessageId::new()));
    body.nonce = Some(nonce.clone());
    let ids = jobs
        .upload_staged(api, token, stager, channel, &nonce, staged, on_progress)
        .await?;
    body.attachments.extend(ids);
    let m = posted(jobs, api, token, &nonce, || {
        api.create_build(token, channel, &body)
    })
    .await?;
    jobs.finish(&nonce, staged, stager);
    Ok(m)
}

/// An edit: the new screenshots in `order` uploaded, then `body.attachments` written in that order and the build
/// updated. Retrying an edit of the same build reuses what went up.
#[allow(clippy::too_many_arguments)]
pub async fn save_build_edit(
    jobs: &Jobs,
    api: &Api,
    token: &str,
    stager: &Stager,
    message: MessageId,
    channel: ChannelId,
    mut body: UpdateBuildRequest,
    order: &[String],
    on_progress: impl Fn(Progress) + Send + Sync + 'static,
) -> Result<Message, SendError> {
    let shots = parse_order(order)?;
    let staged: Vec<String> = shots
        .iter()
        .filter_map(|s| match s {
            Shot::Staged(id) => Some(id.clone()),
            Shot::Kept(_) => None,
        })
        .collect();
    let nonce = format!("build-edit:{message}");
    let mut uploaded = jobs
        .upload_staged(api, token, stager, channel, &nonce, &staged, on_progress)
        .await?
        .into_iter();
    body.attachments = shots
        .into_iter()
        .map(|s| match s {
            Shot::Kept(id) => id,
            // one uploaded id per staged file, in order
            Shot::Staged(_) => uploaded.next().expect("an id per staged file"),
        })
        .collect();
    let m = posted(jobs, api, token, &nonce, || {
        api.update_build(token, message, &body)
    })
    .await?;
    jobs.finish(&nonce, &staged, stager);
    Ok(m)
}

/// At most four progress events a second per save.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// `attachments://progress` for a save's uploads, throttled as a message's are.
fn progress_to(app: &AppHandle) -> impl Fn(Progress) + Send + Sync + 'static {
    let app = app.clone();
    let throttle = std::sync::Mutex::new(None::<Instant>);
    move |p: Progress| {
        let mut last = throttle.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) || p.sent >= p.total {
            *last = Some(Instant::now());
            let _ = app.emit("attachments://progress", &p);
        }
    }
}

/// A save's error as the UI reads it.
fn told(app: &AppHandle, core: &Core, e: SendError) -> ApiError {
    match e {
        SendError::Api(ApiError::Unauthorized) => core
            .check::<()>(app, Err(ApiError::Unauthorized))
            .err()
            .unwrap_or(ApiError::Unauthorized),
        SendError::Api(e) => e,
        other => ApiError::Rejected(other.to_string()),
    }
}

#[tauri::command]
pub async fn create_build(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    body: CreateBuildRequest,
    staged: Vec<String>,
) -> Result<Message, ApiError> {
    let (api, token) = core.current()?;
    save_new_build(
        &core.attachment_sends,
        &api,
        &token,
        &core.attachments,
        channel_id,
        body,
        &staged,
        progress_to(&app),
    )
    .await
    .map_err(|e| told(&app, &core, e))
}

#[tauri::command]
pub async fn update_build(
    app: AppHandle,
    core: State<'_, Core>,
    message_id: MessageId,
    channel_id: ChannelId,
    body: UpdateBuildRequest,
    order: Vec<String>,
) -> Result<Message, ApiError> {
    let (api, token) = core.current()?;
    save_build_edit(
        &core.attachment_sends,
        &api,
        &token,
        &core.attachments,
        message_id,
        channel_id,
        body,
        &order,
        progress_to(&app),
    )
    .await
    .map_err(|e| told(&app, &core, e))
}

/// Give up a failed save: its uploads are dropped (`release_build_save`).
#[tauri::command]
pub async fn cancel_build_save(core: State<'_, Core>, nonce: String) -> Result<(), ApiError> {
    if let Ok((api, token)) = core.current() {
        release_build_save(&core.attachment_sends, &api, &token, &nonce).await;
    } else {
        core.attachment_sends.cancel(&nonce);
    }
    Ok(())
}

/// `favourite` (only your starred builds) may be absent: not starred-only.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn list_builds(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    game: Option<GameId>,
    archived: bool,
    favourite: Option<bool>,
    q: Option<String>,
    before: Option<MessageId>,
) -> Result<Vec<Message>, ApiError> {
    let (api, token) = core.current()?;
    let favourite = favourite.unwrap_or(false);
    core.check(
        &app,
        api.list_builds(
            &token,
            channel_id,
            game,
            archived,
            favourite,
            q.as_deref(),
            before,
        )
        .await,
    )
}

#[tauri::command]
pub async fn get_build(
    app: AppHandle,
    core: State<'_, Core>,
    message_id: MessageId,
) -> Result<Message, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.get_build(&token, message_id).await)
}

#[tauri::command]
pub async fn build_facets(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    archived: bool,
) -> Result<BuildFacets, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.build_facets(&token, channel_id, archived).await)
}

#[tauri::command]
pub async fn set_build_archived(
    app: AppHandle,
    core: State<'_, Core>,
    message_id: MessageId,
    archived: bool,
) -> Result<Message, ApiError> {
    let (api, token) = core.current()?;
    core.check(
        &app,
        api.set_build_archived(&token, message_id, archived).await,
    )
}

#[tauri::command]
pub async fn bulk_archive_builds(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    body: BulkArchiveRequest,
) -> Result<BulkArchiveResult, ApiError> {
    let (api, token) = core.current()?;
    core.check(
        &app,
        api.bulk_archive_builds(&token, channel_id, &body).await,
    )
}

#[tauri::command]
pub async fn archive_many_builds(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    body: ArchiveBuildsRequest,
) -> Result<BuildsBatchResult, ApiError> {
    let (api, token) = core.current()?;
    core.check(
        &app,
        api.archive_many_builds(&token, channel_id, &body).await,
    )
}

#[tauri::command]
pub async fn delete_many_builds(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    body: DeleteBuildsRequest,
) -> Result<BuildsBatchResult, ApiError> {
    let (api, token) = core.current()?;
    core.check(
        &app,
        api.delete_many_builds(&token, channel_id, &body).await,
    )
}

#[tauri::command]
pub async fn set_build_favourite(
    app: AppHandle,
    core: State<'_, Core>,
    message_id: MessageId,
    favourite: bool,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(
        &app,
        api.set_build_favourite(&token, message_id, favourite).await,
    )
}

#[tauri::command]
pub async fn favourite_many_builds(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    body: FavouriteBuildsRequest,
) -> Result<BuildsBatchResult, ApiError> {
    let (api, token) = core.current()?;
    core.check(
        &app,
        api.favourite_many_builds(&token, channel_id, &body).await,
    )
}

#[tauri::command]
pub async fn delete_build(
    app: AppHandle,
    core: State<'_, Core>,
    message_id: MessageId,
) -> Result<(), ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.delete_build(&token, message_id).await)
}

#[tauri::command]
pub async fn search_builds(
    app: AppHandle,
    core: State<'_, Core>,
    q: String,
    game: Option<String>,
) -> Result<Vec<Message>, ApiError> {
    let (api, token) = core.current()?;
    core.check(&app, api.search_builds(&token, &q, game.as_deref()).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A server that answers every request with 200 and `body` (JSON), keeping each request it was sent (head and
    /// body, as text).
    async fn capture(body: &'static str) -> (Api, Arc<Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let keep = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                let keep = keep.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    // read the head, then as much body as it says it has
                    loop {
                        let Ok(n) = s.read(&mut chunk).await else {
                            return;
                        };
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        let text = String::from_utf8_lossy(&buf).to_string();
                        if let Some(end) = text.find("\r\n\r\n") {
                            let len = text[..end]
                                .lines()
                                .find_map(|l| {
                                    let (k, v) = l.split_once(':')?;
                                    k.eq_ignore_ascii_case("content-length")
                                        .then(|| v.trim().parse::<usize>().ok())?
                                })
                                .unwrap_or(0);
                            if buf.len() >= end + 4 + len {
                                break;
                            }
                        }
                    }
                    keep.lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&buf).to_string());
                    let (status, body) = if body.is_empty() {
                        ("204 No Content", "")
                    } else {
                        ("200 OK", body)
                    };
                    let resp = format!(
                        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = s.write_all(resp.as_bytes()).await;
                    let _ = s.shutdown().await;
                });
            }
        });
        (Api::new(&format!("http://{addr}")), seen)
    }

    fn first(seen: &Arc<Mutex<Vec<String>>>) -> String {
        seen.lock().unwrap().first().cloned().expect("a request")
    }

    #[tokio::test]
    async fn list_builds_sends_its_filters() {
        let (api, seen) = capture("[]").await;
        let (ch, game, before) = (ChannelId::new(), GameId::new(), MessageId::new());
        let got = api
            .list_builds(
                "t",
                ch,
                Some(game),
                true,
                true,
                Some("raven build"),
                Some(before),
            )
            .await
            .unwrap();
        assert!(got.is_empty());
        let req = first(&seen);
        let line = req.lines().next().unwrap();
        assert!(
            line.starts_with(&format!("GET /channels/{ch}/builds?")),
            "{line}"
        );
        assert!(line.contains(&format!("game={game}")), "{line}");
        assert!(line.contains("archived=true"), "{line}");
        assert!(line.contains("q=raven+build"), "{line}");
        assert!(line.contains("favourite=true"), "{line}");
        assert!(line.contains(&format!("before={before}")), "{line}");
    }

    #[tokio::test]
    async fn list_builds_without_filters_sends_none() {
        let (api, seen) = capture("[]").await;
        let ch = ChannelId::new();
        api.list_builds("t", ch, None, false, false, None, None)
            .await
            .unwrap();
        let req = first(&seen);
        let line = req.lines().next().unwrap();
        assert!(
            line.starts_with(&format!("GET /channels/{ch}/builds ")),
            "{line}"
        );
    }

    #[tokio::test]
    async fn bulk_archive_posts_its_body() {
        let (api, seen) = capture(r#"{"builds":[],"archived":0}"#).await;
        let (ch, game) = (ChannelId::new(), GameId::new());
        let body = BulkArchiveRequest {
            game_id: Some(game),
            before: "2026-10-25T00:00:00Z".into(),
            dry_run: true,
        };
        let got = api.bulk_archive_builds("t", ch, &body).await.unwrap();
        assert_eq!(got.archived, 0);
        let req = first(&seen);
        assert!(
            req.starts_with(&format!("POST /channels/{ch}/builds/archive ")),
            "{req}"
        );
        let json: serde_json::Value =
            serde_json::from_str(req.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(json["game_id"], game.to_string());
        assert_eq!(json["before"], "2026-10-25T00:00:00Z");
        assert_eq!(json["dry_run"], true);
    }

    #[tokio::test]
    async fn archive_many_posts_its_ids() {
        let (api, seen) = capture(r#"{"done":1,"skipped":1}"#).await;
        let (ch, a, b) = (ChannelId::new(), MessageId::new(), MessageId::new());
        let body = ArchiveBuildsRequest {
            message_ids: vec![a, b],
            archived: true,
        };
        let got = api.archive_many_builds("t", ch, &body).await.unwrap();
        assert_eq!(
            got,
            BuildsBatchResult {
                done: 1,
                skipped: 1
            }
        );
        let req = first(&seen);
        assert!(
            req.starts_with(&format!("POST /channels/{ch}/builds/archive-many ")),
            "{req}"
        );
        let json: serde_json::Value =
            serde_json::from_str(req.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(
            json["message_ids"],
            serde_json::json!([a.to_string(), b.to_string()])
        );
        assert_eq!(json["archived"], true);
    }

    #[tokio::test]
    async fn delete_many_posts_its_ids() {
        let (api, seen) = capture(r#"{"done":1,"skipped":0}"#).await;
        let (ch, a) = (ChannelId::new(), MessageId::new());
        let body = DeleteBuildsRequest {
            message_ids: vec![a],
        };
        let got = api.delete_many_builds("t", ch, &body).await.unwrap();
        assert_eq!(got.done, 1);
        let req = first(&seen);
        assert!(
            req.starts_with(&format!("POST /channels/{ch}/builds/delete-many ")),
            "{req}"
        );
    }

    #[tokio::test]
    async fn a_star_is_a_put_and_unstarring_a_delete() {
        let (api, seen) = capture("").await;
        let id = MessageId::new();
        api.set_build_favourite("t", id, true).await.unwrap();
        api.set_build_favourite("t", id, false).await.unwrap();
        let seen = seen.lock().unwrap().clone();
        assert!(
            seen[0].starts_with(&format!("PUT /builds/{id}/favourite ")),
            "{}",
            seen[0]
        );
        assert!(
            seen[1].starts_with(&format!("DELETE /builds/{id}/favourite ")),
            "{}",
            seen[1]
        );
    }

    #[tokio::test]
    async fn favourite_many_posts_its_ids() {
        let (api, seen) = capture(r#"{"done":1,"skipped":0}"#).await;
        let (ch, a) = (ChannelId::new(), MessageId::new());
        let body = FavouriteBuildsRequest {
            message_ids: vec![a],
            favourite: true,
        };
        let got = api.favourite_many_builds("t", ch, &body).await.unwrap();
        assert_eq!(got.done, 1);
        let req = first(&seen);
        assert!(
            req.starts_with(&format!("POST /channels/{ch}/builds/favourite-many ")),
            "{req}"
        );
        let json: serde_json::Value =
            serde_json::from_str(req.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(json["favourite"], true);
    }

    #[tokio::test]
    async fn delete_build_is_a_delete() {
        let (api, seen) = capture("").await;
        let id = MessageId::new();
        api.delete_build("t", id).await.unwrap();
        assert!(
            first(&seen).starts_with(&format!("DELETE /builds/{id} ")),
            "{}",
            first(&seen)
        );
    }

    #[tokio::test]
    async fn the_other_calls_reach_their_routes() {
        let (api, seen) = capture(r#"{"total":0,"games":[]}"#).await;
        let ch = ChannelId::new();
        api.build_facets("t", ch, true).await.unwrap();
        let req = first(&seen);
        assert!(
            req.starts_with(&format!("GET /channels/{ch}/builds/facets?archived=true ")),
            "{req}"
        );

        let (api, seen) = capture("[]").await;
        api.search_builds("t", "raven", Some("the first descendant"))
            .await
            .unwrap();
        let req = first(&seen);
        let line = req.lines().next().unwrap();
        assert!(line.starts_with("GET /builds/search?"), "{line}");
        assert!(line.contains("q=raven"), "{line}");
        assert!(line.contains("game=the+first+descendant"), "{line}");

        // set_build_archived answers the build; a canned message isn't worth building here, so a parse failure after
        // the request went out is fine: the request is what's under test
        let (api, seen) = capture("{}").await;
        let id = MessageId::new();
        let _ = api.set_build_archived("t", id, true).await;
        let req = first(&seen);
        assert!(
            req.starts_with(&format!("PUT /builds/{id}/archived ")),
            "{req}"
        );
        assert!(req.ends_with(r#"{"archived":true}"#), "{req}");

        let (api, seen) = capture("{}").await;
        let _ = api.get_build("t", id).await;
        assert!(first(&seen).starts_with(&format!("GET /builds/{id} ")));
    }

    // --- saving builds with screenshots (#618), against a real server ---

    struct Live {
        app: pulse_server::testing::TestApp,
        api: Api,
        token: String,
        channel: ChannelId,
        stager: crate::attachments::Stager,
        dir: tempfile::TempDir,
    }

    async fn live() -> Live {
        use pulse_server::testing;
        let app = testing::spawn_with(testing::test_config()).await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let r = testing::post_json(
            &app,
            &token,
            &format!("/servers/{}/channels", s.id),
            serde_json::json!({ "kind": "builds", "name": "builds" }),
        )
        .await;
        assert_eq!(r.status(), 200, "create builds channel");
        let channel: pulse_protocol::rest::Channel = r.json().await.unwrap();
        let api = Api::new(&format!("http://{}", app.addr));
        let dir = tempfile::tempdir().unwrap();
        let stager = crate::attachments::Stager::new(dir.path().join("staging"));
        Live {
            app,
            api,
            token,
            channel: channel.id,
            stager,
            dir,
        }
    }

    /// A screenshot of `w`×20, staged as `name`.png.
    fn shot(l: &Live, name: &str, w: u32) -> String {
        let img = image::RgbaImage::from_pixel(w, 20, image::Rgba([9, 9, 9, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        let p = l.dir.path().join(format!("{name}.png"));
        std::fs::write(&p, out.into_inner()).unwrap();
        l.stager.stage_path(&p).unwrap().id
    }

    fn puts(l: &Live) -> usize {
        l.app
            .s3
            .as_ref()
            .unwrap()
            .requests()
            .iter()
            .filter(|(method, _)| method == reqwest::Method::PUT)
            .count()
    }

    fn names(m: &Message) -> Vec<&str> {
        m.attachments.iter().map(|a| a.name.as_str()).collect()
    }

    #[tokio::test]
    async fn create_build_uploads_then_posts_in_order() {
        let l = live().await;
        let staged = vec![shot(&l, "first", 40), shot(&l, "second", 41)];
        let jobs = Jobs::default();
        let body = CreateBuildRequest {
            title: Some("Raven bunny".into()),
            nonce: Some("n-1".into()),
            ..Default::default()
        };
        let m = save_new_build(
            &jobs,
            &l.api,
            &l.token,
            &l.stager,
            l.channel,
            body,
            &staged,
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(m.build.as_ref().unwrap().title, "Raven bunny");
        assert_eq!(names(&m), ["first.webp", "second.webp"]);
        assert_eq!(puts(&l), 2);
        // the staged files are let go once the build is saved
        assert!(staged.iter().all(|id| l.stager.get(id).is_none()));
    }

    #[tokio::test]
    async fn update_build_keeps_ids_and_fills_placeholders() {
        let l = live().await;
        let jobs = Jobs::default();
        let first = vec![shot(&l, "kept", 40)];
        let body = CreateBuildRequest {
            title: Some("Raven bunny".into()),
            nonce: Some("n-1".into()),
            ..Default::default()
        };
        let m = save_new_build(
            &jobs,
            &l.api,
            &l.token,
            &l.stager,
            l.channel,
            body,
            &first,
            |_| {},
        )
        .await
        .unwrap();
        let kept = m.attachments[0].id;
        let (a, b) = (shot(&l, "new-a", 42), shot(&l, "new-b", 43));
        let order = vec![
            format!("staged:{a}"),
            format!("id:{kept}"),
            format!("staged:{b}"),
        ];
        let edit = UpdateBuildRequest {
            title: "Raven bunny v2".into(),
            notes: Some("Max crit".into()),
            ..Default::default()
        };
        let m = save_build_edit(
            &jobs,
            &l.api,
            &l.token,
            &l.stager,
            m.id,
            l.channel,
            edit,
            &order,
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(m.build.as_ref().unwrap().title, "Raven bunny v2");
        assert_eq!(names(&m), ["new-a.webp", "kept.webp", "new-b.webp"]);
        assert_eq!(m.attachments[1].id, kept);
        // only the new ones went up
        assert_eq!(puts(&l), 3);
    }

    #[tokio::test]
    async fn a_bad_order_item_is_refused_before_anything_uploads() {
        let l = live().await;
        let jobs = Jobs::default();
        let a = shot(&l, "a", 40);
        let edit = UpdateBuildRequest {
            title: "Raven".into(),
            ..Default::default()
        };
        let order = vec![format!("staged:{a}"), "file:nope".to_string()];
        let err = save_build_edit(
            &jobs,
            &l.api,
            &l.token,
            &l.stager,
            MessageId::new(),
            l.channel,
            edit,
            &order,
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, SendError::Api(ApiError::Rejected(_))),
            "{err:?}"
        );
        assert_eq!(puts(&l), 0);
    }

    #[tokio::test]
    async fn a_refused_create_drops_its_uploads_and_the_retry_sends_them_again() {
        let l = live().await;
        let jobs = Jobs::default();
        let staged = vec![shot(&l, "first", 40), shot(&l, "second", 41)];
        // refused after the files went up: a title over 120 characters
        let long = CreateBuildRequest {
            title: Some("x".repeat(121)),
            nonce: Some("n-1".into()),
            ..Default::default()
        };
        let err = save_new_build(
            &jobs,
            &l.api,
            &l.token,
            &l.stager,
            l.channel,
            long,
            &staged,
            |_| {},
        )
        .await;
        assert!(
            matches!(err, Err(SendError::Api(ApiError::Rejected(_)))),
            "{err:?}"
        );
        assert_eq!(puts(&l), 2);
        // the staged files are still there for the retry, which uploads them again (as a refused message send does)
        assert!(staged.iter().all(|id| l.stager.get(id).is_some()));
        let ok = CreateBuildRequest {
            title: Some("Raven bunny".into()),
            nonce: Some("n-1".into()),
            ..Default::default()
        };
        let m = save_new_build(
            &jobs,
            &l.api,
            &l.token,
            &l.stager,
            l.channel,
            ok,
            &staged,
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(names(&m), ["first.webp", "second.webp"]);
        assert_eq!(puts(&l), 4);
    }

    #[tokio::test]
    async fn uploads_kept_for_a_retry_go_up_once() {
        let l = live().await;
        let jobs = Jobs::default();
        let staged = vec![shot(&l, "first", 40)];
        jobs.upload_staged(
            &l.api,
            &l.token,
            &l.stager,
            l.channel,
            "n-1",
            &staged,
            |_| {},
        )
        .await
        .unwrap();
        // the post failed in a way worth retrying (nothing dropped): the retry's save uploads nothing again
        let ok = CreateBuildRequest {
            title: Some("Raven bunny".into()),
            nonce: Some("n-1".into()),
            ..Default::default()
        };
        save_new_build(
            &jobs,
            &l.api,
            &l.token,
            &l.stager,
            l.channel,
            ok,
            &staged,
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(puts(&l), 1);
    }

    #[tokio::test]
    async fn a_released_save_forgets_its_uploads() {
        let l = live().await;
        let jobs = Jobs::default();
        let staged = vec![shot(&l, "first", 40)];
        jobs.upload_staged(
            &l.api,
            &l.token,
            &l.stager,
            l.channel,
            "n-1",
            &staged,
            |_| {},
        )
        .await
        .unwrap();
        release_build_save(&jobs, &l.api, &l.token, "n-1").await;
        // nothing of it is held any more: the same nonce starts over
        assert!(jobs.cancel("n-1").is_empty());
        jobs.upload_staged(
            &l.api,
            &l.token,
            &l.stager,
            l.channel,
            "n-1",
            &staged,
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(puts(&l), 2);
    }

    #[tokio::test]
    async fn a_save_post_is_retried_while_it_fails_retryably() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let no_wait = |_| Duration::ZERO;
        // a 502 twice, then it lands
        let n = AtomicU32::new(0);
        let r = with_retries(
            || {
                let i = n.fetch_add(1, Ordering::SeqCst);
                async move {
                    if i < 2 {
                        Err(ApiError::Server("502".into()))
                    } else {
                        Ok(i)
                    }
                }
            },
            no_wait,
        )
        .await;
        assert_eq!(r.unwrap(), 2);
        // refused: not tried again
        let n = AtomicU32::new(0);
        let r: Result<(), _> = with_retries(
            || {
                n.fetch_add(1, Ordering::SeqCst);
                async { Err(ApiError::Rejected("no".into())) }
            },
            no_wait,
        )
        .await;
        assert!(matches!(r, Err(ApiError::Rejected(_))));
        assert_eq!(n.load(Ordering::SeqCst), 1);
        // down every time: given up after three tries
        let n = AtomicU32::new(0);
        let r: Result<(), _> = with_retries(
            || {
                n.fetch_add(1, Ordering::SeqCst);
                async { Err(ApiError::Network("down".into())) }
            },
            no_wait,
        )
        .await;
        assert!(r.is_err());
        assert_eq!(n.load(Ordering::SeqCst), 3);
    }
}
