#![allow(dead_code)]
#[allow(unused_imports)] // shutdown.rs only needs the real-binary harness below
pub use pulse_server::testing::*;

pub mod livekit;

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The real `pulse-app-server` binary on a free port with a fresh database, for what the
/// in-process harness can't cover (signals, startup). Killed on drop.
pub struct ServerProcess {
    pub child: std::process::Child,
    pub addr: std::net::SocketAddr,
    pub db_url: String,
    _dir: Option<tempfile::TempDir>,
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

pub fn binary(db_url: &str, bind: std::net::SocketAddr) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_pulse-app-server"));
    c.env("PULSE_DB_URL", db_url)
        .env("PULSE_BIND", bind.to_string())
        .env("PULSE_LIVEKIT_KEY", "devkey")
        .env("PULSE_LIVEKIT_SECRET", "secret-secret-secret-secret-secret")
        // Nothing listens on port 1: LiveKit is "down" (ignored until S4 reads it).
        .env("PULSE_LIVEKIT_API_URL", "http://127.0.0.1:1")
        .env("RUST_LOG", "warn")
        // A developer's own key must not leak into a test; tests that want one pass it.
        .env_remove("PULSE_TOTP_KEY")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    c
}

pub async fn spawn_binary() -> ServerProcess {
    let dir = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}", dir.path().join("pulse.db").display());
    spawn_binary_on(db_url, &[], Some(dir)).await
}

/// Like `spawn_binary`, on an existing database and with extra environment (e.g. `PULSE_TOTP_KEY`).
pub async fn spawn_binary_on(
    db_url: String,
    env: &[(&str, &str)],
    dir: Option<tempfile::TempDir>,
) -> ServerProcess {
    let addr = free_addr();
    let child = binary(&db_url, addr)
        .envs(env.iter().copied())
        .arg("serve")
        .spawn()
        .unwrap();
    let mut p = ServerProcess {
        child,
        addr,
        db_url,
        _dir: dir,
    };
    let http = reqwest::Client::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(r) = http.get(format!("http://{addr}/health")).send().await
            && r.status() == 200
        {
            break;
        }
        if let Some(status) = p.child.try_wait().unwrap() {
            panic!("server exited early: {status}");
        }
        assert!(Instant::now() < deadline, "server didn't come up in 15 s");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    p
}

/// Bind :0 to find a free port, then release it for the child.
fn free_addr() -> std::net::SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

/// Run the binary with `args` until it exits; returns (exit success, stderr + stdout). Panics if
/// it is still running after `timeout` (a server that should have refused to start, say).
pub fn run_binary(
    db_url: &str,
    env: &[(&str, &str)],
    args: &[&str],
    timeout: Duration,
) -> (bool, String) {
    let mut child = binary(db_url, free_addr())
        .envs(env.iter().copied())
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("`{}` still running after {timeout:?}", args.join(" "));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // Output here is a few lines, far below the pipe buffer, so reading after exit can't block.
    let out = child.wait_with_output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    (status.success(), text)
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
