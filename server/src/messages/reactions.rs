//! Reactions on messages. Every add/remove goes through [`toggle`], which applies the channel's
//! reaction mode, so the quotes push can extend one function.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::put;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{Channel, ChannelKind, ReactionSummary};
use sqlx::{SqliteConnection, SqlitePool};
use unicode_segmentation::UnicodeSegmentation;

use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::servers::routes::is_server_member;

/// Distinct emoji on one message.
pub const MAX_DISTINCT: i64 = 20;
const MAX_REACTORS: i64 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReactionMode {
    Normal,
    /// Adding a reaction replaces the reactor's others on that message.
    OnePerPerson,
}

impl ReactionMode {
    fn parse(s: &str) -> Self {
        match s {
            "one_per_person" => Self::OnePerPerson,
            _ => Self::Normal,
        }
    }
}

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/messages/{id}/reactions/{emoji}",
        put(add).delete(remove).get(reactors),
    )
}

async fn add(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path((id, emoji)): Path<(MessageId, String)>,
) -> AppResult<StatusCode> {
    toggle(&s, me, id, &emoji, true).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path((id, emoji)): Path<(MessageId, String)>,
) -> AppResult<StatusCode> {
    toggle(&s, me, id, &emoji, false).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Who reacted with `emoji`, oldest first.
async fn reactors(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path((id, emoji)): Path<(MessageId, String)>,
) -> AppResult<Json<Vec<UserId>>> {
    visible_message(&s.db, me, id).await?;
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT user_id FROM reactions WHERE message_id = ? AND emoji = ?
         ORDER BY created_at, rowid LIMIT ?",
    )
    .bind(id.to_string())
    .bind(&emoji)
    .bind(MAX_REACTORS)
    .fetch_all(&s.db)
    .await?;
    rows.iter()
        .map(|u| {
            u.parse()
                .map_err(|e| AppError::Internal(anyhow::Error::from(e)))
        })
        .collect::<AppResult<_>>()
        .map(Json)
}

/// The channel of a live message `me` can see; missing, deleted and unseen all look the same.
async fn visible_message(db: &SqlitePool, me: UserId, id: MessageId) -> AppResult<Channel> {
    let channel: Option<String> =
        sqlx::query_scalar("SELECT channel_id FROM messages WHERE id = ? AND deleted_at IS NULL")
            .bind(id.to_string())
            .fetch_optional(db)
            .await?;
    let channel: ChannelId = channel
        .ok_or(AppError::NotFound)?
        .parse()
        .map_err(anyhow::Error::from)?;
    channel_for(db, me, channel).await
}

fn strip_skin_tones(s: &str) -> String {
    s.chars()
        .filter(|c| !('\u{1F3FB}'..='\u{1F3FF}').contains(c))
        .collect()
}

fn is_standard(s: &str) -> bool {
    emojis::get(&strip_skin_tones(s)).is_some()
}

/// A valid reaction key: a standard emoji (skin tone kept), or `custom:<id>` that exists. Only a
/// live custom emoji can be added; a deleted one can still be taken off.
async fn check_key(db: &SqlitePool, key: &str, add: bool) -> AppResult<()> {
    let ok = match key.strip_prefix("custom:") {
        Some(id) => match crate::emoji::load(db, id).await? {
            Some(e) => e.hash.is_some() || !add,
            None => false,
        },
        None => is_standard(key),
    };
    if !ok {
        return Err(AppError::BadRequest("unknown emoji".into()));
    }
    Ok(())
}

async fn count(conn: &mut SqliteConnection, msg: MessageId, emoji: &str) -> AppResult<u32> {
    let n: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM reactions WHERE message_id = ? AND emoji = ?")
            .bind(msg.to_string())
            .bind(emoji)
            .fetch_one(conn)
            .await?;
    Ok(n as u32)
}

