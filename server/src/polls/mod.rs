//! Polls (#157). A poll is an ordinary message (its `content` is the poll as plain text, for clients that don't know
//! polls) with a row in `polls` and its votes in `poll_votes`. Every read attaches the poll as its reader may see it
//! ([`view`]): results only after you vote, as its creator, or once it has closed; never anyone's name on an
//! anonymous poll. Events can't carry per-person views, so a change goes out as `PollChanged` and each client fetches
//! its own.
//!
//! Closing: a poll past `ends_at` reads as closed at once and takes no votes; the sweep (every [`SWEEP_EVERY`], and
//! at boot) writes `closed_at` and posts the "ended" line. Whoever's `UPDATE … WHERE closed_at IS NULL` changes the
//! row (the creator's End poll or the sweep) posts the line, in the same transaction, so it is posted once.

pub mod routes;

use std::collections::HashMap;
use std::time::Duration;

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{Message, Poll, PollEnded, PollTally, poll_limits};
use sqlx::{SqliteConnection, SqlitePool};

use crate::AppState;
use crate::db::{begin_write, now};
use crate::error::{AppError, AppResult};
use crate::messages::system;

pub use routes::router;

/// How often polls whose time is up are closed.
pub const SWEEP_EVERY: Duration = Duration::from_secs(30);

/// A poll as stored, with its message's author.
#[derive(Clone, Debug)]
pub(crate) struct Stored {
    pub message_id: MessageId,
    pub channel_id: ChannelId,
    pub author_id: Option<UserId>,
    pub question: String,
    pub options: Vec<String>,
    pub multi: bool,
    pub anonymous: bool,
    pub ends_at: Option<String>,
    pub closed_at: Option<String>,
    pub edited_at: Option<String>,
}

impl Stored {
    /// Closed, or its time is up at `now`.
    pub fn over(&self, now: &str) -> bool {
        self.closed_at.is_some() || self.ends_at.as_deref().is_some_and(|e| e <= now)
    }
}

type StoredRow = (
    String,
    String,
    Option<String>,
    String,
    String,
    i64,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
);

const STORED_COLS: &str = "p.message_id, p.channel_id, m.author_id, p.question, p.options, p.multi, \
     p.anonymous, p.ends_at, p.closed_at, p.edited_at FROM polls p JOIN messages m ON m.id = p.message_id";

fn stored_from(r: StoredRow) -> AppResult<Stored> {
    let (
        message_id,
        channel_id,
        author_id,
        question,
        options,
        multi,
        anonymous,
        ends_at,
        closed_at,
        edited_at,
    ) = r;
    Ok(Stored {
        message_id: message_id.parse().map_err(anyhow::Error::from)?,
        channel_id: channel_id.parse().map_err(anyhow::Error::from)?,
        author_id: author_id
            .map(|a| a.parse())
            .transpose()
            .map_err(anyhow::Error::from)?,
        question,
        options: serde_json::from_str(&options).map_err(anyhow::Error::from)?,
        multi: multi != 0,
        anonymous: anonymous != 0,
        ends_at,
        closed_at,
        edited_at,
    })
}

/// A live (not deleted) message's poll.
pub(crate) async fn load(db: &mut SqliteConnection, id: MessageId) -> AppResult<Option<Stored>> {
    let row: Option<StoredRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {STORED_COLS} WHERE p.message_id = ? AND m.deleted_at IS NULL"
    )))
    .bind(id.to_string())
    .fetch_optional(&mut *db)
    .await?;
    row.map(stored_from).transpose()
}

/// Each vote: (option, voter), oldest first.
type Votes = Vec<(u32, UserId)>;

