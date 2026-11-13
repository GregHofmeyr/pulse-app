//! The inbox (#306): everything meant for you, across every server, DM and group. Its items are your
//! `mentions` rows (a reply already pings whoever it answers; `explicit` tells a typed `@you` from that
//! ping), so nothing is written when an item arrives. An item is read when your read point (or a reply's
//! thread seen point) covers it, when you opened it from the inbox (`inbox_reads`), or when it is at or
//! before your Mark all read (`inbox_cleared`). Items older than [`DAYS`] are gone.
//!
//! Events: [`follow_ups`] turns the events the app already publishes (a new message, a read point or seen
//! point moving, a mute, a delete) into the inbox work they imply; the hub's inbox worker ([`run`]) does it
//! after the publish (#327), sending each user's events to their own sessions only, under their inbox slot.

pub mod routes;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{INBOX_SNIPPET_CHARS, InboxItem, InboxKind, InboxSnippet, Message};
use sqlx::{SqliteExecutor, SqlitePool};

use crate::db::now;
use crate::error::AppResult;
use crate::gateway::hub::{ConnId, Hub};
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
pub async fn unread<'e>(db: impl SqliteExecutor<'e>, me: UserId) -> AppResult<u32> {
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

/// Inbox work an event implies (#327), done by the hub's inbox worker ([`run`]) after the publish, outside its
/// `in_order`.
pub enum Job {
    /// A new message that mentions them: its item, if it is one of theirs, and the count.
    Added { user: UserId, message: Arc<Message> },
    /// Messages that mentioned them were deleted: the items to take out, and the count.
    Removed { user: UserId, ids: Vec<MessageId> },
    /// A read point, seen point or mute moved: the count, if it changed from what they were told.
    Recount { user: UserId },
    /// Their count changed with no inbox event of its own (a conversation left): the count, whatever it is.
    Retell { user: UserId },
    /// A server channel was deleted or restored: [`Job::Retell`] whoever is connected and has items in it.
    Channel { channel: ChannelId },
    /// Likewise for every channel of a server.
    Server { server: ServerId },
}

/// The inbox work `events` (published together) imply, for users who are connected (a later Ready has it all):
/// one [`Job::Removed`] per person for all the batch's deletes.
pub fn follow_ups<'a>(hub: &Hub, events: impl Iterator<Item = &'a Event>) -> Vec<Job> {
    let mut out: Vec<Job> = Vec::new();
    let mut online: Option<HashSet<UserId>> = None;
    for event in events {
        if !matches!(
            event,
            Event::MessageCreated { .. }
                | Event::MessageDeleted { .. }
                | Event::ReadStateUpdated { .. }
                | Event::ThreadSeen { .. }
                | Event::MutesChanged { .. }
                | Event::ChannelRemoved { .. }
                | Event::ChannelDeleted { .. }
                | Event::ChannelRestored { .. }
                | Event::ServerDeleted { .. }
                | Event::ServerRestored { .. }
        ) {
            continue;
        }
        // read once per batch, and only for an event that may need it
        let online = online.get_or_insert_with(|| hub.online_users());
        match event {
            Event::MessageCreated { message, .. }
                if !message.deleted && message.mentions.iter().any(|u| online.contains(u)) =>
            {
                let message = Arc::new(message.clone());
                for user in message.mentions.iter().filter(|u| online.contains(u)) {
                    out.push(Job::Added {
                        user: *user,
                        message: message.clone(),
                    });
                }
            }
            Event::MessageDeleted {
                message_id,
                mentions,
                ..
            } => {
                for &user in mentions.iter().filter(|u| online.contains(u)) {
                    let theirs = out.iter_mut().find_map(|j| match j {
                        Job::Removed { user: u, ids } if *u == user => Some(ids),
                        _ => None,
                    });
                    match theirs {
                        Some(ids) => ids.push(*message_id),
                        None => out.push(Job::Removed {
                            user,
                            ids: vec![*message_id],
                        }),
                    }
                }
            }
            Event::ReadStateUpdated { user_id, .. }
            | Event::ThreadSeen { user_id, .. }
            | Event::MutesChanged { user_id, .. } => {
                if online.contains(user_id) {
                    out.push(Job::Recount { user: *user_id });
                }
            }
            Event::ChannelRemoved { user_id, .. } => {
                if online.contains(user_id) {
                    out.push(Job::Retell { user: *user_id });
                }
            }
            Event::ChannelDeleted { channel_id, .. } => out.push(Job::Channel {
                channel: *channel_id,
            }),
            Event::ChannelRestored { channel } => out.push(Job::Channel {
                channel: channel.id,
            }),
            Event::ServerDeleted { server_id } => out.push(Job::Server { server: *server_id }),
            Event::ServerRestored { snapshot } => out.push(Job::Server {
                server: snapshot.server.id,
            }),
            _ => {}
        }
    }
    out
}

