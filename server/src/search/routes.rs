//! `GET /servers/{id}/search` and `GET /channels/{id}/search`: the index, filtered to what you can
//! read, newest first.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{Channel, SearchHas, SearchHit, SearchResults};
use serde::Deserialize;
use sqlx::{QueryBuilder, Sqlite};

use crate::AppState;
use crate::access::{channel_for, readable_server_channels};
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::messages::routes::{COLS, Row, fill};

use super::text::{self, Term};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/servers/{id}/search", get(in_server))
        .route("/channels/{id}/search", get(in_conversation))
}

/// Hits per page.
pub const PAGE: i64 = 25;
/// Matches counted for the header; one past "50", so the UI can say "50+".
const COUNT_CAP: i64 = 51;
/// Longest `q`, in characters.
const MAX_Q: usize = 200;

#[derive(Deserialize)]
struct Params {
    q: Option<String>,
    from: Option<UserId>,
    #[serde(rename = "in")]
    in_channel: Option<ChannelId>,
    /// Comma-separated `SearchHas` words.
    has: Option<String>,
    mentions: Option<UserId>,
    cursor: Option<MessageId>,
}

/// A search, checked: what to match and where.
struct Search {
    terms: Vec<Term>,
    fts: Option<String>,
    from: Option<UserId>,
    has: Vec<SearchHas>,
    mentions: Option<UserId>,
    cursor: Option<MessageId>,
}

impl Search {
    fn parse(p: &Params) -> AppResult<Self> {
        let q = p.q.as_deref().unwrap_or_default();
        if q.chars().count() > MAX_Q {
            return Err(AppError::BadRequest(format!(
                "search for at most {MAX_Q} characters"
            )));
        }
        let terms = text::terms(q);
        let fts = text::fts_query(&terms);
        let mut has = Vec::new();
        for w in p.has.as_deref().unwrap_or_default().split(',') {
            let w = w.trim();
            if w.is_empty() {
                continue;
            }
            let h = SearchHas::parse(w)
                .ok_or_else(|| AppError::BadRequest(format!("can't search for has:{w}")))?;
            if !has.contains(&h) {
                has.push(h);
            }
        }
        if fts.is_none()
            && p.from.is_none()
            && p.in_channel.is_none()
            && has.is_empty()
            && p.mentions.is_none()
        {
            return Err(AppError::BadRequest(
                "search for a word or a filter".into(),
            ));
        }
        Ok(Self {
            terms,
            fts,
            from: p.from,
            has,
            mentions: p.mentions,
            cursor: p.cursor,
        })
    }

    /// `FROM … WHERE …` over the searchable messages of `channels` that match. The index's own
    /// table leads when there are words, so the match narrows the rows first.
    fn push_from_where<'a>(&'a self, q: &mut QueryBuilder<'a, Sqlite>, channels: &'a [String]) {
        match &self.fts {
            Some(expr) => {
                q.push(
                    " FROM message_search s JOIN search_docs d ON d.doc = s.rowid
                      JOIN messages m ON m.id = d.message_id
                      JOIN channels c ON c.id = m.channel_id
                      WHERE message_search MATCH ",
                );
                q.push_bind(expr.as_str());
            }
            None => {
                q.push(
                    " FROM messages m JOIN search_docs d ON d.message_id = m.id
                      JOIN channels c ON c.id = m.channel_id
                      WHERE 1 = 1",
                );
            }
        }
        q.push(" AND m.channel_id IN (");
        let mut list = q.separated(", ");
        for c in channels {
            list.push_bind(c.as_str());
        }
        q.push(")");
        if let Some(from) = self.from {
            // In a quotes channel "who said it" is the speaker: a member line, or a plain name
            // linked to that member now. The poster counts too.
            let u = from.to_string();
            q.push(" AND (m.author_id = ")
                .push_bind(u.clone())
                .push(
                    " OR (c.kind = 'quotes' AND EXISTS (SELECT 1 FROM quote_lines l
                     WHERE l.message_id = m.id AND (l.speaker_id = ",
                )
                .push_bind(u.clone())
                .push(
                    " OR EXISTS (SELECT 1 FROM speaker_links k
                     WHERE k.server_id = c.server_id AND k.name_key = l.speaker_key AND k.user_id = ",
                )
                .push_bind(u)
                .push(")))))");
        }
        for h in &self.has {
            q.push(match h {
                SearchHas::Clip => " AND EXISTS (SELECT 1 FROM clips x WHERE x.message_id = m.id)",
                SearchHas::Quote => " AND EXISTS (SELECT 1 FROM quotes x WHERE x.message_id = m.id)",
                SearchHas::Link => {
                    " AND (instr(m.content, 'http://') > 0 OR instr(m.content, 'https://') > 0
                       OR instr(lower(m.content), 'www.') > 0)"
                }
            });
        }
        if let Some(u) = self.mentions {
            q.push(" AND EXISTS (SELECT 1 FROM mentions x WHERE x.message_id = m.id AND x.user_id = ")
                .push_bind(u.to_string())
                .push(")");
        }
    }
}

