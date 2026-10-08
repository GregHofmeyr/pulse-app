//! `GET /gateway`: Hello → Ready → events, with heartbeats.

use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message as Ws, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use pulse_protocol::gateway::{
    ClientFrame, DmMembers, Event, Ready, ServerFrame, ServerMembers, VoiceMember, VoiceRoom,
};
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{Person, User};
use sqlx::SqlitePool;

use crate::AppState;
use crate::access::channel_for;
use crate::auth::routes::load_user;
use crate::auth::session;
use crate::db::now;
use crate::error::AppResult;
use crate::servers::routes::{all_servers, dms_of, server_channels, server_members};
use crate::voice::VoiceState;

pub use super::hub::{CLOSE_TOO_SLOW, CLOSE_UNAUTHORIZED};
use super::hub::{ConnId, Hub};
pub const CLOSE_TIMEOUT: u16 = 4002;
/// Pre- and post-auth client frames are tiny; cap them so nobody can make us buffer megabytes.
const MAX_CLIENT_FRAME: usize = 64 * 1024;
/// A socket write that cannot complete in this long belongs to a dead or stalled client.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn handler(ws: WebSocketUpgrade, State(s): State<AppState>) -> Response {
    ws.max_message_size(MAX_CLIENT_FRAME)
        .max_frame_size(MAX_CLIENT_FRAME)
        .on_upgrade(move |socket| run(socket, s))
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

enum Auth {
    Ok(UserId, String),
    Denied,
    Internal,
}

async fn authenticate(socket: &mut WebSocket, s: &AppState) -> Auth {
    let first = match tokio::time::timeout(s.cfg.hello_timeout, socket.recv()).await {
        Ok(Some(Ok(m))) => m,
        _ => return Auth::Denied,
    };
    let Ws::Text(text) = first else {
        return Auth::Denied;
    };
    let Ok(ClientFrame::Hello { token }) = serde_json::from_str(&text) else {
        return Auth::Denied;
    };
    match session::authenticate(&s.db, &token).await {
        Ok(Some(user)) => Auth::Ok(user, session::hash_token(&token)),
        Ok(None) => Auth::Denied,
        // Don't tell the client to re-auth (and wipe its keychain) because our DB hiccuped.
        Err(_) => Auth::Internal,
    }
}

/// Every exit path of a connection ends here: unregister and, if it was the user's last
/// connection, record last-seen and announce they went offline.
async fn disconnected(s: &AppState, conn: ConnId, me: UserId) {
    let last = match s.hub.unregister(conn) {
        Some((_, last)) => last,
        None => !s.hub.is_online(me), // the hub already dropped it (logout, too slow, shutdown)
    };
    if !last {
        return;
    }
    let at = now();
    let _ = sqlx::query("UPDATE users SET last_seen_at = ? WHERE id = ?")
        .bind(&at)
        .bind(me.to_string())
        .execute(&s.db)
        .await;
    s.hub
        .publish(
            &s.db,
            Event::PresenceChanged {
                user_id: me,
                online: false,
                last_seen_at: Some(at),
            },
        )
        .await;
}

/// Everyone with an account, online if they have any live connection.
pub async fn people(db: &SqlitePool, hub: &Hub) -> AppResult<Vec<Person>> {
    let rows: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id, username, avatar_hash, last_seen_at FROM users ORDER BY username",
    )
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|(id, username, avatar_hash, last_seen_at)| {
            let id: UserId = id.parse().map_err(anyhow::Error::from)?;
            Ok(Person {
                online: hub.is_online(id),
                user: User {
                    id,
                    username,
                    avatar_hash,
                },
                last_seen_at,
            })
        })
        .collect()
}

pub async fn build_ready(
    db: &SqlitePool,
    voice_state: &VoiceState,
    hub: &Hub,
    me: UserId,
) -> AppResult<Ready> {
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
    let mut voice = vec![];
    for (channel_id, members) in voice_state.rooms() {
        // Same rule as everything else: private rooms only for their members.
        if channel_for(db, me, channel_id).await.is_ok() {
            voice.push(VoiceRoom {
                channel_id,
                members: members
                    .into_iter()
                    .map(|(user_id, flags)| VoiceMember { user_id, flags })
                    .collect(),
            });
        }
    }
    Ok(Ready {
        me: load_user(db, me).await?,
        servers,
        channels,
        members,
        dm_members,
        voice,
        people: people(db, hub).await?,
        read_states: vec![],
        mutes: vec![],
        hidden: vec![],
        latest: vec![],
    })
}

async fn run(mut socket: WebSocket, s: AppState) {
    let (me, token_hash) = match authenticate(&mut socket, &s).await {
        Auth::Ok(u, h) => (u, h),
        Auth::Denied => return close(socket, CLOSE_UNAUTHORIZED, "unauthorized").await,
        Auth::Internal => return close(socket, 1011, "internal error").await,
    };
    // Register before building Ready so no event between the snapshot and the stream is lost.
    let reg = s.hub.register(me, token_hash);
    let (conn, mut rx, mut kick, first) = (reg.id, reg.rx, reg.kick, reg.first);
    let ready = match build_ready(&s.db, &s.voice, &s.hub, me).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = ?e, "build_ready failed");
            disconnected(&s, conn, me).await;
            return;
        }
    };
    if !send(&mut socket, &ServerFrame::Ready(ready)).await {
        disconnected(&s, conn, me).await;
        return;
    }
    if first {
        s.hub
            .publish(
                &s.db,
                Event::PresenceChanged {
                    user_id: me,
                    online: true,
                    last_seen_at: None,
                },
            )
            .await;
    }

    // Only inbound frames prove the client is alive; outbound traffic must not extend the deadline.
    let mut deadline = tokio::time::Instant::now() + s.cfg.heartbeat_timeout;
    loop {
        tokio::select! {
            // A kick closes the queue too; check it first so the client gets the close code.
            biased;
            code = &mut kick => {
                disconnected(&s, conn, me).await;
                let reason = match code { Ok(CLOSE_TOO_SLOW) => "too slow", Ok(1012) => "server restarting", _ => "session ended" };
                return close(socket, code.unwrap_or(CLOSE_UNAUTHORIZED), reason).await;
            }
            out = rx.recv() => {
                let Some(frame) = out else { break };
                if !send(&mut socket, &frame).await { break }
            }
            _ = tokio::time::sleep_until(deadline) => {
                disconnected(&s, conn, me).await;
                return close(socket, CLOSE_TIMEOUT, "heartbeat timeout").await;
            }
            inc = socket.recv() => {
                let msg = match inc {
                    None | Some(Err(_)) => break,
                    Some(Ok(m)) => m,
                };
                deadline = tokio::time::Instant::now() + s.cfg.heartbeat_timeout;
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
                    Ok(ClientFrame::VoiceState { flags }) => {
                        // Only meaningful while in a room; otherwise ignored.
                        if let Some(channel_id) = s.voice.set_flags(me, flags) {
                            s.hub.publish(&s.db, Event::VoiceStateChanged { channel_id, user_id: me, flags }).await;
                        }
                    }
                    Ok(ClientFrame::Hello { .. }) | Err(_) => {}
                }
            }
        }
    }
    disconnected(&s, conn, me).await;
}