/// Do `jobs` (one or more batches, in publish order), each person's under their
/// [`InboxSlot`](crate::gateway::hub::InboxSlot): a count is read and sent while it is held. Items come first, in
/// order; then each person whose count may have moved without an item is counted once, however many read points,
/// mutes or deleted channels the jobs held for them (a count read later is only fresher). A failed read is logged
/// and that event skipped: the trigger has landed, and the user's next Ready (or fetch) has the count.
pub async fn run(db: &SqlitePool, hub: &Hub, jobs: Vec<Job>) {
    // a reply's snippet, read once however many people the reply pings
    let mut replied: HashMap<MessageId, Option<InboxSnippet>> = HashMap::new();
    // counted at the end, once each, in the order first asked: told whatever it is, or only when it changed
    let mut retell: Vec<UserId> = Vec::new();
    let mut recount: Vec<UserId> = Vec::new();
    let mut gone_channels: Vec<ChannelId> = Vec::new();
    let mut gone_servers: Vec<ServerId> = Vec::new();
    for job in jobs {
        match job {
            Job::Added { user, message } => added(db, hub, user, &message, &mut replied).await,
            Job::Removed { user, ids } => {
                tell(db, hub, user, true, |n| updated(user, n, vec![], None, ids)).await;
            }
            Job::Recount { user } => push_new(&mut recount, user),
            Job::Retell { user } => push_new(&mut retell, user),
            Job::Channel { channel } => push_new(&mut gone_channels, channel),
            Job::Server { server } => push_new(&mut gone_servers, server),
        }
    }
    match with_items_in(db, &gone_channels, &gone_servers).await {
        Ok(users) => {
            let online = hub.online_users();
            for user in users.into_iter().filter(|u| online.contains(u)) {
                push_new(&mut retell, user);
            }
        }
        Err(e) => tracing::error!(error = ?e, "couldn't read who had inbox items in deleted chats"),
    }
    for &user in &retell {
        tell(db, hub, user, true, |n| {
            updated(user, n, vec![], None, vec![])
        })
        .await;
    }
    for user in recount.into_iter().filter(|u| !retell.contains(u)) {
        tell(db, hub, user, false, |n| {
            updated(user, n, vec![], None, vec![])
        })
        .await;
    }
}

fn push_new<T: PartialEq>(v: &mut Vec<T>, x: T) {
    if !v.contains(&x) {
        v.push(x);
    }
}

/// [`Job::Added`]: `message`'s item for `user`, if it is one of theirs, with their count.
async fn added(
    db: &SqlitePool,
    hub: &Hub,
    user: UserId,
    message: &Message,
    replied: &mut HashMap<MessageId, Option<InboxSnippet>>,
) {
    let Some(slot) = hub.inbox_slot(user) else {
        return;
    };
    let mut told = slot.lock().await;
    let read = async {
        // Whether it's read and the count, from one snapshot, so they agree: an item read is never counted
        // unread, nor one counted read sent unread. (A read point may still reach the device first; the
        // count that follows it, and the client's own read point, settle the item.)
        let mut snap = db.begin().await?;
        let Some((explicit, read)) = visible(&mut *snap, user, message.id).await? else {
            return Ok(None);
        };
        let unread = unread(&mut *snap, user).await?;
        snap.rollback().await?;
        let replied_to = match message.reply_to_id {
            Some(to) => match replied.get(&to) {
                Some(s) => s.clone(),
                None => {
                    let s = snippets(db, &[to]).await?.remove(&to);
                    replied.insert(to, s.clone());
                    s
                }
            },
            None => None,
        };
        let item = InboxItem {
            kind: if explicit {
                InboxKind::Mention
            } else {
                InboxKind::Reply
            },
            replied_to,
            message: message.clone(),
            read,
        };
        AppResult::Ok(Some((item, unread)))
    };
    match read.await {
        Ok(Some((item, unread))) => {
            *told = Some(unread);
            hub.send_inbox(
                &slot,
                Event::InboxItemAdded {
                    user_id: user,
                    item: Box::new(item),
                    unread,
                },
            );
        }
        Ok(None) => {}
        Err(e) => tracing::error!(error = ?e, %user, "couldn't read an inbox item"),
    }
}

