//! Live connection to the server: Hello → Ready → events, heartbeats that must be answered in time, reconnect with backoff.

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use serde::Serialize;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as Ws;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use tokio::time::Instant;

use crate::backoff::reconnect_delay;

const HEARTBEAT: Duration = Duration::from_secs(15);
const READY_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Backstop: no frame at all for this long = dead, whatever the ack bookkeeping says.
const DEAD_AFTER: Duration = Duration::from_secs(75);
/// Least time a heartbeat gets for its ack; a slow link gets 3× its measured round trip.
const ACK_MIN: Duration = Duration::from_secs(5);
/// A probe (window focused, network back) must be answered within this.
const PROBE_ACK: Duration = Duration::from_secs(3);
/// Up at least this long after Ready = a healthy session: if it drops, retry at once.
const HEALTHY_AFTER: Duration = Duration::from_secs(10);

/// Timings (production by default; tests shrink them).
#[derive(Debug, Clone, Copy)]
pub struct Tuning {
    /// Scales the backoff schedule (1 s in production).
    pub unit: Duration,
    pub heartbeat: Duration,
    pub dead_after: Duration,
    pub ack_min: Duration,
    pub probe_ack: Duration,
    pub healthy_after: Duration,
}

/// Frames sent right after every Ready: state the server keeps only in memory (our mute/deafen),
/// which a server restart or a dropped offline send would otherwise lose.
pub type AfterReady = std::sync::Arc<dyn Fn() -> Vec<ClientFrame> + Send + Sync>;

impl Default for Tuning {
    fn default() -> Self {
        Self {
            unit: Duration::from_secs(1),
            heartbeat: HEARTBEAT,
            dead_after: DEAD_AFTER,
            ack_min: ACK_MIN,
            probe_ack: PROBE_ACK,
            healthy_after: HEALTHY_AFTER,
        }
    }
}

/// Whether the server still answers: every heartbeat (or probe) must be answered in time. Any
/// frame from the server counts as an answer; an ack also measures the round trip.
#[derive(Debug)]
struct Liveness {
    ack_min: Duration,
    /// When the oldest unanswered heartbeat went out.
    sent_at: Option<Instant>,
    /// Dead unless the server says something by then.
    due: Option<Instant>,
    rtt: Option<Duration>,
}

impl Liveness {
    fn new(ack_min: Duration) -> Self {
        Self {
            ack_min,
            sent_at: None,
            due: None,
            rtt: None,
        }
    }

    /// How long a regular heartbeat may wait for its ack.
    fn ack_wait(&self) -> Duration {
        self.rtt.map_or(self.ack_min, |r| (r * 3).max(self.ack_min))
    }

    /// How long a probe may wait: `probe_ack`, or 3 round trips on a link slower than that.
    /// (Not `ack_wait`: its floor is above `probe_ack`, which would make every probe as slow
    /// as a heartbeat.)
    fn probe_wait(&self, probe_ack: Duration) -> Duration {
        self.rtt.map_or(probe_ack, |r| (r * 3).max(probe_ack))
    }

    /// A heartbeat went out at `now`; it must be answered within `wait` (a probe can only shorten
    /// an outstanding deadline, never extend it).
    fn sent(&mut self, now: Instant, wait: Duration) {
        if self.due.is_none() {
            self.sent_at = Some(now);
        }
        let due = now + wait;
        self.due = Some(self.due.map_or(due, |d| d.min(due)));
    }

    fn heard(&mut self, now: Instant, ack: bool) {
        self.due = None;
        if ack && let Some(at) = self.sent_at.take() {
            self.rtt = Some(now - at);
        }
    }
}

/// Sleeps until `at`, or forever when there's nothing to wait for.
async fn until(at: Option<Instant>) {
    match at {
        Some(t) => tokio::time::sleep_until(t).await,
        None => std::future::pending().await,
    }
}

#[derive(Debug, Clone)]
pub enum GatewayUpdate {
    Ready(Box<Ready>),
    Event(Box<Event>),
    Connection(ConnState),
}

/// How long startup keeps retrying a stored session before saying the server is unreachable
/// (the network can come up a few seconds after an autostarted app).
pub const BOOT_GRACE: Duration = Duration::from_secs(10);

/// What the first gateway session says about a stored login. Startup no longer asks `/me`
/// first: Ready carries the user, a 4001 close means the token is dead.
#[derive(Debug, Clone)]
pub enum Boot {
    /// No verdict yet. `failing_since`: when the first attempt before Ready failed.
    Waiting {
        failing_since: Option<std::time::Instant>,
        grace: Duration,
    },
    Ready(pulse_protocol::rest::User),
    LoggedOut,
    UpdateRequired,
    /// The server turned us away (too many devices / attempts). It's reachable and we stay
    /// signed in: the gateway keeps retrying and the app opens on Ready.
    Refused(ConnState),
    /// Attempts before Ready kept failing for the whole grace period.
    Unreachable,
}

