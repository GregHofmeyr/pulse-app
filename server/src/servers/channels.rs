//! Renaming and deleting server channels. Anyone in the server may (the v1 trust model, as for
//! creating them). DMs and groups have their own rules (`dms::routes`).
//!
//! A delete moves the channel to its server's Recently deleted (#112): invisible everywhere (the
//! access rule and the channel listings skip it), nothing purged. From there the server owner or
//! whoever deleted it may restore it; the owner may purge it now, and the sweeper purges it after
//! [`RECENTLY_DELETED_DAYS`]. A deleted channel still counts toward the server's channel cap, so a
//! restore never takes a server past it.
//!
//! A purge goes in short batches (#108, `servers::purge`), so it can be cut off part-way by a
//! restart. "Delete now" therefore first makes the channel due (as if its 7 days were up): from
//! then on it is out of Recently deleted and can't be restored, and if the purge doesn't finish,
//! the next sweep takes up from what is left.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use pulse_protocol::rest::{
    CHANNEL_BITRATES_KBPS, Channel, ChannelKind, ClipView, DeletedChannel, SetChannelBitrateRequest,
};
use sqlx::SqliteConnection;

use crate::AppState;
use crate::access::{CHANNEL_COLS, DELETED_SERVERS, channel_for, channel_from_row, load_channel};
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::servers::categories::{bottom_of, bump, publish_layout};

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

