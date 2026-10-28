//! Quotes: lines said by members (or plain names), read back with awards, replies and wins.

pub mod awards;
pub use awards::award_totals;
pub mod links;
pub mod routes;

use std::collections::{HashMap, HashSet};

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{LineId, MessageId, QotyEventId, ServerId, UserId};
use pulse_protocol::rest::{LineMedia, Message, NewQuoteLine, Quote, QuoteLine};
use sqlx::{SqliteConnection, SqlitePool};

use crate::error::{AppError, AppResult};
use crate::messages::MAX_LEN;

/// Most lines in one quote.
pub const MAX_LINES: usize = 10;
pub const MAX_CAPTION: usize = 200;
pub const MAX_NAME: usize = 32;

/// A plain speaker name as stored: trimmed, 1..=[`MAX_NAME`] characters, and not only dots (its
/// `name_key` is a path segment, where "." and ".." can't be used, so a link to it could never be
/// undone).
pub fn clean_name(name: &str) -> AppResult<&str> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(AppError::BadRequest(format!(
            "a name must be 1-{MAX_NAME} characters"
        )));
    }
    if name.chars().all(|c| c == '.') {
        return Err(bad("a name needs more than dots"));
    }
    Ok(name)
}

/// Quote marks that wrap a line, as (opening, closing) pairs.
const QUOTE_PAIRS: [(char, char); 5] =
    [('"', '"'), ('“', '”'), ('‘', '’'), ('\'', '\''), ('«', '»')];

/// What a plain speaker name is matched on: Unicode lowercase, trimmed, whitespace runs as one space.
pub fn name_key(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Line text as stored: trimmed, and when it is wrapped in one matching pair of quote marks,
/// that pair removed (one mark from each end). A pair wraps the text only if the opening mark's
/// first closing mark is the last character, so inner quotes (`"Yes" and "no"`) and one-sided
/// marks (`'tis great`, `the dogs'`) are part of the text. An apostrophe inside a word (`don't`)
/// doesn't close a single quote. 1..=[`MAX_LEN`] characters.
pub fn clean_text(s: &str) -> AppResult<String> {
    let t = s.trim();
    let t = match unwrap_pair(t) {
        Some(inner) => inner.trim(),
        None => t,
    };
    if t.is_empty() || t.chars().count() > MAX_LEN {
        return Err(AppError::BadRequest(format!(
            "a line must be 1-{MAX_LEN} characters"
        )));
    }
    Ok(t.to_string())
}

/// `t` without its wrapping pair of quote marks, if it has one (see [`clean_text`]).
fn unwrap_pair(t: &str) -> Option<&str> {
    let chars: Vec<(usize, char)> = t.char_indices().collect();
    let &(_, open) = chars.first()?;
    let &(last_at, _) = chars.last()?;
    let close = QUOTE_PAIRS.iter().find(|(o, _)| *o == open)?.1;
    let word = |i: usize| chars.get(i).is_some_and(|(_, c)| c.is_alphanumeric());
    let first_close = (1..chars.len()).find(|&i| {
        chars[i].1 == close && !(matches!(close, '\'' | '’') && word(i - 1) && word(i + 1))
    })?;
    (first_close == chars.len() - 1).then(|| &t[open.len_utf8()..last_at])
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

/// The rules that need no database: shape, lengths and text cleaning. Run before taking the write
/// lock; [`check_lines`] does the rest on the write transaction.
pub fn clean_lines(lines: &[NewQuoteLine]) -> AppResult<Vec<CleanLine>> {
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
                let name = clean_name(name)?;
                (
                    Some(name.to_string()),
                    Some(name_key(name)),
                    name.to_string(),
                )
            }
            _ => return Err(bad("a line has either a member or a name as its speaker")),
        };
        let (text, media_hash, caption) = match (&l.text, &l.media_hash) {
            (Some(t), None) if l.caption.is_none() => (Some(clean_text(t)?), None, None),
            (None, Some(hash)) => {
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
            speaker_label,
            text,
            media_hash,
            caption,
        });
    }

    Ok(out)
}

