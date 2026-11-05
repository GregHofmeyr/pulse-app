//! `PUT /me/status` and `PUT|DELETE /me/appear-offline/{id}` (#45). Each change is saved, told to
//! your own sessions only, and then re-announced per viewer through [`crate::gateway::hub::Hub::sync_presence`].

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::put;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{SetStatusRequest, Status};

use super::{NEVER_SEEN, load_subject, view_for};
use crate::AppState;
use crate::auth::AuthUser;
use crate::db::{begin_write, now, stamp};
use crate::error::{AppError, AppResult};

/// The longest DND timer the server takes (the client offers at most "until tomorrow morning").
pub const MAX_DND: chrono::Duration = chrono::Duration::days(7);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/status", put(set_status))
        .route("/me/appear-offline/{id}", put(hide).delete(unhide))
}

fn too_fast() -> AppError {
    AppError::TooManyRequests(Some(
        "You're changing your status too fast. Try again in a minute.".into(),
    ))
}

/// A DND end as stored, if it's a real time in the next [`MAX_DND`].
fn dnd_end(raw: &str, at: DateTime<Utc>) -> AppResult<String> {
    let t = DateTime::parse_from_rfc3339(raw)
        .map_err(|_| AppError::BadRequest("That end time isn't a time.".into()))?
        .with_timezone(&Utc);
    if t <= at {
        return Err(AppError::BadRequest("That end time has passed.".into()));
    }
    if t > at + MAX_DND {
        return Err(AppError::BadRequest(
            "Do Not Disturb can be timed for up to 7 days.".into(),
        ));
    }
    Ok(stamp(t))
}

async fn set_status(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(req): Json<SetStatusRequest>,
) -> AppResult<StatusCode> {
    let at = Utc::now();
    let dnd_until = match (&req.dnd_until, req.status) {
        (None, _) => None,
        (Some(raw), Status::Dnd) => Some(dnd_end(raw, at)?),
        (Some(_), _) => {
            return Err(AppError::BadRequest(
                "Only Do Not Disturb has a timer.".into(),
            ));
        }
    };
    if !s.limits.status_user.hit(&me.to_string()) {
        return Err(too_fast());
    }
    let now = stamp(at);
    // Going invisible: you stop looking online now if you're connected, else at your last disconnect,
    // (or never, if you never were), so nobody's "last seen" for you jumps. Staying invisible keeps the first moment.
    let freeze = if s.hub.is_online(me) {
        now.clone()
    } else {
        load_subject(&s.db, me)
            .await?
            .and_then(|x| x.last_seen_at)
            .unwrap_or_else(|| NEVER_SEEN.into())
    };
    let mut tx = begin_write(&s.db).await?;
    let was: String = sqlx::query_scalar("SELECT presence FROM users WHERE id = ?")
        .bind(me.to_string())
        .fetch_one(&mut *tx)
        .await?;
    let staying_invisible = was == "invisible" && req.status == Status::Invisible;
    if !staying_invisible {
        // Off-list rows only matter during the invisibility they were taken off in.
        sqlx::query("DELETE FROM presence_hidden WHERE owner_id = ? AND active = 0")
            .bind(me.to_string())
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query(
        "UPDATE users SET presence = ?, dnd_until = ?, \
         invisible_at = CASE WHEN ? = 'invisible' THEN COALESCE(CASE WHEN presence = 'invisible' THEN invisible_at END, ?) END \
         WHERE id = ?",
    )
    .bind(req.status.as_str())
    .bind(&dnd_until)
    .bind(req.status.as_str())
    .bind(&freeze)
    .bind(me.to_string())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    announce_own(&s, me).await;
    s.hub.sync_presence(&s.db, me).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Tell `me`'s own sessions their status as it now stands.
pub(super) async fn announce_own(s: &AppState, me: UserId) {
    match load_subject(&s.db, me).await {
        Ok(Some(x)) => {
            s.hub
                .publish(
                    &s.db,
                    Event::StatusChanged {
                        user_id: me,
                        status: x.own(&now()),
                    },
                )
                .await
        }
        Ok(None) => {}
        Err(e) => tracing::error!(error = ?e, "status not announced to its owner"),
    }
}

/// The person a hide/unhide names: real and not you.
async fn target(s: &AppState, me: UserId, id: &str) -> AppResult<UserId> {
    // A malformed id names no user, so it is a 404 like an unknown one.
    let target: UserId = id.parse().map_err(|_| AppError::NotFound)?;
    if target == me {
        return Err(AppError::BadRequest(
            "You can't appear offline to yourself.".into(),
        ));
    }
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM users WHERE id = ?")
        .bind(target.to_string())
        .fetch_optional(&s.db)
        .await?;
    if exists.is_none() {
        return Err(AppError::NotFound);
    }
    if !s.limits.status_user.hit(&me.to_string()) {
        return Err(too_fast());
    }
    Ok(target)
}

/// Appear offline to someone. Their last seen of you freezes at what they see right now if you
/// already look offline to them (Invisible), else now.
async fn hide(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let target = target(&s, me, &id).await?;
    let now = now();
    let subject = load_subject(&s.db, me).await?.ok_or(AppError::NotFound)?;
    let seen = view_for(&subject, target, s.hub.is_online(me), &now);
    let hidden_at = if seen.online() {
        now.clone()
    } else {
        seen.last_seen_at.unwrap_or_else(|| NEVER_SEEN.into())
    };
    // An off-list row (unhidden while Invisible) already holds what they see: keep it.
    sqlx::query(
        "INSERT INTO presence_hidden (owner_id, target_id, hidden_at, active) VALUES (?, ?, ?, 1) \
         ON CONFLICT (owner_id, target_id) DO UPDATE SET active = 1",
    )
    .bind(me.to_string())
    .bind(target.to_string())
    .bind(&hidden_at)
    .execute(&s.db)
    .await?;
    changed(&s, me, target, true).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Stop appearing offline to someone. While you're Invisible they keep the same frozen last seen
/// (the row stays, off your list) until you're visible again.
async fn unhide(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let target = target(&s, me, &id).await?;
    let mut tx = begin_write(&s.db).await?;
    sqlx::query(
        "DELETE FROM presence_hidden WHERE owner_id = ?1 AND target_id = ?2 \
         AND (SELECT presence FROM users WHERE id = ?1) <> 'invisible'",
    )
    .bind(me.to_string())
    .bind(target.to_string())
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE presence_hidden SET active = 0 WHERE owner_id = ? AND target_id = ?")
        .bind(me.to_string())
        .bind(target.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    changed(&s, me, target, false).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn changed(s: &AppState, me: UserId, target: UserId, hidden: bool) {
    s.hub
        .publish(
            &s.db,
            Event::AppearOfflineChanged {
                user_id: me,
                target_id: target,
                hidden,
            },
        )
        .await;
    s.hub.sync_presence(&s.db, me).await;
}
