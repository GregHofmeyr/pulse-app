//! View-once files (#311). A file sent "view once" (`attachments.view_once`, set when its upload starts) can be opened
//! once by each person who can see the message: `POST /messages/{id}/view-once/open` records that they opened it and
//! answers with links to the message's view-once files. Those links are the only ones the server ever signs for
//! them: `GET /messages/{id}/attachments/urls` leaves them out (for anyone but the sender), and a second open is
//! `410 Gone`. A view-once message can't be forwarded.
//!
//! The limits, said plainly to people in the client: a signed link can't be taken back once given, so it works for
//! [`OPEN_TTL`] (longer for a long video, up to [`OPEN_TTL_MAX`]) after the open, and it's marked not to be cached.
//! Nothing stops a screenshot, a screen recording or a modified client keeping the bytes it was sent.
//!
//! `GET /messages/{id}/view-once`: whether you opened them; for the sender, who did and when. Each open is told
//! (`ViewOnceOpened`) to the viewer's sessions and the sender's.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{AttachmentUrl, ViewOnceState, ViewOnceViewer};
use sqlx::SqlitePool;

use super::{object_key, storage};
use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::servers::routes::is_server_member;

/// How long an opened file's link works: long enough to load a picture or a short clip.
pub const OPEN_TTL: Duration = Duration::from_secs(120);
/// The longest an opened file's link works (a long video is given its length plus a minute, up to this).
pub const OPEN_TTL_MAX: Duration = Duration::from_secs(30 * 60);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/messages/{id}/view-once", get(state))
        .route("/messages/{id}/view-once/open", post(open))
}

/// Who sent `message` (None for a system line or one not there).
pub async fn sender_of(db: &SqlitePool, message: MessageId) -> AppResult<Option<UserId>> {
    let author: Option<Option<String>> =
        sqlx::query_scalar("SELECT author_id FROM messages WHERE id = ?")
            .bind(message.to_string())
            .fetch_optional(db)
            .await?;
    author
        .flatten()
        .map(|a| a.parse().map_err(|e| anyhow::Error::from(e).into()))
        .transpose()
}

/// A live message `me` can see, with view-once files: its channel and sender.
async fn visible(
    s: &AppState,
    me: UserId,
    id: MessageId,
) -> AppResult<(ChannelId, Option<UserId>)> {
    // a DM withheld from `me` (#474) isn't there for them
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT channel_id, author_id FROM messages WHERE id = ? AND deleted_at IS NULL
           AND (withheld_from IS NULL OR withheld_from != ?)",
    )
    .bind(id.to_string())
    .bind(me.to_string())
    .fetch_optional(&s.db)
    .await?;
    let (channel, author) = row.ok_or(AppError::NotFound)?;
    let channel: ChannelId = channel.parse().map_err(anyhow::Error::from)?;
    let ch = channel_for(&s.db, me, channel).await?;
    if let Some(server) = ch.server_id
        && !is_server_member(&s.db, server, me).await?
    {
        return Err(AppError::NotFound);
    }
    let any: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM attachments WHERE message_id = ? AND view_once = 1 LIMIT 1",
    )
    .bind(id.to_string())
    .fetch_optional(&s.db)
    .await?;
    if any.is_none() {
        return Err(AppError::NotFound);
    }
    let author = author
        .map(|a| a.parse::<UserId>())
        .transpose()
        .map_err(anyhow::Error::from)?;
    Ok((channel, author))
}

