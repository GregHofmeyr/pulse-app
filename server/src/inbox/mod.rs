//! The inbox (#306): everything meant for you, across every server, DM and group. Its items are your
//! `mentions` rows (a reply already pings whoever it answers; `explicit` tells a typed `@you` from that
//! ping), so nothing is written when an item arrives. An item is read when your read point (or a reply's
//! thread seen point) covers it, when you opened it from the inbox (`inbox_reads`), or when it is at or
//! before your Mark all read (`inbox_cleared`). Items older than [`DAYS`] are gone.
//!
//! Events: [`follow_ups`] turns the events the app already publishes (a new message, a read point or seen
//! point moving, a mute, a delete) into the inbox events they imply; `Hub::publish` delivers them right
//! after their trigger, to the user's own sessions only.

pub mod routes;

use std::collections::HashMap;

use chrono::{DateTime, TimeDelta, Utc};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{MessageId, UserId};
use pulse_protocol::rest::{INBOX_SNIPPET_CHARS, InboxItem, InboxKind, InboxSnippet};
use sqlx::SqlitePool;

use crate::db::now;
use crate::error::AppResult;
use crate::gateway::hub::Hub;
use crate::messages::routes::{COLS, Row, fill};

pub use routes::router;

/// How long the inbox keeps an item.
pub const DAYS: i64 = 30;

/// The smallest message id still in the inbox at `now`.
pub fn cutoff(now: DateTime<Utc>) -> MessageId {
    MessageId::floor((now - TimeDelta::days(DAYS)).timestamp_millis().max(0) as u64)
}

/// The user's visible items, the way the mention badges count them: a chat they have a read point in, not
/// in Recently deleted (nor its server), a DM or group they're still in, the message (and a reply's quote)
/// not deleted, not their own, within [`DAYS`]. In a muted chat (or server) only typed mentions count.
/// Binds: user, cutoff, now (for timed mutes).
const ITEMS: &str = "FROM mentions mn
    JOIN messages m ON m.id = mn.message_id
    JOIN channels c ON c.id = m.channel_id
    JOIN read_states rs ON rs.user_id = mn.user_id AND rs.channel_id = m.channel_id
    LEFT JOIN thread_reads tr ON tr.user_id = mn.user_id AND tr.root_id = m.thread_root_id
    LEFT JOIN messages root ON root.id = m.thread_root_id
    LEFT JOIN inbox_cleared ic ON ic.user_id = mn.user_id
    WHERE mn.user_id = ? AND mn.message_id >= ?
      AND m.deleted_at IS NULL AND COALESCE(m.author_id, '') != mn.user_id
      AND (m.thread_root_id IS NULL OR root.deleted_at IS NULL)
      AND c.deleted_at IS NULL
      AND (c.server_id IS NULL OR c.server_id NOT IN (SELECT id FROM servers WHERE deleted_at IS NOT NULL))
      AND (c.server_id IS NOT NULL OR EXISTS (SELECT 1 FROM channel_members cm
                                               WHERE cm.channel_id = c.id AND cm.user_id = mn.user_id))
      AND (mn.explicit = 1 OR NOT EXISTS (
            SELECT 1 FROM notification_prefs np
            WHERE np.user_id = mn.user_id AND np.muted = 1
              AND (np.muted_until IS NULL OR np.muted_until > ?)
              AND (np.target_kind = 'channel' AND np.target_id = c.id
                   OR np.target_kind = 'server' AND np.target_id = c.server_id)))";

/// Whether an item of [`ITEMS`] is read.
const READ: &str = "(m.id <= COALESCE(ic.upto, '')
    OR EXISTS (SELECT 1 FROM inbox_reads ir WHERE ir.user_id = mn.user_id AND ir.message_id = m.id)
    OR CASE WHEN m.thread_root_id IS NULL THEN m.id <= COALESCE(rs.last_read_message_id, '')
            ELSE m.id <= COALESCE(tr.last_seen_id, '') END)";

/// How many of `me`'s items are unread.
pub async fn unread(db: &SqlitePool, me: UserId) -> AppResult<u32> {
    let n: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) {ITEMS} AND NOT {READ}"))
        .bind(me.to_string())
        .bind(cutoff(Utc::now()).to_string())
        .bind(now())
        .fetch_one(db)
        .await?;
    Ok(n as u32)
}

/// What to list: a page older than `before` (newest first), only typed mentions, or exactly one message.
#[derive(Default)]
pub struct Filter {
    pub before: Option<MessageId>,
    pub mentions_only: bool,
    pub only: Option<MessageId>,
    pub limit: i64,
}

