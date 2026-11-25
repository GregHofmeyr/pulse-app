//! Build favourites (#617): each person's own stars on builds. A build's `favourite` is the caller's (gateway events
//! carry `false`); starring needs only membership of the build's channel, and is told only to the person's own
//! sessions (`BuildFavouriteChanged`, never to a version-4 client: `compat`).

use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{post, put};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{BuildsBatchResult, FavouriteBuildsRequest};
use sqlx::{SqliteConnection, SqlitePool};

use super::archive::{batch_ids, builds_in};
use super::routes::builds_channel;
use crate::AppState;
use crate::auth::AuthUser;
use crate::db::{begin_write, now};
use crate::error::{AppError, AppResult};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/builds/{message_id}/favourite", put(star).delete(unstar))
        .route("/channels/{id}/builds/favourite-many", post(many))
}

/// Which of `ids` (builds' message ids) `me` has starred.
pub async fn starred(db: &SqlitePool, me: UserId, ids: &[String]) -> AppResult<HashSet<String>> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let mut q = sqlx::QueryBuilder::new("SELECT message_id FROM build_favourites WHERE user_id = ");
    q.push_bind(me.to_string()).push(" AND message_id IN (");
    let mut list = q.separated(", ");
    for id in ids {
        list.push_bind(id);
    }
    q.push(")");
    let rows: Vec<String> = q.build_query_scalar().fetch_all(db).await?;
    Ok(rows.into_iter().collect())
}

/// Stars (or unstars) live builds for `me`; a build gone meanwhile is left alone. How many stars changed.
async fn set_in(
    c: &mut SqliteConnection,
    me: UserId,
    ids: &[MessageId],
    on: bool,
) -> AppResult<u64> {
    let mut changed = 0;
    for id in ids {
        let q = if on {
            sqlx::query(
                "INSERT OR IGNORE INTO build_favourites (user_id, message_id, created_at)
                 SELECT ?, message_id, ? FROM builds WHERE message_id = ?",
            )
            .bind(me.to_string())
            .bind(now())
        } else {
            sqlx::query("DELETE FROM build_favourites WHERE user_id = ? AND message_id = ?")
                .bind(me.to_string())
        };
        changed += q
            .bind(id.to_string())
            .execute(&mut *c)
            .await?
            .rows_affected();
    }
    Ok(changed)
}

async fn one(s: AppState, me: UserId, id: MessageId, on: bool) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let (channel, _) = super::build_channel(&s.db, id).await?;
    builds_channel(&s, me, channel).await?;
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    let changed = set_in(&mut tx, me, &[id], on).await?;
    tx.commit().await?;
    // starred already, not starred, or the build gone meanwhile: nothing to tell
    if changed == 0 {
        return Ok(StatusCode::NO_CONTENT);
    }
    s.hub
        .publish(
            &s.db,
            Event::BuildFavouriteChanged {
                user_id: me,
                channel_id: channel,
                message_ids: vec![id],
                favourite: on,
            },
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}

/// `PUT /builds/{id}/favourite`: star a build for yourself (any build of a channel you're in, archived ones too).
async fn star(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<StatusCode> {
    one(s, me, id, true).await
}

/// `DELETE /builds/{id}/favourite`: unstar it.
async fn unstar(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<StatusCode> {
    one(s, me, id, false).await
}

/// `POST /channels/{id}/builds/favourite-many`: star (or unstar) up to 100 of the channel's builds for yourself in one
/// write; any that aren't its live builds are skipped. One `BuildFavouriteChanged` names the ones done.
async fn many(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<FavouriteBuildsRequest>,
) -> AppResult<Json<BuildsBatchResult>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    builds_channel(&s, me, id).await?;
    let ids = batch_ids(req.message_ids)?;
    if ids.is_empty() {
        return Ok(Json(BuildsBatchResult {
            done: 0,
            skipped: 0,
        }));
    }
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    let done: Vec<MessageId> = builds_in(&mut tx, id, &ids)
        .await?
        .into_iter()
        .map(|b| b.id)
        .collect();
    set_in(&mut tx, me, &done, req.favourite).await?;
    tx.commit().await?;
    let n = done.len() as u32;
    if !done.is_empty() {
        s.hub
            .publish(
                &s.db,
                Event::BuildFavouriteChanged {
                    user_id: me,
                    channel_id: id,
                    message_ids: done,
                    favourite: req.favourite,
                },
            )
            .await;
    }
    Ok(Json(BuildsBatchResult {
        done: n,
        skipped: ids.len() as u32 - n,
    }))
}
