//! Builds (#616): character builds kept in `builds` channels. A build is a message of a builds channel plus one
//! `builds` row, as a clip is: author, unread, reactions and attachments (its screenshots) come with the message, and
//! its comments are thread replies. The message's `content` is the build as plain text ([`render_content`]), so the
//! search index finds a build by its title, game, link title or notes with no triggers of its own.

pub mod compat;
pub mod routes;

use std::collections::HashMap;

use pulse_protocol::ids::{GameId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{Build, BuildLink, EmbedProvider, EmbedThumb, Message, build_limits};
use sqlx::SqlitePool;

use crate::error::{AppError, AppResult};

pub const TITLE_MAX: usize = build_limits::TITLE_CHARS;
pub const NOTES_MAX: usize = build_limits::NOTES_CHARS;
pub const MAX_SCREENSHOTS: usize = build_limits::MAX_SCREENSHOTS;
pub const LINK_MAX: usize = build_limits::LINK_CHARS;
/// Builds per page of a channel's list.
pub const PAGE: u32 = 30;
/// The title of a build saved from screenshots alone.
pub const UNTITLED: &str = "Untitled build";

const _: () = assert!(NOTES_MAX == crate::messages::MAX_LEN);

/// `builds` columns as [`attach`] reads them, with the thumbnail's size from `media`.
pub(crate) const ROW_COLS: &str = "b.message_id, b.title, b.title_auto, b.game_id, b.game_guessed, b.link_url, \
     b.link_title, b.link_thumb, t.width, t.height, b.link_provider, b.link_provider_id, b.notes, b.quick_drop, \
     b.updated_at, b.archived_at, b.archived_by";

#[derive(sqlx::FromRow)]
pub(crate) struct BuildRow {
    pub message_id: String,
    pub title: String,
    pub title_auto: bool,
    pub game_id: Option<String>,
    pub game_guessed: bool,
    pub link_url: Option<String>,
    pub link_title: Option<String>,
    pub link_thumb: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub link_provider: Option<String>,
    pub link_provider_id: Option<String>,
    pub notes: Option<String>,
    pub quick_drop: bool,
    pub updated_at: Option<String>,
    pub archived_at: Option<String>,
    pub archived_by: Option<String>,
}

impl BuildRow {
    fn into_build(self, thread: Option<crate::messages::threads::ThreadSummary>) -> AppResult<Build> {
        let parse_game = |g: Option<String>| -> AppResult<Option<GameId>> {
            Ok(g.map(|g| g.parse()).transpose().map_err(anyhow::Error::from)?)
        };
        let thumb = match (self.link_thumb, self.width, self.height) {
            (Some(hash), Some(w), Some(h)) => Some(EmbedThumb {
                hash,
                width: w as u32,
                height: h as u32,
            }),
            _ => None,
        };
        Ok(Build {
            title: self.title,
            title_auto: self.title_auto,
            game_id: parse_game(self.game_id)?,
            game_guessed: self.game_guessed,
            link: self.link_url.map(|url| BuildLink {
                url,
                title: self.link_title,
                thumb,
                provider: self
                    .link_provider
                    .as_deref()
                    .map_or(EmbedProvider::Generic, crate::embeds::parse_provider),
                provider_id: self.link_provider_id,
            }),
            notes: self.notes,
            quick_drop: self.quick_drop,
            updated_at: self.updated_at,
            archived_at: self.archived_at,
            archived_by: self
                .archived_by
                .map(|u| u.parse::<UserId>())
                .transpose()
                .map_err(anyhow::Error::from)?,
            reply_count: thread.as_ref().map_or(0, |t| t.reply_count),
            last_reply_id: thread.as_ref().and_then(|t| t.last_reply_id),
            replies_seen: thread.and_then(|t| t.replies_seen),
        })
    }
}

/// Fill `build` on every live build message in `msgs` (`me` is whose view it is for the comments' seen point; events
/// pass `None`).
pub async fn attach(db: &SqlitePool, me: Option<UserId>, msgs: &mut [Message]) -> AppResult<()> {
    let ids: Vec<String> = msgs
        .iter()
        .filter(|m| !m.deleted && m.thread_root_id.is_none())
        .map(|m| m.id.to_string())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let mut q = sqlx::QueryBuilder::new(format!(
        "SELECT {ROW_COLS} FROM builds b LEFT JOIN media t ON t.hash = b.link_thumb WHERE b.message_id IN ("
    ));
    let mut list = q.separated(", ");
    for id in &ids {
        list.push_bind(id);
    }
    q.push(")");
    let rows: Vec<BuildRow> = q.build_query_as().fetch_all(db).await?;
    if rows.is_empty() {
        return Ok(());
    }
    let build_ids: Vec<String> = rows.iter().map(|r| r.message_id.clone()).collect();
    let mut threads = crate::messages::threads::summaries(db, me, &build_ids).await?;
    let mut by_msg: HashMap<String, Build> = HashMap::new();
    for row in rows {
        let id = row.message_id.clone();
        let t = threads.remove(&id);
        by_msg.insert(id, row.into_build(t)?);
    }
    for m in msgs.iter_mut().filter(|m| !m.deleted) {
        m.build = by_msg.remove(&m.id.to_string()).map(Box::new);
    }
    Ok(())
}

/// The build as plain text, for the message's `content` (what search indexes and a notification quotes): its
/// non-empty parts, one per line.
pub fn render_content(
    title: &str,
    game: Option<&str>,
    link_title: Option<&str>,
    notes: Option<&str>,
) -> String {
    [Some(title), game, link_title, notes]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn bad(msg: &str) -> AppError {
    AppError::BadRequest(msg.into())
}

/// A build's link as stored: trimmed, at most [`LINK_MAX`] characters, a URL, `http` or `https`.
pub fn clean_link(raw: &str) -> AppResult<String> {
    let link = raw.trim();
    if link.chars().count() > LINK_MAX {
        return Err(bad("Links can be up to 2048 characters."));
    }
    let url = reqwest::Url::parse(link).map_err(|_| bad("That link isn't a web address."))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(bad("Links must start with http:// or https://."));
    }
    Ok(link.to_string())
}

/// A title as typed: trimmed, 1 to [`TITLE_MAX`] characters (`None` when blank).
pub fn clean_title(raw: Option<&str>) -> AppResult<Option<String>> {
    let Some(t) = raw.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    if t.chars().count() > TITLE_MAX {
        return Err(bad("Titles can be up to 120 characters."));
    }
    Ok(Some(t.to_string()))
}

/// Notes as stored: trimmed at the ends, at most [`NOTES_MAX`] characters (`None` when blank).
pub fn clean_notes(raw: Option<&str>) -> AppResult<Option<String>> {
    let Some(n) = raw.map(str::trim).filter(|n| !n.is_empty()) else {
        return Ok(None);
    };
    if n.chars().count() > NOTES_MAX {
        return Err(bad("Notes can be up to 4000 characters."));
    }
    Ok(Some(n.to_string()))
}

/// A title made from a link until its page's title is fetched: host and path, no scheme, query or trailing slash,
/// cut to fit ([`fit_title`]).
pub fn link_title(link: &str) -> String {
    let short = match reqwest::Url::parse(link) {
        Ok(u) => {
            let host = u.host_str().unwrap_or_default().trim_start_matches("www.");
            format!("{host}{}", u.path().trim_end_matches('/'))
        }
        Err(_) => link.to_string(),
    };
    fit_title(&short)
}

/// Trimmed, at most [`TITLE_MAX`] characters: a longer one is cut and ends with "…".
pub fn fit_title(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= TITLE_MAX {
        return s.to_string();
    }
    let mut cut: String = s.chars().take(TITLE_MAX - 1).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

/// The game's name, for [`render_content`].
pub(crate) async fn game_name(
    c: impl sqlx::SqliteExecutor<'_>,
    game: Option<GameId>,
) -> AppResult<Option<String>> {
    let Some(game) = game else { return Ok(None) };
    Ok(sqlx::query_scalar("SELECT name FROM games WHERE id = ?")
        .bind(game.to_string())
        .fetch_optional(c)
        .await?)
}

/// The builds channel's server, as the build's message says. 404 for anything that isn't a live build.
pub(crate) async fn build_channel(
    db: &SqlitePool,
    id: MessageId,
) -> AppResult<(pulse_protocol::ids::ChannelId, Option<ServerId>)> {
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT m.channel_id, c.server_id FROM builds b
         JOIN messages m ON m.id = b.message_id AND m.deleted_at IS NULL
         JOIN channels c ON c.id = m.channel_id
         WHERE b.message_id = ?",
    )
    .bind(id.to_string())
    .fetch_optional(db)
    .await?;
    let (ch, server) = row.ok_or(AppError::NotFound)?;
    Ok((
        ch.parse().map_err(anyhow::Error::from)?,
        server
            .map(|s| s.parse())
            .transpose()
            .map_err(anyhow::Error::from)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_is_the_non_empty_parts_one_per_line() {
        assert_eq!(render_content("Raven", None, None, None), "Raven");
        assert_eq!(
            render_content("Raven", Some("TFD"), Some("  "), Some("crit\nreload")),
            "Raven\nTFD\ncrit\nreload"
        );
    }

    #[test]
    fn links_are_checked() {
        assert!(clean_link(" https://youtu.be/x ").is_ok());
        assert!(clean_link("ftp://a.com/x").is_err());
        assert!(clean_link("javascript:alert(1)").is_err());
        assert!(clean_link("not a link").is_err());
        assert!(clean_link(&format!("https://a.com/{}", "x".repeat(2040))).is_err());
    }

    #[test]
    fn a_links_title_is_its_host_and_path() {
        assert_eq!(link_title("https://youtu.be/abc12345678"), "youtu.be/abc12345678");
        assert_eq!(link_title("https://www.example.com/planner/?a=1"), "example.com/planner");
    }

    #[test]
    fn long_titles_are_cut_with_an_ellipsis() {
        let t = fit_title(&"a".repeat(300));
        assert_eq!(t.chars().count(), 120);
        assert!(t.ends_with('…'));
        assert_eq!(fit_title("  short "), "short");
    }
}
