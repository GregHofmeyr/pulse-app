//! Message reminders (#300): "Remind me about this" on a message, private to whoever set it. When its time comes it
//! lands in their inbox (counted unread until they open it or mark all read) and their devices hear it
//! (`ReminderSaved` with `due`), which is when the app notifies them.
//!
//! The sweep (every [`SWEEP_EVERY`], and at boot) finds due reminders from `remind_at` in the database; whoever's
//! `UPDATE … WHERE fired_at IS NULL` changes the row tells its person, so a restart neither loses nor repeats one. A
//! reminder whose message they can no longer see when it comes due (deleted, its chat gone or left) goes quietly.
//! Due ones stay listed until marked done (deleted), or for the inbox's 30 days.
//!
//! One that comes due while none of its person's devices is connected is `missed` (#412): their next device to
//! connect claims it (`POST /reminders/{id}/alerted`, which only one claim wins) and tells them then.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::{DateTime, TimeDelta, Utc};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{MessageId, ReminderId, UserId};
use pulse_protocol::rest::{
    CreateReminderRequest, INBOX_SNIPPET_CHARS, InboxSnippet, MessageReminder, schedule_limits,
};
use sqlx::{SqliteExecutor, SqlitePool};

use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::{begin_write, now, stamp};
use crate::error::{AppError, AppResult};

/// How often due reminders are sent.
pub const SWEEP_EVERY: Duration = Duration::from_secs(15);

/// How many due ones one sweep sends (the rest go on the next).
const SWEEP_BATCH: i64 = 200;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/reminders", get(list))
        .route("/messages/{id}/reminders", post(create))
        .route("/reminders/{id}", delete(remove))
        .route("/reminders/{id}/seen", post(seen))
        .route("/reminders/{id}/alerted", post(alerted))
}

/// A person's reminders whose message they can still see: not deleted, in a chat that isn't in Recently deleted (nor
/// its server), and a DM or group they're still in, or a server they're still a member of. Binds: the person. The
/// inbox counts its due, unseen ones unread.
pub(crate) const VISIBLE: &str = "FROM message_reminders r
    JOIN messages m ON m.id = r.message_id
    JOIN channels c ON c.id = m.channel_id
    WHERE r.user_id = ? AND m.deleted_at IS NULL AND c.deleted_at IS NULL
      AND (c.server_id IS NULL OR c.server_id NOT IN (SELECT id FROM servers WHERE deleted_at IS NOT NULL))
      AND (c.server_id IS NOT NULL OR EXISTS (SELECT 1 FROM channel_members cm
                                               WHERE cm.channel_id = c.id AND cm.user_id = r.user_id))
      AND (c.server_id IS NULL OR EXISTS (SELECT 1 FROM server_members sm
                                           WHERE sm.server_id = c.server_id AND sm.user_id = r.user_id))";

const COLS: &str = "SELECT r.id, r.message_id, m.channel_id, m.thread_root_id, r.remind_at, r.created_at, \
     r.fired_at IS NOT NULL, r.seen_at IS NOT NULL, \
     r.fired_at IS NOT NULL AND r.seen_at IS NULL AND r.alerted_at IS NULL, m.author_id, m.content";

type Row = (
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    bool,
    bool,
    bool,
    Option<String>,
    String,
);

fn from_row(r: Row) -> AppResult<MessageReminder> {
    let (
        id,
        message_id,
        channel_id,
        thread_root_id,
        remind_at,
        created_at,
        due,
        seen,
        missed,
        author,
        content,
    ) = r;
    let message_id: MessageId = message_id.parse().map_err(anyhow::Error::from)?;
    Ok(MessageReminder {
        id: id.parse().map_err(anyhow::Error::from)?,
        message_id,
        channel_id: channel_id.parse().map_err(anyhow::Error::from)?,
        thread_root_id: thread_root_id
            .map(|t| t.parse())
            .transpose()
            .map_err(anyhow::Error::from)?,
        remind_at,
        created_at,
        due,
        seen,
        missed,
        message: InboxSnippet {
            message_id,
            author_id: author
                .map(|a| a.parse())
                .transpose()
                .map_err(anyhow::Error::from)?,
            content: content.chars().take(INBOX_SNIPPET_CHARS).collect(),
            deleted: false,
        },
    })
}

/// `me`'s reminders they can still see, soonest first: for Ready and `GET /reminders`.
pub async fn list_for(db: &SqlitePool, me: UserId) -> AppResult<Vec<MessageReminder>> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{COLS} {VISIBLE} ORDER BY r.remind_at, r.id"
    )))
    .bind(me.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter().map(from_row).collect()
}

