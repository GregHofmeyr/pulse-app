//! Stopping the server: `docker stop` sends SIGTERM, Ctrl-C sends SIGINT. Both must close every
//! gateway socket with 1012 ("server restarting") and exit 0 promptly, not wait for a SIGKILL.
#![cfg(unix)]

mod common;

use std::time::{Duration, Instant};

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::ClientFrame;
use pulse_protocol::rest::SessionResponse;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connected_socket(p: &ServerProcess) -> Socket {
    let s: SessionResponse = reqwest::Client::new()
        .post(p.url("/auth/register"))
        .json(&json!({"invite_code": binary_invite(p), "username": "alex", "password": "hunter2hunter2"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mut ws = tokio_tungstenite::connect_async(p.ws_url("/gateway"))
        .await
        .unwrap()
        .0;
    ws.send(Ws::text(
        serde_json::to_string(&ClientFrame::Hello {
            token: s.token,
            client_version: pulse_protocol::PROTOCOL_VERSION,
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    // Bounded, and a socket that ends before Ready fails the test instead of spinning on `None`.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match ws.next().await {
                Some(Ok(Ws::Text(t))) if t.contains("\"Ready\"") => return,
                Some(Ok(_)) => continue,
                other => panic!("gateway ended before Ready: {other:?}\n{}", p.log()),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no Ready within 10 s\n{}", p.log()));
    ws
}

async fn stops_cleanly_on(signal: &str) {
    let mut p = spawn_binary().await;
    let mut ws = connected_socket(&p).await;
    let sent_at = Instant::now();
    let killed = std::process::Command::new("kill")
        .args([signal, &p.child.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());

    let code = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(m) = ws.next().await {
            match m {
                Ok(Ws::Close(f)) => return f.map(|f| f.code),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
        None
    })
    .await
    .ok()
    .flatten();
    assert_eq!(
        code,
        Some(CloseCode::Restart),
        "{signal}: clients must be told the server is restarting"
    );

    let status = loop {
        if let Some(s) = p.child.try_wait().unwrap() {
            break s;
        }
        assert!(
            sent_at.elapsed() < Duration::from_secs(5),
            "{signal}: still running 5 s after the signal"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(status.success(), "{signal}: exit status {status:?}");
}

#[tokio::test]
async fn sigterm_closes_sockets_with_1012_and_exits_0() {
    stops_cleanly_on("-TERM").await;
}

#[tokio::test]
async fn sigint_closes_sockets_with_1012_and_exits_0() {
    stops_cleanly_on("-INT").await;
}
