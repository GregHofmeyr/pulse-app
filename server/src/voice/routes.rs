use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use livekit_api::access_token::{AccessToken, TokenVerifier, VideoGrants};
use livekit_api::webhooks::WebhookReceiver;
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{Channel, StreamEndReason, VoiceTokenResponse};
use sqlx::SqlitePool;

use crate::AppState;
use crate::access::{channel_for, is_channel_member, load_channel};
use crate::auth::AuthUser;
use crate::auth::extractor::BearerToken;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::voice::state::Left;

const TOKEN_TTL: Duration = Duration::from_secs(10 * 60);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/voice/{id}/token", post(token))
        .route("/livekit/webhook", post(webhook))
}

async fn token(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    BearerToken(session): BearerToken,
    Path(id): Path<ChannelId>,
) -> AppResult<Json<VoiceTokenResponse>> {
    let ch = channel_for(&s.db, me, id).await?;
    if ch.kind.is_server_chat() {
        return Err(AppError::BadRequest(
            "text and quotes channels have no voice".into(),
        ));
    }
    if !may_join(&s.db, me, &ch).await? {
        return Err(AppError::Forbidden);
    }
    // A random grant id, mapped to this session server-side only: the join webhook checks it
    // and revoking this session removes the participant that joined with it.
    let room = id.to_string();
    if !s.limits.livekit_token_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let grant = crate::auth::session::record_grant(&s.db, &session, &room, &me.to_string())
        .await?
        .ok_or(AppError::Unauthorized)?;
    let jwt = AccessToken::with_api_key(&s.cfg.livekit_key, &s.cfg.livekit_secret)
        .with_identity(&me.to_string())
        .with_attributes([(crate::auth::session::GRANT_ATTRIBUTE, grant)])
        .with_ttl(TOKEN_TTL)
        .with_grants(VideoGrants {
            room_join: true,
            room,
            can_publish: Some(true),
            // Voice rooms carry the microphone only: screen share has its own room (`<id>~s`).
            can_publish_sources: vec!["microphone".into()],
            can_subscribe: Some(true),
            // Nothing in a call uses data messages; refusing them keeps a client from sending
            // arbitrary payloads to everyone in the room.
            can_publish_data: Some(false),
            ..Default::default()
        })
        .to_jwt()
        .map_err(|e| AppError::Internal(e.into()))?;
    Ok(Json(VoiceTokenResponse {
        url: s.cfg.livekit_url.clone(),
        token: jwt,
    }))
}

/// Who may be in a channel's call: members only for DMs and groups, and server members for a
/// server's voice channels (same rule as posting). Checked when a token is issued and again on
/// every LiveKit join (`revoke::admit`), so a token stops working once access is lost.
pub async fn may_join(db: &SqlitePool, user: UserId, ch: &Channel) -> AppResult<bool> {
    if ch.kind.is_server_chat() {
        return Ok(false);
    }
    if ch.kind.is_private() && !is_channel_member(db, user, ch.id).await? {
        return Ok(false);
    }
    match ch.server_id {
        Some(server) => crate::servers::routes::is_server_member(db, server, user).await,
        None => Ok(true),
    }
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

    // The stream room (`<channel>~s`) has its own rules.
    if let Some(room) = ev.room.as_ref()
        && let Some(channel) = room.name.strip_suffix("~s")
    {
        crate::streams::webhook::handle(&s, channel, &ev).await?;
        return Ok(StatusCode::OK);
    }

    // Rooms are channel IDs, identities are user IDs. Anything else isn't ours: ack and ignore.
    let (Some(room), Some(p)) = (ev.room.as_ref(), ev.participant.as_ref()) else {
        return Ok(StatusCode::OK);
    };
    let (Ok(channel_id), Ok(user_id)) =
        (room.name.parse::<ChannelId>(), p.identity.parse::<UserId>())
    else {
        return Ok(StatusCode::OK);
    };
    // A join needs a live session behind it (a revoked device's JWT still works at LiveKit).
    if ev.event == "participant_joined" && !crate::voice::revoke::admit(&s, &room.name, p).await? {
        return Ok(StatusCode::OK);
    }
    if ev.event == "participant_left" {
        crate::voice::revoke::left(&s, p).await?;
    }
    let Some(ch) = load_channel(&s.db, channel_id).await? else {
        return Ok(StatusCode::OK);
    };
    // Only server voice channels feed stats; DM/group calls are never recorded (spec §8).
    let record = ch.server_id.is_some();

    match ev.event.as_str() {
        "participant_joined" if s.voice.join(channel_id, user_id, &p.sid) => {
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
            let in_memory = match s.voice.leave(channel_id, user_id, &p.sid) {
                Left::Stale => return Ok(StatusCode::OK),
                Left::Removed => true,
                // Still close the open row (e.g. we restarted mid-call).
                Left::NotPresent => false,
            };
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
            if in_memory {
                // Only the stream in THIS channel: a late leave for an old channel must not
                // end a stream the user has since started elsewhere.
                if s.streams.end(channel_id, user_id).is_some() {
                    tracing::info!(user = %user_id, channel = %channel_id, reason = "left_voice", "stream ended");
                    crate::streams::routes::publish_ended(
                        &s,
                        channel_id,
                        user_id,
                        StreamEndReason::LeftVoice,
                    )
                    .await;
                }
                for (streamer, viewers) in s.streams.remove_viewer_everywhere(channel_id, user_id) {
                    crate::streams::routes::publish_viewers(&s, channel_id, streamer, viewers)
                        .await;
                }
                // Stream rooms exist only for server voice channels; access to one ends here.
                if record {
                    crate::voice::revoke::end_stream_access(&s, channel_id, user_id);
                }
            }
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
