use axum::extract::{Path, Query, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use futures::TryStreamExt;
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, LineId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{
    ChannelKind, EditQuoteRequest, Leaderboard, LeaderboardPeriod, LeaderboardWin, Message,
    MessageKind, PostQuoteRequest, QuoteSpeaker, RankedQuote, SpeakerCount,
};
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};

use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::messages::routes::{COLS, Row, fill, load, own_message};
use crate::servers::routes::is_server_member;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/channels/{id}/quotes", post(post_quote))
        .route("/quotes/{message_id}", put(edit_quote))
        .route("/servers/{id}/quotes/leaderboard", get(leaderboard))
        .route("/servers/{id}/quotes/search", get(search))
        .merge(super::awards::router())
        .merge(super::links::router())
}

/// The day a quote counts for: the client's own date if it is within a day of UTC's, else UTC's.
fn posted_on(today: Option<&str>) -> chrono::NaiveDate {
    let utc = chrono::Utc::now().date_naive();
    today
        .and_then(|t| chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d").ok())
        .filter(|d| (*d - utc).num_days().abs() <= 1)
        .unwrap_or(utc)
}

async fn post_quote(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<PostQuoteRequest>,
) -> AppResult<Json<Message>> {
    if !s.limits.send_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    crate::media::writes_ok(&s.cfg)?;
    let ch = channel_for(&s.db, me, id).await?;
    let server = match ch.server_id {
        Some(server) if ch.kind == ChannelKind::Quotes => server,
        _ => return Err(AppError::BadRequest("this isn't a quotes channel".into())),
    };
    if !is_server_member(&s.db, server, me).await? {
        return Err(AppError::Forbidden);
    }
    crate::messages::nonce::check(req.nonce.as_deref())?;
    let mut lines = super::clean_lines(&req.lines)?;
    let day = posted_on(req.today.as_deref());
    let msg_id = MessageId::new();

    // The read-state event carries an absolute mention count: publish in commit order.
    let _order = s.hub.in_order().await;
    // One transaction: the message, the quote, its lines and your read point land together.
    let mut tx = crate::db::begin_write(&s.db).await?;
    crate::access::channel_live_in(&mut tx, id).await?; // channel deleted since (#109)
    // An outbox retry of a post that already landed: hand back the original (see `send`).
    if let Some(existing) =
        crate::messages::nonce::existing(&mut tx, me, id, req.nonce.as_deref()).await?
    {
        tx.rollback().await?;
        return load(&s.db, Some(me), existing)
            .await?
            .map(Json)
            .ok_or(AppError::NotFound);
    }
    super::check_lines(&mut tx, server, &mut lines).await?;
    sqlx::query("INSERT INTO messages (id, channel_id, author_id, kind, content, created_at, nonce) VALUES (?, ?, ?, 'normal', ?, ?, ?)")
        .bind(msg_id.to_string())
        .bind(id.to_string())
        .bind(me.to_string())
        .bind(super::render_content(&lines))
        .bind(now())
        .bind(&req.nonce)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO quotes (message_id, posted_on, year) VALUES (?, ?, ?)")
        .bind(msg_id.to_string())
        .bind(day.to_string())
        .bind(chrono::Datelike::year(&day))
        .execute(&mut *tx)
        .await?;
    for (position, l) in lines.iter().enumerate() {
        super::insert_line(&mut tx, msg_id, position, l).await?;
    }
    let speakers = super::speakers_now(&mut tx, server, &lines).await?;
    let check = crate::badges::hooks::quoted(
        &mut tx,
        me,
        &speakers,
        s.cfg.quote_clock.unwrap_or_else(chrono::Utc::now),
    )
    .await?;
    let moved = crate::reads::advance(&mut tx, me, id, msg_id).await?;
    tx.commit().await?;

    let msg = reread(&s, msg_id).await;
    if let Some(message) = msg.clone() {
        s.hub
            .publish(
                &s.db,
                Event::MessageCreated {
                    message,
                    nonce: req.nonce,
                },
            )
            .await;
    }
    if moved
        && let Some((unread, mentions)) = crate::reads::counts_after_commit(&s.db, me, id).await
    {
        s.hub
            .publish(
                &s.db,
                Event::ReadStateUpdated {
                    user_id: me,
                    channel_id: id,
                    last_read_message_id: Some(msg_id),
                    mentions: Some(mentions),
                    unread: Some(unread),
                },
            )
            .await;
    }
    crate::badges::engine::after(&s, &check).await;
    // A new quote has no awards or replies, so the poster's view is the same as an event's.
    match msg {
        Some(msg) => Ok(Json(msg)),
        None => answer(&s, me, msg_id, None).await,
    }
}

/// A quote as events show it, read back after its change committed. The change has landed, so a
/// failed read is logged and gives `None` (nothing to publish) rather than failing the request.
async fn reread(s: &AppState, id: MessageId) -> Option<Message> {
    match load(&s.db, None, id).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = ?e, %id, "couldn't read back a committed quote to publish");
            None
        }
    }
}

