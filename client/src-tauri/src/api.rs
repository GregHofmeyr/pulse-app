//! HTTP client for the Pulse server. Only the Rust core talks to the network.

use std::time::Duration;

use pulse_protocol::ids::{
    CategoryId, ChannelId, ClipUploadId, GameId, LineId, MessageId, QotyEventId, ServerId, UserId,
};
use pulse_protocol::rest::{
    AddGameRequest, AddMembersRequest, AddQotyOptionRequest, BadgeCatalogue, Category,
    ChangePasswordRequest, Channel, ChannelKind, ClipFacets, ClipStorage, ClipUpload,
    ClipUploadRequest, ClipUrl, ClipView, ConfirmClipRequest, CreateCategoryRequest,
    CreateDmRequest, CreateEmojiRequest, CreateSoundRequest, CreateWheelRequest, CustomEmoji,
    DeleteServerRequest, DeletedChannel, DeletedServer, EditClipRequest, EditMessageRequest,
    EditQuoteRequest, EggRequest, Game, Gif, GifConfig, GoLiveRequest, HideEmbedRequest,
    Leaderboard, LinkSpeakerRequest, LoginMfaRequest, LoginRequest, MarkBadgesSeenRequest,
    MarkReadRequest, MediaInfo, Member, Message, MfaRequired, MuteTarget, NameArchivePage, Pin,
    PinDuration, PinRequest, PinnedMessage, PostQuoteRequest, Profile, QotyEvent, QotyVoteRequest,
    QuietSpin, RecoveryCodesResponse, RegisterRequest, RenameCategoryRequest, RenameChannelRequest,
    RenameEmojiRequest, ReopenQotyRequest, SaveLayoutRequest, SearchRequest, SearchResults,
    SendMessageRequest, Server, ServerLayout, ServerVoiceMod, SessionResponse,
    SetChannelBitrateRequest, SetClipViewRequest, SetJoinSoundRequest, SetMuteRequest,
    SetNicknameRequest, SetNotifyLevelRequest, SetServerVoiceRequest, SetShowcaseRequest,
    SetStatusRequest, Sound, SpinAgainRequest, SpinWheelRequest, StageShare, StageShareRequest,
    StartQotyRequest, Status, StreamInfo, StreamRole, StreamSettings, StreamTicketRequest,
    ThreadSeenRequest, TwoFactorCodeRequest, TwoFactorEnabledResponse, TwoFactorSetupRequest,
    TwoFactorSetupResponse, TwoFactorStatus, UpdateProfileRequest, UpdateSoundRequest,
    UpdateWheelRequest, User, VoiceTokenResponse, WatchRequest, Wheel,
};
use serde::de::DeserializeOwned;

/// Where a message search looks: one whole server, or one DM or group on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchScope {
    Server(ServerId),
    Conversation(ChannelId),
}

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
    /// A clip call refused with a 429 or 503 in Pulse's own words (`message`, shown as-is):
    /// "That's 20 clips this hour. Try again later." / "Clips storage isn't set up on this
    /// server." Nothing was done, so asking again is safe (`never_arrived` is true for a 503).
    #[error("{message}")]
    Told { status: u16, message: String },
    /// Confirming a clip found its ticket or its upload gone (404, or a 409 such as "That upload
    /// expired. Try again."), in Pulse's words: a new ticket and upload can still post it.
    #[error("{0}")]
    UploadGone(String),
    /// Confirming a clip found its poster missing (a 409 `thumb_missing`): upload the poster
    /// again, the video is fine.
    #[error("{0}")]
    PosterGone(String),
}

impl ApiError {
    /// The request (almost) certainly never reached Pulse: no connection, or the proxy found it
    /// down (502/503; a proxy can also say 502 after a reset mid-response, so a duplicate is
    /// possible but rare). Safe enough to send again.
    pub fn never_arrived(&self) -> bool {
        match self {
            Self::Network(t) => t == CONNECT_FAILED,
            Self::Server(s) => s.starts_with("502") || s.starts_with("503"),
            // a clip call the server refused up front (storage not set up / unavailable)
            Self::Told { status, .. } => *status == 503,
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

impl ApiError {
    /// A stable name for the errors the UI acts on rather than shows, so it needn't match the
    /// words (#123). `"rate_limited"`: a 429, the server's own `rate_limited` code.
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Self::RateLimited | Self::Told { status: 429, .. } => Some("rate_limited"),
            _ => None,
        }
    }
}

/// An [`ApiError`] for a command whose UI branches on the kind of failure: serialised as
/// `{ "message": <the same words a plain ApiError serialises to>, "code": <ApiError::code> | null }`.
/// Plain `ApiError` stays a string, which every other command's error handling expects.
#[derive(Debug)]
pub struct Coded(pub ApiError);

impl From<ApiError> for Coded {
    fn from(e: ApiError) -> Self {
        Self(e)
    }
}

impl serde::Serialize for Coded {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("Coded", 2)?;
        st.serialize_field("message", &self.0.to_string())?;
        st.serialize_field("code", &self.0.code())?;
        st.end()
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
/// What an avatar or banner upload over the server's body limit says (its plain-text 413 isn't
/// our JSON error, so the core supplies the words).
pub const PROFILE_TOO_BIG: &str = "Profile pictures and banners can be up to 8 MB.";

/// Which profile picture an upload is for: the media route's last segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileMediaKind {
    Avatar,
    Banner,
}

impl ProfileMediaKind {
    fn segment(self) -> &'static str {
        match self {
            Self::Avatar => "avatar",
            Self::Banner => "banner",
        }
    }
}

/// The part of the picture to keep, in source pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// The connection was never made: a request with this error certainly didn't arrive.
pub const CONNECT_FAILED: &str = "couldn't connect";

/// A reqwest error in plain words, without the URL (its Display would include host + query).
pub(crate) fn failure_text(e: reqwest::Error) -> String {
    // connect first: a connect timeout is both, and it certainly never arrived
    if e.is_connect() {
        CONNECT_FAILED.into()
    } else if e.is_timeout() {
        "it took too long to answer".into()
    } else {
        e.without_url().to_string()
    }
}

