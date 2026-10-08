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
    for id in ids {
        let users: Vec<String> =
            sqlx::query_scalar("SELECT user_id FROM mentions WHERE message_id = ?")
                .bind(id.to_string())
                .fetch_all(db)
                .await?;
        if !users.is_empty() {
            out.insert(
                *id,
                users
                    .iter()
                    .map(|u| u.parse())
                    .collect::<Result<_, _>>()
                    .map_err(anyhow::Error::from)?,
            );
        }
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