/// The answer to a committed change: the quote as `me` sees it, or, if that read fails, as the
/// event showed it (`event`). Only when neither could be read is there nothing to answer with.
async fn answer(
    s: &AppState,
    me: UserId,
    id: MessageId,
    event: Option<Message>,
) -> AppResult<Json<Message>> {
    match load(&s.db, Some(me), id).await {
        Ok(m) => m.map(Json).ok_or(AppError::NotFound),
        Err(e) => match event {
            Some(m) => {
                tracing::error!(error = ?e, %id, "couldn't read back a committed quote; answering with the event's view");
                Ok(Json(m))
            }
            None => Err(e),
        },
    }
}

/// A line as stored, for telling what an edit changes.
type StoredLine = (String, Option<String>, Option<String>);

/// Who a stored line names: the member, or the plain name's `name_key`.
type Speaker = (Option<String>, Option<String>);

async fn edit_quote(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
    Json(req): Json<EditQuoteRequest>,
) -> AppResult<Json<Message>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let (msg, ch) = own_message(&s.db, me, id).await?;
    if msg.quote.is_none() {
        return Err(AppError::BadRequest("this isn't a quote".into()));
    }
    let server = ch.server_id.ok_or(AppError::NotFound)?;
    if !is_server_member(&s.db, server, me).await? {
        return Err(AppError::Forbidden);
    }
    // An edit rewrites the lines and can add up to nine more.
    crate::media::writes_ok(&s.cfg)?;
    let mut lines = super::clean_lines(&req.lines)?;

    // Awards counts change with the lines, so publish in commit order (as reactions do).
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    // A delete (of the quote, or its channel: #109) that committed since the check wins.
    crate::access::channel_live_in(&mut tx, ch.id).await?;
    let live: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM messages WHERE id = ? AND author_id = ? AND deleted_at IS NULL",
    )
    .bind(id.to_string())
    .bind(me.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    if live.is_none() {
        return Err(AppError::NotFound);
    }
    // A vote whose time ran out is over even if no sweep has ended it yet: end it first, so its
    // card shows the quote as it was voted on (and its speakers are no longer locked).
    let overdue = crate::qoty::ending::end_if_overdue(&mut tx, server).await?;
    super::check_lines(&mut tx, server, &mut lines).await?;

    let stored: Vec<StoredLine> =
        sqlx::query_as("SELECT id, speaker_id, speaker_key FROM quote_lines WHERE message_id = ?")
            .bind(id.to_string())
            .fetch_all(&mut *tx)
            .await?;
    let stored: HashMap<LineId, Speaker> = stored
        .into_iter()
        .map(|(line, who, name)| Ok((line.parse().map_err(anyhow::Error::from)?, (who, name))))
        .collect::<AppResult<_>>()?;
    let mut kept = HashSet::new();
    for l in &lines {
        if let Some(line) = l.id
            && (!stored.contains_key(&line) || !kept.insert(line))
        {
            return Err(AppError::BadRequest(
                "a line id isn't one of this quote's lines".into(),
            ));
        }
    }
    // A plain name changes only when its key does: a recased or respaced name is the same
    // person, so the line keeps its awards.
    let speaker_changed = |l: &super::CleanLine, old: &Speaker| {
        (l.speaker_id.map(|u| u.to_string()), l.speaker_key.clone()) != *old
    };
    let changes_speakers = kept.len() != stored.len()
        || lines.iter().any(|l| match l.id {
            None => true,
            Some(line) => speaker_changed(l, &stored[&line]),
        });
    if changes_speakers && super::speakers_locked(&mut tx, id).await? {
        return Err(AppError::Conflict(
            "this quote is up for Quote of the Year, so who said what is locked".into(),
        ));
    }

    for line in stored.keys().filter(|line| !kept.contains(line)) {
        sqlx::query("DELETE FROM quote_lines WHERE id = ?")
            .bind(line.to_string())
            .execute(&mut *tx)
            .await?;
    }
    for (position, l) in lines.iter().enumerate() {
        let (speaker_id, speaker_name, speaker_key) = (
            l.speaker_id.map(|u| u.to_string()),
            &l.speaker_name,
            &l.speaker_key,
        );
        match l.id {
            Some(line) => {
                // A different speaker is a different claim: the awards for the old one go.
                if speaker_changed(l, &stored[&line]) {
                    sqlx::query("DELETE FROM quote_awards WHERE line_id = ?")
                        .bind(line.to_string())
                        .execute(&mut *tx)
                        .await?;
                }
                sqlx::query(
                    "UPDATE quote_lines SET position = ?, speaker_id = ?, speaker_name = ?, speaker_key = ?,
                         text = ?, media_hash = ?, caption = ?
                     WHERE id = ?",
                )
                .bind(position as i64)
                .bind(speaker_id)
                .bind(speaker_name)
                .bind(speaker_key)
                .bind(&l.text)
                .bind(&l.media_hash)
                .bind(&l.caption)
                .bind(line.to_string())
                .execute(&mut *tx)
                .await?;
            }
            None => super::insert_line(&mut tx, id, position, l).await?,
        }
    }
    sqlx::query("UPDATE messages SET content = ?, edited_at = ? WHERE id = ?")
        .bind(super::render_content(&lines))
        .bind(now())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    if let Some(ended) = overdue {
        crate::qoty::ending::announce(&s, ended).await;
    }

    let event = reread(&s, id).await;
    if let Some(message) = event.clone() {
        s.hub
            .publish(&s.db, Event::MessageUpdated { message })
            .await;
    }
    answer(&s, me, id, event).await
}

