//! Renaming and deleting server channels. Anyone in the server may (the v1 trust model, as for
//! creating them). DMs and groups have their own rules (`dms::routes`).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use pulse_protocol::rest::{Channel, ChannelKind};

use crate::AppState;
use crate::access::{channel_for, load_channel};
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

/// The server channel `id`, if `me` may change it: 404 when there is none (or it's a
/// conversation `me` isn't in), 400 for a DM or group, 403 when `me` isn't in its server.
async fn server_channel_for(
    s: &AppState,
    me: UserId,
    id: ChannelId,
) -> AppResult<(Channel, ServerId)> {
    let ch = channel_for(&s.db, me, id).await?;
    let Some(server) = ch.server_id else {
        return Err(AppError::BadRequest(
            "conversations are changed from their own menu".into(),
        ));
    };
    if !crate::servers::routes::is_server_member(&s.db, server, me).await? {
        return Err(AppError::Forbidden);
    }
    Ok((ch, server))
}

/// Rename a server channel (`PATCH /channels/{id}` for a channel with a server). It always keeps
/// a name; no system message is posted (a group's rename posts one).
pub async fn rename(
    s: &AppState,
    me: UserId,
    id: ChannelId,
    name: Option<String>,
) -> AppResult<Channel> {
    server_channel_for(s, me, id).await?;
    let name = crate::servers::routes::clean_name(name.as_deref().unwrap_or(""))?;
    let changed = sqlx::query("UPDATE channels SET name = ? WHERE id = ?")
        .bind(&name)
        .bind(id.to_string())
        .execute(&s.db)
        .await?
        .rows_affected();
    // Deleted in the meantime.
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    let channel = load_channel(&s.db, id).await?.ok_or(AppError::NotFound)?;
    s.hub
        .publish(
            &s.db,
            Event::ChannelUpdated {
                channel: channel.clone(),
            },
        )
        .await;
    Ok(channel)
}

/// `DELETE /channels/{id}`: a server channel and everything in it, for good (a hard delete, like
/// a group whose last member leaves). Its messages go, their search entries with them (by
/// trigger), and the cascades take their reactions, mentions, quotes, clips rows, previews, seen
/// points of quote replies, options of ended Quote of the Year votes, and the channel's read
/// states, voice sessions and clip views; its mutes are deleted here, and its open clip tickets
/// keep no channel (their confirm then refuses). Disk media nothing uses any more is left to the
/// media cleanup, and clip objects are deleted after the commit (the clip sweeper retries a
/// failure). Anyone in its call is taken out. A quotes channel stays while its server has a live
/// Quote of the Year vote (whose options are quotes), and for good once it holds a winner.
pub async fn delete(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let (ch, server) = server_channel_for(&s, me, id).await?;
    // Unread and mention counts others publish as absolutes span channels: in commit order.
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    if ch.kind == ChannelKind::Quotes && crate::qoty::live_in(&mut tx, server).await?.is_some() {
        return Err(AppError::Conflict(
            "A Quote of the Year vote is on: end it before deleting a quotes channel.".into(),
        ));
    }
    // What won is kept when its quote is deleted (`qoty_winners`): the channel can't take it.
    let holds_winner: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM qoty_winners w JOIN messages m ON m.id = w.message_id
         WHERE m.channel_id = ? LIMIT 1",
    )
    .bind(id.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    if holds_winner.is_some() {
        return Err(AppError::Conflict(
            "This channel holds a Quote of the Year winner, so it can't be deleted.".into(),
        ));
    }
    let clips: Vec<String> = sqlx::query_scalar(
        "SELECT c.video_key FROM clips c JOIN messages m ON m.id = c.message_id
         WHERE m.channel_id = ?",
    )
    .bind(id.to_string())
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM notification_prefs WHERE target_kind = 'channel' AND target_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    // Before the channel, so the search triggers run for each message (one statement: replies and
    // quote replies point at messages going with them).
    // Quote lines first: their trigger rewrites a quote's entry, which the message's then drops.
    sqlx::query(
        "DELETE FROM quote_lines WHERE message_id IN (SELECT id FROM messages WHERE channel_id = ?)",
    )
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM messages WHERE channel_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    let gone = sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if gone == 0 {
        return Err(AppError::NotFound); // deleted in the meantime; dropping `tx` rolls back
    }
    tx.commit().await?;
    s.hub
        .publish(
            &s.db,
            Event::ChannelDeleted {
                channel_id: id,
                server_id: server,
            },
        )
        .await;
    drop(_order);
    // The channel is gone, so a join webhook from now on finds nothing to join. Its events can't
    // reach anyone (they'd be scoped to a channel that isn't there): clients drop its voice
    // presence and streams on `ChannelDeleted`.
    if ch.kind == ChannelKind::Voice {
        let people: Vec<UserId> = s.voice.members(id).into_iter().collect();
        for u in &people {
            s.voice.leave(id, *u, "");
            s.streams.end(id, *u);
            s.streams.remove_viewer_everywhere(id, *u);
        }
        let s = s.clone();
        tokio::spawn(async move { crate::voice::revoke::close_call(&s, id, &people).await });
    }
    if !clips.is_empty() {
        let s = s.clone();
        tokio::spawn(async move {
            for key in clips {
                crate::messages::routes::remove_clip_object(&s, &key).await;
            }
        });
    }
    Ok(StatusCode::NO_CONTENT)
}