/// Who has (or, deleted, had) inbox items in these server channels or servers' channels: mentions within
/// [`DAYS`].
async fn with_items_in(
    db: &SqlitePool,
    channels: &[ChannelId],
    servers: &[ServerId],
) -> AppResult<Vec<UserId>> {
    if channels.is_empty() && servers.is_empty() {
        return Ok(vec![]);
    }
    let mut q = sqlx::QueryBuilder::new(
        "SELECT DISTINCT mn.user_id FROM mentions mn JOIN messages m ON m.id = mn.message_id
         JOIN channels c ON c.id = m.channel_id WHERE mn.message_id >= ",
    );
    q.push_bind(cutoff(Utc::now()).to_string());
    q.push(" AND (c.id IN (");
    let mut list = q.separated(", ");
    list.push_bind("");
    for c in channels {
        list.push_bind(c.to_string());
    }
    q.push(") OR c.server_id IN (");
    let mut list = q.separated(", ");
    list.push_bind("");
    for s in servers {
        list.push_bind(s.to_string());
    }
    q.push("))");
    let rows: Vec<String> = q.build_query_scalar().fetch_all(db).await?;
    rows.into_iter()
        .map(|u| Ok(u.parse().map_err(anyhow::Error::from)?))
        .collect()
}

/// Whether `message` is one of `me`'s visible items: (a typed mention, read).
async fn visible<'e>(
    db: impl SqliteExecutor<'e>,
    me: UserId,
    message: MessageId,
) -> AppResult<Option<(bool, bool)>> {
    Ok(sqlx::query_as::<_, (bool, bool)>(&format!(
        "SELECT mn.explicit, {READ} {ITEMS} AND mn.message_id = ?"
    ))
    .bind(me.to_string())
    .bind(cutoff(Utc::now()).to_string())
    .bind(now())
    .bind(message.to_string())
    .fetch_optional(db)
    .await?)
}

/// Read `user`'s count under their inbox slot and send `make(count)` to their devices: always when `always`
/// (the event carries more than the count), otherwise only when it differs from what they were last told.
/// Nothing when they aren't connected.
pub(crate) async fn tell(
    db: &SqlitePool,
    hub: &Hub,
    user: UserId,
    always: bool,
    make: impl FnOnce(u32) -> Event,
) {
    let Some(slot) = hub.inbox_slot(user) else {
        return;
    };
    let mut told = slot.lock().await;
    let Some(n) = count(db, user).await else {
        return;
    };
    if always || *told != Some(n) {
        *told = Some(n);
        hub.send_inbox(&slot, make(n));
    }
}

/// Ready's inbox count for `me`. For a connection (`conn`) it is read under their inbox slot and counts as told;
/// their other devices hear it if it differs from what they were told, and this connection gets inbox events
/// from now on (none read before this count, which supersedes them).
pub async fn ready(db: &SqlitePool, hub: &Hub, me: UserId, conn: Option<ConnId>) -> AppResult<u32> {
    let Some((conn, slot)) = conn.and_then(|c| Some((c, hub.inbox_slot(me)?))) else {
        return unread(db, me).await;
    };
    let mut told = slot.lock().await;
    let n = unread(db, me).await?;
    if *told != Some(n) {
        *told = Some(n);
        // before this connection is ready: it has the count in its Ready
        hub.send_inbox(&slot, updated(me, n, vec![], None, vec![]));
    }
    hub.inbox_ready(conn);
    Ok(n)
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
