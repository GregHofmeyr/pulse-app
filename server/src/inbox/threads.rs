//! The inbox's Threads (#326): reply threads you're part of that have replies you haven't seen. A "thread" is a
//! quote's or clip's replies (`thread_root_id`; plain replies in a chat already reach you as inbox items when they
//! answer you). You're part of one when you posted the quote or clip, or replied in it. New = replies by others after
//! your seen point in it, or after your own last reply there, whichever is later (you saw the thread when you replied),
//! within the inbox's [`DAYS`](super::DAYS). As the inbox's items: never in a deleted chat or server, never a deleted
//! root, and none in a muted chat or server. Derived on read, so nothing is written when a reply lands; the client
//! asks again when a reply lands in a thread it's `watching`, or your seen point moves.

use std::collections::HashMap;

use chrono::Utc;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{InboxThread, InboxThreads};
use sqlx::SqlitePool;

use super::{cutoff, snippets};
use crate::db::now;
use crate::error::AppResult;
use crate::messages::routes::{COLS, Row, fill};

/// The most threads listed, and watched.
pub const MAX_THREADS: i64 = 50;
pub const MAX_WATCHING: i64 = 500;

/// The live quotes and clips `me` is part of, in chats they can see and haven't muted. Binds: me, me, now.
const WATCHED: &str = "SELECT r.id AS root_id, r.channel_id AS channel_id, COALESCE(r.author_id = ?1, 0) AS started
    FROM messages r JOIN channels c ON c.id = r.channel_id
    WHERE r.deleted_at IS NULL
      AND (EXISTS (SELECT 1 FROM quotes q WHERE q.message_id = r.id)
           OR EXISTS (SELECT 1 FROM clips k WHERE k.message_id = r.id))
      AND (r.author_id = ?1 OR r.id IN (SELECT thread_root_id FROM messages
                                        WHERE author_id = ?1 AND thread_root_id IS NOT NULL AND deleted_at IS NULL))
      AND c.deleted_at IS NULL
      AND (c.server_id IS NULL OR c.server_id NOT IN (SELECT id FROM servers WHERE deleted_at IS NOT NULL))
      AND (c.server_id IS NOT NULL OR EXISTS (SELECT 1 FROM channel_members cm
                                               WHERE cm.channel_id = c.id AND cm.user_id = ?1))
      AND NOT EXISTS (
            SELECT 1 FROM notification_prefs np
            WHERE np.user_id = ?1 AND np.muted = 1
              AND (np.muted_until IS NULL OR np.muted_until > ?2)
              AND (np.target_kind = 'channel' AND np.target_id = c.id
                   OR np.target_kind = 'server' AND np.target_id = c.server_id))";

/// `me`'s threads with new replies, newest reply first, and the threads they're watching.
pub async fn list(db: &SqlitePool, me: UserId) -> AppResult<InboxThreads> {
    let watched: Vec<(String, String, bool)> = sqlx::query_as(&format!(
        "SELECT root_id, channel_id, started FROM ({WATCHED}) ORDER BY root_id DESC LIMIT ?3"
    ))
    .bind(me.to_string())
    .bind(now())
    .bind(MAX_WATCHING)
    .fetch_all(db)
    .await?;
    let watching: Vec<MessageId> = watched
        .iter()
        .map(|(r, _, _)| r.parse())
        .collect::<Result<_, _>>()
        .map_err(anyhow::Error::from)?;
    if watched.is_empty() {
        return Ok(InboxThreads {
            threads: vec![],
            watching,
        });
    }
    // per watched root: the replies by others past the later of your seen point and your own last reply
    let mut q = sqlx::QueryBuilder::new(
        "SELECT x.thread_root_id, COUNT(*), MAX(x.id) FROM messages x
         LEFT JOIN thread_reads tr ON tr.user_id = ",
    );
    q.push_bind(me.to_string());
    q.push(" AND tr.root_id = x.thread_root_id WHERE x.deleted_at IS NULL AND x.id >= ");
    q.push_bind(cutoff(Utc::now()).to_string());
    q.push(" AND COALESCE(x.author_id, '') != ");
    q.push_bind(me.to_string());
    q.push(" AND x.id > COALESCE(tr.last_seen_id, '') AND x.id > COALESCE((SELECT MAX(y.id) FROM messages y WHERE y.thread_root_id = x.thread_root_id AND y.deleted_at IS NULL AND y.author_id = ");
    q.push_bind(me.to_string());
    q.push("), '') AND x.thread_root_id IN (");
    let mut ids = q.separated(", ");
    for (r, _, _) in &watched {
        ids.push_bind(r.clone());
    }
    q.push(") GROUP BY x.thread_root_id ORDER BY MAX(x.id) DESC LIMIT ");
    q.push_bind(MAX_THREADS);
    let fresh: Vec<(String, i64, String)> = q.build_query_as().fetch_all(db).await?;
    if fresh.is_empty() {
        return Ok(InboxThreads {
            threads: vec![],
            watching,
        });
    }
    let info: HashMap<&str, (&str, bool)> = watched
        .iter()
        .map(|(r, c, s)| (r.as_str(), (c.as_str(), *s)))
        .collect();
    let mut q = sqlx::QueryBuilder::new(format!("SELECT {COLS} FROM messages WHERE id IN ("));
    let mut ids = q.separated(", ");
    for (_, _, latest) in &fresh {
        ids.push_bind(latest.clone());
    }
    q.push(")");
    let rows: Vec<Row> = q.build_query_as().fetch_all(db).await?;
    let mut latest: HashMap<MessageId, _> = fill(db, Some(me), rows)
        .await?
        .into_iter()
        .map(|m| (m.id, m))
        .collect();
    let roots: Vec<MessageId> = fresh
        .iter()
        .map(|(r, _, _)| r.parse())
        .collect::<Result<_, _>>()
        .map_err(anyhow::Error::from)?;
    let mut root_snips = snippets(db, &roots).await?;
    let mut threads = Vec::with_capacity(fresh.len());
    for ((root, n, last), root_id) in fresh.iter().zip(roots) {
        let last: MessageId = last.parse().map_err(anyhow::Error::from)?;
        let (Some(message), Some(snip), Some((channel, started))) = (
            latest.remove(&last),
            root_snips.remove(&root_id),
            info.get(root.as_str()),
        ) else {
            continue;
        };
        let channel_id: ChannelId = channel.parse().map_err(anyhow::Error::from)?;
        threads.push(InboxThread {
            root: snip,
            channel_id,
            latest: message,
            new_replies: *n as u32,
            started: *started,
        });
    }
    Ok(InboxThreads { threads, watching })
}
