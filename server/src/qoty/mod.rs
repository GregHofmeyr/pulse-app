//! Quote of the Year: one vote per server at a time and one per (server, year). Options start as
//! the year's top 5 quotes by reaction score (ties at 5th all go in), anyone in the server can add
//! another quote, vote, move or take back their vote (votes are public), end it early or reopen an
//! early end within a day. Ending posts winner cards (system messages) and records the wins.
//!
//! Every write that changes what an event shows holds `hub.in_order()` from before its write
//! transaction until its events are published, so clients get absolute state in commit order.

pub mod ending;
pub mod routes;

pub use ending::{SWEEP_EVERY, end_overdue, spawn_sweeper};
pub use routes::router;

use std::collections::HashMap;
use std::ops::RangeInclusive;

use chrono::{DateTime, Utc};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{MessageId, QotyEventId, ServerId, UserId};
use pulse_protocol::rest::{Message, QotyEndReason, QotyEvent, QotyOption};
use sqlx::{SqliteConnection, SqlitePool};

use crate::AppState;
use crate::db::stamp;
use crate::error::{AppError, AppResult};
use crate::messages::routes::{COLS, Row, fill};
use crate::quotes::{parse_id, parse_opt};

/// The automatic options: this many best quotes of the year, plus any tied with the last.
pub const TOP: usize = 5;
/// A vote's length, when it has one: 5 minutes to 14 days.
pub const MINUTES: RangeInclusive<u32> = 5..=20160;
/// How long after an early end the vote can be reopened.
pub const REOPEN_HOURS: i64 = 24;
/// The first year a vote can be for (the last is the current UTC year).
pub const FIRST_YEAR: i32 = 2000;
/// The most options a vote takes by hand: every update of it carries every option to everyone.
pub const MAX_OPTIONS: i64 = 50;

fn parse_time(s: &str) -> AppResult<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(s)
        .map_err(anyhow::Error::from)?
        .with_timezone(&Utc))
}

/// `None` (open until ended) or a length in [`MINUTES`].
pub(crate) fn check_minutes(minutes: Option<u32>) -> AppResult<()> {
    match minutes {
        Some(m) if !MINUTES.contains(&m) => Err(AppError::BadRequest(
            "a vote runs for 5 minutes to 14 days, or until it's ended".into(),
        )),
        _ => Ok(()),
    }
}

/// When a vote of `minutes` starting `now` ends.
pub(crate) fn ends_at(now: DateTime<Utc>, minutes: Option<u32>) -> Option<String> {
    minutes.map(|m| stamp(now + chrono::Duration::minutes(i64::from(m))))
}

fn reason_of(s: &str) -> AppResult<QotyEndReason> {
    match s {
        "early" => Ok(QotyEndReason::Early),
        "time" => Ok(QotyEndReason::Time),
        other => Err(AppError::Internal(anyhow::anyhow!(
            "unknown end_reason {other}"
        ))),
    }
}

pub(crate) fn reason_str(r: QotyEndReason) -> &'static str {
    match r {
        QotyEndReason::Early => "early",
        QotyEndReason::Time => "time",
    }
}

const EVENT_COLS: &str =
    "id, server_id, year, started_by, started_at, ends_at, ended_at, ended_by, end_reason";

type EventRow = (
    String,
    String,
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// The event's server, if it exists.
pub(crate) async fn server_of(db: &SqlitePool, id: QotyEventId) -> AppResult<Option<ServerId>> {
    let server: Option<String> =
        sqlx::query_scalar("SELECT server_id FROM qoty_events WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(db)
            .await?;
    parse_opt(server.as_deref())
}

/// The server's live event, if any.
pub(crate) async fn live_in(
    db: &mut SqliteConnection,
    server: ServerId,
) -> AppResult<Option<QotyEventId>> {
    let id: Option<String> =
        sqlx::query_scalar("SELECT id FROM qoty_events WHERE server_id = ? AND ended_at IS NULL")
            .bind(server.to_string())
            .fetch_optional(db)
            .await?;
    parse_opt(id.as_deref())
}

/// Whether the event is live: 409 once it has ended or its time has run out (even before a sweep
/// ends it), 404 if it's gone.
pub(crate) async fn require_live(db: &mut SqliteConnection, id: QotyEventId) -> AppResult<()> {
    let row: Option<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT ended_at, ends_at FROM qoty_events WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(db)
            .await?;
    match row {
        None => Err(AppError::NotFound),
        Some((None, ends_at)) if !past(ends_at.as_deref()) => Ok(()),
        Some(_) => Err(AppError::Conflict("This vote is over.".into())),
    }
}

/// Whether a deadline (`None`: open until ended) has passed.
pub(crate) fn past(ends_at: Option<&str>) -> bool {
    ends_at.is_some_and(|t| t <= crate::db::now().as_str())
}

/// The automatic options for `year` in `server`, best first: the year's ranked quotes (see
/// [`crate::quotes::ranked`]) that never won a vote, the top [`TOP`] and everything tied with the
/// last of them. A quote nobody reacted to is never one.
pub(crate) async fn top_options(
    db: &mut SqliteConnection,
    server: ServerId,
    year: i32,
) -> AppResult<Vec<String>> {
    let ranked = crate::quotes::ranked(db, server, Some(year), true, None).await?;
    let cut = ranked.get(TOP - 1).map_or(0, |r| r.1);
    Ok(ranked
        .into_iter()
        .enumerate()
        .take_while(|(i, r)| *i < TOP || r.1 == cut)
        .map(|(_, r)| r.0)
        .collect())
}

/// Every vote of the event, in vote order, by option.
pub(crate) async fn votes_of<'e, E>(
    db: E,
    id: QotyEventId,
) -> AppResult<HashMap<String, Vec<UserId>>>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT message_id, user_id FROM qoty_votes WHERE event_id = ? ORDER BY rowid",
    )
    .bind(id.to_string())
    .fetch_all(db)
    .await?;
    let mut out: HashMap<String, Vec<UserId>> = HashMap::new();
    for (message, user) in rows {
        out.entry(message).or_default().push(parse_id(&user)?);
    }
    Ok(out)
}