/// Each top list and `most_*` list holds this many.
const BOARD_LEN: usize = 10;
/// Years a leaderboard can be asked for.
const YEARS: std::ops::RangeInclusive<i32> = 2000..=9999;
/// A search is 1..=this many characters, and returns at most [`SEARCH_MAX`] quotes.
const SEARCH_CHARS: usize = 100;
const SEARCH_MAX: u32 = 20;

#[derive(serde::Deserialize)]
struct BoardQuery {
    year: Option<i32>,
    /// The window for `top` and `most_quoted`. Without one, the older shape: `top_year` and
    /// `top_all_time`, and `most_quoted` over all time.
    period: Option<LeaderboardPeriod>,
    /// The caller's local `YYYY-MM-DD`, which the period counts back from (as a posted quote's
    /// `today`: within a day of UTC's, else UTC's).
    today: Option<String>,
}

/// The first day of `period` around `day`, as `quotes.posted_on` stores days; `None` for all time.
/// A week starts on Monday.
fn period_start(period: LeaderboardPeriod, day: chrono::NaiveDate) -> Option<String> {
    use chrono::Datelike;
    let start = match period {
        LeaderboardPeriod::Week => {
            day - chrono::Duration::days(i64::from(day.weekday().num_days_from_monday()))
        }
        LeaderboardPeriod::Month => day.with_day(1)?,
        LeaderboardPeriod::Year => chrono::NaiveDate::from_ymd_opt(day.year(), 1, 1)?,
        LeaderboardPeriod::All => return None,
    };
    Some(start.to_string())
}

/// `me` must be a member of `server` (404 for no such server, 403 for not a member).
async fn members_only(s: &AppState, server: ServerId, me: UserId) -> AppResult<()> {
    crate::servers::routes::require_member(&s.db, server, me).await
}

/// The best live quotes of `server` (of one year, or posted since a day, or all): see
/// [`super::ranked`]. Returns (message id, score).
async fn top_quotes(
    db: &SqlitePool,
    server: ServerId,
    year: Option<i32>,
    since: Option<&str>,
) -> AppResult<Vec<(String, i64)>> {
    super::ranked(db, server, year, since, false, Some(BOARD_LEN)).await
}

