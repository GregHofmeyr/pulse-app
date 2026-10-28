//! Test harness: a real server on an ephemeral port with a fresh temp-file SQLite.

pub mod fake_s3;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use sqlx::SqlitePool;

use crate::config::Config;
use crate::{AppState, db, gateway::Hub, router};

pub struct TestApp {
    pub addr: SocketAddr,
    pub http: reqwest::Client,
    pub db: SqlitePool,
    pub cfg: Arc<Config>,
    pub hub: Hub,
    pub voice: crate::voice::VoiceState,
    pub streams: crate::streams::state::StreamState,
    pub limits: Arc<crate::limits::Limits>,
    _dir: tempfile::TempDir,
}

impl TestApp {
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
    pub fn ws_url(&self, path: &str) -> String {
        format!("ws://{}{}", self.addr, path)
    }
    pub fn voice_members(
        &self,
        channel: pulse_protocol::ids::ChannelId,
    ) -> std::collections::HashSet<pulse_protocol::ids::UserId> {
        self.voice.members(channel)
    }
    /// The shared state the test server runs on, for calling startup tasks directly.
    pub fn state(&self) -> AppState {
        AppState {
            db: self.db.clone(),
            cfg: self.cfg.clone(),
            hub: self.hub.clone(),
            voice: self.voice.clone(),
            streams: self.streams.clone(),
            limits: self.limits.clone(),
        }
    }
}

pub fn test_config() -> Config {
    Config {
        db_url: String::new(),
        bind: "127.0.0.1:0".parse().unwrap(),
        livekit_url: "ws://localhost:7880".into(),
        livekit_api_url: "http://127.0.0.1:1".into(), // LiveKit "down" unless a test overrides it
        livekit_key: "devkey".into(),
        livekit_secret: "secret-secret-secret-secret-secret".into(),
        hello_timeout: Duration::from_millis(300),
        heartbeat_timeout: Duration::from_secs(5),
        limits: crate::limits::LimitsConfig::unlimited(),
        stream_media_timeout: Duration::from_millis(300),
        media_dir: Default::default(), // under the test's temp dir unless a test sets one
        media_min_free_bytes: 0,
        totp_key: Some(crate::auth::totp::TotpKey::parse(&"42".repeat(32)).unwrap()),
    }
}

pub async fn spawn() -> TestApp {
    spawn_with(test_config()).await
}

pub async fn spawn_with(cfg: Config) -> TestApp {
    spawn_in(cfg, tempfile::tempdir().unwrap()).await
}

/// Another server on a copy of `app`'s database as it is now, run with `cfg` (say a full disk):
/// for how a route behaves under that config on data set up through the API. The copy is
/// independent of `app`, which keeps running; its media dir starts empty.
pub async fn spawn_copy(app: &TestApp, cfg: Config) -> TestApp {
    let dir = tempfile::tempdir().unwrap();
    db::backup(&app.db, &dir.path().join("test.db"))
        .await
        .unwrap();
    spawn_in(cfg, dir).await
}

