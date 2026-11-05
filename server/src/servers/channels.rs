//! Renaming and deleting server channels. Anyone in the server may (the v1 trust model, as for
//! creating them). DMs and groups have their own rules (`dms::routes`).
//!
//! A delete moves the channel to its server's Recently deleted (#112): invisible everywhere (the
//! access rule and the channel listings skip it), nothing purged. From there the server owner or
//! whoever deleted it may restore it; the owner may purge it now, and the sweeper purges it after
//! [`RECENTLY_DELETED_DAYS`]. A deleted channel still counts toward the server's channel cap, so a
//! restore never takes a server past it.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use pulse_protocol::rest::{Channel, ChannelKind, ClipView, DeletedChannel};
use sqlx::SqliteConnection;

use crate::AppState;
use crate::access::{CHANNEL_COLS, channel_for, channel_from_row, load_channel};
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
    let changed = sqlx::query("UPDATE channels SET name = ? WHERE id = ? AND deleted_at IS NULL")
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

/// `DELETE /channels/{id}`: move a server channel to its server's Recently deleted (#112). It goes
/// from everyone's sidebar at once, as a hard delete would (`ChannelDeleted`), and anyone in its
/// call is taken out, but nothing in it is purged: a restore brings it all back, and the purge (the
/// owner's "Delete now", or the sweeper after [`RECENTLY_DELETED_DAYS`]) removes it for good.
pub async fn delete(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let (ch, server) = server_channel_for(&s, me, id).await?;
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    if ch.kind == ChannelKind::Quotes && on_ballot(&mut tx, id).await? {
        return Err(AppError::Conflict(
            "One of its quotes is up in the Quote of the Year vote: end the vote before deleting it."
                .into(),
        ));
    }
    let at = crate::db::now();
    let moved = sqlx::query(
        "UPDATE channels SET deleted_at = ?, deleted_by = ? WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(&at)
    .bind(me.to_string())
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if moved == 0 {
        return Err(AppError::NotFound); // deleted in the meantime; dropping `tx` rolls back
    }
    // Its call ends here: the leave webhooks that follow find no channel to update, so its voice
    // time counts toward badges now.
    let closed: Vec<(String, String, String)> = sqlx::query_as(
        "UPDATE voice_sessions SET left_at = ? WHERE channel_id = ? AND left_at IS NULL
         RETURNING user_id, joined_at, left_at",
    )
    .bind(&at)
    .bind(id.to_string())
    .fetch_all(&mut *tx)
    .await?;
    let closed: Vec<(UserId, String, String)> = closed
        .into_iter()
        .filter_map(|(user, joined, left)| Some((user.parse().ok()?, joined, left)))
        .collect();
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
    // Badge bookkeeping in its own transaction, after the delete: it never fails it.
    crate::badges::voice::closed(&s, id, &closed).await;
    // The channel can't be found now, so a join webhook finds nothing to join. Its events can't
    // reach anyone (they'd be scoped to a channel that can't be found): clients drop its voice
    // presence and streams on `ChannelDeleted`.
    if ch.kind == ChannelKind::Voice {
        let people: Vec<UserId> = s.voice.members(id).into_iter().collect();
        for u in &people {
            s.voice.leave(id, *u, "");
            let ended = s.streams.end(id, *u);
            s.streams.remove_viewer_everywhere(id, *u);
            if let Some(info) = ended {
                crate::badges::streams::ended(&s, &info).await;
            }
        }
        crate::streams::ghosts::channel_gone(&s, id).await;
        let s = s.clone();
        tokio::spawn(async move { crate::voice::revoke::close_call(&s, id, &people).await });
    }
    Ok(StatusCode::NO_CONTENT)
}

/// How long a deleted channel waits in Recently deleted before the sweeper purges it.
pub const RECENTLY_DELETED_DAYS: i64 = 7;

/// Whether one of the channel's quotes is an option of a live Quote of the Year vote. Such a
/// channel isn't deleted (the vote would offer, count and crown a quote nobody can see), and isn't
/// purged either (a vote reopened after the delete can bring its options back).
async fn on_ballot(db: &mut SqliteConnection, id: ChannelId) -> AppResult<bool> {
    let hit: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM qoty_options o
         JOIN qoty_events e ON e.id = o.event_id AND e.ended_at IS NULL
         JOIN messages m ON m.id = o.message_id
         WHERE m.channel_id = ? LIMIT 1",
    )
    .bind(id.to_string())
    .fetch_optional(&mut *db)
    .await?;
    Ok(hit.is_some())
}