/// `me`'s items as `filter` picks them, newest first.
pub async fn list(db: &SqlitePool, me: UserId, filter: Filter) -> AppResult<Vec<InboxItem>> {
    let mut sql = format!("SELECT m.id, mn.explicit, {READ} {ITEMS}");
    if filter.before.is_some() {
        sql.push_str(" AND mn.message_id < ?");
    }
    if filter.mentions_only {
        sql.push_str(" AND mn.explicit = 1");
    }
    if filter.only.is_some() {
        sql.push_str(" AND mn.message_id = ?");
    }
    sql.push_str(" ORDER BY mn.message_id DESC LIMIT ?");
    let mut q = sqlx::query_as::<_, (String, bool, bool)>(&sql)
        .bind(me.to_string())
        .bind(cutoff(Utc::now()).to_string())
        .bind(now());
    if let Some(b) = filter.before {
        q = q.bind(b.to_string());
    }
    if let Some(o) = filter.only {
        q = q.bind(o.to_string());
    }
    let picked = q.bind(filter.limit).fetch_all(db).await?;
    if picked.is_empty() {
        return Ok(vec![]);
    }
    let mut q = sqlx::QueryBuilder::new(format!("SELECT {COLS} FROM messages WHERE id IN ("));
    let mut ids = q.separated(", ");
    for (id, _, _) in &picked {
        ids.push_bind(id.clone());
    }
    q.push(")");
    let rows: Vec<Row> = q.build_query_as().fetch_all(db).await?;
    let mut messages: HashMap<MessageId, _> = fill(db, Some(me), rows)
        .await?
        .into_iter()
        .map(|m| (m.id, m))
        .collect();
    let targets: Vec<MessageId> = messages.values().filter_map(|m| m.reply_to_id).collect();
    let mut snippets = snippets(db, &targets).await?;
    let mut out = Vec::with_capacity(picked.len());
    for (id, explicit, read) in picked {
        let id: MessageId = id.parse().map_err(anyhow::Error::from)?;
        let Some(message) = messages.remove(&id) else {
            continue;
        };
        out.push(InboxItem {
            kind: if explicit {
                InboxKind::Mention
            } else {
                InboxKind::Reply
            },
            replied_to: message.reply_to_id.and_then(|r| snippets.remove(&r)),
            message,
            read,
        });
    }
    Ok(out)
}

/// `me`'s item for `message`, if it is one of their visible items.
pub async fn item(db: &SqlitePool, me: UserId, message: MessageId) -> AppResult<Option<InboxItem>> {
    Ok(list(
        db,
        me,
        Filter {
            only: Some(message),
            limit: 1,
            ..Filter::default()
        },
    )
    .await?
    .pop())
}

/// The replied-to messages, one short line each.
async fn snippets(
    db: &SqlitePool,
    ids: &[MessageId],
) -> AppResult<HashMap<MessageId, InboxSnippet>> {
    let mut out = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let mut q = sqlx::QueryBuilder::new(
        "SELECT id, author_id, content, deleted_at IS NOT NULL FROM messages WHERE id IN (",
    );
    let mut list = q.separated(", ");
    for id in ids {
        list.push_bind(id.to_string());
    }
    q.push(")");
    let rows: Vec<(String, Option<String>, String, bool)> =
        q.build_query_as().fetch_all(db).await?;
    for (id, author, content, deleted) in rows {
        let id: MessageId = id.parse().map_err(anyhow::Error::from)?;
        out.insert(
            id,
            InboxSnippet {
                message_id: id,
                author_id: author
                    .map(|a| a.parse())
                    .transpose()
                    .map_err(anyhow::Error::from)?,
                content: if deleted {
                    String::new()
                } else {
                    content.chars().take(INBOX_SNIPPET_CHARS).collect()
                },
                deleted,
            },
        );
    }
    Ok(out)
}

/// The inbox events `event` implies, each for one user's own sessions, and only for users who are connected
/// (a later Ready has it all). A read point, seen point or mute tells them only when their unread count
/// changed. A failed read is logged and that user's event skipped: the trigger has landed, and their next
/// Ready (or fetch) has the count.
pub async fn follow_ups(db: &SqlitePool, hub: &Hub, event: &Event) -> Vec<Event> {
    let mut out = Vec::new();
    match event {
        Event::MessageCreated { message, .. } if !message.deleted => {
            for &user in message.mentions.iter().filter(|u| hub.is_online(**u)) {
                let added = async {
                    let Some(item) = item(db, user, message.id).await? else {
                        return Ok(None);
                    };
                    AppResult::Ok(Some((item, unread(db, user).await?)))
                };
                match added.await {
                    Ok(Some((item, unread))) => {
                        hub.tell_inbox(user, unread);
                        out.push(Event::InboxItemAdded {
                            user_id: user,
                            item: Box::new(item),
                            unread,
                        });
                    }
                    Ok(None) => {}
                    Err(e) => tracing::error!(error = ?e, %user, "couldn't read an inbox item"),
                }
            }
        }
        Event::MessageDeleted {
            message_id,
            mentions,
            ..
        } => {
            for &user in mentions.iter().filter(|u| hub.is_online(**u)) {
                if let Some(unread) = count(db, user).await {
                    hub.tell_inbox(user, unread);
                    out.push(updated(user, unread, vec![], None, vec![*message_id]));
                }
            }
        }
        Event::ReadStateUpdated { user_id, .. }
        | Event::ThreadSeen { user_id, .. }
        | Event::MutesChanged { user_id, .. } => {
            if hub.is_online(*user_id)
                && let Some(unread) = count(db, *user_id).await
                && hub.inbox_changed(*user_id, unread)
            {
                out.push(updated(*user_id, unread, vec![], None, vec![]));
            }
        }
        _ => {}
    }
    out
}

async fn count(db: &SqlitePool, user: UserId) -> Option<u32> {
    match unread(db, user).await {
        Ok(n) => Some(n),
        Err(e) => {
            tracing::error!(error = ?e, %user, "couldn't count the inbox; event skipped");
            None
        }
    }
}

pub(crate) fn updated(
    user_id: UserId,
    unread: u32,
    read: Vec<MessageId>,
    read_upto: Option<MessageId>,
    removed: Vec<MessageId>,
) -> Event {
    Event::InboxUpdated {
        user_id,
        unread,
        read,
        read_upto,
        removed,
    }
}