impl Boot {
    pub fn starting() -> Self {
        Self::starting_with(BOOT_GRACE)
    }

    pub fn starting_with(grace: Duration) -> Self {
        Boot::Waiting {
            failing_since: None,
            grace,
        }
    }

    pub fn is_waiting(&self) -> bool {
        matches!(self, Boot::Waiting { .. })
    }

    /// Fold in one update; once decided it stays (later reconnects aren't startup).
    pub fn observe(&mut self, u: &GatewayUpdate, now: std::time::Instant) {
        let Boot::Waiting {
            failing_since,
            grace,
        } = self
        else {
            return;
        };
        *self = match u {
            GatewayUpdate::Ready(r) => Boot::Ready(r.me.clone()),
            GatewayUpdate::Connection(ConnState::LoggedOut) => Boot::LoggedOut,
            GatewayUpdate::Connection(ConnState::UpdateRequired) => Boot::UpdateRequired,
            GatewayUpdate::Connection(
                c @ (ConnState::TooManyDevices | ConnState::RateLimited(_)),
            ) => Boot::Refused(*c),
            GatewayUpdate::Connection(ConnState::Reconnecting) => {
                let since = *failing_since.get_or_insert(now);
                if now.saturating_duration_since(since) < *grace {
                    return; // keep retrying (the gateway's own backoff paces it)
                }
                Boot::Unreachable
            }
            _ => return,
        };
    }
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ConnState {
    Connecting,
    Connected,
    Reconnecting,
    /// Refused with 4006: this account is at its device cap. Still retrying.
    TooManyDevices,
    /// Refused with HTTP 429 at the upgrade: seconds until the next attempt. Still retrying.
    RateLimited(u64),
    LoggedOut,
    /// The server is newer than this app: stopped until it's updated.
    UpdateRequired,
}

enum Cmd {
    Send(ClientFrame),
    ReconnectNow,
    /// Window focused / network back: prove the connection now (or stop waiting out a backoff).
    Probe,
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
    /// The server is newer than us: stop until the app is updated.
    UpdateRequired,
    /// Anything else: retry, including 4006 (over the per-user connection cap) and an HTTP 429
    /// at the upgrade (per-IP gateway limit).
    Retry(Went),
    /// Retry, but the server said why it refused us (shown to the user instead of "Reconnecting").
    Refused(Refusal),
    Stop,
}

/// Why the gateway turned a connection away (both are retried with the normal backoff).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// Closed with 4006: over the per-user connection cap.
    TooManyDevices,
    /// HTTP 429 at the upgrade: per-IP attempt limit.
    RateLimited,
}

