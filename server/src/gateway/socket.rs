//! `GET /gateway`: Hello → Ready → events, with heartbeats.

use std::collections::HashSet;
use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message as Ws, WebSocket, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};
use pulse_protocol::gateway::{
    ClientFrame, DmMembers, Event, Ready, ServerFrame, ServerMembers, VoiceMember, VoiceRoom,
};
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{Person, User};
use sqlx::SqlitePool;

use crate::AppState;
use crate::access::channel_for;
use crate::auth::routes::load_user;
use crate::auth::session;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::limits::ClientIp;
use crate::servers::routes::{all_server_channels, all_server_members, all_servers, dms_of};
use crate::streams::state::StreamState;
use crate::voice::VoiceState;

pub use super::hub::{
    CLOSE_TOO_MANY_CONNECTIONS, CLOSE_TOO_SLOW, CLOSE_UNAUTHORIZED, CLOSE_UPDATE_REQUIRED,
};
use super::hub::{ConnId, Hub};
pub const CLOSE_TIMEOUT: u16 = 4002;
/// Pre- and post-auth client frames are tiny; cap them so nobody can make us buffer megabytes.
const MAX_CLIENT_FRAME: usize = 64 * 1024;
/// A socket write that cannot complete in this long belongs to a dead or stalled client.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn handler(
    ws: WebSocketUpgrade,
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
) -> Response {
    // Spend up front (like login) so a burst of attempts can't all slip past; the token comes
    // back once the socket authenticates, so only failed or abandoned attempts keep the spend.
    let ip_key = crate::limits::rate_key(ip).to_string();
    if !s.limits.gateway_ip.hit(&ip_key) {
        return AppError::TooManyRequests.into_response();
    }
    ws.max_message_size(MAX_CLIENT_FRAME)
        .max_frame_size(MAX_CLIENT_FRAME)
        .on_upgrade(move |socket| run(socket, s, ip_key))
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

/// Oldest client protocol this server accepts.
pub const MIN_CLIENT_VERSION: u32 = 3;

enum Auth {
    Ok(UserId, String),
    Denied,
    /// The client is older than [`MIN_CLIENT_VERSION`].
    Outdated,
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
    let Ok(ClientFrame::Hello {
        token,
        client_version,
    }) = serde_json::from_str(&text)
    else {
        return Auth::Denied;
    };
    if client_version < MIN_CLIENT_VERSION {
        return Auth::Outdated;
    }
    match session::authenticate(&s.db, &token).await {
        Ok(Some(user)) => Auth::Ok(user, session::hash_token(&token)),
        Ok(None) => Auth::Denied,
        // Don't tell the client to re-auth (and wipe its keychain) because our DB hiccuped.
        Err(_) => Auth::Internal,
    }
}

/// Every exit path of a connection ends here: unregister and, if that was the user's last
/// connection, record last-seen and announce they went offline.
async fn disconnected(s: &AppState, conn: ConnId, me: UserId) {
    s.hub.unregister(conn);
    if s.hub.is_online(me) {
        return; // still connected elsewhere
    }
    let at = now();
    let _ = sqlx::query("UPDATE users SET last_seen_at = ? WHERE id = ?")
        .bind(&at)
        .bind(me.to_string())
        .execute(&s.db)
        .await;
    s.hub.sync_presence(&s.db, me, at).await;
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
    streams: &StreamState,
    hub: &Hub,
    me: UserId,
) -> AppResult<Ready> {
    // A fixed number of queries however many servers and conversations there are: each table
    // is read once and regrouped here.
    let servers = all_servers(db).await?;
    let mut server_channels = all_server_channels(db).await?;
    let mut server_members = all_server_members(db).await?;
    let mut channels = vec![];
    let mut members = vec![];
    for srv in &servers {
        channels.extend(server_channels.remove(&srv.id).unwrap_or_default());
        members.push(ServerMembers {
            server_id: srv.id,
            members: server_members.remove(&srv.id).unwrap_or_default(),
        });
    }
    let mut conversation_members = crate::dms::routes::members_of_my_conversations(db, me).await?;
    let mut dm_members = vec![];
    for dm in dms_of(db, me).await? {
        dm_members.push(DmMembers {
            channel_id: dm.id,
            user_ids: conversation_members.remove(&dm.id).unwrap_or_default(),
        });
        channels.push(dm);
    }
    // Same rule as `channel_for`: every server channel, and the DMs/groups `me` is in, which is
    // exactly the channel list above.
    let visible: HashSet<ChannelId> = channels.iter().map(|c| c.id).collect();
    let mut voice = vec![];
    for (channel_id, members) in voice_state.rooms() {
        // Same rule as everything else: private rooms only for their members.
        if visible.contains(&channel_id) {
            voice.push(VoiceRoom {
                channel_id,
                members: members
                    .into_iter()
                    .map(|(user_id, flags)| VoiceMember { user_id, flags })
                    .collect(),
            });
        }
    }
    let mut live = vec![];
    for stream in streams.all() {
        // Same visibility rule as voice rooms.
        if visible.contains(&stream.channel_id) {
            live.push(stream);
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
        read_states: crate::reads::states_for(db, me).await?,
        mutes: crate::dms::routes::mutes_of(db, me).await?,
        hidden: crate::dms::routes::hidden_of(db, me).await?,
        latest: crate::reads::latest_for(db, me).await?,
        streams: live,
        custom_emoji: crate::emoji::for_ready(db).await?,
        frequent_emoji: crate::emoji::frequent(db, me, crate::emoji::FREQUENT).await?,
        qoty: crate::qoty::for_ready(db).await?,
        speaker_links: crate::quotes::links::for_ready(db).await?,
        thread_seen: crate::reads::thread_seen_for(db, me).await?,
        // Filled by the games and view-preference tasks.
        games: vec![],
        clip_views: vec![],
    })
}

async fn run(mut socket: WebSocket, s: AppState, ip_key: String) {
    let (me, token_hash) = match authenticate(&mut socket, &s).await {
        Auth::Ok(u, h) => {
            s.limits.gateway_ip.refund(&ip_key);
            (u, h)
        }
        Auth::Denied => return close(socket, CLOSE_UNAUTHORIZED, "unauthorized").await,
        Auth::Outdated => {
            return close(socket, CLOSE_UPDATE_REQUIRED, "update required").await;
        }
        Auth::Internal => {
            s.limits.gateway_ip.refund(&ip_key); // our fault, not the client's
            return close(socket, 1011, "internal error").await;
        }
    };
    // Register before building Ready so no event between the snapshot and the stream is lost.
    // Over the per-user cap: refuse this one, never kick an existing connection.
    let Some(reg) = s
        .hub
        .try_register(me, token_hash.clone(), s.limits.gateway_per_user)
    else {
        return close(socket, CLOSE_TOO_MANY_CONNECTIONS, "too many connections").await;
    };
    let (conn, mut rx, mut kick) = (reg.id, reg.rx, reg.kick);
    // A logout / logout-others may have landed between authenticate and try_register: its kick
    // ran before this socket was in the hub. Every revoke commits the DELETE and only then
    // kicks, so after registering either this re-check sees the row gone, or the kick (which
    // now finds this conn) fires and the loop below closes the socket with 4001.
    match session::exists(&s.db, &token_hash).await {
        Ok(true) => {}
        Ok(false) => {
            disconnected(&s, conn, me).await;
            return close(socket, CLOSE_UNAUTHORIZED, "unauthorized").await;
        }
        Err(e) => {
            tracing::error!(error = ?e, "session re-check failed");
            disconnected(&s, conn, me).await;
            return close(socket, 1011, "internal error").await;
        }
    }
    let ready = match build_ready(&s.db, &s.voice, &s.streams, &s.hub, me).await {
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
    s.hub.sync_presence(&s.db, me, now()).await;

    // Only inbound frames prove the client is alive; outbound traffic must not extend the deadline.
    let mut deadline = tokio::time::Instant::now() + s.cfg.heartbeat_timeout;
    // VoiceState publishes held back by the limit: the flags are stored at once, and one
    // trailing publish of the latest ones goes out when the bucket next allows it.
    let mut voice_pending = false;
    let mut voice_retry = tokio::time::Instant::now();
    let voice_wait = voice_retry_wait(s.cfg.limits.voice_state_user);
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
            _ = tokio::time::sleep_until(voice_retry), if voice_pending => {
                match s.voice.room_flags(me) {
                    Some((channel_id, flags)) => {
                        if s.limits.voice_state_user.hit(&me.to_string()) {
                            voice_pending = false;
                            s.hub.publish(&s.db, Event::VoiceStateChanged { channel_id, user_id: me, flags }).await;
                        } else {
                            voice_retry = tokio::time::Instant::now() + voice_wait;
                        }
                    }
                    None => voice_pending = false, // left voice; the leave event says it all
                }
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
                        // Over the limit or into a channel you cannot see: silently ignored.
                        if s.limits.typing_user.hit(&me.to_string())
                            && channel_for(&s.db, me, channel_id).await.is_ok()
                        {
                            s.hub.publish(&s.db, Event::Typing { channel_id, user_id: me }).await;
                        }
                    }
                    Ok(ClientFrame::VoiceState { flags }) => {
                        // Always stored; only a change made while in a room is published. Over the
                        // limit, the publish waits for the trailing one above.
                        if let Some(channel_id) = s.voice.set_flags(me, flags) {
                            if s.limits.voice_state_user.hit(&me.to_string()) {
                                voice_pending = false;
                                s.hub.publish(&s.db, Event::VoiceStateChanged { channel_id, user_id: me, flags }).await;
                            } else if !voice_pending {
                                voice_pending = true;
                                voice_retry = tokio::time::Instant::now() + voice_wait;
                            }
                        }
                    }
                    Ok(ClientFrame::Hello { .. }) | Err(_) => {}
                }
            }
        }
    }
    disconnected(&s, conn, me).await;
}

/// How long a held-back VoiceState publish waits before trying the bucket again: about one
/// token's refill, kept between 50 ms and 5 s.
fn voice_retry_wait(rate: crate::limits::Rate) -> std::time::Duration {
    std::time::Duration::from_secs_f64((1.0 / rate.refill_per_sec).clamp(0.05, 5.0))
}
