use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, patch};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{
    Channel, ChannelKind, EditMessageRequest, Message, MessageKind, SendMessageRequest,
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::servers::routes::is_server_member;

use super::{MAX_LEN, mentions, nonce, reactions};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/channels/{id}/messages", get(list).post(send))
        .route("/messages/{id}", patch(edit).delete(remove))
}

pub(crate) type Row = (
    String,
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);
pub(crate) const COLS: &str = "id, channel_id, author_id, kind, content, reply_to_id, created_at, edited_at, deleted_at, thread_root_id, system_payload";

fn parse<T: std::str::FromStr>(s: &str) -> AppResult<T>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    s.parse::<T>().map_err(|e| AppError::Internal(e.into()))
}

fn from_row(r: Row) -> AppResult<Message> {
    let (
        id,
        channel_id,
        author_id,
        kind,
        content,
        reply_to_id,
        created_at,
        edited_at,
        deleted_at,
        thread_root_id,
        system_payload,
    ) = r;
    let system = kind == "system";
    Ok(Message {
        id: parse(&id)?,
        channel_id: parse(&channel_id)?,
        author_id: author_id.as_deref().map(parse).transpose()?,
        kind: if system {
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
        thread_root_id: thread_root_id.as_deref().map(parse).transpose()?,
        quote: None,
        clip: None,
        qoty_card: match system_payload {
            Some(p) if system && deleted_at.is_none() => super::system::qoty_card(&p),
            _ => None,
        },
    })
}

/// `me` is whose view the quote part is (events and previews pass `None`).
pub(crate) async fn load(
    db: &SqlitePool,
    me: Option<UserId>,
    id: MessageId,
) -> AppResult<Option<Message>> {
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
    crate::quotes::attach(db, me, &mut one).await?;
    crate::clips::attach(db, me, &mut one).await?;
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
pub(super) struct Page {
    pub before: Option<MessageId>,
    pub limit: Option<i64>,
}

impl Page {
    pub(super) fn limit(&self) -> i64 {
        self.limit.unwrap_or(50).clamp(1, 100)
    }

    /// (limit, "before" bound). ULIDs sort by time, so "before" is a plain string comparison;
    /// "~" sorts after every ULID.
    pub(super) fn bounds(&self) -> (i64, String) {
        (
            self.limit(),
            self.before
                .map(|b| b.to_string())
                .unwrap_or_else(|| "~".into()),
        )
    }
}

/// Turn rows into messages with mentions, reactions and the quote part, as `me` sees them
/// (`None`: as events show them, with nothing marked as yours).
pub(crate) async fn fill(
    db: &SqlitePool,
    me: Option<UserId>,
    rows: Vec<Row>,
) -> AppResult<Vec<Message>> {
    let mut page: Vec<Message> = rows.into_iter().map(from_row).collect::<AppResult<_>>()?;
    let ids: Vec<MessageId> = page.iter().map(|m| m.id).collect();
    let mut map = mentions::for_messages(db, &ids).await?;
    // A deleted message shows no reactions, like no content.
    let live: Vec<MessageId> = page.iter().filter(|m| !m.deleted).map(|m| m.id).collect();
    let mut reacted = reactions::summaries(db, me, &live).await?;
    for m in &mut page {
        m.mentions = map.remove(&m.id).unwrap_or_default();
        m.reactions = reacted.remove(&m.id).unwrap_or_default();
    }
    crate::quotes::attach(db, me, &mut page).await?;
    crate::clips::attach(db, me, &mut page).await?;
    Ok(page)
}

/// The list's query: at most one of `before` (older than an id, the default is the newest),
/// `after` (newer than an id) and `around` (a window centred on an id).
#[derive(Deserialize)]
struct ListQuery {
    before: Option<MessageId>,
    after: Option<MessageId>,
    around: Option<MessageId>,
    limit: Option<i64>,
}

/// One bounded slice of a channel's main list, on the `messages_channel (channel_id, id)` index.
/// Replies live in their quote's thread, not in the list. `newer` takes the rows just above
/// `bound` (returned newest first, so every slice reads the same way), otherwise the rows just below.
async fn slice(
    db: &SqlitePool,
    channel: ChannelId,
    bound: &str,
    newer: bool,
    n: i64,
) -> AppResult<Vec<Row>> {
    let (cmp, order) = if newer { (">", "ASC") } else { ("<", "DESC") };
    let mut rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM messages WHERE channel_id = ? AND id {cmp} ? AND thread_root_id IS NULL ORDER BY id {order} LIMIT ?"
    ))
    .bind(channel.to_string())
    .bind(bound)
    .bind(n)
    .fetch_all(db)
    .await?;
    if newer {
        rows.reverse();
    }
    Ok(rows)
}

