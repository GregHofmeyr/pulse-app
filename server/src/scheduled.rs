//! Scheduled messages (#299): a message written now that the server posts at its time, as its author. Only its
//! author sees it (their Ready, and `ScheduledSaved` / `ScheduledRemoved` to their own sessions) until it is posted.
//! Text only: mentions and emoji are text and resolve when it is posted; files and GIFs aren't scheduled (an upload
//! is claimed by the send that carries it, and an unclaimed one is let go long before most scheduled times).
//!
//! Posting: the sweep (every [`SWEEP_EVERY`], and at boot) finds due rows from `send_at` in the database, so a
//! restart neither loses one nor posts one twice. Each goes through the send path ([`send_as`]) with every check a
//! send has, run again now: the chat is still there, its author may still post in it, a reply's target is still in
//! it. The send carries a nonce made from the scheduled id, so a post that landed before its row was deleted (a
//! crash between the two, or the sweep and Send now at once) is handed back, never posted again. One that can't be
//! posted is kept as failed, with why, and its author's devices hear it (`ScheduledSaved` with `failed`); it waits in
//! their list until they delete it or send it again. A passing problem (the database, the disk) leaves it for the
//! next sweep.
//!
//! Edits only touch one still to come (`send_at` after now, checked in the `UPDATE` itself) or a failed one, so the
//! sweep never posts a text being changed under it.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use jiff::{SignedDuration, Timestamp};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, ScheduledId, UserId};
use pulse_protocol::rest::{
    ChannelKind, EditScheduledRequest, Message, ScheduleMessageRequest, ScheduledMessage,
    SendMessageRequest, schedule_limits,
};
use sqlx::SqlitePool;

use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::{begin_write, now, stamp};
use crate::error::{AppError, AppResult};
use crate::messages::routes::{clean, send_as};
use crate::servers::routes::is_server_member;

/// How often due scheduled messages are posted.
pub const SWEEP_EVERY: Duration = Duration::from_secs(15);

/// How many due ones one sweep posts (the rest go on the next).
const SWEEP_BATCH: i64 = 100;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/scheduled", get(list))
        .route("/channels/{id}/scheduled", post(create))
        .route("/scheduled/{id}", patch(edit).delete(remove))
        .route("/scheduled/{id}/send", post(send_now))
}

type Row = (
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    Option<String>,
);
const COLS: &str = "id, channel_id, content, reply_to_id, send_at, created_at, edited_at, failed FROM scheduled_messages";

fn from_row(r: Row) -> AppResult<ScheduledMessage> {
    let (id, channel_id, content, reply_to_id, send_at, created_at, edited_at, failed) = r;
    Ok(ScheduledMessage {
        id: id.parse().map_err(anyhow::Error::from)?,
        channel_id: channel_id.parse().map_err(anyhow::Error::from)?,
        content,
        reply_to_id: reply_to_id
            .map(|r| r.parse())
            .transpose()
            .map_err(anyhow::Error::from)?,
        send_at,
        created_at,
        edited_at,
        failed,
    })
}

/// A time as asked for (RFC 3339, any offset) and checked at `now`: not over (a minute's grace for a slow form: it
/// then goes on the next sweep) and at most [`schedule_limits::MAX_DAYS_AHEAD`] away. Stored as UTC. Reminders (#300)
/// take times the same way.
pub fn future_time(raw: &str, now: Timestamp) -> AppResult<String> {
    let t = crate::times::parse(raw.trim())
        .map_err(|_| AppError::BadRequest("that isn't a date and time".into()))?;
    if t < now - SignedDuration::from_mins(1) {
        return Err(AppError::BadRequest("that time has already passed".into()));
    }
    if t > now + crate::times::days(schedule_limits::MAX_DAYS_AHEAD) {
        return Err(AppError::BadRequest("that's more than a year away".into()));
    }
    Ok(stamp(t))
}

/// `me`'s scheduled messages, soonest first (failed ones too): for Ready and `GET /scheduled`.
pub async fn list_for(db: &SqlitePool, me: UserId) -> AppResult<Vec<ScheduledMessage>> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} WHERE author_id = ? ORDER BY send_at, id"
    )))
    .bind(me.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter().map(from_row).collect()
}

