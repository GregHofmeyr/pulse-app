//! Screen-share routes: stream-room tickets, go live / stop, watch / unwatch. All bearer-authed,
//! and every one requires the caller to be in that server voice channel right now.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, post};
use axum::{Json, Router};
use livekit_api::access_token::{AccessToken, VideoGrants};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{
    Channel, ChannelKind, GoLiveRequest, StreamEndReason, StreamInfo, StreamRole,
    StreamTicketRequest, VoiceTokenResponse, WatchRequest,
};

use super::state::Started;
use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};

const TICKET_TTL: Duration = Duration::from_secs(10 * 60);
const HEIGHTS: [u16; 3] = [720, 1080, 1440];
const FPS: [u8; 3] = [15, 30, 60];

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/streams/{channel}/ticket", post(ticket))
        .route("/streams/{channel}/live", post(go_live))
        .route("/streams/{channel}/live", delete(stop))
        .route("/streams/{channel}/watch", post(watch))
        .route("/streams/{channel}/unwatch", post(unwatch))
}

/// Streaming exists only in server voice channels, and only for people in the call right now
/// (and in the server, same rule as joining voice).
async fn require_voice_member(s: &AppState, me: UserId, ch: ChannelId) -> AppResult<Channel> {
    let channel = channel_for(&s.db, me, ch).await?;
    let Some(server) = channel
        .server_id
        .filter(|_| channel.kind == ChannelKind::Voice)
    else {
        return Err(AppError::BadRequest(
            "streaming is only in server voice channels".into(),
        ));
    };
    if !crate::servers::routes::is_server_member(&s.db, server, me).await? {
        return Err(AppError::Forbidden);
    }
    if !s.voice.members(ch).contains(&me) {
        return Err(AppError::Forbidden);
    }
    Ok(channel)
}

async fn ticket(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(ch): Path<ChannelId>,
    Json(req): Json<StreamTicketRequest>,
) -> AppResult<Json<VoiceTokenResponse>> {
    require_voice_member(&s, me, ch).await?;
    let (suffix, source) = match req.role {
        StreamRole::V => ("v", "screen_share"),
        StreamRole::A => ("a", "screen_share_audio"),
    };
    let jwt = AccessToken::with_api_key(&s.cfg.livekit_key, &s.cfg.livekit_secret)
        .with_identity(&format!("{me}#{suffix}"))
        .with_ttl(TICKET_TTL)
        .with_grants(VideoGrants {
            room_join: true,
            room: format!("{ch}~s"),
            can_publish: Some(true),
            can_publish_sources: vec![source.into()],
            can_subscribe: Some(true),
            can_publish_data: Some(false),
            ..Default::default()
        })
        .to_jwt()
        .map_err(|e| AppError::Internal(e.into()))?;
    tracing::info!(user = %me, channel = %ch, role = suffix, "stream ticket");
    Ok(Json(VoiceTokenResponse {
        url: s.cfg.livekit_url.clone(),
        token: jwt,
    }))
}

async fn go_live(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(ch): Path<ChannelId>,
    Json(req): Json<GoLiveRequest>,
) -> AppResult<Json<StreamInfo>> {
    require_voice_member(&s, me, ch).await?;
    let st = &req.settings;
    if !HEIGHTS.contains(&st.height) || !FPS.contains(&st.fps) {
        return Err(AppError::BadRequest(
            "height must be 720, 1080 or 1440 and fps 15, 30 or 60".into(),
        ));
    }
    let (info, event) = match s.streams.start(ch, me, req.settings, now()) {
        Started::New(stream) => (stream.clone(), Event::StreamStarted { stream }),
        Started::Updated(stream) => (stream.clone(), Event::StreamUpdated { stream }),
    };
    tracing::info!(user = %me, channel = %ch, height = info.settings.height, fps = info.settings.fps, "stream live");
    s.hub.publish(&s.db, event).await;
    Ok(Json(info))
}

async fn stop(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(ch): Path<ChannelId>,
) -> AppResult<StatusCode> {
    require_voice_member(&s, me, ch).await?;
    s.streams.end(ch, me).ok_or(AppError::NotFound)?;
    tracing::info!(user = %me, channel = %ch, reason = "stopped", "stream ended");
    s.hub
        .publish(
            &s.db,
            Event::StreamEnded {
                channel_id: ch,
                user_id: me,
                reason: StreamEndReason::Stopped,
            },
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn watch(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(ch): Path<ChannelId>,
    Json(req): Json<WatchRequest>,
) -> AppResult<StatusCode> {
    require_voice_member(&s, me, ch).await?;
    ensure_watchable(&s, me, ch, req.streamer)?;
    if let Some(viewers) = s.streams.add_viewer(ch, req.streamer, me) {
        publish_viewers(&s, ch, req.streamer, viewers).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn unwatch(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(ch): Path<ChannelId>,
    Json(req): Json<WatchRequest>,
) -> AppResult<StatusCode> {
    require_voice_member(&s, me, ch).await?;
    ensure_watchable(&s, me, ch, req.streamer)?;
    if let Some(viewers) = s.streams.remove_viewer(ch, req.streamer, me) {
        publish_viewers(&s, ch, req.streamer, viewers).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

fn ensure_watchable(s: &AppState, me: UserId, ch: ChannelId, streamer: UserId) -> AppResult<()> {
    if streamer == me {
        return Err(AppError::BadRequest("you can't watch yourself".into()));
    }
    s.streams.get(ch, streamer).ok_or(AppError::NotFound)?;
    Ok(())
}

pub async fn publish_viewers(s: &AppState, ch: ChannelId, streamer: UserId, viewers: Vec<UserId>) {
    s.hub
        .publish(
            &s.db,
            Event::ViewersChanged {
                channel_id: ch,
                user_id: streamer,
                viewers,
            },
        )
        .await;
}
