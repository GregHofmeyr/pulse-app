//! Saved messages (#315): "Save message" on any message you can see keeps a private bookmark, with an optional note,
//! listed in your Saved view. Only its person ever sees it, and their devices hear every change (`BookmarkSaved`,
//! `BookmarkRemoved`), so each agrees. One whose message is deleted, or that they can no longer see (they left the DM
//! or group, its chat or server was deleted), stays listed with no message (never its content) until they remove it.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{BookmarkId, MessageId, UserId};
use pulse_protocol::rest::{
    EditBookmarkRequest, MessageBookmark, SaveMessageRequest, SavedMessage, bookmark_limits,
};
use sqlx::{SqliteExecutor, SqlitePool};

use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::{begin_write, now};
use crate::error::{AppError, AppResult};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/bookmarks", get(list))
        .route("/messages/{id}/bookmark", post(create))
        .route("/bookmarks/{id}", delete(remove).patch(edit))
}

/// A person's bookmarks with each message's parts, and whether they can still see it: not deleted, in a chat that
/// isn't in Recently deleted (nor its server), a DM or group they're still in (the rule reminders use), and a server
/// they're still in (#371: someone who left sees its messages as unavailable until they rejoin). Binds: the person.
const SELECT: &str = "SELECT b.id, b.message_id, b.note, b.created_at,
       (m.id IS NOT NULL AND m.deleted_at IS NULL AND c.id IS NOT NULL AND c.deleted_at IS NULL
        AND (c.server_id IS NULL OR c.server_id NOT IN (SELECT id FROM servers WHERE deleted_at IS NOT NULL))
        AND (c.server_id IS NOT NULL OR EXISTS (SELECT 1 FROM channel_members cm
                                                WHERE cm.channel_id = c.id AND cm.user_id = b.user_id))
        AND (c.server_id IS NULL OR EXISTS (SELECT 1 FROM server_members sm
                                            WHERE sm.server_id = c.server_id AND sm.user_id = b.user_id))) AS visible,
       m.channel_id, m.thread_root_id, m.author_id, m.content, m.created_at
    FROM message_bookmarks b
    LEFT JOIN messages m ON m.id = b.message_id
    LEFT JOIN channels c ON c.id = m.channel_id
    WHERE b.user_id = ?";

type Row = (
    String,
    String,
    Option<String>,
    String,
    bool,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn parse<T: std::str::FromStr>(s: &str) -> AppResult<T>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    s.parse::<T>().map_err(|e| AppError::Internal(e.into()))
}

fn from_row(r: Row) -> AppResult<MessageBookmark> {
    let (id, message_id, note, created_at, visible, channel, thread, author, content, sent_at) = r;
    let message = match (visible, channel, content, sent_at) {
        (true, Some(channel), Some(content), Some(sent_at)) => Some(SavedMessage {
            channel_id: parse(&channel)?,
            thread_root_id: thread.as_deref().map(parse).transpose()?,
            author_id: author.as_deref().map(parse).transpose()?,
            content: content
                .chars()
                .take(bookmark_limits::SNIPPET_CHARS)
                .collect(),
            created_at: sent_at,
        }),
        _ => None,
    };
    Ok(MessageBookmark {
        id: parse(&id)?,
        message_id: parse(&message_id)?,
        note,
        created_at,
        message,
    })
}

/// `me`'s saved messages, newest first: for Ready and `GET /bookmarks`.
pub async fn list_for(db: &SqlitePool, me: UserId) -> AppResult<Vec<MessageBookmark>> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{SELECT} ORDER BY b.created_at DESC, b.id DESC"
    )))
    .bind(me.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter().map(from_row).collect()
}

/// `me`'s bookmark `id`.
async fn one<'e>(
    db: impl SqliteExecutor<'e>,
    me: UserId,
    id: BookmarkId,
) -> AppResult<Option<MessageBookmark>> {
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!("{SELECT} AND b.id = ?")))
        .bind(me.to_string())
        .bind(id.to_string())
        .fetch_optional(db)
        .await?;
    row.map(from_row).transpose()
}

/// A note as kept: trimmed, at most [`bookmark_limits::NOTE_MAX_CHARS`]; empty is none.
fn clean_note(note: Option<String>) -> AppResult<Option<String>> {
    let Some(n) = note else { return Ok(None) };
    let n: String = n
        .trim()
        .chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .collect();
    if n.chars().count() > bookmark_limits::NOTE_MAX_CHARS {
        return Err(AppError::BadRequest(format!(
            "a note can be up to {} characters",
            bookmark_limits::NOTE_MAX_CHARS
        )));
    }
    Ok((!n.is_empty()).then_some(n))
}

