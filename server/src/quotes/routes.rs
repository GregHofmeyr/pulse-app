use axum::extract::{Path, State};
use axum::routing::{post, put};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::LineId;
use pulse_protocol::ids::{ChannelId, MessageId};
use pulse_protocol::rest::{ChannelKind, EditQuoteRequest, Message, PostQuoteRequest};
use std::collections::HashMap;

use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::messages::routes::{load, own_message};
use crate::servers::routes::is_server_member;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/channels/{id}/quotes", post(post_quote))
        .route("/quotes/{message_id}", put(edit_quote))
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
        return Err(AppError::TooManyRequests);
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

    // One transaction: the message, the quote, its lines and your read point land together.
    let mut tx = crate::db::begin_write(&s.db).await?;
    // An outbox retry of a post that already landed: hand back the original (see `send`).
    if let Some(existing) =
        crate::messages::nonce::existing(&mut tx, me, req.nonce.as_deref()).await?
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
        sqlx::query(
            "INSERT INTO quote_lines (id, message_id, position, speaker_id, speaker_name, speaker_key, text, media_hash, caption)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(pulse_protocol::ids::LineId::new().to_string())
        .bind(msg_id.to_string())
        .bind(position as i64)
        .bind(l.speaker_id.map(|u| u.to_string()))
        .bind(&l.speaker_name)
        .bind(&l.speaker_key)
        .bind(&l.text)
        .bind(&l.media_hash)
        .bind(&l.caption)
        .execute(&mut *tx)
        .await?;
    }
    let moved = crate::reads::advance(&mut tx, me, id, msg_id).await?;
    tx.commit().await?;

    // A new quote has no awards or replies, so the poster's view is the same as an event's.
    let msg = load(&s.db, None, msg_id).await?.ok_or(AppError::NotFound)?;
    s.hub
        .publish(
            &s.db,
            Event::MessageCreated {
                message: msg.clone(),
                nonce: req.nonce,
            },
        )
        .await;
    if moved {
        s.hub
            .publish(
                &s.db,
                Event::ReadStateUpdated {
                    user_id: me,
                    channel_id: id,
                    last_read_message_id: Some(msg_id),
                },
            )
            .await;
    }
    Ok(Json(msg))
}

/// A line as stored, for telling what an edit changes.
type StoredLine = (String, Option<String>, Option<String>);

async fn edit_quote(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
    Json(req): Json<EditQuoteRequest>,
) -> AppResult<Json<Message>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    let msg = own_message(&s.db, me, id).await?;
    if msg.quote.is_none() {
        return Err(AppError::BadRequest("this isn't a quote".into()));
    }
    let server = channel_for(&s.db, me, msg.channel_id)
        .await?
        .server_id
        .ok_or(AppError::NotFound)?;
    if !is_server_member(&s.db, server, me).await? {
        return Err(AppError::Forbidden);
    }
    let mut lines = super::clean_lines(&req.lines)?;

    // Awards counts change with the lines, so publish in commit order (as reactions do).
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    // A delete that committed since the check wins.
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
    super::check_lines(&mut tx, server, &mut lines).await?;

    let stored: Vec<StoredLine> =
        sqlx::query_as("SELECT id, speaker_id, speaker_name FROM quote_lines WHERE message_id = ?")
            .bind(id.to_string())
            .fetch_all(&mut *tx)
            .await?;
    let stored: HashMap<LineId, (Option<String>, Option<String>)> = stored
        .into_iter()
        .map(|(line, who, name)| Ok((line.parse().map_err(anyhow::Error::from)?, (who, name))))
        .collect::<AppResult<_>>()?;
    let mut kept = std::collections::HashSet::new();
    for l in &lines {
        if let Some(line) = l.id
            && (!stored.contains_key(&line) || !kept.insert(line))
        {
            return Err(AppError::BadRequest(
                "a line id isn't one of this quote's lines".into(),
            ));
        }
    }
    let speaker_changed = |l: &super::CleanLine, old: &(Option<String>, Option<String>)| {
        (l.speaker_id.map(|u| u.to_string()), l.speaker_name.clone()) != *old
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
            None => {
                sqlx::query(
                    "INSERT INTO quote_lines (id, message_id, position, speaker_id, speaker_name, speaker_key, text, media_hash, caption)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(LineId::new().to_string())
                .bind(id.to_string())
                .bind(position as i64)
                .bind(speaker_id)
                .bind(speaker_name)
                .bind(speaker_key)
                .bind(&l.text)
                .bind(&l.media_hash)
                .bind(&l.caption)
                .execute(&mut *tx)
                .await?;
            }
        }
    }
    sqlx::query("UPDATE messages SET content = ?, edited_at = ? WHERE id = ?")
        .bind(super::render_content(&lines))
        .bind(now())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    let event = load(&s.db, None, id).await?.ok_or(AppError::NotFound)?;
    s.hub
        .publish(&s.db, Event::MessageUpdated { message: event })
        .await;
    load(&s.db, Some(me), id)
        .await?
        .map(Json)
        .ok_or(AppError::NotFound)
}
