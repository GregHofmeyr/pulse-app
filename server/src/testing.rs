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
    let state = AppState {
        db: db.clone(),
        cfg: cfg.clone(),
        hub: Hub::default(),
    };
    tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
    TestApp {
        addr,
        http: reqwest::Client::new(),
        db,
        cfg,
        _dir: dir,
    }
}