/// Quotes per person over the live quotes of `server` posted since `since` (all, for `None`),
/// most first. A quote counts once for each person in it, however many of its lines they say and
/// whether a line names them as a member or by name (see [`super::awards::Names::person`]).
async fn most_quoted(
    db: &SqlitePool,
    server: ServerId,
    since: Option<&str>,
) -> AppResult<Vec<SpeakerCount>> {
    type LineRow = (Option<String>, Option<String>, Option<String>, String);
    let rows: Vec<LineRow> = sqlx::query_as(
        "SELECT l.speaker_id, l.speaker_name, l.speaker_key, l.message_id
         FROM quote_lines l
         JOIN quotes q ON q.message_id = l.message_id
         JOIN messages msg ON msg.id = l.message_id AND msg.deleted_at IS NULL
         JOIN channels c ON c.id = msg.channel_id AND c.deleted_at IS NULL
         WHERE c.server_id = ? AND (? IS NULL OR q.posted_on >= ?)
         ORDER BY msg.id, l.position",
    )
    .bind(server.to_string())
    .bind(since)
    .bind(since)
    .fetch_all(db)
    .await?;
    let mut seen = HashSet::new();
    let mut out = super::awards::ranked(
        super::awards::with_speakers(db, server, rows)
            .await?
            .into_iter()
            .filter(|(who, key, _, msg)| {
                let person = match who {
                    Some(u) => format!("u:{u}"),
                    None => format!("n:{}", key.as_deref().unwrap_or_default()),
                };
                seen.insert((msg.clone(), person))
            })
            .map(|(who, key, name, _)| (who, key, name)),
    );
    out.truncate(BOARD_LEN);
    Ok(out)
}

