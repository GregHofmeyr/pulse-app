use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use livekit_api::access_token::{AccessToken, TokenVerifier, VideoGrants};
use livekit_api::webhooks::WebhookReceiver;
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{ChannelKind, VoiceTokenResponse};

use crate::AppState;
use crate::access::{channel_for, load_channel};
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};

const TOKEN_TTL: Duration = Duration::from_secs(10 * 60);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/voice/{id}/token", post(token))
        .route("/livekit/webhook", post(webhook))
}

async fn token(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
) -> AppResult<Json<VoiceTokenResponse>> {
    let ch = channel_for(&s.db, me, id).await?;
    if ch.kind == ChannelKind::Text {
        return Err(AppError::BadRequest("text channels have no voice".into()));
    }
    // Server voice: join the server first (same rule as posting).
    if let Some(server) = ch.server_id
        && !crate::servers::routes::is_server_member(&s.db, server, me).await?
    {
        return Err(AppError::Forbidden);
    }
    let jwt = AccessToken::with_api_key(&s.cfg.livekit_key, &s.cfg.livekit_secret)
        .with_identity(&me.to_string())
        .with_ttl(TOKEN_TTL)
        .with_grants(VideoGrants {
            room_join: true,
            room: id.to_string(),
            can_publish: Some(true),
            can_subscribe: Some(true),
            ..Default::default()
        })
        .to_jwt()
        .map_err(|e| AppError::Internal(e.into()))?;
    Ok(Json(VoiceTokenResponse {
        url: s.cfg.livekit_url.clone(),
        token: jwt,
    }))
}

async fn webhook(
    State(s): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> AppResult<StatusCode> {
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.strip_prefix("Bearer ").unwrap_or(v))
        .unwrap_or_default();
    let receiver = WebhookReceiver::new(TokenVerifier::with_api_key(
        &s.cfg.livekit_key,
        &s.cfg.livekit_secret,
    ));
    let ev = receiver.receive(&body, auth).map_err(|e| {
        tracing::warn!(error = %e, "rejected livekit webhook");
        AppError::Unauthorized
    })?;

    // Rooms are channel IDs, identities are user IDs. Anything else isn't ours: ack and ignore.
    let (Some(room), Some(p)) = (ev.room.as_ref(), ev.participant.as_ref()) else {
        return Ok(StatusCode::OK);
    };
    let (Ok(channel_id), Ok(user_id)) =
        (room.name.parse::<ChannelId>(), p.identity.parse::<UserId>())
    else {
        return Ok(StatusCode::OK);
    };
    let Some(ch) = load_channel(&s.db, channel_id).await? else {
        return Ok(StatusCode::OK);
    };
    // Only server voice channels feed stats; DM/group calls are never recorded (spec §8).
    let record = ch.server_id.is_some();

    match ev.event.as_str() {
        "participant_joined" if s.voice.join(channel_id, user_id) => {
            if record {
                sqlx::query("INSERT INTO voice_sessions (id, user_id, channel_id, joined_at) VALUES (?, ?, ?, ?)")
                        .bind(pulse_protocol::ids::next_ulid().to_string())
                        .bind(user_id.to_string())
                        .bind(channel_id.to_string())
                        .bind(now())
                        .execute(&s.db)
                        .await?;
            }
            s.hub
                .publish(
                    &s.db,
                    Event::VoiceJoined {
                        channel_id,
                        user_id,
                        flags: s.voice.declared(user_id),
                    },
                )
                .await;
        }
        "participant_left" => {
            // Close the open row even if we never saw the join (e.g. we restarted mid-call).
            let in_memory = s.voice.leave(channel_id, user_id);
            let closed = if record {
                sqlx::query(
                    "UPDATE voice_sessions SET left_at = ? WHERE user_id = ? AND channel_id = ? AND left_at IS NULL",
                )
                .bind(now())
                .bind(user_id.to_string())
                .bind(channel_id.to_string())
                .execute(&s.db)
                .await?
                .rows_affected()
                    > 0
            } else {
                false
            };
            if in_memory || closed {
                s.hub
                    .publish(
                        &s.db,
                        Event::VoiceLeft {
                            channel_id,
                            user_id,
                        },
                    )
                    .await;
            }
        }
        _ => {}
    }
    Ok(StatusCode::OK)
}
