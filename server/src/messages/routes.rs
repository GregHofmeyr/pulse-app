use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, patch};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{
    ChannelKind, EditMessageRequest, Message, MessageKind, SendMessageRequest,
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::servers::routes::is_server_member;

use super::{mentions, reactions};

const MAX_LEN: usize = 4000;
/// Client nonces are UUIDs (36 bytes); anything much longer isn't one of ours.
const MAX_NONCE: usize = 64;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/channels/{id}/messages", get(list).post(send))
        .route("/messages/{id}", patch(edit).delete(remove))
}

type Row = (
    String,
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
);
const COLS: &str =
    "id, channel_id, author_id, kind, content, reply_to_id, created_at, edited_at, deleted_at";

fn parse<T: std::str::FromStr>(s: &str) -> AppResult<T>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    s.parse::<T>().map_err(|e| AppError::Internal(e.into()))
}

fn from_row(r: Row) -> AppResult<Message> {
    let (id, channel_id, author_id, kind, content, reply_to_id, created_at, edited_at, deleted_at) =
        r;
    Ok(Message {
        id: parse(&id)?,
        channel_id: parse(&channel_id)?,
        author_id: author_id.as_deref().map(parse).transpose()?,
        kind: if kind == "system" {
            MessageKind::System
        } else {
            MessageKind::Normal
        },
        // Backstop: a deleted message never exposes content, whatever is stored.
        content: if deleted_at.is_some() {
            String::new()
        } else {
            content
        },
        reply_to_id: reply_to_id.as_deref().map(parse).transpose()?,
        created_at,
        edited_at,
        deleted: deleted_at.is_some(),
        mentions: vec![],
        reactions: vec![],
        thread_root_id: None,
        quote: None,
        qoty_card: None,
    })
}

pub(crate) async fn load(db: &SqlitePool, id: MessageId) -> AppResult<Option<Message>> {
    let row: Option<Row> = sqlx::query_as(&format!("SELECT {COLS} FROM messages WHERE id = ?"))
        .bind(id.to_string())
        .fetch_optional(db)
        .await?;
    let Some(mut m) = row.map(from_row).transpose()? else {
        return Ok(None);
    };
    m.mentions = mentions::for_messages(db, &[m.id])
        .await?
        .remove(&m.id)
        .unwrap_or_default();
    if !m.deleted {
        m.reactions = reactions::summaries(db, None, &[m.id])
            .await?
            .remove(&m.id)
            .unwrap_or_default();
    }
    let mut one = [m];
    crate::quotes::attach(db, None, &mut one).await?;
    let [m] = one;
    Ok(Some(m))
}

fn clean(content: &str) -> AppResult<String> {
    let c = content.trim();
    if c.is_empty() || c.chars().count() > MAX_LEN {
        return Err(AppError::BadRequest(format!(
            "message must be 1-{MAX_LEN} characters"
        )));
    }
    Ok(c.to_string())
}

#[derive(Deserialize)]
struct Page {
    before: Option<MessageId>,
    limit: Option<i64>,
}

async fn list(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Query(p): Query<Page>,
) -> AppResult<Json<Vec<Message>>> {
    channel_for(&s.db, me, id).await?;
    let limit = p.limit.unwrap_or(50).clamp(1, 100);
    // ULIDs sort by time, so "before" is a plain string comparison. "~" sorts after every ULID.
    let before = p
        .before
        .map(|b| b.to_string())
        .unwrap_or_else(|| "~".into());
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM messages WHERE channel_id = ? AND id < ? ORDER BY id DESC LIMIT ?"
    ))
    .bind(id.to_string())
    .bind(before)
    .bind(limit)
    .fetch_all(&s.db)
    .await?;
    let mut page: Vec<Message> = rows.into_iter().map(from_row).collect::<AppResult<_>>()?;
    let ids: Vec<MessageId> = page.iter().map(|m| m.id).collect();
    let mut map = mentions::for_messages(&s.db, &ids).await?;
    // A deleted message shows no reactions, like no content.
    let live: Vec<MessageId> = page.iter().filter(|m| !m.deleted).map(|m| m.id).collect();
    let mut reacted = reactions::summaries(&s.db, Some(me), &live).await?;
    for m in &mut page {
        m.mentions = map.remove(&m.id).unwrap_or_default();
        m.reactions = reacted.remove(&m.id).unwrap_or_default();
    }
    crate::quotes::attach(&s.db, Some(me), &mut page).await?;
    Ok(Json(page))
}