/// `me`'s reminder `id`, if they can still see its message.
async fn one<'e>(
    db: impl SqliteExecutor<'e>,
    me: UserId,
    id: ReminderId,
) -> AppResult<Option<MessageReminder>> {
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{COLS} {VISIBLE} AND r.id = ?"
    )))
    .bind(me.to_string())
    .bind(id.to_string())
    .fetch_optional(db)
    .await?;
    row.map(from_row).transpose()
}

async fn list(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
) -> AppResult<Json<Vec<MessageReminder>>> {
    Ok(Json(list_for(&s.db, me).await?))
}

async fn publish_saved(s: &AppState, me: UserId, reminder: MessageReminder) {
    s.hub
        .publish(
            &s.db,
            Event::ReminderSaved {
                user_id: me,
                reminder,
            },
        )
        .await;
}

/// Tell `me`'s devices their inbox count (a reminder came due, was seen or went).
async fn recount(s: &AppState, me: UserId) {
    crate::inbox::tell(&s.db, &s.hub, me, true, |n| {
        crate::inbox::updated(me, n, vec![], None, vec![])
    })
    .await;
}

/// `POST /messages/{id}/reminders`: `201` with the reminder. Any message you can see (404 otherwise). At most
/// [`schedule_limits::MAX_REMINDERS`] still to come each (checked in the write).
async fn create(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(message): Path<MessageId>,
    Json(req): Json<CreateReminderRequest>,
) -> AppResult<(StatusCode, Json<MessageReminder>)> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let remind_at = crate::scheduled::future_time(&req.remind_at, Utc::now())?;
    let channel: Option<String> =
        sqlx::query_scalar("SELECT channel_id FROM messages WHERE id = ? AND deleted_at IS NULL")
            .bind(message.to_string())
            .fetch_optional(&s.db)
            .await?;
    let channel = channel
        .ok_or(AppError::NotFound)?
        .parse()
        .map_err(anyhow::Error::from)?;
    let ch = channel_for(&s.db, me, channel).await?;
    // a server's messages: as a member (#371), as bookmarks are and as `VISIBLE` lists them
    if let Some(server) = ch.server_id
        && !crate::servers::routes::is_server_member(&s.db, server, me).await?
    {
        return Err(AppError::NotFound);
    }
    let id = ReminderId::new();
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    let have: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM message_reminders WHERE user_id = ? AND fired_at IS NULL",
    )
    .bind(me.to_string())
    .fetch_one(&mut *tx)
    .await?;
    if have >= schedule_limits::MAX_REMINDERS {
        return Err(AppError::BadRequest(format!(
            "you can have up to {} reminders waiting",
            schedule_limits::MAX_REMINDERS
        )));
    }
    crate::access::channel_live_in(&mut tx, channel).await?;
    sqlx::query(
        "INSERT INTO message_reminders (id, user_id, message_id, remind_at, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(me.to_string())
    .bind(message.to_string())
    .bind(&remind_at)
    .bind(now())
    .execute(&mut *tx)
    .await?;
    let made = one(&mut *tx, me, id).await?.ok_or(AppError::NotFound)?;
    tx.commit().await?;
    publish_saved(&s, me, made.clone()).await;
    Ok((StatusCode::CREATED, Json(made)))
}