/// The event as everyone sees it (`me` only marks your own reactions and awards on the quotes).
pub(crate) async fn load(
    db: &SqlitePool,
    id: QotyEventId,
    me: Option<UserId>,
) -> AppResult<Option<QotyEvent>> {
    let row: Option<EventRow> = sqlx::query_as(&format!(
        "SELECT {EVENT_COLS} FROM qoty_events WHERE id = ?"
    ))
    .bind(id.to_string())
    .fetch_optional(db)
    .await?;
    match row {
        Some(row) => Ok(Some(build(db, row, me).await?)),
        None => Ok(None),
    }
}

async fn build(db: &SqlitePool, row: EventRow, me: Option<UserId>) -> AppResult<QotyEvent> {
    let (id, server, year, started_by, started_at, ends_at, ended_at, ended_by, end_reason) = row;
    let event_id: QotyEventId = parse_id(&id)?;
    let options: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT message_id, added_by FROM qoty_options WHERE event_id = ? ORDER BY rowid",
    )
    .bind(&id)
    .fetch_all(db)
    .await?;
    let mut votes = votes_of(db, event_id).await?;
    let members: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM server_members WHERE server_id = ?")
            .bind(&server)
            .fetch_one(db)
            .await?;

    let mut messages: HashMap<MessageId, Message> = HashMap::new();
    if !options.is_empty() {
        let mut q = sqlx::QueryBuilder::new(format!("SELECT {COLS} FROM messages WHERE id IN ("));
        let mut list = q.separated(", ");
        for (m, _) in &options {
            list.push_bind(m);
        }
        q.push(")");
        let rows: Vec<Row> = q.build_query_as().fetch_all(db).await?;
        for m in fill(db, me, rows).await? {
            messages.insert(m.id, m);
        }
    }
    let mut out = Vec::with_capacity(options.len());
    for (message, added_by) in options {
        let Some(msg) = messages.remove(&parse_id::<MessageId>(&message)?) else {
            continue;
        };
        let voters = votes.remove(&message).unwrap_or_default();
        out.push(QotyOption {
            message: msg,
            added_by: parse_opt(added_by.as_deref())?,
            votes: voters.len() as u32,
            voters,
        });
    }
    let end_reason = end_reason.as_deref().map(reason_of).transpose()?;
    let reopen_until = match (&ended_at, end_reason) {
        (Some(at), Some(QotyEndReason::Early)) => Some(stamp(
            parse_time(at)? + chrono::Duration::hours(REOPEN_HOURS),
        )),
        _ => None,
    };
    Ok(QotyEvent {
        id: event_id,
        server_id: parse_id(&server)?,
        year: year as i32,
        started_by: parse_id(&started_by)?,
        started_at,
        ends_at,
        ended_at,
        ended_by: parse_opt(ended_by.as_deref())?,
        end_reason,
        reopen_until,
        voters: out.iter().map(|o| o.votes).sum(),
        options: out,
        members: members as u32,
    })
}

/// For Ready: every live event, and every event ended early within [`REOPEN_HOURS`] (so the
/// reopen button can show), oldest first. Every server's, as QotY events go to everyone, and as
/// the events show them (nothing marked as yours), so the first update changes nothing else.
pub async fn for_ready(db: &SqlitePool) -> AppResult<Vec<QotyEvent>> {
    let since = stamp(Utc::now() - chrono::Duration::hours(REOPEN_HOURS));
    let rows: Vec<EventRow> = sqlx::query_as(&format!(
        "SELECT {EVENT_COLS} FROM qoty_events
         WHERE ended_at IS NULL OR (end_reason = 'early' AND ended_at > ?)
         ORDER BY started_at, id"
    ))
    .bind(since)
    .fetch_all(db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(build(db, row, None).await?);
    }
    Ok(out)
}

/// Publish the event as it now stands. Called after commit, under the caller's `in_order` guard
/// (never takes it: tokio's mutex isn't reentrant). A failure to read it back is logged, not
/// returned: the change it reports has already landed.
pub(crate) async fn publish(
    s: &AppState,
    id: QotyEventId,
    wrap: fn(QotyEvent) -> Event,
) -> Option<QotyEvent> {
    match load(&s.db, id, None).await {
        Ok(Some(event)) => {
            s.hub.publish(&s.db, wrap(event.clone())).await;
            Some(event)
        }
        Ok(None) => None,
        Err(e) => {
            tracing::error!(error = ?e, %id, "couldn't load a Quote of the Year event to publish");
            None
        }
    }
}

/// `QotyUpdated` for `id`, e.g. after a quote that was an option was deleted. Under the caller's
/// `in_order` guard.
pub async fn publish_updated(s: &AppState, id: QotyEventId) {
    publish(s, id, |event| Event::QotyUpdated { event }).await;
}