async fn leaderboard(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(server): Path<ServerId>,
    Query(q): Query<BoardQuery>,
) -> AppResult<Json<Leaderboard>> {
    members_only(&s, server, me).await?;
    let year = q
        .year
        .unwrap_or_else(|| chrono::Datelike::year(&chrono::Utc::now()));
    if !YEARS.contains(&year) {
        return Err(AppError::BadRequest("that isn't a year we can show".into()));
    }
    let since = q
        .period
        .and_then(|p| period_start(p, posted_on(q.today.as_deref())));
    // With a period, one ranked list for it; without, the older pair.
    let (top, top_year, top_all_time) = match q.period {
        Some(_) => (
            top_quotes(&s.db, server, None, since.as_deref()).await?,
            vec![],
            vec![],
        ),
        None => (
            vec![],
            top_quotes(&s.db, server, Some(year), None).await?,
            top_quotes(&s.db, server, None, None).await?,
        ),
    };
    let ids = |v: &[(String, i64)]| v.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
    let (top_year, top_all_time) = (ids(&top_year), ids(&top_all_time));

    // Winners: newest year first, a tie's co-winners newest quote first. Deleted quotes stay (the
    // win is history); a quote in a soft-deleted channel is hidden like the rest of that channel,
    // until it is restored or purged. Once purged (#110) its message row is gone, so the NOT EXISTS
    // below no longer hides it and it shows as a deleted quote, with `voters` keeping the votes
    // that went with it.
    let wins: Vec<(i64, String, i64, String, Option<String>)> = sqlx::query_as(
        "SELECT e.year, w.message_id, w.votes, e.id, w.voters
         FROM qoty_winners w JOIN qoty_events e ON e.id = w.event_id
         WHERE e.server_id = ?
           AND NOT EXISTS (SELECT 1 FROM messages m JOIN channels c ON c.id = m.channel_id
                           WHERE m.id = w.message_id AND c.deleted_at IS NOT NULL)
         ORDER BY e.year DESC, w.message_id DESC",
    )
    .bind(server.to_string())
    .fetch_all(&s.db)
    .await?;
    let mut voters: HashMap<(String, String), Vec<UserId>> = HashMap::new();
    let votes: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT v.event_id, v.message_id, v.user_id
         FROM qoty_votes v JOIN qoty_winners w ON w.event_id = v.event_id AND w.message_id = v.message_id
         JOIN qoty_events e ON e.id = v.event_id
         WHERE e.server_id = ?
         ORDER BY v.rowid",
    )
    .bind(server.to_string())
    .fetch_all(&s.db)
    .await?;
    for (event, message, user) in votes {
        voters
            .entry((event, message))
            .or_default()
            .push(super::parse_id(&user)?);
    }

    // Who said each winner's lines when its vote ended, in line order.
    let mut speakers: HashMap<(String, String), Vec<QuoteSpeaker>> = HashMap::new();
    let said: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT ws.event_id, ws.message_id, ws.user_id, ws.name
         FROM qoty_winner_speakers ws JOIN qoty_events e ON e.id = ws.event_id
         WHERE e.server_id = ?
         ORDER BY ws.event_id, ws.message_id, ws.position",
    )
    .bind(server.to_string())
    .fetch_all(&s.db)
    .await?;
    for (event, message, user, name) in said {
        speakers
            .entry((event, message))
            .or_default()
            .push(QuoteSpeaker {
                user_id: super::parse_opt(user.as_deref())?,
                name,
            });
    }

    // One batched load for every message on the board, as the caller sees them.
    let mut wanted: Vec<&String> = top_year
        .iter()
        .chain(&top_all_time)
        .chain(top.iter().map(|t| &t.0))
        .chain(wins.iter().map(|w| &w.1))
        .collect();
    wanted.sort();
    wanted.dedup();
    let mut messages: HashMap<String, Message> = HashMap::new();
    if !wanted.is_empty() {
        let mut qb = sqlx::QueryBuilder::new(format!("SELECT {COLS} FROM messages WHERE id IN ("));
        let mut list = qb.separated(", ");
        for id in wanted {
            list.push_bind(id);
        }
        qb.push(")");
        let rows: Vec<Row> = qb.build_query_as().fetch_all(&s.db).await?;
        for m in fill(&s.db, Some(me), rows).await? {
            messages.insert(m.id.to_string(), m);
        }
    }
    let pick = |ids: &[String]| -> Vec<Message> {
        ids.iter()
            .filter_map(|id| messages.get(id).cloned())
            .collect()
    };
    let mut winners = Vec::with_capacity(wins.len());
    for (year, message, votes, event, kept_voters) in &wins {
        // A quote purged with its channel (#110) reads as a deleted one: the win stays (spec §3.1).
        let msg = match messages.get(message) {
            Some(m) => m.clone(),
            None => purged_winner(message)?,
        };
        let voters = match kept_voters {
            Some(json) => serde_json::from_str::<Vec<String>>(json)
                .map_err(anyhow::Error::from)?
                .iter()
                .map(|u| super::parse_id(u))
                .collect::<AppResult<Vec<UserId>>>()?,
            None => voters
                .remove(&(event.clone(), message.clone()))
                .unwrap_or_default(),
        };
        winners.push(LeaderboardWin {
            year: *year as i32,
            message: msg,
            votes: *votes as u32,
            voters,
            speakers: speakers
                .remove(&(event.clone(), message.clone()))
                .unwrap_or_default(),
        });
    }

    let mut most_awarded = super::award_totals(&s.db, server).await?;
    most_awarded.truncate(BOARD_LEN);
    Ok(Json(Leaderboard {
        winners,
        top_year: pick(&top_year),
        top_all_time: pick(&top_all_time),
        top: top
            .iter()
            .filter_map(|(id, score)| {
                Some(RankedQuote {
                    message: messages.get(id)?.clone(),
                    score: *score as u32,
                })
            })
            .collect(),
        most_quoted: most_quoted(&s.db, server, since.as_deref()).await?,
        most_awarded,
    }))
}

/// The stand-in for a winning quote whose message was purged with its channel (#110): a deleted
/// message with nothing in it (clients show "This quote was deleted." and offer nothing to open;
/// its channel is gone, so it names none).
fn purged_winner(id: &str) -> AppResult<Message> {
    Ok(Message {
        id: super::parse_id(id)?,
        channel_id: ChannelId::floor(0),
        author_id: None,
        kind: MessageKind::Normal,
        content: String::new(),
        reply_to_id: None,
        created_at: String::new(),
        edited_at: None,
        deleted: true,
        mentions: vec![],
        reactions: vec![],
        thread_root_id: None,
        quote: None,
        qoty_card: None,
        clip: None,
        embeds: vec![],
        embeds_hidden: false,
        call: None,
        pin: None,
        pin_line: None,
        wheel: None,
        poll: None,
        poll_ended: None,
        stage_share_line: None,
        gif: None,
        attachments: vec![],
        caption_is_fallback: false,
        achievement: None,
        deadlock: None,
        plan: None,
        plan_reminder: None,
        sports: None,
        f1: None,
        forwarded: None,
        group_line: None,
    })
}

