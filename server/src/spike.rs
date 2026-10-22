//! THROWAWAY (screen-share spike). Stream-room tickets shaped like the spec's §5.2, behind PULSE_SPIKE=1.

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use livekit_api::access_token::{AccessToken, VideoGrants};
use pulse_protocol::ids::ChannelId;
use pulse_protocol::rest::VoiceTokenResponse;
use serde::Deserialize;

use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

pub fn router() -> Router<AppState> {
    Router::new().route("/spike/stream/{id}/ticket", post(ticket))
}

#[derive(Deserialize)]
struct Req {
    role: String,
}

async fn ticket(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<Req>,
) -> AppResult<Json<VoiceTokenResponse>> {
    if !s.cfg.spike {
        return Err(AppError::NotFound);
    }
    let ch = channel_for(&s.db, me, id).await?;
    // same rule as voice/routes.rs `token`: server voice needs server membership
    if let Some(server) = ch.server_id
        && !crate::servers::routes::is_server_member(&s.db, server, me).await?
    {
        return Err(AppError::Forbidden);
    }
    if !s.voice.members(id).contains(&me) {
        return Err(AppError::Forbidden);
    }
    let sources: Vec<String> = match req.role.as_str() {
        "v" => vec!["screen_share".into()],
        "a" => vec!["screen_share_audio".into()],
        "v2" => vec![],
        _ => return Err(AppError::BadRequest("role is v, v2 or a".into())),
    };
    let jwt = AccessToken::with_api_key(&s.cfg.livekit_key, &s.cfg.livekit_secret)
        .with_identity(&format!("{me}#{}", req.role))
        .with_ttl(std::time::Duration::from_secs(10 * 60))
        .with_grants(VideoGrants {
            room_join: true,
            room: format!("{id}~s"),
            can_publish: Some(!sources.is_empty()),
            can_publish_sources: sources,
            can_subscribe: Some(true),
            can_publish_data: Some(false),
            ..Default::default()
        })
        .to_jwt()
        .map_err(|e| AppError::Internal(e.into()))?;
    tracing::info!(user = %me, channel = %id, role = %req.role, "spike ticket");
    Ok(Json(VoiceTokenResponse {
        url: s.cfg.livekit_url.clone(),
        token: jwt,
    }))
}
