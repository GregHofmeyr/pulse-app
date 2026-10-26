//! HTTP client for the Pulse server. Only the Rust core talks to the network.

use std::time::Duration;

use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{
    AddMembersRequest, Channel, ChannelKind, CreateDmRequest, CreateEmojiRequest, CustomEmoji,
    EditMessageRequest, GoLiveRequest, LoginRequest, MarkReadRequest, MediaInfo, Member, Message,
    MuteTarget, RegisterRequest, RenameChannelRequest, RenameEmojiRequest, SendMessageRequest,
    Server, SessionResponse, SetMuteRequest, StreamInfo, StreamRole, StreamSettings,
    StreamTicketRequest, User, VoiceTokenResponse, WatchRequest,
};
use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("wrong username or password")]
    Unauthorized,
    #[error("{0}")]
    Rejected(String),
    /// 403: you are signed in but not allowed (for a stream ticket: not in that voice channel yet).
    #[error("{0}")]
    Forbidden(String),
    /// 5xx: the server (or a proxy in front of it) is having trouble; worth retrying.
    #[error("the server is having trouble ({0})")]
    Server(String),
    /// 429: we're going too fast; worth retrying a little later.
    #[error("slow down a little, then try again")]
    RateLimited,
    #[error("can't reach the server: {0}")]
    Network(String),
}

impl ApiError {
    /// The request (almost) certainly never reached Pulse: no connection, or the proxy found it
    /// down (502/503; a proxy can also say 502 after a reset mid-response, so a duplicate is
    /// possible but rare). Safe enough to send again.
    pub fn never_arrived(&self) -> bool {
        match self {
            Self::Network(t) => t == CONNECT_FAILED,
            Self::Server(s) => s.starts_with("502") || s.starts_with("503"),
            _ => false,
        }
    }

    /// Startup restores the last session; if its server doesn't answer, say which one (it may be
    /// an old dev server) rather than a bare network error.
    pub fn while_restoring(self, server: &str) -> Self {
        let host = server
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        let why = match self {
            Self::Network(_) => "it didn't answer",
            Self::Server(_) => "it's having trouble",
            other => return other,
        };
        Self::Rejected(format!(
            "Couldn't restore your last session on {host}: {why}. Log in below, or restart Pulse to try again."
        ))
    }
}

impl serde::Serialize for ApiError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// Plain http:// only for this machine; anything remote must be https (passwords + tokens).
pub fn check_server_url(url: &str) -> Result<(), ApiError> {
    let reject = |m: &str| Err(ApiError::Rejected(m.into()));
    if url.starts_with("https://") {
        return Ok(());
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return reject("server address must start with https://");
    };
    let authority = rest.split('/').next().unwrap_or_default();
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => authority.split(':').next().unwrap_or_default(),
    };
    if matches!(host, "localhost" | "127.0.0.1" | "::1") {
        Ok(())
    } else {
        reject("use https:// for remote servers")
    }
}

/// What a failed-request log line may show: the path only (no host, no query, never the token).
pub fn path_only(url: &reqwest::Url) -> String {
    url.path().to_string()
}

/// What an emoji upload over the server's body limit says.
pub const TOO_BIG: &str = "Files can be 2 MB at most";

/// The connection was never made: a request with this error certainly didn't arrive.
pub const CONNECT_FAILED: &str = "couldn't connect";

/// A reqwest error in plain words, without the URL (its Display would include host + query).
fn failure_text(e: reqwest::Error) -> String {
    // connect first: a connect timeout is both, and it certainly never arrived
    if e.is_connect() {
        CONNECT_FAILED.into()
    } else if e.is_timeout() {
        "it took too long to answer".into()
    } else {
        e.without_url().to_string()
    }
}

fn log_failure(path: Option<String>, what: &str) {
    tracing::warn!(
        path = path.as_deref().unwrap_or("?"),
        error = what,
        "request failed"
    );
}

/// A fresh HTTPS connection costs two extra round trips (TCP + TLS: ~320 ms on a 160 ms path),
/// so keep the one we have. reqwest's default drops it after 90 s idle, which made most voice
/// joins and first messages after a lull pay for a new one. Caddy keeps idle connections for
/// 60 min (deploy/Caddyfile); we let go a little earlier so we never reuse one it just closed.
pub const POOL_IDLE: Duration = Duration::from_secs(55 * 60);
/// TCP keepalive and HTTP/2 pings keep NAT and firewall mappings alive under an idle connection.
const TCP_KEEPALIVE: Duration = Duration::from_secs(30);
const H2_PING_EVERY: Duration = Duration::from_secs(25);
const H2_PING_TIMEOUT: Duration = Duration::from_secs(10);

// Tests count calls on their own thread (the log line itself is once per process).
#[cfg(test)]
thread_local!(static NOTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) });
#[cfg(test)]
fn noted() -> usize {
    NOTED.with(|n| n.get())
}

/// Logged once per run: the HTTP version the server agreed to (h2 through ALPN behind Caddy).
fn note_version(r: &reqwest::Response) {
    #[cfg(test)]
    NOTED.with(|n| n.set(n.get() + 1));
    static ONCE: std::sync::Once = std::sync::Once::new();
    let version = r.version();
    ONCE.call_once(|| tracing::info!(?version, "rest: connected"));
}

#[derive(Clone)]
pub struct Api {
    base: String,
    http: reqwest::Client,
}