/// How far a session that's to be retried got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Went {
    /// Never reached Ready.
    Failed,
    /// Reached Ready but dropped within `healthy_after`.
    Brief,
    /// Was up for a while: the first retry is immediate.
    Healthy,
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
        Self::spawn_tuning(
            base_url,
            token,
            tx,
            Tuning {
                unit,
                ..Tuning::default()
            },
        )
    }

    /// `dead_after`: silence after which the connection is presumed dead (tests shorten it).
    pub fn spawn_tuned(
        base_url: String,
        token: String,
        tx: mpsc::UnboundedSender<GatewayUpdate>,
        unit: Duration,
        dead_after: Duration,
    ) -> Self {
        Self::spawn_tuning(
            base_url,
            token,
            tx,
            Tuning {
                unit,
                dead_after,
                ..Tuning::default()
            },
        )
    }

    pub fn spawn_tuning(
        base_url: String,
        token: String,
        tx: mpsc::UnboundedSender<GatewayUpdate>,
        t: Tuning,
    ) -> Self {
        Self::spawn_full(
            base_url,
            token,
            tx,
            t,
            std::sync::Arc::new(Vec::<ClientFrame>::new),
        )
    }

    pub fn spawn_full(
        base_url: String,
        token: String,
        tx: mpsc::UnboundedSender<GatewayUpdate>,
        t: Tuning,
        after_ready: AfterReady,
    ) -> Self {
        let (cmd, mut cmd_rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let url = ws_url(&base_url);
            // attempts since the last session that reached Ready; whether that one was healthy
            let (mut n, mut after_healthy) = (0u32, false);
            let _ = tx.send(GatewayUpdate::Connection(ConnState::Connecting));
            loop {
                let (went, refusal) =
                    match session(&url, &token, &tx, &mut cmd_rx, &t, &after_ready).await {
                        End::Stop => return,
                        End::LoggedOut => {
                            let _ = tx.send(GatewayUpdate::Connection(ConnState::LoggedOut));
                            return;
                        }
                        End::UpdateRequired => {
                            let _ = tx.send(GatewayUpdate::Connection(ConnState::UpdateRequired));
                            return;
                        }
                        End::Retry(went) => (went, None),
                        End::Refused(r) => (Went::Failed, Some(r)),
                    };
                if went != Went::Failed {
                    n = 0;
                    after_healthy = went == Went::Healthy;
                }
                let delay = reconnect_delay(n, after_healthy, rand::random::<f64>())
                    .mul_f64(t.unit.as_secs_f64());
                let state = match refusal {
                    None => ConnState::Reconnecting,
                    Some(Refusal::TooManyDevices) => ConnState::TooManyDevices,
                    // the real wait, rounded up so it never reads "0s"
                    Some(Refusal::RateLimited) => {
                        ConnState::RateLimited(delay.as_secs_f64().ceil().max(1.0) as u64)
                    }
                };
                let _ = tx.send(GatewayUpdate::Connection(state));
                n = n.saturating_add(1);
                // Wait out the backoff, unless told to reconnect now / probe, or stop.
                let sleep = tokio::time::sleep(delay);
                tokio::pin!(sleep);
                loop {
                    tokio::select! {
                        _ = &mut sleep => break,
                        c = cmd_rx.recv() => match c {
                            None | Some(Cmd::Stop) => return,
                            Some(Cmd::ReconnectNow | Cmd::Probe) => break,
                            Some(Cmd::Send(_)) => {} // dropped while offline
                        },
                    }
                }
                // a countdown ("retrying in 8s") must not outlive the wait it counted down
                if matches!(state, ConnState::RateLimited(_)) {
                    let _ = tx.send(GatewayUpdate::Connection(ConnState::Connecting));
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

    /// Check the connection now (window focused, network back): a dead one is noticed within
    /// seconds; while offline, retry at once.
    pub fn probe(&self) {
        let _ = self.cmd.send(Cmd::Probe);
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
    t: &Tuning,
    after_ready: &AfterReady,
) -> End {
    let mut ws =
        match tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(url)).await {
            Ok(Ok((ws, _))) => ws,
            Ok(Err(tokio_tungstenite::tungstenite::Error::Http(resp)))
                if resp.status()
                    == tokio_tungstenite::tungstenite::http::StatusCode::TOO_MANY_REQUESTS =>
            {
                return End::Refused(Refusal::RateLimited);
            }
            _ => return End::Retry(Went::Failed),
        };
    crate::startup::mark("gateway_open");
    let hello = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .expect("serialize");
    if ws.send(Ws::text(hello)).await.is_err() {
        return End::Retry(Went::Failed);
    }

    // First frame must be Ready (or a close).
    match tokio::time::timeout(READY_TIMEOUT, next_frame(&mut ws)).await {
        Ok(Frame::Server(ServerFrame::Ready(r))) => {
            if let Some(ms) = crate::startup::first("ready") {
                // Ready's size decides whether it's worth compressing (slow-start round trips)
                let bytes = serde_json::to_string(&r).map_or(0, |s| s.len());
                tracing::info!(phase = "ready", since_start_ms = ms, bytes, "startup");
            }
            let _ = tx.send(GatewayUpdate::Ready(Box::new(r)));
            let _ = tx.send(GatewayUpdate::Connection(ConnState::Connected));
            // what the server only keeps in memory (our mute/deafen): say it again on every Ready
            for f in after_ready() {
                let f = serde_json::to_string(&f).expect("serialize");
                if ws.send(Ws::text(f)).await.is_err() {
                    return End::Retry(Went::Brief);
                }
            }
        }
        Ok(Frame::Closed(Some(CloseCode::Library(4001)))) => return End::LoggedOut,
        Ok(Frame::Closed(Some(CloseCode::Library(4005)))) => return End::UpdateRequired,
        Ok(Frame::Closed(Some(CloseCode::Library(4006)))) => {
            return End::Refused(Refusal::TooManyDevices);
        }
        _ => return End::Retry(Went::Failed),
    }

    let ready_at = Instant::now();
    // lost after Ready: a healthy session (retry at once) or a brief one (back off)
    let lost = || {
        End::Retry(if ready_at.elapsed() >= t.healthy_after {
            Went::Healthy
        } else {
            Went::Brief
        })
    };
    let heartbeat = serde_json::to_string(&ClientFrame::Heartbeat).expect("serialize");
    let mut beat = tokio::time::interval(t.heartbeat);
    beat.tick().await; // first tick is immediate
    // Only frames *from* the server prove the connection is alive (writes succeed into a dead socket).
    let mut deadline = Instant::now() + t.dead_after;
    let mut live = Liveness::new(t.ack_min);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => return lost(),
            _ = until(live.due) => {
                tracing::info!(rtt = ?live.rtt, "gateway: no answer in time; reconnecting");
                return lost();
            }
            _ = beat.tick() => {
                if ws.send(Ws::text(heartbeat.clone())).await.is_err() { return lost() }
                live.sent(Instant::now(), live.ack_wait());
            }
            frame = next_frame(&mut ws) => {
                let now = Instant::now();
                deadline = now + t.dead_after;
                live.heard(now, matches!(frame, Frame::Server(ServerFrame::HeartbeatAck)));
                match frame {
                    Frame::Server(ServerFrame::Event(e)) => { let _ = tx.send(GatewayUpdate::Event(Box::new(e))); }
                    Frame::Server(ServerFrame::Ready(r)) => { let _ = tx.send(GatewayUpdate::Ready(Box::new(r))); }
                    Frame::Server(ServerFrame::HeartbeatAck) => {}
                    Frame::Closed(Some(CloseCode::Library(4001))) => return End::LoggedOut,
                    Frame::Closed(Some(CloseCode::Library(4005))) => return End::UpdateRequired,
                    Frame::Closed(_) => return lost(),
                }
            }
            c = cmd_rx.recv() => match c {
                None | Some(Cmd::Stop) => { let _ = ws.close(None).await; return End::Stop }
                Some(Cmd::ReconnectNow) => return lost(),
                Some(Cmd::Probe) => {
                    if ws.send(Ws::text(heartbeat.clone())).await.is_err() { return lost() }
                    live.sent(Instant::now(), live.probe_wait(t.probe_ack));
                }
                Some(Cmd::Send(f)) => {
                    let f = serde_json::to_string(&f).expect("serialize");
                    if ws.send(Ws::text(f)).await.is_err() { return lost() }
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
            if let GatewayUpdate::Event(e) = next(&mut rx).await
                && let Event::MessageCreated { message, .. } = *e
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

    /// Collect connection states for `dur`; panics on anything terminal or a Ready.
    async fn retrying_states(
        rx: &mut mpsc::UnboundedReceiver<GatewayUpdate>,
        dur: Duration,
    ) -> Vec<ConnState> {
        let mut states = vec![];
        let until = tokio::time::Instant::now() + dur;
        while let Ok(Some(u)) = tokio::time::timeout_at(until, rx.recv()).await {
            match u {
                GatewayUpdate::Connection(s) => {
                    assert!(
                        !matches!(s, ConnState::LoggedOut | ConnState::UpdateRequired),
                        "a refused connect is not terminal: {s:?}"
                    );
                    states.push(s);
                }
                GatewayUpdate::Ready(_) => panic!("Ready while refused"),
                GatewayUpdate::Event(_) => {}
            }
        }
        states
    }

    /// 4006 (over the per-user connection cap) is retried with backoff, never a logout.
    #[tokio::test]
    async fn too_many_connections_is_retried_not_logged_out() {
        use futures::{SinkExt, StreamExt};
        let mut cfg = testing::test_config();
        cfg.limits.gateway_per_user = 1;
        let app = testing::spawn_with(cfg).await;
        let (me, token) = testing::register(&app, "alex").await;
        // another device holds the only slot
        let (mut hog, _) = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
            .await
            .unwrap();
        let hello = serde_json::to_string(&ClientFrame::Hello {
            token: token.clone(),
            client_version: pulse_protocol::PROTOCOL_VERSION,
        })
        .unwrap();
        hog.send(Ws::text(hello)).await.unwrap();
        while let Some(Ok(m)) = hog.next().await {
            if matches!(m, Ws::Text(t) if t.contains("Ready")) {
                break;
            }
        }
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _gw = GatewayHandle::spawn_with(
            format!("http://{}", app.addr),
            token,
            tx,
            Duration::from_millis(20),
        );
        let states = retrying_states(&mut rx, Duration::from_millis(600)).await;
        let retries = states
            .iter()
            .filter(|s| **s == ConnState::TooManyDevices)
            .count();
        assert!(retries >= 2, "kept retrying, saying why: {states:?}");
        assert!(!states.contains(&ConnState::Reconnecting), "{states:?}");
        assert_eq!(app.hub.connections_for(me), 1, "the hog was never kicked");
        // the slot frees: the next retry gets in
        drop(hog);
        next_ready(&mut rx).await;
    }

    /// HTTP 429 at the upgrade (per-IP gateway limit) is retried with backoff, never a logout.
    #[tokio::test]
    async fn rate_limited_upgrade_is_retried_not_logged_out() {
        let mut cfg = testing::test_config();
        cfg.limits.gateway_ip = pulse_server::limits::Rate::new(1, 0.0);
        let app = testing::spawn_with(cfg).await;
        let (_, token) = testing::register(&app, "alex").await;
        // an abandoned attempt (no Hello) keeps the address's only token
        let (abandoned, _) = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
            .await
            .unwrap();
        drop(abandoned);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _gw = GatewayHandle::spawn_with(
            format!("http://{}", app.addr),
            token,
            tx,
            Duration::from_millis(20),
        );
        let states = retrying_states(&mut rx, Duration::from_millis(600)).await;
        let retries = states
            .iter()
            .filter(|s| matches!(s, ConnState::RateLimited(_)))
            .count();
        assert!(retries >= 2, "kept retrying, saying why: {states:?}");
        assert!(!states.contains(&ConnState::Reconnecting), "{states:?}");
        // the bucket refills: the next retry gets in
        app.limits.gateway_ip.refund("127.0.0.1");
        next_ready(&mut rx).await;
    }

    /// The 429 message carries the real wait: the first backoff step (~1 s) at the 1 s unit.
    #[tokio::test]
    async fn rate_limited_reports_the_real_delay() {
        let mut cfg = testing::test_config();
        cfg.limits.gateway_ip = pulse_server::limits::Rate::new(1, 0.0);
        let app = testing::spawn_with(cfg).await;
        let (_, token) = testing::register(&app, "alex").await;
        let (abandoned, _) = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
            .await
            .unwrap();
        drop(abandoned);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _gw = GatewayHandle::spawn(format!("http://{}", app.addr), token, tx);
        let secs = loop {
            if let GatewayUpdate::Connection(ConnState::RateLimited(s)) = next(&mut rx).await {
                break s;
            }
        };
        assert!(
            (1..=2).contains(&secs),
            "first backoff is 0.8-1.2 s: {secs}"
        );
    }

    #[test]
    fn refusals_serialize_for_the_ui() {
        assert_eq!(
            serde_json::to_string(&ConnState::TooManyDevices).unwrap(),
            "\"too_many_devices\""
        );
        assert_eq!(
            serde_json::to_string(&ConnState::RateLimited(7)).unwrap(),
            "{\"rate_limited\":7}"
        );
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

    /// I1: a half-open connection (Wi-Fi drop, suspend) must be noticed: no frames for the
    /// deadline → reconnect, even though writes still "succeed".
    #[tokio::test]
    async fn silent_server_is_detected_as_dead() {
        let addr = silent_server().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _gw = GatewayHandle::spawn_tuned(
            format!("http://{addr}"),
            "t".into(),
            tx,
            Duration::from_millis(50),
            Duration::from_millis(400),
        );
        next_ready(&mut rx).await;
        let saw = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(GatewayUpdate::Connection(ConnState::Reconnecting)) = rx.recv().await {
                    return true;
                }
            }
        })
        .await;
        assert_eq!(saw, Ok(true), "dead connection not detected");
    }

    #[tokio::test]
    async fn update_required_stops_reconnecting() {
        use futures::StreamExt;
        use tokio_tungstenite::tungstenite::protocol::CloseFrame;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
                    let _hello = ws.next().await;
                    let _ = ws
                        .close(Some(CloseFrame {
                            code: CloseCode::Library(4005),
                            reason: "update required".into(),
                        }))
                        .await;
                });
            }
        });
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _gw = GatewayHandle::spawn_with(
            format!("http://{addr}"),
            "t".into(),
            tx,
            Duration::from_millis(20),
        );
        let mut states = vec![];
        while let Ok(Some(GatewayUpdate::Connection(s))) =
            tokio::time::timeout(Duration::from_millis(500), rx.recv()).await
        {
            states.push(s);
        }
        assert_eq!(
            states,
            vec![ConnState::Connecting, ConnState::UpdateRequired]
        );
    }

    async fn boot_outcome(base: String, token: &str, grace: Duration) -> Boot {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _gw = GatewayHandle::spawn_with(base, token.into(), tx, Duration::from_millis(50));
        let mut boot = Boot::starting_with(grace);
        while boot.is_waiting() {
            let u = next(&mut rx).await;
            boot.observe(&u, std::time::Instant::now());
        }
        boot
    }

    /// Startup no longer asks `/me`: Ready says who we are.
    #[tokio::test]
    async fn startup_learns_who_we_are_from_ready() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        match boot_outcome(
            format!("http://{}", app.addr),
            &token,
            Duration::from_secs(10),
        )
        .await
        {
            Boot::Ready(me) => assert_eq!(me.username, "alex"),
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    /// A stored token the server no longer accepts → the login screen, as with `/me`'s 401.
    #[tokio::test]
    async fn startup_with_a_dead_token_ends_logged_out() {
        let app = testing::spawn().await;
        let b = boot_outcome(
            format!("http://{}", app.addr),
            "garbage",
            Duration::from_secs(10),
        )
        .await;
        assert!(matches!(b, Boot::LoggedOut), "{b:?}");
    }

    /// Nothing answers: say so (the UI shows "couldn't restore your last session").
    #[tokio::test]
    async fn startup_with_no_server_is_unreachable() {
        let b = boot_outcome("http://localhost:9".into(), "t", Duration::from_millis(300)).await;
        assert!(matches!(b, Boot::Unreachable), "{b:?}");
    }

    #[test]
    fn a_decided_boot_stays_decided() {
        let mut b = Boot::LoggedOut;
        b.observe(
            &GatewayUpdate::Connection(ConnState::Reconnecting),
            std::time::Instant::now(),
        );
        assert!(
            matches!(b, Boot::LoggedOut),
            "a later reconnect isn't startup"
        );
    }

    /// A TCP front that refuses (accepts and drops) for `down`, then forwards to `upstream`:
    /// the network coming up a moment after the app started.
    async fn late_network(upstream: std::net::SocketAddr, down: Duration) -> String {
        let l = tokio::net::TcpListener::bind("localhost:0").await.unwrap();
        let base = format!("http://localhost:{}", l.local_addr().unwrap().port());
        let up_at = tokio::time::Instant::now() + down;
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                if tokio::time::Instant::now() < up_at {
                    drop(s); // no network yet
                    continue;
                }
                tokio::spawn(async move {
                    if let Ok(mut u) = tokio::net::TcpStream::connect(upstream).await {
                        let _ = tokio::io::copy_bidirectional(&mut s, &mut u).await;
                    }
                });
            }
        });
        base
    }

    #[tokio::test]
    async fn startup_survives_a_late_network() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let base = late_network(app.addr, Duration::from_secs(1)).await;
        match boot_outcome(base, &token, Duration::from_secs(5)).await {
            Boot::Ready(me) => assert_eq!(me.username, "alex"),
            other => panic!("gave up on a network that came back after 1 s: {other:?}"),
        }
    }

    #[test]
    fn failures_within_the_grace_keep_waiting_then_give_up() {
        let t0 = std::time::Instant::now();
        let failed = GatewayUpdate::Connection(ConnState::Reconnecting);
        let mut b = Boot::starting_with(Duration::from_secs(10));
        b.observe(&failed, t0);
        b.observe(&failed, t0 + Duration::from_secs(9));
        assert!(b.is_waiting(), "{b:?}");
        b.observe(&failed, t0 + Duration::from_secs(10));
        assert!(matches!(b, Boot::Unreachable), "{b:?}");
    }

    #[test]
    fn a_refusal_decides_startup_at_once() {
        let t0 = std::time::Instant::now();
        for c in [ConnState::TooManyDevices, ConnState::RateLimited(3)] {
            let mut b = Boot::starting_with(Duration::from_secs(10));
            b.observe(&GatewayUpdate::Connection(c), t0);
            assert!(matches!(b, Boot::Refused(r) if r == c), "{b:?}");
        }
    }

    /// Startup against a full account says so (not "unreachable"), and the gateway keeps going.
    #[tokio::test]
    async fn startup_over_the_device_cap_is_refused_not_unreachable() {
        let mut cfg = testing::test_config();
        cfg.limits.gateway_per_user = 0;
        let app = testing::spawn_with(cfg).await;
        let (_, token) = testing::register(&app, "alex").await;
        let b = boot_outcome(
            format!("http://{}", app.addr),
            &token,
            Duration::from_secs(10),
        )
        .await;
        assert!(
            matches!(b, Boot::Refused(ConnState::TooManyDevices)),
            "{b:?}"
        );
    }

    /// Serves Ready on `listener`, then never says anything again (never acks, never closes): a
    /// half-open socket.
    fn silent_server_on(listener: tokio::net::TcpListener) {
        use futures::{SinkExt, StreamExt};
        use pulse_protocol::gateway::{Ready, ServerFrame};
        use pulse_protocol::rest::User;
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
                    let _hello = ws.next().await;
                    let ready = ServerFrame::Ready(Ready {
                        me: User {
                            id: pulse_protocol::ids::UserId::new(),
                            username: "alex".into(),
                            avatar_hash: None,
                            avatar_still: None,
                            style: Default::default(),
                        },
                        servers: vec![],
                        channels: vec![],
                        members: vec![],
                        dm_members: vec![],
                        voice: vec![],
                        people: vec![],
                        read_states: vec![],
                        mutes: vec![],
                        hidden: vec![],
                        latest: vec![],
                        streams: vec![],
                        custom_emoji: vec![],
                        frequent_emoji: vec![],
                        qoty: vec![],
                        speaker_links: vec![],
                        thread_seen: vec![],
                        games: vec![],
                        clip_views: vec![],
                    });
                    ws.send(tokio_tungstenite::tungstenite::Message::text(
                        serde_json::to_string(&ready).unwrap(),
                    ))
                    .await
                    .unwrap();
                    // keep reading (so writes succeed) but never answer
                    while ws.next().await.is_some() {}
                });
            }
        });
    }

    async fn silent_server() -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        silent_server_on(listener);
        addr
    }

    async fn wait_for(ok: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !ok() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("condition never held");
    }

    /// R6: what the server keeps only in memory (our mute/deafen) is declared again after every
    /// Ready, so a server restart or a reconnect can't show us unmuted while we're muted.
    #[tokio::test]
    async fn after_ready_frames_are_sent_on_every_connect() {
        use pulse_protocol::gateway::VoiceFlags;
        let app = testing::spawn().await;
        let (me, token) = testing::register(&app, "alex").await;
        let muted = VoiceFlags {
            muted: true,
            deafened: false,
        };
        let after: AfterReady =
            std::sync::Arc::new(move || vec![ClientFrame::VoiceState { flags: muted }]);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _gw = GatewayHandle::spawn_full(
            format!("http://{}", app.addr),
            token,
            tx,
            quick(Duration::from_secs(60)),
            after,
        );
        next_ready(&mut rx).await;
        wait_for(|| app.voice.declared(me).muted).await;
        // a restart forgets what we declared; the reconnect must say it again
        app.voice.set_flags(me, VoiceFlags::default());
        app.hub.close_all(1012);
        next_ready(&mut rx).await;
        wait_for(|| app.voice.declared(me).muted).await;
    }

    fn quick(heartbeat: Duration) -> Tuning {
        Tuning {
            unit: Duration::from_millis(50),
            heartbeat,
            dead_after: Duration::from_secs(60), // the old backstop: far beyond these tests
            ack_min: Duration::from_millis(200),
            probe_ack: Duration::from_millis(200),
            healthy_after: Duration::from_secs(10),
        }
    }

    async fn reconnecting_within(
        rx: &mut mpsc::UnboundedReceiver<GatewayUpdate>,
        limit: Duration,
    ) -> bool {
        tokio::time::timeout(limit, async {
            loop {
                match rx.recv().await {
                    Some(GatewayUpdate::Connection(ConnState::Reconnecting)) => return,
                    None => std::future::pending::<()>().await,
                    _ => {}
                }
            }
        })
        .await
        .is_ok()
    }

    /// A3: an unanswered heartbeat ends the session within its ack deadline, long before the
    /// no-frames backstop (60 s here).
    #[tokio::test]
    async fn unanswered_heartbeat_is_noticed_within_the_ack_deadline() {
        let addr = silent_server().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _gw = GatewayHandle::spawn_tuning(
            format!("http://{addr}"),
            "t".into(),
            tx,
            quick(Duration::from_millis(100)),
        );
        next_ready(&mut rx).await;
        let started = std::time::Instant::now();
        assert!(
            reconnecting_within(&mut rx, Duration::from_secs(2)).await,
            "dead connection not noticed"
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
    }

    /// A3: window focus / network back probes a connection that still *looks* fine. A dead one is
    /// noticed within the probe's deadline, even with heartbeats a minute apart.
    #[tokio::test]
    async fn a_probe_notices_a_dead_connection_at_once() {
        let addr = silent_server().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let gw = GatewayHandle::spawn_tuning(
            format!("http://{addr}"),
            "t".into(),
            tx,
            quick(Duration::from_secs(60)),
        );
        next_ready(&mut rx).await;
        gw.probe();
        assert!(reconnecting_within(&mut rx, Duration::from_secs(1)).await);
    }

    #[tokio::test]
    async fn a_probe_leaves_a_live_connection_alone() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let gw = GatewayHandle::spawn_tuning(
            format!("http://{}", app.addr),
            token,
            tx,
            quick(Duration::from_secs(60)),
        );
        next_ready(&mut rx).await;
        gw.probe();
        assert!(
            !reconnecting_within(&mut rx, Duration::from_millis(800)).await,
            "a healthy connection was dropped by a probe"
        );
    }

    /// A3: a session that was fine reconnects straight away after a blip; the backoff (1 s here,
    /// as in production) only starts if that first try fails.
    #[tokio::test]
    async fn first_retry_after_a_healthy_session_is_immediate() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let t = Tuning {
            healthy_after: Duration::ZERO, // any session counts as healthy
            ..Tuning::default()
        };
        let _gw = GatewayHandle::spawn_tuning(format!("http://{}", app.addr), token, tx, t);
        next_ready(&mut rx).await;
        app.hub.close_all(1012);
        loop {
            if let GatewayUpdate::Connection(ConnState::Reconnecting) = next(&mut rx).await {
                break;
            }
        }
        let lost = std::time::Instant::now();
        next_ready(&mut rx).await;
        assert!(
            lost.elapsed() < Duration::from_millis(500),
            "waited {:?} before reconnecting",
            lost.elapsed()
        );
    }

    /// Resume from sleep: the network isn't back yet, so the first tries fail and the backoff
    /// grows. When the network returns, the `online` / focus probe reconnects at once instead of
    /// waiting the backoff out. The schedule is stretched (×10: 8-12 s, then 16-24 s) so "at once"
    /// and "waited it out" stay far apart even on a loaded machine; the first wait is probed
    /// through as well, so the test needn't sit it out.
    #[tokio::test]
    async fn a_probe_cuts_a_grown_backoff_short() {
        // a free port with nobody listening yet: connects are refused
        let addr = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        let gw = GatewayHandle::spawn_tuning(
            format!("http://{addr}"),
            "t".into(),
            tx,
            Tuning {
                unit: Duration::from_secs(10),
                ..Tuning::default()
            },
        );
        // two failed tries (the second one probed out of the first wait): now waiting out the
        // grown, 16-24 s step
        let mut failures = 0;
        while failures < 2 {
            if let GatewayUpdate::Connection(ConnState::Reconnecting) = next(&mut rx).await {
                failures += 1;
                if failures == 1 {
                    gw.probe();
                }
            }
        }
        silent_server_on(tokio::net::TcpListener::bind(addr).await.unwrap()); // network back
        let t = std::time::Instant::now();
        gw.probe();
        next_ready(&mut rx).await;
        assert!(
            t.elapsed() < Duration::from_secs(4),
            "waited {:?}",
            t.elapsed()
        );
    }

    #[test]
    fn ack_wait_is_three_round_trips_but_never_under_the_floor() {
        let mut l = Liveness::new(Duration::from_secs(5));
        assert_eq!(l.ack_wait(), Duration::from_secs(5));
        let t0 = Instant::now();
        l.sent(t0, l.ack_wait());
        l.heard(t0 + Duration::from_secs(2), true); // a 2 s round trip (a very slow link)
        assert_eq!(l.ack_wait(), Duration::from_secs(6));
        assert_eq!(l.due, None);
    }

    #[test]
    fn a_probe_shortens_the_wait_and_any_frame_answers_it() {
        let mut l = Liveness::new(Duration::from_secs(5));
        let t0 = Instant::now();
        l.sent(t0, Duration::from_secs(5));
        l.sent(t0 + Duration::from_secs(1), Duration::from_secs(3)); // probe 1 s later
        assert_eq!(l.due, Some(t0 + Duration::from_secs(4)));
        l.heard(t0 + Duration::from_millis(1500), false); // an event: alive
        assert_eq!(l.due, None);
        assert_eq!(l.rtt, None, "only an ack measures the round trip");
    }

    /// A probe on a link slower than its own deadline (alt-tab on a >1 s RTT link) must not
    /// drop a live connection: it gets 3 round trips when that's longer than the probe deadline.
    #[test]
    fn a_probe_waits_at_least_three_round_trips() {
        let probe = Duration::from_secs(3);
        let mut l = Liveness::new(Duration::from_secs(5));
        assert_eq!(l.probe_wait(probe), probe, "no round trip measured yet");
        let t0 = Instant::now();
        l.sent(t0, l.ack_wait());
        l.heard(t0 + Duration::from_millis(500), true);
        assert_eq!(
            l.probe_wait(probe),
            probe,
            "a quick link keeps the short deadline"
        );
        l.sent(t0, l.ack_wait());
        l.heard(t0 + Duration::from_secs(2), true); // a 2 s round trip
        assert_eq!(l.probe_wait(probe), Duration::from_secs(6));
    }
}
