//! `PUT`/`DELETE /messages/{id}/pin` and `GET /channels/{id}/pins` (#178).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{
    Channel, Message, MessageKind, Pin, PinDuration, PinLine, PinRequest, PinnedMessage,
};

use super::{MAX_PINS, PinRow, pin_from_row, pinnable};
use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::{now, stamp};
use crate::error::{AppError, AppResult};
use crate::messages::routes::{COLS, Row, fill, load};
use crate::messages::system;
use crate::servers::routes::is_server_member;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/messages/{id}/pin", put(pin).delete(unpin))
        .route("/channels/{id}/pins", get(list))
}

/// The chat `ch`, if it takes pins and `me` may write in it: the same rule as sending (a server channel needs
/// you in its server; a DM or group was already checked by `channel_for`).
async fn writable(s: &AppState, me: UserId, ch: &Channel) -> AppResult<()> {
    if !pinnable(ch.kind) {
        return Err(AppError::BadRequest(
            "pins aren't kept in quotes or clips channels".into(),
        ));
    }
    if let Some(server) = ch.server_id
        && !is_server_member(&s.db, server, me).await?
    {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

/// The message and its chat, if `me` may pin or unpin there. Missing, unseen and deleted look alike (404).
async fn target(s: &AppState, me: UserId, id: MessageId) -> AppResult<(Message, Channel)> {
    let msg = load(&s.db, Some(me), id).await?.ok_or(AppError::NotFound)?;
    let ch = channel_for(&s.db, me, msg.channel_id).await?;
    writable(s, me, &ch).await?;
    if msg.deleted {
        return Err(AppError::NotFound);
    }
    Ok((msg, ch))
}

async fn pin(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
    Json(req): Json<PinRequest>,
) -> AppResult<Json<Pin>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    // A pin posts a line, as a send does: refused the same way while the server's disk is full.
    crate::media::writes_ok(&s.cfg)?;
    let (mut msg, ch) = target(&s, me, id).await?;
    if msg.kind == MessageKind::System {
        return Err(AppError::BadRequest("system lines can't be pinned".into()));
    }
    if msg.thread_root_id.is_some() {
        return Err(AppError::BadRequest("replies can't be pinned".into()));
    }
    let at = chrono::Utc::now();
    let pin = Pin {
        pinned_by: me,
        pinned_at: stamp(at),
        expires_at: match req.duration {
            PinDuration::Hour => Some(stamp(at + chrono::Duration::hours(1))),
            PinDuration::Day => Some(stamp(at + chrono::Duration::days(1))),
            PinDuration::Week => Some(stamp(at + chrono::Duration::weeks(1))),
            PinDuration::Forever => None,
        },
    };
    let name: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id = ?")
        .bind(me.to_string())
        .fetch_one(&s.db)
        .await?;
    let content = match req.duration.phrase() {
        Some(p) => format!("{name} pinned a message {p}"),
        None => format!("{name} pinned a message"),
    };
    let payload = system::pin_line_payload(&PinLine {
        message_id: id,
        by: me,
        duration: req.duration,
    })?;
    // The pin's event and its line go out in commit order with other pins, unpins and the sweep's expiries.
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    // Still live: a delete may have committed since the check above.
    let live: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM messages WHERE id = ? AND deleted_at IS NULL")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    if live.is_none() {
        return Err(AppError::NotFound);
    }
    // A pin that has run out but the sweep hasn't reached is no pin: it makes way (with no event: clients
    // already treat it as gone or will on the sweep's).
    sqlx::query(
        "DELETE FROM pins WHERE message_id = ? AND expires_at IS NOT NULL AND expires_at <= ?",
    )
    .bind(id.to_string())
    .bind(&pin.pinned_at)
    .execute(&mut *tx)
    .await?;
    let already: Option<i64> = sqlx::query_scalar("SELECT 1 FROM pins WHERE message_id = ?")
        .bind(id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
    if already.is_some() {
        return Err(AppError::Conflict("That message is already pinned.".into()));
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pins WHERE channel_id = ? AND (expires_at IS NULL OR expires_at > ?)",
    )
    .bind(ch.id.to_string())
    .bind(&pin.pinned_at)
    .fetch_one(&mut *tx)
    .await?;
    if count >= MAX_PINS {
        return Err(AppError::Conflict(format!(
            "This chat already has {MAX_PINS} pins. Unpin something first."
        )));
    }
    sqlx::query(
        "INSERT INTO pins (message_id, channel_id, pinned_by, pinned_at, expires_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(ch.id.to_string())
    .bind(me.to_string())
    .bind(&pin.pinned_at)
    .bind(&pin.expires_at)
    .execute(&mut *tx)
    .await?;
    let line = system::insert(&mut tx, ch.id, content, payload).await?;
    tx.commit().await?;
    // The message as it is now, as events show it (an edit may have landed since the check above; still under
    // `in_order`, so nothing newer can go out before this). The pin has landed: a failed read falls back to the
    // copy from the check, logged.
    match load(&s.db, None, id).await {
        Ok(Some(now)) => msg = now,
        Ok(None) => {}
        Err(e) => {
            tracing::error!(error = ?e, %id, "couldn't read a pinned message back");
            for r in &mut msg.reactions {
                r.me = false;
            }
        }
    }
    msg.pin = Some(pin.clone());
    s.hub
        .publish(
            &s.db,
            Event::MessagePinned {
                channel_id: ch.id,
                pinned: PinnedMessage {
                    message: msg,
                    pin: pin.clone(),
                },
            },
        )
        .await;
    s.hub
        .publish(
            &s.db,
            Event::MessageCreated {
                message: line,
                nonce: None,
            },
        )
        .await;
    Ok(Json(pin))
}

async fn unpin(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let (_, ch) = target(&s, me, id).await?;
    let _order = s.hub.in_order().await;
    // A pin that has run out is already gone, as far as anyone can see.
    let gone = sqlx::query(
        "DELETE FROM pins WHERE message_id = ? AND (expires_at IS NULL OR expires_at > ?)",
    )
    .bind(id.to_string())
    .bind(now())
    .execute(&s.db)
    .await?
    .rows_affected();
    if gone == 0 {
        return Err(AppError::NotFound);
    }
    s.hub
        .publish(
            &s.db,
            Event::MessageUnpinned {
                channel_id: ch.id,
                message_id: id,
            },
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn list(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
) -> AppResult<Json<Vec<PinnedMessage>>> {
    let ch = channel_for(&s.db, me, id).await?;
    if !pinnable(ch.kind) {
        return Err(AppError::BadRequest(
            "pins aren't kept in quotes or clips channels".into(),
        ));
    }
    let pins: Vec<PinRow> = sqlx::query_as(
        "SELECT p.message_id, p.pinned_by, p.pinned_at, p.expires_at FROM pins p
         JOIN messages m ON m.id = p.message_id
         WHERE p.channel_id = ? AND m.deleted_at IS NULL AND (p.expires_at IS NULL OR p.expires_at > ?)
         ORDER BY p.pinned_at DESC, p.message_id DESC LIMIT ?",
    )
    .bind(id.to_string())
    .bind(now())
    .bind(MAX_PINS)
    .fetch_all(&s.db)
    .await?;
    if pins.is_empty() {
        return Ok(Json(vec![]));
    }
    let mut q = sqlx::QueryBuilder::new(format!("SELECT {COLS} FROM messages WHERE id IN ("));
    let mut ids = q.separated(", ");
    for p in &pins {
        ids.push_bind(p.0.clone());
    }
    q.push(")");
    let rows: Vec<Row> = q.build_query_as().fetch_all(&s.db).await?;
    let mut messages: std::collections::HashMap<MessageId, Message> = fill(&s.db, Some(me), rows)
        .await?
        .into_iter()
        .map(|m| (m.id, m))
        .collect();
    let mut out = Vec::with_capacity(pins.len());
    for row in pins {
        let mid: MessageId = row.0.parse().map_err(anyhow::Error::from)?;
        let pin = pin_from_row(row)?;
        // Read apart from its pin: one that ran out between the two reads stays out.
        if let Some(mut message) = messages.remove(&mid) {
            message.pin = Some(pin.clone());
            out.push(PinnedMessage { message, pin });
        }
    }
    Ok(Json(out))
}
