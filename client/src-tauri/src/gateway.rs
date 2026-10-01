//! Live connection to the server: Hello → Ready → events, heartbeat, reconnect with backoff.

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use serde::Serialize;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as Ws;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use crate::backoff::backoff_delay;

const HEARTBEAT: Duration = Duration::from_secs(30);
const READY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub enum GatewayUpdate {
    Ready(Box<Ready>),
    Event(Event),
    Connection(ConnState),
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ConnState {
    Connecting,
    Connected,
    Reconnecting,
    LoggedOut,
}

enum Cmd {
    Send(ClientFrame),
    ReconnectNow,
    Stop,
}

/// Handle to the background connection task. Dropping it stops the task.
pub struct GatewayHandle {
    cmd: mpsc::UnboundedSender<Cmd>,
}

impl Drop for GatewayHandle {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Stop);
    }
}

pub fn ws_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    let ws = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    format!("{ws}/gateway")
}

/// How a session ended.
enum End {
    /// Server rejected our token: stop for good.
    LoggedOut,
    /// Anything else: retry. `true` if we got as far as Ready (resets backoff).
    Retry(bool),
    Stop,
}

impl GatewayHandle {
    pub fn spawn(
        base_url: String,
        token: String,
        tx: mpsc::UnboundedSender<GatewayUpdate>,
    ) -> Self {
        Self::spawn_with(base_url, token, tx, Duration::from_secs(1))
    }

    /// `unit` scales the backoff schedule (1 s in production; small in tests).
    pub fn spawn_with(
        base_url: String,
        token: String,
        tx: mpsc::UnboundedSender<GatewayUpdate>,
        unit: Duration,
    ) -> Self {
        let (cmd, mut cmd_rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let url = ws_url(&base_url);
            let mut attempt: u32 = 0;
            let _ = tx.send(GatewayUpdate::Connection(ConnState::Connecting));
            loop {
                match session(&url, &token, &tx, &mut cmd_rx).await {
                    End::Stop => return,
                    End::LoggedOut => {
                        let _ = tx.send(GatewayUpdate::Connection(ConnState::LoggedOut));
                        return;
                    }
                    End::Retry(was_ready) => {
                        if was_ready {
                            attempt = 0;
                        }
                        let _ = tx.send(GatewayUpdate::Connection(ConnState::Reconnecting));
                        let delay = backoff_delay(attempt, rand::random::<f64>())
                            .mul_f64(unit.as_secs_f64());
                        attempt = attempt.saturating_add(1);
                        // Wait out the backoff, unless told to reconnect now or stop.
                        let sleep = tokio::time::sleep(delay);
                        tokio::pin!(sleep);
                        loop {
                            tokio::select! {
                                _ = &mut sleep => break,
                                c = cmd_rx.recv() => match c {
                                    None | Some(Cmd::Stop) => return,
                                    Some(Cmd::ReconnectNow) => break,
                                    Some(Cmd::Send(_)) => {} // dropped while offline
                                },
                            }
                        }
                    }
                }
            }
        });
        Self { cmd }
    }

    pub fn send(&self, f: ClientFrame) {
        let _ = self.cmd.send(Cmd::Send(f));
    }

    pub fn reconnect_now(&self) {
        let _ = self.cmd.send(Cmd::ReconnectNow);
    }

    pub fn stop(&self) {
        let _ = self.cmd.send(Cmd::Stop);
    }
}

async fn session(
    url: &str,
    token: &str,
    tx: &mpsc::UnboundedSender<GatewayUpdate>,
    cmd_rx: &mut mpsc::UnboundedReceiver<Cmd>,
) -> End {
    let Ok((mut ws, _)) = tokio_tungstenite::connect_async(url).await else {
        return End::Retry(false);
    };
    let hello = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
    })
    .expect("serialize");
    if ws.send(Ws::text(hello)).await.is_err() {
        return End::Retry(false);
    }

    // First frame must be Ready (or a close).
    match tokio::time::timeout(READY_TIMEOUT, next_frame(&mut ws)).await {
        Ok(Frame::Server(ServerFrame::Ready(r))) => {
            let _ = tx.send(GatewayUpdate::Ready(Box::new(r)));
            let _ = tx.send(GatewayUpdate::Connection(ConnState::Connected));
        }
        Ok(Frame::Closed(Some(CloseCode::Library(4001)))) => return End::LoggedOut,
        _ => return End::Retry(false),
    }

    let mut beat = tokio::time::interval(HEARTBEAT);
    beat.tick().await; // first tick is immediate
    loop {
        tokio::select! {
            _ = beat.tick() => {
                let f = serde_json::to_string(&ClientFrame::Heartbeat).expect("serialize");
                if ws.send(Ws::text(f)).await.is_err() { return End::Retry(true) }
            }
            frame = next_frame(&mut ws) => match frame {
                Frame::Server(ServerFrame::Event(e)) => { let _ = tx.send(GatewayUpdate::Event(e)); }
                Frame::Server(ServerFrame::Ready(r)) => { let _ = tx.send(GatewayUpdate::Ready(Box::new(r))); }
                Frame::Server(ServerFrame::HeartbeatAck) => {}
                Frame::Closed(Some(CloseCode::Library(4001))) => return End::LoggedOut,
                Frame::Closed(_) => return End::Retry(true),
            },
            c = cmd_rx.recv() => match c {
                None | Some(Cmd::Stop) => { let _ = ws.close(None).await; return End::Stop }
                Some(Cmd::ReconnectNow) => return End::Retry(true),
                Some(Cmd::Send(f)) => {
                    let f = serde_json::to_string(&f).expect("serialize");
                    if ws.send(Ws::text(f)).await.is_err() { return End::Retry(true) }
                }
            },
        }
    }
}