/// Why a deleted channel can't be purged yet, if it can't (the Quote of the Year guards #104 put on
/// the delete itself): a quotes channel stays while one of its quotes is [`on_ballot`], and for
/// good once it holds a winner (what won is kept when its quote is deleted: `qoty_winners`). The
/// sweeper leaves such a channel in Recently deleted, where the list shows this as `held`.
async fn held(
    db: &mut SqliteConnection,
    id: ChannelId,
    kind: ChannelKind,
) -> AppResult<Option<&'static str>> {
    if kind != ChannelKind::Quotes {
        return Ok(None);
    }
    if on_ballot(db, id).await? {
        return Ok(Some(
            "One of its quotes is up in the Quote of the Year vote: it can't be deleted for good until the vote ends.",
        ));
    }
    let holds_winner: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM qoty_winners w JOIN messages m ON m.id = w.message_id
         WHERE m.channel_id = ? LIMIT 1",
    )
    .bind(id.to_string())
    .fetch_optional(&mut *db)
    .await?;
    Ok(holds_winner
        .map(|_| "It holds a Quote of the Year winner, so it can't be deleted for good."))
}

type DeletedRow = (
    String,
    Option<String>,
    String,
    Option<String>,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
);

/// The deleted server channel `id`: (channel, its server, who deleted it). 404 when there is no
/// such channel in Recently deleted.
async fn deleted_channel(
    db: &mut SqliteConnection,
    id: ChannelId,
) -> AppResult<(Channel, ServerId, Option<UserId>)> {
    let row: Option<DeletedRow> = sqlx::query_as(&format!(
        "SELECT {CHANNEL_COLS}, deleted_by, deleted_at FROM channels
         WHERE id = ? AND deleted_at IS NOT NULL AND server_id IS NOT NULL"
    ))
    .bind(id.to_string())
    .fetch_optional(&mut *db)
    .await?;
    let (id, server, kind, name, position, icon, still, by, _) = row.ok_or(AppError::NotFound)?;
    let ch = channel_from_row((id, server, kind, name, position, icon, still))?;
    let server = ch.server_id.ok_or(AppError::NotFound)?;
    Ok((ch, server, parse_user(by)?))
}

fn parse_user(id: Option<String>) -> AppResult<Option<UserId>> {
    Ok(id
        .map(|u| u.parse::<UserId>())
        .transpose()
        .map_err(anyhow::Error::from)?)
}

/// Who owns `server` (its creator).
async fn owner_of(db: &mut SqliteConnection, server: ServerId) -> AppResult<UserId> {
    let owner: String = sqlx::query_scalar("SELECT created_by FROM servers WHERE id = ?")
        .bind(server.to_string())
        .fetch_one(&mut *db)
        .await?;
    Ok(owner.parse().map_err(anyhow::Error::from)?)
}

async fn member_in(db: &mut SqliteConnection, server: ServerId, user: UserId) -> AppResult<bool> {
    let hit: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM server_members WHERE server_id = ? AND user_id = ?")
            .bind(server.to_string())
            .bind(user.to_string())
            .fetch_optional(&mut *db)
            .await?;
    Ok(hit.is_some())
}

/// When a channel deleted at `deleted_at` is due to be purged.
fn purge_at(deleted_at: &str) -> AppResult<String> {
    let at = chrono::DateTime::parse_from_rfc3339(deleted_at)
        .map_err(anyhow::Error::from)?
        .with_timezone(&chrono::Utc);
    Ok(crate::db::stamp(
        at + chrono::Duration::days(RECENTLY_DELETED_DAYS),
    ))
}