/// The rules that read the database: speakers must be members of `server` (their labels are filled
/// in) and pictures must be usable. Run on the write transaction, so a picture can't be swept
/// between the check and the insert.
pub async fn check_lines(
    db: &mut SqliteConnection,
    server: ServerId,
    out: &mut [CleanLine],
) -> AppResult<()> {
    // Members and pictures are checked together: one query each, however many lines.
    let members = member_names(db, server, out).await?;
    for l in out.iter_mut() {
        if let Some(who) = l.speaker_id {
            l.speaker_label = members
                .get(&who)
                .cloned()
                .ok_or_else(|| bad("the speaker isn't in this server"))?;
        }
    }
    let usable = usable_media(db, out).await?;
    if out
        .iter()
        .filter_map(|l| l.media_hash.as_ref())
        .any(|h| !usable.contains(h))
    {
        return Err(bad("unknown picture"));
    }
    Ok(())
}

/// Usernames of the lines' member speakers who are in `server`.
async fn member_names(
    db: &mut SqliteConnection,
    server: ServerId,
    lines: &[CleanLine],
) -> AppResult<HashMap<UserId, String>> {
    let ids: Vec<String> = lines
        .iter()
        .filter_map(|l| l.speaker_id.map(|u| u.to_string()))
        .collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut q = sqlx::QueryBuilder::new(
        "SELECT u.id, u.username FROM server_members sm JOIN users u ON u.id = sm.user_id
         WHERE sm.server_id = ",
    );
    q.push_bind(server.to_string());
    q.push(" AND u.id IN (");
    in_list(&mut q, &ids);
    let rows: Vec<(String, String)> = q.build_query_as().fetch_all(db).await?;
    rows.into_iter()
        .map(|(id, name)| Ok((id.parse().map_err(anyhow::Error::from)?, name)))
        .collect()
}

/// Of the lines' pictures, the ones a line may show: uploaded quote or emoji images that are not
/// just an animation's still.
async fn usable_media(
    db: &mut SqliteConnection,
    lines: &[CleanLine],
) -> AppResult<HashSet<String>> {
    let hashes: Vec<String> = lines
        .iter()
        .filter_map(|l| l.media_hash.clone())
        .filter(|h| crate::media::valid_hash(h))
        .collect();
    if hashes.is_empty() {
        return Ok(HashSet::new());
    }
    let mut q = sqlx::QueryBuilder::new(
        "SELECT hash FROM media WHERE kind IN ('quote', 'emoji')
           AND NOT EXISTS (SELECT 1 FROM media o WHERE o.still_hash = media.hash)
           AND hash IN (",
    );
    in_list(&mut q, &hashes);
    let rows: Vec<String> = q.build_query_scalar().fetch_all(db).await?;
    Ok(rows.into_iter().collect())
}

/// Store `l` as line number `position` of quote `message_id`, with a fresh id.
pub async fn insert_line(
    tx: &mut SqliteConnection,
    message_id: MessageId,
    position: usize,
    l: &CleanLine,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO quote_lines (id, message_id, position, speaker_id, speaker_name, speaker_key, text, media_hash, caption)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(LineId::new().to_string())
    .bind(message_id.to_string())
    .bind(position as i64)
    .bind(l.speaker_id.map(|u| u.to_string()))
    .bind(&l.speaker_name)
    .bind(&l.speaker_key)
    .bind(&l.text)
    .bind(&l.media_hash)
    .bind(&l.caption)
    .execute(tx)
    .await?;
    Ok(())
}

/// Whether the quote's speakers are frozen: it is an option of a live Quote of the Year, so
/// changing a speaker or adding or removing a line would change what people vote on. Linking a
/// plain name is not an edit and is never locked.
pub async fn speakers_locked(tx: &mut SqliteConnection, message_id: MessageId) -> AppResult<bool> {
    let hit: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM qoty_options o JOIN qoty_events e ON e.id = o.event_id
         WHERE o.message_id = ? AND e.ended_at IS NULL LIMIT 1",
    )
    .bind(message_id.to_string())
    .fetch_optional(tx)
    .await?;
    Ok(hit.is_some())
}