async fn in_server(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(server): Path<ServerId>,
    Query(p): Query<Params>,
) -> AppResult<Json<SearchResults>> {
    limit(&s, me)?;
    let search = Search::parse(&p)?;
    let mut channels = readable_server_channels(&s.db, me, server).await?;
    // Another server's channel (or one you can't read) is simply not among them: nothing matches.
    if let Some(only) = p.in_channel {
        channels.retain(|c| c.id == only);
    }
    run(&s, me, &search, &channels).await.map(Json)
}

async fn in_conversation(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Query(p): Query<Params>,
) -> AppResult<Json<SearchResults>> {
    limit(&s, me)?;
    let ch = channel_for(&s.db, me, id).await?;
    if !ch.kind.is_private() {
        return Err(AppError::BadRequest(
            "server channels are searched with the server's search".into(),
        ));
    }
    if p.in_channel.is_some() {
        return Err(AppError::BadRequest("in: is for server search".into()));
    }
    let search = Search::parse(&p)?;
    run(&s, me, &search, &[ch]).await.map(Json)
}

fn limit(s: &AppState, me: UserId) -> AppResult<()> {
    if s.limits.search_user.hit(&me.to_string()) {
        Ok(())
    } else {
        Err(AppError::TooManyRequests(None))
    }
}

async fn run(
    s: &AppState,
    me: UserId,
    search: &Search,
    channels: &[Channel],
) -> AppResult<SearchResults> {
    let channels: Vec<String> = channels
        .iter()
        .map(|c| c.id.to_string())
        .collect();
    if channels.is_empty() {
        return Ok(SearchResults {
            hits: vec![],
            next_cursor: None,
            total: search.cursor.is_none().then_some(0),
        });
    }
    let cols = COLS
        .split(", ")
        .map(|c| format!("m.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut q = QueryBuilder::new(format!("SELECT {cols}"));
    search.push_from_where(&mut q, &channels);
    // ULIDs sort by time: newest first, and the cursor is a plain comparison.
    if let Some(c) = search.cursor {
        q.push(" AND m.id < ").push_bind(c.to_string());
    }
    q.push(" ORDER BY m.id DESC LIMIT ").push_bind(PAGE);
    let rows: Vec<Row> = q.build_query_as().fetch_all(&s.db).await?;
    // Read past even if every hit on the page is dropped below: the next page starts after it.
    let next_cursor = if rows.len() as i64 == PAGE {
        Some(rows[rows.len() - 1].0.parse::<MessageId>().map_err(anyhow::Error::from)?)
    } else {
        None
    };

    let total = if search.cursor.is_none() {
        let mut q = QueryBuilder::new("SELECT COUNT(*) FROM (SELECT 1");
        search.push_from_where(&mut q, &channels);
        q.push(" LIMIT ").push_bind(COUNT_CAP).push(")");
        let n: i64 = q.build_query_scalar().fetch_one(&s.db).await?;
        Some(n as u32)
    } else {
        None
    };

    let messages = fill(&s.db, Some(me), rows).await?;
    let roots = roots(s, me, &messages).await?;
    let mut hits = Vec::with_capacity(messages.len());
    for message in messages {
        let captions: Vec<String> = message
            .quote
            .iter()
            .flat_map(|q| q.lines.iter().filter_map(|l| l.caption.clone()))
            .collect();
        let shown = text::display_text(&message.content, &captions);
        let (highlights, all) = text::highlights(&shown, &search.terms);
        // The index also holds custom emoji ids as words: a hit that only matched one of those
        // has none of the words in what it shows.
        if !all && text::has_custom_emoji(&message.content) {
            continue;
        }
        hits.push(SearchHit {
            root: message.thread_root_id.and_then(|r| roots.get(&r).cloned()),
            message,
            text: shown,
            highlights,
        });
    }
    Ok(SearchResults {
        hits,
        next_cursor,
        total,
    })
}

/// The quotes and clips the page's thread replies belong to (same channel, so as readable).
async fn roots(
    s: &AppState,
    me: UserId,
    messages: &[pulse_protocol::rest::Message],
) -> AppResult<HashMap<MessageId, pulse_protocol::rest::Message>> {
    let mut ids: Vec<String> = messages
        .iter()
        .filter_map(|m| m.thread_root_id.map(|r| r.to_string()))
        .collect();
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut q = QueryBuilder::new(format!("SELECT {COLS} FROM messages WHERE id IN ("));
    let mut list = q.separated(", ");
    for id in &ids {
        list.push_bind(id.as_str());
    }
    q.push(")");
    let rows: Vec<Row> = q.build_query_as().fetch_all(&s.db).await?;
    Ok(fill(&s.db, Some(me), rows)
        .await?
        .into_iter()
        .map(|m| (m.id, m))
        .collect())
}
