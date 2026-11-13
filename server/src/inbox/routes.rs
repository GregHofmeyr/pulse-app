//! `GET /inbox`, `GET /inbox/unread`, `POST /inbox/{message_id}/read`, `POST /inbox/read-all` (#306).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use pulse_protocol::ids::MessageId;
use pulse_protocol::rest::{InboxPage, InboxReadAllRequest, InboxUnread};
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
        .route("/inbox/{message_id}/read", post(read_one))
}

#[derive(Deserialize)]
struct PageQuery {
    before: Option<MessageId>,
    limit: Option<i64>,
    /// The Mentions tab: typed mentions only.
    #[serde(default)]
    mentions: bool,
}

async fn page(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Query(q): Query<PageQuery>,
) -> AppResult<Json<InboxPage>> {
    let items = list(
        &s.db,
        me,
        Filter {
            before: q.before,
            mentions_only: q.mentions,
            only: None,
            limit: q.limit.unwrap_or(PAGE).clamp(1, MAX_PAGE),
        },
    )
    .await?;
    Ok(Json(InboxPage {
        items,
        unread: unread(&s.db, me).await?,
    }))
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

/// Mark all read: every item up to `upto` (never past now; none: now). An older mark than the one you have
/// changes nothing. `204` either way.
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
    if moved > 0 {
        tell(&s.db, &s.hub, me, true, |n| {
            updated(me, n, vec![], Some(upto), vec![])
        })
        .await;
    }
    Ok(StatusCode::NO_CONTENT)
}
