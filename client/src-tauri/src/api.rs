//! HTTP client for the Pulse server. Only the Rust core talks to the network.

use std::time::Duration;

use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{
    AddMembersRequest, Channel, ChannelKind, CreateDmRequest, CreateEmojiRequest, CustomEmoji,
    EditMessageRequest, GoLiveRequest, LoginMfaRequest, LoginRequest, MarkReadRequest, MediaInfo,
    Member, Message, MfaRequired, MuteTarget, RecoveryCodesResponse, RegisterRequest,
    RenameChannelRequest, RenameEmojiRequest, SendMessageRequest, Server, SessionResponse,
    SetMuteRequest, StreamInfo, StreamRole, StreamSettings, StreamTicketRequest,
    TwoFactorCodeRequest, TwoFactorEnabledResponse, TwoFactorSetupRequest, TwoFactorSetupResponse,
    TwoFactorStatus, User, VoiceTokenResponse, WatchRequest,
};
use serde::de::DeserializeOwned;

/// What a password login led to. Not `Debug`: both variants hold a credential.
pub enum LoginOutcome {
    Session(SessionResponse),
    /// The account has two-factor on: the ticket goes back with the code (`Api::login_mfa`).
    MfaRequired {
        ticket: String,
    },
}

/// What a second-step login led to. Not `Debug`: the session holds the token.
pub enum MfaOutcome {
    Session(SessionResponse),
    /// The ticket is gone (expired, spent, or too many wrong codes): start again with the password.
    Expired,
}