/// `DELETE /reminders/{id}`: cancel one still to come, or mark a due one done. `204`.
async fn remove(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ReminderId>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let _order = s.hub.in_order().await;
    let gone: Option<bool> = sqlx::query_scalar(
        "DELETE FROM message_reminders WHERE id = ? AND user_id = ?
         RETURNING fired_at IS NOT NULL AND seen_at IS NULL",
    )
    .bind(id.to_string())
    .bind(me.to_string())
    .fetch_optional(&s.db)
    .await?;
    let Some(was_unread) = gone else {
        return Err(AppError::NotFound);
    };
    s.hub
        .publish(&s.db, Event::ReminderRemoved { user_id: me, id })
        .await;
    drop(_order);
    if was_unread {
        recount(&s, me).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /reminders/{id}/seen`: a due one opened (read on every device). `204`, also when it already was.
async fn seen(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ReminderId>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    if one(&s.db, me, id).await?.is_none_or(|r| !r.due) {
        return Err(AppError::NotFound);
    }
    let order = s.hub.in_order().await;
    let changed = sqlx::query(
        "UPDATE message_reminders SET seen_at = ? WHERE id = ? AND user_id = ? AND fired_at IS NOT NULL
         AND seen_at IS NULL",
    )
    .bind(now())
    .bind(id.to_string())
    .bind(me.to_string())
    .execute(&s.db)
    .await?
    .rows_affected();
    if changed > 0 {
        if let Some(r) = one(&s.db, me, id).await? {
            publish_saved(&s, me, r).await;
        }
        drop(order);
        recount(&s, me).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /reminders/{id}/alerted`: this device tells its person about a reminder that came due while none of theirs
/// was connected (#412). `true` for the one claim that wins (it tells them), `false` once one has, or for one that
/// isn't missed (not due, already seen, or told live). Their other devices hear it's no longer missed.
async fn alerted(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ReminderId>,
) -> AppResult<Json<bool>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    if one(&s.db, me, id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    let _order = s.hub.in_order().await;
    let won = sqlx::query(
        "UPDATE message_reminders SET alerted_at = ? WHERE id = ? AND user_id = ? AND fired_at IS NOT NULL
         AND seen_at IS NULL AND alerted_at IS NULL",
    )
    .bind(now())
    .bind(id.to_string())
    .bind(me.to_string())
    .execute(&s.db)
    .await?
    .rows_affected()
        > 0;
    if won && let Some(r) = one(&s.db, me, id).await? {
        publish_saved(&s, me, r).await;
    }
    Ok(Json(won))
}

/// Mark all read (#306) covers `me`'s due reminders too: each is seen, and their devices hear it. Whether any were.
/// The caller tells the count.
pub(crate) async fn mark_all_seen(s: &AppState, me: UserId) -> AppResult<bool> {
    let _order = s.hub.in_order().await;
    let ids: Vec<String> = sqlx::query_scalar(
        "UPDATE message_reminders SET seen_at = ? WHERE user_id = ? AND fired_at IS NOT NULL AND seen_at IS NULL
         RETURNING id",
    )
    .bind(now())
    .bind(me.to_string())
    .fetch_all(&s.db)
    .await?;
    for id in &ids {
        let id: ReminderId = id.parse().map_err(anyhow::Error::from)?;
        if let Some(r) = one(&s.db, me, id).await? {
            publish_saved(s, me, r).await;
        }
    }
    Ok(!ids.is_empty())
}

/// Send every reminder due at `now` (up to [`SWEEP_BATCH`]): each lands in its person's inbox and their devices hear
/// it. One whose message they can no longer see goes quietly. Due ones older than the inbox's days go too. Returns
/// how many were sent.
pub async fn remind_due(s: &AppState, now: DateTime<Utc>) -> AppResult<usize> {
    let at = stamp(now);
    sqlx::query("DELETE FROM message_reminders WHERE fired_at IS NOT NULL AND fired_at < ?")
        .bind(stamp(now - TimeDelta::days(crate::inbox::DAYS)))
        .execute(&s.db)
        .await?;
    let due: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, user_id FROM message_reminders WHERE fired_at IS NULL AND remind_at <= ?
         ORDER BY remind_at, id LIMIT ?",
    )
    .bind(&at)
    .bind(SWEEP_BATCH)
    .fetch_all(&s.db)
    .await?;
    let mut sent = 0;
    for (id, user) in due {
        let id: ReminderId = id.parse().map_err(anyhow::Error::from)?;
        let user: UserId = user.parse().map_err(anyhow::Error::from)?;
        let order = s.hub.in_order().await;
        let mut tx = begin_write(&s.db).await?;
        // With one of their devices connected they hear it now; with none it's missed until one claims it (#412).
        let live = s.hub.connections_for(user) > 0;
        let changed = sqlx::query(
            "UPDATE message_reminders SET fired_at = ?1, alerted_at = CASE WHEN ?2 THEN ?1 END
             WHERE id = ?3 AND fired_at IS NULL",
        )
        .bind(&at)
        .bind(live)
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if changed == 0 {
            continue;
        }
        let Some(r) = one(&mut *tx, user, id).await? else {
            // its message is gone from them: nothing to remind them of
            sqlx::query("DELETE FROM message_reminders WHERE id = ?")
                .bind(id.to_string())
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            s.hub
                .publish(&s.db, Event::ReminderRemoved { user_id: user, id })
                .await;
            continue;
        };
        tx.commit().await?;
        publish_saved(s, user, r).await;
        drop(order);
        recount(s, user).await;
        sent += 1;
    }
    Ok(sent)
}

/// Send due reminders every [`SWEEP_EVERY`] (the first tick runs at once: the boot sweep). Call after binding the
/// listener.
pub fn spawn_sweeper(s: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            match remind_due(&s, Utc::now()).await {
                Ok(0) => {}
                Ok(n) => tracing::debug!(n, "message reminders sent"),
                Err(e) => tracing::warn!(error = %e, "message reminder sweep failed"),
            }
        }
    });
}