/// `me`'s scheduled message `id` (404 for anyone else's, as if there were none).
async fn mine(db: &SqlitePool, me: UserId, id: ScheduledId) -> AppResult<ScheduledMessage> {
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} WHERE id = ? AND author_id = ?"
    )))
    .bind(id.to_string())
    .bind(me.to_string())
    .fetch_optional(db)
    .await?;
    row.map(from_row).transpose()?.ok_or(AppError::NotFound)
}

async fn list(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
) -> AppResult<Json<Vec<ScheduledMessage>>> {
    Ok(Json(list_for(&s.db, me).await?))
}

/// The chat `me` may schedule a message in: one they could send in now (a DM or group they're in, a server chat
/// of a server they're in), not a quotes or clips channel, not a DM with someone they've blocked (#474; #412).
/// Checked again when it's posted: blocking them before then fails it, and one to someone who has blocked `me` is
/// posted withheld from them, as any send is (never a refusal that would tell `me`).
async fn postable(s: &AppState, me: UserId, id: ChannelId) -> AppResult<()> {
    let ch = channel_for(&s.db, me, id).await?;
    if matches!(ch.kind, ChannelKind::Quotes | ChannelKind::Clips)
        || !(ch.kind.has_chat() || ch.kind.is_private())
    {
        return Err(AppError::BadRequest(
            "messages can't be scheduled in that channel".into(),
        ));
    }
    if let Some(server) = ch.server_id
        && !is_server_member(&s.db, server, me).await?
    {
        return Err(AppError::Forbidden);
    }
    let mut conn = s.db.acquire().await?;
    if let Some(other) = crate::messages::routes::dm_other(&mut conn, &ch, me).await?
        && crate::blocks::has_blocked(&mut *conn, me, other).await?
    {
        return Err(AppError::ForbiddenMsg(
            "You've blocked them. Unblock them to schedule a message.".into(),
        ));
    }
    Ok(())
}

/// A reply's target: a live message in the chat's main list (as the composer replies).
async fn check_reply(s: &AppState, channel: ChannelId, to: Option<MessageId>) -> AppResult<()> {
    let Some(to) = to else {
        return Ok(());
    };
    let ok: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM messages WHERE id = ? AND channel_id = ? AND deleted_at IS NULL",
    )
    .bind(to.to_string())
    .bind(channel.to_string())
    .fetch_optional(&s.db)
    .await?;
    if ok.is_none() {
        return Err(AppError::BadRequest(
            "reply target not in this channel".into(),
        ));
    }
    Ok(())
}

async fn publish_saved(s: &AppState, me: UserId, scheduled: ScheduledMessage) {
    s.hub
        .publish(
            &s.db,
            Event::ScheduledSaved {
                user_id: me,
                scheduled,
            },
        )
        .await;
}

async fn publish_removed(s: &AppState, me: UserId, id: ScheduledId, sent: Option<MessageId>) {
    s.hub
        .publish(
            &s.db,
            Event::ScheduledRemoved {
                user_id: me,
                id,
                sent,
            },
        )
        .await;
}

