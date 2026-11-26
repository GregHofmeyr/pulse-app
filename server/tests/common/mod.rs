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
    /// The child's stdout and stderr.
    log: tempfile::NamedTempFile,
}

impl ServerProcess {
    /// What the server has printed so far.
    pub fn log(&self) -> String {
        std::fs::read_to_string(self.log.path()).unwrap_or_default()
    }

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
        .env_remove("PULSE_CLIPS_S3_ENDPOINT")
        .env_remove("PULSE_CLIPS_S3_BUCKET")
        .env_remove("PULSE_CLIPS_S3_KEY_ID")
        .env_remove("PULSE_CLIPS_S3_SECRET")
        .env_remove("PULSE_CLIPS_S3_REGION")
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
/// The server binds port 0 and the port is read back from its "listening" line, so no other
/// process can take it between picking and binding. Its output goes to a file, shown if it fails
/// to start.
pub async fn spawn_binary_on(
    db_url: String,
    env: &[(&str, &str)],
    dir: Option<tempfile::TempDir>,
) -> ServerProcess {
    let log = tempfile::NamedTempFile::new().unwrap();
    // One handle for both streams, so they share an offset instead of overwriting each other.
    let out = log.reopen().unwrap();
    let child = binary(&db_url, ANY_PORT.parse().unwrap())
        // The "listening" line is info, from our own crate.
        .env("RUST_LOG", "warn,pulse_server=info")
        .envs(env.iter().copied())
        .arg("serve")
        .stdout(out.try_clone().unwrap())
        .stderr(out)
        .spawn()
        .unwrap();
    let mut p = ServerProcess {
        child,
        addr: ANY_PORT.parse().unwrap(),
        db_url,
        _dir: dir,
        log,
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    let still_starting = |p: &mut ServerProcess| {
        if let Some(status) = p.child.try_wait().unwrap() {
            panic!("server exited early: {status}\n{}", p.log());
        }
        assert!(
            Instant::now() < deadline,
            "server didn't come up in 15 s\n{}",
            p.log()
        );
    };
    p.addr = loop {
        if let Some(addr) = listening_addr(&p.log()) {
            break addr;
        }
        still_starting(&mut p);
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let http = reqwest::Client::new();
    loop {
        if let Ok(r) = http.get(p.url("/health")).send().await
            && r.status() == 200
        {
            break;
        }
        still_starting(&mut p);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    p
}

/// Let the OS pick the port when the server binds.
const ANY_PORT: &str = "127.0.0.1:0";

/// The address in the server's "pulse-app-server listening addr=127.0.0.1:<port>" line. Only the
/// digits after the host are read, so colour codes around the field name don't matter.
fn listening_addr(log: &str) -> Option<std::net::SocketAddr> {
    const HOST: &str = "127.0.0.1:";
    let line = log
        .lines()
        .find(|l| l.contains("pulse-app-server listening"))?;
    let port: String = line[line.find(HOST)? + HOST.len()..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    format!("{HOST}{port}").parse().ok()
}

/// Run the binary with `args` until it exits; returns (exit success, stderr + stdout). Panics if
/// it is still running after `timeout` (a server that should have refused to start, say).
pub fn run_binary(
    db_url: &str,
    env: &[(&str, &str)],
    args: &[&str],
    timeout: Duration,
) -> (bool, String) {
    let mut child = binary(db_url, ANY_PORT.parse().unwrap())
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

/// A test's gateway socket.
pub type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The next frame, or `None` once the socket has been quiet for `quiet` (or is gone). It heartbeats
/// first, as a real client does, so a socket idling across several drains outlives the server's
/// heartbeat timeout (5 s in tests); the acks are skipped.
pub async fn next_frame_beating(
    ws: &mut Socket,
    quiet: Duration,
) -> Option<pulse_protocol::gateway::ServerFrame> {
    use futures::{SinkExt, StreamExt};
    use pulse_protocol::gateway::{ClientFrame, ServerFrame};
    use tokio_tungstenite::tungstenite::Message as Ws;
    let beat = serde_json::to_string(&ClientFrame::Heartbeat).unwrap();
    ws.send(Ws::text(beat)).await.ok()?;
    loop {
        let m = tokio::time::timeout(quiet, ws.next()).await.ok()??.ok()?;
        if let Ws::Text(t) = m {
            match serde_json::from_str(&t).unwrap() {
                ServerFrame::HeartbeatAck => continue,
                frame => return Some(frame),
            }
        }
    }
}
