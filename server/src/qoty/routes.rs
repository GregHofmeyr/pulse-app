//! The HTTP side of Quote of the Year. Taking part needs server membership; every route answers
//! with the event as everyone now sees it.

use axum::extract::{Path, State};
use axum::routing::{post, put};
use axum::{Json, Router};
use chrono::Utc;
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{QotyEventId, ServerId, UserId};
use pulse_protocol::rest::{
    AddQotyOptionRequest, QotyEndReason, QotyEvent, QotyVoteRequest, ReopenQotyRequest,
    StartQotyRequest,
};

use super::{
    FIRST_YEAR, REOPEN_HOURS, check_minutes, ending, ends_at, live_in, load, publish, require_live,
    server_of, top_options,
};
use crate::AppState;
use crate::auth::AuthUser;
use crate::db::{now, stamp};
use crate::error::{AppError, AppResult};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/servers/{id}/qoty", post(start))
        .route("/qoty/{id}/vote", put(vote).delete(unvote))
        .route("/qoty/{id}/options", post(add_option))
        .route("/qoty/{id}/end", post(end))
        .route("/qoty/{id}/reopen", post(reopen))
}

fn limit(s: &AppState, me: UserId) -> AppResult<()> {
    if s.limits.mutate_user.hit(&me.to_string()) {
        Ok(())
    } else {
        Err(AppError::TooManyRequests)
    }
}

/// `me` must be a member of `server` (404 for no such server, 403 for not a member).
async fn members_only(s: &AppState, server: ServerId, me: UserId) -> AppResult<()> {
    crate::servers::routes::require_member(&s.db, server, me).await
}

/// The rate limit, then the event's server (404 if there's no such event) and membership (403).
async fn take_part(s: &AppState, me: UserId, id: QotyEventId) -> AppResult<ServerId> {
    limit(s, me)?;
    let server = server_of(&s.db, id).await?.ok_or(AppError::NotFound)?;
    members_only(s, server, me).await?;
    Ok(server)
}

/// What a route answers once its change is published: the event as published, or as it stands
/// when nothing changed (or reading it back for the publish failed).
async fn answer(
    s: &AppState,
    id: QotyEventId,
    published: Option<QotyEvent>,
) -> AppResult<Json<QotyEvent>> {
    match published {
        Some(e) => Ok(Json(e)),
        None => load(&s.db, id, None)
            .await?
            .map(Json)
            .ok_or(AppError::NotFound),
    }
}