/// What a quote's delete leaves to publish once committed.
pub struct Deleted {
    /// One `MessageDeleted` per reply that was live, carrying `thread_root_id`.
    pub replies: Vec<Event>,
    /// Live Quote of the Year events that lost the quote as an option.
    pub qoty: Vec<QotyEventId>,
    /// Winner cards of the quote, rewritten without its words: publish each as `MessageUpdated`.
    pub cards: Vec<MessageId>,
}

/// Everything a deleted quote leaves behind, in the caller's transaction (the quote's own soft
/// delete is the caller's): its reactions, awards and lines go, its replies are soft-deleted,
/// and its Quote of the Year options and votes in a live event go; an ended event keeps its own
/// (and `qoty_winners` rows stay). Returns what to publish once committed.
pub async fn on_delete(tx: &mut SqliteConnection, message_id: MessageId) -> AppResult<Deleted> {
    let id = message_id.to_string();
    sqlx::query("DELETE FROM reactions WHERE message_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "DELETE FROM quote_awards WHERE line_id IN (SELECT id FROM quote_lines WHERE message_id = ?)",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    // Its lines go too: nothing reads a deleted quote's lines (wins keep their own speaker
    // snapshot), and keeping them would keep the text and keep its pictures from being reclaimed.
    sqlx::query("DELETE FROM quote_lines WHERE message_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    // A win stays (spec §3.1), but its card stops showing the deleted words: the payload (votes,
    // speakers) and `qoty_winners` are untouched. The client writes the same text on the quote's
    // MessageDeleted (`deletedWinnerContent` in client/ui/src/lib/state.ts): change both together.
    let cards: Vec<String> = sqlx::query_scalar(
        "UPDATE messages
         SET content = 'Quote of the Year ' || json_extract(system_payload, '$.year') || ': a deleted quote'
         WHERE kind = 'system' AND deleted_at IS NULL
           AND json_extract(system_payload, '$.type') = ?
           AND json_extract(system_payload, '$.message_id') = ?
         RETURNING id",
    )
    .bind(crate::messages::system::QOTY_CARD)
    .bind(&id)
    .fetch_all(&mut *tx)
    .await?;
    // Only a live event loses the quote; an ended one keeps its options and votes as history.
    // Votes go with their option (foreign key cascade), but not by luck: say so.
    const LIVE: &str = "event_id IN (SELECT id FROM qoty_events WHERE ended_at IS NULL)";
    let qoty: Vec<String> = sqlx::query_scalar(&format!(
        "SELECT event_id FROM qoty_options WHERE message_id = ? AND {LIVE}"
    ))
    .bind(&id)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query(&format!(
        "DELETE FROM qoty_votes WHERE message_id = ? AND {LIVE}"
    ))
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(&format!(
        "DELETE FROM qoty_options WHERE message_id = ? AND {LIVE}"
    ))
    .bind(&id)
    .execute(&mut *tx)
    .await?;

    let replies = crate::messages::threads::soft_delete_replies(&mut *tx, message_id).await?;
    Ok(Deleted {
        replies,
        qoty: qoty.iter().map(|e| parse_id(e)).collect::<AppResult<_>>()?,
        cards: cards
            .iter()
            .map(|c| parse_id(c))
            .collect::<AppResult<_>>()?,
    })
}

/// Live quotes of `server` ranked by reaction score: distinct people who reacted to the quote
/// itself (replies don't count), best first, then newest. A quote nobody reacted to isn't ranked.
/// `year` keeps one year's; `new_winners_only` leaves out quotes that already won a Quote of the
/// Year; `limit` caps the list (`None`: all). Returns (message id, score).
pub(crate) async fn ranked<'e, E>(
    db: E,
    server: ServerId,
    year: Option<i32>,
    new_winners_only: bool,
    limit: Option<usize>,
) -> AppResult<Vec<(String, i64)>>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    Ok(sqlx::query_as(
        "SELECT q.message_id, COUNT(DISTINCT r.user_id) AS score
         FROM quotes q
         JOIN messages msg ON msg.id = q.message_id AND msg.deleted_at IS NULL
         JOIN channels c ON c.id = msg.channel_id
         JOIN reactions r ON r.message_id = q.message_id
         WHERE c.server_id = ? AND (? IS NULL OR q.year = ?)
           AND (? = 0 OR NOT EXISTS (SELECT 1 FROM qoty_winners w WHERE w.message_id = q.message_id))
         GROUP BY q.message_id
         ORDER BY score DESC, q.message_id DESC
         LIMIT ?",
    )
    .bind(server.to_string())
    .bind(year)
    .bind(year)
    .bind(new_winners_only)
    // SQLite reads a negative LIMIT as none.
    .bind(limit.map_or(-1, |n| n as i64))
    .fetch_all(db)
    .await?)
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
/// to. `speaker_key` is the stored key of the plain name; `links` maps `name_key` to member for
/// the line's server.
pub fn effective_speaker(
    speaker_id: Option<UserId>,
    speaker_key: Option<&str>,
    links: &HashMap<String, UserId>,
) -> Option<UserId> {
    speaker_id.or_else(|| speaker_key.and_then(|k| links.get(k).copied()))
}

/// Parse a stored id.
pub(crate) fn parse_id<T: std::str::FromStr>(s: &str) -> AppResult<T>
where
    T::Err: Into<anyhow::Error>,
{
    s.parse().map_err(|e: T::Err| AppError::Internal(e.into()))
}

/// Parse a stored id that may be absent.
pub(crate) fn parse_opt<T: std::str::FromStr>(s: Option<&str>) -> AppResult<Option<T>>
where
    T::Err: Into<anyhow::Error>,
{
    s.map(parse_id).transpose()
}

/// Order of awards on a line: oldest first. The one place the rule lives, for SQL.
pub(crate) const AWARD_ORDER: &str = "ORDER BY created_at, rowid";

/// The counted awarders among `givers` (oldest first, as given): everyone but the line's
/// effective speaker, whose own award doesn't count.
pub(crate) fn counted(
    givers: impl IntoIterator<Item = UserId>,
    speaker: Option<UserId>,
) -> Vec<UserId> {
    givers.into_iter().filter(|g| Some(*g) != speaker).collect()
}

/// `name_key` to member for every speaker link of `servers`.
pub(crate) async fn links_for<'e, E>(
    db: E,
    servers: &[String],
) -> AppResult<HashMap<String, HashMap<String, UserId>>>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
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

