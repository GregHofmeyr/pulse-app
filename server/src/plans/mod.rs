//! Plan something (#387). A plan is an ordinary message (its `content` is the plan as plain text, for clients that
//! don't know plans) with a row in `plans` and everyone's answers in `plan_rsvps`, as polls are (#157). Unlike a
//! poll, nothing on a plan is per person, so every read attaches the same [`Plan`] and a change goes out whole, as
//! `PlanChanged`.
//!
//! Reminders: an hour before a plan starts, its Going and Maybe people are pinged by a system line (a mention each,
//! so it reaches their inbox and their notifications the way a typed `@name` does). The sweep (every
//! [`SWEEP_EVERY`], and at boot) finds due plans from `plans.reminded_at` in the database, so a restart neither loses
//! nor repeats one. Whoever's `UPDATE … WHERE reminded_at IS NULL` changes the row posts the line, in the same
//! transaction, so it is posted once; a cancelled plan is never reminded.
//!
//! The forecast ([`weather`]) and the calendar file ([`ics`]) are read from here by their routes.

pub mod ics;
pub mod routes;
pub mod weather;

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{
    CreatePlanRequest, Message, MessageKind, Plan, PlanReminder, Rsvp, plan_limits,
};
use sqlx::{SqliteConnection, SqlitePool};

use crate::AppState;
use crate::db::{begin_write, stamp};
use crate::error::{AppError, AppResult};
use crate::messages::system;

pub use routes::router;

/// How often due reminders are sent.
pub const SWEEP_EVERY: Duration = Duration::from_secs(30);

/// The payload `type` of a plan's reminder line.
pub const PLAN_REMINDER: &str = "plan_reminder";

/// A plan as stored, with its message's author.
#[derive(Clone, Debug)]
pub(crate) struct Stored {
    pub message_id: MessageId,
    pub channel_id: ChannelId,
    pub author_id: Option<UserId>,
    pub title: String,
    pub starts_at: String,
    pub ends_at: Option<String>,
    pub place: Option<String>,
    pub note: Option<String>,
    pub cancelled_at: Option<String>,
    pub edited_at: Option<String>,
}

type StoredRow = (
    String,
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

const STORED_COLS: &str = "p.message_id, p.channel_id, m.author_id, p.title, p.starts_at, p.ends_at, p.place, \
     p.note, p.cancelled_at, p.edited_at FROM plans p JOIN messages m ON m.id = p.message_id";

fn stored_from(r: StoredRow) -> AppResult<Stored> {
    let (
        message_id,
        channel_id,
        author_id,
        title,
        starts_at,
        ends_at,
        place,
        note,
        cancelled_at,
        edited_at,
    ) = r;
    Ok(Stored {
        message_id: message_id.parse().map_err(anyhow::Error::from)?,
        channel_id: channel_id.parse().map_err(anyhow::Error::from)?,
        author_id: author_id
            .map(|a| a.parse())
            .transpose()
            .map_err(anyhow::Error::from)?,
        title,
        starts_at,
        ends_at,
        place,
        note,
        cancelled_at,
        edited_at,
    })
}

/// A live (not deleted) message's plan.
pub(crate) async fn load(db: &mut SqliteConnection, id: MessageId) -> AppResult<Option<Stored>> {
    let row: Option<StoredRow> = sqlx::query_as(&format!(
        "SELECT {STORED_COLS} WHERE p.message_id = ? AND m.deleted_at IS NULL"
    ))
    .bind(id.to_string())
    .fetch_optional(&mut *db)
    .await?;
    row.map(stored_from).transpose()
}

pub(crate) fn rsvp_str(r: Rsvp) -> &'static str {
    match r {
        Rsvp::Going => "going",
        Rsvp::Maybe => "maybe",
        Rsvp::Cant => "cant",
    }
}

fn rsvp_from(s: &str) -> Option<Rsvp> {
    match s {
        "going" => Some(Rsvp::Going),
        "maybe" => Some(Rsvp::Maybe),
        "cant" => Some(Rsvp::Cant),
        _ => None,
    }
}

/// Each answer: (answer, who), oldest first.
type Answers = Vec<(Rsvp, UserId)>;