async fn send(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<SendMessageRequest>,
) -> AppResult<Json<Message>> {
    if !s.limits.send_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    crate::media::writes_ok(&s.cfg)?;
    let ch = channel_for(&s.db, me, id).await?;
    match (ch.kind, ch.server_id) {
        (ChannelKind::Voice, _) => {
            return Err(AppError::BadRequest(
                "voice channels have no messages".into(),
            ));
        }
        (kind, Some(server))
            if kind.is_server_chat() && !is_server_member(&s.db, server, me).await? =>
        {
            return Err(AppError::Forbidden);
        }
        _ => {}
    }
    // Threads (replies to a quote) arrive with the quotes routes; until then none can be started,
    // and a quotes channel holds quotes only.
    if req.thread_root_id.is_some() {
        return Err(AppError::BadRequest(
            "replies to quotes aren't supported yet".into(),
        ));
    }
    if ch.kind == ChannelKind::Quotes {
        return Err(AppError::BadRequest(
            "only quotes can be posted here".into(),
        ));
    }
    let content = clean(&req.content)?;
    if req
        .nonce
        .as_ref()
        .is_some_and(|n| n.is_empty() || n.len() > MAX_NONCE)
    {
        return Err(AppError::BadRequest(format!(
            "nonce must be 1-{MAX_NONCE} bytes"
        )));
    }
    let mut mentioned = mentions::resolve(&s.db, &ch, me, &mentions::parse(&content)).await?;
    if let Some(r) = req.reply_to_id {
        // Same-channel only; a reply target elsewhere (including private) is indistinguishable from missing.
        match load(&s.db, r).await? {
            Some(target) if target.channel_id == id => {
                // A reply pings whoever it replies to, as long as they can still see the chat.
                if let Some(author) = target.author_id
                    && author != me
                    && !mentioned.contains(&author)
                    && mentions::can_see(&s.db, &ch, author).await?
                {
                    mentioned.push(author);
                }
            }
            _ => {
                return Err(AppError::BadRequest(
                    "reply target not in this channel".into(),
                ));
            }
        }
    }
    let msg = Message {
        id: MessageId::new(),
        channel_id: id,
        author_id: Some(me),
        kind: MessageKind::Normal,
        content,
        reply_to_id: req.reply_to_id,
        created_at: now(),
        edited_at: None,
        deleted: false,
        mentions: mentioned.clone(),
        reactions: vec![],
        thread_root_id: None,
        quote: None,
        qoty_card: None,
    };
    // One transaction: the message, its mentions and your read point land together or not at all.
    let mut tx = crate::db::begin_write(&s.db).await?;
    // An outbox retry of a send that already landed (e.g. the proxy said 502 after we committed):
    // hand back the original instead of posting twice. Under the write lock, check-then-insert is
    // atomic; the unique index is the backstop.
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
        // Not re-published: the first send already fanned it out, and a second MessageCreated
        // would double-count unreads on other clients.
        return load(&s.db, parse(&existing)?)
            .await?
            .map(Json)
            .ok_or(AppError::NotFound);
    }
    sqlx::query("INSERT INTO messages (id, channel_id, author_id, kind, content, reply_to_id, created_at, nonce) VALUES (?, ?, ?, 'normal', ?, ?, ?, ?)")
        .bind(msg.id.to_string())
        .bind(id.to_string())
        .bind(me.to_string())
        .bind(&msg.content)
        .bind(msg.reply_to_id.map(|r| r.to_string()))
        .bind(&msg.created_at)
        .bind(&req.nonce)
        .execute(&mut *tx)
        .await?;
    mentions::store(&mut tx, msg.id, &mentioned).await?;
    let used = reactions::live_keys(&mut tx, me, reactions::used_in(&msg.content)).await?;
    crate::emoji::record_use(&mut tx, me, &used).await?;
    if ch.kind.is_private() {
        // A new message reopens closed conversations (clients un-hide on MessageCreated).
        // Server channels can't be closed, so they skip this write entirely.
        sqlx::query("DELETE FROM dm_hidden WHERE channel_id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
    }
    // Sending marks your own read point (you've obviously seen everything up to here).
    let moved = crate::reads::advance(&mut tx, me, id, msg.id).await?;
    tx.commit().await?;
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
                    last_read_message_id: Some(msg.id),
                },
            )
            .await;
    }
    if !used.is_empty() {
        let _order = s.hub.in_order().await;
        crate::emoji::publish_frequent(&s, me).await;
    }
    Ok(Json(msg))
}

/// Load a live message the caller can see and authored. Outsiders get 404, others' messages 403.
async fn own_message(db: &SqlitePool, me: UserId, id: MessageId) -> AppResult<Message> {
    let msg = load(db, id).await?.ok_or(AppError::NotFound)?;
    channel_for(db, me, msg.channel_id).await?;
    if msg.deleted {
        return Err(AppError::NotFound);
    }
    if msg.author_id != Some(me) {
        return Err(AppError::Forbidden);
    }
    Ok(msg)
}

async fn edit(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
    Json(req): Json<EditMessageRequest>,
) -> AppResult<Json<Message>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    let mut msg = own_message(&s.db, me, id).await?;
    if msg.quote.is_some() {
        return Err(AppError::BadRequest(
            "quotes can't be edited like messages".into(),
        ));
    }
    msg.content = clean(&req.content)?;
    msg.edited_at = Some(now());
    // Guarded: a delete that commits between our read and this write wins.
    let changed = sqlx::query(
        "UPDATE messages SET content = ?, edited_at = ? WHERE id = ? AND author_id = ? AND deleted_at IS NULL",
    )
    .bind(&msg.content)
    .bind(&msg.edited_at)
    .bind(id.to_string())
    .bind(me.to_string())
    .execute(&s.db)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    s.hub
        .publish(
            &s.db,
            Event::MessageUpdated {
                message: msg.clone(),
            },
        )
        .await;
    Ok(Json(msg))
}

async fn remove(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    let msg = own_message(&s.db, me, id).await?;
    let changed = sqlx::query(
        "UPDATE messages SET content = '', deleted_at = ? WHERE id = ? AND author_id = ? AND deleted_at IS NULL",
    )
    .bind(now())
    .bind(id.to_string())
    .bind(me.to_string())
    .execute(&s.db)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    s.hub
        .publish(
            &s.db,
            Event::MessageDeleted {
                channel_id: msg.channel_id,
                message_id: id,
                author_id: Some(me),
                mentions: crate::messages::mentions::for_messages(&s.db, &[id])
                    .await?
                    .remove(&id)
                    .unwrap_or_default(),
                thread_root_id: None,
            },
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}