async fn list(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<Vec<Message>>> {
    channel_for(&s.db, me, id).await?;
    if [q.before.is_some(), q.after.is_some(), q.around.is_some()]
        .iter()
        .filter(|set| **set)
        .count()
        > 1
    {
        return Err(AppError::BadRequest(
            "use only one of before, after and around".into(),
        ));
    }
    let page = Page {
        before: q.before,
        limit: q.limit,
    };
    let limit = page.limit();
    let rows = if let Some(after) = q.after {
        slice(&s.db, id, &after.to_string(), true, limit).await?
    } else if let Some(target) = q.around {
        // The target must be a main-list message of this channel (deleted and system ones count).
        let anchor: Option<Row> = sqlx::query_as(&format!(
            "SELECT {COLS} FROM messages WHERE id = ? AND channel_id = ? AND thread_root_id IS NULL"
        ))
        .bind(target.to_string())
        .bind(id.to_string())
        .fetch_optional(&s.db)
        .await?;
        let anchor = anchor.ok_or(AppError::NotFound)?;
        let bound = target.to_string();
        // Three plain queries, no transaction: the table is append-only and deletes are soft, so a
        // concurrent write can only shift the window's edges.
        // Each side gets its share; one running out doesn't give its room to the other.
        let mut rows = slice(&s.db, id, &bound, true, limit - 1 - limit / 2).await?;
        rows.push(anchor);
        rows.extend(slice(&s.db, id, &bound, false, limit / 2).await?);
        rows
    } else {
        slice(&s.db, id, &page.bounds().1, false, limit).await?
    };
    Ok(Json(fill(&s.db, Some(me), rows).await?))
}

async fn send(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<SendMessageRequest>,
) -> AppResult<Json<Message>> {
    if !s.limits.send_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
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
    // A quotes or clips channel holds those only; the one plain message it takes is a reply in a thread.
    if let Some(root) = req.thread_root_id {
        if !super::threads::is_live_root(&s.db, root, id).await? {
            return Err(AppError::BadRequest(
                "replies go to a quote or clip in this channel".into(),
            ));
        }
    } else if ch.kind == ChannelKind::Quotes {
        return Err(AppError::BadRequest(
            "only quotes can be posted here".into(),
        ));
    } else if ch.kind == ChannelKind::Clips {
        return Err(AppError::BadRequest("only clips can be posted here".into()));
    }
    let content = clean(&req.content)?;
    nonce::check(req.nonce.as_deref())?;
    let mut mentioned = mentions::resolve(&s.db, &ch, me, &mentions::parse(&content)).await?;
    if let Some(r) = req.reply_to_id {
        // Same-channel only; a reply target elsewhere (including private) is indistinguishable from missing.
        match load(&s.db, None, r).await? {
            Some(target)
                if target.channel_id == id
                    && req.thread_root_id.is_none_or(|root| {
                        target.id == root || target.thread_root_id == Some(root)
                    }) =>
            {
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
                    "reply target not in this channel or thread".into(),
                ));
            }
        }
    }
    let mut msg = Message {
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
        thread_root_id: req.thread_root_id,
        quote: None,
        clip: None,
        qoty_card: None,
    };
    // A reply's ThreadSeen is a delta, so it must reach clients in commit order with the
    // MessageCreated of the replies around it: hold the publish order across both.
    // The same hold orders the absolute mention counts of the read-state events below with every
    // message that may mention someone (and the frequent-emoji publish at the end).
    let _order = s.hub.in_order().await;
    // One transaction: the message, its mentions and your read point land together or not at all.
    let mut tx = crate::db::begin_write(&s.db).await?;
    // Under the write lock the id is minted in commit order, so a seen point can never pass a
    // reply that hasn't committed yet.
    msg.id = MessageId::new();
    msg.created_at = now();
    // A delete of the quote may have committed since the early check: the root must still be live.
    if let Some(root) = req.thread_root_id
        && !super::threads::is_live_root(&mut *tx, root, id).await?
    {
        return Err(AppError::BadRequest(
            "replies go to a quote or clip in this channel".into(),
        ));
    }
    // An outbox retry of a send that already landed (e.g. the proxy said 502 after we committed):
    // hand back the original instead of posting twice. Under the write lock, check-then-insert is
    // atomic; the unique index is the backstop.
    if let Some(existing) = nonce::existing(&mut tx, me, id, req.nonce.as_deref()).await? {
        tx.rollback().await?;
        // Not re-published: the first send already fanned it out, and a second MessageCreated
        // would double-count unreads on other clients.
        return load(&s.db, Some(me), existing)
            .await?
            .map(Json)
            .ok_or(AppError::NotFound);
    }
    sqlx::query("INSERT INTO messages (id, channel_id, author_id, kind, content, reply_to_id, created_at, nonce, thread_root_id) VALUES (?, ?, ?, 'normal', ?, ?, ?, ?, ?)")
        .bind(msg.id.to_string())
        .bind(id.to_string())
        .bind(me.to_string())
        .bind(&msg.content)
        .bind(msg.reply_to_id.map(|r| r.to_string()))
        .bind(&msg.created_at)
        .bind(&req.nonce)
        .bind(req.thread_root_id.map(|r| r.to_string()))
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
    let mut thread_cleared = None;
    // A reply is not channel traffic: it marks your place in its thread instead.
    let moved = match req.thread_root_id {
        Some(root) => {
            thread_cleared = super::threads::mark_seen(&mut tx, me, root, msg.id).await?;
            false
        }
        None => crate::reads::advance(&mut tx, me, id, msg.id).await?,
    };
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
    // Replying moves your place in the thread (and covers earlier mentions of you): tell your
    // other sessions.
    if let (Some(root), Some(mentions_cleared)) = (req.thread_root_id, thread_cleared)
        && let Some((_, mentions)) = crate::reads::counts_after_commit(&s.db, me, id).await
    {
        s.hub
            .publish(
                &s.db,
                Event::ThreadSeen {
                    user_id: me,
                    channel_id: id,
                    root_id: root,
                    last_seen_id: msg.id,
                    mentions_cleared,
                    mentions: Some(mentions),
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
                    last_read_message_id: Some(msg.id),
                    mentions: Some(mentions),
                    unread: Some(unread),
                },
            )
            .await;
    }
    if !used.is_empty() {
        // Still under this send's `in_order` guard (tokio's mutex isn't reentrant).
        crate::emoji::publish_frequent(&s, me).await;
    }
    Ok(Json(msg))
}

