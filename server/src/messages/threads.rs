//! Replies to a quote: a thread per quote, kept out of the channel's list, unread counts and
//! scores. A reply is an ordinary message whose `thread_root_id` names the quote.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{Channel, Message, ThreadSeenRequest};
use sqlx::{SqliteConnection, SqlitePool};

use super::routes::{COLS, Page, Row, fill};
use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::servers::routes::is_server_member;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/messages/{id}/replies", get(list))
        .route("/messages/{id}/replies/seen", post(seen))
}

/// Is `root` a live quote in `channel`?
pub(crate) async fn is_live_quote(
    db: impl sqlx::SqliteExecutor<'_>,
    root: MessageId,
    channel: ChannelId,
) -> AppResult<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM quotes q JOIN messages m ON m.id = q.message_id
         WHERE m.id = ? AND m.channel_id = ? AND m.deleted_at IS NULL",
    )
    .bind(root.to_string())
    .bind(channel.to_string())
    .fetch_optional(db)
    .await?
    .is_some())
}

/// The channel of a live quote the caller may see, else 404.
async fn visible_quote(db: &SqlitePool, me: UserId, root: MessageId) -> AppResult<Channel> {
    let channel: String = sqlx::query_scalar(
        "SELECT m.channel_id FROM quotes q JOIN messages m ON m.id = q.message_id
         WHERE m.id = ? AND m.deleted_at IS NULL",
    )
    .bind(root.to_string())
    .fetch_optional(db)
    .await?
    .ok_or(AppError::NotFound)?;
    let channel: ChannelId = channel.parse().map_err(anyhow::Error::from)?;
    channel_for(db, me, channel).await
}

/// Move `user`'s seen point in `root`'s thread forward to `upto` (never backwards). `None` if it
/// didn't move; otherwise how many of their mentions in replies it just covered.
pub(crate) async fn mark_seen(
    c: &mut SqliteConnection,
    user: UserId,
    root: MessageId,
    upto: MessageId,
) -> AppResult<Option<u32>> {
    let old: Option<String> = sqlx::query_scalar(
        "SELECT last_seen_id FROM thread_reads WHERE user_id = ? AND root_id = ?",
    )
    .bind(user.to_string())
    .bind(root.to_string())
    .fetch_optional(&mut *c)
    .await?;
    let old = old.unwrap_or_default();
    if old >= upto.to_string() {
        return Ok(None);
    }
    sqlx::query(
        "INSERT INTO thread_reads (user_id, root_id, last_seen_id) VALUES (?, ?, ?)
         ON CONFLICT (user_id, root_id) DO UPDATE SET last_seen_id = excluded.last_seen_id",
    )
    .bind(user.to_string())
    .bind(root.to_string())
    .bind(upto.to_string())
    .execute(&mut *c)
    .await?;
    let cleared: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM mentions mn JOIN messages m ON m.id = mn.message_id
         WHERE mn.user_id = ? AND m.thread_root_id = ? AND m.deleted_at IS NULL
           AND m.id > ? AND m.id <= ?",
    )
    .bind(user.to_string())
    .bind(root.to_string())
    .bind(old)
    .bind(upto.to_string())
    .fetch_one(&mut *c)
    .await?;
    Ok(Some(cleared as u32))
}

async fn list(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(root): Path<MessageId>,
    Query(p): Query<Page>,
) -> AppResult<Json<Vec<Message>>> {
    visible_quote(&s.db, me, root).await?;
    let (limit, before) = p.bounds();
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM messages WHERE thread_root_id = ? AND id < ? ORDER BY id DESC LIMIT ?"
    ))
    .bind(root.to_string())
    .bind(before)
    .bind(limit)
    .fetch_all(&s.db)
    .await?;
    Ok(Json(fill(&s.db, Some(me), rows).await?))
}

/// `204` also when the point was already at or past `message_id`: a no-op that publishes nothing.
async fn seen(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(root): Path<MessageId>,
    Json(req): Json<ThreadSeenRequest>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let channel = visible_quote(&s.db, me, root).await?;
    if let Some(server) = channel.server_id
        && !is_server_member(&s.db, server, me).await?
    {
        return Err(AppError::Forbidden);
    }
    // A deleted reply still counts as a place in the thread (it may have been the newest).
    let is_reply =
        sqlx::query_scalar::<_, i64>("SELECT 1 FROM messages WHERE id = ? AND thread_root_id = ?")
            .bind(req.message_id.to_string())
            .bind(root.to_string())
            .fetch_optional(&s.db)
            .await?
            .is_some();
    if !is_reply {
        return Err(AppError::BadRequest("not a reply to this quote".into()));
    }
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let cleared = mark_seen(&mut tx, me, root, req.message_id).await?;
    tx.commit().await?;
    if let Some(mentions_cleared) = cleared
        && let Some((_, mentions)) = crate::reads::counts_after_commit(&s.db, me, channel.id).await
    {
        s.hub
            .publish(
                &s.db,
                Event::ThreadSeen {
                    user_id: me,
                    channel_id: channel.id,
                    root_id: root,
                    last_seen_id: req.message_id,
                    mentions_cleared,
                    mentions: Some(mentions),
                },
            )
            .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Soft-delete every live reply in `root`'s thread and return the `MessageDeleted` events to
/// publish once the transaction commits.
pub(crate) async fn soft_delete_replies(
    tx: &mut SqliteConnection,
    root: MessageId,
) -> AppResult<Vec<Event>> {
    let id = root.to_string();
    let replies: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "UPDATE messages SET content = '', deleted_at = ?
         WHERE thread_root_id = ? AND deleted_at IS NULL
         RETURNING id, channel_id, author_id",
    )
    .bind(crate::db::now())
    .bind(&id)
    .fetch_all(&mut *tx)
    .await?;
    let mut mentioned: std::collections::HashMap<String, Vec<UserId>> =
        std::collections::HashMap::new();
    for (msg, user) in sqlx::query_as::<_, (String, String)>(
        "SELECT mt.message_id, mt.user_id FROM mentions mt JOIN messages m ON m.id = mt.message_id
         WHERE m.thread_root_id = ? ORDER BY mt.rowid",
    )
    .bind(&id)
    .fetch_all(&mut *tx)
    .await?
    {
        mentioned
            .entry(msg)
            .or_default()
            .push(user.parse().map_err(anyhow::Error::from)?);
    }
    let replies: Vec<Event> = replies
        .into_iter()
        .map(|(reply, channel, author)| {
            Ok(Event::MessageDeleted {
                channel_id: channel.parse().map_err(anyhow::Error::from)?,
                author_id: author
                    .as_deref()
                    .map(str::parse)
                    .transpose()
                    .map_err(anyhow::Error::from)?,
                mentions: mentioned.remove(&reply).unwrap_or_default(),
                message_id: reply.parse().map_err(anyhow::Error::from)?,
                thread_root_id: Some(root),
            })
        })
        .collect::<AppResult<_>>()?;
    Ok(replies)
}
