//! Per-person Do Not Disturb (#310): `PUT|DELETE /me/person-dnd/{user_id}`. Someone's messages stop alerting you (no
//! message sound, no desktop notification), everywhere or only in DMs and groups, for a while or until you turn it
//! off. Their messages still arrive, and unread and mention counts count them as before: only the alert is held
//! back, which the client decides (as it does for your own Do Not Disturb, #45).
//!
//! Kept per person (`person_dnd`) so it follows you to every device, sent only to you (Ready's `person_dnd` and
//! `PersonDndChanged`, always the whole list). A timed one past its end reads as none; clients drop it at its end on
//! their own, so nothing has to sweep it (the row goes the next time the list changes).

use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};
use jiff::Timestamp;
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{PersonDnd, PersonDndRequest};
use sqlx::SqlitePool;

use crate::AppState;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};

/// People one person may have on the list.
pub const MAX_PEOPLE: i64 = 200;

pub fn router() -> Router<AppState> {
    Router::new().route("/me/person-dnd/{id}", put(set).delete(clear))
}

/// `owner`'s list, run-out ones left out, in the order they were added.
pub async fn list(db: &SqlitePool, owner: UserId) -> AppResult<Vec<PersonDnd>> {
    let rows: Vec<(String, i64, Option<String>)> = sqlx::query_as(
        "SELECT target_id, dms_only, until FROM person_dnd
         WHERE user_id = ? AND (until IS NULL OR until > ?) ORDER BY created_at, rowid",
    )
    .bind(owner.to_string())
    .bind(now())
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|(id, dms_only, until)| {
            Ok(PersonDnd {
                user_id: id.parse().map_err(anyhow::Error::from)?,
                dms_only: dms_only != 0,
                until,
            })
        })
        .collect()
}

/// The person named: real and not you. Also the rate limit.
async fn target(s: &AppState, me: UserId, id: &str) -> AppResult<UserId> {
    // a malformed id names no user, so it is a 404 like an unknown one
    let target: UserId = id.parse().map_err(|_| AppError::NotFound)?;
    if target == me {
        return Err(AppError::BadRequest(
            "Do Not Disturb for yourself is your status.".into(),
        ));
    }
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM users WHERE id = ?")
        .bind(target.to_string())
        .fetch_optional(&s.db)
        .await?;
    if exists.is_none() {
        return Err(AppError::NotFound);
    }
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(Some(
            "You're changing Do Not Disturb too fast. Try again in a minute.".into(),
        )));
    }
    Ok(target)
}

/// `PUT /me/person-dnd/{id}`: on for them (or changed). Answers with your whole list.
async fn set(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
    Json(req): Json<PersonDndRequest>,
) -> AppResult<Json<Vec<PersonDnd>>> {
    let until = req
        .until
        .as_deref()
        .map(|u| crate::scheduled::future_time(u, Timestamp::now()))
        .transpose()?;
    let target = target(&s, me, &id).await?;
    let mut tx = crate::db::begin_write(&s.db).await?;
    // run-out rows go first, so they neither count nor linger
    sqlx::query("DELETE FROM person_dnd WHERE user_id = ? AND until IS NOT NULL AND until <= ?")
        .bind(me.to_string())
        .bind(now())
        .execute(&mut *tx)
        .await?;
    let (count, has): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(target_id = ?), 0) FROM person_dnd WHERE user_id = ?",
    )
    .bind(target.to_string())
    .bind(me.to_string())
    .fetch_one(&mut *tx)
    .await?;
    if has == 0 && count >= MAX_PEOPLE {
        return Err(AppError::BadRequest(format!(
            "Do Not Disturb can be on for up to {MAX_PEOPLE} people."
        )));
    }
    sqlx::query(
        "INSERT INTO person_dnd (user_id, target_id, dms_only, until, created_at) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (user_id, target_id) DO UPDATE SET dms_only = excluded.dms_only, until = excluded.until",
    )
    .bind(me.to_string())
    .bind(target.to_string())
    .bind(i64::from(req.dms_only))
    .bind(until)
    .bind(now())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(announce(&s, me).await?))
}

/// `DELETE /me/person-dnd/{id}`: off for them. Answers with your whole list.
async fn clear(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Vec<PersonDnd>>> {
    let target = target(&s, me, &id).await?;
    sqlx::query(
        "DELETE FROM person_dnd WHERE user_id = ? AND (target_id = ? OR (until IS NOT NULL AND until <= ?))",
    )
    .bind(me.to_string())
    .bind(target.to_string())
    .bind(now())
    .execute(&s.db)
    .await?;
    Ok(Json(announce(&s, me).await?))
}

/// Tell `me`'s own sessions their list as it is now.
async fn announce(s: &AppState, me: UserId) -> AppResult<Vec<PersonDnd>> {
    let list = list(&s.db, me).await?;
    s.hub
        .publish(
            &s.db,
            Event::PersonDndChanged {
                user_id: me,
                list: list.clone(),
            },
        )
        .await;
    Ok(list)
}