async fn start(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(server): Path<ServerId>,
    Json(req): Json<StartQotyRequest>,
) -> AppResult<Json<QotyEvent>> {
    limit(&s, me)?;
    crate::media::writes_ok(&s.cfg)?;
    let this_year = chrono::Datelike::year(&Utc::now());
    if !(FIRST_YEAR..=this_year).contains(&req.year) {
        return Err(AppError::BadRequest(format!(
            "pick a year from {FIRST_YEAR} to {this_year}"
        )));
    }
    check_minutes(req.minutes)?;
    members_only(&s, server, me).await?;

    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let overdue = ending::end_if_overdue(&mut tx, server).await?;
    if live_in(&mut tx, server).await?.is_some() {
        return Err(AppError::Conflict(
            "There's already a Quote of the Year vote on in this server.".into(),
        ));
    }
    let held: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM qoty_events WHERE server_id = ? AND year = ?")
            .bind(server.to_string())
            .bind(req.year)
            .fetch_optional(&mut *tx)
            .await?;
    if held.is_some() {
        return Err(AppError::Conflict(format!(
            "Quote of the Year {} has already happened.",
            req.year
        )));
    }
    let id = QotyEventId::new();
    let started = Utc::now();
    sqlx::query(
        "INSERT INTO qoty_events (id, server_id, year, started_by, started_at, ends_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(server.to_string())
    .bind(req.year)
    .bind(me.to_string())
    .bind(stamp(started))
    .bind(ends_at(started, req.minutes))
    .execute(&mut *tx)
    .await?;
    for message in top_options(&mut tx, server, req.year).await? {
        sqlx::query("INSERT INTO qoty_options (event_id, message_id) VALUES (?, ?)")
            .bind(id.to_string())
            .bind(message)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    if let Some(ended) = overdue {
        ending::announce(&s, ended).await;
    }
    let published = publish(&s, id, |event| Event::QotyStarted { event }).await;
    answer(&s, id, published).await
}

/// Vote for an option, or move your vote to it (you go to the end of its voters). Voting for
/// what you already voted for changes nothing.
async fn vote(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<QotyEventId>,
    Json(req): Json<QotyVoteRequest>,
) -> AppResult<Json<QotyEvent>> {
    take_part(&s, me, id).await?;
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    require_live(&mut tx, id).await?;
    let option: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM qoty_options WHERE event_id = ? AND message_id = ?")
            .bind(id.to_string())
            .bind(req.message_id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    if option.is_none() {
        return Err(AppError::BadRequest(
            "that quote isn't one of the options".into(),
        ));
    }
    let current: Option<String> =
        sqlx::query_scalar("SELECT message_id FROM qoty_votes WHERE event_id = ? AND user_id = ?")
            .bind(id.to_string())
            .bind(me.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    if current.as_deref() == Some(req.message_id.to_string().as_str()) {
        tx.rollback().await?;
        return answer(&s, id, None).await;
    }
    // Delete and insert rather than update: the new row sorts last, which is the vote order.
    sqlx::query("DELETE FROM qoty_votes WHERE event_id = ? AND user_id = ?")
        .bind(id.to_string())
        .bind(me.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO qoty_votes (event_id, user_id, message_id) VALUES (?, ?, ?)")
        .bind(id.to_string())
        .bind(me.to_string())
        .bind(req.message_id.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let published = publish(&s, id, |event| Event::QotyUpdated { event }).await;
    answer(&s, id, published).await
}

/// Take back your vote. Having none changes nothing.
async fn unvote(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<QotyEventId>,
) -> AppResult<Json<QotyEvent>> {
    take_part(&s, me, id).await?;
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    require_live(&mut tx, id).await?;
    let gone = sqlx::query("DELETE FROM qoty_votes WHERE event_id = ? AND user_id = ?")
        .bind(id.to_string())
        .bind(me.to_string())
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    let published = if gone > 0 {
        publish(&s, id, |event| Event::QotyUpdated { event }).await
    } else {
        None
    };
    answer(&s, id, published).await
}

/// Add a live quote of this server as an option, from any year, unless it already won a vote.
async fn add_option(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<QotyEventId>,
    Json(req): Json<AddQotyOptionRequest>,
) -> AppResult<Json<QotyEvent>> {
    let server = take_part(&s, me, id).await?;
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    require_live(&mut tx, id).await?;
    let message = req.message_id.to_string();
    let quote_server: Option<String> = sqlx::query_scalar(
        "SELECT c.server_id FROM quotes q
         JOIN messages msg ON msg.id = q.message_id AND msg.deleted_at IS NULL
         JOIN channels c ON c.id = msg.channel_id
         WHERE q.message_id = ?",
    )
    .bind(&message)
    .fetch_optional(&mut *tx)
    .await?;
    if quote_server.as_deref() != Some(server.to_string().as_str()) {
        return Err(AppError::BadRequest(
            "that isn't a quote in this server".into(),
        ));
    }
    let already: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM qoty_options WHERE event_id = ? AND message_id = ?")
            .bind(id.to_string())
            .bind(&message)
            .fetch_optional(&mut *tx)
            .await?;
    if already.is_some() {
        return Err(AppError::Conflict(
            "That quote is already an option.".into(),
        ));
    }
    let won: Option<i64> = sqlx::query_scalar("SELECT 1 FROM qoty_winners WHERE message_id = ?")
        .bind(&message)
        .fetch_optional(&mut *tx)
        .await?;
    if won.is_some() {
        return Err(AppError::Conflict(
            "That quote has already won Quote of the Year.".into(),
        ));
    }
    sqlx::query("INSERT INTO qoty_options (event_id, message_id, added_by) VALUES (?, ?, ?)")
        .bind(id.to_string())
        .bind(&message)
        .bind(me.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let published = publish(&s, id, |event| Event::QotyUpdated { event }).await;
    answer(&s, id, published).await
}

/// End the vote now. Anyone in the server can. Once its time has run out it is an end by time,
/// whoever gets there first (the sweeper or this).
async fn end(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<QotyEventId>,
) -> AppResult<Json<QotyEvent>> {
    take_part(&s, me, id).await?;
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let ends_at: Option<Option<String>> =
        sqlx::query_scalar("SELECT ends_at FROM qoty_events WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    let (reason, by) = if super::past(ends_at.flatten().as_deref()) {
        (QotyEndReason::Time, None)
    } else {
        (QotyEndReason::Early, Some(me))
    };
    let Some(ended) = ending::finish(&mut tx, id, reason, by, &now()).await? else {
        return Err(AppError::Conflict("This vote is over.".into()));
    };
    tx.commit().await?;
    let published = ending::announce(&s, ended).await;
    answer(&s, id, published).await
}

/// Reopen a vote that was ended early, within a day of the end, with a new length. Its votes and
/// options stay; its winners and cards go.
async fn reopen(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<QotyEventId>,
    Json(req): Json<ReopenQotyRequest>,
) -> AppResult<Json<QotyEvent>> {
    limit(&s, me)?;
    check_minutes(req.minutes)?;
    let server = server_of(&s.db, id).await?.ok_or(AppError::NotFound)?;
    members_only(&s, server, me).await?;

    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let row: Option<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT ended_at, end_reason FROM qoty_events WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    let (ended_at, reason) = row.ok_or(AppError::NotFound)?;
    let Some(ended_at) = ended_at else {
        return Err(AppError::Conflict("This vote is still on.".into()));
    };
    if reason.as_deref() != Some("early") {
        return Err(AppError::Conflict(
            "Only a vote that was ended early can be reopened.".into(),
        ));
    }
    let reopened = Utc::now();
    if stamp(reopened - chrono::Duration::hours(REOPEN_HOURS)) >= ended_at {
        return Err(AppError::Conflict(
            "It's too late to reopen this vote: it ended more than a day ago.".into(),
        ));
    }
    // Before undoing ours: a quote that wins the overdue vote can't then win this one too.
    let overdue = ending::end_if_overdue(&mut tx, server).await?;
    if live_in(&mut tx, server).await?.is_some() {
        return Err(AppError::Conflict(
            "There's already a Quote of the Year vote on in this server.".into(),
        ));
    }
    let cards = ending::undo(&mut tx, id, server).await?;
    sqlx::query(
        "UPDATE qoty_events SET ends_at = ?, ended_at = NULL, ended_by = NULL, end_reason = NULL
         WHERE id = ? AND ended_at IS NOT NULL",
    )
    .bind(ends_at(reopened, req.minutes))
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    if let Some(ended) = overdue {
        ending::announce(&s, ended).await;
    }
    for (channel_id, message_id) in cards {
        s.hub
            .publish(
                &s.db,
                Event::MessageDeleted {
                    channel_id,
                    message_id,
                    author_id: None,
                    mentions: vec![],
                    thread_root_id: None,
                },
            )
            .await;
    }
    // Live again: clients treat it like a start.
    let published = publish(&s, id, |event| Event::QotyStarted { event }).await;
    answer(&s, id, published).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths() {
        assert!(check_minutes(None).is_ok());
        assert!(check_minutes(Some(5)).is_ok());
        assert!(check_minutes(Some(20160)).is_ok());
        assert!(check_minutes(Some(4)).is_err());
        assert!(check_minutes(Some(20161)).is_err());
    }
}