// Short-lived (one per read); boxing would just add an allocation per frame.
#[allow(clippy::large_enum_variant)]
enum Frame {
    Server(ServerFrame),
    Closed(Option<CloseCode>),
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn next_frame(ws: &mut Socket) -> Frame {
    loop {
        match ws.next().await {
            Some(Ok(Ws::Text(t))) => match serde_json::from_str::<ServerFrame>(&t) {
                Ok(f) => return Frame::Server(f),
                Err(_) => continue, // unknown frame from a newer server: ignore
            },
            Some(Ok(Ws::Close(f))) => return Frame::Closed(f.map(|f| f.code)),
            Some(Ok(_)) => continue,
            Some(Err(_)) | None => return Frame::Closed(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_protocol::gateway::Event;
    use pulse_server::testing;
    use tokio::sync::mpsc;

    async fn next(rx: &mut mpsc::UnboundedReceiver<GatewayUpdate>) -> GatewayUpdate {
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out")
            .expect("closed")
    }

    async fn next_ready(
        rx: &mut mpsc::UnboundedReceiver<GatewayUpdate>,
    ) -> pulse_protocol::gateway::Ready {
        loop {
            if let GatewayUpdate::Ready(r) = next(rx).await {
                return *r;
            }
        }
    }

    #[tokio::test]
    async fn connects_and_receives_ready_then_events() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let g = testing::general(&app, &token, s.id).await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _gw = GatewayHandle::spawn_with(
            format!("http://{}", app.addr),
            token.clone(),
            tx,
            Duration::from_millis(50),
        );
        let ready = next_ready(&mut rx).await;
        assert_eq!(ready.servers.len(), 1);
        let sent = testing::send(&app, &token, g.id, "hi").await;
        loop {
            if let GatewayUpdate::Event(Event::MessageCreated { message, .. }) = next(&mut rx).await
            {
                assert_eq!(message.id, sent.id);
                break;
            }
        }
    }

    #[tokio::test]
    async fn bad_token_reports_logged_out_and_stops() {
        let app = testing::spawn().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _gw = GatewayHandle::spawn_with(
            format!("http://{}", app.addr),
            "garbage".into(),
            tx,
            Duration::from_millis(50),
        );
        loop {
            match next(&mut rx).await {
                GatewayUpdate::Connection(ConnState::LoggedOut) => break,
                GatewayUpdate::Ready(_) => panic!("garbage token got Ready"),
                _ => {}
            }
        }
        // no retries afterwards
        let more = tokio::time::timeout(Duration::from_millis(800), rx.recv()).await;
        assert!(
            matches!(more, Err(_) | Ok(None)),
            "unexpected retry: {more:?}"
        );
    }

    #[tokio::test]
    async fn reconnects_after_server_restart() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _gw = GatewayHandle::spawn_with(
            format!("http://{}", app.addr),
            token,
            tx,
            Duration::from_millis(50),
        );
        next_ready(&mut rx).await;
        app.hub.close_all(1012);
        let mut saw_reconnecting = false;
        loop {
            match next(&mut rx).await {
                GatewayUpdate::Connection(ConnState::Reconnecting) => saw_reconnecting = true,
                GatewayUpdate::Ready(_) => break,
                _ => {}
            }
        }
        assert!(saw_reconnecting);
    }

    #[test]
    fn ws_url_from_http_base() {
        assert_eq!(
            ws_url("http://localhost:7890"),
            "ws://localhost:7890/gateway"
        );
        assert_eq!(
            ws_url("https://pulse.example.com/"),
            "wss://pulse.example.com/gateway"
        );
    }
}