#[derive(serde::Deserialize)]
struct SearchQuery {
    q: Option<String>,
    limit: Option<u32>,
}

async fn search(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(server): Path<ServerId>,
    Query(q): Query<SearchQuery>,
) -> AppResult<Json<Vec<Message>>> {
    members_only(&s, server, me).await?;
    let needle = q.q.as_deref().unwrap_or_default().trim();
    if needle.is_empty() || needle.chars().count() > SEARCH_CHARS {
        return Err(AppError::BadRequest(format!(
            "search for 1-{SEARCH_CHARS} characters"
        )));
    }
    let needle = needle.to_lowercase();
    let limit = q.limit.unwrap_or(SEARCH_MAX).clamp(1, SEARCH_MAX) as usize;

    // Every line of the server's live quotes, newest quote first, with the names it can be found
    // by: each speaker's username, display name and nickname here, joined by a unit separator (no
    // name can hold one: names have no control characters). Matching is in Rust so case folds for
    // all of Unicode (SQLite's LIKE does ASCII only); we stop reading at the `limit`th quote that
    // matches.
    let mut lines = sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT l.message_id, l.text, l.caption, l.speaker_name,
                u.username || char(31) || u.display_name || char(31) || COALESCE(um.nickname, ''),
                lu.username || char(31) || lu.display_name || char(31) || COALESCE(lm.nickname, '')
         FROM quote_lines l
         JOIN messages msg ON msg.id = l.message_id AND msg.deleted_at IS NULL
         JOIN channels c ON c.id = msg.channel_id AND c.deleted_at IS NULL
         LEFT JOIN users u ON u.id = l.speaker_id
         LEFT JOIN server_members um ON um.server_id = c.server_id AND um.user_id = u.id
         LEFT JOIN speaker_links k ON k.server_id = c.server_id AND k.name_key = l.speaker_key
         LEFT JOIN users lu ON lu.id = k.user_id
         LEFT JOIN server_members lm ON lm.server_id = c.server_id AND lm.user_id = lu.id
         WHERE c.server_id = ?
         ORDER BY l.message_id DESC",
    )
    .bind(server.to_string())
    .fetch(&s.db);
    let mut found: Vec<String> = Vec::new();
    while let Some((message, a, b, c, d, e)) = lines.try_next().await? {
        if found.last() == Some(&message) {
            continue;
        }
        if [a, b, c, d, e]
            .iter()
            .flatten()
            .any(|f| f.to_lowercase().contains(&needle))
        {
            found.push(message);
            if found.len() == limit {
                break;
            }
        }
    }
    drop(lines);
    if found.is_empty() {
        return Ok(Json(vec![]));
    }
    let mut qb = sqlx::QueryBuilder::new(format!("SELECT {COLS} FROM messages WHERE id IN ("));
    let mut list = qb.separated(", ");
    for id in &found {
        list.push_bind(id);
    }
    qb.push(") ORDER BY id DESC");
    let rows: Vec<Row> = qb.build_query_as().fetch_all(&s.db).await?;
    Ok(Json(fill(&s.db, Some(me), rows).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn periods_start_on_monday_the_first_and_new_year() {
        use LeaderboardPeriod::*;
        let start = |p, day| period_start(p, d(day));
        // Wednesday 18 November 2026.
        assert_eq!(start(Week, "2026-11-18").as_deref(), Some("2026-11-16"));
        assert_eq!(start(Month, "2026-11-18").as_deref(), Some("2026-11-01"));
        assert_eq!(start(Year, "2026-11-18").as_deref(), Some("2026-01-01"));
        assert_eq!(start(All, "2026-11-18"), None);
        // A Monday is its own week's start; a Sunday belongs to the week before it.
        assert_eq!(start(Week, "2026-11-16").as_deref(), Some("2026-11-16"));
        assert_eq!(start(Week, "2026-11-15").as_deref(), Some("2026-11-09"));
        // A week can start in last month, or last year.
        assert_eq!(start(Week, "2026-12-02").as_deref(), Some("2026-11-30"));
        assert_eq!(start(Week, "2027-01-01").as_deref(), Some("2026-12-28"));
        assert_eq!(start(Month, "2027-01-01").as_deref(), Some("2027-01-01"));
    }
}
