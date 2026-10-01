//! `GET /gateway`: Hello → Ready → events, with heartbeats.

use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message as Ws, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use pulse_protocol::gateway::{ClientFrame, DmMembers, Event, Ready, ServerFrame, ServerMembers};
use pulse_protocol::ids::UserId;
use sqlx::SqlitePool;

use crate::AppState;
use crate::access::channel_for;
use crate::auth::routes::load_user;
use crate::auth::session;
use crate::error::AppResult;
use crate::servers::routes::{all_servers, dms_of, server_channels, server_members};

pub const CLOSE_UNAUTHORIZED: u16 = 4001;
pub const CLOSE_TIMEOUT: u16 = 4002;
/// A socket write that cannot complete in this long belongs to a dead or stalled client.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn handler(ws: WebSocketUpgrade, State(s): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| run(socket, s))
}

async fn close(mut socket: WebSocket, code: u16, reason: &'static str) {
    let _ = socket
        .send(Ws::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

async fn send(socket: &mut WebSocket, frame: &ServerFrame) -> bool {
    let Ok(text) = serde_json::to_string(frame) else {
        return false;
    };
    matches!(
        tokio::time::timeout(SEND_TIMEOUT, socket.send(Ws::Text(text.into()))).await,
        Ok(Ok(()))
    )
}

async fn authenticate(socket: &mut WebSocket, s: &AppState) -> Option<UserId> {
    let first = tokio::time::timeout(s.cfg.hello_timeout, socket.recv())
        .await
        .ok()??
        .ok()?;
    let Ws::Text(text) = first else { return None };
    let ClientFrame::Hello { token } = serde_json::from_str(&text).ok()? else {
        return None;
    };
    session::authenticate(&s.db, &token).await.ok()?
}

pub async fn build_ready(db: &SqlitePool, me: UserId) -> AppResult<Ready> {
    let servers = all_servers(db).await?;
    let mut channels = vec![];
    let mut members = vec![];
    for srv in &servers {
        channels.extend(server_channels(db, srv.id).await?);
        members.push(ServerMembers {
            server_id: srv.id,
            members: server_members(db, srv.id).await?,
        });
    }
    let mut dm_members = vec![];
    for dm in dms_of(db, me).await? {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT user_id FROM channel_members WHERE channel_id = ? ORDER BY user_id",
        )
        .bind(dm.id.to_string())
        .fetch_all(db)
        .await?;
        dm_members.push(DmMembers {
            channel_id: dm.id,
            user_ids: ids
                .iter()
                .map(|i| i.parse())
                .collect::<Result<_, _>>()
                .map_err(anyhow::Error::from)?,
        });
        channels.push(dm);
    }
    Ok(Ready {
        me: load_user(db, me).await?,
        servers,
        channels,
        members,
        dm_members,
    })
}

async fn run(mut socket: WebSocket, s: AppState) {
    let Some(me) = authenticate(&mut socket, &s).await else {
        return close(socket, CLOSE_UNAUTHORIZED, "unauthorized").await;
    };
    // Register before building Ready so no event between the snapshot and the stream is lost.
    let (conn, mut rx) = s.hub.register(me);
    let ready = match build_ready(&s.db, me).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = ?e, "build_ready failed");
            s.hub.unregister(conn);
            return;
        }
    };
    if !send(&mut socket, &ServerFrame::Ready(ready)).await {
        s.hub.unregister(conn);
        return;
    }

    loop {
        tokio::select! {
            out = rx.recv() => {
                // None: the hub dropped us (too slow) — just end.
                let Some(frame) = out else { break };
                if !send(&mut socket, &frame).await { break }
            }
            inc = tokio::time::timeout(s.cfg.heartbeat_timeout, socket.recv()) => {
                let msg = match inc {
                    Err(_) => { s.hub.unregister(conn); return close(socket, CLOSE_TIMEOUT, "heartbeat timeout").await }
                    Ok(None) | Ok(Some(Err(_))) => break,
                    Ok(Some(Ok(m))) => m,
                };
                let Ws::Text(text) = msg else {
                    if matches!(msg, Ws::Close(_)) { break }
                    continue;
                };
                match serde_json::from_str::<ClientFrame>(&text) {
                    Ok(ClientFrame::Heartbeat) => {
                        if !send(&mut socket, &ServerFrame::HeartbeatAck).await { break }
                    }
                    Ok(ClientFrame::Typing { channel_id }) => {
                        // Typing into a channel you cannot see is silently ignored.
                        if channel_for(&s.db, me, channel_id).await.is_ok() {
                            s.hub.publish(&s.db, Event::Typing { channel_id, user_id: me }).await;
                        }
                    }
                    Ok(ClientFrame::Hello { .. }) | Err(_) => {}
                }
            }
        }
    }
    s.hub.unregister(conn);
}
