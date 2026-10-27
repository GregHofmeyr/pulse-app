//! Reactions on messages. Every add/remove goes through [`toggle`], which applies the channel's
//! reaction mode, so the quotes push can extend one function.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::put;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{Channel, ReactionSummary};
use sqlx::{SqliteConnection, SqlitePool};
use unicode_segmentation::UnicodeSegmentation;

use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::now;
use crate::emoji::canonical_key;
use crate::error::{AppError, AppResult};
use crate::servers::routes::is_server_member;

/// Distinct emoji on one message.
pub const MAX_DISTINCT: i64 = 20;
const MAX_REACTORS: i64 = 100;
/// Distinct emoji counted as usage per message; the rest of a long message goes uncounted. The
/// cap applies before [`live_keys`] drops the ones the sender can't use.
pub const MAX_USED: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReactionMode {
    Normal,
    /// Adding a reaction replaces the reactor's others on that message.
    OnePerPerson,
}

impl ReactionMode {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "normal" => Some(Self::Normal),
            "one_per_person" => Some(Self::OnePerPerson),
            _ => None,
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
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    toggle(&s, me, id, &emoji, true).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path((id, emoji)): Path<(MessageId, String)>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
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
    let emoji = canonical_key(&emoji);
    // Any well-formed custom key lists (possibly nobody), as DELETE accepts it; only junk is refused.
    if !is_custom_key(&emoji) {
        check_key(&s.db, me, id, &emoji, false).await?;
    }
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

/// Every standard emoji and each of its skin-tone variants, in canonical form. Exact sequences
/// only, so a skin tone can't sit anywhere but where Unicode puts one.
static STANDARD: LazyLock<HashSet<String>> = LazyLock::new(|| {
    emojis::iter()
        .flat_map(|e| std::iter::once(e).chain(e.skin_tones().into_iter().flatten()))
        .map(|e| canonical_key(e.as_str()))
        .collect()
});

fn is_standard(s: &str) -> bool {
    STANDARD.contains(&canonical_key(s))
}

/// `custom:<id>` with an id that could be one (letters, digits, `_`), whether or not it exists.
fn is_custom_key(key: &str) -> bool {
    key.strip_prefix("custom:").is_some_and(|id| {
        !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Whether `key` is on `msg` at all.
async fn on_message(db: &SqlitePool, msg: MessageId, key: &str) -> AppResult<bool> {
    let hit: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM reactions WHERE message_id = ? AND emoji = ? LIMIT 1")
            .bind(msg.to_string())
            .bind(key)
            .fetch_optional(db)
            .await?;
    Ok(hit.is_some())
}

/// A valid canonical reaction key: a standard emoji (skin tone kept), or `custom:<id>`. Adding a
/// custom one needs it live and `me` in its server (DMs too); taking one off or listing who
/// reacted never does, so a reaction can't get stuck.
async fn check_key(
    db: &SqlitePool,
    me: UserId,
    msg: MessageId,
    key: &str,
    add: bool,
) -> AppResult<()> {
    let ok = match key.strip_prefix("custom:") {
        Some(id) => match crate::emoji::load(db, id).await? {
            Some(e) if add => {
                if e.hash.is_some() && !is_server_member(db, e.server_id, me).await? {
                    return Err(AppError::ForbiddenMsg(
                        "you can only react with emoji from servers you're in".into(),
                    ));
                }
                e.hash.is_some()
            }
            Some(_) => true,
            // The emoji row is gone. Servers can't be deleted yet, but deleting one would take its
            // emoji with it, leaving reactions that must still be listable and removable.
            None => !add && on_message(db, msg, key).await?,
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
    let emoji = &canonical_key(emoji);
    // Access checks read through the pool, so they run before the write lock: holding it while
    // waiting for a second pool connection could starve the pool under concurrent toggles.
    let ch = visible_message(&s.db, me, msg).await?;
    let channel_id = ch.id;
    match (ch.kind, ch.server_id) {
        (kind, Some(server))
            if kind.is_server_chat() && !is_server_member(&s.db, server, me).await? =>
        {
            return Err(AppError::Forbidden);
        }
        _ => {}
    }
    // Taking off a well-formed custom key needs no emoji: if it's gone, or this is a second click,
    // there's simply nothing to remove.
    if add || !is_custom_key(emoji) {
        check_key(&s.db, me, msg, emoji, add).await?;
    }
    // Events carry absolute counts, so two racing toggles must not publish out of commit order.
    let _order = s.hub.in_order().await;
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
    let mut used = false;
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
        let mode = ReactionMode::parse(&mode).unwrap_or_else(|| {
            // Normal is the safe reading: it never removes anyone's reactions.
            tracing::warn!(%channel_id, mode, "unknown reaction mode; treating as normal");
            ReactionMode::Normal
        });
        // One per person is for quotes; replies and system cards in the channel react normally.
        let is_quote: bool = sqlx::query_scalar(
            "SELECT kind = 'normal' AND thread_root_id IS NULL FROM messages WHERE id = ?",
        )
        .bind(msg.to_string())
        .fetch_one(&mut *tx)
        .await?;
        if mode == ReactionMode::OnePerPerson && is_quote {
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
            crate::emoji::record_use(&mut tx, me, std::slice::from_ref(emoji)).await?;
            used = true;
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
    if used {
        crate::emoji::publish_frequent(s, me).await;
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
/// `<a:name:id>` tokens, canonical Unicode for standard emoji. Deduplicated, in order, and at
/// most [`MAX_USED`] of them.
pub fn used_in(content: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut push = |k: String| {
        if out.len() < MAX_USED && seen.insert(k.clone()) {
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
            push(canonical_key(g));
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

/// Of `keys`, those `user` may use: standard emoji, and live custom emoji of servers `user` is
/// in. One query, however many tokens. (The same "usable" rule as `emoji::frequent`'s SQL.)
pub async fn live_keys(
    conn: &mut SqliteConnection,
    user: UserId,
    keys: Vec<String>,
) -> AppResult<Vec<String>> {
    let ids: Vec<&str> = keys
        .iter()
        .filter_map(|k| k.strip_prefix("custom:"))
        .collect();
    let usable: HashSet<String> = if ids.is_empty() {
        HashSet::new()
    } else {
        let mut q = sqlx::QueryBuilder::new(
            "SELECT e.id FROM custom_emoji e
             JOIN server_members m ON m.server_id = e.server_id AND m.user_id = ",
        );
        q.push_bind(user.to_string());
        q.push(" WHERE e.deleted_at IS NULL AND e.id IN (");
        let mut list = q.separated(", ");
        for id in &ids {
            list.push_bind(*id);
        }
        q.push(")");
        q.build_query_scalar()
            .fetch_all(&mut *conn)
            .await?
            .into_iter()
            .collect()
    };
    Ok(keys
        .into_iter()
        .filter(|k| {
            k.strip_prefix("custom:")
                .is_none_or(|id| usable.contains(id))
        })
        .collect())
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

    #[test]
    fn used_keys_are_canonical_and_capped() {
        assert_eq!(used_in("\u{2764}\u{FE0F} \u{2764} 👍️"), ["\u{2764}", "👍"]);
        let many: String = (0..60).map(|i| format!("<:e{i}:E{i:03}> ")).collect();
        let used = used_in(&many);
        assert_eq!(used.len(), MAX_USED);
        assert_eq!(used[MAX_USED - 1], format!("custom:E{:03}", MAX_USED - 1));
    }

    #[test]
    fn unknown_reaction_mode_is_not_a_mode() {
        assert_eq!(ReactionMode::parse("normal"), Some(ReactionMode::Normal));
        assert_eq!(
            ReactionMode::parse("one_per_person"),
            Some(ReactionMode::OnePerPerson)
        );
        assert_eq!(ReactionMode::parse("bogus"), None);
    }

    #[test]
    fn skin_tones_only_where_they_belong() {
        for junk in [
            "👍🏽🏽",
            "🏽👍",
            "🏽",
            "😀🏽",
            "👍\u{200D}🏽",
            "👩🏽🏽\u{200D}💻",
        ] {
            assert!(!is_standard(junk), "{junk:?}");
        }
        assert!(is_standard("🧑🏻\u{200D}🤝\u{200D}🧑🏿"));
    }

    /// Every emoji the picker offers, skin tones included, is a valid key once canonical.
    #[test]
    fn picker_emoji_are_all_accepted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../client/ui/src/lib/emoji-data.json"
        );
        let data: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut rejected = Vec::new();
        let mut n = 0;
        for e in data["emoji"].as_array().unwrap() {
            let tones = e["s"].as_array().into_iter().flatten();
            for u in std::iter::once(&e["u"]).chain(tones) {
                let u = u.as_str().unwrap();
                n += 1;
                if !is_standard(&crate::emoji::canonical_key(u)) {
                    rejected.push(u.to_string());
                }
            }
        }
        assert!(n > 3000, "{n}");
        assert!(rejected.is_empty(), "{rejected:?}");
    }
}
