//! Archiving builds (#616): one at a time or in bulk, never deleting; deleting one for good; and the per-game counts
//! the channel's panel shows. Archiving, restoring and deleting are for the build's author and the server's owner.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, GameId, MessageId, UserId};
use pulse_protocol::rest::{
    BuildBrief, BuildFacets, BulkArchiveRequest, BulkArchiveResult, Channel, GameCount, Message,
    SetBuildArchivedRequest,
};
use serde::Deserialize;

use super::routes::{bad, builds_channel};
use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::{begin_write, now};
use crate::error::{AppError, AppResult};
use crate::messages::routes::load;

/// The most builds one bulk archive takes.
pub const BULK_MAX: usize = 500;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/builds/{message_id}/archived", put(set_archived))
        .route("/channels/{id}/builds/archive", post(bulk))
        .route("/channels/{id}/builds/facets", get(facets))
}

/// A live build `me` may see, with its channel, and whether `me` may archive or delete it (its author, or the
/// server's owner). 404 for anyone who can't see it.
async fn build_for(s: &AppState, me: UserId, id: MessageId) -> AppResult<(Message, Channel, bool)> {
    let (channel, _) = super::build_channel(&s.db, id).await?;
    let ch = channel_for(&s.db, me, channel).await?;
    let msg = load(&s.db, Some(me), id).await?.ok_or(AppError::NotFound)?;
    if msg.build.is_none() || msg.deleted {
        return Err(AppError::NotFound);
    }
    let allowed = msg.author_id == Some(me) || owns(s, &ch, me).await?;
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
    let changed = sqlx::query(
        "UPDATE messages SET content = '', gif = NULL, deleted_at = ? WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(now())
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    sqlx::query("DELETE FROM reactions WHERE message_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM builds WHERE message_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    let mut replies = crate::messages::threads::soft_delete_replies(&mut tx, id).await?;
    crate::embeds::on_delete(&mut tx, id).await?;
    crate::messages::history::on_delete(&mut tx, id).await?;
    let files = crate::attachments::stored_of_message(&mut tx, id).await?;
    let unpinned = crate::pins::on_delete(&mut tx, id).await?;
    tx.commit().await?;
    let mut events = vec![Event::MessageDeleted {
        channel_id: msg.channel_id,
        message_id: id,
        author_id: msg.author_id,
        mentions: msg.mentions,
        thread_root_id: None,
    }];
    if unpinned {
        events.push(Event::MessageUnpinned {
            channel_id: msg.channel_id,
            message_id: id,
        });
    }
    events.append(&mut replies);
    s.hub.publish_all(&s.db, events).await;
    drop(_order);
    crate::attachments::remove_objects(&s, files).await;
    Ok(StatusCode::NO_CONTENT)
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