/// `GET /messages/{id}/view-once`.
async fn state(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<Json<ViewOnceState>> {
    let (_, author) = visible(&s, me, id).await?;
    if author == Some(me) {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT user_id, viewed_at FROM view_once_views WHERE message_id = ? ORDER BY viewed_at, user_id",
        )
        .bind(id.to_string())
        .fetch_all(&s.db)
        .await?;
        let viewers = rows
            .into_iter()
            .map(|(u, at)| {
                Ok(ViewOnceViewer {
                    user_id: u.parse().map_err(anyhow::Error::from)?,
                    viewed_at: at,
                })
            })
            .collect::<AppResult<_>>()?;
        return Ok(Json(ViewOnceState {
            opened: false,
            viewers,
        }));
    }
    let opened: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM view_once_views WHERE message_id = ? AND user_id = ?")
            .bind(id.to_string())
            .bind(me.to_string())
            .fetch_optional(&s.db)
            .await?;
    Ok(Json(ViewOnceState {
        opened: opened.is_some(),
        viewers: vec![],
    }))
}

/// How long the links of an open work: [`OPEN_TTL`], or the longest file's length plus a minute, up to
/// [`OPEN_TTL_MAX`].
pub fn open_ttl(longest_ms: Option<i64>) -> Duration {
    let by_length =
        Duration::from_millis(longest_ms.unwrap_or(0).max(0) as u64) + Duration::from_secs(60);
    by_length.max(OPEN_TTL).min(OPEN_TTL_MAX)
}

/// A view-once file as an open needs it: id, key, expires_at, purged_at, duration_ms.
type OpenRow = (String, String, Option<String>, Option<String>, Option<i64>);

/// `POST /messages/{id}/view-once/open`: the one open. Links to the message's view-once files (none for one past its
/// time); `410` when you opened them before; `400` for their sender (who sees them as usual).
async fn open(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<Json<Vec<AttachmentUrl>>> {
    if !s.limits.clip_url_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let r2 = storage(&s)?;
    let (channel, author) = visible(&s, me, id).await?;
    if author == Some(me) {
        return Err(AppError::BadRequest(
            "you sent these: they open as usual for you".into(),
        ));
    }
    let at = now();
    // the open is recorded before anything is signed: two opens at once can't both get links
    let first = sqlx::query(
        "INSERT INTO view_once_views (message_id, user_id, viewed_at) VALUES (?, ?, ?)
         ON CONFLICT (message_id, user_id) DO NOTHING",
    )
    .bind(id.to_string())
    .bind(me.to_string())
    .bind(&at)
    .execute(&s.db)
    .await?
    .rows_affected();
    if first == 0 {
        return Err(AppError::Gone);
    }
    let rows: Vec<OpenRow> = sqlx::query_as(
        "SELECT id, key, expires_at, purged_at, duration_ms FROM attachments
         WHERE message_id = ? AND view_once = 1 ORDER BY position",
    )
    .bind(id.to_string())
    .fetch_all(&s.db)
    .await?;
    let ttl = open_ttl(rows.iter().filter_map(|r| r.4).max());
    let urls = rows
        .into_iter()
        .map(|(att, key, expires_at, purged_at, _)| {
            let expired =
                purged_at.is_some() || expires_at.as_deref().is_some_and(|e| e <= at.as_str());
            let (url, until) = if expired {
                (None, None)
            } else {
                let (url, until) =
                    r2.download_url_no_store(&object_key(&key, expires_at.is_some()), ttl);
                (Some(url.to_string()), Some(until))
            };
            Ok(AttachmentUrl {
                id: att.parse().map_err(anyhow::Error::from)?,
                url,
                expires_at: until,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    for to in std::iter::once(me).chain(author) {
        s.hub
            .publish(
                &s.db,
                Event::ViewOnceOpened {
                    user_id: to,
                    channel_id: channel,
                    message_id: id,
                    viewer_id: me,
                    viewed_at: at.clone(),
                },
            )
            .await;
    }
    Ok(Json(urls))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_video_gets_its_length_up_to_the_cap() {
        assert_eq!(open_ttl(None), OPEN_TTL);
        assert_eq!(open_ttl(Some(5_000)), OPEN_TTL);
        assert_eq!(open_ttl(Some(10 * 60_000)), Duration::from_secs(11 * 60));
        assert_eq!(open_ttl(Some(5 * 60 * 60_000)), OPEN_TTL_MAX);
    }
}