/// Load a live message the caller can see and authored. Outsiders get 404, others' messages 403.
pub(crate) async fn own_message(
    db: &SqlitePool,
    me: UserId,
    id: MessageId,
) -> AppResult<(Message, Channel)> {
    let msg = load(db, Some(me), id).await?.ok_or(AppError::NotFound)?;
    let channel = channel_for(db, me, msg.channel_id).await?;
    if msg.deleted {
        return Err(AppError::NotFound);
    }
    if msg.author_id != Some(me) {
        return Err(AppError::Forbidden);
    }
    Ok((msg, channel))
}

async fn edit(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
    Json(req): Json<EditMessageRequest>,
) -> AppResult<Json<Message>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let (mut msg, _) = own_message(&s.db, me, id).await?;
    if msg.quote.is_some() {
        return Err(AppError::BadRequest(
            "quotes can't be edited like messages".into(),
        ));
    }
    let is_clip: Option<i64> = sqlx::query_scalar("SELECT 1 FROM clips WHERE message_id = ?")
        .bind(id.to_string())
        .fetch_optional(&s.db)
        .await?;
    if is_clip.is_some() {
        return Err(AppError::BadRequest(
            "clips are edited with their own route".into(),
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
        return Err(AppError::TooManyRequests(None));
    }
    let (msg, ch) = own_message(&s.db, me, id).await?;
    let is_quote = msg.quote.is_some();
    let is_clip = msg.clip.is_some();
    // Every delete changes counts others publish as absolutes (mark_read's unread, a mention
    // count, a quote's reactions and awards): publish in commit order. Deletes are rare.
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    // A vote whose time ran out is over even if no sweep has ended it yet: end it first, so the
    // quote it may have won with is still there (and still its option) when it is counted.
    let overdue = match ch.server_id {
        Some(server) if is_quote => crate::qoty::ending::end_if_overdue(&mut tx, server).await?,
        _ => None,
    };
    let changed = sqlx::query(
        "UPDATE messages SET content = '', deleted_at = ? WHERE id = ? AND author_id = ? AND deleted_at IS NULL",
    )
    .bind(now())
    .bind(id.to_string())
    .bind(me.to_string())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    let deleted = if is_quote {
        Some(crate::quotes::on_delete(&mut tx, id).await?)
    } else {
        None
    };
    let clip = if is_clip {
        Some(crate::clips::on_delete(&mut tx, id).await?)
    } else {
        None
    };
    tx.commit().await?;
    let mut deleted = deleted;
    if let Some(mut ended) = overdue {
        // The overdue vote this delete ended may have crowned the quote; its card was then
        // rewritten in the same transaction, so announce it as stored, with no update to follow.
        if let Some(d) = deleted.as_mut() {
            as_stored(&mut ended.cards, &mut d.cards, |c| load(&s.db, None, c)).await;
        }
        // Under this delete's `in_order` guard.
        crate::qoty::ending::announce(&s, ended).await;
    }
    s.hub
        .publish(
            &s.db,
            Event::MessageDeleted {
                channel_id: msg.channel_id,
                message_id: id,
                author_id: Some(me),
                // As loaded before the delete (only sending stores mentions): nothing after the
                // commit may fail the request, since the delete has landed.
                mentions: msg.mentions,
                thread_root_id: msg.thread_root_id,
            },
        )
        .await;
    if let Some(deleted) = deleted {
        for event in deleted.replies {
            s.hub.publish(&s.db, event).await;
        }
        // The delete has landed: a card that can't be read back is logged, not a failed request.
        for card in deleted.cards {
            match load(&s.db, None, card).await {
                Ok(Some(message)) => {
                    s.hub
                        .publish(&s.db, Event::MessageUpdated { message })
                        .await;
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::error!(error = ?e, %card, "couldn't load a rewritten winner card")
                }
            }
        }
        // Still under this delete's `in_order` guard.
        for event in deleted.qoty {
            crate::qoty::publish_updated(&s, event).await;
        }
    }
    if let Some(clip) = clip {
        for event in clip.replies {
            s.hub.publish(&s.db, event).await;
        }
        drop(_order);
        remove_clip_object(&s, &clip.key).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Delete a deleted clip's video from storage, then its media row. A failure is logged, not
/// returned (the delete has landed): the row stays and the sweeper retries.
async fn remove_clip_object(s: &AppState, key: &str) {
    let Some(r2) = &s.r2 else { return };
    if let Err(e) = r2.delete(&crate::clips::object_key(key)).await {
        tracing::warn!(error = ?e, "couldn't delete a deleted clip's object; the sweeper will retry");
        return;
    }
    if let Err(e) = sqlx::query("DELETE FROM media WHERE hash = ? AND kind = 'clip'")
        .bind(key)
        .execute(&s.db)
        .await
    {
        tracing::warn!(error = ?e, "couldn't delete a deleted clip's media row");
    }
}

/// Replace each of `cards` (just created, about to be announced) that `rewritten` lists with its
/// stored copy, taking it off `rewritten`, so it goes out once and already rewritten. A card that
/// can't be read back stays on `rewritten` (logged), so the `MessageUpdated` that follows still
/// carries the stored version.
async fn as_stored<F, Fut>(cards: &mut [Message], rewritten: &mut Vec<MessageId>, mut load: F)
where
    F: FnMut(MessageId) -> Fut,
    Fut: std::future::Future<Output = AppResult<Option<Message>>>,
{
    for card in cards.iter_mut() {
        let Some(i) = rewritten.iter().position(|c| *c == card.id) else {
            continue;
        };
        match load(card.id).await {
            Ok(Some(stored)) => {
                *card = stored;
                rewritten.swap_remove(i);
            }
            Ok(None) => {
                tracing::error!(card = %card.id, "a rewritten winner card is gone before it was announced");
            }
            Err(e) => {
                tracing::error!(error = ?e, card = %card.id, "couldn't load a rewritten winner card to announce");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_protocol::rest::MessageKind;

    fn card(content: &str) -> Message {
        Message {
            id: MessageId::new(),
            channel_id: ChannelId::new(),
            author_id: None,
            kind: MessageKind::System,
            content: content.into(),
            reply_to_id: None,
            created_at: now(),
            edited_at: None,
            deleted: false,
            mentions: vec![],
            reactions: vec![],
            thread_root_id: None,
            quote: None,
            clip: None,
            qoty_card: None,
        }
    }

    #[tokio::test]
    async fn a_rewritten_card_goes_out_as_stored_or_stays_for_the_update() {
        let (a, b, other) = (card("words a"), card("words b"), card("other"));
        let mut cards = vec![a.clone(), b.clone(), other.clone()];
        let mut rewritten = vec![a.id, b.id];
        let stored_a = Message {
            content: "a deleted quote".into(),
            ..a.clone()
        };
        as_stored(&mut cards, &mut rewritten, |id| {
            let stored_a = stored_a.clone();
            async move {
                if id == stored_a.id {
                    Ok(Some(stored_a))
                } else {
                    Err(AppError::Internal(anyhow::anyhow!("db down")))
                }
            }
        })
        .await;
        assert_eq!(cards, vec![stored_a, b.clone(), other]);
        // b couldn't be read back: it stays for the MessageUpdated that follows.
        assert_eq!(rewritten, vec![b.id]);
    }
}