async fn list(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    headers: axum::http::HeaderMap,
) -> AppResult<Json<Vec<MessageBookmark>>> {
    let mut list = list_for(&s.db, me).await?;
    if !crate::builds::compat::speaks_builds(crate::builds::compat::rest_version(&headers)) {
        // an older client can't open a builds channel (#616)
        let builds = crate::builds::compat::builds_among(
            &s.db,
            list.iter()
                .filter_map(|b| b.message.as_ref().map(|m| m.channel_id)),
        )
        .await?;
        list.retain(|b| {
            b.message
                .as_ref()
                .is_none_or(|m| !builds.contains(&m.channel_id))
        });
    }
    Ok(Json(list))
}

async fn publish_saved(s: &AppState, me: UserId, bookmark: MessageBookmark) {
    s.hub
        .publish(
            &s.db,
            Event::BookmarkSaved {
                user_id: me,
                bookmark,
            },
        )
        .await;
}

/// `POST /messages/{id}/bookmark`: `201` with the new bookmark, or `200` with the one you already have (unchanged).
/// Any live message you can see (404 otherwise). At most [`bookmark_limits::MAX_BOOKMARKS`] each (checked in the
/// write).
async fn create(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(message): Path<MessageId>,
    Json(req): Json<SaveMessageRequest>,
) -> AppResult<(StatusCode, Json<MessageBookmark>)> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let note = clean_note(req.note)?;
    let channel: Option<String> =
        sqlx::query_scalar("SELECT channel_id FROM messages WHERE id = ? AND deleted_at IS NULL")
            .bind(message.to_string())
            .fetch_optional(&s.db)
            .await?;
    let channel = parse(&channel.ok_or(AppError::NotFound)?)?;
    let ch = channel_for(&s.db, me, channel).await?;
    // a server's messages: as a member (#371), as `SELECT` shows them
    if let Some(server) = ch.server_id
        && !crate::servers::routes::is_server_member(&s.db, server, me).await?
    {
        return Err(AppError::NotFound);
    }
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    let have: Option<String> =
        sqlx::query_scalar("SELECT id FROM message_bookmarks WHERE user_id = ? AND message_id = ?")
            .bind(me.to_string())
            .bind(message.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(have) = have {
        let b = one(&mut *tx, me, parse(&have)?)
            .await?
            .ok_or(AppError::NotFound)?;
        return Ok((StatusCode::OK, Json(b)));
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM message_bookmarks WHERE user_id = ?")
        .bind(me.to_string())
        .fetch_one(&mut *tx)
        .await?;
    if count >= bookmark_limits::MAX_BOOKMARKS {
        return Err(AppError::BadRequest(format!(
            "you can save up to {} messages",
            bookmark_limits::MAX_BOOKMARKS
        )));
    }
    crate::access::channel_live_in(&mut tx, channel).await?;
    let id = BookmarkId::new();
    sqlx::query(
        "INSERT INTO message_bookmarks (id, user_id, message_id, note, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(me.to_string())
    .bind(message.to_string())
    .bind(&note)
    .bind(now())
    .execute(&mut *tx)
    .await?;
    let made = one(&mut *tx, me, id).await?.ok_or(AppError::NotFound)?;
    // deleted between the check and the write: nothing to save
    if made.message.is_none() {
        return Err(AppError::NotFound);
    }
    tx.commit().await?;
    publish_saved(&s, me, made.clone()).await;
    Ok((StatusCode::CREATED, Json(made)))
}

/// `PATCH /bookmarks/{id}`: a new note (also on one whose message is unavailable). `200` with the bookmark.
async fn edit(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<BookmarkId>,
    Json(req): Json<EditBookmarkRequest>,
) -> AppResult<Json<MessageBookmark>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let note = clean_note(req.note)?;
    let _order = s.hub.in_order().await;
    let changed = sqlx::query("UPDATE message_bookmarks SET note = ? WHERE id = ? AND user_id = ?")
        .bind(&note)
        .bind(id.to_string())
        .bind(me.to_string())
        .execute(&s.db)
        .await?
        .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    let b = one(&s.db, me, id).await?.ok_or(AppError::NotFound)?;
    publish_saved(&s, me, b.clone()).await;
    Ok(Json(b))
}

/// `DELETE /bookmarks/{id}`: `204`.
async fn remove(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<BookmarkId>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let _order = s.hub.in_order().await;
    let gone = sqlx::query("DELETE FROM message_bookmarks WHERE id = ? AND user_id = ?")
        .bind(id.to_string())
        .bind(me.to_string())
        .execute(&s.db)
        .await?
        .rows_affected();
    if gone == 0 {
        return Err(AppError::NotFound);
    }
    s.hub
        .publish(&s.db, Event::BookmarkRemoved { user_id: me, id })
        .await;
    Ok(StatusCode::NO_CONTENT)
}