/// What kind of JSON problem a decode error was, and where: never any of the body.
fn json_problem(e: &reqwest::Error) -> String {
    use std::error::Error;
    let mut src = e.source();
    while let Some(s) = src {
        if let Some(j) = s.downcast_ref::<serde_json::Error>() {
            let kind = match j.classify() {
                serde_json::error::Category::Io => "io",
                serde_json::error::Category::Syntax => "syntax",
                serde_json::error::Category::Data => "data",
                serde_json::error::Category::Eof => "eof",
            };
            return format!("{kind} at {}:{}", j.line(), j.column());
        }
        src = s.source();
    }
    "-".into()
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
        let resp = Self::checked(r).await?;
        Self::read_json(resp).await
    }

    /// The body of a response that already passed a status check.
    async fn read_json<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T, ApiError> {
        let (path, status) = (path_only(resp.url()), resp.status());
        resp.json().await.map_err(|e| {
            // a 2xx we couldn't read: checked() logged nothing, yet the caller sees a failure
            let json = json_problem(&e);
            let text = failure_text(e);
            tracing::warn!(
                path,
                status = status.as_u16(),
                json,
                error = text,
                "request failed"
            );
            ApiError::Network(text)
        })
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
        self.create_channel_in(token, server, kind, name, None)
            .await
    }

    /// A new channel at the bottom of `category` (#54), or uncategorised (`None`).
    pub async fn create_channel_in(
        &self,
        token: &str,
        server: ServerId,
        kind: ChannelKind,
        name: &str,
        category: Option<CategoryId>,
    ) -> Result<Channel, ApiError> {
        let body = pulse_protocol::rest::CreateChannelRequest {
            kind,
            name: name.into(),
            category_id: category,
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

    /// A new category at the bottom of `server`'s list (#54).
    pub async fn create_category(
        &self,
        token: &str,
        server: ServerId,
        name: &str,
    ) -> Result<Category, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/servers/{server}/categories", self.base))
                .bearer_auth(token)
                .json(&CreateCategoryRequest { name: name.into() })
                .send()
                .await,
        )
        .await
    }

    pub async fn rename_category(
        &self,
        token: &str,
        category: CategoryId,
        name: &str,
    ) -> Result<Category, ApiError> {
        Self::parse(
            self.http
                .patch(format!("{}/categories/{category}", self.base))
                .bearer_auth(token)
                .json(&RenameCategoryRequest { name: name.into() })
                .send()
                .await,
        )
        .await
    }

    /// Deletes a category; its channels move to the uncategorised top of the list.
    pub async fn delete_category(&self, token: &str, category: CategoryId) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .delete(format!("{}/categories/{category}", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// Saves a whole new order of `server`'s list (#55). A save based on an old revision (someone
    /// else's landed first) is a 409, which arrives as [`ApiError::Rejected`] in the server's
    /// words, for the UI to show as its draft snaps back.
    pub async fn save_layout(
        &self,
        token: &str,
        server: ServerId,
        layout: &SaveLayoutRequest,
    ) -> Result<ServerLayout, ApiError> {
        Self::parse(
            self.http
                .put(format!("{}/servers/{server}/layout", self.base))
                .bearer_auth(token)
                .json(layout)
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

    /// A voice channel's bitrate in kbps (#75), or `None` for Default.
    pub async fn set_channel_bitrate(
        &self,
        token: &str,
        ch: ChannelId,
        bitrate: Option<u32>,
    ) -> Result<Channel, ApiError> {
        Self::parse(
            self.http
                .put(format!("{}/channels/{ch}/bitrate", self.base))
                .bearer_auth(token)
                .json(&SetChannelBitrateRequest { bitrate })
                .send()
                .await,
        )
        .await
    }

    /// Set (an avatar-upload hash) or clear a group's icon (#56).
    pub async fn set_group_icon(
        &self,
        token: &str,
        ch: ChannelId,
        hash: Option<String>,
    ) -> Result<Channel, ApiError> {
        Self::parse(
            self.http
                .put(format!("{}/channels/{ch}/icon", self.base))
                .bearer_auth(token)
                .json(&pulse_protocol::rest::SetGroupIconRequest { hash })
                .send()
                .await,
        )
        .await
    }

    /// Moves a server channel to its server's Recently deleted.
    pub async fn delete_channel(&self, token: &str, ch: ChannelId) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .delete(format!("{}/channels/{ch}", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn list_deleted_channels(
        &self,
        token: &str,
        server: ServerId,
    ) -> Result<Vec<DeletedChannel>, ApiError> {
        Self::parse(
            self.http
                .get(format!("{}/servers/{server}/deleted-channels", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn restore_channel(&self, token: &str, ch: ChannelId) -> Result<Channel, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/channels/{ch}/restore", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// Moves a server to its owner's Recently deleted (#184): `name` is its name, typed to confirm.
    pub async fn delete_server(
        &self,
        token: &str,
        server: ServerId,
        name: &str,
    ) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .delete(format!("{}/servers/{server}", self.base))
                .bearer_auth(token)
                .json(&DeleteServerRequest { name: name.into() })
                .send()
                .await,
        )
        .await
    }

    /// Your own servers in Recently deleted.
    pub async fn list_deleted_servers(&self, token: &str) -> Result<Vec<DeletedServer>, ApiError> {
        Self::parse(
            self.http
                .get(format!("{}/servers/deleted", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn restore_server(&self, token: &str, server: ServerId) -> Result<Server, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/servers/{server}/restore", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// A channel in Recently deleted, for good (the server owner's "Delete now").
    pub async fn purge_channel(&self, token: &str, ch: ChannelId) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .delete(format!("{}/channels/{ch}/permanent", self.base))
                .bearer_auth(token)
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

    /// The channel menu's "Mark as read" (#111): the channel up to its newest message and every
    /// quote or clip thread in it up to its newest reply.
    pub async fn mark_channel_read(&self, token: &str, ch: ChannelId) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .post(format!("{}/channels/{ch}/read-all", self.base))
                .bearer_auth(token)
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

    /// Your notification level for a server or one of its channels (#195); `level: None` = the default.
    pub async fn set_notify_level(
        &self,
        token: &str,
        req: &SetNotifyLevelRequest,
    ) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .put(format!("{}/notify-levels", self.base))
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

    /// Removes the preview for `url` from your own message, or all of them (`None`), for everyone.
    pub async fn hide_embed(
        &self,
        token: &str,
        message_id: MessageId,
        url: Option<&str>,
    ) -> Result<Message, ApiError> {
        let body = HideEmbedRequest {
            url: url.map(Into::into),
        };
        Self::parse(
            self.http
                .post(format!("{}/messages/{message_id}/embeds/hide", self.base))
                .bearer_auth(token)
                .json(&body)
                .send()
                .await,
        )
        .await
    }

    /// Pin a message for `duration` (#178).
    pub async fn pin_message(
        &self,
        token: &str,
        id: MessageId,
        duration: PinDuration,
    ) -> Result<Pin, ApiError> {
        Self::parse(
            self.http
                .put(format!("{}/messages/{id}/pin", self.base))
                .bearer_auth(token)
                .json(&PinRequest { duration })
                .send()
                .await,
        )
        .await
    }

    pub async fn unpin_message(&self, token: &str, id: MessageId) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .delete(format!("{}/messages/{id}/pin", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// The key this server's people search GIPHY with (#58), `api_key: None` when GIFs aren't set up there.
    pub async fn gif_config(&self, token: &str) -> Result<GifConfig, ApiError> {
        Self::parse(
            self.http
                .get(format!("{}/gifs/config", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// Your favourite GIFs (#58), newest first.
    pub async fn gif_favourites(&self, token: &str) -> Result<Vec<Gif>, ApiError> {
        Self::parse(
            self.http
                .get(format!("{}/gifs/favourites", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// Star a GIF (#58); starring one already starred changes nothing.
    pub async fn add_gif_favourite(&self, token: &str, gif: &Gif) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .post(format!("{}/gifs/favourites", self.base))
                .bearer_auth(token)
                .json(gif)
                .send()
                .await,
        )
        .await
    }

    /// Unstar a GIF (#58); unstarring one that isn't starred changes nothing.
    pub async fn remove_gif_favourite(&self, token: &str, id: &str) -> Result<(), ApiError> {
        let mut url = reqwest::Url::parse(&format!("{}/gifs/favourites", self.base))
            .map_err(|e| ApiError::Rejected(e.to_string()))?;
        url.path_segments_mut()
            .map_err(|_| ApiError::Rejected("not a valid server address".into()))?
            .push(id);
        Self::ok_empty(self.http.delete(url).bearer_auth(token).send().await).await
    }

    /// Share a Spotify link to a server voice channel's stage (#234); you must be in its call.
    pub async fn share_to_stage(
        &self,
        token: &str,
        channel: ChannelId,
        url: &str,
    ) -> Result<StageShare, ApiError> {
        Self::parse(
            self.http
                .put(format!("{}/channels/{channel}/stage-share", self.base))
                .bearer_auth(token)
                .json(&StageShareRequest { url: url.into() })
                .send()
                .await,
        )
        .await
    }

    /// Stop the stage share `share_id` for everyone (#234).
    pub async fn stop_stage_share(
        &self,
        token: &str,
        channel: ChannelId,
        share_id: &str,
    ) -> Result<(), ApiError> {
        // A server-made id; anything else would be a different path.
        if share_id.is_empty() || !share_id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(ApiError::Rejected("that share isn't up".into()));
        }
        Self::ok_empty(
            self.http
                .delete(format!(
                    "{}/channels/{channel}/stage-share/{share_id}",
                    self.base
                ))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// A chat's pins, newest pin first.
    pub async fn list_pins(
        &self,
        token: &str,
        channel: ChannelId,
    ) -> Result<Vec<PinnedMessage>, ApiError> {
        Self::parse(
            self.http
                .get(format!("{}/channels/{channel}/pins", self.base))
                .bearer_auth(token)
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

    /// Start a call in a DM or group, ringing everyone else, or join the one going on (#50:
    /// `rang` false). The caller then joins the room with [`Self::voice_token`] as usual.
    pub async fn start_call(
        &self,
        token: &str,
        channel: ChannelId,
    ) -> Result<pulse_protocol::rest::StartCallResponse, ApiError> {
        Self::parse(
            self.http
                .post(self.url(&["calls", &channel.to_string()])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// Stop the ringing for a call in `channel` on all your devices; the caller isn't told (#50).
    pub async fn decline_call(&self, token: &str, channel: ChannelId) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .post(self.url(&["calls", &channel.to_string(), "decline"])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// A stream-room ticket; `ghost` for watching from outside the voice channel (an anonymous,
    /// subscribe-only identity).
    pub async fn stream_ticket(
        &self,
        token: &str,
        channel: ChannelId,
        role: StreamRole,
        ghost: bool,
    ) -> Result<VoiceTokenResponse, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/streams/{channel}/ticket", self.base))
                .bearer_auth(token)
                .json(&StreamTicketRequest { role, ghost })
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

    /// `ghost`: counted, never named (from outside the voice channel).
    pub async fn watch_stream(
        &self,
        token: &str,
        channel: ChannelId,
        streamer: UserId,
        ghost: bool,
    ) -> Result<(), ApiError> {
        self.watch_call(token, channel, "watch", streamer, ghost)
            .await
    }

    pub async fn unwatch_stream(
        &self,
        token: &str,
        channel: ChannelId,
        streamer: UserId,
    ) -> Result<(), ApiError> {
        self.watch_call(token, channel, "unwatch", streamer, false)
            .await
    }

    async fn watch_call(
        &self,
        token: &str,
        channel: ChannelId,
        action: &str,
        streamer: UserId,
        ghost: bool,
    ) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .post(format!("{}/streams/{channel}/{action}", self.base))
                .bearer_auth(token)
                .json(&WatchRequest { streamer, ghost })
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

    /// Change this account's password (`code` only when two-factor is on). Every other device
    /// is signed out; this one stays. The passwords go only into the request body.
    pub async fn change_password(
        &self,
        token: &str,
        current_password: &str,
        new_password: &str,
        code: Option<&str>,
    ) -> Result<(), ApiError> {
        friendly_wrong_code(
            Self::ok_empty(
                self.http
                    .post(format!("{}/auth/password", self.base))
                    .bearer_auth(token)
                    .json(&ChangePasswordRequest {
                        current_password: current_password.into(),
                        new_password: new_password.into(),
                        code: code.map(Into::into),
                    })
                    .send()
                    .await,
            )
            .await,
        )
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
        self.upload_media(token, "emoji", bytes, false).await
    }

    /// `kind` is the media route's last segment (`emoji`, `quote`): the raw file as the body.
    async fn upload_media(
        &self,
        token: &str,
        kind: &str,
        bytes: Vec<u8>,
        clip: bool,
    ) -> Result<MediaInfo, ApiError> {
        let url = self.url(&["media", kind])?;
        self.post_media(token, url, bytes, 45, TOO_BIG, clip).await
    }

    /// An 8 MB picture and its resize can take a while: 90 s. `crop` is all four numbers or none.
    pub async fn upload_profile_media(
        &self,
        token: &str,
        kind: ProfileMediaKind,
        bytes: Vec<u8>,
        crop: Option<CropRect>,
    ) -> Result<MediaInfo, ApiError> {
        let mut url = self.url(&["media", kind.segment()])?;
        if let Some(c) = crop {
            url.query_pairs_mut()
                .append_pair("x", &c.x.to_string())
                .append_pair("y", &c.y.to_string())
                .append_pair("w", &c.w.to_string())
                .append_pair("h", &c.h.to_string());
        }
        self.post_media(token, url, bytes, 90, PROFILE_TOO_BIG, false)
            .await
    }

    async fn post_media(
        &self,
        token: &str,
        url: reqwest::Url,
        bytes: Vec<u8>,
        timeout_secs: u64,
        too_big: &str,
        clip: bool,
    ) -> Result<MediaInfo, ApiError> {
        let r = self
            .http
            .post(url)
            .bearer_auth(token)
            .timeout(Duration::from_secs(timeout_secs))
            .body(bytes)
            .send()
            .await;
        // axum's body limit answers in plain text, not our JSON error
        if let Ok(resp) = &r
            && resp.status() == reqwest::StatusCode::PAYLOAD_TOO_LARGE
        {
            note_version(resp);
            log_failure(Some(path_only(resp.url())), resp.status().as_str());
            return Err(ApiError::Rejected(too_big.into()));
        }
        // a clip's poster keeps the server's own 429/503 words, like the other clip calls
        if clip {
            Self::clip_parse(r).await
        } else {
            Self::parse(r).await
        }
    }

    pub async fn profile(&self, token: &str, user: UserId) -> Result<Profile, ApiError> {
        Self::parse(
            self.http
                .get(self.url(&["users", &user.to_string(), "profile"])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn update_profile(
        &self,
        token: &str,
        req: &UpdateProfileRequest,
    ) -> Result<Profile, ApiError> {
        Self::parse(
            self.http
                .put(self.url(&["me", "profile"])?)
                .bearer_auth(token)
                .json(req)
                .send()
                .await,
        )
        .await
    }

    /// `GET /badges`: every badge as you may see it.
    pub async fn badge_catalogue(&self, token: &str) -> Result<BadgeCatalogue, ApiError> {
        Self::parse(
            self.http
                .get(self.url(&["badges"])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// Choose your card's showcase (empty: back to the rarest five).
    pub async fn set_showcase(
        &self,
        token: &str,
        req: &SetShowcaseRequest,
    ) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .put(self.url(&["me", "badges", "showcase"])?)
                .bearer_auth(token)
                .json(req)
                .send()
                .await,
        )
        .await
    }

    /// Your trophy case showed these badges [NEW]: they aren't now (anything since stays [NEW]).
    pub async fn badges_seen(
        &self,
        token: &str,
        req: &MarkBadgesSeenRequest,
    ) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .post(self.url(&["me", "badges", "seen"])?)
                .bearer_auth(token)
                .json(req)
                .send()
                .await,
        )
        .await
    }

    /// The "you're muted" detector fired; the server decides whether it counts.
    pub async fn muted_talk(&self, token: &str) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .post(self.url(&["me", "badges", "muted-talk"])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// You found a hidden egg.
    pub async fn badge_egg(&self, token: &str, req: &EggRequest) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .post(self.url(&["me", "badges", "egg"])?)
                .bearer_auth(token)
                .json(req)
                .send()
                .await,
        )
        .await
    }

    /// Set (or, `None`, reset) someone's nickname in a server.
    pub async fn set_nickname(
        &self,
        token: &str,
        server: ServerId,
        user: UserId,
        nickname: Option<String>,
    ) -> Result<Member, ApiError> {
        let url = self.url(&[
            "servers",
            &server.to_string(),
            "members",
            &user.to_string(),
            "nickname",
        ])?;
        Self::parse(
            self.http
                .put(url)
                .bearer_auth(token)
                .json(&SetNicknameRequest { nickname })
                .send()
                .await,
        )
        .await
    }

    /// Server mute / deafen someone in a server's voice, or undo it (#140); `None` leaves one as is.
    pub async fn set_server_voice(
        &self,
        token: &str,
        server: ServerId,
        user: UserId,
        req: &SetServerVoiceRequest,
    ) -> Result<ServerVoiceMod, ApiError> {
        let url = self.url(&["servers", &server.to_string(), "voice", &user.to_string()])?;
        Self::parse(
            self.http
                .post(url)
                .bearer_auth(token)
                .json(req)
                .send()
                .await,
        )
        .await
    }

    /// Disconnect someone from a server's voice now (#140).
    pub async fn disconnect_voice(
        &self,
        token: &str,
        server: ServerId,
        user: UserId,
    ) -> Result<(), ApiError> {
        let url = self.url(&[
            "servers",
            &server.to_string(),
            "voice",
            &user.to_string(),
            "disconnect",
        ])?;
        Self::ok_empty(self.http.post(url).bearer_auth(token).send().await).await
    }

    /// Set (or, `None`, clear) the name only you see for someone.
    pub async fn set_private_nickname(
        &self,
        token: &str,
        user: UserId,
        nickname: Option<String>,
    ) -> Result<(), ApiError> {
        let url = self.url(&["me", "private-nicknames", &user.to_string()])?;
        Self::ok_empty(
            self.http
                .put(url)
                .bearer_auth(token)
                .json(&SetNicknameRequest { nickname })
                .send()
                .await,
        )
        .await
    }

    /// Set your status (#45): Online, Dnd (optionally until `dnd_until`) or Invisible.
    pub async fn set_status(
        &self,
        token: &str,
        status: Status,
        dnd_until: Option<String>,
    ) -> Result<(), ApiError> {
        let url = self.url(&["me", "status"])?;
        Self::ok_empty(
            self.http
                .put(url)
                .bearer_auth(token)
                .json(&SetStatusRequest { status, dnd_until })
                .send()
                .await,
        )
        .await
    }

    /// Appear offline to `user` (`hidden`), or stop.
    pub async fn set_appear_offline(
        &self,
        token: &str,
        user: UserId,
        hidden: bool,
    ) -> Result<(), ApiError> {
        let url = self.url(&["me", "appear-offline", &user.to_string()])?;
        let req = if hidden {
            self.http.put(url)
        } else {
            self.http.delete(url)
        };
        Self::ok_empty(req.bearer_auth(token).send().await).await
    }

    /// Get a notification when `user` goes live (`on`), or stop (#60).
    pub async fn set_live_notify(
        &self,
        token: &str,
        user: UserId,
        on: bool,
    ) -> Result<(), ApiError> {
        let url = self.url(&["me", "live-notify", &user.to_string()])?;
        let req = if on {
            self.http.put(url)
        } else {
            self.http.delete(url)
        };
        Self::ok_empty(req.bearer_auth(token).send().await).await
    }

    /// A page of someone's Name Archive in a server (`cursor`: the last page's `next_cursor`).
    pub async fn name_archive(
        &self,
        token: &str,
        server: ServerId,
        user: UserId,
        cursor: Option<String>,
    ) -> Result<NameArchivePage, ApiError> {
        let url = self.url(&[
            "servers",
            &server.to_string(),
            "members",
            &user.to_string(),
            "names",
        ])?;
        let mut req = self.http.get(url).bearer_auth(token);
        if let Some(c) = cursor {
            req = req.query(&[("cursor", c)]);
        }
        Self::parse(req.send().await).await
    }

    pub async fn media(&self, token: &str, hash: &str) -> Result<Vec<u8>, ApiError> {
        let r = self.media_request(token, hash)?.send().await;
        let bytes = Self::checked(r)
            .await?
            .bytes()
            .await
            .map_err(|e| ApiError::Network(failure_text(e)))?;
        Ok(bytes.to_vec())
    }

    /// The GET of a stored file, unsent: `media` reads it into memory, Save image (#206) streams it
    /// to disk. Refuses anything but a hash.
    pub fn media_request(
        &self,
        token: &str,
        hash: &str,
    ) -> Result<reqwest::RequestBuilder, ApiError> {
        if !crate::media::is_hash(hash) {
            return Err(ApiError::Rejected("not a media hash".into()));
        }
        Ok(self
            .http
            .get(self.url(&["media", hash])?)
            .bearer_auth(token))
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

    /// The trimmed Ogg Opus file as the body (the server checks it is one).
    pub async fn upload_sound_media(
        &self,
        token: &str,
        bytes: Vec<u8>,
    ) -> Result<MediaInfo, ApiError> {
        self.upload_media(token, "sound", bytes, false).await
    }

    pub async fn create_sound(
        &self,
        token: &str,
        server: ServerId,
        body: &CreateSoundRequest,
    ) -> Result<Sound, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/servers/{server}/sounds", self.base))
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    pub async fn list_sounds(&self, token: &str, server: ServerId) -> Result<Vec<Sound>, ApiError> {
        Self::parse(
            self.http
                .get(format!("{}/servers/{server}/sounds", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn update_sound(
        &self,
        token: &str,
        id: &str,
        update: &UpdateSoundRequest,
    ) -> Result<Sound, ApiError> {
        Self::parse(
            self.http
                .patch(self.url(&["sounds", id])?)
                .bearer_auth(token)
                .json(update)
                .send()
                .await,
        )
        .await
    }

    pub async fn delete_sound(&self, token: &str, id: &str) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .delete(self.url(&["sounds", id])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// Play into the caller's current call (409 when not in one).
    pub async fn play_sound_to_call(&self, token: &str, id: &str) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .post(self.url(&["sounds", id, "play"])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn set_sound_favourite(
        &self,
        token: &str,
        id: &str,
        on: bool,
    ) -> Result<(), ApiError> {
        let url = self.url(&["sounds", id, "favourite"])?;
        let req = if on {
            self.http.put(url)
        } else {
            self.http.delete(url)
        };
        Self::ok_empty(req.bearer_auth(token).send().await).await
    }

    /// Your join sound in `server` (#121): a sound of its soundboard, or `None` for the normal join cue.
    pub async fn set_join_sound(
        &self,
        token: &str,
        server: ServerId,
        sound: Option<&str>,
    ) -> Result<(), ApiError> {
        let url = self.url(&["servers", &server.to_string(), "join-sound"])?;
        let req = match sound {
            Some(id) => self.http.put(url).json(&SetJoinSoundRequest {
                sound_id: id.to_owned(),
            }),
            None => self.http.delete(url),
        };
        Self::ok_empty(req.bearer_auth(token).send().await).await
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

    // ---------- Search ----------

    /// Search one server, or one DM or group, newest first (`req.cursor` pages back).
    pub async fn search(
        &self,
        token: &str,
        scope: SearchScope,
        req: &SearchRequest,
    ) -> Result<SearchResults, ApiError> {
        let url = match scope {
            SearchScope::Server(s) => self.url(&["servers", &s.to_string(), "search"])?,
            SearchScope::Conversation(c) => self.url(&["channels", &c.to_string(), "search"])?,
        };
        let mut q: Vec<(&str, String)> = Vec::new();
        if !req.q.trim().is_empty() {
            q.push(("q", req.q.clone()));
        }
        if let Some(u) = req.from {
            q.push(("from", u.to_string()));
        }
        if let Some(c) = req.in_channel {
            q.push(("in", c.to_string()));
        }
        if !req.has.is_empty() {
            let has: Vec<&str> = req.has.iter().map(|h| h.as_str()).collect();
            q.push(("has", has.join(",")));
        }
        if let Some(u) = req.mentions {
            q.push(("mentions", u.to_string()));
        }
        if let Some(c) = &req.cursor {
            q.push(("cursor", c.clone()));
        }
        Self::parse(self.http.get(url).query(&q).bearer_auth(token).send().await).await
    }

    // ---------- Quotes ----------

    /// A slice of the channel's main list centred on `message`; `None` when the server has no such
    /// message there (deleted, a reply, another channel's), so the UI can tell that from a failure.
    pub async fn messages_around(
        &self,
        token: &str,
        channel: ChannelId,
        message: MessageId,
    ) -> Result<Option<Vec<Message>>, ApiError> {
        let r = self
            .http
            .get(self.url(&["channels", &channel.to_string(), "messages"])?)
            .query(&[("around", message.to_string())])
            .bearer_auth(token)
            .send()
            .await;
        if matches!(&r, Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND) {
            return Ok(None);
        }
        Self::parse(r).await.map(Some)
    }

    /// The messages just newer than `message`, newest first (like every list).
    pub async fn messages_after(
        &self,
        token: &str,
        channel: ChannelId,
        message: MessageId,
    ) -> Result<Vec<Message>, ApiError> {
        self.messages_near(token, channel, "after", message).await
    }

    async fn messages_near(
        &self,
        token: &str,
        channel: ChannelId,
        param: &str,
        message: MessageId,
    ) -> Result<Vec<Message>, ApiError> {
        Self::parse(
            self.http
                .get(self.url(&["channels", &channel.to_string(), "messages"])?)
                .query(&[(param, message.to_string())])
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn upload_quote_media(
        &self,
        token: &str,
        bytes: Vec<u8>,
    ) -> Result<MediaInfo, ApiError> {
        self.upload_media(token, "quote", bytes, false).await
    }

    pub async fn post_quote(
        &self,
        token: &str,
        channel: ChannelId,
        body: &PostQuoteRequest,
    ) -> Result<Message, ApiError> {
        Self::parse(
            self.http
                .post(self.url(&["channels", &channel.to_string(), "quotes"])?)
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    pub async fn edit_quote(
        &self,
        token: &str,
        message: MessageId,
        body: &EditQuoteRequest,
    ) -> Result<Message, ApiError> {
        Self::parse(
            self.http
                .put(self.url(&["quotes", &message.to_string()])?)
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    /// A quote's replies, newest first; `before` pages back.
    pub async fn replies(
        &self,
        token: &str,
        message: MessageId,
        before: Option<MessageId>,
    ) -> Result<Vec<Message>, ApiError> {
        let mut req = self
            .http
            .get(self.url(&["messages", &message.to_string(), "replies"])?)
            .bearer_auth(token);
        if let Some(b) = before {
            req = req.query(&[("before", b.to_string())]);
        }
        Self::parse(req.send().await).await
    }

    pub async fn mark_replies_seen(
        &self,
        token: &str,
        root: MessageId,
        message: MessageId,
    ) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .post(self.url(&["messages", &root.to_string(), "replies", "seen"])?)
                .bearer_auth(token)
                .json(&ThreadSeenRequest {
                    message_id: message,
                })
                .send()
                .await,
        )
        .await
    }

    pub async fn give_award(&self, token: &str, line: LineId) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .put(self.url(&["quote-lines", &line.to_string(), "award"])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn take_award(&self, token: &str, line: LineId) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .delete(self.url(&["quote-lines", &line.to_string(), "award"])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn link_speaker(
        &self,
        token: &str,
        server: ServerId,
        body: &LinkSpeakerRequest,
    ) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .put(self.url(&["servers", &server.to_string(), "speaker-links"])?)
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    pub async fn unlink_speaker(
        &self,
        token: &str,
        server: ServerId,
        name_key: &str,
    ) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .delete(self.url(&["servers", &server.to_string(), "speaker-links", name_key])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// `year`: `None` = this year, as the server sees it.
    pub async fn leaderboard(
        &self,
        token: &str,
        server: ServerId,
        year: Option<i32>,
    ) -> Result<Leaderboard, ApiError> {
        let mut req = self
            .http
            .get(self.url(&["servers", &server.to_string(), "quotes", "leaderboard"])?)
            .bearer_auth(token);
        if let Some(y) = year {
            req = req.query(&[("year", y)]);
        }
        Self::parse(req.send().await).await
    }

    pub async fn search_quotes(
        &self,
        token: &str,
        server: ServerId,
        q: &str,
    ) -> Result<Vec<Message>, ApiError> {
        Self::parse(
            self.http
                .get(self.url(&["servers", &server.to_string(), "quotes", "search"])?)
                .query(&[("q", q)])
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    // ---------- Clips ----------

    /// Like [`Self::checked`], but a 429 or 503 keeps the server's own words (`ApiError::Told`);
    /// the generic text is only the fallback when the body isn't Pulse's (a proxy's page).
    async fn clip_checked(
        r: Result<reqwest::Response, reqwest::Error>,
    ) -> Result<reqwest::Response, ApiError> {
        use reqwest::StatusCode;
        let resp = match r {
            Ok(resp)
                if matches!(
                    resp.status(),
                    StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE
                ) =>
            {
                note_version(&resp);
                let status = resp.status();
                log_failure(Some(path_only(resp.url())), status.as_str());
                return Err(match resp.json::<pulse_protocol::rest::ApiError>().await {
                    Ok(e) if !e.message.is_empty() => ApiError::Told {
                        status: status.as_u16(),
                        message: e.message,
                    },
                    _ if status == StatusCode::TOO_MANY_REQUESTS => ApiError::RateLimited,
                    _ => ApiError::Server(status.to_string()),
                });
            }
            other => other,
        };
        Self::checked(resp).await
    }

    async fn clip_parse<T: DeserializeOwned>(
        r: Result<reqwest::Response, reqwest::Error>,
    ) -> Result<T, ApiError> {
        Self::read_json(Self::clip_checked(r).await?).await
    }

    /// For clip endpoints that answer 204.
    async fn clip_ok_empty(r: Result<reqwest::Response, reqwest::Error>) -> Result<(), ApiError> {
        Self::clip_checked(r).await.map(|_| ())
    }

    pub async fn add_game(
        &self,
        token: &str,
        server: ServerId,
        name: &str,
    ) -> Result<Game, ApiError> {
        Self::clip_parse(
            self.http
                .post(self.url(&["servers", &server.to_string(), "games"])?)
                .bearer_auth(token)
                .json(&AddGameRequest { name: name.into() })
                .send()
                .await,
        )
        .await
    }

    /// A ticket for one clip: PUT the file to its URL, then [`Self::confirm_clip`].
    pub async fn clip_upload(
        &self,
        token: &str,
        channel: ChannelId,
        body: &ClipUploadRequest,
    ) -> Result<ClipUpload, ApiError> {
        Self::clip_parse(
            self.http
                .post(self.url(&["channels", &channel.to_string(), "clips", "uploads"])?)
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    pub async fn cancel_clip_upload(&self, token: &str, id: ClipUploadId) -> Result<(), ApiError> {
        Self::clip_ok_empty(
            self.http
                .delete(self.url(&["clip-uploads", &id.to_string()])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn confirm_clip(
        &self,
        token: &str,
        id: ClipUploadId,
        body: &ConfirmClipRequest,
    ) -> Result<Message, ApiError> {
        use reqwest::StatusCode;
        let r = self
            .http
            .post(self.url(&["clip-uploads", &id.to_string(), "confirm"])?)
            .bearer_auth(token)
            .json(body)
            .send()
            .await;
        match r {
            Ok(resp) if matches!(resp.status(), StatusCode::NOT_FOUND | StatusCode::CONFLICT) => {
                note_version(&resp);
                log_failure(Some(path_only(resp.url())), resp.status().as_str());
                let (code, message) = resp
                    .json::<pulse_protocol::rest::ApiError>()
                    .await
                    .map(|e| (e.code, e.message))
                    .unwrap_or_default();
                if code == "thumb_missing" {
                    return Err(ApiError::PosterGone(message));
                }
                Err(ApiError::UploadGone(if message.is_empty() {
                    "That upload expired. Try again.".into()
                } else {
                    message
                }))
            }
            r => Self::clip_parse(r).await,
        }
    }

    /// A clip's poster (a JPEG or PNG of at most 640 px); the server makes the WebP.
    pub async fn upload_clip_thumb(
        &self,
        token: &str,
        bytes: Vec<u8>,
    ) -> Result<MediaInfo, ApiError> {
        self.upload_media(token, "clip-thumb", bytes, true).await
    }

    pub async fn edit_clip(
        &self,
        token: &str,
        message: MessageId,
        body: &EditClipRequest,
    ) -> Result<Message, ApiError> {
        Self::clip_parse(
            self.http
                .put(self.url(&["clips", &message.to_string()])?)
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    /// A short-lived link to play the clip.
    pub async fn clip_url(&self, token: &str, message: MessageId) -> Result<ClipUrl, ApiError> {
        Self::clip_parse(
            self.http
                .get(self.url(&["clips", &message.to_string(), "url"])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// Newest first, 30 a page; `before` pages back.
    pub async fn list_clips(
        &self,
        token: &str,
        channel: ChannelId,
        before: Option<MessageId>,
        game: Option<GameId>,
        author: Option<UserId>,
    ) -> Result<Vec<Message>, ApiError> {
        let mut req = self
            .http
            .get(self.url(&["channels", &channel.to_string(), "clips"])?)
            .bearer_auth(token);
        if let Some(b) = before {
            req = req.query(&[("before", b.to_string())]);
        }
        if let Some(g) = game {
            req = req.query(&[("game", g.to_string())]);
        }
        if let Some(a) = author {
            req = req.query(&[("author", a.to_string())]);
        }
        Self::clip_parse(req.send().await).await
    }

    pub async fn clip_facets(
        &self,
        token: &str,
        channel: ChannelId,
    ) -> Result<ClipFacets, ApiError> {
        Self::clip_parse(
            self.http
                .get(self.url(&["channels", &channel.to_string(), "clips", "facets"])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn set_clip_view(
        &self,
        token: &str,
        channel: ChannelId,
        view: ClipView,
    ) -> Result<(), ApiError> {
        Self::clip_ok_empty(
            self.http
                .put(self.url(&["channels", &channel.to_string(), "clip-view"])?)
                .bearer_auth(token)
                .json(&SetClipViewRequest { view })
                .send()
                .await,
        )
        .await
    }

    pub async fn clip_storage(
        &self,
        token: &str,
        server: ServerId,
    ) -> Result<ClipStorage, ApiError> {
        Self::clip_parse(
            self.http
                .get(self.url(&["servers", &server.to_string(), "clips", "storage"])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn start_qoty(
        &self,
        token: &str,
        server: ServerId,
        body: &StartQotyRequest,
    ) -> Result<QotyEvent, ApiError> {
        Self::parse(
            self.http
                .post(self.url(&["servers", &server.to_string(), "qoty"])?)
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    pub async fn vote_qoty(
        &self,
        token: &str,
        event: QotyEventId,
        message: MessageId,
    ) -> Result<QotyEvent, ApiError> {
        Self::parse(
            self.http
                .put(self.url(&["qoty", &event.to_string(), "vote"])?)
                .bearer_auth(token)
                .json(&QotyVoteRequest {
                    message_id: message,
                })
                .send()
                .await,
        )
        .await
    }

    pub async fn unvote_qoty(
        &self,
        token: &str,
        event: QotyEventId,
    ) -> Result<QotyEvent, ApiError> {
        Self::parse(
            self.http
                .delete(self.url(&["qoty", &event.to_string(), "vote"])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn add_qoty_option(
        &self,
        token: &str,
        event: QotyEventId,
        message: MessageId,
    ) -> Result<QotyEvent, ApiError> {
        Self::parse(
            self.http
                .post(self.url(&["qoty", &event.to_string(), "options"])?)
                .bearer_auth(token)
                .json(&AddQotyOptionRequest {
                    message_id: message,
                })
                .send()
                .await,
        )
        .await
    }

    pub async fn end_qoty(&self, token: &str, event: QotyEventId) -> Result<QotyEvent, ApiError> {
        Self::parse(
            self.http
                .post(self.url(&["qoty", &event.to_string(), "end"])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// `minutes`: `None` = open until ended.
    pub async fn reopen_qoty(
        &self,
        token: &str,
        event: QotyEventId,
        minutes: Option<u32>,
    ) -> Result<QotyEvent, ApiError> {
        Self::parse(
            self.http
                .post(self.url(&["qoty", &event.to_string(), "reopen"])?)
                .bearer_auth(token)
                .json(&ReopenQotyRequest { minutes })
                .send()
                .await,
        )
        .await
    }

    /// A server's saved picker wheels (#158).
    pub async fn server_wheels(
        &self,
        token: &str,
        server: ServerId,
    ) -> Result<Vec<Wheel>, ApiError> {
        Self::parse(
            self.http
                .get(format!("{}/servers/{server}/wheels", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// Your own (Just me) picker wheels (#158).
    pub async fn my_wheels(&self, token: &str) -> Result<Vec<Wheel>, ApiError> {
        Self::parse(
            self.http
                .get(format!("{}/wheels/mine", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn create_wheel(
        &self,
        token: &str,
        body: &CreateWheelRequest,
    ) -> Result<Wheel, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/wheels", self.base))
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    pub async fn update_wheel(
        &self,
        token: &str,
        id: &str,
        body: &UpdateWheelRequest,
    ) -> Result<Wheel, ApiError> {
        Self::parse(
            self.http
                .patch(self.url(&["wheels", id])?)
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    pub async fn delete_wheel(&self, token: &str, id: &str) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .delete(self.url(&["wheels", id])?)
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// Spin a saved wheel or a quick one in a chat: the spin's line.
    pub async fn spin_wheel(
        &self,
        token: &str,
        channel: ChannelId,
        body: &SpinWheelRequest,
    ) -> Result<Message, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/channels/{channel}/wheel-spins", self.base))
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    /// Spin quietly (#270): the chat watches it land, but nothing is saved.
    pub async fn spin_wheel_quietly(
        &self,
        token: &str,
        channel: ChannelId,
        body: &SpinWheelRequest,
    ) -> Result<QuietSpin, ApiError> {
        Self::parse(
            self.http
                .post(format!(
                    "{}/channels/{channel}/wheel-spins/quiet",
                    self.base
                ))
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    /// Spin again from a spin's line, perhaps without its winner.
    pub async fn spin_again(
        &self,
        token: &str,
        line: MessageId,
        remove_winner: bool,
    ) -> Result<Message, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/messages/{line}/spin-again", self.base))
                .bearer_auth(token)
                .json(&SpinAgainRequest { remove_winner })
                .send()
                .await,
        )
        .await
    }

    /// Post a poll (#157): its message, with the poll as its creator sees it.
    pub async fn create_poll(
        &self,
        token: &str,
        channel: ChannelId,
        body: &pulse_protocol::rest::CreatePollRequest,
    ) -> Result<Message, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/channels/{channel}/polls", self.base))
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    /// A poll as you see it now.
    pub async fn get_poll(
        &self,
        token: &str,
        poll: MessageId,
    ) -> Result<pulse_protocol::rest::Poll, ApiError> {
        Self::parse(
            self.http
                .get(format!("{}/messages/{poll}/poll", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    /// Your choices in a poll, replacing any before (none takes your vote back).
    pub async fn vote_poll(
        &self,
        token: &str,
        poll: MessageId,
        options: Vec<u32>,
    ) -> Result<pulse_protocol::rest::Poll, ApiError> {
        Self::parse(
            self.http
                .put(format!("{}/messages/{poll}/poll/votes", self.base))
                .bearer_auth(token)
                .json(&pulse_protocol::rest::PollVoteRequest { options })
                .send()
                .await,
        )
        .await
    }

    /// End your poll now.
    pub async fn close_poll(
        &self,
        token: &str,
        poll: MessageId,
    ) -> Result<pulse_protocol::rest::Poll, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/messages/{poll}/poll/close", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
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
    use pulse_protocol::rest::NotifyLevel;
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
    async fn a_quiet_spin_comes_back_drawn() {
        let app = testing::spawn().await;
        let (_, alex) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &alex, "Main").await;
        let g = testing::general(&app, &alex, s.id).await;
        let api = Api::new(&format!("http://{}", app.addr));
        let req = SpinWheelRequest {
            wheel_id: None,
            options: Some(vec!["pizza".into(), "sushi".into()]),
        };
        let q = api.spin_wheel_quietly(&alex, g.id, &req).await.unwrap();
        assert_eq!(q.channel_id, g.id);
        assert_eq!(q.spin.options, vec!["pizza", "sushi"]);
        assert!(q.spin.winner < 2);
    }

    #[tokio::test]
    async fn a_server_channel_is_renamed_then_deleted() {
        let app = testing::spawn().await;
        let (_, alex) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &alex, "Main").await;
        let g = testing::general(&app, &alex, s.id).await;
        let api = Api::new(&format!("http://{}", app.addr));
        let ch = api
            .rename_channel(&alex, g.id, Some("chat".into()))
            .await
            .unwrap();
        assert_eq!(ch.name.as_deref(), Some("chat"));
        api.delete_channel(&alex, g.id).await.unwrap();
        assert!(
            api.delete_channel(&alex, g.id).await.is_err(),
            "already deleted"
        );
        // Recently deleted: restore, delete again, then purge.
        let gone = api.list_deleted_channels(&alex, s.id).await.unwrap();
        assert_eq!(gone.len(), 1);
        assert!(gone[0].can_restore && gone[0].can_purge);
        assert_eq!(api.restore_channel(&alex, g.id).await.unwrap().id, g.id);
        api.delete_channel(&alex, g.id).await.unwrap();
        api.purge_channel(&alex, g.id).await.unwrap();
        assert!(
            api.list_deleted_channels(&alex, s.id)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(api.restore_channel(&alex, g.id).await.is_err(), "gone");
    }

    #[tokio::test]
    async fn a_server_is_deleted_and_restored() {
        let app = testing::spawn().await;
        let (_, alex) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &alex, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        assert!(
            api.delete_server(&alex, s.id, "nope").await.is_err(),
            "the name must match"
        );
        api.delete_server(&alex, s.id, "Main").await.unwrap();
        let gone = api.list_deleted_servers(&alex).await.unwrap();
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].server.id, s.id);
        assert!(gone[0].can_restore);
        assert_eq!(api.restore_server(&alex, s.id).await.unwrap().id, s.id);
        assert!(api.list_deleted_servers(&alex).await.unwrap().is_empty());
        assert!(api.restore_server(&alex, s.id).await.is_err(), "live again");
    }

    #[tokio::test]
    async fn categories_round_trip_through_the_core() {
        use pulse_protocol::rest::{LayoutCategory, SaveLayoutRequest, ServerLayout};
        let app = testing::spawn().await;
        let (_, alex) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &alex, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let made = api.create_category(&alex, s.id, "Gaming").await.unwrap();
        assert_eq!((made.name.as_str(), made.server_id), ("Gaming", s.id));
        let renamed = api.rename_category(&alex, made.id, "Games").await.unwrap();
        assert_eq!((renamed.id, renamed.name.as_str()), (made.id, "Games"));
        let ch = api
            .create_channel_in(&alex, s.id, ChannelKind::Text, "speedruns", Some(made.id))
            .await
            .unwrap();
        assert_eq!(ch.category_id, Some(made.id));
        // Move it out to the top, keeping everything else where it is.
        let base: ServerLayout = {
            let mut c = app.db.acquire().await.unwrap();
            pulse_server::servers::categories::layout_of(&mut c, s.id)
                .await
                .unwrap()
        };
        let request = SaveLayoutRequest {
            revision: base.revision,
            uncategorised: vec![ch.id],
            categories: base
                .categories
                .iter()
                .map(|cat| LayoutCategory {
                    id: cat.id,
                    channels: base
                        .channels
                        .iter()
                        .filter(|p| p.category_id == Some(cat.id) && p.channel_id != ch.id)
                        .map(|p| p.channel_id)
                        .collect(),
                })
                .collect(),
        };
        let saved = api.save_layout(&alex, s.id, &request).await.unwrap();
        assert_eq!(saved.revision, base.revision + 1);
        let moved = saved.channels.iter().find(|p| p.channel_id == ch.id);
        assert_eq!(moved.map(|p| p.category_id), Some(None));
        // The same save again is based on an old revision: refused in the server's words.
        match api.save_layout(&alex, s.id, &request).await {
            Err(ApiError::Rejected(m)) => assert!(m.contains("order changed"), "{m}"),
            other => panic!("a stale save should be refused, got {other:?}"),
        }
        api.delete_category(&alex, made.id).await.unwrap();
        assert!(
            api.rename_category(&alex, made.id, "Again").await.is_err(),
            "deleted"
        );
    }

    #[tokio::test]
    async fn search_finds_and_filters() {
        use pulse_protocol::rest::SearchHas;
        let app = testing::spawn().await;
        let (alex_id, alex) = testing::register(&app, "alex").await;
        let (sam_id, sam) = testing::register(&app, "sam").await;
        let s = testing::create_server(&app, &alex, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        api.join_server(&sam, s.id).await.unwrap();
        let g = testing::general(&app, &alex, s.id).await;
        let mine = testing::send(&app, &alex, g.id, "pizza tonight").await;
        let theirs = testing::send(&app, &sam, g.id, "pizza please").await;
        let all = api
            .search(
                &alex,
                SearchScope::Server(s.id),
                &SearchRequest {
                    q: "pizza".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            all.hits.iter().map(|h| h.message.id).collect::<Vec<_>>(),
            vec![theirs.id, mine.id]
        );
        let req = SearchRequest {
            q: "pizza".into(),
            from: Some(sam_id),
            has: vec![],
            ..Default::default()
        };
        let from = api
            .search(&alex, SearchScope::Server(s.id), &req)
            .await
            .unwrap();
        assert_eq!(from.hits.len(), 1);
        // A conversation is searched on its own.
        let dm = api.create_dm(&alex, vec![sam_id]).await.unwrap();
        testing::send(&app, &sam, dm.id, "pizza in private").await;
        let dm_hits = api
            .search(&alex, SearchScope::Conversation(dm.id), &req)
            .await
            .unwrap();
        assert_eq!(dm_hits.hits[0].text, "pizza in private");
        // Every filter reaches the server: `has` too.
        let none = api
            .search(
                &alex,
                SearchScope::Server(s.id),
                &SearchRequest {
                    from: Some(alex_id),
                    in_channel: Some(g.id),
                    has: vec![SearchHas::Link, SearchHas::Clip],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(none.hits.is_empty());
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
    async fn change_password_signs_out_the_other_devices() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let LoginOutcome::Session(other) = api.login("alex", "hunter2hunter2").await.unwrap()
        else {
            panic!("no 2FA here");
        };
        let err = api
            .change_password(&token, "nopenopenope", "new password 1", None)
            .await
            .err()
            .unwrap();
        assert!(matches!(err, ApiError::Forbidden(_)), "{err:?}");
        let err = api
            .change_password(&token, "hunter2hunter2", "short", None)
            .await
            .err()
            .unwrap();
        assert!(matches!(err, ApiError::Rejected(_)), "{err:?}");
        api.change_password(&token, "hunter2hunter2", "new password 1", None)
            .await
            .unwrap();
        assert!(matches!(
            api.me(&other.token).await,
            Err(ApiError::Unauthorized)
        ));
        assert!(api.me(&token).await.is_ok(), "this device stays signed in");
        assert!(matches!(
            api.login("alex", "hunter2hunter2").await,
            Err(ApiError::Unauthorized)
        ));
        assert!(matches!(
            api.login("alex", "new password 1").await,
            Ok(LoginOutcome::Session(_))
        ));
    }

    #[tokio::test]
    async fn change_password_with_two_factor_needs_a_code() {
        let app = testing::spawn().await;
        let api = Api::new(&format!("http://{}", app.addr));
        let (secret, token, _) = two_factor_user(&app, &api).await;
        for code in [None, Some("not-a-code")] {
            let err = api
                .change_password(&token, "hunter2hunter2", "new password 1", code)
                .await
                .err()
                .unwrap();
            assert!(
                matches!(&err, ApiError::Forbidden(m) if m == WRONG_CODE),
                "{err:?}"
            );
        }
        api.change_password(
            &token,
            "hunter2hunter2",
            "new password 1",
            Some(&code_at(&secret, 1)),
        )
        .await
        .unwrap();
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

    /// #123: the UI tells a rate limit by its code, and still gets the same words.
    #[test]
    fn a_coded_error_carries_the_rate_limit_code_and_the_usual_words() {
        let json = |e: ApiError| serde_json::to_value(Coded(e)).unwrap();
        assert_eq!(
            json(ApiError::RateLimited),
            serde_json::json!({ "message": "slow down a little, then try again", "code": "rate_limited" })
        );
        assert_eq!(
            json(ApiError::Told {
                status: 429,
                message: "That's 20 clips this hour. Try again later.".into()
            })["code"],
            "rate_limited"
        );
        assert_eq!(
            json(ApiError::Rejected("not found".into())),
            serde_json::json!({ "message": "not found", "code": null })
        );
        // a plain ApiError is still just its words
        assert_eq!(
            serde_json::to_value(ApiError::RateLimited).unwrap(),
            "slow down a little, then try again"
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
                thread_root_id: None,
                gif: None,
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
                    thread_root_id: None,
                    gif: None,
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
    async fn hide_one_preview_then_the_rest() {
        use pulse_protocol::rest::{EmbedKind, EmbedProvider};
        use pulse_server::embeds::Unfurled;
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let g = testing::general(&app, &token, s.id).await;
        let api = Api::new(&format!("http://{}", app.addr));
        // No links in the text: nothing is fetched; the previews are stored as the unfurler would.
        let m = api
            .send_message(
                &token,
                g.id,
                &SendMessageRequest {
                    content: "look".into(),
                    reply_to_id: None,
                    nonce: None,
                    thread_root_id: None,
                    gif: None,
                },
            )
            .await
            .unwrap();
        let card = |title: &str| Unfurled {
            provider: EmbedProvider::Generic,
            kind: EmbedKind::Card,
            provider_id: None,
            title: Some(title.into()),
            description: None,
            site_name: None,
            author: None,
            thumb: None,
            start: None,
            vertical: false,
        };
        let rows = vec![
            (0, "https://example.com/a".to_string(), card("A")),
            (1, "https://example.com/b".to_string(), card("B")),
        ];
        assert!(
            pulse_server::embeds::store(&app.db, m.id, "look", rows)
                .await
                .unwrap()
        );
        let after = api
            .hide_embed(&token, m.id, Some("https://example.com/a"))
            .await
            .unwrap();
        let urls: Vec<_> = after.embeds.iter().map(|e| e.url.as_str()).collect();
        assert_eq!(urls, ["https://example.com/b"]);
        let after = api.hide_embed(&token, m.id, None).await.unwrap();
        assert!(after.embeds.is_empty());
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
            allow_ghosts: true,
        }
    }

    #[tokio::test]
    async fn stream_ticket_after_join() {
        let app = testing::spawn().await;
        let (api, token, uid, ch) = voice_fixture(&app, "alex").await;
        // not in the call yet: refused, and as Forbidden (the retry keys on it)
        assert!(matches!(
            api.stream_ticket(&token, ch, StreamRole::V, false).await,
            Err(ApiError::Forbidden(_))
        ));
        app.voice.join(ch, uid, "PA_test");
        for role in [StreamRole::V, StreamRole::A] {
            let t = api.stream_ticket(&token, ch, role, false).await.unwrap();
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
        api.watch_stream(&sam_token, ch, uid, false).await.unwrap();
        api.unwatch_stream(&sam_token, ch, uid).await.unwrap();
        // you can't watch yourself
        assert!(matches!(
            api.watch_stream(&token, ch, uid, false).await,
            Err(ApiError::Rejected(_))
        ));
    }

    /// Out of the call, a server member watches as a ghost: an anonymous ticket and a count.
    #[tokio::test]
    async fn ghost_ticket_and_watch_from_outside_the_call() {
        let app = testing::spawn().await;
        let (api, token, uid, ch) = voice_fixture(&app, "alex").await;
        let (sam, sam_token) = testing::register(&app, "sam").await;
        let server = api.servers(&token).await.unwrap()[0].id;
        testing::post_json(
            &app,
            &sam_token,
            &format!("/servers/{server}/join"),
            serde_json::json!({}),
        )
        .await;
        app.voice.join(ch, uid, "PA_test");
        api.go_live(&token, ch, settings()).await.unwrap();
        // as themselves, out of the call: refused (and the retry keys on Forbidden)
        assert!(matches!(
            api.stream_ticket(&sam_token, ch, StreamRole::V, false)
                .await,
            Err(ApiError::Forbidden(_))
        ));
        let t = api
            .stream_ticket(&sam_token, ch, StreamRole::V, true)
            .await
            .unwrap();
        assert!(!t.token.is_empty());
        api.watch_stream(&sam_token, ch, uid, true).await.unwrap();
        // never named (the count follows its LiveKit connection, which this test has none of)
        assert!(app.streams.get(ch, uid).unwrap().viewers.is_empty());
        // as a ghost from inside the call: refused
        assert!(matches!(
            api.watch_stream(&token, ch, uid, true).await,
            Err(ApiError::Forbidden(_))
        ));
        api.unwatch_stream(&sam_token, ch, uid).await.unwrap();
        assert!(!app.voice.members(ch).contains(&sam));
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
        let t = crate::stream::ticket_with_retry(&api, &token, ch, StreamRole::V, false)
            .await
            .unwrap();
        assert!(!t.token.is_empty());
    }

    #[tokio::test]
    async fn stream_ticket_gives_up_on_a_persistent_403() {
        let app = testing::spawn().await;
        let (api, token, _, ch) = voice_fixture(&app, "alex").await;
        let started = std::time::Instant::now();
        let r = crate::stream::ticket_with_retry(&api, &token, ch, StreamRole::V, false).await;
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

    /// Everything logged (at any level) while `f` runs, on this thread.
    async fn logged<F: std::future::Future>(f: F) -> (F::Output, String) {
        use std::sync::{Arc, Mutex};
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let w = buf.clone();
        let sub = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(move || Writer(w.clone()))
            .finish();
        let out = {
            let _g = tracing::subscriber::set_default(sub);
            f.await
        };
        let text = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        (out, text)
    }
    struct Writer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Writer {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_answer_that_wont_decode_is_logged() {
        // a 200 whose body isn't what we expected: the UI shows an error, so the log must say why
        let resp = reqwest::Response::from(
            tauri::http::Response::builder()
                .status(200)
                .body("<html>not json</html>")
                .unwrap(),
        );
        let (r, log) = logged(Api::parse::<MediaInfo>(Ok(resp))).await;
        assert!(matches!(r, Err(ApiError::Network(_))), "{r:?}");
        assert!(log.contains("request failed"), "{log}");
        assert!(log.contains("decod"), "{log}");
        assert!(log.contains("path=\"/\""), "{log}");
        // the status, and what kind of JSON problem (never the body itself)
        assert!(log.contains("status=200"), "{log}");
        assert!(log.contains("json=\"syntax at 1:1\""), "{log}");
        assert!(!log.contains("not json"), "{log}");
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

    /// The channel menu's Mark as read (#111): the channel and its reply threads, in one call.
    #[tokio::test]
    async fn mark_channel_read_marks_its_threads_seen() {
        use pulse_protocol::rest::NewQuoteLine;
        let app = testing::spawn().await;
        let (_, alex) = testing::register(&app, "alex").await;
        let (sam_id, sam) = testing::register(&app, "sam").await;
        let s = testing::create_server(&app, &alex, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        api.join_server(&sam, s.id).await.unwrap();
        let ch = api
            .create_channel(&alex, s.id, ChannelKind::Quotes, "quotes")
            .await
            .unwrap();
        let quote = api
            .post_quote(
                &alex,
                ch.id,
                &PostQuoteRequest {
                    lines: vec![NewQuoteLine {
                        id: None,
                        speaker_id: Some(sam_id),
                        speaker_name: None,
                        text: Some("hm".into()),
                        media_hash: None,
                        caption: None,
                    }],
                    today: None,
                    nonce: None,
                },
            )
            .await
            .unwrap();
        let reply = api
            .send_message(
                &sam,
                ch.id,
                &SendMessageRequest {
                    content: "hey @alex".into(),
                    reply_to_id: None,
                    nonce: None,
                    thread_root_id: Some(quote.id),
                    gif: None,
                },
            )
            .await
            .unwrap();
        api.mark_channel_read(&alex, ch.id).await.unwrap();
        let listed = api.messages(&alex, ch.id, None).await.unwrap();
        assert_eq!(
            listed[0].quote.as_ref().unwrap().replies_seen,
            Some(reply.id)
        );
    }

    #[tokio::test]
    async fn quotes_round_trip_through_the_core() {
        use pulse_protocol::rest::NewQuoteLine;
        let app = testing::spawn().await;
        let (_, alex) = testing::register(&app, "alex").await;
        let (sam_id, sam) = testing::register(&app, "sam").await;
        let s = testing::create_server(&app, &alex, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        api.join_server(&sam, s.id).await.unwrap();
        let ch = api
            .create_channel(&alex, s.id, ChannelKind::Quotes, "quotes")
            .await
            .unwrap();
        let line = |text: &str| NewQuoteLine {
            id: None,
            speaker_id: Some(sam_id),
            speaker_name: None,
            text: Some(text.into()),
            media_hash: None,
            caption: None,
        };
        let quote = api
            .post_quote(
                &alex,
                ch.id,
                &PostQuoteRequest {
                    lines: vec![line("I meant to do that")],
                    today: None,
                    nonce: Some("q1".into()),
                },
            )
            .await
            .unwrap();
        let q = quote.quote.as_ref().expect("a quote");
        assert_eq!(q.lines[0].text.as_deref(), Some("I meant to do that"));

        // edit: same line id, new text
        let mut edited = line("I meant to do that, honest");
        edited.id = Some(q.lines[0].id);
        let quote = api
            .edit_quote(
                &alex,
                quote.id,
                &EditQuoteRequest {
                    lines: vec![edited],
                },
            )
            .await
            .unwrap();
        let line_id = quote.quote.as_ref().unwrap().lines[0].id;

        // an award from someone who isn't the speaker
        let (_, jo) = testing::register(&app, "jo").await;
        api.join_server(&jo, s.id).await.unwrap();
        api.give_award(&jo, line_id).await.unwrap();
        let listed = api.messages(&jo, ch.id, None).await.unwrap();
        assert_eq!(listed[0].quote.as_ref().unwrap().lines[0].awards, 1);
        api.take_award(&jo, line_id).await.unwrap();
        api.give_award(&jo, line_id).await.unwrap();

        // a reply stays out of the main list, and reads as seen
        let reply = api
            .send_message(
                &sam,
                ch.id,
                &SendMessageRequest {
                    content: "no I didn't".into(),
                    reply_to_id: None,
                    nonce: Some("r1".into()),
                    thread_root_id: Some(quote.id),
                    gif: None,
                },
            )
            .await
            .unwrap();
        let thread = api.replies(&alex, quote.id, None).await.unwrap();
        assert_eq!(thread.iter().map(|m| m.id).collect::<Vec<_>>(), [reply.id]);
        let seen_before = api.messages(&alex, ch.id, None).await.unwrap();
        assert_eq!(seen_before.len(), 1, "the reply is not in the main list");
        assert_eq!(seen_before[0].quote.as_ref().unwrap().replies_seen, None);
        api.mark_replies_seen(&alex, quote.id, reply.id)
            .await
            .unwrap();
        let seen_after = api.messages(&alex, ch.id, None).await.unwrap();
        assert_eq!(
            seen_after[0].quote.as_ref().unwrap().replies_seen,
            Some(reply.id)
        );
        assert!(
            api.replies(&alex, quote.id, Some(reply.id))
                .await
                .unwrap()
                .is_empty(),
            "nothing older than the only reply"
        );

        // the board ranks by reactions; the search finds it by its words
        api.add_reaction(&jo, quote.id, "🔥").await.unwrap();
        let board = api.leaderboard(&sam, s.id, None).await.unwrap();
        assert_eq!(board.top_all_time[0].id, quote.id);
        let this_year = quote.quote.as_ref().unwrap().year;
        let board_year = api.leaderboard(&sam, s.id, Some(this_year)).await.unwrap();
        assert_eq!(board_year.top_year[0].id, quote.id);
        let board_other = api
            .leaderboard(&sam, s.id, Some(this_year - 1))
            .await
            .unwrap();
        assert!(board_other.top_year.is_empty());
        let found = api.search_quotes(&sam, s.id, "honest").await.unwrap();
        assert_eq!(found[0].id, quote.id);

        // a second quote, then the window around the first
        let later = api
            .post_quote(
                &alex,
                ch.id,
                &PostQuoteRequest {
                    lines: vec![line("and another")],
                    today: None,
                    nonce: None,
                },
            )
            .await
            .unwrap();
        let latest = api
            .post_quote(
                &alex,
                ch.id,
                &PostQuoteRequest {
                    lines: vec![line("and one more")],
                    today: None,
                    nonce: None,
                },
            )
            .await
            .unwrap();
        let around = api
            .messages_around(&alex, ch.id, quote.id)
            .await
            .unwrap()
            .expect("found");
        // a message that isn't there (deleted, another channel's, never existed) isn't an error
        let missing = api.messages_around(&alex, ch.id, MessageId::new()).await;
        assert!(matches!(missing, Ok(None)), "{missing:?}");
        assert!(around.iter().any(|m| m.id == quote.id));
        assert!(around.iter().any(|m| m.id == later.id));
        let after = api.messages_after(&alex, ch.id, quote.id).await.unwrap();
        assert_eq!(
            after.iter().map(|m| m.id).collect::<Vec<_>>(),
            [latest.id, later.id],
            "newest first"
        );

        // a plain-name speaker can be linked and unlinked
        let named = api
            .post_quote(
                &alex,
                ch.id,
                &PostQuoteRequest {
                    lines: vec![NewQuoteLine {
                        speaker_id: None,
                        speaker_name: Some("Sammy".into()),
                        ..line("hello")
                    }],
                    today: None,
                    nonce: None,
                },
            )
            .await
            .unwrap();
        assert!(named.quote.is_some());
        api.link_speaker(
            &alex,
            s.id,
            &LinkSpeakerRequest {
                name: "Sammy".into(),
                user_id: sam_id,
            },
        )
        .await
        .unwrap();
        api.unlink_speaker(&alex, s.id, "sammy").await.unwrap();
        assert!(
            api.unlink_speaker(&alex, s.id, "sammy").await.is_err(),
            "the link was removed"
        );

        // Quote of the Year: start, vote, end
        let year = quote.quote.as_ref().unwrap().year;
        let ev = api
            .start_qoty(
                &alex,
                s.id,
                &StartQotyRequest {
                    year,
                    minutes: None,
                },
            )
            .await
            .unwrap();
        assert!(
            ev.options.iter().any(|o| o.message.id == quote.id),
            "the reacted-to quote is a starting option"
        );
        assert!(!ev.options.iter().any(|o| o.message.id == later.id));
        let ev = api.add_qoty_option(&alex, ev.id, later.id).await.unwrap();
        assert!(ev.options.iter().any(|o| o.message.id == later.id));
        let voted = api.vote_qoty(&sam, ev.id, quote.id).await.unwrap();
        let option = voted
            .options
            .iter()
            .find(|o| o.message.id == quote.id)
            .unwrap();
        assert_eq!(option.voters, [sam_id]);
        let unvoted = api.unvote_qoty(&sam, ev.id).await.unwrap();
        assert_eq!(unvoted.voters, 0);
        api.vote_qoty(&sam, ev.id, quote.id).await.unwrap();
        let ended = api.end_qoty(&alex, ev.id).await.unwrap();
        assert!(ended.ended_at.is_some());
        let reopened = api.reopen_qoty(&alex, ev.id, Some(60)).await.unwrap();
        assert!(reopened.ended_at.is_none());
        assert!(reopened.ends_at.is_some());
        api.end_qoty(&alex, ev.id).await.unwrap();
        let open_ended = api.reopen_qoty(&alex, ev.id, None).await.unwrap();
        assert!(open_ended.ended_at.is_none() && open_ended.ends_at.is_none());
    }

    #[tokio::test]
    async fn a_quote_image_uploads_through_the_core() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let info = api
            .upload_quote_media(&token, crate::media::tests::PNG.to_vec())
            .await
            .unwrap();
        assert!(crate::media::is_hash(&info.hash));
    }

    #[tokio::test]
    async fn a_quote_image_saves_as_its_original_file() {
        // Save image (#206): the stored file itself (not its still), streamed to the chosen path
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let info = api
            .upload_quote_media(&token, crate::media::tests::PNG.to_vec())
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("image.webp");
        let req = api.media_request(&token, &info.hash).unwrap();
        crate::clips::download::save_request(req, &dest)
            .await
            .unwrap();
        let saved = std::fs::read(&dest).unwrap();
        assert!(
            crate::media::verify(&info.hash, &saved),
            "the saved bytes are the stored file"
        );
        assert!(api.media_request(&token, "../me").is_err());
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
    async fn a_profile_round_trips_through_the_core() {
        let app = testing::spawn().await;
        let (me, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let crop = CropRect {
            x: 0,
            y: 0,
            w: 1,
            h: 1,
        };
        let avatar = api
            .upload_profile_media(
                &token,
                ProfileMediaKind::Avatar,
                crate::media::tests::PNG.to_vec(),
                Some(crop),
            )
            .await
            .unwrap();
        let before = api.profile(&token, me).await.unwrap();
        let saved = api
            .update_profile(
                &token,
                &UpdateProfileRequest {
                    avatar_hash: Some(avatar.hash.clone()),
                    banner_hash: None,
                    theme_top: "#8a3cff".into(),
                    theme_bottom: "#ff8a2a".into(),
                    style: before.user.style.clone(),
                    pronouns: Some("they/them".into()),
                    bio: Some("hi".into()),
                    display_name: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(saved.card.theme_top, "#8a3cff");
        let read = api.profile(&token, me).await.unwrap();
        assert_eq!(read.user.avatar_hash.as_deref(), Some(avatar.hash.as_str()));
        assert_eq!(read.card.pronouns.as_deref(), Some("they/them"));
        assert_eq!(read.card.bio.as_deref(), Some("hi"));
    }

    #[tokio::test]
    async fn badge_catalogue_and_showcase_round_trip() {
        let app = testing::spawn().await;
        let (me, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let before = api.badge_catalogue(&token).await.unwrap();
        assert!(before.people >= 1);
        assert!(
            before.badges.iter().all(|b| b.key != "logo_egg"),
            "masked until found"
        );

        api.badge_egg(&token, &EggRequest { egg: "logo".into() })
            .await
            .unwrap();
        let after = api.badge_catalogue(&token).await.unwrap();
        assert!(after.badges.iter().any(|b| b.key == "logo_egg"));

        api.set_showcase(
            &token,
            &SetShowcaseRequest {
                keys: vec!["logo_egg".into()],
            },
        )
        .await
        .unwrap();
        let shown: Vec<_> = api
            .profile(&token, me)
            .await
            .unwrap()
            .badges
            .iter()
            .map(|b| pulse_protocol::rest::SeenBadge {
                key: b.key.clone(),
                tier: b.tier,
            })
            .collect();
        api.badges_seen(&token, &MarkBadgesSeenRequest { badges: shown })
            .await
            .unwrap();
        // not in a voice channel: accepted, quietly not counted
        api.muted_talk(&token).await.unwrap();
        let read = api.profile(&token, me).await.unwrap();
        assert_eq!(read.showcase, vec!["logo_egg".to_string()]);
        assert!(read.badges.iter().all(|b| b.seen));
        assert!(
            api.badge_egg(&token, &EggRequest { egg: "nope".into() })
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn server_mute_and_disconnect_round_trip_through_the_core() {
        let app = testing::spawn().await;
        let (alex_id, alex) = testing::register(&app, "alex").await;
        let (sam_id, sam) = testing::register(&app, "sam").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let server = api.create_server(&alex, "Main").await.unwrap();
        api.join_server(&sam, server.id).await.unwrap();
        let lounge = api
            .channels(&alex, server.id)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.name.as_deref() == Some("Lounge"))
            .unwrap();
        app.voice.join(lounge.id, sam_id, "PA_sam");
        let req = SetServerVoiceRequest {
            mute: Some(true),
            deafen: None,
        };
        let m = api
            .set_server_voice(&alex, server.id, sam_id, &req)
            .await
            .unwrap();
        assert_eq!((m.muted_by, m.deafened_by), (Some(alex_id), None));
        // (LiveKit is down in tests: the removal is logged, the request still succeeds)
        api.disconnect_voice(&alex, server.id, sam_id)
            .await
            .unwrap();
        assert!(
            api.disconnect_voice(&sam, server.id, sam_id).await.is_err(),
            "not on yourself"
        );
    }

    #[tokio::test]
    async fn calls_round_trip_through_the_core() {
        let app = testing::spawn().await;
        let (_, alex) = testing::register(&app, "alex").await;
        let (sam_id, sam) = testing::register(&app, "sam").await;
        let (_, kim) = testing::register(&app, "kim").await;
        let dm = testing::create_dm(&app, &alex, &[sam_id]).await;
        let api = Api::new(&format!("http://{}", app.addr));
        let first = api.start_call(&alex, dm.id).await.unwrap();
        assert!(first.rang);
        let again = api.start_call(&sam, dm.id).await.unwrap();
        assert!(!again.rang, "joining a call going on rings nobody");
        assert_eq!(again.call.id, first.call.id);
        api.decline_call(&sam, dm.id).await.unwrap();
        assert!(api.start_call(&kim, dm.id).await.is_err(), "not kim's DM");
        assert!(api.decline_call(&kim, dm.id).await.is_err());
    }

    #[tokio::test]
    async fn status_and_appear_offline_round_trip_through_the_core() {
        let app = testing::spawn().await;
        let (alex_id, alex) = testing::register(&app, "alex").await;
        let (sam_id, _) = testing::register(&app, "sam").await;
        let api = Api::new(&format!("http://{}", app.addr));
        api.set_status(&alex, Status::Dnd, None).await.unwrap();
        // a past end is refused (the server checks every timer)
        assert!(
            api.set_status(&alex, Status::Dnd, Some("2000-01-01T00:00:00Z".into()))
                .await
                .is_err()
        );
        let err = api
            .set_status(&alex, Status::Online, Some("2000-01-01T00:00:00Z".into()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("timer"), "{err}");
        api.set_status(&alex, Status::Invisible, None)
            .await
            .unwrap();
        api.set_appear_offline(&alex, sam_id, true).await.unwrap();
        api.set_appear_offline(&alex, sam_id, false).await.unwrap();
        assert!(api.set_appear_offline(&alex, alex_id, true).await.is_err());
    }

    #[tokio::test]
    async fn group_icon_round_trips_through_the_core() {
        let app = testing::spawn().await;
        let (_, alex) = testing::register(&app, "alex").await;
        let (sam_id, _) = testing::register(&app, "sam").await;
        let (kim_id, _) = testing::register(&app, "kim").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let g = api.create_dm(&alex, vec![sam_id, kim_id]).await.unwrap();
        // a hash that was never uploaded is refused
        assert!(
            api.set_group_icon(&alex, g.id, Some("ab".repeat(32)))
                .await
                .is_err()
        );
        let ch = api.set_group_icon(&alex, g.id, None).await.unwrap();
        assert_eq!(ch.icon_hash, None);
    }

    #[tokio::test]
    async fn live_notify_round_trips_through_the_core() {
        let app = testing::spawn().await;
        let (alex_id, alex) = testing::register(&app, "alex").await;
        let (sam_id, _) = testing::register(&app, "sam").await;
        let api = Api::new(&format!("http://{}", app.addr));
        api.set_live_notify(&alex, sam_id, true).await.unwrap();
        api.set_live_notify(&alex, sam_id, false).await.unwrap();
        assert!(api.set_live_notify(&alex, alex_id, true).await.is_err());
        assert!(
            api.set_live_notify(&alex, UserId::new(), true)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn notify_levels_round_trip_through_the_core() {
        let app = testing::spawn().await;
        let (_, alex) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let server = api.create_server(&alex, "Main").await.unwrap();
        let req = |level| SetNotifyLevelRequest {
            target_kind: MuteTarget::Server,
            target_id: server.id.to_string(),
            level,
        };
        api.set_notify_level(&alex, &req(Some(NotifyLevel::Nothing)))
            .await
            .unwrap();
        api.set_notify_level(&alex, &req(None)).await.unwrap();
        let unknown = SetNotifyLevelRequest {
            target_id: UserId::new().to_string(),
            ..req(Some(NotifyLevel::All))
        };
        assert!(api.set_notify_level(&alex, &unknown).await.is_err());
    }

    #[tokio::test]
    async fn nicknames_and_the_name_archive_round_trip_through_the_core() {
        let app = testing::spawn().await;
        let (_, alex) = testing::register(&app, "alex").await;
        let (sam_id, sam) = testing::register(&app, "sam").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let server = api.create_server(&alex, "Main").await.unwrap();
        api.join_server(&sam, server.id).await.unwrap();

        let m = api
            .set_nickname(&alex, server.id, sam_id, Some("Sammy".into()))
            .await
            .unwrap();
        assert_eq!(m.nickname.as_deref(), Some("Sammy"));
        api.set_nickname(&alex, server.id, sam_id, None)
            .await
            .unwrap();
        let page = api
            .name_archive(&sam, server.id, sam_id, None)
            .await
            .unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.entries[0].name, None);
        assert_eq!(page.entries[1].name.as_deref(), Some("Sammy"));

        api.set_private_nickname(&alex, sam_id, Some("S".into()))
            .await
            .unwrap();
        api.set_private_nickname(&alex, sam_id, None).await.unwrap();
        let err = api
            .set_nickname(&alex, server.id, sam_id, Some("x".repeat(33)))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("1 to 32"), "{err}");
    }

    #[tokio::test]
    async fn a_crop_outside_the_picture_reaches_the_server_and_is_refused() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let past = CropRect {
            x: 0,
            y: 0,
            w: 2,
            h: 1,
        };
        let err = api
            .upload_profile_media(
                &token,
                ProfileMediaKind::Avatar,
                crate::media::tests::PNG.to_vec(),
                Some(past),
            )
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "The crop is outside the image.");
    }

    #[tokio::test]
    async fn an_oversized_profile_upload_says_8_mb() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let err = api
            .upload_profile_media(
                &token,
                ProfileMediaKind::Banner,
                vec![0; 9 * 1024 * 1024],
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), PROFILE_TOO_BIG);
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
                    thread_root_id: None,
                    gif: None,
                },
            )
            .await
            .unwrap();
        api.add_reaction(&token, m.id, "👍🏽").await.unwrap();
        assert_eq!(api.reactors(&token, m.id, "👍🏽").await.unwrap(), vec![me]);
        api.remove_reaction(&token, m.id, "👍🏽").await.unwrap();
        assert!(api.reactors(&token, m.id, "👍🏽").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn pin_list_and_unpin() {
        let app = testing::spawn().await;
        let (me, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let g = testing::general(&app, &token, s.id).await;
        let api = Api::new(&format!("http://{}", app.addr));
        let m = testing::send(&app, &token, g.id, "plan for tonight").await;
        let pin = api
            .pin_message(&token, m.id, PinDuration::Day)
            .await
            .unwrap();
        assert_eq!(pin.pinned_by, me);
        assert!(pin.expires_at.is_some());
        let list = api.list_pins(&token, g.id).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].message.id, m.id);
        api.unpin_message(&token, m.id).await.unwrap();
        assert!(api.list_pins(&token, g.id).await.unwrap().is_empty());
        let again = api.unpin_message(&token, m.id).await.unwrap_err();
        assert!(matches!(again, ApiError::Rejected(_)), "{again:?}");
    }

    #[tokio::test]
    async fn share_to_stage_needs_the_call_and_stop_checks_the_id() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let chans: Vec<Channel> =
            testing::get_json(&app, &token, &format!("/servers/{}/channels", s.id)).await;
        let lounge = chans.iter().find(|c| c.kind == ChannelKind::Voice).unwrap();
        let api = Api::new(&format!("http://{}", app.addr));
        // not in the call
        let e = api
            .share_to_stage(&token, lounge.id, "https://example.com/x")
            .await
            .unwrap_err();
        assert!(matches!(e, ApiError::Forbidden(_)), "{e:?}");
        // an id that isn't one never leaves the client
        let e = api
            .stop_stage_share(&token, lounge.id, "../pins")
            .await
            .unwrap_err();
        assert!(matches!(e, ApiError::Rejected(_)), "{e:?}");
        let e = api
            .stop_stage_share(&token, lounge.id, "01ARZ3NDEKTSV4RRFFQ69G5FAV")
            .await
            .unwrap_err();
        assert!(matches!(e, ApiError::Forbidden(_)), "{e:?}");
    }

    // ---------- Clips ----------

    /// A decodable image no other call produces (the media store dedups by content hash and keeps
    /// the first uploader).
    fn unique_png() -> Vec<u8> {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
        let n = N.fetch_add(1, Ordering::SeqCst);
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

    /// A clips channel with alex in it, a ticket for a file of `len` bytes, and that file.
    struct ClipRig {
        app: testing::TestApp,
        api: Api,
        token: String,
        server: pulse_protocol::rest::Server,
        channel: Channel,
        file: tempfile::NamedTempFile,
        bytes: Vec<u8>,
    }

    async fn clip_rig(len: usize) -> ClipRig {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let server = testing::create_server(&app, &token, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let channel = api
            .create_channel(&token, server.id, ChannelKind::Clips, "clips")
            .await
            .unwrap();
        let bytes: Vec<u8> = (0..len).map(|i| (i * 31 % 251) as u8).collect();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &bytes).unwrap();
        ClipRig {
            app,
            api,
            token,
            server,
            channel,
            file,
            bytes,
        }
    }

    impl ClipRig {
        async fn ticket(&self) -> pulse_protocol::rest::ClipUpload {
            self.api
                .clip_upload(
                    &self.token,
                    self.channel.id,
                    &ClipUploadRequest {
                        size: self.bytes.len() as u64,
                        duration_ms: 3000,
                    },
                )
                .await
                .unwrap()
        }
    }

    #[tokio::test]
    async fn clips_round_trip_through_the_core() {
        use crate::clips::upload::{CancellationFlag, put_file};
        use pulse_protocol::rest::{ClipView, ConfirmClipRequest};
        const THREE_MIB: usize = 3 * 1024 * 1024;
        let r = clip_rig(THREE_MIB).await;
        let game = r
            .api
            .add_game(&r.token, r.server.id, "Rocket League")
            .await
            .unwrap();
        assert_eq!(game.name, "Rocket League");

        let ticket = r.ticket().await;
        let sent = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let seen = sent.clone();
        put_file(
            &reqwest::Client::new(),
            &ticket.url,
            r.file.path(),
            &ticket.content_type,
            move |s, t| {
                assert_eq!(t, THREE_MIB as u64);
                seen.fetch_max(s, Ordering::SeqCst);
            },
            CancellationFlag::new(),
        )
        .await
        .unwrap();
        assert_eq!(sent.load(Ordering::SeqCst), THREE_MIB as u64);

        let thumb = r
            .api
            .upload_clip_thumb(&r.token, unique_png())
            .await
            .unwrap();
        let msg = r
            .api
            .confirm_clip(
                &r.token,
                ticket.id,
                &ConfirmClipRequest {
                    caption: "ace".into(),
                    game_id: Some(game.id),
                    duration_ms: 3000,
                    width: 1280,
                    height: 720,
                    thumb_hash: thumb.hash,
                    nonce: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(msg.content, "ace");

        let listed = r
            .api
            .list_clips(&r.token, r.channel.id, None, None, None)
            .await
            .unwrap();
        assert_eq!(
            listed.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![msg.id]
        );
        let by_game = r
            .api
            .list_clips(&r.token, r.channel.id, None, Some(game.id), None)
            .await
            .unwrap();
        assert_eq!(by_game.len(), 1);
        assert!(
            r.api
                .list_clips(&r.token, r.channel.id, Some(msg.id), None, None)
                .await
                .unwrap()
                .is_empty()
        );
        let facets = r.api.clip_facets(&r.token, r.channel.id).await.unwrap();
        assert_eq!(facets.games[0].game_id, game.id);

        let url = r.api.clip_url(&r.token, msg.id).await.unwrap();
        let got = reqwest::get(&url.url).await.unwrap().bytes().await.unwrap();
        assert!(
            got.as_ref() == r.bytes.as_slice(),
            "the played bytes are the uploaded ones"
        );
        // Download (#204): the same link, streamed to the file the Save dialog picked
        assert_eq!(crate::clips::download::extension_of(&url.url), "mp4");
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("ace.mp4");
        crate::clips::download::save_url(&reqwest::Client::new(), &url.url, &dest)
            .await
            .unwrap();
        assert!(
            std::fs::read(&dest).unwrap() == r.bytes,
            "the saved file is the uploaded one"
        );

        let edited = r
            .api
            .edit_clip(
                &r.token,
                msg.id,
                &EditClipRequest {
                    caption: "ace!".into(),
                    game_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(edited.content, "ace!");

        r.api
            .set_clip_view(&r.token, r.channel.id, ClipView::Wall)
            .await
            .unwrap();
        let storage = r.api.clip_storage(&r.token, r.server.id).await.unwrap();
        assert_eq!(storage.count, 1);
        // the video plus its small poster
        assert!((THREE_MIB as u64..THREE_MIB as u64 + 4096).contains(&storage.bytes));
    }

    #[tokio::test]
    async fn a_cancelled_ticket_is_gone() {
        let r = clip_rig(1024).await;
        let ticket = r.ticket().await;
        r.api.cancel_clip_upload(&r.token, ticket.id).await.unwrap();
        assert!(r.api.cancel_clip_upload(&r.token, ticket.id).await.is_err());
    }

    #[tokio::test]
    async fn put_file_stops_on_cancel() {
        use crate::clips::upload::{CancellationFlag, UploadError, put_file};
        let r = clip_rig(3 * 1024 * 1024).await;
        let ticket = r.ticket().await;
        r.app
            .s3
            .as_ref()
            .unwrap()
            .set_delay(Duration::from_secs(30));
        let cancel = CancellationFlag::new();
        let c = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            c.cancel();
        });
        let started = std::time::Instant::now();
        let res = put_file(
            &reqwest::Client::new(),
            &ticket.url,
            r.file.path(),
            &ticket.content_type,
            |_, _| {},
            cancel,
        )
        .await;
        assert!(matches!(res, Err(UploadError::Cancelled)), "{res:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
        // the aborted PUT stored nothing
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(r.app.s3.as_ref().unwrap().keys().is_empty());
    }

    #[tokio::test]
    async fn put_file_gives_up_when_nothing_moves() {
        use crate::clips::upload::{CancellationFlag, UploadError, put_file_with_stall};
        let r = clip_rig(1024 * 1024).await;
        let ticket = r.ticket().await;
        r.app
            .s3
            .as_ref()
            .unwrap()
            .set_delay(Duration::from_secs(30));
        let res = put_file_with_stall(
            &reqwest::Client::new(),
            &ticket.url,
            r.file.path(),
            &ticket.content_type,
            |_, _| {},
            CancellationFlag::new(),
            Duration::from_millis(500),
        )
        .await;
        assert!(matches!(res, Err(UploadError::Stalled)), "{res:?}");
    }

    #[tokio::test]
    async fn put_file_reports_a_rejected_put() {
        use crate::clips::upload::{CancellationFlag, UploadError, put_file};
        let r = clip_rig(4096).await;
        let ticket = r.ticket().await;
        assert!(ticket.url.contains("X-Amz-Signature="));
        let bad = ticket.url.replace("X-Amz-Signature=", "X-Amz-Signature=00");
        let res = put_file(
            &reqwest::Client::new(),
            &bad,
            r.file.path(),
            &ticket.content_type,
            |_, _| {},
            CancellationFlag::new(),
        )
        .await;
        assert!(matches!(res, Err(UploadError::Rejected(403))), "{res:?}");
    }

    #[tokio::test]
    async fn clip_errors_carry_the_servers_own_words() {
        // 503: this server has no clips storage
        let app = testing::spawn_without_clips().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let ch = api
            .create_channel(&token, s.id, ChannelKind::Clips, "clips")
            .await
            .unwrap();
        let req = ClipUploadRequest {
            size: 10,
            duration_ms: 1000,
        };
        let err = api.clip_upload(&token, ch.id, &req).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "Clips storage isn't set up on this server."
        );

        // 429: the hourly limit
        let r = clip_rig(10).await;
        let mut last = None;
        for _ in 0..21 {
            match r
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
            {
                Ok(_) => {}
                Err(e) => {
                    last = Some(e);
                    break;
                }
            }
        }
        assert_eq!(
            last.expect("the 21st ticket is refused").to_string(),
            "That's 20 clips this hour. Try again later."
        );

        // everything else keeps the generic words
        let e = Api::new("http://127.0.0.1:1")
            .clip_storage("t", s.id)
            .await
            .unwrap_err();
        assert!(matches!(e, ApiError::Network(_)));
    }

    #[tokio::test]
    async fn progress_stays_under_the_total_until_the_store_answers() {
        use crate::clips::upload::{CancellationFlag, put_file};
        const LEN: u64 = 2 * 1024 * 1024;
        let r = clip_rig(LEN as usize).await;
        let ticket = r.ticket().await;
        r.app
            .s3
            .as_ref()
            .unwrap()
            .set_delay(Duration::from_millis(800));
        let seen = Arc::new(std::sync::Mutex::new(Vec::<u64>::new()));
        let log = seen.clone();
        let http = reqwest::Client::new();
        let done = put_file(
            &http,
            &ticket.url,
            r.file.path(),
            &ticket.content_type,
            move |s, _| log.lock().unwrap().push(s),
            CancellationFlag::new(),
        );
        tokio::pin!(done);
        // the whole file has been handed over, the answer has not come
        tokio::select! {
            _ = &mut done => panic!("answered before the delay"),
            () = tokio::time::sleep(Duration::from_millis(400)) => {}
        }
        let mid = seen.lock().unwrap().clone();
        assert!(!mid.is_empty() && mid.iter().all(|&s| s < LEN), "{mid:?}");
        done.await.unwrap();
        let all = seen.lock().unwrap().clone();
        assert_eq!(all.last(), Some(&LEN));
        assert_eq!(all.iter().filter(|&&s| s == LEN).count(), 1);
    }

    #[tokio::test]
    async fn list_clips_filters_by_author() {
        use pulse_protocol::rest::ConfirmClipRequest;
        let r = clip_rig(2048).await;
        let (sam_id, sam) = testing::register(&r.app, "sam").await;
        r.api.join_server(&sam, r.server.id).await.unwrap();
        let (alex_id, _) = r.api.me(&r.token).await.map(|u| (u.id, ())).unwrap();
        let mut ids = vec![];
        for (who, token) in [(alex_id, r.token.clone()), (sam_id, sam.clone())] {
            let ticket = r
                .api
                .clip_upload(
                    &token,
                    r.channel.id,
                    &ClipUploadRequest {
                        size: 2048,
                        duration_ms: 1000,
                    },
                )
                .await
                .unwrap();
            crate::clips::upload::put_file(
                &reqwest::Client::new(),
                &ticket.url,
                r.file.path(),
                &ticket.content_type,
                |_, _| {},
                crate::clips::upload::CancellationFlag::new(),
            )
            .await
            .unwrap();
            let thumb = r.api.upload_clip_thumb(&token, unique_png()).await.unwrap();
            let m = r
                .api
                .confirm_clip(
                    &token,
                    ticket.id,
                    &ConfirmClipRequest {
                        caption: String::new(),
                        game_id: None,
                        duration_ms: 1000,
                        width: 640,
                        height: 360,
                        thumb_hash: thumb.hash,
                        nonce: None,
                    },
                )
                .await
                .unwrap();
            ids.push((who, m.id));
        }
        for (who, id) in ids {
            let got = r
                .api
                .list_clips(&r.token, r.channel.id, None, None, Some(who))
                .await
                .unwrap();
            assert_eq!(got.iter().map(|m| m.id).collect::<Vec<_>>(), vec![id]);
        }
    }

    /// A server that answers every request with this status and JSON body (then closes).
    async fn canned(status: u16, message: &str) -> Api {
        canned_code(status, "x", message).await
    }

    async fn canned_code(status: u16, code: &str, message: &str) -> Api {
        let body = serde_json::json!({ "code": code, "message": message }).to_string();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 16 * 1024];
                    let _ = sock.read(&mut buf).await;
                    let resp = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Api::new(&format!("http://{addr}"))
    }

    #[tokio::test]
    async fn every_clip_call_carries_the_servers_words_for_429_and_503() {
        use pulse_protocol::rest::ConfirmClipRequest;
        let ch = ChannelId::new();
        let sv = ServerId::new();
        let msg = (ClipUploadId::new(), MessageId::new());
        for (status, words) in [
            (429u16, "Slow down, clip."),
            (503u16, "Clips storage isn't set up on this server."),
        ] {
            let api = canned(status, words).await;
            let t = "t";
            let said = |e: ApiError| {
                assert!(
                    matches!(&e, ApiError::Told { status: s, .. } if *s == status),
                    "{e:?}"
                );
                assert_eq!(e.to_string(), words);
                // a 503 means nothing was done: safe to ask again; a 429 is not "never arrived"
                assert_eq!(e.never_arrived(), status == 503);
                assert!(crate::outbox::retryable(&e));
            };
            said(api.add_game(t, sv, "x").await.unwrap_err());
            said(
                api.clip_upload(
                    t,
                    ch,
                    &ClipUploadRequest {
                        size: 1,
                        duration_ms: 1,
                    },
                )
                .await
                .unwrap_err(),
            );
            said(api.cancel_clip_upload(t, msg.0).await.unwrap_err());
            said(
                api.confirm_clip(
                    t,
                    msg.0,
                    &ConfirmClipRequest {
                        caption: String::new(),
                        game_id: None,
                        duration_ms: 1,
                        width: 16,
                        height: 16,
                        thumb_hash: "0".repeat(64),
                        nonce: None,
                    },
                )
                .await
                .unwrap_err(),
            );
            said(api.upload_clip_thumb(t, unique_png()).await.unwrap_err());
            said(
                api.edit_clip(
                    t,
                    msg.1,
                    &EditClipRequest {
                        caption: String::new(),
                        game_id: None,
                    },
                )
                .await
                .unwrap_err(),
            );
            said(api.clip_url(t, msg.1).await.unwrap_err());
            said(api.list_clips(t, ch, None, None, None).await.unwrap_err());
            said(api.clip_facets(t, ch).await.unwrap_err());
            said(api.set_clip_view(t, ch, ClipView::List).await.unwrap_err());
            said(api.clip_storage(t, sv).await.unwrap_err());
        }
        // not Pulse's body (a proxy page): the generic words, as before
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut b = vec![0u8; 4096];
            let _ = sock.read(&mut b).await;
            let _ = sock
                .write_all(b"HTTP/1.1 503 X\r\ncontent-length: 4\r\nconnection: close\r\n\r\nnope")
                .await;
        });
        let e = Api::new(&format!("http://{addr}"))
            .clip_storage("t", sv)
            .await
            .unwrap_err();
        assert!(
            matches!(e, ApiError::Server(_)) && e.never_arrived(),
            "{e:?}"
        );
    }
    #[tokio::test]
    async fn a_confirm_whose_upload_is_gone_says_so() {
        use pulse_protocol::rest::ConfirmClipRequest;
        let body = ConfirmClipRequest {
            caption: String::new(),
            game_id: None,
            duration_ms: 1,
            width: 16,
            height: 16,
            thumb_hash: "0".repeat(64),
            nonce: None,
        };
        for (status, words, gone) in [
            (404u16, "not found", true),
            (409, "That upload expired. Try again.", true),
            (400, "That game isn't in this server.", false),
            (403, "you can't do that", false),
        ] {
            let api = canned(status, words).await;
            let e = api
                .confirm_clip("t", ClipUploadId::new(), &body)
                .await
                .unwrap_err();
            assert_eq!(
                matches!(e, ApiError::UploadGone(_)),
                gone,
                "{status}: {e:?}"
            );
            assert_eq!(e.to_string(), words);
        }
        // the poster, not the upload, is what's missing
        let api = canned_code(
            409,
            "thumb_missing",
            "The preview image is missing. Try again.",
        )
        .await;
        let e = api
            .confirm_clip("t", ClipUploadId::new(), &body)
            .await
            .unwrap_err();
        assert!(matches!(e, ApiError::PosterGone(_)), "{e:?}");
        assert_eq!(e.to_string(), "The preview image is missing. Try again.");
    }
}
