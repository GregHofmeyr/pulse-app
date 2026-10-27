use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId};
use pulse_protocol::rest::{ChannelKind, Message, PostQuoteRequest};

use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::messages::routes::load;
use crate::servers::routes::is_server_member;

/// Client nonces are UUIDs (36 bytes); anything much longer isn't one of ours.
const MAX_NONCE: usize = 64;

pub fn router() -> Router<AppState> {
    Router::new().route("/channels/{id}/quotes", post(post_quote))
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
    if req
        .nonce
        .as_ref()
        .is_some_and(|n| n.is_empty() || n.len() > MAX_NONCE)
    {
        return Err(AppError::BadRequest(format!(
            "nonce must be 1-{MAX_NONCE} bytes"
        )));
    }
    let lines = super::validate_lines(&s.db, server, &req.lines).await?;
    let day = posted_on(req.today.as_deref());
    let msg_id = MessageId::new();

    // One transaction: the message, the quote, its lines and your read point land together.
    let mut tx = crate::db::begin_write(&s.db).await?;
    // An outbox retry of a post that already landed: hand back the original (see `send`).
    if let Some(nonce) = &req.nonce
        && let Some(existing) = sqlx::query_scalar::<_, String>(
            "SELECT id FROM messages WHERE author_id = ? AND nonce = ?",
        )
        .bind(me.to_string())
        .bind(nonce)
        .fetch_optional(&mut *tx)
        .await?
    {
        tx.rollback().await?;
        let mut m = [load(&s.db, existing.parse().map_err(anyhow::Error::from)?)
            .await?
            .ok_or(AppError::NotFound)?];
        crate::quotes::attach(&s.db, Some(me), &mut m).await?;
        let [m] = m;
        return Ok(Json(m));
    }
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

    // Events serve nobody in particular; the reply is for the poster.
    let msg = load(&s.db, msg_id).await?.ok_or(AppError::NotFound)?;
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
    let mut mine = [msg];
    super::attach(&s.db, Some(me), &mut mine).await?;
    let [msg] = mine;
    Ok(Json(msg))
}
