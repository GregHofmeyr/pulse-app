//! Test harness: a real server on an ephemeral port with a fresh temp-file SQLite.

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
    _dir: tempfile::TempDir,
}

impl TestApp {
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
    pub fn ws_url(&self, path: &str) -> String {
        format!("ws://{}{}", self.addr, path)
    }
}

pub fn test_config() -> Config {
    Config {
        db_url: String::new(),
        bind: "127.0.0.1:0".parse().unwrap(),
        livekit_url: "ws://localhost:7880".into(),
        livekit_key: "devkey".into(),
        livekit_secret: "secret-secret-secret-secret-secret".into(),
        hello_timeout: Duration::from_millis(300),
        heartbeat_timeout: Duration::from_secs(5),
    }
}

pub async fn spawn() -> TestApp {
    spawn_with(test_config()).await
}

pub async fn spawn_with(mut cfg: Config) -> TestApp {
    let dir = tempfile::tempdir().unwrap();
    cfg.db_url = format!("sqlite://{}", dir.path().join("test.db").display());
    let db = db::connect(&cfg.db_url).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let cfg = Arc::new(cfg);
    let hub = Hub::default();
    let state = AppState {
        db: db.clone(),
        cfg: cfg.clone(),
        hub: hub.clone(),
    };
    tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
    TestApp {
        addr,
        http: reqwest::Client::new(),
        db,
        cfg,
        hub,
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