async fn answers_for(
    db: &mut SqliteConnection,
    ids: &[String],
) -> AppResult<HashMap<String, Answers>> {
    let mut out: HashMap<String, Answers> = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let mut q = sqlx::QueryBuilder::new(
        "SELECT message_id, rsvp, user_id FROM plan_rsvps WHERE message_id IN (",
    );
    let mut list = q.separated(", ");
    for id in ids {
        list.push_bind(id);
    }
    q.push(") ORDER BY at, rowid");
    let rows: Vec<(String, String, String)> = q.build_query_as().fetch_all(&mut *db).await?;
    for (message, rsvp, user) in rows {
        let Some(rsvp) = rsvp_from(&rsvp) else {
            continue;
        };
        let user: UserId = user.parse().map_err(anyhow::Error::from)?;
        out.entry(message).or_default().push((rsvp, user));
    }
    Ok(out)
}

/// The plan as everyone sees it.
pub(crate) fn view(p: &Stored, answers: &[(Rsvp, UserId)]) -> Plan {
    let who = |want: Rsvp| -> Vec<UserId> {
        answers
            .iter()
            .filter(|(r, _)| *r == want)
            .map(|(_, u)| *u)
            .collect()
    };
    Plan {
        title: p.title.clone(),
        starts_at: p.starts_at.clone(),
        ends_at: p.ends_at.clone(),
        place: p.place.clone(),
        note: p.note.clone(),
        going: who(Rsvp::Going),
        maybe: who(Rsvp::Maybe),
        cant: who(Rsvp::Cant),
        cancelled_at: p.cancelled_at.clone(),
        edited_at: p.edited_at.clone(),
    }
}

/// One live plan as it is now.
pub(crate) async fn view_of(db: &mut SqliteConnection, id: MessageId) -> AppResult<Option<Plan>> {
    let Some(p) = load(db, id).await? else {
        return Ok(None);
    };
    let answers = answers_for(db, &[id.to_string()])
        .await?
        .remove(&id.to_string())
        .unwrap_or_default();
    Ok(Some(view(&p, &answers)))
}

/// Set each live plan's `plan` (and clear any other).
pub(crate) async fn attach(db: &SqlitePool, page: &mut [Message]) -> AppResult<()> {
    let ids: Vec<String> = page
        .iter()
        .filter(|m| !m.deleted && m.kind == MessageKind::Normal)
        .map(|m| m.id.to_string())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let mut conn = db.acquire().await?;
    let mut q = sqlx::QueryBuilder::new(format!("SELECT {STORED_COLS} WHERE p.message_id IN ("));
    let mut list = q.separated(", ");
    for id in &ids {
        list.push_bind(id);
    }
    q.push(")");
    let rows: Vec<StoredRow> = q.build_query_as().fetch_all(&mut *conn).await?;
    if rows.is_empty() {
        return Ok(());
    }
    let plans: Vec<Stored> = rows
        .into_iter()
        .map(stored_from)
        .collect::<AppResult<_>>()?;
    let plan_ids: Vec<String> = plans.iter().map(|p| p.message_id.to_string()).collect();
    let mut answers = answers_for(&mut conn, &plan_ids).await?;
    let mut by_id: HashMap<MessageId, Plan> = plans
        .iter()
        .map(|p| {
            let a = answers
                .remove(&p.message_id.to_string())
                .unwrap_or_default();
            (p.message_id, view(p, &a))
        })
        .collect();
    for m in page.iter_mut() {
        m.plan = by_id.remove(&m.id).map(Box::new);
    }
    Ok(())
}