/// What the login screen and Settings say for a wrong code (401 from `/auth/login/2fa`, 403
/// "wrong code" from the signed-in `/auth/2fa/*` calls).
const WRONG_CODE: &str = "That code didn't work. Check the time on your phone and try again.";

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
/// Returns the normalised address to use from here on.
pub fn check_server_url(url: &str) -> Result<String, ApiError> {
    use reqwest::Url;
    use url::Host;
    let reject = |m: &str| Err(ApiError::Rejected(m.into()));
    let Ok(u) = Url::parse(url) else {
        return reject("not a valid server address");
    };
    // Judge the URL the way the HTTP client reads it: the host it connects to is `u.host()`
    // (userinfo ends at the last '@'), so anything that could disguise it is refused.
    if !u.username().is_empty() || u.password().is_some() {
        return reject("the server address can't contain a user name or password");
    }
    if u.query().is_some() || u.fragment().is_some() {
        return reject("the server address can't contain a ? or # part");
    }
    let allowed = match (u.scheme(), u.host()) {
        ("https", Some(_)) => true,
        ("http", Some(Host::Domain("localhost"))) => true,
        ("http", Some(Host::Ipv4(ip))) => ip.is_loopback(),
        ("http", Some(Host::Ipv6(ip))) => ip.is_loopback(),
        ("http", Some(_)) => false,
        _ => return reject("server address must start with https://"),
    };
    if !allowed {
        return reject("use https:// for remote servers");
    }
    // The address as checked (and as the client will read it), without a trailing '/'.
    Ok(u.as_str().trim_end_matches('/').to_string())
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

/// The server's terse "wrong code" (403) in the login screen's words; still `Forbidden`, which
/// a signed-in caller must get (never `Unauthorized`). Other refusals keep the server's text.
fn friendly_wrong_code<T>(r: Result<T, ApiError>) -> Result<T, ApiError> {
    match r {
        Err(ApiError::Forbidden(m)) if m == "wrong code" => {
            Err(ApiError::Forbidden(WRONG_CODE.into()))
        }
        other => other,
    }
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

    /// Upload a preview frame, labelled with the type its bytes are (the server checks they agree).
    pub async fn put_preview(
        &self,
        token: &str,
        channel: ChannelId,
        image: Vec<u8>,
    ) -> Result<(), ApiError> {
        let Some(content_type) = preview_type(&image) else {
            return Err(ApiError::Rejected("preview isn't WebP, PNG or JPEG".into()));
        };
        Self::ok_empty(
            self.http
                .put(format!("{}/streams/{channel}/preview", self.base))
                .bearer_auth(token)
                .header(reqwest::header::CONTENT_TYPE, content_type)
                .body(image)
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

    pub async fn login(&self, username: &str, password: &str) -> Result<LoginOutcome, ApiError> {
        let body = LoginRequest {
            username: username.into(),
            password: password.into(),
        };
        let r = self
            .http
            .post(format!("{}/auth/login", self.base))
            .json(&body)
            .send()
            .await;
        // A 403 here is either "second step needed" (its body carries the ticket) or an
        // ordinary refusal, and `checked` would flatten both to a message: look first.
        let r = match r {
            Ok(resp) if resp.status() == reqwest::StatusCode::FORBIDDEN => {
                note_version(&resp);
                let path = path_only(resp.url());
                let bytes = resp.bytes().await.map_err(|e| {
                    let text = failure_text(e);
                    log_failure(Some(path.clone()), &text);
                    ApiError::Network(text)
                })?;
                if let Ok(m) = serde_json::from_slice::<MfaRequired>(&bytes)
                    && m.code == "mfa_required"
                {
                    return Ok(LoginOutcome::MfaRequired { ticket: m.ticket });
                }
                log_failure(Some(path), "403");
                let msg = serde_json::from_slice::<pulse_protocol::rest::ApiError>(&bytes)
                    .map(|e| e.message)
                    .unwrap_or_else(|_| "unexpected server error".into());
                return Err(ApiError::Forbidden(msg));
            }
            other => other,
        };
        Self::parse(r).await.map(LoginOutcome::Session)
    }

    /// Second login step. 410 = the ticket is dead; 401 = the code was wrong (the ticket lives on).
    pub async fn login_mfa(&self, ticket: &str, code: &str) -> Result<MfaOutcome, ApiError> {
        let body = LoginMfaRequest {
            ticket: ticket.into(),
            code: code.into(),
        };
        let r = self
            .http
            .post(format!("{}/auth/login/2fa", self.base))
            .json(&body)
            .send()
            .await;
        if let Ok(resp) = &r
            && resp.status() == reqwest::StatusCode::GONE
        {
            note_version(resp);
            log_failure(Some(path_only(resp.url())), "410");
            return Ok(MfaOutcome::Expired);
        }
        match Self::parse(r).await {
            Err(ApiError::Unauthorized) => Err(ApiError::Rejected(WRONG_CODE.into())),
            other => other.map(MfaOutcome::Session),
        }
    }

    pub async fn two_factor_status(&self, token: &str) -> Result<TwoFactorStatus, ApiError> {
        Self::parse(
            self.http
                .get(format!("{}/auth/2fa", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// Starts (or restarts) setup; the account password is the proof it's really you.
    pub async fn two_factor_setup(
        &self,
        token: &str,
        password: &str,
    ) -> Result<TwoFactorSetupResponse, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/auth/2fa/setup", self.base))
                .bearer_auth(token)
                .json(&TwoFactorSetupRequest {
                    password: password.into(),
                })
                .send()
                .await,
        )
        .await
    }

    pub async fn two_factor_enable(
        &self,
        token: &str,
        code: &str,
    ) -> Result<TwoFactorEnabledResponse, ApiError> {
        friendly_wrong_code(Self::parse(self.code_post(token, "enable", code).await).await)
    }

    /// A TOTP or a recovery code turns it off.
    pub async fn two_factor_disable(&self, token: &str, code: &str) -> Result<(), ApiError> {
        friendly_wrong_code(Self::ok_empty(self.code_post(token, "disable", code).await).await)
    }

    pub async fn two_factor_regenerate(
        &self,
        token: &str,
        code: &str,
    ) -> Result<RecoveryCodesResponse, ApiError> {
        friendly_wrong_code(Self::parse(self.code_post(token, "recovery-codes", code).await).await)
    }

    async fn code_post(
        &self,
        token: &str,
        action: &str,
        code: &str,
    ) -> Result<reqwest::Response, reqwest::Error> {
        self.http
            .post(format!("{}/auth/2fa/{action}", self.base))
            .bearer_auth(token)
            .json(&TwoFactorCodeRequest { code: code.into() })
            .send()
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

/// The `Content-Type` of a preview frame, from its leading bytes: what a webview's canvas gives.
fn preview_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else {
        None
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
        let LoginOutcome::Session(other) = api.login("alex", "hunter2hunter2").await.unwrap()
        else {
            panic!("no 2FA here");
        };
        let other = other.token;
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

        let LoginOutcome::Session(login) = api.login("alex", "hunter2hunter2").await.unwrap()
        else {
            panic!("no 2FA here");
        };
        assert_eq!(login.user.id, reg.user.id);
        assert_eq!(api.me(&login.token).await.unwrap().username, "alex");
    }

    /// The code a phone would show for `secret` at `offset` 30 s steps from now. The server
    /// remembers the last step it accepted, so a test that uses several codes uses several steps.
    fn code_at(secret_b32: &str, offset: i64) -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        use totp_rs::{Algorithm, Builder, Secret};
        let t = Builder::new()
            .with_algorithm(Algorithm::SHA1)
            .with_digits(6)
            .with_skew(0)
            .with_step_duration(30)
            .with_secret(
                Secret::try_from_base32(secret_b32)
                    .unwrap()
                    .as_bytes()
                    .to_vec(),
            )
            .build_noncompliant();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        t.generate((now as i64 + offset * 30) as u64).to_string()
    }

    /// The current 30 s TOTP step by this machine's clock (the server's clock in these tests).
    fn now_step() -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            / 30) as i64
    }

    /// A 6-digit code that cannot be valid when the server checks it. Every 6-digit string is the
    /// code for *some* step, so "wrong" only means something at a given time. This one differs from
    /// the codes for steps t-2..=t+2 (t = the step when it is made), and dropping it asserts the
    /// clock moved at most one step meanwhile. The server checks {s-1, s, s+1} for its own step s,
    /// read between the making and the drop, so s is t or t+1 and its whole window lies inside the
    /// five excluded steps: it can never collide, and a call slow enough to break that bound fails
    /// loudly here instead of flaking. Make one just before use rather than holding it for long.
    struct WrongCode {
        code: String,
        step: i64,
    }

    impl std::ops::Deref for WrongCode {
        type Target = str;
        fn deref(&self) -> &str {
            &self.code
        }
    }

    impl Drop for WrongCode {
        fn drop(&mut self) {
            if !std::thread::panicking() {
                let moved = now_step() - self.step;
                assert!(
                    (0..=1).contains(&moved),
                    "a wrong code outlived its window ({moved} steps); it may have been valid"
                );
            }
        }
    }

    fn wrong_code(secret_b32: &str) -> WrongCode {
        let step = now_step();
        let near: Vec<String> = (-2..=2).map(|o| code_at(secret_b32, o)).collect();
        let code = (0..)
            .map(|n| format!("{n:06}"))
            .find(|c| !near.contains(c))
            .unwrap();
        WrongCode { code, step }
    }

    /// A registered user with 2FA on (enabled with the current step's code, so the next code
    /// to use is offset +1); returns the secret, the signed-in token and the recovery codes.
    async fn two_factor_user(app: &testing::TestApp, api: &Api) -> (String, String, Vec<String>) {
        let (_, token) = testing::register(app, "alex").await;
        let setup = api
            .two_factor_setup(&token, "hunter2hunter2")
            .await
            .unwrap();
        let on = api
            .two_factor_enable(&token, &code_at(&setup.secret, 0))
            .await
            .unwrap();
        (setup.secret, token, on.recovery_codes)
    }

    #[tokio::test]
    async fn login_for_a_two_factor_user_asks_for_a_code() {
        let app = testing::spawn().await;
        let api = Api::new(&format!("http://{}", app.addr));
        two_factor_user(&app, &api).await;
        let out = api.login("alex", "hunter2hunter2").await.unwrap();
        let LoginOutcome::MfaRequired { ticket } = out else {
            panic!("expected a ticket");
        };
        assert!(!ticket.is_empty());
    }

    #[tokio::test]
    async fn login_for_a_user_without_two_factor_still_gives_a_session() {
        let app = testing::spawn().await;
        testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let out = api.login("alex", "hunter2hunter2").await.unwrap();
        assert!(matches!(out, LoginOutcome::Session(s) if s.user.username == "alex"));
    }

    #[tokio::test]
    async fn a_403_that_is_not_mfa_required_stays_forbidden() {
        // a login 403 whose body is a plain api error (not the mfa_required one)
        let base = canned_server(
            "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: 49\r\nConnection: close\r\n\r\n{\"code\":\"forbidden\",\"message\":\"not allowed here\"}",
        )
        .await;
        let err = Api::new(&base).login("alex", "x").await.err().unwrap();
        assert!(
            matches!(&err, ApiError::Forbidden(m) if m == "not allowed here"),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn login_mfa_with_the_right_code_gives_a_session_then_the_ticket_is_spent() {
        let app = testing::spawn().await;
        let api = Api::new(&format!("http://{}", app.addr));
        let (secret, _, _) = two_factor_user(&app, &api).await;
        let LoginOutcome::MfaRequired { ticket } =
            api.login("alex", "hunter2hunter2").await.unwrap()
        else {
            panic!("expected a ticket");
        };
        let out = api.login_mfa(&ticket, &code_at(&secret, 1)).await.unwrap();
        assert!(matches!(out, MfaOutcome::Session(s) if s.user.username == "alex"));
        // The ticket is checked before the code, so any code shows it is spent.
        let again = api.login_mfa(&ticket, &code_at(&secret, 1)).await.unwrap();
        assert!(matches!(again, MfaOutcome::Expired));
    }

    #[tokio::test]
    async fn login_mfa_with_a_wrong_code_says_so_and_keeps_the_ticket() {
        let app = testing::spawn().await;
        let api = Api::new(&format!("http://{}", app.addr));
        let (secret, _, _) = two_factor_user(&app, &api).await;
        let LoginOutcome::MfaRequired { ticket } =
            api.login("alex", "hunter2hunter2").await.unwrap()
        else {
            panic!("expected a ticket");
        };
        let err = api
            .login_mfa(&ticket, &wrong_code(&secret))
            .await
            .err()
            .unwrap();
        assert!(
            matches!(&err, ApiError::Rejected(m) if m == "That code didn't work. Check the time on your phone and try again."),
            "{err:?}"
        );
        let out = api.login_mfa(&ticket, &code_at(&secret, 1)).await.unwrap();
        assert!(matches!(out, MfaOutcome::Session(_)));
    }

    #[tokio::test]
    async fn two_factor_setup_enable_regenerate_disable_round_trip() {
        let app = testing::spawn().await;
        let api = Api::new(&format!("http://{}", app.addr));
        let (_, token) = testing::register(&app, "alex").await;
        let st = api.two_factor_status(&token).await.unwrap();
        assert!((st.available, st.enabled) == (true, false));

        let setup = api
            .two_factor_setup(&token, "hunter2hunter2")
            .await
            .unwrap();
        assert!(setup.otpauth_uri.starts_with("otpauth://totp/Pulse:alex?"));
        let on = api
            .two_factor_enable(&token, &code_at(&setup.secret, 0))
            .await
            .unwrap();
        assert_eq!(on.recovery_codes.len(), 10);
        assert!(api.two_factor_status(&token).await.unwrap().enabled);

        let fresh = api
            .two_factor_regenerate(&token, &code_at(&setup.secret, 1))
            .await
            .unwrap();
        assert_eq!(fresh.recovery_codes.len(), 10);
        assert_ne!(fresh.recovery_codes, on.recovery_codes);

        api.two_factor_disable(&token, &fresh.recovery_codes[0])
            .await
            .unwrap();
        assert!(!api.two_factor_status(&token).await.unwrap().enabled);
    }

    #[tokio::test]
    async fn a_wrong_password_or_code_while_signed_in_is_forbidden_not_unauthorized() {
        let app = testing::spawn().await;
        let api = Api::new(&format!("http://{}", app.addr));
        let (_, token) = testing::register(&app, "alex").await;
        let err = api
            .two_factor_setup(&token, "nopenopenope")
            .await
            .err()
            .unwrap();
        assert!(matches!(err, ApiError::Forbidden(_)), "{err:?}");
        let setup = api
            .two_factor_setup(&token, "hunter2hunter2")
            .await
            .unwrap();
        // A fresh wrong code at each use (see `WrongCode`). Worded like the login screen's wrong-code message, still Forbidden.
        let err = api
            .two_factor_enable(&token, &wrong_code(&setup.secret))
            .await
            .err()
            .unwrap();
        assert!(
            matches!(&err, ApiError::Forbidden(m) if m == WRONG_CODE),
            "{err:?}"
        );
        api.two_factor_enable(&token, &code_at(&setup.secret, 0))
            .await
            .unwrap();
        let err = api
            .two_factor_regenerate(&token, &wrong_code(&setup.secret))
            .await
            .err()
            .unwrap();
        assert!(
            matches!(&err, ApiError::Forbidden(m) if m == WRONG_CODE),
            "{err:?}"
        );
        let err = api
            .two_factor_disable(&token, &wrong_code(&setup.secret))
            .await
            .err()
            .unwrap();
        assert!(
            matches!(&err, ApiError::Forbidden(m) if m == WRONG_CODE),
            "{err:?}"
        );
        // Other refusals keep the server's words.
        let err = api
            .two_factor_setup(&token, "hunter2hunter2")
            .await
            .err()
            .unwrap();
        assert!(
            matches!(&err, ApiError::Rejected(m) if m.contains("already on")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn early_login_answers_note_the_http_version_too() {
        let (forbidden, mfa, gone) = (
            canned_server(
                "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: 49\r\nConnection: close\r\n\r\n{\"code\":\"forbidden\",\"message\":\"not allowed here\"}",
            )
            .await,
            canned_server(
                "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: 55\r\nConnection: close\r\n\r\n{\"code\":\"mfa_required\",\"message\":\"code\",\"ticket\":\"tkt\"}",
            )
            .await,
            canned_server(
                "HTTP/1.1 410 Gone\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await,
        );
        let before = noted();
        let _ = Api::new(&forbidden).login("alex", "x").await;
        assert_eq!(noted(), before + 1);
        let out = Api::new(&mfa).login("alex", "x").await.unwrap();
        assert!(matches!(out, LoginOutcome::MfaRequired { ticket } if ticket == "tkt"));
        assert_eq!(noted(), before + 2);
        let out = Api::new(&gone).login_mfa("tkt", "123456").await.unwrap();
        assert!(matches!(out, MfaOutcome::Expired));
        assert_eq!(noted(), before + 3);
    }

    /// Answers every request with `reply` verbatim, then closes.
    async fn canned_server(reply: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("localhost:0").await.unwrap();
        let base = format!("http://localhost:{}", l.local_addr().unwrap().port());
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                tokio::spawn(async move {
                    let mut chunk = [0u8; 4096];
                    let _ = s.read(&mut chunk).await;
                    let _ = s.write_all(reply.as_bytes()).await;
                });
            }
        });
        base
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
        let err = api.login("alex", "nopenopenope").await.err().unwrap();
        assert!(matches!(err, ApiError::RateLimited), "{err:?}");
        assert_eq!(err.to_string(), "slow down a little, then try again");
        assert!(
            crate::outbox::retryable(&err),
            "a rate-limited send is retried later"
        );
    }

    /// The server's shipped login limits, which the slow-down message above is written for.
    #[test]
    fn default_login_limits_are_five_a_minute_per_ip_and_ten_an_hour_per_user_and_address() {
        use pulse_server::limits::{LimitsConfig, Rate};
        let d = LimitsConfig::default();
        assert_eq!(d.login_ip, Rate::per_minute(5));
        assert_eq!(d.login_user, Rate::per_hour(100));
        assert_eq!(d.login_user_ip, Rate::per_hour(10));
    }

    #[tokio::test]
    async fn wrong_password_maps_to_unauthorized() {
        let app = testing::spawn().await;
        testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let err = api.login("alex", "nopenopenope").await.err().unwrap();
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
    async fn a_png_preview_goes_out_as_png_and_comes_back_as_webp() {
        // A webview that can't encode WebP gives PNG (or JPEG); it is labelled as what it is.
        let app = testing::spawn().await;
        let (api, token, uid, ch) = voice_fixture(&app, "alex").await;
        app.voice.join(ch, uid, "PA_test");
        api.go_live(&token, ch, settings()).await.unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(48, 27, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        api.put_preview(&token, ch, png.into_inner()).await.unwrap();
        let got = api.get_preview(&token, ch, uid).await.unwrap().unwrap();
        assert_eq!((&got[..4], &got[8..12]), (&b"RIFF"[..], &b"WEBP"[..]));
    }

    #[test]
    fn preview_types_come_from_the_bytes() {
        assert_eq!(preview_type(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(preview_type(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(preview_type(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(preview_type(b"GIF89a"), None);
        assert_eq!(preview_type(b""), None);
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

    #[test]
    fn accepted_url_comes_back_normalised() {
        assert_eq!(
            check_server_url("https://Example.COM/").unwrap(),
            "https://example.com"
        );
        assert_eq!(
            check_server_url("http://localhost:7890").unwrap(),
            "http://localhost:7890"
        );
        assert_eq!(
            check_server_url("https://example.com:8443/base/").unwrap(),
            "https://example.com:8443/base"
        );
    }

    #[test]
    fn server_url_check_parses_like_the_http_client() {
        // The url crate ends userinfo at the last '@': these connect to evil.example.com.
        for bad in [
            "http://localhost:1@evil.example.com",
            "http://[::1]@evil.example.com",
            "http://127.0.0.1:x@evil.example.com/",
            "https://user:pw@example.com",
            "https://example.com/?a=b",
            "https://example.com/#x",
            "http://localhost:7890/?a=b",
            "http://localhost.evil.example.com",
            "https://",
            "",
        ] {
            assert!(
                matches!(check_server_url(bad), Err(ApiError::Rejected(_))),
                "{bad} must be rejected"
            );
        }
        for ok in [
            "http://localhost:7890",
            "http://127.0.0.1:7890",
            "http://127.5.6.7:7890",
            "http://[::1]:7890",
            "https://example.com",
            "https://example.com/",
            "https://example.com:8443/base",
        ] {
            assert!(check_server_url(ok).is_ok(), "{ok} must be accepted");
        }
    }

    #[tokio::test]
    async fn timeout_on_unroutable_host() {
        let api = Api::new("http://10.255.255.1:9");
        let started = std::time::Instant::now();
        let err = api.login("a", "b").await.err().unwrap();
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