/// `GET /servers/{id}/deleted-channels`: the server's Recently deleted, newest first, for its
/// members, each with what the asker may do with it.
pub async fn list_deleted(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(server): Path<ServerId>,
) -> AppResult<Json<Vec<DeletedChannel>>> {
    crate::servers::routes::require_member(&s.db, server, me).await?;
    let mut db = s.db.acquire().await?;
    let owner = owner_of(&mut db, server).await? == me;
    let rows: Vec<DeletedRow> = sqlx::query_as(&format!(
        "SELECT {CHANNEL_COLS}, deleted_by, deleted_at FROM channels
         WHERE server_id = ? AND deleted_at IS NOT NULL ORDER BY deleted_at DESC, id"
    ))
    .bind(server.to_string())
    .fetch_all(&mut *db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, sid, kind, name, position, icon, still, by, at) in rows {
        let channel = channel_from_row((id, sid, kind, name, position, icon, still))?;
        let deleted_by = parse_user(by)?;
        out.push(DeletedChannel {
            held: held(&mut db, channel.id, channel.kind)
                .await?
                .map(str::to_string),
            can_restore: owner || deleted_by == Some(me),
            can_purge: owner,
            purge_at: purge_at(&at)?,
            deleted_at: at,
            deleted_by,
            channel,
        });
    }
    Ok(Json(out))
}

/// `POST /channels/{id}/restore`: put a deleted server channel back where it was, with everything
/// in it, live for everyone. The server owner and whoever deleted it may.
pub async fn restore(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
) -> AppResult<Json<Channel>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    // The counts that follow are absolutes: in commit order.
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let (_, server, deleted_by) = deleted_channel(&mut tx, id).await?;
    if !member_in(&mut tx, server, me).await?
        || (owner_of(&mut tx, server).await? != me && deleted_by != Some(me))
    {
        return Err(AppError::Forbidden);
    }
    sqlx::query("UPDATE channels SET deleted_at = NULL, deleted_by = NULL WHERE id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let channel = load_channel(&s.db, id).await?.ok_or(AppError::NotFound)?;
    s.hub
        .publish(
            &s.db,
            Event::ChannelRestored {
                channel: channel.clone(),
            },
        )
        .await;
    // Clients dropped its counts and mutes with the channel: each member gets theirs back. The
    // restore has landed, so a failed read is logged and that event skipped.
    if let Err(e) = republish_own_state(&s, id).await {
        tracing::error!(error = ?e, channel = %id, "couldn't send a restored channel's counts");
    }
    Ok(Json(channel))
}

/// Each member's read state in `channel`, the mutes of each who has muted it, and the clip view of
/// each who set one (clients drop all three with the channel). Quote reply seen points come back
/// on the next Ready.
async fn republish_own_state(s: &AppState, channel: ChannelId) -> AppResult<()> {
    let readers: Vec<String> =
        sqlx::query_scalar("SELECT user_id FROM read_states WHERE channel_id = ?")
            .bind(channel.to_string())
            .fetch_all(&s.db)
            .await?;
    for u in readers {
        let user: UserId = u.parse().map_err(anyhow::Error::from)?;
        if let Some(r) = crate::reads::state_in(&s.db, user, channel).await? {
            s.hub
                .publish(
                    &s.db,
                    Event::ReadStateUpdated {
                        user_id: user,
                        channel_id: channel,
                        last_read_message_id: r.last_read_message_id,
                        mentions: Some(r.mentions),
                        unread: Some(r.unread),
                    },
                )
                .await;
        }
    }
    let views: Vec<(String, String)> =
        sqlx::query_as("SELECT user_id, view FROM clip_view_prefs WHERE channel_id = ?")
            .bind(channel.to_string())
            .fetch_all(&s.db)
            .await?;
    for (u, view) in views {
        s.hub
            .publish(
                &s.db,
                Event::ClipViewChanged {
                    user_id: u.parse().map_err(anyhow::Error::from)?,
                    channel_id: channel,
                    view: if view == "list" {
                        ClipView::List
                    } else {
                        ClipView::Wall
                    },
                },
            )
            .await;
    }
    let muters: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT user_id FROM notification_prefs
         WHERE target_kind = 'channel' AND target_id = ?",
    )
    .bind(channel.to_string())
    .fetch_all(&s.db)
    .await?;
    for u in muters {
        let user: UserId = u.parse().map_err(anyhow::Error::from)?;
        let mutes = crate::dms::routes::mutes_of(&s.db, user).await?;
        s.hub
            .publish(
                &s.db,
                Event::MutesChanged {
                    user_id: user,
                    mutes,
                },
            )
            .await;
    }
    Ok(())
}