/// A message is being deleted (in `tx`): its plan and answers go with it, so it is never reminded again.
pub(crate) async fn on_delete(tx: &mut SqliteConnection, id: MessageId) -> AppResult<()> {
    sqlx::query("DELETE FROM plans WHERE message_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// A plan's fields as checked and stored.
#[derive(Clone, Debug, PartialEq)]
pub struct Cleaned {
    pub title: String,
    pub starts_at: String,
    pub ends_at: Option<String>,
    pub place: Option<String>,
    pub note: Option<String>,
}

fn one_line(raw: &str, max: usize, what: &str) -> AppResult<String> {
    let t = raw.trim();
    let n = t.chars().count();
    if n == 0 || n > max || t.chars().any(char::is_control) {
        return Err(AppError::BadRequest(format!(
            "a plan's {what} is 1-{max} characters on one line"
        )));
    }
    Ok(t.to_string())
}

/// The request's fields, trimmed and checked at `at`: a title, a start that isn't over (a few minutes' grace for a
/// slow form) and at most [`plan_limits::MAX_DAYS_AHEAD`] away, an optional end after the start (at most
/// [`plan_limits::MAX_HOURS_LONG`] later), and an optional place and note (blank = none).
pub fn clean(req: &CreatePlanRequest, at: DateTime<Utc>) -> AppResult<Cleaned> {
    let title = one_line(&req.title, plan_limits::TITLE_CHARS, "title")?;
    let starts = DateTime::parse_from_rfc3339(req.starts_at.trim())
        .map_err(|_| AppError::BadRequest("a plan's time isn't a date and time".into()))?
        .with_timezone(&Utc);
    if starts < at - TimeDelta::minutes(5) {
        return Err(AppError::BadRequest("that time has already passed".into()));
    }
    if starts > at + TimeDelta::days(plan_limits::MAX_DAYS_AHEAD) {
        return Err(AppError::BadRequest(
            "a plan can be at most two years ahead".into(),
        ));
    }
    let ends_at = match req.ends_at.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(e) => {
            let ends = DateTime::parse_from_rfc3339(e)
                .map_err(|_| AppError::BadRequest("a plan's end isn't a date and time".into()))?
                .with_timezone(&Utc);
            if ends <= starts {
                return Err(AppError::BadRequest(
                    "a plan has to end after it starts".into(),
                ));
            }
            if ends > starts + TimeDelta::hours(plan_limits::MAX_HOURS_LONG) {
                return Err(AppError::BadRequest(
                    "a plan can run for at most a week".into(),
                ));
            }
            Some(stamp(ends))
        }
    };
    let place = match req.place.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(p) => Some(one_line(p, plan_limits::PLACE_CHARS, "place")?),
    };
    let note = match req.note.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(n) => {
            let count = n.chars().count();
            // a note may run over lines, but no other control characters
            if count > plan_limits::NOTE_CHARS || n.chars().any(|c| c.is_control() && c != '\n') {
                return Err(AppError::BadRequest(format!(
                    "a plan's note is at most {} characters",
                    plan_limits::NOTE_CHARS
                )));
            }
            Some(n.to_string())
        }
    };
    Ok(Cleaned {
        title,
        starts_at: stamp(starts),
        ends_at,
        place,
        note,
    })
}

/// What `reminded_at` starts as for a plan starting at `starts_at`, made or moved at `now`: already done when the
/// reminder's time has come (everyone just saw the plan), otherwise still to come.
pub fn reminded_at_for(starts_at: &str, now: DateTime<Utc>) -> Option<String> {
    let starts = DateTime::parse_from_rfc3339(starts_at)
        .ok()?
        .with_timezone(&Utc);
    (starts - TimeDelta::minutes(plan_limits::REMIND_MINUTES) <= now).then(|| stamp(now))
}

/// The plan as text, for clients that don't know plans (and previews, search and notifications). The time is UTC:
/// the one time everyone can read the same way.
pub fn fallback_text(c: &Cleaned) -> String {
    let utc = |s: &str| DateTime::parse_from_rfc3339(s).map(|t| t.with_timezone(&Utc));
    let when = match utc(&c.starts_at) {
        Ok(start) => {
            let end = c.ends_at.as_deref().and_then(|e| utc(e).ok());
            match end {
                // the same day: just the end's time; another day: its day too
                Some(end) if end.date_naive() == start.date_naive() => format!(
                    "{}–{} UTC",
                    start.format("%a %-d %b %Y, %H:%M"),
                    end.format("%H:%M")
                ),
                Some(end) => format!(
                    "{} – {} UTC",
                    start.format("%a %-d %b %Y, %H:%M"),
                    end.format("%a %-d %b, %H:%M")
                ),
                None => start.format("%a %-d %b %Y, %H:%M UTC").to_string(),
            }
        }
        Err(_) => c.starts_at.clone(),
    };
    let mut t = format!("📅 Plan: {}\n🕒 {when}", c.title);
    if let Some(p) = &c.place {
        t.push_str("\n📍 ");
        t.push_str(p);
    }
    if let Some(n) = &c.note {
        t.push('\n');
        t.push_str(n);
    }
    t.push_str("\n(Update Pulse to RSVP)");
    t
}

