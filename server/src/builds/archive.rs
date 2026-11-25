//! Archiving builds (#616): one at a time or in bulk, never deleting; deleting one for good; and the per-game counts
//! the channel's panel shows. Archiving, restoring and deleting are for the build's author and the server's owner.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, GameId, MessageId, UserId};
use std::collections::{HashMap, HashSet};

use pulse_protocol::rest::{
    ArchiveBuildsRequest, BuildBrief, BuildFacets, BuildsBatchResult, BulkArchiveRequest,
    BulkArchiveResult, Channel, DeleteBuildsRequest, GameCount, Message, SetBuildArchivedRequest,
};
use serde::Deserialize;
use sqlx::SqliteConnection;

use super::routes::{bad, builds_channel};
use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::{begin_write, now};
use crate::error::{AppError, AppResult};
use crate::messages::routes::load;

/// The most builds one bulk archive takes.
pub const BULK_MAX: usize = 500;
/// The most builds one archive-many or delete-many names.
pub const BATCH_MAX: usize = 100;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/builds/{message_id}/archived", put(set_archived))
        .route("/channels/{id}/builds/archive", post(bulk))
        .route("/channels/{id}/builds/archive-many", post(archive_many))
        .route("/channels/{id}/builds/delete-many", post(delete_many))
        .route("/channels/{id}/builds/facets", get(facets))
}

/// A live build `me` may see, with its channel, and whether `me` may archive or delete it (its author, or the
/// server's owner). 404 for anyone who can't see it. The author's own build stays theirs after they leave the server
/// (#418, as `own_message`): they can still delete it; changing it (archiving) is for members, checked by the caller.
async fn build_for(s: &AppState, me: UserId, id: MessageId) -> AppResult<(Message, Channel, bool)> {
    let (channel, _) = super::build_channel(&s.db, id).await?;
    let msg = load(&s.db, Some(me), id).await?.ok_or(AppError::NotFound)?;
    if msg.build.is_none() || msg.deleted {
        return Err(AppError::NotFound);
    }
    if msg.author_id == Some(me) {
        let ch = crate::access::load_channel(&s.db, channel)
            .await?
            .ok_or(AppError::NotFound)?;
        return Ok((msg, ch, true));
    }
    let ch = channel_for(&s.db, me, channel).await?;
    let allowed = owns(s, &ch, me).await?;
    Ok((msg, ch, allowed))
}

async fn owns(s: &AppState, ch: &Channel, me: UserId) -> AppResult<bool> {
    match ch.server_id {
        Some(server) => crate::servers::routes::is_owner_in(&s.db, server, me).await,
        None => Ok(false),
    }
}

/// `PUT /builds/{id}/archived`: archive a build (hidden from the list, kept, still openable), or restore it.
async fn set_archived(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
    Json(req): Json<SetBuildArchivedRequest>,
) -> AppResult<Json<Message>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let (_, ch, allowed) = build_for(&s, me, id).await?;
    if !allowed {
        return Err(AppError::Forbidden);
    }
    // left the server: their build is no longer theirs to change (deleting it stays allowed)
    crate::access::require_member_of(&s.db, me, &ch).await?;
    crate::access::require_open(&ch)?;
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    crate::access::channel_live_in(&mut tx, ch.id).await?;
    crate::access::open_in(&mut tx, ch.id).await?;
    let q = if req.archived {
        // archiving an archived build keeps who archived it and when
        sqlx::query(
            "UPDATE builds SET archived_at = COALESCE(archived_at, ?), archived_by = COALESCE(archived_by, ?)
             WHERE message_id = ?",
        )
        .bind(now())
        .bind(me.to_string())
    } else {
        sqlx::query("UPDATE builds SET archived_at = NULL, archived_by = NULL WHERE message_id = ?")
    };
    q.bind(id.to_string()).execute(&mut *tx).await?;
    tx.commit().await?;
    if let Some(message) = load(&s.db, None, id).await? {
        s.hub
            .publish(&s.db, Event::MessageUpdated { message })
            .await;
    }
    drop(_order);
    Ok(Json(
        load(&s.db, Some(me), id).await?.ok_or(AppError::NotFound)?,
    ))
}