/// `POST /channels/{id}/scheduled`: `201` with the scheduled message. At most [`schedule_limits::MAX_SCHEDULED`]
/// each (checked in the write).
async fn create(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(channel): Path<ChannelId>,
    Json(req): Json<ScheduleMessageRequest>,
) -> AppResult<(StatusCode, Json<ScheduledMessage>)> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let content = clean(&req.content)?;
    let send_at = future_time(&req.send_at, Timestamp::now())?;
    postable(&s, me, channel).await?;
    check_reply(&s, channel, req.reply_to_id).await?;
    let made = ScheduledMessage {
        id: ScheduledId::new(),
        channel_id: channel,
        content,
        reply_to_id: req.reply_to_id,
        send_at,
        created_at: now(),
        edited_at: None,
        failed: None,
    };
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    let have: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM scheduled_messages WHERE author_id = ?")
            .bind(me.to_string())
            .fetch_one(&mut *tx)
            .await?;
    if have >= schedule_limits::MAX_SCHEDULED {
        return Err(AppError::BadRequest(format!(
            "you can have up to {} scheduled messages",
            schedule_limits::MAX_SCHEDULED
        )));
    }
    crate::access::channel_live_in(&mut tx, channel).await?;
    sqlx::query(
        "INSERT INTO scheduled_messages (id, author_id, channel_id, content, reply_to_id, send_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(made.id.to_string())
    .bind(me.to_string())
    .bind(channel.to_string())
    .bind(&made.content)
    .bind(made.reply_to_id.map(|r| r.to_string()))
    .bind(&made.send_at)
    .bind(&made.created_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    publish_saved(&s, me, made.clone()).await;
    Ok((StatusCode::CREATED, Json(made)))
}

/// `PATCH /scheduled/{id}`: a new text and/or time. Only one still to come, or a failed one (a new time puts a
/// failed one back in the queue); one whose time has come is being posted (`409`).
async fn edit(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ScheduledId>,
    Json(req): Json<EditScheduledRequest>,
) -> AppResult<Json<ScheduledMessage>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let at = Timestamp::now();
    let content = req.content.as_deref().map(clean).transpose()?;
    let send_at = req
        .send_at
        .as_deref()
        .map(|t| future_time(t, at))
        .transpose()?;
    if content.is_none() && send_at.is_none() {
        return Err(AppError::BadRequest("nothing to change".into()));
    }
    let was = mine(&s.db, me, id).await?;
    if send_at.is_some() {
        // a new time on a failed one sends it again: the chat must be one they can post in now
        postable(&s, me, was.channel_id).await?;
    }
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    let changed = sqlx::query(
        "UPDATE scheduled_messages SET content = COALESCE(?, content), send_at = COALESCE(?, send_at),
           edited_at = ?,
           failed_at = CASE WHEN ? IS NULL THEN failed_at END,
           failed = CASE WHEN ? IS NULL THEN failed END
         WHERE id = ? AND author_id = ? AND (failed_at IS NOT NULL OR send_at > ?)",
    )
    .bind(&content)
    .bind(&send_at)
    .bind(now())
    .bind(&send_at)
    .bind(&send_at)
    .bind(id.to_string())
    .bind(me.to_string())
    .bind(stamp(at))
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::Conflict("it's being sent right now".into()));
    }
    tx.commit().await?;
    let now_is = mine(&s.db, me, id).await?;
    publish_saved(&s, me, now_is.clone()).await;
    Ok(Json(now_is))
}