async fn votes_for(db: &mut SqliteConnection, ids: &[String]) -> AppResult<HashMap<String, Votes>> {
    let mut out: HashMap<String, Votes> = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let mut q = sqlx::QueryBuilder::new(
        "SELECT message_id, option, voter_id FROM poll_votes WHERE message_id IN (",
    );
    let mut list = q.separated(", ");
    for id in ids {
        list.push_bind(id);
    }
    q.push(") ORDER BY voted_at, rowid");
    let rows: Vec<(String, i64, String)> = q.build_query_as().fetch_all(&mut *db).await?;
    for (message, option, voter) in rows {
        let option = u32::try_from(option).map_err(anyhow::Error::from)?;
        let voter: UserId = voter.parse().map_err(anyhow::Error::from)?;
        out.entry(message).or_default().push((option, voter));
    }
    Ok(out)
}

/// Of `ids`, the polls `me` has voted in at some point (so still sees the results of after taking the vote back).
async fn seen_by(
    db: &mut SqliteConnection,
    me: Option<UserId>,
    ids: &[String],
) -> AppResult<std::collections::HashSet<String>> {
    let Some(me) = me else {
        return Ok(Default::default());
    };
    if ids.is_empty() {
        return Ok(Default::default());
    }
    let mut q = sqlx::QueryBuilder::new("SELECT message_id FROM poll_seen WHERE user_id = ");
    q.push_bind(me.to_string());
    q.push(" AND message_id IN (");
    let mut list = q.separated(", ");
    for id in ids {
        list.push_bind(id);
    }
    q.push(")");
    let rows: Vec<String> = q.build_query_scalar().fetch_all(&mut *db).await?;
    Ok(rows.into_iter().collect())
}

/// The poll as `me` may see it at `now` (`None`: as events show it, to no one in particular). `seen`: `me` has voted
/// in it at some point.
pub(crate) fn view(
    p: &Stored,
    votes: &[(u32, UserId)],
    me: Option<UserId>,
    seen: bool,
    now: &str,
) -> Poll {
    let closed = p.over(now);
    let mut mine: Vec<u32> = votes
        .iter()
        .filter(|(_, v)| Some(*v) == me)
        .map(|(o, _)| *o)
        .collect();
    mine.sort_unstable();
    let mut people: Vec<UserId> = votes.iter().map(|(_, v)| *v).collect();
    people.sort_unstable_by_key(|u| u.to_string());
    people.dedup();
    let creator = me.is_some() && me == p.author_id;
    let results = (closed || creator || seen || !mine.is_empty()).then(|| {
        p.options
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let here = votes.iter().filter(|(o, _)| *o as usize == i);
                PollTally {
                    votes: u32::try_from(here.clone().count()).unwrap_or(u32::MAX),
                    voters: if p.anonymous {
                        vec![]
                    } else {
                        here.map(|(_, v)| *v).collect()
                    },
                }
            })
            .collect()
    });
    Poll {
        question: p.question.clone(),
        options: p.options.clone(),
        multi: p.multi,
        anonymous: p.anonymous,
        ends_at: p.ends_at.clone(),
        closed_at: p.closed_at.clone(),
        closed,
        voters: u32::try_from(people.len()).unwrap_or(u32::MAX),
        mine,
        results,
        edited_at: p.edited_at.clone(),
    }
}

/// One poll as `me` sees it now, if the message is a live poll.
pub(crate) async fn view_for(
    db: &SqlitePool,
    me: Option<UserId>,
    id: MessageId,
) -> AppResult<Option<Poll>> {
    let mut conn = db.acquire().await?;
    let Some(p) = load(&mut conn, id).await? else {
        return Ok(None);
    };
    let votes = votes_for(&mut conn, &[id.to_string()])
        .await?
        .remove(&id.to_string())
        .unwrap_or_default();
    let seen = !seen_by(&mut conn, me, &[id.to_string()]).await?.is_empty();
    Ok(Some(view(&p, &votes, me, seen, &now())))
}