/// The reminder line's text.
pub fn reminder_text(title: &str) -> String {
    format!("⏰ {title} starts in an hour")
}

/// A reminder line's stored payload.
pub fn reminder_payload(line: &PlanReminder) -> AppResult<serde_json::Value> {
    let mut v = serde_json::to_value(line).map_err(anyhow::Error::from)?;
    v["type"] = PLAN_REMINDER.into();
    Ok(v)
}

/// The reminder in a stored payload, if it is one.
pub fn reminder(payload: &str) -> Option<PlanReminder> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    if v.get("type")?.as_str()? != PLAN_REMINDER {
        return None;
    }
    serde_json::from_value(v).ok()
}

/// Announce a plan as it is now (after a commit).
pub(crate) async fn announce(s: &AppState, channel_id: ChannelId, message_id: MessageId) {
    let plan = match s.db.acquire().await {
        Ok(mut conn) => view_of(&mut conn, message_id).await,
        Err(e) => Err(e.into()),
    };
    match plan {
        Ok(Some(plan)) => {
            s.hub
                .publish(
                    &s.db,
                    Event::PlanChanged {
                        channel_id,
                        message_id,
                        plan: Box::new(plan),
                    },
                )
                .await;
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, "couldn't read a plan to announce it"),
    }
}

/// The plan's Going and Maybe people who can still see its chat (a DM or group's members, a server's members) and
/// haven't turned Plan reminders off (#399).
async fn to_remind(db: &mut SqliteConnection, p: &Stored) -> AppResult<Vec<UserId>> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT r.user_id FROM plan_rsvps r JOIN channels c ON c.id = ?
         JOIN users u ON u.id = r.user_id
         WHERE r.message_id = ? AND r.rsvp IN ('going', 'maybe') AND u.plan_reminders = 1
           AND (EXISTS (SELECT 1 FROM channel_members cm WHERE cm.channel_id = c.id AND cm.user_id = r.user_id)
                OR (c.server_id IS NOT NULL AND EXISTS (SELECT 1 FROM server_members sm
                     WHERE sm.server_id = c.server_id AND sm.user_id = r.user_id)))
         ORDER BY r.at, r.rowid",
    )
    .bind(p.channel_id.to_string())
    .bind(p.message_id.to_string())
    .fetch_all(&mut *db)
    .await?;
    rows.into_iter()
        .map(|u| {
            u.parse()
                .map_err(|e| AppError::from(anyhow::Error::from(e)))
        })
        .collect()
}