/// `DELETE /channels/{id}/permanent`: the owner's "Delete now" from Recently deleted. 409 while
/// it is [`held`].
pub async fn purge_now(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let mut db = s.db.acquire().await?;
    let (_, server, _) = deleted_channel(&mut db, id).await?;
    if owner_of(&mut db, server).await? != me {
        return Err(AppError::Forbidden);
    }
    drop(db);
    purge(&s, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A deleted server channel and everything in it, for good (a hard delete, like a group whose
/// last member leaves): 404 unless it is in Recently deleted, 409 while it is [`held`]. Its
/// messages go, their search entries with them (by trigger), and the cascades take their
/// reactions, mentions, quotes, clips rows, previews, seen points of quote replies, options of
/// ended Quote of the Year votes, and the channel's read states, voice sessions and clip views;
/// its mutes are deleted here, and its open clip tickets keep no channel (their confirm then
/// refuses). Disk media nothing uses any more is left to the media cleanup, and clip objects are
/// deleted after the commit (the clip sweeper retries a failure). Nobody hears of it: the channel
/// left everyone's view when it was deleted.
async fn purge(s: &AppState, id: ChannelId) -> AppResult<()> {
    // Unread and mention counts others publish as absolutes span channels: in commit order.
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let (ch, _, _) = deleted_channel(&mut tx, id).await?;
    if let Some(why) = held(&mut tx, id, ch.kind).await? {
        return Err(AppError::Conflict(why.into()));
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
    sqlx::query("DELETE FROM channels WHERE id = ? AND deleted_at IS NOT NULL")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    drop(_order);
    if !clips.is_empty() {
        let s = s.clone();
        tokio::spawn(async move {
            for key in clips {
                crate::messages::routes::remove_clip_object(&s, &key).await;
            }
        });
    }
    Ok(())
}

/// Purge every channel deleted [`RECENTLY_DELETED_DAYS`] or more before `now`, except those
/// [`held`] (they stay in Recently deleted, flagged). Returns how many went.
pub async fn purge_expired(s: &AppState, now: chrono::DateTime<chrono::Utc>) -> AppResult<u32> {
    let due = crate::db::stamp(now - chrono::Duration::days(RECENTLY_DELETED_DAYS));
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM channels WHERE deleted_at IS NOT NULL AND deleted_at <= ? ORDER BY deleted_at",
    )
    .bind(&due)
    .fetch_all(&s.db)
    .await?;
    let mut purged = 0;
    for id in ids {
        let id: ChannelId = id.parse().map_err(anyhow::Error::from)?;
        match purge(s, id).await {
            Ok(()) => purged += 1,
            // Held, or restored or purged in the meantime.
            Err(AppError::Conflict(_) | AppError::NotFound) => {}
            // One channel that fails doesn't keep the rest (it's tried again next sweep).
            Err(e) => tracing::warn!(error = ?e, channel = %id, "couldn't purge a deleted channel"),
        }
    }
    Ok(purged)
}

/// How often the sweeper looks for channels due to be purged.
const SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(3600);

/// Run [`purge_expired`] now and then hourly. Call after binding the listener.
pub fn spawn_sweeper(s: AppState) {
    tokio::spawn(async move {
        loop {
            match purge_expired(&s, chrono::Utc::now()).await {
                Ok(n) if n > 0 => tracing::info!(channels = n, "Recently deleted purged"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = ?e, "Recently deleted sweep failed"),
            }
            tokio::time::sleep(SWEEP_EVERY).await;
        }
    });
}