/// Set each live poll's `poll` as `me` sees it (and clear any other).
pub(crate) async fn attach(
    db: &SqlitePool,
    me: Option<UserId>,
    page: &mut [Message],
) -> AppResult<()> {
    let ids: Vec<String> = page
        .iter()
        .filter(|m| !m.deleted && m.kind == pulse_protocol::rest::MessageKind::Normal)
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
    let polls: Vec<Stored> = rows
        .into_iter()
        .map(stored_from)
        .collect::<AppResult<_>>()?;
    let poll_ids: Vec<String> = polls.iter().map(|p| p.message_id.to_string()).collect();
    let mut votes = votes_for(&mut conn, &poll_ids).await?;
    let seen = seen_by(&mut conn, me, &poll_ids).await?;
    let at = now();
    let mut by_id: HashMap<MessageId, Poll> = polls
        .iter()
        .map(|p| {
            let id = p.message_id.to_string();
            let v = votes.remove(&id).unwrap_or_default();
            (p.message_id, view(p, &v, me, seen.contains(&id), &at))
        })
        .collect();
    for m in page.iter_mut() {
        m.poll = by_id.remove(&m.id).map(Box::new);
    }
    Ok(())
}

/// A message is being deleted (in `tx`): its poll and votes go with it, so it is never closed or voted on again.
pub(crate) async fn on_delete(tx: &mut SqliteConnection, id: MessageId) -> AppResult<()> {
    sqlx::query("DELETE FROM polls WHERE message_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// The question, trimmed: 1-200 characters on one line.
pub fn clean_question(raw: &str) -> AppResult<String> {
    let q = raw.trim();
    let n = q.chars().count();
    if n == 0 || n > poll_limits::QUESTION_CHARS || q.chars().any(char::is_control) {
        return Err(AppError::BadRequest(format!(
            "a poll's question is 1-{} characters on one line",
            poll_limits::QUESTION_CHARS
        )));
    }
    Ok(q.to_string())
}

/// The options, each trimmed: 2-10 of them, 1-80 characters on one line each. The same option twice is allowed:
/// votes go by position, not text.
pub fn clean_options(raw: Vec<String>) -> AppResult<Vec<String>> {
    if !(poll_limits::MIN_OPTIONS..=poll_limits::MAX_OPTIONS).contains(&raw.len()) {
        return Err(AppError::BadRequest(format!(
            "a poll has {}-{} options",
            poll_limits::MIN_OPTIONS,
            poll_limits::MAX_OPTIONS
        )));
    }
    raw.into_iter()
        .map(|o| {
            let o = o.trim();
            let n = o.chars().count();
            if n == 0 || n > poll_limits::OPTION_CHARS || o.chars().any(char::is_control) {
                return Err(AppError::BadRequest(format!(
                    "options are 1-{} characters on one line",
                    poll_limits::OPTION_CHARS
                )));
            }
            Ok(o.to_string())
        })
        .collect()
}

/// A poll edit worked out (#265): the options after it and, for each option before it, where its votes go (`None`:
/// they are cleared, because the option was removed or reworded).
#[derive(Debug, PartialEq)]
pub struct EditPlan {
    pub options: Vec<String>,
    pub moves: Vec<Option<u32>>,
}

/// Work out an edit of `before` into `edits`, cleaned as a new poll's options are. Rules: a new option (no `from`) is
/// always fine; an option kept with the same text (after trimming) keeps its votes, wherever it now sits; an option
/// removed, or kept with new text, loses its votes (votes are for what the option said). Each old option is kept at
/// most once.
pub fn plan_edit(
    before: &[String],
    edits: &[pulse_protocol::rest::PollOptionEdit],
) -> AppResult<EditPlan> {
    let options = clean_options(edits.iter().map(|e| e.text.clone()).collect())?;
    let mut moves: Vec<Option<u32>> = vec![None; before.len()];
    let mut kept = vec![false; before.len()];
    for (new_at, (e, text)) in edits.iter().zip(&options).enumerate() {
        let Some(from) = e.from else { continue };
        let from = from as usize;
        if from >= before.len() {
            return Err(AppError::BadRequest("no such option".into()));
        }
        if std::mem::replace(&mut kept[from], true) {
            return Err(AppError::BadRequest(
                "an option is kept at most once".into(),
            ));
        }
        if before[from] == *text {
            moves[from] = Some(new_at as u32);
        }
    }
    Ok(EditPlan { options, moves })
}

/// The poll as text, for clients that don't know polls (and previews, search and notifications).
pub fn fallback_text(question: &str, options: &[String]) -> String {
    let mut t = format!("📊 Poll: {question}");
    for o in options {
        t.push_str("\n• ");
        t.push_str(o);
    }
    t.push_str("\n(Update Pulse to vote)");
    t
}

fn votes_phrase(n: u32) -> String {
    format!("{n} vote{}", if n == 1 { "" } else { "s" })
}

/// The ended line's text: "📊 alex's poll ended: Valheim won (4 votes)". Tied options with the same text are named
/// once ("Pizza tied (2 votes each)").
pub fn ended_text(who: &str, winners: &[String], votes: u32) -> String {
    match winners {
        [] => format!("📊 {who}'s poll ended with no votes"),
        [one] => format!("📊 {who}'s poll ended: {one} won ({})", votes_phrase(votes)),
        many => {
            let mut names: Vec<String> = Vec::with_capacity(many.len());
            for w in many {
                if !names.contains(w) {
                    names.push(w.clone());
                }
            }
            format!(
                "📊 {who}'s poll ended: {} tied ({} each)",
                system::join_names(&names),
                votes_phrase(votes)
            )
        }
    }
}

/// The option(s) with the most votes and that count; none when nobody voted.
pub fn winners(options: &[String], votes: &[(u32, UserId)]) -> (Vec<String>, u32) {
    let counts: Vec<u32> = (0..options.len())
        .map(|i| votes.iter().filter(|(o, _)| *o as usize == i).count() as u32)
        .collect();
    let top = counts.iter().copied().max().unwrap_or(0);
    if top == 0 {
        return (vec![], 0);
    }
    let won = options
        .iter()
        .zip(&counts)
        .filter(|(_, c)| **c == top)
        .map(|(o, _)| o.clone())
        .collect();
    (won, top)
}

/// Close the poll in `tx` at `now` and store its ended line, if it is still open (the sweep may have got there
/// first, or the creator). `who`: its creator's name as shown. The line, to announce after the commit.
pub(crate) async fn close_in(
    tx: &mut SqliteConnection,
    p: &Stored,
    who: &str,
    now: &str,
) -> AppResult<Option<Message>> {
    let changed =
        sqlx::query("UPDATE polls SET closed_at = ? WHERE message_id = ? AND closed_at IS NULL")
            .bind(now)
            .bind(p.message_id.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if changed == 0 {
        return Ok(None);
    }
    let votes = votes_for(&mut *tx, &[p.message_id.to_string()])
        .await?
        .remove(&p.message_id.to_string())
        .unwrap_or_default();
    let (won, top) = winners(&p.options, &votes);
    let ended = PollEnded {
        poll_message_id: p.message_id,
        author_id: p.author_id,
        winners: won,
        votes: top,
    };
    let content = ended_text(who, &ended.winners, ended.votes);
    let payload = system::poll_ended_payload(&ended)?;
    let line = system::insert(&mut *tx, p.channel_id, content, payload).await?;
    // the end of a withheld poll (#474) is withheld from the same person
    let from = crate::blocks::withheld_from(&mut *tx, p.message_id).await?;
    crate::blocks::withhold_line(&mut *tx, line.id, from).await?;
    Ok(Some(line))
}

/// The creator's name as the ended line shows it.
pub(crate) async fn creator_name(db: &SqlitePool, p: &Stored) -> AppResult<String> {
    match p.author_id {
        Some(a) => crate::dms::routes::shown_name(db, a).await,
        None => Ok("Someone".into()),
    }
}

/// Announce a closed poll: its line, then the change.
pub(crate) async fn announce_close(s: &AppState, p: &Stored, line: Message) {
    s.hub
        .publish(
            &s.db,
            Event::MessageCreated {
                message: line,
                nonce: None,
            },
        )
        .await;
    s.hub
        .publish(
            &s.db,
            Event::PollChanged {
                channel_id: p.channel_id,
                message_id: p.message_id,
            },
        )
        .await;
}

/// Close every open poll whose time is up at `now`, posting each one's line. Returns how many it closed. A poll
/// in a channel that is in Recently deleted (or whose server is) is left open (#334): its line would land in the
/// deleted channel; restored, the next sweep closes it. A poll in an archived channel waits the same way (#418): an
/// archived channel takes no new lines, automatic ones included, so its "ended" line posts once it's unarchived. Its
/// time is up all the same, so everyone already sees it as over, with its results.
pub async fn close_due(s: &AppState, now: &str) -> AppResult<usize> {
    let rows: Vec<StoredRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {STORED_COLS} WHERE p.closed_at IS NULL AND p.ends_at IS NOT NULL AND p.ends_at <= ? \
         AND m.deleted_at IS NULL AND p.channel_id IN (SELECT id FROM channels WHERE deleted_at IS NULL \
         AND archived_at IS NULL \
         AND (server_id IS NULL OR server_id NOT IN {}))",
        crate::access::DELETED_SERVERS
    )))
    .bind(now)
    .fetch_all(&s.db)
    .await?;
    let mut closed = 0;
    for r in rows {
        let p = stored_from(r)?;
        let who = creator_name(&s.db, &p).await?;
        // each line goes out in commit order with the chat's other messages (as a send)
        let _order = s.hub.in_order().await;
        let mut tx = begin_write(&s.db).await?;
        // deleted since the read above: left for after a restore (dropping `tx` ends it)
        if !crate::access::channel_is_live_in(&mut tx, p.channel_id).await? {
            continue;
        }
        // archived since the read above: left for after an unarchive, as above
        if crate::access::open_in(&mut tx, p.channel_id).await.is_err() {
            continue;
        }
        let line = close_in(&mut tx, &p, &who, now).await?;
        tx.commit().await?;
        if let Some(line) = line {
            closed += 1;
            announce_close(s, &p, line).await;
        }
    }
    Ok(closed)
}

