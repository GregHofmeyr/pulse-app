//! `PUT /channels/{id}/watch-party` starts a YouTube watch party in a server voice channel (or changes its video);
//! `POST /channels/{id}/watch-party` makes one change to it (play, pause, seek, the queue, the host, joining and
//! leaving); `DELETE /channels/{id}/watch-party` ends it for everyone (the host only). All need you in the call.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::put;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{ChannelKind, StartWatchPartyRequest, WatchParty, WatchPartyAction};

use super::{Outcome, Refused};
use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/channels/{id}/watch-party",
        put(start).post(act).delete(end),
    )
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// A server voice channel the caller can see, belongs to the server of, and is in the call of right now.
async fn in_call(s: &AppState, me: UserId, id: ChannelId) -> AppResult<()> {
    let ch = channel_for(&s.db, me, id).await?;
    let Some(server) = ch.server_id.filter(|_| ch.kind == ChannelKind::Voice) else {
        return Err(AppError::BadRequest(
            "only a server voice channel has watch parties".into(),
        ));
    };
    if !crate::servers::routes::is_server_member(&s.db, server, me).await?
        || !s.voice.members(id).contains(&me)
    {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

fn refused(r: Refused) -> AppError {
    match r {
        Refused::NoParty => AppError::NotFound,
        Refused::NotAllowed => AppError::Forbidden,
        Refused::AlreadyOn => AppError::Conflict("a watch party is on here already".into()),
        Refused::Bad(why) => AppError::BadRequest(why.into()),
    }
}

/// Tell the channel what a change did (and keep its snapshot in step, #409).
pub(crate) async fn announce(s: &AppState, channel: ChannelId, outcome: Outcome) {
    super::snapshot(&s.db, channel, &outcome, now_ms()).await;
    let event = match outcome {
        Outcome::Unchanged => return,
        Outcome::Changed(party) => Event::WatchPartyChanged {
            party,
            server_now_ms: now_ms(),
        },
        Outcome::Ended => Event::WatchPartyEnded {
            channel_id: channel,
        },
    };
    s.hub.publish(&s.db, event).await;
}

async fn start(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<StartWatchPartyRequest>,
) -> AppResult<Json<WatchParty>> {
    in_call(&s, me, id).await?;
    if !s.limits.watch_party_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    // Starts, changes and leaves go out in the order they happened.
    let _order = s.hub.in_order().await;
    if !s.voice.members(id).contains(&me) {
        return Err(AppError::Forbidden);
    }
    let party = s
        .voice
        .watch
        .start(id, me, &req.video_id, req.title, now_ms())
        .map_err(refused)?;
    announce(&s, id, Outcome::Changed(party.clone())).await;
    Ok(Json(party))
}

async fn act(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(action): Json<WatchPartyAction>,
) -> AppResult<StatusCode> {
    // Leaving is always allowed, call or not (and never rate limited: it only ever takes you out).
    let leaving = matches!(action, WatchPartyAction::Leave);
    if leaving {
        channel_for(&s.db, me, id).await?;
    } else {
        in_call(&s, me, id).await?;
        if !s.limits.watch_party_user.hit(&me.to_string()) {
            return Err(AppError::TooManyRequests(None));
        }
    }
    let _order = s.hub.in_order().await;
    let members = s.voice.members(id);
    if !leaving && !members.contains(&me) {
        return Err(AppError::Forbidden);
    }
    let outcome = s
        .voice
        .watch
        .apply(id, me, action, now_ms(), |u| members.contains(&u))
        .map_err(refused)?;
    announce(&s, id, outcome).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn end(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
) -> AppResult<StatusCode> {
    in_call(&s, me, id).await?;
    let _order = s.hub.in_order().await;
    let party = s.voice.watch.of(id).ok_or(AppError::NotFound)?;
    if party.host_id != me {
        return Err(AppError::Forbidden);
    }
    if s.voice.watch.remove_channel(id) {
        announce(&s, id, Outcome::Ended).await;
    }
    Ok(StatusCode::NO_CONTENT)
}
