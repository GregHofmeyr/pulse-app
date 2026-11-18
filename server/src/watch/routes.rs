//! `PUT /channels/{id}/watch-party` starts a YouTube watch party in a server voice channel (or changes its video);
//! `POST /channels/{id}/watch-party` makes one change to it (play, pause, seek, the queue, the host, joining and
//! leaving); `DELETE /channels/{id}/watch-party` ends it for everyone (the host only). All need you in the call.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::put;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{
    ChannelKind, EmbedProvider, EmbedThumb, StartWatchPartyRequest, WatchParty, WatchPartyAction,
};

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

/// A link preview's title, thumbnail hash, width and height.
type CardRow = (Option<String>, Option<String>, Option<i64>, Option<i64>);

/// A video's card as a message's link preview already has it (#464): its title and thumbnail (kept, so it outlives the
/// party). Nothing when no message has shown it.
async fn known_card(s: &AppState, video_id: &str) -> (Option<String>, Option<EmbedThumb>) {
    let row: Option<CardRow> = sqlx::query_as(
        "SELECT title, thumb_hash, thumb_width, thumb_height FROM link_embeds \
         WHERE provider = ? AND provider_id = ? ORDER BY fetched_at DESC LIMIT 1",
    )
    .bind(crate::embeds::provider_str(EmbedProvider::Youtube))
    .bind(video_id)
    .fetch_optional(&s.db)
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "couldn't look up a watch party video's card");
        None
    });
    let Some((title, hash, w, h)) = row else {
        return (None, None);
    };
    let thumb = match (hash, w, h) {
        (Some(hash), Some(w), Some(h)) => EmbedThumb {
            hash,
            width: u32::try_from(w).unwrap_or(0),
            height: u32::try_from(h).unwrap_or(0),
        }
        .into(),
        _ => None,
    };
    let thumb = match thumb {
        Some(t) => crate::embeds::keep_thumb(&s.db, t).await.ok().flatten(),
        None => None,
    };
    (title, thumb)
}

/// A pasted link no message has shown has no title (#464): fetch its card in the background (the link previewer's
/// cache and bounds), and fill it in wherever the party still lacks it.
fn fetch_card(s: &AppState, channel: ChannelId, video_id: String, by: UserId) {
    if crate::media::writes_ok(&s.cfg).is_err() {
        return;
    }
    let s = s.clone();
    tokio::spawn(async move {
        let url = format!("https://www.youtube.com/watch?v={video_id}");
        let Some(card) = s
            .embeds
            .unfurl(&s.db, &s.cfg.media_dir, by, &s.limits.link_thumb_user, &url)
            .await
            .filter(|c| {
                c.provider == EmbedProvider::Youtube
                    && c.provider_id.as_deref() == Some(video_id.as_str())
            })
        else {
            return;
        };
        let thumb = match card.thumb {
            Some(t) => crate::embeds::keep_thumb(&s.db, t).await.ok().flatten(),
            None => None,
        };
        let _order = s.hub.in_order().await;
        let outcome =
            s.voice
                .watch
                .fill_card(channel, &video_id, card.title.as_deref(), thumb.as_ref());
        announce(&s, channel, outcome).await;
    });
}

/// After a change that put `video_id` on or in the queue: its known card filled in, folded into `outcome` (one event
/// out, not two), and a fetch started when it still has no title.
async fn with_card(
    s: &AppState,
    channel: ChannelId,
    me: UserId,
    video_id: &str,
    card: (Option<String>, Option<EmbedThumb>),
    outcome: Outcome,
) -> Outcome {
    if !matches!(outcome, Outcome::Changed(_)) {
        return outcome;
    }
    let (title, thumb) = card;
    let filled = s
        .voice
        .watch
        .fill_card(channel, video_id, title.as_deref(), thumb.as_ref());
    let outcome = match filled {
        Outcome::Unchanged => outcome,
        o => o,
    };
    if title.is_none()
        && let Outcome::Changed(p) = &outcome
        && (p.video_id == video_id && p.title.is_none()
            || p.queue
                .iter()
                .any(|i| i.video_id == video_id && i.title.is_none()))
    {
        fetch_card(s, channel, video_id.to_owned(), me);
    }
    outcome
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
    let card = if super::valid_video_id(&req.video_id) {
        known_card(&s, &req.video_id).await
    } else {
        (None, None)
    };
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
    let outcome = with_card(
        &s,
        id,
        me,
        &req.video_id,
        card,
        Outcome::Changed(party.clone()),
    )
    .await;
    let party = match &outcome {
        Outcome::Changed(p) => p.clone(),
        _ => party,
    };
    announce(&s, id, outcome).await;
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
    // a video put on or in the queue: its card, when a message already showed it (#464)
    let added = match &action {
        WatchPartyAction::Load { video_id, .. } | WatchPartyAction::Enqueue { video_id, .. }
            if super::valid_video_id(video_id) =>
        {
            Some((video_id.clone(), known_card(&s, video_id).await))
        }
        _ => None,
    };
    let _order = s.hub.in_order().await;
    let members = s.voice.members(id);
    if !leaving && !members.contains(&me) {
        return Err(AppError::Forbidden);
    }
    let mut outcome = s
        .voice
        .watch
        .apply(id, me, action, now_ms(), |u| members.contains(&u))
        .map_err(refused)?;
    if let Some((video_id, card)) = added {
        outcome = with_card(&s, id, me, &video_id, card, outcome).await;
    }
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
