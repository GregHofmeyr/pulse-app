//! Builds channels (#617): browsing a channel's builds, their per-game counts, archiving and deleting them, and
//! searching builds across servers.

use pulse_protocol::ids::{ChannelId, GameId, MessageId};
use pulse_protocol::rest::{
    BuildFacets, BulkArchiveRequest, BulkArchiveResult, Message, SetBuildArchivedRequest,
};
use tauri::{AppHandle, State};

use crate::api::{Api, ApiError};
use crate::commands::Core;

impl Api {
    /// A page of a builds channel's builds (30), newest first by when they were saved; `before` pages back. Archived
    /// ones only with `archived`; `game` narrows to one game, `q` to builds with those words.
    pub async fn list_builds(
        &self,
        token: &str,
        channel: ChannelId,
        game: Option<GameId>,
        archived: bool,
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
        if let Some(q) = q.map(str::trim).filter(|q| !q.is_empty()) {
            req = req.query(&[("q", q)]);
        }
        if let Some(b) = before {
            req = req.query(&[("before", b.to_string())]);
        }
        Self::clip_parse(req.send().await).await
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

#[tauri::command]
pub async fn list_builds(
    app: AppHandle,
    core: State<'_, Core>,
    channel_id: ChannelId,
    game: Option<GameId>,
    archived: bool,
    q: Option<String>,
    before: Option<MessageId>,
) -> Result<Vec<Message>, ApiError> {
    let (api, token) = core.current()?;
    core.check(
        &app,
        api.list_builds(&token, channel_id, game, archived, q.as_deref(), before)
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
            .list_builds("t", ch, Some(game), true, Some("raven build"), Some(before))
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
        assert!(line.contains(&format!("before={before}")), "{line}");
    }

    #[tokio::test]
    async fn list_builds_without_filters_sends_none() {
        let (api, seen) = capture("[]").await;
        let ch = ChannelId::new();
        api.list_builds("t", ch, None, false, None, None)
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
}