/// A stored line with its picture's still and size and the server it belongs to.
#[derive(sqlx::FromRow)]
struct LineRow {
    message_id: String,
    id: String,
    speaker_id: Option<String>,
    speaker_name: Option<String>,
    speaker_key: Option<String>,
    text: Option<String>,
    media_hash: Option<String>,
    caption: Option<String>,
    still_hash: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    server_id: Option<String>,
}

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
        "SELECT l.message_id, l.id, l.speaker_id, l.speaker_name, l.speaker_key, l.text, l.media_hash,
                l.caption, m.still_hash, m.width, m.height, c.server_id
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
        let mut s: Vec<String> = rows.iter().filter_map(|r| r.server_id.clone()).collect();
        s.sort();
        s.dedup();
        s
    };
    let links = links_for(db, &servers).await?;
    let no_links = HashMap::new();

    let line_ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let mut awards: HashMap<String, Vec<UserId>> = HashMap::new();
    if !line_ids.is_empty() {
        let mut q = sqlx::QueryBuilder::new(
            "SELECT line_id, giver_id FROM quote_awards WHERE line_id IN (",
        );
        in_list(&mut q, &line_ids);
        q.push(" ").push(AWARD_ORDER);
        let given: Vec<(String, String)> = q.build_query_as().fetch_all(db).await?;
        for (line, giver) in given {
            awards.entry(line).or_default().push(parse_id(&giver)?);
        }
    }

    let mut by_msg: HashMap<String, Vec<QuoteLine>> = HashMap::new();
    for r in rows {
        let speaker_id: Option<UserId> = parse_opt(r.speaker_id.as_deref())?;
        let speaker = effective_speaker(
            speaker_id,
            r.speaker_key.as_deref(),
            r.server_id
                .as_ref()
                .and_then(|s| links.get(s))
                .unwrap_or(&no_links),
        );
        let awarders = counted(awards.remove(&r.id).unwrap_or_default(), speaker);
        by_msg.entry(r.message_id).or_default().push(QuoteLine {
            id: parse_id(&r.id)?,
            speaker_id,
            speaker_name: r.speaker_name,
            text: r.text,
            media: r.media_hash.map(|hash| LineMedia {
                hash,
                still_hash: r.still_hash,
                width: r.width.unwrap_or(0) as u32,
                height: r.height.unwrap_or(0) as u32,
            }),
            caption: r.caption,
            awards: awarders.len() as u32,
            awarded: me.is_some_and(|me| awarders.contains(&me)),
            awarders,
        });
    }

    let threads = crate::messages::threads::summaries(db, me, &quote_ids).await?;

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

    let index: HashMap<String, usize> = msgs
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id.to_string(), i))
        .collect();
    for (msg_id, year, said_on) in heads {
        let Some(&i) = index.get(&msg_id) else {
            continue;
        };
        let m = &mut msgs[i];
        let t = threads.get(&msg_id);
        m.quote = Some(Quote {
            lines: by_msg.remove(&msg_id).unwrap_or_default(),
            year: year as i32,
            said_on,
            reply_count: t.map_or(0, |t| t.reply_count),
            last_reply_id: t.and_then(|t| t.last_reply_id),
            replies_seen: t.and_then(|t| t.replies_seen),
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
        for s in ["\"x\"", "“x”", "'x'", "‘x’", "«x»", "  “ x ”  "] {
            assert_eq!(clean_text(s).unwrap(), "x", "{s}");
        }
        assert_eq!(
            clean_text("he said \"hi\" twice").unwrap(),
            "he said \"hi\" twice"
        );
        assert_eq!(clean_text("'tis great").unwrap(), "'tis great");
        assert_eq!(clean_text("the dogs'").unwrap(), "the dogs'");
        assert_eq!(clean_text("\"").unwrap(), "\"");
        assert!(clean_text("  ").is_err());
        assert!(clean_text("“”").is_err());
        assert!(clean_text(&"a".repeat(MAX_LEN)).is_ok());
        assert!(clean_text(&"a".repeat(MAX_LEN + 1)).is_err());
    }

    /// One mark from each end, only for a pair that opens and closes the whole text (M6).
    #[test]
    fn clean_text_strips_one_matching_pair() {
        for (s, want) in [
            // Not a pair: kept as typed.
            ("“x\"", "“x\""),
            ("'x\"", "'x\""),
            // The first mark closes before the end: these are inner quotes, not a wrapper.
            ("\"Yes\" and \"no\"", "\"Yes\" and \"no\""),
            ("“Yes” and “no”", "“Yes” and “no”"),
            ("\"\"x\"\"", "\"\"x\"\""),
            // Only one pair goes; marks inside stay.
            ("“He said ‘hi’”", "He said ‘hi’"),
            ("“‘x’”", "‘x’"),
            // An apostrophe inside a word doesn't close a single quote.
            ("'don't do that'", "don't do that"),
            ("‘don’t’", "don’t"),
            // Ruling (fix wave U1): a leading elision can't be told from an opening quote, so a
            // line that both opens and closes with a single quote is a wrapper and loses both.
            ("'tis the dogs'", "tis the dogs"),
            // One-sided, it's part of the text.
            ("'tis great", "'tis great"),
        ] {
            assert_eq!(clean_text(s).unwrap(), want, "{s}");
        }
    }
}