impl Api {
    pub fn new(base: &str) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .pool_idle_timeout(POOL_IDLE)
            .tcp_keepalive(TCP_KEEPALIVE)
            .http2_keep_alive_interval(H2_PING_EVERY)
            .http2_keep_alive_timeout(H2_PING_TIMEOUT)
            .http2_keep_alive_while_idle(true)
            .build()
            .expect("http client");
        Self {
            base: base.trim_end_matches('/').to_string(),
            http,
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// The response if it succeeded; otherwise the matching `ApiError`.
    async fn checked(
        r: Result<reqwest::Response, reqwest::Error>,
    ) -> Result<reqwest::Response, ApiError> {
        if let Ok(resp) = &r {
            note_version(resp);
        }
        let r = r.map_err(|e| {
            let path = e.url().map(path_only);
            let text = failure_text(e);
            log_failure(path, &text);
            ApiError::Network(text)
        })?;
        if !r.status().is_success() {
            log_failure(Some(path_only(r.url())), r.status().as_str());
        }
        if r.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ApiError::Unauthorized);
        }
        if r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ApiError::RateLimited);
        }
        if r.status().is_server_error() {
            return Err(ApiError::Server(r.status().to_string()));
        }
        if !r.status().is_success() {
            let status = r.status();
            let msg = r
                .json::<pulse_protocol::rest::ApiError>()
                .await
                .map(|e| e.message)
                .unwrap_or_else(|_| "unexpected server error".into());
            return Err(if status == reqwest::StatusCode::FORBIDDEN {
                ApiError::Forbidden(msg)
            } else {
                ApiError::Rejected(msg)
            });
        }
        Ok(r)
    }

    async fn parse<T: DeserializeOwned>(
        r: Result<reqwest::Response, reqwest::Error>,
    ) -> Result<T, ApiError> {
        Self::checked(r)
            .await?
            .json()
            .await
            .map_err(|e| ApiError::Network(failure_text(e)))
    }

    /// For endpoints that answer 204.
    async fn ok_empty(r: Result<reqwest::Response, reqwest::Error>) -> Result<(), ApiError> {
        Self::checked(r).await.map(|_| ())
    }

    pub async fn create_server(&self, token: &str, name: &str) -> Result<Server, ApiError> {
        let body = pulse_protocol::rest::CreateServerRequest { name: name.into() };
        Self::parse(
            self.http
                .post(format!("{}/servers", self.base))
                .bearer_auth(token)
                .json(&body)
                .send()
                .await,
        )
        .await
    }

    pub async fn create_channel(
        &self,
        token: &str,
        server: ServerId,
        kind: ChannelKind,
        name: &str,
    ) -> Result<Channel, ApiError> {
        let body = pulse_protocol::rest::CreateChannelRequest {
            kind,
            name: name.into(),
        };
        Self::parse(
            self.http
                .post(format!("{}/servers/{server}/channels", self.base))
                .bearer_auth(token)
                .json(&body)
                .send()
                .await,
        )
        .await
    }

    pub async fn create_dm(&self, token: &str, user_ids: Vec<UserId>) -> Result<Channel, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/dms", self.base))
                .bearer_auth(token)
                .json(&CreateDmRequest { user_ids })
                .send()
                .await,
        )
        .await
    }

    pub async fn add_members(
        &self,
        token: &str,
        ch: ChannelId,
        user_ids: Vec<UserId>,
    ) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .post(format!("{}/channels/{ch}/members", self.base))
                .bearer_auth(token)
                .json(&AddMembersRequest { user_ids })
                .send()
                .await,
        )
        .await
    }

    pub async fn remove_member(
        &self,
        token: &str,
        ch: ChannelId,
        user: UserId,
    ) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .delete(format!("{}/channels/{ch}/members/{user}", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn rename_channel(
        &self,
        token: &str,
        ch: ChannelId,
        name: Option<String>,
    ) -> Result<Channel, ApiError> {
        Self::parse(
            self.http
                .patch(format!("{}/channels/{ch}", self.base))
                .bearer_auth(token)
                .json(&RenameChannelRequest { name })
                .send()
                .await,
        )
        .await
    }

    pub async fn mark_read(
        &self,
        token: &str,
        ch: ChannelId,
        message_id: MessageId,
    ) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .post(format!("{}/channels/{ch}/read", self.base))
                .bearer_auth(token)
                .json(&MarkReadRequest { message_id })
                .send()
                .await,
        )
        .await
    }

    pub async fn set_mute(&self, token: &str, req: &SetMuteRequest) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .put(format!("{}/mutes", self.base))
                .bearer_auth(token)
                .json(req)
                .send()
                .await,
        )
        .await
    }

    pub async fn clear_mute(
        &self,
        token: &str,
        kind: MuteTarget,
        id: &str,
    ) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .delete(format!("{}/mutes/{}/{id}", self.base, kind.as_str()))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn close_conversation(&self, token: &str, ch: ChannelId) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .post(format!("{}/channels/{ch}/close", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn join_server(&self, token: &str, server: ServerId) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .post(format!("{}/servers/{server}/join", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn channels(&self, token: &str, server: ServerId) -> Result<Vec<Channel>, ApiError> {
        Self::parse(
            self.http
                .get(format!("{}/servers/{server}/channels", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn members(&self, token: &str, server: ServerId) -> Result<Vec<Member>, ApiError> {
        Self::parse(
            self.http
                .get(format!("{}/servers/{server}/members", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn messages(
        &self,
        token: &str,
        channel: ChannelId,
        before: Option<MessageId>,
    ) -> Result<Vec<Message>, ApiError> {
        let mut req = self
            .http
            .get(format!("{}/channels/{channel}/messages", self.base))
            .bearer_auth(token);
        if let Some(b) = before {
            req = req.query(&[("before", b.to_string())]);
        }
        Self::parse(req.send().await).await
    }

    pub async fn send_message(
        &self,
        token: &str,
        channel: ChannelId,
        body: &SendMessageRequest,
    ) -> Result<Message, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/channels/{channel}/messages", self.base))
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    pub async fn edit_message(
        &self,
        token: &str,
        id: MessageId,
        content: &str,
    ) -> Result<Message, ApiError> {
        let body = EditMessageRequest {
            content: content.into(),
        };
        Self::parse(
            self.http
                .patch(format!("{}/messages/{id}", self.base))
                .bearer_auth(token)
                .json(&body)
                .send()
                .await,
        )
        .await
    }

    pub async fn delete_message(&self, token: &str, id: MessageId) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .delete(format!("{}/messages/{id}", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn voice_token(
        &self,
        token: &str,
        channel: ChannelId,
    ) -> Result<VoiceTokenResponse, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/voice/{channel}/token", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn stream_ticket(
        &self,
        token: &str,
        channel: ChannelId,
        role: StreamRole,
    ) -> Result<VoiceTokenResponse, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/streams/{channel}/ticket", self.base))
                .bearer_auth(token)
                .json(&StreamTicketRequest { role })
                .send()
                .await,
        )
        .await
    }

    pub async fn go_live(
        &self,
        token: &str,
        channel: ChannelId,
        settings: StreamSettings,
    ) -> Result<StreamInfo, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/streams/{channel}/live", self.base))
                .bearer_auth(token)
                .json(&GoLiveRequest { settings })
                .send()
                .await,
        )
        .await
    }

    pub async fn stop_live(&self, token: &str, channel: ChannelId) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .delete(format!("{}/streams/{channel}/live", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn watch_stream(
        &self,
        token: &str,
        channel: ChannelId,
        streamer: UserId,
    ) -> Result<(), ApiError> {
        self.watch_call(token, channel, "watch", streamer).await
    }

    pub async fn unwatch_stream(
        &self,
        token: &str,
        channel: ChannelId,
        streamer: UserId,
    ) -> Result<(), ApiError> {
        self.watch_call(token, channel, "unwatch", streamer).await
    }

    async fn watch_call(
        &self,
        token: &str,
        channel: ChannelId,
        action: &str,
        streamer: UserId,
    ) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .post(format!("{}/streams/{channel}/{action}", self.base))
                .bearer_auth(token)
                .json(&WatchRequest { streamer })
                .send()
                .await,
        )
        .await
    }

    pub async fn put_preview(
        &self,
        token: &str,
        channel: ChannelId,
        webp: Vec<u8>,
    ) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .put(format!("{}/streams/{channel}/preview", self.base))
                .bearer_auth(token)
                .header(reqwest::header::CONTENT_TYPE, "image/webp")
                .body(webp)
                .send()
                .await,
        )
        .await
    }

    /// A live stream's latest preview; `None` when there is none (or it isn't live).
    pub async fn get_preview(
        &self,
        token: &str,
        channel: ChannelId,
        user: UserId,
    ) -> Result<Option<Vec<u8>>, ApiError> {
        let r = self
            .http
            .get(format!("{}/streams/{channel}/{user}/preview", self.base))
            .bearer_auth(token)
            .send()
            .await;
        if matches!(&r, Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND) {
            return Ok(None);
        }
        let resp = Self::checked(r).await?;
        resp.bytes()
            .await
            .map(|b| Some(b.to_vec()))
            .map_err(|e| ApiError::Network(failure_text(e)))
    }

    pub async fn register(
        &self,
        invite_code: &str,
        username: &str,
        password: &str,
    ) -> Result<SessionResponse, ApiError> {
        let body = RegisterRequest {
            invite_code: invite_code.into(),
            username: username.into(),
            password: password.into(),
        };
        Self::parse(
            self.http
                .post(format!("{}/auth/register", self.base))
                .json(&body)
                .send()
                .await,
        )
        .await
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<SessionResponse, ApiError> {
        let body = LoginRequest {
            username: username.into(),
            password: password.into(),
        };
        Self::parse(
            self.http
                .post(format!("{}/auth/login", self.base))
                .json(&body)
                .send()
                .await,
        )
        .await
    }

    pub async fn logout(&self, token: &str) -> Result<(), ApiError> {
        self.http
            .post(format!("{}/auth/logout", self.base))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| ApiError::Network(failure_text(e)))?;
        Ok(())
    }

    /// Sign out every other device of this account; returns how many sessions were revoked.
    pub async fn logout_others(&self, token: &str) -> Result<u32, ApiError> {
        let r: pulse_protocol::rest::LogoutOthersResponse = Self::parse(
            self.http
                .post(format!("{}/auth/logout-others", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await?;
        Ok(r.revoked)
    }

    /// Reports carry up to 256 KB of log: allow longer than the usual 15 s on a slow line.
    pub async fn create_report(
        &self,
        token: &str,
        req: &pulse_protocol::rest::CreateReportRequest,
    ) -> Result<pulse_protocol::rest::CreateReportResponse, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/reports", self.base))
                .bearer_auth(token)
                .timeout(Duration::from_secs(45))
                .json(req)
                .send()
                .await,
        )
        .await
    }

    pub async fn me(&self, token: &str) -> Result<User, ApiError> {
        Self::parse(
            self.http
                .get(format!("{}/me", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn servers(&self, token: &str) -> Result<Vec<Server>, ApiError> {
        Self::parse(
            self.http
                .get(format!("{}/servers", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// Processing a 2 MB image takes a moment: allow longer than the usual 15 s.
    pub async fn upload_emoji_media(
        &self,
        token: &str,
        bytes: Vec<u8>,
    ) -> Result<MediaInfo, ApiError> {
        let r = self
            .http
            .post(format!("{}/media/emoji", self.base))
            .bearer_auth(token)
            .timeout(Duration::from_secs(45))
            .body(bytes)
            .send()
            .await;
        // axum's body limit answers in plain text, not our JSON error
        if let Ok(resp) = &r
            && resp.status() == reqwest::StatusCode::PAYLOAD_TOO_LARGE
        {
            note_version(resp);
            log_failure(Some(path_only(resp.url())), resp.status().as_str());
            return Err(ApiError::Rejected(TOO_BIG.into()));
        }
        Self::parse(r).await
    }

    pub async fn media(&self, token: &str, hash: &str) -> Result<Vec<u8>, ApiError> {
        if !crate::media::is_hash(hash) {
            return Err(ApiError::Rejected("not a media hash".into()));
        }
        let r = self
            .http
            .get(self.url(&["media", hash])?)
            .bearer_auth(token)
            .send()
            .await;
        let bytes = Self::checked(r)
            .await?
            .bytes()
            .await
            .map_err(|e| ApiError::Network(failure_text(e)))?;
        Ok(bytes.to_vec())
    }

    pub async fn create_emoji(
        &self,
        token: &str,
        server: ServerId,
        name: &str,
        media_hash: &str,
    ) -> Result<CustomEmoji, ApiError> {
        let body = CreateEmojiRequest {
            name: name.into(),
            media_hash: media_hash.into(),
        };
        Self::parse(
            self.http
                .post(format!("{}/servers/{server}/emoji", self.base))
                .bearer_auth(token)
                .json(&body)
                .send()
                .await,
        )
        .await
    }

    pub async fn rename_emoji(
        &self,
        token: &str,
        id: &str,
        name: &str,
    ) -> Result<CustomEmoji, ApiError> {
        let body = RenameEmojiRequest { name: name.into() };
        Self::parse(
            self.http
                .patch(self.url(&["emoji", id])?)
                .bearer_auth(token)
                .json(&body)
                .send()
                .await,
        )
        .await
    }

    pub async fn delete_emoji(&self, token: &str, id: &str) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .delete(self.url(&["emoji", id])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// `emoji`: a Unicode emoji or `custom:<id>`.
    pub async fn add_reaction(
        &self,
        token: &str,
        message: MessageId,
        emoji: &str,
    ) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .put(self.reaction_url(message, emoji)?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn remove_reaction(
        &self,
        token: &str,
        message: MessageId,
        emoji: &str,
    ) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .delete(self.reaction_url(message, emoji)?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn reactors(
        &self,
        token: &str,
        message: MessageId,
        emoji: &str,
    ) -> Result<Vec<UserId>, ApiError> {
        Self::parse(
            self.http
                .get(self.reaction_url(message, emoji)?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    fn reaction_url(&self, message: MessageId, emoji: &str) -> Result<reqwest::Url, ApiError> {
        self.url(&["messages", &message.to_string(), "reactions", emoji])
    }

    /// `base/seg/seg…`, each segment percent-encoded on its own (an emoji or id can't add a `/`).
    /// A `.` or `..` segment is refused: the URL parser would drop it (or the one before it) and
    /// send the request somewhere else.
    fn url(&self, segments: &[&str]) -> Result<reqwest::Url, ApiError> {
        if segments.iter().any(|s| matches!(*s, "." | "..")) {
            return Err(ApiError::Rejected("that name can't be used here".into()));
        }
        let bad = || ApiError::Rejected("the server address isn't valid".into());
        let mut url = reqwest::Url::parse(&self.base).map_err(|_| bad())?;
        url.path_segments_mut()
            .map_err(|_| bad())?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_server::testing;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A bare HTTP/1.1 server that answers every request with 204 and never closes an idle
    /// connection itself (axum's test server drops idle ones after 30 s: hyper's header read
    /// timeout), counting the connections made to it.
    async fn keepalive_server() -> (String, Arc<AtomicUsize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("localhost:0").await.unwrap();
        let base = format!("http://localhost:{}", l.local_addr().unwrap().port());
        let conns = Arc::new(AtomicUsize::new(0));
        let count = conns.clone();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                count.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let (mut buf, mut chunk) = (Vec::new(), [0u8; 1024]);
                    loop {
                        let Ok(n) = s.read(&mut chunk).await else {
                            return;
                        };
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        // the requests in these tests have no body: a blank line ends each one
                        while let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            buf.drain(..end + 4);
                            if s.write_all(b"HTTP/1.1 204 No Content\r\n\r\n")
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                });
            }
        });
        (base, conns)
    }

    #[tokio::test]
    async fn back_to_back_requests_share_one_connection() {
        let (base, conns) = keepalive_server().await;
        let api = Api::new(&base);
        api.logout("t").await.unwrap();
        api.logout("t").await.unwrap();
        assert_eq!(conns.load(Ordering::SeqCst), 1);
    }

    /// Field log 2026-10-13: a voice join 3.5 min after the last request paid ~300 ms more for
    /// its token (a new TCP + TLS handshake). The connection must survive a long lull.
    #[tokio::test]
    #[ignore = "idles 100 s; run: cargo test -p pulse-client --lib rest_connection -- --ignored"]
    async fn rest_connection_survives_a_long_idle() {
        let (base, conns) = keepalive_server().await;
        let api = Api::new(&base);
        api.logout("t").await.unwrap();
        tokio::time::sleep(Duration::from_secs(100)).await; // past reqwest's 90 s default
        api.logout("t").await.unwrap();
        assert_eq!(
            conns.load(Ordering::SeqCst),
            1,
            "the idle connection was dropped: the second call paid a new handshake"
        );
    }

    #[test]
    fn idle_connections_are_kept_longer_than_a_lull_but_shorter_than_caddy_keeps_them() {
        assert!(POOL_IDLE >= Duration::from_secs(30 * 60));
        assert!(
            POOL_IDLE < Duration::from_secs(60 * 60),
            "Caddy's idle timeout is 60 min"
        );
    }

    #[tokio::test]
    async fn a_report_reaches_the_server() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let req = pulse_protocol::rest::CreateReportRequest {
            kind: pulse_protocol::rest::ReportKind::Bug,
            text: "voice cut out".into(),
            meta: serde_json::json!({}),
            log: String::new(),
        };
        assert_eq!(api.create_report(&token, &req).await.unwrap().id.len(), 26);
    }

    #[tokio::test]
    async fn logout_others_signs_out_the_other_sessions_and_reports_how_many() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let other = api.login("alex", "hunter2hunter2").await.unwrap().token;
        assert_eq!(api.logout_others(&token).await.unwrap(), 1);
        assert!(matches!(api.me(&other).await, Err(ApiError::Unauthorized)));
        assert!(api.me(&token).await.is_ok(), "this device stays signed in");
        assert_eq!(api.logout_others(&token).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn logout_others_with_a_dead_token_is_unauthorized() {
        let app = testing::spawn().await;
        let api = Api::new(&format!("http://{}", app.addr));
        assert!(matches!(
            api.logout_others("garbage").await,
            Err(ApiError::Unauthorized)
        ));
    }

    #[test]
    fn a_failed_restore_says_which_server_and_what_to_do() {
        let e =
            ApiError::Network("couldn't connect".into()).while_restoring("http://localhost:7890");
        let t = e.to_string();
        assert!(t.contains("localhost:7890") && t.contains("restore"), "{t}");
        assert!(!t.contains("http://") && !t.contains("/me"), "{t}");
        // anything that isn't about reaching the server is passed through as is
        assert!(matches!(
            ApiError::RateLimited.while_restoring("https://pulsechat.co.za"),
            ApiError::RateLimited
        ));
    }

    /// A server that never answers the connection (dropped SYNs) times out while connecting:
    /// that request certainly never arrived, so it must read (and retry) as a connect failure.
    #[tokio::test]
    async fn a_connect_timeout_is_a_connect_failure() {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_millis(200))
            .build()
            .unwrap();
        // TEST-NET-1: routable nowhere, so the connection hangs (or fails at once offline)
        let e = client.get("http://192.0.2.1:9/").send().await.unwrap_err();
        if !e.is_timeout() {
            // no route here (offline / sandbox): the connect failed at once, so this run says
            // nothing about timeouts
            eprintln!("skipped: connect failed immediately ({e})");
            return;
        }
        assert_eq!(failure_text(e), CONNECT_FAILED);
    }

    #[tokio::test]
    async fn connection_failures_read_plainly() {
        let e = reqwest::get("http://127.0.0.1:9/x").await.unwrap_err();
        assert_eq!(failure_text(e), "couldn't connect");
    }

    #[tokio::test]
    async fn failure_text_leaves_out_the_url() {
        let e = reqwest::get("http://127.0.0.1:9/x?token=secret")
            .await
            .unwrap_err();
        let t = failure_text(e);
        assert!(!t.contains("secret") && !t.contains("127.0.0.1"), "{t}");
    }

    #[test]
    fn failed_request_lines_keep_only_the_path() {
        let u = reqwest::Url::parse(
            "https://pulsechat.co.za/channels/X/messages?before=Y&token=secret",
        )
        .unwrap();
        assert_eq!(path_only(&u), "/channels/X/messages");
    }

    #[tokio::test]
    async fn register_then_login_against_test_server_returns_user() {
        let app = testing::spawn().await;
        let code = testing::invite(&app).await;
        let api = Api::new(&format!("http://{}", app.addr));

        let reg = api.register(&code, "alex", "hunter2hunter2").await.unwrap();
        assert_eq!(reg.user.username, "alex");

        let login = api.login("alex", "hunter2hunter2").await.unwrap();
        assert_eq!(login.user.id, reg.user.id);
        assert_eq!(api.me(&login.token).await.unwrap().username, "alex");
    }

    #[tokio::test]
    async fn too_many_failed_logins_says_slow_down() {
        use pulse_server::limits::{LimitsConfig, Rate};
        let mut cfg = testing::test_config();
        // The production burst of 5, but no refill: argon2 is capped at 2 hashes per process, so
        // under a busy test run these 5 attempts can take 30 s, and per_minute(5) would hand
        // back a token every 12 s and let the 6th through to the password check.
        cfg.limits = LimitsConfig {
            login_ip: Rate::new(LimitsConfig::default().login_ip.burst, 0.0),
            ..LimitsConfig::default()
        };
        let app = testing::spawn_with(cfg).await;
        testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        for _ in 0..5 {
            let _ = api.login("alex", "nopenopenope").await;
        }
        let err = api.login("alex", "nopenopenope").await.unwrap_err();
        assert!(matches!(err, ApiError::RateLimited), "{err:?}");
        assert_eq!(err.to_string(), "slow down a little, then try again");
        assert!(
            crate::outbox::retryable(&err),
            "a rate-limited send is retried later"
        );
    }

    #[tokio::test]
    async fn wrong_password_maps_to_unauthorized() {
        let app = testing::spawn().await;
        testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let err = api.login("alex", "nopenopenope").await.unwrap_err();
        assert!(matches!(err, ApiError::Unauthorized), "{err:?}");
    }

    #[tokio::test]
    async fn lists_servers() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        testing::create_server(&app, &token, "Main Hangout").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let servers = api.servers(&token).await.unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "Main Hangout");
    }

    #[tokio::test]
    async fn join_then_post_and_page_messages() {
        let app = testing::spawn().await;
        let (_, owner) = testing::register(&app, "alex").await;
        let (_, token) = testing::register(&app, "sam").await;
        let s = testing::create_server(&app, &owner, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        api.join_server(&token, s.id).await.unwrap();
        let chans = api.channels(&token, s.id).await.unwrap();
        let general = chans
            .iter()
            .find(|c| c.name.as_deref() == Some("general"))
            .unwrap();
        for i in 0..55 {
            let req = SendMessageRequest {
                content: format!("m{i}"),
                reply_to_id: None,
                nonce: Some(format!("n{i}")),
            };
            api.send_message(&token, general.id, &req).await.unwrap();
        }
        let page1 = api.messages(&token, general.id, None).await.unwrap();
        assert_eq!(page1.len(), 50);
        let page2 = api
            .messages(&token, general.id, Some(page1[49].id))
            .await
            .unwrap();
        assert_eq!(page2.len(), 5);
        assert_eq!(api.members(&token, s.id).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn edit_and_delete_roundtrip() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let g = testing::general(&app, &token, s.id).await;
        let api = Api::new(&format!("http://{}", app.addr));
        let m = api
            .send_message(
                &token,
                g.id,
                &SendMessageRequest {
                    content: "tpyo".into(),
                    reply_to_id: None,
                    nonce: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            api.edit_message(&token, m.id, "typo")
                .await
                .unwrap()
                .content,
            "typo"
        );
        api.delete_message(&token, m.id).await.unwrap();
        assert!(api.messages(&token, g.id, None).await.unwrap()[0].deleted);
    }

    #[tokio::test]
    async fn voice_token_after_join() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let lounge = api
            .channels(&token, s.id)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.name.as_deref() == Some("Lounge"))
            .unwrap();
        let t = api.voice_token(&token, lounge.id).await.unwrap();
        assert!(!t.token.is_empty());
    }

    /// A server voice channel plus a client for it, with one registered user.
    async fn voice_fixture(
        app: &testing::TestApp,
        name: &str,
    ) -> (Api, String, pulse_protocol::ids::UserId, ChannelId) {
        let (uid, token) = testing::register(app, name).await;
        let s = testing::create_server(app, &token, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let lounge = api
            .channels(&token, s.id)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.name.as_deref() == Some("Lounge"))
            .unwrap();
        (api, token, uid, lounge.id)
    }

    fn settings() -> StreamSettings {
        StreamSettings {
            height: 720,
            fps: 60,
            audio: true,
        }
    }

    #[tokio::test]
    async fn stream_ticket_after_join() {
        let app = testing::spawn().await;
        let (api, token, uid, ch) = voice_fixture(&app, "alex").await;
        // not in the call yet: refused, and as Forbidden (the retry keys on it)
        assert!(matches!(
            api.stream_ticket(&token, ch, StreamRole::V).await,
            Err(ApiError::Forbidden(_))
        ));
        app.voice.join(ch, uid, "PA_test");
        for role in [StreamRole::V, StreamRole::A] {
            let t = api.stream_ticket(&token, ch, role).await.unwrap();
            assert!(!t.token.is_empty());
        }
    }

    #[tokio::test]
    async fn go_live_then_stop_round_trip() {
        let app = testing::spawn().await;
        let (api, token, uid, ch) = voice_fixture(&app, "alex").await;
        app.voice.join(ch, uid, "PA_test");
        let info = api.go_live(&token, ch, settings()).await.unwrap();
        assert_eq!((info.channel_id, info.user_id), (ch, uid));
        assert_eq!(info.settings, settings());
        api.stop_live(&token, ch).await.unwrap();
        // already stopped: the server says 404
        assert!(matches!(
            api.stop_live(&token, ch).await,
            Err(ApiError::Rejected(_))
        ));
    }

    #[tokio::test]
    async fn watch_then_unwatch() {
        let app = testing::spawn().await;
        let (api, token, uid, ch) = voice_fixture(&app, "alex").await;
        let (sam, sam_token) = testing::register(&app, "sam").await;
        let server = api.servers(&token).await.unwrap()[0].id;
        let joined = testing::post_json(
            &app,
            &sam_token,
            &format!("/servers/{server}/join"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(joined.status(), 204);
        app.voice.join(ch, uid, "PA_test");
        app.voice.join(ch, sam, "PA_test");
        api.go_live(&token, ch, settings()).await.unwrap();
        api.watch_stream(&sam_token, ch, uid).await.unwrap();
        api.unwatch_stream(&sam_token, ch, uid).await.unwrap();
        // you can't watch yourself
        assert!(matches!(
            api.watch_stream(&token, ch, uid).await,
            Err(ApiError::Rejected(_))
        ));
    }

    #[tokio::test]
    async fn preview_put_then_get() {
        let app = testing::spawn().await;
        let (api, token, uid, ch) = voice_fixture(&app, "alex").await;
        app.voice.join(ch, uid, "PA_test");
        api.go_live(&token, ch, settings()).await.unwrap();
        assert_eq!(api.get_preview(&token, ch, uid).await.unwrap(), None);
        // A real 1×1 WebP: the server decodes and re-encodes previews, refusing anything else.
        use base64::Engine;
        let webp = base64::engine::general_purpose::STANDARD
            .decode("UklGRhoAAABXRUJQVlA4TA0AAAAvAAAAEAcQERGIiP4HAA==")
            .unwrap();
        api.put_preview(&token, ch, webp).await.unwrap();
        let got = api.get_preview(&token, ch, uid).await.unwrap().unwrap();
        assert_eq!((&got[..4], &got[8..12]), (&b"RIFF"[..], &b"WEBP"[..]));
        // and junk is refused
        assert!(
            api.put_preview(&token, ch, b"RIFF....WEBPfake".to_vec())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn stream_ticket_retries_a_403_while_voice_join_lands() {
        let app = testing::spawn().await;
        let (api, token, uid, ch) = voice_fixture(&app, "alex").await;
        let voice = app.voice.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            voice.join(ch, uid, "PA_test"); // what the LiveKit join webhook does
        });
        let t = crate::stream::ticket_with_retry(&api, &token, ch, StreamRole::V)
            .await
            .unwrap();
        assert!(!t.token.is_empty());
    }

    #[tokio::test]
    async fn stream_ticket_gives_up_on_a_persistent_403() {
        let app = testing::spawn().await;
        let (api, token, _, ch) = voice_fixture(&app, "alex").await;
        let started = std::time::Instant::now();
        let r = crate::stream::ticket_with_retry(&api, &token, ch, StreamRole::V).await;
        assert!(matches!(r, Err(ApiError::Forbidden(_))));
        assert!(started.elapsed() >= std::time::Duration::from_millis(1500));
    }

    #[tokio::test]
    async fn reannounce_retries_a_403_until_the_server_knows_we_are_in_voice() {
        let app = testing::spawn().await;
        let (api, token, uid, ch) = voice_fixture(&app, "alex").await;
        let voice = app.voice.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            voice.join(ch, uid, "PA_test"); // the restarted server learns we are in voice
        });
        let info = crate::stream::reannounce_with_retry(&api, &token, ch, settings())
            .await
            .unwrap();
        assert_eq!(info.user_id, uid);
    }

    #[tokio::test]
    async fn reannounce_gives_up_on_a_persistent_403_but_not_on_a_400_at_once() {
        let app = testing::spawn().await;
        let (api, token, uid, ch) = voice_fixture(&app, "alex").await;
        let started = std::time::Instant::now();
        let r = crate::stream::reannounce_with_retry(&api, &token, ch, settings()).await;
        assert!(matches!(r, Err(ApiError::Forbidden(_))), "{r:?}");
        assert!(started.elapsed() >= std::time::Duration::from_millis(4500));

        app.voice.join(ch, uid, "PA_test");
        let mut odd = settings();
        odd.height = 999;
        let started = std::time::Instant::now();
        let r = crate::stream::reannounce_with_retry(&api, &token, ch, odd).await;
        assert!(matches!(r, Err(ApiError::Rejected(_))), "{r:?}");
        assert!(started.elapsed() < std::time::Duration::from_millis(1000));
    }

    #[test]
    fn remote_http_refused() {
        assert!(check_server_url("http://localhost:7890").is_ok());
        assert!(check_server_url("http://127.0.0.1:7890").is_ok());
        assert!(check_server_url("http://[::1]:7890").is_ok());
        assert!(check_server_url("https://pulse.example.com").is_ok());
        assert!(matches!(
            check_server_url("http://pulse.example.com"),
            Err(ApiError::Rejected(_))
        ));
        assert!(matches!(
            check_server_url("ftp://x"),
            Err(ApiError::Rejected(_))
        ));
    }

    #[tokio::test]
    async fn timeout_on_unroutable_host() {
        let api = Api::new("http://10.255.255.1:9");
        let started = std::time::Instant::now();
        let err = api.login("a", "b").await.unwrap_err();
        assert!(matches!(err, ApiError::Network(_)), "{err:?}");
        assert!(started.elapsed() < std::time::Duration::from_secs(7));
    }

    #[tokio::test]
    async fn create_server_then_listed() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let s = api.create_server(&token, "Game Night").await.unwrap();
        assert_eq!(s.name, "Game Night");
        assert_eq!(api.servers(&token).await.unwrap(), vec![s]);
        assert!(matches!(
            api.create_server(&token, "   ").await,
            Err(ApiError::Rejected(_))
        ));
    }

    #[tokio::test]
    async fn create_text_and_voice_channels() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let s = api.create_server(&token, "Main").await.unwrap();
        let t = api
            .create_channel(&token, s.id, ChannelKind::Text, "memes")
            .await
            .unwrap();
        let v = api
            .create_channel(&token, s.id, ChannelKind::Voice, "Gaming")
            .await
            .unwrap();
        assert_eq!(
            (t.kind, t.name.as_deref()),
            (ChannelKind::Text, Some("memes"))
        );
        assert_eq!(
            (v.kind, v.name.as_deref()),
            (ChannelKind::Voice, Some("Gaming"))
        );
        assert_eq!(api.channels(&token, s.id).await.unwrap().len(), 4);
    }

    #[test]
    fn reaction_emoji_is_one_path_segment() {
        let api = Api::new("https://pulse.example.com/");
        let id: MessageId = "01J0000000000000000000000A".parse().unwrap();
        let u = api.reaction_url(id, "👍🏽").unwrap();
        assert_eq!(
            u.as_str(),
            "https://pulse.example.com/messages/01J0000000000000000000000A/reactions/%F0%9F%91%8D%F0%9F%8F%BD"
        );
        let u = api.reaction_url(id, "custom:01ABC").unwrap();
        assert!(u.as_str().ends_with("/reactions/custom:01ABC"), "{u}");
        let u = api.reaction_url(id, "a/../b%").unwrap();
        assert!(u.as_str().ends_with("/reactions/a%2F..%2Fb%25"), "{u}");
    }

    #[test]
    fn dot_segments_are_refused_not_dropped() {
        let api = Api::new("https://pulse.example.com/");
        let id: MessageId = "01J0000000000000000000000A".parse().unwrap();
        for seg in [".", ".."] {
            assert!(api.reaction_url(id, seg).is_err(), "{seg:?}");
            assert!(api.url(&["emoji", seg]).is_err(), "{seg:?}");
        }
        // only whole segments: dots inside one are ordinary text
        assert!(api.url(&["emoji", "..a"]).is_ok());
        assert!(api.url(&["emoji", "a.b"]).is_ok());
    }

    #[tokio::test]
    async fn media_refuses_anything_but_a_hash() {
        // never sent: nothing listens here
        let api = Api::new("http://localhost:9");
        let hash = "ab0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcd";
        for bad in [
            "..",
            "../me",
            &hash.to_uppercase(),
            &hash[1..],
            &format!("{hash}/x"),
            &format!("{}?x", &hash[1..]),
        ] {
            match api.media("t", bad).await {
                Err(ApiError::Rejected(_)) => {}
                other => panic!("{bad:?}: {:?}", other.map(|b| b.len())),
            }
        }
        // a valid hash is sent (and fails to connect)
        assert!(matches!(
            api.media("t", hash).await,
            Err(ApiError::Network(_))
        ));
    }

    #[tokio::test]
    async fn custom_emoji_roundtrip() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let media = api
            .upload_emoji_media(&token, crate::media::tests::PNG.to_vec())
            .await
            .unwrap();
        let bytes = api.media(&token, &media.hash).await.unwrap();
        assert!(crate::media::verify(&media.hash, &bytes));
        let e = api
            .create_emoji(&token, s.id, "pog", &media.hash)
            .await
            .unwrap();
        assert_eq!(
            (e.name.as_str(), e.hash.as_deref()),
            ("pog", Some(media.hash.as_str()))
        );
        assert_eq!(
            api.rename_emoji(&token, &e.id, "poggers")
                .await
                .unwrap()
                .name,
            "poggers"
        );
        api.delete_emoji(&token, &e.id).await.unwrap();
    }

    #[tokio::test]
    async fn an_oversized_upload_says_how_big_files_can_be() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let err = api
            .upload_emoji_media(&token, vec![0; 3 * 1024 * 1024])
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), TOO_BIG);
    }

    #[tokio::test]
    async fn an_oversized_upload_still_notes_the_http_version() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let before = noted();
        let err = api
            .upload_emoji_media(&token, vec![0; 3 * 1024 * 1024])
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), TOO_BIG);
        assert_eq!(noted(), before + 1);
    }

    #[tokio::test]
    async fn react_list_and_unreact() {
        let app = testing::spawn().await;
        let (me, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let g = testing::general(&app, &token, s.id).await;
        let api = Api::new(&format!("http://{}", app.addr));
        let m = api
            .send_message(
                &token,
                g.id,
                &SendMessageRequest {
                    content: "hi".into(),
                    reply_to_id: None,
                    nonce: None,
                },
            )
            .await
            .unwrap();
        api.add_reaction(&token, m.id, "👍🏽").await.unwrap();
        assert_eq!(api.reactors(&token, m.id, "👍🏽").await.unwrap(), vec![me]);
        api.remove_reaction(&token, m.id, "👍🏽").await.unwrap();
        assert!(api.reactors(&token, m.id, "👍🏽").await.unwrap().is_empty());
    }
}