/// Serve `dir/test.db` (created if missing) with `cfg`; `dir` lives as long as the app. For a
/// test that needs the database on a particular filesystem (pass a `tempdir_in` there).
pub async fn spawn_in(mut cfg: Config, dir: tempfile::TempDir) -> TestApp {
    cfg.db_url = format!("sqlite://{}", dir.path().join("test.db").display());
    if cfg.media_dir.as_os_str().is_empty() {
        cfg.media_dir = dir.path().join("media");
    }
    let db = db::connect(&cfg.db_url).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let cfg = Arc::new(cfg);
    let hub = Hub::default();
    let voice = crate::voice::VoiceState::default();
    let streams = crate::streams::state::StreamState::default();
    let limits = Arc::new(crate::limits::Limits::new(&cfg.limits));
    let state = AppState {
        db: db.clone(),
        cfg: cfg.clone(),
        hub: hub.clone(),
        voice: voice.clone(),
        streams: streams.clone(),
        limits: limits.clone(),
    };
    tokio::spawn(async move {
        axum::serve(
            listener,
            router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    TestApp {
        addr,
        http: reqwest::Client::new(),
        db,
        cfg,
        hub,
        voice,
        streams,
        limits,
        _dir: dir,
    }
}

/// Mint an invite straight into the DB.
pub async fn invite(app: &TestApp) -> String {
    crate::auth::invites::create(&app.db, None).await.unwrap()
}

/// Register a user via the API; returns (id, token).
pub async fn register(app: &TestApp, username: &str) -> (pulse_protocol::ids::UserId, String) {
    let code = invite(app).await;
    let r = app
        .http
        .post(app.url("/auth/register"))
        .json(&serde_json::json!({"invite_code": code, "username": username, "password": "hunter2hunter2"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "register {username}");
    let s: pulse_protocol::rest::SessionResponse = r.json().await.unwrap();
    (s.user.id, s.token)
}

/// Authed JSON POST.
pub async fn post_json(
    app: &TestApp,
    token: &str,
    path: &str,
    body: serde_json::Value,
) -> reqwest::Response {
    app.http
        .post(app.url(path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

/// Authed GET returning parsed JSON (asserts 200).
pub async fn get_json<T: serde::de::DeserializeOwned>(app: &TestApp, token: &str, path: &str) -> T {
    let r = app
        .http
        .get(app.url(path))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "GET {path}");
    r.json().await.unwrap()
}

/// Create a server via the API.
pub async fn create_server(app: &TestApp, token: &str, name: &str) -> pulse_protocol::rest::Server {
    let r = post_json(app, token, "/servers", serde_json::json!({ "name": name })).await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

/// Create (or reuse) a DM/group with the given other users.
pub async fn create_dm(
    app: &TestApp,
    token: &str,
    others: &[pulse_protocol::ids::UserId],
) -> pulse_protocol::rest::Channel {
    let r = post_json(
        app,
        token,
        "/dms",
        serde_json::json!({ "user_ids": others }),
    )
    .await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

/// Send a message via the API (asserts 200).
pub async fn send(
    app: &TestApp,
    token: &str,
    channel: pulse_protocol::ids::ChannelId,
    content: &str,
) -> pulse_protocol::rest::Message {
    let r = post_json(
        app,
        token,
        &format!("/channels/{channel}/messages"),
        serde_json::json!({ "content": content }),
    )
    .await;
    assert_eq!(r.status(), 200, "send to {channel}");
    r.json().await.unwrap()
}

/// The `#general` text channel of a server.
pub async fn general(
    app: &TestApp,
    token: &str,
    server: pulse_protocol::ids::ServerId,
) -> pulse_protocol::rest::Channel {
    let chans: Vec<pulse_protocol::rest::Channel> =
        get_json(app, token, &format!("/servers/{server}/channels")).await;
    chans
        .into_iter()
        .find(|c| c.name.as_deref() == Some("general"))
        .unwrap()
}

/// Issue a LiveKit grant for `identity` (`<user>`, `<user>#v` or `<user>#a`) in `room` through
/// that user's newest session, as the voice-token and stream-ticket routes do, and return its
/// id. Tests that post LiveKit join webhooks directly put it in the participant's attributes
/// (`pulse_grant`): a join without a live session's grant is removed, not recorded.
pub async fn grant_livekit(app: &TestApp, room: &str, identity: &str) -> String {
    let user = identity.split('#').next().unwrap_or(identity);
    let id = format!("test-{}", pulse_protocol::ids::next_ulid());
    let n = sqlx::query(
        "INSERT INTO livekit_grants (id, token_hash, room, identity, issued_at)
         SELECT ?, token_hash, ?, ?, ? FROM sessions WHERE user_id = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&id)
    .bind(room)
    .bind(identity)
    .bind(db::now())
    .bind(user)
    .execute(&app.db)
    .await
    .unwrap()
    .rows_affected();
    assert_eq!(n, 1, "{identity} has no session to grant through");
    id
}
