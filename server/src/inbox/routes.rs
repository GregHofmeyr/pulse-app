//! `GET /inbox`, `GET /inbox/unread`, `POST /inbox/{message_id}/read`, `POST /inbox/read-all` (#306), and
//! `GET /inbox/threads` (#326). `GET /inbox` takes a custom filter (#307): `from`, `in` (comma-separated ids) and
//! `unread`.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use pulse_protocol::ids::MessageId;
use pulse_protocol::rest::{
    InboxPage, InboxReadAllRequest, InboxThreads, InboxUnread, inbox_filter_limits,
};
use serde::Deserialize;

use super::{Filter, item, list, tell, unread, updated};
use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

/// Items per page unless the client asks for fewer (at most [`MAX_PAGE`]).
pub const PAGE: i64 = 30;
pub const MAX_PAGE: i64 = 50;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/inbox", get(page))
        .route("/inbox/unread", get(count))
        .route("/inbox/read-all", post(read_all))
        .route("/inbox/threads", get(threads))
        .route("/inbox/{message_id}/read", post(read_one))
}

#[derive(Deserialize)]
struct PageQuery {
    before: Option<MessageId>,
    limit: Option<i64>,
    /// The Mentions tab: typed mentions only.
    #[serde(default)]
    mentions: bool,
    /// A custom filter (#307): only from these people (comma-separated ids).
    from: Option<String>,
    /// Only in these chats (comma-separated ids).
    #[serde(rename = "in")]
    in_channels: Option<String>,
    /// Only unread items.
    #[serde(default)]
    unread: bool,
}

/// A comma-separated list of ids, at most [`inbox_filter_limits::MAX_PER_FILTER`]; a malformed one is a 400.
fn ids<T: std::str::FromStr>(raw: Option<&str>) -> AppResult<Vec<T>> {
    let list: Vec<&str> = raw
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .collect();
    if list.len() > inbox_filter_limits::MAX_PER_FILTER {
        return Err(AppError::BadRequest("too many ids in a filter".into()));
    }
    list.into_iter()
        .map(|x| {
            x.parse()
                .map_err(|_| AppError::BadRequest("not an id".into()))
        })
        .collect()
}

async fn page(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Query(q): Query<PageQuery>,
    headers: axum::http::HeaderMap,
) -> AppResult<Json<InboxPage>> {
    let mut items = list(
        &s.db,
        me,
        Filter {
            before: q.before,
            mentions_only: q.mentions,
            only: None,
            limit: q.limit.unwrap_or(PAGE).clamp(1, MAX_PAGE),
            authors: ids(q.from.as_deref())?,
            channels: ids(q.in_channels.as_deref())?,
            unread_only: q.unread,
        },
    )
    .await?;
    if !crate::builds::compat::speaks_builds(crate::builds::compat::rest_version(&headers)) {
        // an older client can't open a builds channel (#616)
        let builds =
            crate::builds::compat::builds_among(&s.db, items.iter().map(|i| i.message.channel_id))
                .await?;
        items.retain(|i| !builds.contains(&i.message.channel_id));
    }
    Ok(Json(InboxPage {
        items,
        unread: unread(&s.db, me).await?,
    }))
}

/// `GET /inbox/threads` (#326): reply threads you're part of with replies you haven't seen.
async fn threads(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    headers: axum::http::HeaderMap,
) -> AppResult<Json<InboxThreads>> {
    let mut list = super::threads::list(&s.db, me).await?;
    if !crate::builds::compat::speaks_builds(crate::builds::compat::rest_version(&headers)) {
        // an older client can't open a builds channel (#616): no build's thread, nor builds among what it watches
        let builds =
            crate::builds::compat::builds_among(&s.db, list.threads.iter().map(|t| t.channel_id))
                .await?;
        list.threads.retain(|t| !builds.contains(&t.channel_id));
        let mut roots = std::collections::HashSet::new();
        for id in &list.watching {
            let hit: Option<i64> = sqlx::query_scalar("SELECT 1 FROM builds WHERE message_id = ?")
                .bind(id.to_string())
                .fetch_optional(&s.db)
                .await?;
            if hit.is_some() {
                roots.insert(*id);
            }
        }
        list.watching.retain(|id| !roots.contains(id));
    }
    Ok(Json(list))
}

async fn count(State(s): State<AppState>, AuthUser(me): AuthUser) -> AppResult<Json<InboxUnread>> {
    Ok(Json(InboxUnread {
        unread: unread(&s.db, me).await?,
    }))
}

/// Opened from the inbox: read on every device. 404 unless it is one of your items; `204` also when it was
/// already opened (nothing is published then).
async fn read_one(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    if item(&s.db, me, id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    let added =
        sqlx::query("INSERT OR IGNORE INTO inbox_reads (user_id, message_id) VALUES (?, ?)")
            .bind(me.to_string())
            .bind(id.to_string())
            .execute(&s.db)
            .await?
            .rows_affected();
    if added > 0 {
        // The event carries an absolute count: read and sent under your inbox slot (#327).
        tell(&s.db, &s.hub, me, true, |n| {
            updated(me, n, vec![id], None, vec![])
        })
        .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Mark all read: every item up to `upto` (never past now; none: now), and every reminder that's come due (#300). An
/// older mark than the one you have changes nothing. `204` either way.
async fn read_all(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(req): Json<InboxReadAllRequest>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let now = MessageId::new();
    let upto = req.upto.map_or(now, |u| u.min(now));
    let moved = sqlx::query(
        "INSERT INTO inbox_cleared (user_id, upto) VALUES (?, ?)
         ON CONFLICT (user_id) DO UPDATE SET upto = excluded.upto
         WHERE excluded.upto > inbox_cleared.upto",
    )
    .bind(me.to_string())
    .bind(upto.to_string())
    .execute(&s.db)
    .await?
    .rows_affected();
    // due reminders (#300) are read with everything else
    let reminders = crate::reminders::mark_all_seen(&s, me).await?;
    if moved > 0 || reminders {
        tell(&s.db, &s.hub, me, true, |n| {
            updated(me, n, vec![], Some(upto), vec![])
        })
        .await;
    }
    Ok(StatusCode::NO_CONTENT)
}