/// `POST /channels/{id}/builds/archive`: archive every live, unarchived build saved before `before` (of one game, or
/// all) that the caller may archive: their own, or everyone's for the server's owner. A dry run lists them (newest
/// first); otherwise they're archived in one write and the channel hears one `BuildsArchived`.
async fn bulk(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<BulkArchiveRequest>,
) -> AppResult<Json<BulkArchiveResult>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let ch = builds_channel(&s, me, id).await?;
    let before = crate::times::parse(req.before.trim())
        .map(crate::db::stamp)
        .map_err(|_| bad("That time isn't one I can read."))?;
    let everyone = owns(&s, &ch, me).await?;
    if !req.dry_run {
        crate::access::require_open(&ch)?;
    }
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    crate::access::channel_live_in(&mut tx, id).await?;
    let mut qb = sqlx::QueryBuilder::new(
        "SELECT b.message_id, b.title, b.game_id, m.created_at FROM builds b
         JOIN messages m ON m.id = b.message_id AND m.deleted_at IS NULL
         WHERE b.archived_at IS NULL AND b.channel_id = ",
    );
    qb.push_bind(id.to_string());
    qb.push(" AND m.created_at < ").push_bind(&before);
    if let Some(game) = req.game_id {
        qb.push(" AND b.game_id = ").push_bind(game.to_string());
    }
    if !everyone {
        qb.push(" AND m.author_id = ").push_bind(me.to_string());
    }
    qb.push(" ORDER BY b.message_id DESC LIMIT ")
        .push_bind(BULK_MAX as i64 + 1);
    let rows: Vec<(String, String, Option<String>, String)> =
        qb.build_query_as().fetch_all(&mut *tx).await?;
    if rows.len() > BULK_MAX {
        return Err(bad("Narrow it down: over 500 builds match."));
    }
    let builds = rows
        .into_iter()
        .map(|(message_id, title, game, created_at)| {
            Ok(BuildBrief {
                message_id: message_id.parse().map_err(anyhow::Error::from)?,
                title,
                game_id: game
                    .map(|g| g.parse::<GameId>())
                    .transpose()
                    .map_err(anyhow::Error::from)?,
                created_at,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    if req.dry_run || builds.is_empty() {
        tx.rollback().await?;
        return Ok(Json(BulkArchiveResult {
            builds,
            archived: 0,
        }));
    }
    crate::access::open_in(&mut tx, id).await?;
    let at = now();
    let mut up = sqlx::QueryBuilder::new("UPDATE builds SET archived_at = ");
    up.push_bind(&at)
        .push(", archived_by = ")
        .push_bind(me.to_string())
        .push(" WHERE archived_at IS NULL AND message_id IN (");
    let mut list = up.separated(", ");
    for b in &builds {
        list.push_bind(b.message_id.to_string());
    }
    up.push(")");
    let archived = up.build().execute(&mut *tx).await?.rows_affected() as u32;
    tx.commit().await?;
    s.hub
        .publish(
            &s.db,
            Event::BuildsArchived {
                channel_id: id,
                message_ids: builds.iter().map(|b| b.message_id).collect(),
                archived_at: at,
                archived_by: me,
            },
        )
        .await;
    Ok(Json(BulkArchiveResult { builds, archived }))
}

/// `DELETE /builds/{id}`: delete a build for good, with its screenshots and comments (its author or the server's
/// owner). The message is soft-deleted as any message is; its `builds` row goes.
pub(crate) async fn remove(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let (msg, ch, allowed) = build_for(&s, me, id).await?;
    if !allowed {
        return Err(AppError::Forbidden);
    }
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    crate::access::channel_live_in(&mut tx, ch.id).await?;
    let (events, files) = delete_in(&mut tx, &msg).await?.ok_or(AppError::NotFound)?;
    tx.commit().await?;
    s.hub.publish_all(&s.db, events).await;
    drop(_order);
    crate::attachments::remove_objects(&s, files).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes the live build `msg` inside `tx`: its message soft-deleted as any message is, its reactions, `builds` row,
/// comments, embeds, history and pin with it. The events to send once committed and the stored files to remove after
/// that; `None` when it was already gone.
async fn delete_in(
    tx: &mut SqliteConnection,
    msg: &Message,
) -> AppResult<Option<(Vec<Event>, Vec<crate::attachments::Stored>)>> {
    let id = msg.id;
    let changed = sqlx::query(
        "UPDATE messages SET content = '', gif = NULL, deleted_at = ? WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(now())
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed == 0 {
        return Ok(None);
    }
    sqlx::query("DELETE FROM reactions WHERE message_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM builds WHERE message_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    let mut replies = crate::messages::threads::soft_delete_replies(tx, id).await?;
    crate::embeds::on_delete(tx, id).await?;
    crate::messages::history::on_delete(tx, id).await?;
    let files = crate::attachments::stored_of_message(tx, id).await?;
    let unpinned = crate::pins::on_delete(tx, id).await?;
    let mut events = vec![Event::MessageDeleted {
        channel_id: msg.channel_id,
        message_id: id,
        author_id: msg.author_id,
        mentions: msg.mentions.clone(),
        thread_root_id: None,
    }];
    if unpinned {
        events.push(Event::MessageUnpinned {
            channel_id: msg.channel_id,
            message_id: id,
        });
    }
    events.append(&mut replies);
    Ok(Some((events, files)))
}

/// The ids of a batch: at most [`BATCH_MAX`], each once.
pub(super) fn batch_ids(ids: Vec<MessageId>) -> AppResult<Vec<MessageId>> {
    if ids.len() > BATCH_MAX {
        return Err(bad("Up to 100 builds at a time."));
    }
    let mut seen = HashSet::new();
    Ok(ids.into_iter().filter(|i| seen.insert(*i)).collect())
}

/// One live build of a batch's channel: its id, its author, and whether it's archived.
pub(super) struct Held {
    pub id: MessageId,
    pub author: Option<UserId>,
    pub archived: bool,
}

/// The live builds of `channel` among `ids` (a build of another channel, or a deleted one, isn't among them).
pub(super) async fn builds_in(
    c: &mut SqliteConnection,
    channel: ChannelId,
    ids: &[MessageId],
) -> AppResult<Vec<Held>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let mut q = sqlx::QueryBuilder::new(
        "SELECT b.message_id, m.author_id, b.archived_at IS NOT NULL FROM builds b
         JOIN messages m ON m.id = b.message_id AND m.deleted_at IS NULL
         WHERE b.channel_id = ",
    );
    q.push_bind(channel.to_string())
        .push(" AND b.message_id IN (");
    let mut list = q.separated(", ");
    for id in ids {
        list.push_bind(id.to_string());
    }
    q.push(")");
    let rows: Vec<(String, Option<String>, bool)> = q.build_query_as().fetch_all(&mut *c).await?;
    rows.into_iter()
        .map(|(id, author, archived)| {
            Ok(Held {
                id: id.parse().map_err(anyhow::Error::from)?,
                author: author
                    .map(|a| a.parse::<UserId>())
                    .transpose()
                    .map_err(anyhow::Error::from)?,
                archived,
            })
        })
        .collect()
}

/// `POST /channels/{id}/builds/archive-many`: archive (or restore) up to 100 of the channel's builds in one write,
/// each only if the caller may (its author, or the server's owner: the single route's rule); the rest are skipped. One
/// already as asked counts as done. Archiving sends one `BuildsArchived` naming the ones it changed; restoring sends
/// each its `MessageUpdated`, as the single route does (`BuildsArchived` can't say "restored").
async fn archive_many(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<ArchiveBuildsRequest>,
) -> AppResult<Json<BuildsBatchResult>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let ch = builds_channel(&s, me, id).await?;
    let ids = batch_ids(req.message_ids)?;
    crate::access::require_open(&ch)?;
    if ids.is_empty() {
        return Ok(Json(BuildsBatchResult {
            done: 0,
            skipped: 0,
        }));
    }
    let everyone = owns(&s, &ch, me).await?;
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    crate::access::channel_live_in(&mut tx, id).await?;
    crate::access::open_in(&mut tx, id).await?;
    let allowed: Vec<Held> = builds_in(&mut tx, id, &ids)
        .await?
        .into_iter()
        .filter(|b| everyone || b.author == Some(me))
        .collect();
    let change: Vec<MessageId> = allowed
        .iter()
        .filter(|b| b.archived != req.archived)
        .map(|b| b.id)
        .collect();
    let at = now();
    if !change.is_empty() {
        let mut up = sqlx::QueryBuilder::new("UPDATE builds SET ");
        if req.archived {
            up.push("archived_at = ")
                .push_bind(&at)
                .push(", archived_by = ")
                .push_bind(me.to_string());
        } else {
            up.push("archived_at = NULL, archived_by = NULL");
        }
        up.push(" WHERE message_id IN (");
        let mut list = up.separated(", ");
        for m in &change {
            list.push_bind(m.to_string());
        }
        up.push(")");
        up.build().execute(&mut *tx).await?;
    }
    tx.commit().await?;
    if !change.is_empty() {
        let events = if req.archived {
            vec![Event::BuildsArchived {
                channel_id: id,
                message_ids: change,
                archived_at: at,
                archived_by: me,
            }]
        } else {
            let mut events = Vec::with_capacity(change.len());
            for m in change {
                if let Some(message) = load(&s.db, None, m).await? {
                    events.push(Event::MessageUpdated { message });
                }
            }
            events
        };
        s.hub.publish_all(&s.db, events).await;
    }
    drop(_order);
    let done = allowed.len() as u32;
    Ok(Json(BuildsBatchResult {
        done,
        skipped: ids.len() as u32 - done,
    }))
}

/// `POST /channels/{id}/builds/delete-many`: delete up to 100 of the channel's builds for good in one transaction,
/// each only if the caller may (its author, or the server's owner); the rest are skipped. Each sends the single
/// route's `MessageDeleted` (and its comments').
async fn delete_many(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<DeleteBuildsRequest>,
) -> AppResult<Json<BuildsBatchResult>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let ch = builds_channel(&s, me, id).await?;
    let ids = batch_ids(req.message_ids)?;
    if ids.is_empty() {
        return Ok(Json(BuildsBatchResult {
            done: 0,
            skipped: 0,
        }));
    }
    let everyone = owns(&s, &ch, me).await?;
    // what each event says of its message (author, mentions), read before the write; the write checks them again
    let mut held: HashMap<MessageId, Message> = HashMap::new();
    for m in &ids {
        if let Some(msg) = load(&s.db, None, *m).await?
            && msg.channel_id == id
            && !msg.deleted
        {
            held.insert(*m, msg);
        }
    }
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    crate::access::channel_live_in(&mut tx, id).await?;
    let mut done = 0u32;
    let mut events = vec![];
    let mut files = vec![];
    for b in builds_in(&mut tx, id, &ids).await? {
        if !(everyone || b.author == Some(me)) {
            continue;
        }
        let Some(msg) = held.get(&b.id) else { continue };
        if let Some((mut ev, mut f)) = delete_in(&mut tx, msg).await? {
            done += 1;
            events.append(&mut ev);
            files.append(&mut f);
        }
    }
    tx.commit().await?;
    s.hub.publish_all(&s.db, events).await;
    drop(_order);
    crate::attachments::remove_objects(&s, files).await;
    Ok(Json(BuildsBatchResult {
        done,
        skipped: ids.len() as u32 - done,
    }))
}

#[derive(Deserialize)]
struct FacetsQuery {
    #[serde(default)]
    archived: bool,
}

/// `GET /channels/{id}/builds/facets`: the channel's live builds counted per game (most first), and in all (`total`
/// counts the ones with no game too). Archived ones only with `archived=true`.
async fn facets(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Query(q): Query<FacetsQuery>,
) -> AppResult<Json<BuildFacets>> {
    builds_channel(&s, me, id).await?;
    let archived = if q.archived {
        ""
    } else {
        " AND b.archived_at IS NULL"
    };
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT COUNT(*) FROM builds b JOIN messages m ON m.id = b.message_id AND m.deleted_at IS NULL
         WHERE b.channel_id = ?{archived}"
    )))
    .bind(id.to_string())
    .fetch_one(&s.db)
    .await?;
    let games: Vec<(String, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT b.game_id, COUNT(*) AS n FROM builds b
         JOIN messages m ON m.id = b.message_id AND m.deleted_at IS NULL
         JOIN games g ON g.id = b.game_id
         WHERE b.channel_id = ?{archived}
         GROUP BY b.game_id ORDER BY n DESC, g.name_key, g.id"
    )))
    .bind(id.to_string())
    .fetch_all(&s.db)
    .await?;
    Ok(Json(BuildFacets {
        total: total as u32,
        games: games
            .into_iter()
            .map(|(g, n)| {
                Ok(GameCount {
                    game_id: g.parse().map_err(anyhow::Error::from)?,
                    count: n as u32,
                })
            })
            .collect::<AppResult<_>>()?,
    }))
}
