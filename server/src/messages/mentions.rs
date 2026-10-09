//! `@username` mentions: parsed on send, kept only for people who can see the channel.

use std::collections::HashMap;

use pulse_protocol::ids::{MessageId, UserId};
use pulse_protocol::rest::Channel;
use sqlx::SqlitePool;

use crate::error::AppResult;

/// Lowercased candidate names in order of first appearance; ignores inline code and emails.
pub fn parse(content: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut in_code = false;
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            in_code = !in_code;
        } else if c == '@'
            && !in_code
            && (i == 0 || chars[i - 1].is_whitespace() || chars[i - 1] == '(')
        {
            let name: String = chars[i + 1..]
                .iter()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
                .take(32)
                .collect();
            let name = name.trim_end_matches('.').to_lowercase();
            if !name.is_empty() && !out.contains(&name) {
                out.push(name);
            }
        }
        i += 1;
    }
    out
}

/// Names → user ids of people who can see `channel`, excluding the author.
pub async fn resolve(
    db: &SqlitePool,
    channel: &Channel,
    author: UserId,
    names: &[String],
) -> AppResult<Vec<UserId>> {
    let mut out = Vec::new();
    for name in names {
        let id: Option<String> = if channel.kind.is_private() {
            sqlx::query_scalar(
                "SELECT u.id FROM users u JOIN channel_members cm ON cm.user_id = u.id
                 WHERE cm.channel_id = ? AND lower(u.username) = ?",
            )
            .bind(channel.id.to_string())
            .bind(name)
            .fetch_optional(db)
            .await?
        } else {
            sqlx::query_scalar("SELECT id FROM users WHERE lower(username) = ?")
                .bind(name)
                .fetch_optional(db)
                .await?
        };
        if let Some(id) = id {
            let id: UserId = id.parse().map_err(anyhow::Error::from)?;
            if id != author && !out.contains(&id) {
                out.push(id);
            }
        }
    }
    Ok(out)
}

/// Whether `user` can see `channel` (the same rule `resolve` applies to typed names).
pub async fn can_see(db: &SqlitePool, channel: &Channel, user: UserId) -> AppResult<bool> {
    if !channel.kind.is_private() {
        return Ok(true);
    }
    let hit: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM channel_members WHERE channel_id = ? AND user_id = ?")
            .bind(channel.id.to_string())
            .bind(user.to_string())
            .fetch_optional(db)
            .await?;
    Ok(hit.is_some())
}

pub async fn store(db: &SqlitePool, message: MessageId, users: &[UserId]) -> AppResult<()> {
    for u in users {
        sqlx::query("INSERT OR IGNORE INTO mentions (message_id, user_id) VALUES (?, ?)")
            .bind(message.to_string())
            .bind(u.to_string())
            .execute(db)
            .await?;
    }
    Ok(())
}

pub async fn for_messages(
    db: &SqlitePool,
    ids: &[MessageId],
) -> AppResult<HashMap<MessageId, Vec<UserId>>> {
    let mut out: HashMap<MessageId, Vec<UserId>> = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    // One round trip for the whole page (history loads up to 100 at a time).
    let mut q =
        sqlx::QueryBuilder::new("SELECT message_id, user_id FROM mentions WHERE message_id IN (");
    let mut list = q.separated(", ");
    for id in ids {
        list.push_bind(id.to_string());
    }
    q.push(") ORDER BY rowid");
    let rows: Vec<(String, String)> = q.build_query_as().fetch_all(db).await?;
    for (m, u) in rows {
        out.entry(m.parse().map_err(anyhow::Error::from)?)
            .or_default()
            .push(u.parse().map_err(anyhow::Error::from)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_names_case_insensitively_and_dedupes() {
        assert_eq!(
            super::parse("@Sam hi @sam, @jo.k! email a@b.c `@code`"),
            vec!["sam", "jo.k"]
        );
    }
}
