//! `PUT /channels/{id}/stage-share` shares a Spotify link to a server voice channel's stage;
//! `DELETE /channels/{id}/stage-share/{share_id}` stops that share for everyone. Both need you in the call.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, put};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{
    Channel, ChannelKind, EmbedProvider, LinkEmbed, StageShare, StageShareRequest,
};

use super::{ANYONE_IN_CALL_MAY_STOP, announce_cleared, spotify_link};
use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::stamp;
use crate::error::{AppError, AppResult};

/// How a share's chat message starts, before the link (#288).
pub const SHARE_LEAD: &str = "🎵 Shared to the stage:";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/channels/{id}/stage-share", put(share))
        .route("/channels/{id}/stage-share/{share_id}", delete(stop))
}

/// A server voice channel the caller can see, belongs to the server of, and is in the call of right now.
async fn in_call(s: &AppState, me: UserId, id: ChannelId) -> AppResult<Channel> {
    let ch = channel_for(&s.db, me, id).await?;
    let Some(server) = ch.server_id.filter(|_| ch.kind == ChannelKind::Voice) else {
        return Err(AppError::BadRequest(
            "only a server voice channel has a stage".into(),
        ));
    };
    if !crate::servers::routes::is_server_member(&s.db, server, me).await?
        || !s.voice.members(id).contains(&me)
    {
        return Err(AppError::Forbidden);
    }
    Ok(ch)
}

async fn share(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<StageShareRequest>,
) -> AppResult<Json<StageShare>> {
    let ch = in_call(&s, me, id).await?;
    let Some(link) = spotify_link(&req.url) else {
        return Err(AppError::BadRequest("that isn't a Spotify link".into()));
    };
    // Sharing what you already have up again changes nothing (no second line).
    if let Some(up) = s.voice.stage.same(id, me, &link) {
        return Ok(Json(up));
    }
    if !s.limits.stage_share_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    crate::media::writes_ok(&s.cfg)?;
    let card = s
        .embeds
        .unfurl(
            &s.db,
            &s.cfg.media_dir,
            me,
            &s.limits.link_thumb_user,
            &link,
        )
        .await
        // a short link that led somewhere else is no Spotify card either
        .filter(|c| c.provider == EmbedProvider::Spotify && c.provider_id.is_some())
        .ok_or_else(|| AppError::BadRequest("couldn't read that Spotify link".into()))?;
    let mut thumb = card.thumb;
    // The artwork is media no message holds: restart its cleanup clock (a day) so it outlives the share (hours). A
    // cached one already cleaned up goes.
    if let Some(t) = &thumb {
        let touched = sqlx::query("UPDATE media SET created_at = ? WHERE hash = ?")
            .bind(crate::db::now())
            .bind(&t.hash)
            .execute(&s.db)
            .await?
            .rows_affected();
        if touched == 0 {
            thumb = None;
        }
    }
    let at = chrono::Utc::now();
    let ttl = chrono::Duration::from_std(s.cfg.stage_share_ttl).map_err(anyhow::Error::from)?;
    let share = StageShare {
        id: pulse_protocol::ids::next_ulid().to_string(),
        channel_id: id,
        shared_by: me,
        shared_at: stamp(at),
        expires_at: stamp(at + ttl),
        embed: LinkEmbed {
            url: link,
            provider: card.provider,
            kind: card.kind,
            provider_id: card.provider_id,
            title: card.title,
            description: card.description,
            site_name: card.site_name,
            author: card.author,
            thumb,
            start: None,
            vertical: false,
        },
    };
    {
        // Shares, stops, leaves and expiries go out in the order they happened.
        let _order = s.hub.in_order().await;
        // The card took a moment: a leave or the channel's delete in the meantime means no share, and the same
        // link shared meanwhile (a double click) is the one up.
        if crate::access::load_channel(&s.db, id).await?.is_none() {
            return Err(AppError::NotFound);
        }
        if !s.voice.members(id).contains(&me) {
            return Err(AppError::Forbidden);
        }
        if let Some(up) = s.voice.stage.same(id, me, &share.embed.url) {
            return Ok(Json(up));
        }
        let gone = s.voice.stage.put(share.clone());
        announce_cleared(&s, gone).await;
        s.hub
            .publish(
                &s.db,
                Event::StageShared {
                    share: share.clone(),
                },
            )
            .await;
    }
    // The link in chat as the sharer's own message (#288), through the send path once the order guard is let go: it
    // unfurls as the Spotify or Jam card for everyone (people not in the call, 0.2.x clients), follows each person's
    // notification level for the channel, and mentions nobody.
    let content = format!("{SHARE_LEAD} {}", share.embed.url);
    if let Err(e) = crate::messages::routes::post_plain(&s, me, &ch, content).await {
        tracing::warn!(error = ?e, channel = %id, "stage share message not posted");
    }
    expire_later(&s, &share);
    Ok(Json(share))
}

/// Take `share` down when it runs out, unless it's gone by then.
fn expire_later(s: &AppState, share: &StageShare) {
    let (s, channel, id) = (s.clone(), share.channel_id, share.id.clone());
    tokio::spawn(async move {
        tokio::time::sleep(s.cfg.stage_share_ttl).await;
        let _order = s.hub.in_order().await;
        if let Some(gone) = s.voice.stage.remove(channel, &id) {
            announce_cleared(&s, vec![gone]).await;
        }
    });
}

async fn stop(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path((id, share_id)): Path<(ChannelId, String)>,
) -> AppResult<StatusCode> {
    in_call(&s, me, id).await?;
    let _order = s.hub.in_order().await;
    let up = s
        .voice
        .stage
        .of(id)
        .into_iter()
        .find(|x| x.id == share_id)
        .ok_or(AppError::NotFound)?;
    if !ANYONE_IN_CALL_MAY_STOP && up.shared_by != me {
        return Err(AppError::Forbidden);
    }
    if let Some(gone) = s.voice.stage.remove(id, &share_id) {
        announce_cleared(&s, vec![gone]).await;
    }
    Ok(StatusCode::NO_CONTENT)
}
