//! Deleting a server (#184). Only its owner may, typing its name to confirm. It moves to the
//! owner's Recently deleted for [`RECENTLY_DELETED_DAYS`], as a deleted channel does (#112): gone
//! for everyone (the access rule, the server checks and the Ready lists skip it) but nothing in it
//! purged, so the owner can restore it whole. After that the sweeper purges it with everything in
//! it, in batches (`servers::purge`), so a big server never holds the write lock for long (#108).
//!
//! [`RECENTLY_DELETED_DAYS`]: crate::servers::channels::RECENTLY_DELETED_DAYS

use std::collections::{BTreeSet, HashMap};

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pulse_protocol::gateway::{Event, ServerSnapshot};
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use pulse_protocol::rest::{ClipView, DeleteServerRequest, DeletedServer, Server};
use sqlx::SqliteConnection;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::servers::channels::{end_gone_call, purge_at};
use crate::servers::routes::{SERVER_COLS, ServerRow, server_from_row};

use crate::servers::purge::{BATCH as PURGE_BATCH, Purging};

/// Deleted at or before this, a server is due to be purged and can no longer be restored (as a
/// channel is: the same 7 days).
fn due(now: chrono::DateTime<chrono::Utc>) -> String {
    crate::servers::channels::due(now)
}

/// `DELETE /servers/{id}`: the owner moves their server to their Recently deleted. 404 when there
/// is no such live server, 403 unless the owner, 400 unless `name` (trimmed) is its name. Its
/// channels go from everyone's view as deleted channels do (`ChannelDeleted` each, which older
/// clients understand), then the server (`ServerDeleted`); anyone in its calls is taken out and
/// their streams end. Nothing is purged.
pub async fn delete(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
    Json(req): Json<DeleteServerRequest>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT name, created_by FROM servers WHERE id = ? AND deleted_at IS NULL")
            .bind(id.to_string())
            .fetch_optional(&s.db)
            .await?;
    let (name, owner) = row.ok_or(AppError::NotFound)?;
    if owner != me.to_string() {
        return Err(AppError::Forbidden);
    }
    if req.name.trim() != name {
        return Err(AppError::BadRequest(
            "Type the server's name to delete it.".into(),
        ));
    }
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let at = crate::db::now();
    let moved = sqlx::query(
        "UPDATE servers SET deleted_at = ?, deleted_by = ? WHERE id = ? AND deleted_at IS NULL",
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
    let live: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, kind FROM channels WHERE server_id = ? AND deleted_at IS NULL ORDER BY id",
    )
    .bind(id.to_string())
    .fetch_all(&mut *tx)
    .await?;
    // Its calls end here, as a deleted channel's do: their voice time counts toward badges now.
    let closed: Vec<(String, String, String, String)> = sqlx::query_as(
        "UPDATE voice_sessions SET left_at = ?
         WHERE left_at IS NULL AND channel_id IN (SELECT id FROM channels WHERE server_id = ?)
         RETURNING channel_id, user_id, joined_at, left_at",
    )
    .bind(&at)
    .bind(id.to_string())
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let live: Vec<(ChannelId, bool)> = live
        .into_iter()
        .filter_map(|(c, kind)| Some((c.parse().ok()?, kind == "voice")))
        .collect();
    // One batch: whoever had inbox items in it is told their count once (#327).
    let mut events: Vec<Event> = live
        .iter()
        .map(|(channel_id, _)| Event::ChannelDeleted {
            channel_id: *channel_id,
            server_id: id,
        })
        .collect();
    events.push(Event::ServerDeleted { server_id: id });
    s.hub.publish_all(&s.db, events).await;
    // Their stages go in the same order as shares (#234).
    for (channel_id, voice) in &live {
        if *voice {
            crate::stage::channel_gone(&s, *channel_id);
        }
    }
    drop(_order);
    let mut by_channel: HashMap<ChannelId, Vec<(UserId, String, String)>> = HashMap::new();
    for (channel, user, joined, left) in closed {
        if let (Ok(c), Ok(u)) = (channel.parse(), user.parse()) {
            by_channel.entry(c).or_default().push((u, joined, left));
        }
    }
    // Badge bookkeeping in its own transactions, after the delete: it never fails it.
    for (channel, sessions) in &by_channel {
        crate::badges::voice::closed(&s, *channel, sessions).await;
    }
    for (channel_id, voice) in &live {
        if *voice {
            end_gone_call(&s, *channel_id).await;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /servers/deleted`: your own servers in Recently deleted, newest first.
pub async fn list(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
) -> AppResult<Json<Vec<DeletedServer>>> {
    let rows: Vec<(String, String, Option<String>, String, String)> = sqlx::query_as(&format!(
        "SELECT {SERVER_COLS}, deleted_at FROM servers
         WHERE created_by = ? AND deleted_at IS NOT NULL ORDER BY deleted_at DESC, id"
    ))
    .bind(me.to_string())
    .fetch_all(&s.db)
    .await?;
    let due = due(chrono::Utc::now());
    let mut out = Vec::with_capacity(rows.len());
    for (id, name, icon, owner, deleted_at) in rows {
        out.push(DeletedServer {
            server: server_from_row((id, name, icon, owner))?,
            can_restore: deleted_at > due,
            purge_at: purge_at(&deleted_at)?,
            deleted_at,
        });
    }
    Ok(Json(out))
}

/// `POST /servers/{id}/restore`: the owner brings their deleted server back, whole, for everyone.
/// 404 unless it is in Recently deleted and not yet due to be purged (so a restore and a purge
/// never overlap), 403 unless the owner.
pub async fn restore(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
) -> AppResult<Json<Server>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    // The counts that follow are absolutes: in commit order.
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let owner: Option<String> = sqlx::query_scalar(
        "SELECT created_by FROM servers WHERE id = ? AND deleted_at IS NOT NULL AND deleted_at > ?",
    )
    .bind(id.to_string())
    .bind(due(chrono::Utc::now()))
    .fetch_optional(&mut *tx)
    .await?;
    if owner.ok_or(AppError::NotFound)? != me.to_string() {
        return Err(AppError::Forbidden);
    }
    sqlx::query("UPDATE servers SET deleted_at = NULL, deleted_by = NULL WHERE id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let snapshot = snapshot_of(&s, id).await?;
    let server = snapshot.server.clone();
    s.hub
        .publish(&s.db, Event::ServerRestored { snapshot })
        .await;
    // Clients dropped their own counts, mutes and clip views in it with the server: each member
    // gets theirs back. The restore has landed, so a failed read is logged and that event skipped.
    if let Err(e) = republish_own_state(&s, id).await {
        tracing::error!(error = ?e, server = %id, "couldn't send a restored server's counts");
    }
    Ok(Json(server))
}

/// What Ready would carry for `id`.
async fn snapshot_of(s: &AppState, id: ServerId) -> AppResult<ServerSnapshot> {
    let row: ServerRow = sqlx::query_as(&format!("SELECT {SERVER_COLS} FROM servers WHERE id = ?"))
        .bind(id.to_string())
        .fetch_one(&s.db)
        .await?;
    let mut layout = {
        let mut db = s.db.acquire().await?;
        crate::servers::categories::layout_of(&mut db, id).await?
    };
    // as in Ready: the channels carry their own placement
    layout.channels = vec![];
    Ok(ServerSnapshot {
        server: server_from_row(row)?,
        channels: crate::servers::routes::server_channels(&s.db, id).await?,
        members: crate::servers::routes::server_members(&s.db, id).await?,
        layout,
        custom_emoji: crate::emoji::for_ready(&s.db)
            .await?
            .into_iter()
            .filter(|e| e.server_id == id)
            .collect(),
        sounds: crate::sounds::for_server(&s.db, id).await?,
        games: crate::clips::games::for_ready(&s.db)
            .await?
            .into_iter()
            .filter(|g| g.server_id == id)
            .collect(),
        speaker_links: crate::quotes::links::for_ready(&s.db)
            .await?
            .into_iter()
            .filter(|l| l.server_id == id)
            .collect(),
        qoty: crate::qoty::for_ready(&s.db)
            .await?
            .into_iter()
            .filter(|q| q.server_id == id)
            .collect(),
        server_voice: crate::voice::moderation::for_ready(&s.db)
            .await?
            .into_iter()
            .filter(|m| m.server_id == id)
            .collect(),
    })
}

/// Each member's read state in the server's live channels, the clip views they set there, and the
/// mutes and notification levels of each who has one on the server or one of its channels.
async fn republish_own_state(s: &AppState, server: ServerId) -> AppResult<()> {
    const LIVE: &str = "(SELECT id FROM channels WHERE server_id = ? AND deleted_at IS NULL)";
    let reads: Vec<(String, String)> = sqlx::query_as(&format!(
        "SELECT user_id, channel_id FROM read_states WHERE channel_id IN {LIVE}"
    ))
    .bind(server.to_string())
    .fetch_all(&s.db)
    .await?;
    for (u, c) in reads {
        let user: UserId = u.parse().map_err(anyhow::Error::from)?;
        let channel: ChannelId = c.parse().map_err(anyhow::Error::from)?;
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
    let views: Vec<(String, String, String)> = sqlx::query_as(&format!(
        "SELECT user_id, channel_id, view FROM clip_view_prefs WHERE channel_id IN {LIVE}"
    ))
    .bind(server.to_string())
    .fetch_all(&s.db)
    .await?;
    for (u, c, view) in views {
        s.hub
            .publish(
                &s.db,
                Event::ClipViewChanged {
                    user_id: u.parse().map_err(anyhow::Error::from)?,
                    channel_id: c.parse().map_err(anyhow::Error::from)?,
                    view: if view == "list" {
                        ClipView::List
                    } else {
                        ClipView::Wall
                    },
                },
            )
            .await;
    }
    let muters: BTreeSet<String> = sqlx::query_scalar(&format!(
        "SELECT DISTINCT user_id FROM notification_prefs
         WHERE (target_kind = 'server' AND target_id = ?)
            OR (target_kind = 'channel' AND target_id IN {LIVE})"
    ))
    .bind(server.to_string())
    .bind(server.to_string())
    .fetch_all(&s.db)
    .await?
    .into_iter()
    .collect();
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

/// Purge every server deleted [`RECENTLY_DELETED_DAYS`] or more before `now`. Returns how many went.
///
/// [`RECENTLY_DELETED_DAYS`]: crate::servers::channels::RECENTLY_DELETED_DAYS
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
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM servers WHERE deleted_at IS NOT NULL AND deleted_at <= ? ORDER BY deleted_at",
    )
    .bind(&due)
    .fetch_all(&s.db)
    .await?;
    let mut purged = 0;
    for id in ids {
        let id: ServerId = id.parse().map_err(anyhow::Error::from)?;
        match purge(s, id, &due, batch).await {
            Ok(true) => purged += 1,
            Ok(false) => {} // purged in the meantime
            // One server that fails doesn't keep the rest (it's tried again next sweep).
            Err(e) => tracing::warn!(error = ?e, server = %id, "couldn't purge a deleted server"),
        }
    }
    Ok(purged)
}

/// Whether `id` is still deleted and due (`false`: someone else purged it).
pub(crate) async fn still_due(
    tx: &mut SqliteConnection,
    id: ServerId,
    due: &str,
) -> AppResult<bool> {
    let hit: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM servers WHERE id = ? AND deleted_at IS NOT NULL AND deleted_at <= ?",
    )
    .bind(id.to_string())
    .bind(due)
    .fetch_optional(&mut *tx)
    .await?;
    Ok(hit.is_some())
}

/// A deleted, due server and everything in it, for good. Its channels' messages go first, one
/// channel after another, `batch` per transaction (`servers::purge::messages`: each batch its own
/// short hold of the write lock, #108, with the messages' search entries, reactions, mentions,
/// quotes, clips rows and objects, polls, pins, previews, inbox and thread reads and wheel spins).
/// Then, in one last transaction, its notification prefs (they have no foreign key), its channels
/// (with their read states, voice sessions, clip views, calls and stream sessions) and the server
/// row, whose cascades take its categories, members and their join sounds, custom emoji, sounds,
/// games, wheels, speaker links, Quote of the Year votes and winners, server mutes and nickname
/// history; the channels' files (#57) go after that commit (the attachments sweep retries a
/// failure). Disk media nothing uses any more is left to the media cleanup. Nobody hears of it:
/// the server left everyone's view when it was deleted. A purge cut off part-way (a restart) is
/// taken up from what is left by the next sweep. `false` when it was purged in the meantime.
async fn purge(s: &AppState, id: ServerId, due: &str, batch: i64) -> AppResult<bool> {
    let channels: Vec<String> = sqlx::query_scalar("SELECT id FROM channels WHERE server_id = ?")
        .bind(id.to_string())
        .fetch_all(&s.db)
        .await?;
    let channel_ids = channels
        .iter()
        .map(|c| c.parse())
        .collect::<Result<Vec<ChannelId>, _>>()
        .map_err(anyhow::Error::from)?;
    let what = Purging::Server { id, due };
    for channel in &channel_ids {
        match crate::servers::purge::messages(s, *channel, &what, batch, None).await {
            Ok(_) => {}
            Err(AppError::NotFound) => return Ok(false),
            Err(e) => return Err(e),
        }
    }
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    if !still_due(&mut tx, id, due).await? {
        return Ok(false);
    }
    sqlx::query(
        "DELETE FROM notification_prefs
         WHERE (target_kind = 'server' AND target_id = ?1)
            OR (target_kind = 'channel'
                AND target_id IN (SELECT id FROM channels WHERE server_id = ?1))",
    )
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?;
    // Every file started in its channels, sent or not (#57): their objects go after the commit.
    let files = crate::attachments::stored_in_channels(&mut tx, &channel_ids).await?;
    // Nothing can post in a deleted server, so its channels are empty by now (were a message left,
    // the cascade would take it, its search entry by trigger); a channel made between the list
    // above and here (there is no way to) would go with them, and its files to the sweep.
    sqlx::query("DELETE FROM channels WHERE server_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM servers WHERE id = ? AND deleted_at IS NOT NULL")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    if !files.is_empty() {
        let s = s.clone();
        tokio::spawn(async move { crate::attachments::remove_objects(&s, files).await });
    }
    Ok(true)
}

/// How often the sweeper looks for servers due to be purged.
const SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(3600);

/// Run [`purge_expired`] now and then hourly. Call after binding the listener.
pub fn spawn_sweeper(s: AppState) {
    tokio::spawn(async move {
        loop {
            match purge_expired(&s, chrono::Utc::now()).await {
                Ok(n) if n > 0 => tracing::info!(servers = n, "deleted servers purged"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = ?e, "deleted server sweep failed"),
            }
            tokio::time::sleep(SWEEP_EVERY).await;
        }
    });
}