/// `DELETE /scheduled/{id}`: `204`. One being posted right now may still land.
async fn remove(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ScheduledId>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let _order = s.hub.in_order().await;
    let gone = sqlx::query("DELETE FROM scheduled_messages WHERE id = ? AND author_id = ?")
        .bind(id.to_string())
        .bind(me.to_string())
        .execute(&s.db)
        .await?
        .rows_affected();
    if gone == 0 {
        return Err(AppError::NotFound);
    }
    publish_removed(&s, me, id, None).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /scheduled/{id}/send`: post it now (a failed one too, once its reason is gone). The posted message; one
/// that can't be posted is kept as failed, and the answer says why.
async fn send_now(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ScheduledId>,
) -> AppResult<Json<Message>> {
    if !s.limits.send_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let row = mine(&s.db, me, id).await?;
    match post_one(&s, me, &row).await? {
        Posted::Sent(m) => Ok(Json(*m)),
        Posted::Failed(why) => Err(AppError::BadRequest(why)),
    }
}

enum Posted {
    Sent(Box<Message>),
    Failed(String),
}

/// Why a send's refusal means it can't be posted at all (rather than not right now), in words for its author.
fn lasting(e: &AppError) -> Option<String> {
    match e {
        AppError::NotFound => Some("that chat is gone, or you're no longer in it".into()),
        AppError::Forbidden => Some("you can't post in that chat any more".into()),
        AppError::ForbiddenMsg(m) | AppError::BadRequest(m) | AppError::Conflict(m) => {
            Some(m.clone())
        }
        _ => None,
    }
}

/// Post `row` as `me` now, through the send path. Sent: the row goes (its author's devices hear where it went).
/// Refused for good: kept as failed, with why. A passing failure is the error (it stays to be tried again).
async fn post_one(s: &AppState, me: UserId, row: &ScheduledMessage) -> AppResult<Posted> {
    let req = SendMessageRequest {
        content: row.content.clone(),
        reply_to_id: row.reply_to_id,
        // one send per scheduled message, whoever posts it and however often
        nonce: Some(format!("scheduled:{}", row.id)),
        thread_root_id: None,
        gif: None,
        attachments: vec![],
    };
    match send_as(s, me, row.channel_id, req).await {
        Ok(m) => {
            sqlx::query("DELETE FROM scheduled_messages WHERE id = ?")
                .bind(row.id.to_string())
                .execute(&s.db)
                .await?;
            let _order = s.hub.in_order().await;
            publish_removed(s, me, row.id, Some(m.id)).await;
            Ok(Posted::Sent(Box::new(m)))
        }
        Err(e) => {
            let Some(why) = lasting(&e) else {
                return Err(e);
            };
            let _order = s.hub.in_order().await;
            let marked = sqlx::query(
                "UPDATE scheduled_messages SET failed_at = ?, failed = ? WHERE id = ? AND failed_at IS NULL",
            )
            .bind(now())
            .bind(&why)
            .bind(row.id.to_string())
            .execute(&s.db)
            .await?
            .rows_affected();
            if marked > 0
                && let Ok(failed) = mine(&s.db, me, row.id).await
            {
                publish_saved(s, me, failed).await;
            }
            Ok(Posted::Failed(why))
        }
    }
}

/// Post every scheduled message due at `now` (up to [`SWEEP_BATCH`]), oldest time first. Returns how many were
/// posted; failed ones are marked as such and counted out.
pub async fn send_due(s: &AppState, now: Timestamp) -> AppResult<usize> {
    let due: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, author_id FROM scheduled_messages WHERE failed_at IS NULL AND send_at <= ?
         ORDER BY send_at, id LIMIT ?",
    )
    .bind(stamp(now))
    .bind(SWEEP_BATCH)
    .fetch_all(&s.db)
    .await?;
    let mut sent = 0;
    for (raw_id, raw_author) in due {
        // one bad row is skipped (and logged), never the end of the sweep for every row behind it
        let (Ok(id), Ok(author)) = (raw_id.parse::<ScheduledId>(), raw_author.parse::<UserId>())
        else {
            tracing::error!(id = %raw_id, "a scheduled message's row doesn't parse; skipped");
            continue;
        };
        // deleted (or sent now) since the look: nothing to do
        let Ok(row) = mine(&s.db, author, id).await else {
            continue;
        };
        if row.failed.is_some() {
            continue;
        }
        match post_one(s, author, &row).await {
            Ok(Posted::Sent(_)) => sent += 1,
            Ok(Posted::Failed(why)) => {
                tracing::debug!(id = %row.id, %why, "a scheduled message couldn't be posted");
            }
            Err(e) => {
                tracing::warn!(error = %e, id = %row.id, "a scheduled message wasn't posted; trying again later");
            }
        }
    }
    Ok(sent)
}

/// Post due scheduled messages every [`SWEEP_EVERY`] (the first tick runs at once: the boot sweep). Call after
/// binding the listener.
pub fn spawn_sweeper(s: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            match send_due(&s, Timestamp::now()).await {
                Ok(0) => {}
                Ok(n) => tracing::debug!(n, "scheduled messages posted"),
                Err(e) => tracing::warn!(error = %e, "scheduled message sweep failed"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> Timestamp {
        crate::times::parse("2026-11-16T10:00:00Z").unwrap()
    }

    #[test]
    fn a_time_is_kept_as_utc_and_must_be_ahead_within_a_year() {
        assert_eq!(
            future_time(" 2026-11-16T20:00:00+02:00 ", at()).unwrap(),
            "2026-11-16T18:00:00.000Z"
        );
        // a minute's grace, then it's over
        assert!(future_time("2026-11-16T09:59:30Z", at()).is_ok());
        assert!(future_time("2026-11-16T09:58:00Z", at()).is_err());
        assert!(future_time("2027-11-16T10:00:00Z", at()).is_ok());
        assert!(future_time("2027-11-17T10:00:00Z", at()).is_err());
        assert!(future_time("tomorrow", at()).is_err());
    }

    #[test]
    fn only_lasting_refusals_fail_a_scheduled_message() {
        assert!(lasting(&AppError::NotFound).is_some());
        assert!(lasting(&AppError::Forbidden).is_some());
        assert_eq!(
            lasting(&AppError::BadRequest("reply target gone".into())).as_deref(),
            Some("reply target gone")
        );
        assert!(lasting(&AppError::TooManyRequests(None)).is_none());
        assert!(lasting(&AppError::Internal(anyhow::anyhow!("disk"))).is_none());
    }
}
