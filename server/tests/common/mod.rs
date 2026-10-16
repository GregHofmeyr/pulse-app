#![allow(dead_code)]
#[allow(unused_imports)] // shutdown.rs only needs the real-binary harness below
pub use pulse_server::testing::*;

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The real `pulse-app-server` binary on a free port with a fresh database, for what the
/// in-process harness can't cover (signals, startup). Killed on drop.
pub struct ServerProcess {
    pub child: std::process::Child,
    pub addr: std::net::SocketAddr,
    pub db_url: String,
    _dir: tempfile::TempDir,
}

impl ServerProcess {
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
    pub fn ws_url(&self, path: &str) -> String {
        format!("ws://{}{}", self.addr, path)
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn binary(db_url: &str, bind: std::net::SocketAddr) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_pulse-app-server"));
    c.env("PULSE_DB_URL", db_url)
        .env("PULSE_BIND", bind.to_string())
        .env("PULSE_LIVEKIT_KEY", "devkey")
        .env("PULSE_LIVEKIT_SECRET", "secret-secret-secret-secret-secret")
        // Nothing listens on port 1: LiveKit is "down" (ignored until S4 reads it).
        .env("PULSE_LIVEKIT_API_URL", "http://127.0.0.1:1")
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    c
}

pub async fn spawn_binary() -> ServerProcess {
    let dir = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}", dir.path().join("pulse.db").display());
    // Bind :0 to find a free port, then release it for the child.
    let addr = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let child = binary(&db_url, addr).arg("serve").spawn().unwrap();
    let http = reqwest::Client::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(r) = http.get(format!("http://{addr}/health")).send().await
            && r.status() == 200
        {
            break;
        }
        assert!(Instant::now() < deadline, "server didn't come up in 15 s");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    ServerProcess {
        child,
        addr,
        db_url,
        _dir: dir,
    }
}

/// A single-use invite from the binary's own `create-invite`, against the same database.
pub fn binary_invite(p: &ServerProcess) -> String {
    let out = binary(&p.db_url, p.addr)
        .arg("create-invite")
        .stdout(Stdio::piped())
        .output()
        .unwrap();
    assert!(out.status.success(), "create-invite failed");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}
