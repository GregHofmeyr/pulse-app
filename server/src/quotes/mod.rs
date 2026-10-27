//! Quotes: lines said by members (or plain names), read back with awards, replies and wins.

pub mod routes;

use std::collections::HashMap;

use pulse_protocol::ids::{LineId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{LineMedia, Message, NewQuoteLine, Quote, QuoteLine};
use sqlx::SqlitePool;

use crate::error::{AppError, AppResult};
use crate::servers::routes::is_server_member;

/// Most lines in one quote.
pub const MAX_LINES: usize = 10;
/// Longest line text, in characters.
pub const MAX_LEN: usize = 4000;
pub const MAX_CAPTION: usize = 200;
pub const MAX_NAME: usize = 32;

const QUOTE_MARKS: [char; 8] = ['"', '“', '”', '\'', '‘', '’', '«', '»'];

/// What a plain speaker name is matched on: Unicode lowercase, trimmed, whitespace runs as one space.
pub fn name_key(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Line text as stored: trimmed, with the quote marks around it removed (a matching pair, or a
/// leading and trailing run), 1..=[`MAX_LEN`] characters.
pub fn clean_text(s: &str) -> AppResult<String> {
    let t = s.trim().trim_matches(|c| QUOTE_MARKS.contains(&c)).trim();
    if t.is_empty() || t.chars().count() > MAX_LEN {
        return Err(AppError::BadRequest(format!(
            "a line must be 1-{MAX_LEN} characters"
        )));
    }
    Ok(t.to_string())
}

/// A validated line, ready to store. `id` is the client's, for edits of existing lines.
#[derive(Clone, Debug, PartialEq)]
pub struct CleanLine {
    pub id: Option<LineId>,
    pub speaker_id: Option<UserId>,
    pub speaker_name: Option<String>,
    pub speaker_key: Option<String>,
    /// Who to show in the plain rendering: the member's username or the plain name.
    pub speaker_label: String,
    pub text: Option<String>,
    pub media_hash: Option<String>,
    pub caption: Option<String>,
}

fn bad(msg: &str) -> AppError {
    AppError::BadRequest(msg.into())
}

/// Check a client's lines against the rules and the server they are posted in.
pub async fn validate_lines(
    db: &SqlitePool,
    server: ServerId,
    lines: &[NewQuoteLine],
) -> AppResult<Vec<CleanLine>> {
    if lines.is_empty() || lines.len() > MAX_LINES {
        return Err(AppError::BadRequest(format!(
            "a quote has 1-{MAX_LINES} lines"
        )));
    }
    let mut out = Vec::with_capacity(lines.len());
    for l in lines {
        let (speaker_name, speaker_key, speaker_label) = match (&l.speaker_id, &l.speaker_name) {
            (Some(_), None) => (None, None, String::new()),
            (None, Some(name)) => {
                let name = name.trim();
                if name.is_empty() || name.chars().count() > MAX_NAME {
                    return Err(AppError::BadRequest(format!(
                        "a name must be 1-{MAX_NAME} characters"
                    )));
                }
                (
                    Some(name.to_string()),
                    Some(name_key(name)),
                    name.to_string(),
                )
            }
            _ => return Err(bad("a line has either a member or a name as its speaker")),
        };
        let mut label = speaker_label;
        if let Some(who) = l.speaker_id {
            if !is_server_member(db, server, who).await? {
                return Err(bad("the speaker isn't in this server"));
            }
            label = sqlx::query_scalar("SELECT username FROM users WHERE id = ?")
                .bind(who.to_string())
                .fetch_one(db)
                .await?;
        }
        let (text, media_hash, caption) = match (&l.text, &l.media_hash) {
            (Some(t), None) if l.caption.is_none() => (Some(clean_text(t)?), None, None),
            (None, Some(hash)) => {
                if !usable_media(db, hash).await? {
                    return Err(bad("unknown picture"));
                }
                let caption = match l.caption.as_deref().map(str::trim) {
                    Some(c) if c.chars().count() > MAX_CAPTION => {
                        return Err(AppError::BadRequest(format!(
                            "a caption is at most {MAX_CAPTION} characters"
                        )));
                    }
                    Some("") | None => None,
                    Some(c) => Some(c.to_string()),
                };
                (None, Some(hash.clone()), caption)
            }
            (Some(_), Some(_)) => return Err(bad("a line is text or a picture, not both")),
            (Some(_), None) => return Err(bad("only a picture has a caption")),
            (None, None) => return Err(bad("a line needs text or a picture")),
        };
        out.push(CleanLine {
            id: l.id,
            speaker_id: l.speaker_id,
            speaker_name,
            speaker_key,
            speaker_label: label,
            text,
            media_hash,
            caption,
        });
    }
    Ok(out)
}

/// A picture a line may show: an uploaded quote or emoji image that is not just an animation's still.
async fn usable_media(db: &SqlitePool, hash: &str) -> AppResult<bool> {
    if !crate::media::valid_hash(hash) {
        return Ok(false);
    }
    let hit: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM media WHERE hash = ? AND kind IN ('quote', 'emoji')
           AND NOT EXISTS (SELECT 1 FROM media o WHERE o.still_hash = media.hash)",
    )
    .bind(hash)
    .fetch_optional(db)
    .await?;
    Ok(hit.is_some())
}

/// The plain rendering stored in `messages.content`: `“text” — speaker` per line, `[image]` for pictures.
pub fn render_content(lines: &[CleanLine]) -> String {
    lines
        .iter()
        .map(|l| match &l.text {
            Some(t) => format!("“{t}” — {}", l.speaker_label),
            None => "[image]".to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A line's effective speaker: its member if it has one, else the member its plain name is linked
/// to. `links` maps `name_key` to member for the line's server.
pub fn effective_speaker(line: &QuoteLine, links: &HashMap<String, UserId>) -> Option<UserId> {
    line.speaker_id.or_else(|| {
        line.speaker_name
            .as_deref()
            .and_then(|n| links.get(&name_key(n)).copied())
    })
}

/// `name_key` to member for every speaker link of `servers`.
async fn links_for(
    db: &SqlitePool,
    servers: &[String],
) -> AppResult<HashMap<String, HashMap<String, UserId>>> {
    let mut out: HashMap<String, HashMap<String, UserId>> = HashMap::new();
    if servers.is_empty() {
        return Ok(out);
    }
    let mut q = sqlx::QueryBuilder::new(
        "SELECT server_id, name_key, user_id FROM speaker_links WHERE server_id IN (",
    );
    let mut list = q.separated(", ");
    for s in servers {
        list.push_bind(s);
    }
    q.push(")");
    let rows: Vec<(String, String, String)> = q.build_query_as().fetch_all(db).await?;
    for (server, key, user) in rows {
        out.entry(server)
            .or_default()
            .insert(key, user.parse().map_err(anyhow::Error::from)?);
    }
    Ok(out)
}

fn in_list<'a>(q: &mut sqlx::QueryBuilder<'a, sqlx::Sqlite>, ids: &'a [String]) {
    let mut list = q.separated(", ");
    for id in ids {
        list.push_bind(id);
    }
    q.push(")");
}

/// message, line, speaker id, speaker name, text, media hash, caption, still, width, height, server.
type LineRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<String>,
);

/// Fill `quote` on every live quote message in `msgs`, for `me` (or for nobody, as in events).
pub async fn attach(db: &SqlitePool, me: Option<UserId>, msgs: &mut [Message]) -> AppResult<()> {
    let ids: Vec<String> = msgs
        .iter()
        .filter(|m| !m.deleted)
        .map(|m| m.id.to_string())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let mut q = sqlx::QueryBuilder::new(
        "SELECT message_id, year, said_on FROM quotes WHERE message_id IN (",
    );
    in_list(&mut q, &ids);
    let heads: Vec<(String, i64, Option<String>)> = q.build_query_as().fetch_all(db).await?;
    if heads.is_empty() {
        return Ok(());
    }
    let quote_ids: Vec<String> = heads.iter().map(|h| h.0.clone()).collect();

    // Lines in order, with the picture's still and size and the server they belong to.
    let mut q = sqlx::QueryBuilder::new(
        "SELECT l.message_id, l.id, l.speaker_id, l.speaker_name, l.text, l.media_hash, l.caption,
                m.still_hash, m.width, m.height, c.server_id
         FROM quote_lines l
         JOIN messages msg ON msg.id = l.message_id
         JOIN channels c ON c.id = msg.channel_id
         LEFT JOIN media m ON m.hash = l.media_hash
         WHERE l.message_id IN (",
    );
    in_list(&mut q, &quote_ids);
    q.push(" ORDER BY l.message_id, l.position");
    let rows: Vec<LineRow> = q.build_query_as().fetch_all(db).await?;

    let servers: Vec<String> = {
        let mut s: Vec<String> = rows.iter().filter_map(|r| r.10.clone()).collect();
        s.sort();
        s.dedup();
        s
    };
    let links = links_for(db, &servers).await?;
    let no_links = HashMap::new();

    let line_ids: Vec<String> = rows.iter().map(|r| r.1.clone()).collect();
    let mut awards: HashMap<String, Vec<UserId>> = HashMap::new();
    if !line_ids.is_empty() {
        let mut q = sqlx::QueryBuilder::new(
            "SELECT line_id, giver_id FROM quote_awards WHERE line_id IN (",
        );
        in_list(&mut q, &line_ids);
        q.push(" ORDER BY created_at, rowid");
        let given: Vec<(String, String)> = q.build_query_as().fetch_all(db).await?;
        for (line, giver) in given {
            awards
                .entry(line)
                .or_default()
                .push(giver.parse().map_err(anyhow::Error::from)?);
        }
    }

    let mut by_msg: HashMap<String, Vec<QuoteLine>> = HashMap::new();
    for (msg, id, speaker_id, speaker_name, text, media_hash, caption, still, w, h, server) in rows
    {
        let mut line = QuoteLine {
            id: id.parse().map_err(anyhow::Error::from)?,
            speaker_id: speaker_id
                .as_deref()
                .map(str::parse)
                .transpose()
                .map_err(anyhow::Error::from)?,
            speaker_name,
            text,
            media: media_hash.map(|hash| LineMedia {
                hash,
                still_hash: still,
                width: w.unwrap_or(0) as u32,
                height: h.unwrap_or(0) as u32,
            }),
            caption,
            awards: 0,
            awarders: vec![],
            awarded: false,
        };
        let speaker = effective_speaker(
            &line,
            server
                .as_ref()
                .and_then(|s| links.get(s))
                .unwrap_or(&no_links),
        );
        line.awarders = awards
            .remove(&id)
            .unwrap_or_default()
            .into_iter()
            .filter(|g| Some(*g) != speaker)
            .collect();
        line.awards = line.awarders.len() as u32;
        line.awarded = me.is_some_and(|me| line.awarders.contains(&me));
        by_msg.entry(msg).or_default().push(line);
    }

    // Live replies, the newest of them, and how far `me` has read.
    let mut q = sqlx::QueryBuilder::new(
        "SELECT thread_root_id, COUNT(*), MAX(id) FROM messages
         WHERE deleted_at IS NULL AND thread_root_id IN (",
    );
    in_list(&mut q, &quote_ids);
    q.push(" GROUP BY thread_root_id");
    let replies: HashMap<String, (i64, String)> = q
        .build_query_as::<(String, i64, String)>()
        .fetch_all(db)
        .await?
        .into_iter()
        .map(|(root, n, last)| (root, (n, last)))
        .collect();
    let mut seen: HashMap<String, String> = HashMap::new();
    if let Some(me) = me {
        let mut q = sqlx::QueryBuilder::new(
            "SELECT root_id, last_seen_id FROM thread_reads WHERE user_id = ",
        );
        q.push_bind(me.to_string());
        q.push(" AND root_id IN (");
        in_list(&mut q, &quote_ids);
        seen = q
            .build_query_as::<(String, String)>()
            .fetch_all(db)
            .await?
            .into_iter()
            .collect();
    }

    let mut q = sqlx::QueryBuilder::new(
        "SELECT w.message_id, e.year FROM qoty_winners w JOIN qoty_events e ON e.id = w.event_id
         WHERE w.message_id IN (",
    );
    in_list(&mut q, &quote_ids);
    q.push(" ORDER BY e.year");
    let mut wins: HashMap<String, Vec<i32>> = HashMap::new();
    for (msg, year) in q.build_query_as::<(String, i64)>().fetch_all(db).await? {
        wins.entry(msg).or_default().push(year as i32);
    }

    let parse_id = |s: &String| -> AppResult<MessageId> {
        Ok(s.parse::<MessageId>().map_err(anyhow::Error::from)?)
    };
    for (msg_id, year, said_on) in heads {
        let Some(m) = msgs.iter_mut().find(|m| m.id.to_string() == msg_id) else {
            continue;
        };
        let (reply_count, last_reply_id) = match replies.get(&msg_id) {
            Some((n, last)) => (*n as u32, Some(parse_id(last)?)),
            None => (0, None),
        };
        m.quote = Some(Quote {
            lines: by_msg.remove(&msg_id).unwrap_or_default(),
            year: year as i32,
            said_on,
            reply_count,
            last_reply_id,
            replies_seen: seen.get(&msg_id).map(parse_id).transpose()?,
            wins: wins.remove(&msg_id).unwrap_or_default(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_keys_fold_case_and_whitespace() {
        assert_eq!(name_key("  Sam   B "), "sam b");
        assert_eq!(name_key("ÉMILE\tX"), "émile x");
    }

    #[test]
    fn clean_text_strips_surrounding_marks_only() {
        for s in [
            "\"x\"",
            "“x”",
            "'x'",
            "‘x’",
            "«x»",
            "  “ x ”  ",
            "\"\"x\"\"",
        ] {
            assert_eq!(clean_text(s).unwrap(), "x", "{s}");
        }
        assert_eq!(
            clean_text("he said \"hi\" twice").unwrap(),
            "he said \"hi\" twice"
        );
        assert!(clean_text("  ").is_err());
        assert!(clean_text("“”").is_err());
        assert!(clean_text(&"a".repeat(MAX_LEN)).is_ok());
        assert!(clean_text(&"a".repeat(MAX_LEN + 1)).is_err());
    }
}