/// `PUT /channels/{id}/bitrate`: a voice channel's audio bitrate (#75), one of
/// [`CHANNEL_BITRATES_KBPS`] or `None` for Default. Anyone in the server may, as with renaming;
/// everyone hears `ChannelUpdated`, and the clients in its call switch to it.
pub async fn set_bitrate(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<SetChannelBitrateRequest>,
) -> AppResult<Json<Channel>> {
    // each change reaches everyone: limited and disk-guarded, like a rename
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    crate::media::writes_ok(&s.cfg)?;
    let (ch, _) = server_channel_for(&s, me, id).await?;
    if ch.kind != ChannelKind::Voice {
        return Err(AppError::BadRequest(
            "only voice channels have a bitrate".into(),
        ));
    }
    if req
        .bitrate
        .is_some_and(|b| !CHANNEL_BITRATES_KBPS.contains(&b))
    {
        return Err(AppError::BadRequest("not one of the bitrates".into()));
    }
    let changed =
        sqlx::query("UPDATE channels SET bitrate = ? WHERE id = ? AND deleted_at IS NULL")
            .bind(req.bitrate.map(i64::from))
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
    Ok(Json(channel))
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
    // It leaves the layout: a save based on the order with it in is stale now.
    bump(&mut tx, server).await?;
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
    publish_layout(&s, server).await;
    // Its stage goes in the same order as shares (#234): a share finishing its card now finds the channel gone.
    crate::stage::channel_gone(&s, id);
    drop(_order);
    // Badge bookkeeping in its own transaction, after the delete: it never fails it.
    crate::badges::voice::closed(&s, id, &closed).await;
    if ch.kind == ChannelKind::Voice {
        end_gone_call(&s, id).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Take everyone out of the call of voice channel `id`, which has just become impossible to find
/// (deleted, or its server deleted, #184): out of voice state, their streams ended (badges told)
/// and their viewing dropped, its ghost viewers gone, and LiveKit told to close its voice and
/// stream rooms in the background (everyone in them, whether this snapshot of the voice state had
/// them or not: #109). A join webhook finds nothing to join now, and one that loaded the channel
/// before the delete undoes itself (`voice::routes`). Its events can't reach anyone (they'd be
/// scoped to a channel that can't be found): clients drop its voice presence and streams on
/// `ChannelDeleted`.
pub(crate) async fn end_gone_call(s: &AppState, id: ChannelId) {
    let people: Vec<UserId> = s.voice.members(id).into_iter().collect();
    for u in people {
        out_of_gone_call(s, id, u, "").await;
    }
    crate::streams::ghosts::channel_gone(s, id).await;
    let s = s.clone();
    tokio::spawn(async move { crate::voice::revoke::close_call(&s, id).await });
}

/// `user`'s connection `sid` (empty: any) leaves the call of gone channel `id`: out of voice
/// state, their stream there ended (badges told) and their viewing dropped. Nothing is published
/// (see [`end_gone_call`]). A stale `sid` (they're in on a newer connection) leaves them be.
pub(crate) async fn out_of_gone_call(s: &AppState, id: ChannelId, user: UserId, sid: &str) {
    if s.voice.leave(id, user, sid) == crate::voice::state::Left::Stale {
        return;
    }
    let ended = s.streams.end(id, user);
    s.streams.remove_viewer_everywhere(id, user);
    if let Some(info) = ended {
        crate::badges::streams::ended(s, &info).await;
    }
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
pub(crate) async fn held(
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

/// Deleted at or before this, a channel is due to be purged: the sweeper takes it, and it can no
/// longer be restored (unless [`held`], when it never will be purged).
pub(crate) fn due(now: chrono::DateTime<chrono::Utc>) -> String {
    crate::db::stamp(now - chrono::Duration::days(RECENTLY_DELETED_DAYS))
}

/// Whether channel `id` is still deleted and due by `due` (`false`: restored, or purged by someone
/// else).
pub(crate) async fn still_due(
    db: &mut SqliteConnection,
    id: ChannelId,
    due: &str,
) -> AppResult<bool> {
    let hit: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM channels WHERE id = ? AND deleted_at IS NOT NULL AND deleted_at <= ?",
    )
    .bind(id.to_string())
    .bind(due)
    .fetch_optional(&mut *db)
    .await?;
    Ok(hit.is_some())
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
    Option<i64>,
    Option<String>,
    String,
);

/// The deleted server channel `id`: (channel, its server, who deleted it, when). 404 when there is
/// no such channel in Recently deleted, or its server is deleted too (#184: frozen with it).
async fn deleted_channel(
    db: &mut SqliteConnection,
    id: ChannelId,
) -> AppResult<(Channel, ServerId, Option<UserId>, String)> {
    let row: Option<DeletedRow> = sqlx::query_as(&format!(
        "SELECT {CHANNEL_COLS}, deleted_by, deleted_at FROM channels
         WHERE id = ? AND deleted_at IS NOT NULL AND server_id IS NOT NULL
           AND server_id NOT IN {DELETED_SERVERS}"
    ))
    .bind(id.to_string())
    .fetch_optional(&mut *db)
    .await?;
    let (id, server, kind, name, position, category, icon, still, bitrate, by, at) =
        row.ok_or(AppError::NotFound)?;
    let ch = channel_from_row((
        id, server, kind, name, position, category, icon, still, bitrate,
    ))?;
    let server = ch.server_id.ok_or(AppError::NotFound)?;
    Ok((ch, server, parse_user(by)?, at))
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

/// When a channel (or server) deleted at `deleted_at` is due to be purged.
pub(crate) fn purge_at(deleted_at: &str) -> AppResult<String> {
    let at = chrono::DateTime::parse_from_rfc3339(deleted_at)
        .map_err(anyhow::Error::from)?
        .with_timezone(&chrono::Utc);
    Ok(crate::db::stamp(
        at + chrono::Duration::days(RECENTLY_DELETED_DAYS),
    ))
}

/// `GET /servers/{id}/deleted-channels`: the server's Recently deleted, newest first, for its
/// members, each with what the asker may do with it. A channel that is due and not [`held`] isn't
/// listed: it is being purged (or about to be), and can't be restored.
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
    let due = due(chrono::Utc::now());
    let mut out = Vec::with_capacity(rows.len());
    for (id, sid, kind, name, position, category, icon, still, bitrate, by, at) in rows {
        let channel = channel_from_row((
            id, sid, kind, name, position, category, icon, still, bitrate,
        ))?;
        let deleted_by = parse_user(by)?;
        let held = held(&mut db, channel.id, channel.kind).await?;
        if held.is_none() && at <= due {
            continue;
        }
        out.push(DeletedChannel {
            held: held.map(str::to_string),
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

/// `POST /channels/{id}/restore`: put a deleted server channel back, with everything in it, live
/// for everyone. The server owner and whoever deleted it may. It goes to the bottom of its
/// category, or of the uncategorised area when its category was deleted meanwhile (which cleared
/// its `category_id`): the order has moved on since, so its old position means nothing now. 404
/// once it is due (being purged, or about to be), unless it is [`held`] (never purged).
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
    let (old, server, deleted_by, at) = deleted_channel(&mut tx, id).await?;
    if at <= due(chrono::Utc::now()) && held(&mut tx, id, old.kind).await?.is_none() {
        return Err(AppError::NotFound);
    }
    if !member_in(&mut tx, server, me).await?
        || (owner_of(&mut tx, server).await? != me && deleted_by != Some(me))
    {
        return Err(AppError::Forbidden);
    }
    // Before it is live again, so it isn't counted in its own bottom.
    let position = bottom_of(&mut tx, server, old.category_id).await?;
    sqlx::query(
        "UPDATE channels SET deleted_at = NULL, deleted_by = NULL, position = ? WHERE id = ?",
    )
    .bind(position)
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?;
    bump(&mut tx, server).await?;
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
    publish_layout(&s, server).await;
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
pub(crate) async fn republish_own_state(s: &AppState, channel: ChannelId) -> AppResult<()> {
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
    // their mutes and notification levels in it (#195)
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
        let levels = crate::dms::routes::levels_of(&s.db, user).await?;
        s.hub
            .publish(
                &s.db,
                Event::NotifyLevelsChanged {
                    user_id: user,
                    levels,
                },
            )
            .await;
    }
    Ok(())
}

/// `DELETE /channels/{id}/permanent`: the owner's "Delete now" from Recently deleted. 409 while
/// it is [`held`]. It first makes the channel due, in its own transaction (out of Recently
/// deleted, no longer restorable), then purges it. The purge runs as its own task, so a client
/// that hangs up doesn't cut it off (a restart still can: the next sweep finishes it), and the
/// answer waits for it at most [`PURGE_NOW_WAIT`]: a big channel goes on purging after it.
pub async fn purge_now(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let due = make_due(&s, me, id).await?;
    let mut task = {
        let s = s.clone();
        tokio::spawn(async move {
            let done = purge(&s, id, &due, PURGE_BATCH, None).await;
            // Restored can't be (it's due), so a 404 or 409 is a purge that raced it, or a hold.
            if let Err(e) = &done
                && !matches!(e, AppError::NotFound | AppError::Conflict(_))
            {
                tracing::warn!(error = ?e, channel = %id, "couldn't purge a deleted channel now; the sweep will");
            }
            done
        })
    };
    if let Ok(done) = tokio::time::timeout(PURGE_NOW_WAIT, &mut task).await {
        done.map_err(anyhow::Error::from)??;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// How long "Delete now" waits for its purge before answering (clients give up on a request after
/// 15 s): a small channel is gone when it answers, a big one shortly after.
const PURGE_NOW_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// "Delete now"'s first step: deleted channel `id` (404 unless in Recently deleted; 403 unless
/// `me` owns its server; 409 while [`held`]) is made due now, if it isn't already. Returns the due
/// mark its purge goes by.
async fn make_due(s: &AppState, me: UserId, id: ChannelId) -> AppResult<String> {
    let mut tx = crate::db::begin_write(&s.db).await?;
    let (ch, server, _, _) = deleted_channel(&mut tx, id).await?;
    if owner_of(&mut tx, server).await? != me {
        return Err(AppError::Forbidden);
    }
    if let Some(why) = held(&mut tx, id, ch.kind).await? {
        return Err(AppError::Conflict(why.into()));
    }
    let due = due(chrono::Utc::now());
    sqlx::query("UPDATE channels SET deleted_at = ?1 WHERE id = ?2 AND deleted_at > ?1")
        .bind(&due)
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(due)
}

/// How many messages one purge transaction deletes.
const PURGE_BATCH: i64 = crate::servers::purge::BATCH;

/// "Delete now" by the server's owner, cut off after `batches` batches of `batch` messages as a
/// restart would cut it off: for tests.
#[cfg(feature = "testing")]
pub async fn purge_partly(
    s: &AppState,
    id: ChannelId,
    batch: i64,
    batches: usize,
) -> AppResult<()> {
    let owner = {
        let mut db = s.db.acquire().await?;
        let (_, server, _, _) = deleted_channel(&mut db, id).await?;
        owner_of(&mut db, server).await?
    };
    let due = make_due(s, owner, id).await?;
    purge(s, id, &due, batch, Some(batches)).await
}

/// A deleted server channel due by `due` and everything in it, for good (a hard delete, like a
/// group whose last member leaves): 404 unless it is in Recently deleted and due, 409 while it is
/// [`held`]. Its messages go first, `batch` per transaction (`servers::purge::messages`: their
/// search entries, reactions, mentions, quotes, clips rows and objects, polls, pins, previews,
/// inbox and thread reads, options of ended Quote of the Year votes and wheel spins with them),
/// stopping after `max_batches` if given. Then, in one last short transaction, its mutes (they
/// have no foreign key) and the channel, whose cascades take its read states, voice sessions,
/// calls, clip views and stream sessions; its open clip tickets keep no channel (their confirm
/// then refuses). Every file started in it (#57) is deleted after that commit (the attachments
/// sweep retries a failure). Disk media nothing uses any more is left to the media cleanup. Nobody
/// hears of it: the channel left everyone's view when it was deleted.
async fn purge(
    s: &AppState,
    id: ChannelId,
    due: &str,
    batch: i64,
    max_batches: Option<usize>,
) -> AppResult<()> {
    let kind = {
        let mut db = s.db.acquire().await?;
        deleted_channel(&mut db, id).await?.0.kind
    };
    let what = crate::servers::purge::Purging::Channel { id, kind, due };
    if !crate::servers::purge::messages(s, id, &what, batch, max_batches).await? {
        return Ok(()); // cut off (tests)
    }
    // Unread and mention counts others publish as absolutes span channels: in commit order.
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    what.check(&mut tx).await?;
    // Every file started in the channel, sent or not (#57): deleted after the commit.
    let files = crate::attachments::stored_in_channels(&mut tx, &[id]).await?;
    sqlx::query("DELETE FROM notification_prefs WHERE target_kind = 'channel' AND target_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    // No write reaches a deleted channel (#109), so it is empty by now; were a message left, the
    // cascade would take it (its search entry by trigger).
    sqlx::query("DELETE FROM channels WHERE id = ? AND deleted_at IS NOT NULL")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    drop(_order);
    if !files.is_empty() {
        let s = s.clone();
        tokio::spawn(async move { crate::attachments::remove_objects(&s, files).await });
    }
    Ok(())
}

/// Purge every channel deleted [`RECENTLY_DELETED_DAYS`] or more before `now` (or made due by
/// "Delete now", including one whose purge was cut off), except those [`held`] (they stay in
/// Recently deleted, flagged) and those of a deleted server (frozen with it, #184: its own purge
/// takes them). Returns how many went.
pub async fn purge_expired(s: &AppState, now: chrono::DateTime<chrono::Utc>) -> AppResult<u32> {
    purge_expired_batched(s, now, PURGE_BATCH).await
}

/// [`purge_expired`] deleting at most `batch` messages per transaction.
pub async fn purge_expired_batched(
    s: &AppState,
    now: chrono::DateTime<chrono::Utc>,
    batch: i64,
) -> AppResult<u32> {
    let due = due(now);
    let ids: Vec<String> = sqlx::query_scalar(&format!(
        "SELECT id FROM channels WHERE deleted_at IS NOT NULL AND deleted_at <= ?
         AND server_id NOT IN {DELETED_SERVERS} ORDER BY deleted_at"
    ))
    .bind(&due)
    .fetch_all(&s.db)
    .await?;
    let mut purged = 0;
    for id in ids {
        let id: ChannelId = id.parse().map_err(anyhow::Error::from)?;
        match purge(s, id, &due, batch, None).await {
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