/// Add (`add`) or remove `me`'s `emoji` on `msg`. Idempotent: a no-op changes nothing and
/// publishes nothing. Events carry the new absolute count.
pub async fn toggle(
    s: &AppState,
    me: UserId,
    msg: MessageId,
    emoji: &str,
    add: bool,
) -> AppResult<()> {
    // Access checks read through the pool, so they run before the write lock: holding it while
    // waiting for a second pool connection could starve the pool under concurrent toggles.
    let ch = visible_message(&s.db, me, msg).await?;
    let channel_id = ch.id;
    match (ch.kind, ch.server_id) {
        (ChannelKind::Text, Some(server)) if !is_server_member(&s.db, server, me).await? => {
            return Err(AppError::Forbidden);
        }
        _ => {}
    }
    check_key(&s.db, emoji, add).await?;
    let mut tx = crate::db::begin_write(&s.db).await?;
    // A delete that committed since the check wins.
    let live: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM messages WHERE id = ? AND deleted_at IS NULL")
            .bind(msg.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    if live.is_none() {
        return Err(AppError::NotFound);
    }

    let mut events = Vec::new();
    let event = |emoji: &str, count, added| {
        let (emoji, user_id) = (emoji.to_string(), me);
        if added {
            Event::ReactionAdded {
                channel_id,
                message_id: msg,
                emoji,
                user_id,
                count,
            }
        } else {
            Event::ReactionRemoved {
                channel_id,
                message_id: msg,
                emoji,
                user_id,
                count,
            }
        }
    };
    if add {
        let mode: String = sqlx::query_scalar("SELECT reaction_mode FROM channels WHERE id = ?")
            .bind(channel_id.to_string())
            .fetch_one(&mut *tx)
            .await?;
        if ReactionMode::parse(&mode) == ReactionMode::OnePerPerson {
            let others: Vec<String> = sqlx::query_scalar(
                "DELETE FROM reactions WHERE message_id = ? AND user_id = ? AND emoji != ?
                 RETURNING emoji",
            )
            .bind(msg.to_string())
            .bind(me.to_string())
            .bind(emoji)
            .fetch_all(&mut *tx)
            .await?;
            for other in others {
                let n = count(&mut tx, msg, &other).await?;
                events.push(event(&other, n, false));
            }
        }
        let distinct: Vec<String> =
            sqlx::query_scalar("SELECT DISTINCT emoji FROM reactions WHERE message_id = ?")
                .bind(msg.to_string())
                .fetch_all(&mut *tx)
                .await?;
        if distinct.len() as i64 >= MAX_DISTINCT && !distinct.iter().any(|e| e == emoji) {
            return Err(AppError::BadRequest(format!(
                "a message can have at most {MAX_DISTINCT} different reactions"
            )));
        }
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO reactions (message_id, user_id, emoji, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(msg.to_string())
        .bind(me.to_string())
        .bind(emoji)
        .bind(now())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if inserted > 0 {
            let n = count(&mut tx, msg, emoji).await?;
            events.push(event(emoji, n, true));
            crate::emoji::record_use(&mut tx, me, &[emoji.to_string()]).await?;
        }
    } else {
        let deleted =
            sqlx::query("DELETE FROM reactions WHERE message_id = ? AND user_id = ? AND emoji = ?")
                .bind(msg.to_string())
                .bind(me.to_string())
                .bind(emoji)
                .execute(&mut *tx)
                .await?
                .rows_affected();
        if deleted > 0 {
            let n = count(&mut tx, msg, emoji).await?;
            events.push(event(emoji, n, false));
        }
    }
    tx.commit().await?;
    for e in events {
        s.hub.publish(&s.db, e).await;
    }
    Ok(())
}

/// Reaction summaries for a page of messages, in first-reacted order. With `me: None` (events,
/// which serve no particular user) `me` is always false.
pub async fn summaries(
    db: &SqlitePool,
    me: Option<UserId>,
    ids: &[MessageId],
) -> AppResult<HashMap<MessageId, Vec<ReactionSummary>>> {
    let mut out: HashMap<MessageId, Vec<ReactionSummary>> = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let mut q = sqlx::QueryBuilder::new("SELECT message_id, emoji, COUNT(*), MAX(user_id = ");
    q.push_bind(me.map(|u| u.to_string()).unwrap_or_default());
    q.push(") FROM reactions WHERE message_id IN (");
    let mut list = q.separated(", ");
    for id in ids {
        list.push_bind(id.to_string());
    }
    q.push(") GROUP BY message_id, emoji ORDER BY MIN(created_at), MIN(rowid)");
    let rows: Vec<(String, String, i64, bool)> = q.build_query_as().fetch_all(db).await?;
    for (m, emoji, count, me) in rows {
        out.entry(m.parse().map_err(anyhow::Error::from)?)
            .or_default()
            .push(ReactionSummary {
                emoji,
                count: count as u32,
                me,
            });
    }
    Ok(out)
}

/// Emoji used in a message's text, as usage keys: `custom:<id>` for `<:name:id>` and
/// `<a:name:id>` tokens, plain Unicode for standard emoji. Deduplicated, in order.
pub fn used_in(content: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut push = |k: String| {
        if seen.insert(k.clone()) {
            out.push(k);
        }
    };
    let mut rest = content;
    while let Some(start) = rest.find('<') {
        push_standard(&rest[..start], &mut push);
        rest = &rest[start..];
        match custom_token(rest) {
            Some((id, len)) => {
                push(format!("custom:{id}"));
                rest = &rest[len..];
            }
            None => rest = &rest[1..],
        }
    }
    push_standard(rest, &mut push);
    out
}

fn push_standard(text: &str, push: &mut impl FnMut(String)) {
    for g in text.graphemes(true) {
        // ASCII digits, `#` and `*` are emoji bases only with a keycap, which isn't ASCII.
        if !g.is_ascii() && is_standard(g) {
            push(g.to_string());
        }
    }
}

/// `<:name:id>` or `<a:name:id>` at the start of `s`: the id and the token's length.
fn custom_token(s: &str) -> Option<(&str, usize)> {
    let end = s.find('>')?;
    let inner = s[1..end]
        .strip_prefix("a:")
        .or_else(|| s[1..end].strip_prefix(':'))?;
    let (name, id) = inner.split_once(':')?;
    let word = |w: &str| !w.is_empty() && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    (word(name) && word(id)).then_some((id, end + 1))
}

/// Of `keys`, those that are standard emoji or live custom emoji.
pub async fn live_keys(conn: &mut SqliteConnection, keys: Vec<String>) -> AppResult<Vec<String>> {
    let mut out = Vec::with_capacity(keys.len());
    for k in keys {
        if let Some(id) = k.strip_prefix("custom:") {
            let live: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM custom_emoji WHERE id = ? AND deleted_at IS NULL",
            )
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
            if live.is_none() {
                continue;
            }
        }
        out.push(k);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_keys() {
        assert!(is_standard("👍"));
        assert!(is_standard("👍🏽"));
        assert!(!is_standard("abc"));
        assert!(!is_standard(""));
    }

    #[test]
    fn finds_tokens_and_unicode() {
        assert_eq!(
            used_in("hi <:pog:01AB> <a:dance:02CD> 👍🏽 <:pog:01AB> 1 # <b> <:bad name:x> 😂"),
            ["custom:01AB", "custom:02CD", "👍🏽", "😂"]
        );
        assert!(used_in("plain text, 123 #tag *bold*").is_empty());
    }
}