/// Close due polls every [`SWEEP_EVERY`] (the first tick runs at once: the boot sweep). Call after binding the
/// listener.
pub fn spawn_sweeper(s: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            match close_due(&s, &now()).await {
                Ok(0) => {}
                Ok(n) => tracing::debug!(n, "polls closed"),
                Err(e) => tracing::warn!(error = %e, "poll sweep failed"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn questions_and_options_are_cleaned() {
        assert_eq!(clean_question("  Where? ").unwrap(), "Where?");
        assert!(clean_question(" ").is_err());
        assert!(clean_question(&"q".repeat(201)).is_err());
        assert!(clean_question(&"é".repeat(200)).is_ok());
        assert!(clean_question("a\nb").is_err());
        assert_eq!(clean_options(s(&[" a ", "b"])).unwrap(), s(&["a", "b"]));
        assert!(clean_options(s(&["a"])).is_err());
        // the same option twice is allowed (votes go by position, not text)
        assert_eq!(clean_options(s(&["a", "A "])).unwrap(), s(&["a", "A"]));
        assert_eq!(
            clean_options(s(&["Pizza", " Pizza"])).unwrap(),
            s(&["Pizza", "Pizza"])
        );
        assert!(clean_options(s(&["a", ""])).is_err());
        assert!(clean_options(vec!["o".repeat(81), "b".into()]).is_err());
        let ten: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        assert!(clean_options(ten.clone()).is_ok());
        assert!(clean_options([ten, s(&["x"])].concat()).is_err());
    }

    #[test]
    fn an_edit_keeps_votes_only_on_options_that_still_say_the_same() {
        use pulse_protocol::rest::PollOptionEdit as E;
        let e = |text: &str, from: Option<u32>| E {
            text: text.into(),
            from,
        };
        let before = s(&["Pizza", "Tacos", "Sushi"]);
        // reorder, reword one, drop one, add one
        let p = plan_edit(
            &before,
            &[
                e("Sushi", Some(2)),
                e("Tacos!", Some(1)),
                e(" Curry ", None),
            ],
        )
        .unwrap();
        assert_eq!(p.options, s(&["Sushi", "Tacos!", "Curry"]));
        assert_eq!(p.moves, vec![None, None, Some(0)]);
        // the same text after trimming keeps its votes
        let p = plan_edit(&before, &[e(" Pizza ", Some(0)), e("Tacos", Some(1))]).unwrap();
        assert_eq!(p.moves, vec![Some(0), Some(1), None]);
        // an option kept twice, or one that isn't there, is refused; so are the usual limits
        assert!(plan_edit(&before, &[e("a", Some(0)), e("b", Some(0))]).is_err());
        assert!(plan_edit(&before, &[e("a", Some(0)), e("b", Some(9))]).is_err());
        assert!(plan_edit(&before, &[e("a", Some(0))]).is_err());
    }

    #[test]
    fn the_winners_and_the_line() {
        let u = UserId::new();
        let opts = s(&["Valheim", "Apex", "Rust"]);
        assert_eq!(winners(&opts, &[]), (vec![], 0));
        assert_eq!(winners(&opts, &[(1, u), (1, u), (0, u)]), (s(&["Apex"]), 2));
        assert_eq!(winners(&opts, &[(1, u), (2, u)]), (s(&["Apex", "Rust"]), 1));
        assert_eq!(
            ended_text("alex", &s(&["Valheim"]), 4),
            "📊 alex's poll ended: Valheim won (4 votes)"
        );
        assert_eq!(
            ended_text("alex", &s(&["a", "b", "c"]), 1),
            "📊 alex's poll ended: a, b and c tied (1 vote each)"
        );
        // options with the same text that tie are named once
        let pizzas = s(&["Pizza", "Pizza", "Tacos"]);
        assert_eq!(
            winners(&pizzas, &[(0, u), (1, u)]),
            (s(&["Pizza", "Pizza"]), 1)
        );
        assert_eq!(
            ended_text("alex", &s(&["Pizza", "Pizza"]), 2),
            "📊 alex's poll ended: Pizza tied (2 votes each)"
        );
        assert_eq!(
            ended_text("alex", &s(&["Pizza", "Tacos", "Pizza"]), 1),
            "📊 alex's poll ended: Pizza and Tacos tied (1 vote each)"
        );
    }

    #[test]
    fn a_view_shows_results_only_to_those_who_may_see_them() {
        let (alex, sam, kim) = (UserId::new(), UserId::new(), UserId::new());
        let p = Stored {
            message_id: MessageId::new(),
            channel_id: ChannelId::new(),
            author_id: Some(alex),
            question: "Q".into(),
            options: s(&["a", "b"]),
            multi: false,
            anonymous: false,
            ends_at: Some("2100-01-01T00:00:00.000Z".into()),
            closed_at: None,
            edited_at: None,
        };
        let votes = vec![(1, sam)];
        let now = "2026-01-01T00:00:00.000Z";
        assert_eq!(view(&p, &votes, Some(kim), false, now).results, None);
        assert_eq!(view(&p, &votes, None, false, now).results, None);
        assert_eq!(view(&p, &votes, Some(kim), false, now).voters, 1);
        // kim voted before and took it back: still sees what she saw
        assert!(view(&p, &votes, Some(kim), true, now).results.is_some());
        assert_eq!(
            view(&p, &votes, Some(alex), false, now).results.unwrap()[1].voters,
            vec![sam]
        );
        assert_eq!(view(&p, &votes, Some(sam), false, now).mine, vec![1]);
        // over by time: everyone (and the events) see it, closed
        let late = "2100-01-02T00:00:00.000Z";
        let v = view(&p, &votes, None, false, late);
        assert!(v.closed && v.results.is_some());
        let anon = Stored {
            anonymous: true,
            ..p
        };
        let v = view(&anon, &votes, Some(alex), false, now);
        assert_eq!(v.results.unwrap()[1].votes, 1);
    }
}