/// Send every reminder due at `now`: plans not cancelled and not yet reminded that start within the hour (and
/// haven't started: one whose hour passed while the server was down is marked done without a line). Returns how
/// many lines it posted. A plan in a channel that is in Recently deleted (or whose server is) is left for after a
/// restore, as polls are (#334).
pub async fn remind_due(s: &AppState, now: DateTime<Utc>) -> AppResult<usize> {
    let at = stamp(now);
    let due_by = stamp(now + TimeDelta::minutes(plan_limits::REMIND_MINUTES));
    let rows: Vec<StoredRow> = sqlx::query_as(&format!(
        "SELECT {STORED_COLS} WHERE p.reminded_at IS NULL AND p.cancelled_at IS NULL AND p.starts_at <= ? \
         AND m.deleted_at IS NULL AND p.channel_id IN (SELECT id FROM channels WHERE deleted_at IS NULL \
         AND (server_id IS NULL OR server_id NOT IN {}))",
        crate::access::DELETED_SERVERS
    ))
    .bind(&due_by)
    .fetch_all(&s.db)
    .await?;
    let mut posted = 0;
    for r in rows {
        let p = stored_from(r)?;
        // each line goes out in commit order with the chat's other messages (as a send)
        let _order = s.hub.in_order().await;
        let mut tx = begin_write(&s.db).await?;
        if !crate::access::channel_is_live_in(&mut tx, p.channel_id).await? {
            continue;
        }
        // as read: an edit (a new time) or a cancel since leaves it to the next sweep
        let changed = sqlx::query(
            "UPDATE plans SET reminded_at = ? WHERE message_id = ? AND reminded_at IS NULL \
             AND cancelled_at IS NULL AND starts_at = ?",
        )
        .bind(&at)
        .bind(p.message_id.to_string())
        .bind(&p.starts_at)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if changed == 0 {
            continue;
        }
        let people = to_remind(&mut tx, &p).await?;
        // started already (the server was down through its hour), or nobody to tell: done, quietly
        if p.starts_at <= at || people.is_empty() {
            tx.commit().await?;
            continue;
        }
        let line = PlanReminder {
            plan_message_id: p.message_id,
            title: p.title.clone(),
            starts_at: p.starts_at.clone(),
        };
        let mut msg = system::insert(
            &mut tx,
            p.channel_id,
            reminder_text(&p.title),
            reminder_payload(&line)?,
        )
        .await?;
        // a typed mention each: the inbox and notifications take it as one
        crate::messages::mentions::store(&mut tx, msg.id, &people, true).await?;
        tx.commit().await?;
        msg.mentions = people;
        posted += 1;
        s.hub
            .publish(
                &s.db,
                Event::MessageCreated {
                    message: msg,
                    nonce: None,
                },
            )
            .await;
    }
    Ok(posted)
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
                Ok(n) => tracing::debug!(n, "plan reminders sent"),
                Err(e) => tracing::warn!(error = %e, "plan reminder sweep failed"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(title: &str, starts_at: &str) -> CreatePlanRequest {
        CreatePlanRequest {
            title: title.into(),
            starts_at: starts_at.into(),
            ends_at: None,
            place: None,
            note: None,
        }
    }

    fn at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-16T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn a_plan_is_cleaned_and_its_time_kept_as_utc() {
        let mut r = req("  Braai at kim's ", "2026-11-20T18:30:00+02:00");
        r.place = Some("  Cape Town ".into());
        r.note = Some("  bring ice\nand chairs ".into());
        let c = clean(&r, at()).unwrap();
        assert_eq!(c.title, "Braai at kim's");
        assert_eq!(c.starts_at, "2026-11-20T16:30:00.000Z");
        assert_eq!(c.place.as_deref(), Some("Cape Town"));
        assert_eq!(c.note.as_deref(), Some("bring ice\nand chairs"));
        // blank place and note are none
        r.place = Some("   ".into());
        r.note = Some("".into());
        let c = clean(&r, at()).unwrap();
        assert_eq!((c.place, c.note), (None, None));
    }

    #[test]
    fn a_plan_is_refused_when_its_fields_or_time_are_wrong() {
        assert!(clean(&req(" ", "2026-11-20T18:00:00Z"), at()).is_err());
        assert!(clean(&req(&"t".repeat(101), "2026-11-20T18:00:00Z"), at()).is_err());
        assert!(clean(&req(&"é".repeat(100), "2026-11-20T18:00:00Z"), at()).is_ok());
        assert!(clean(&req("a\nb", "2026-11-20T18:00:00Z"), at()).is_err());
        assert!(clean(&req("ok", "next friday"), at()).is_err());
        // over (past the few minutes' grace), and too far ahead
        assert!(clean(&req("ok", "2026-11-16T09:50:00Z"), at()).is_err());
        assert!(clean(&req("ok", "2026-11-16T09:58:00Z"), at()).is_ok());
        assert!(clean(&req("ok", "2028-12-01T00:00:00Z"), at()).is_err());
        let mut r = req("ok", "2026-11-20T18:00:00Z");
        r.place = Some("p\tq".into());
        assert!(clean(&r, at()).is_err());
        r.place = None;
        r.note = Some("n".repeat(501));
        assert!(clean(&r, at()).is_err());
    }

    #[test]
    fn an_end_is_optional_after_the_start_and_at_most_a_week_on() {
        let mut r = req("ok", "2026-11-20T18:00:00+02:00");
        r.ends_at = Some(" 2026-11-20T22:30:00+02:00 ".into());
        assert_eq!(
            clean(&r, at()).unwrap().ends_at.as_deref(),
            Some("2026-11-20T20:30:00.000Z")
        );
        r.ends_at = Some("".into());
        assert_eq!(clean(&r, at()).unwrap().ends_at, None);
        // at or before the start, past a week, or not a time
        for bad in [
            "2026-11-20T18:00:00+02:00",
            "2026-11-20T17:00:00+02:00",
            "2026-11-27T18:00:01+02:00",
            "later",
        ] {
            r.ends_at = Some(bad.into());
            assert!(clean(&r, at()).is_err(), "{bad}");
        }
        r.ends_at = Some("2026-11-27T18:00:00+02:00".into());
        assert!(clean(&r, at()).is_ok());
    }

    #[test]
    fn the_reminder_is_already_done_for_a_plan_less_than_an_hour_away() {
        assert_eq!(reminded_at_for("2026-11-16T12:00:00.000Z", at()), None);
        assert_eq!(
            reminded_at_for("2026-11-16T10:30:00.000Z", at()).as_deref(),
            Some("2026-11-16T10:00:00.000Z")
        );
        assert!(reminded_at_for("2026-11-16T11:00:00.000Z", at()).is_some());
    }

    #[test]
    fn the_fallback_text_and_the_reminder_line() {
        let c = Cleaned {
            title: "Braai".into(),
            starts_at: "2026-11-20T16:30:00.000Z".into(),
            ends_at: None,
            place: Some("Cape Town".into()),
            note: Some("bring ice".into()),
        };
        assert_eq!(
            fallback_text(&c),
            "📅 Plan: Braai\n🕒 Fri 20 Nov 2026, 16:30 UTC\n📍 Cape Town\nbring ice\n(Update Pulse to RSVP)"
        );
        assert_eq!(reminder_text("Braai"), "⏰ Braai starts in an hour");
        // an end the same day shows its time; another day, its day too
        let same_day = Cleaned {
            ends_at: Some("2026-11-20T19:00:00.000Z".into()),
            place: None,
            note: None,
            ..c.clone()
        };
        assert_eq!(
            fallback_text(&same_day),
            "📅 Plan: Braai\n🕒 Fri 20 Nov 2026, 16:30–19:00 UTC\n(Update Pulse to RSVP)"
        );
        let overnight = Cleaned {
            ends_at: Some("2026-11-21T01:00:00.000Z".into()),
            ..same_day
        };
        assert!(
            fallback_text(&overnight).contains("🕒 Fri 20 Nov 2026, 16:30 – Sat 21 Nov, 01:00 UTC")
        );
        let line = PlanReminder {
            plan_message_id: MessageId::new(),
            title: "Braai".into(),
            starts_at: c.starts_at.clone(),
        };
        let stored = reminder_payload(&line).unwrap().to_string();
        assert_eq!(reminder(&stored), Some(line));
        assert_eq!(reminder(r#"{"type":"pin"}"#), None);
    }

    #[test]
    fn a_view_lists_each_answer_in_order() {
        let (alex, sam, kim, jo) = (UserId::new(), UserId::new(), UserId::new(), UserId::new());
        let p = Stored {
            message_id: MessageId::new(),
            channel_id: ChannelId::new(),
            author_id: Some(alex),
            title: "T".into(),
            starts_at: "2026-11-20T16:30:00.000Z".into(),
            ends_at: None,
            place: None,
            note: None,
            cancelled_at: None,
            edited_at: None,
        };
        let v = view(
            &p,
            &[
                (Rsvp::Going, sam),
                (Rsvp::Maybe, kim),
                (Rsvp::Going, alex),
                (Rsvp::Cant, jo),
            ],
        );
        assert_eq!(v.going, vec![sam, alex]);
        assert_eq!(v.maybe, vec![kim]);
        assert_eq!(v.cant, vec![jo]);
    }
}
