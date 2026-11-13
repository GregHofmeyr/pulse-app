use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{
    Channel, ChannelKind, CreatePollRequest, Message, MessageKind, Poll, PollDuration,
    PollVoteRequest,
};

use super::{Stored, announce_close, clean_options, clean_question, close_in, creator_name};
use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::{begin_write, now, stamp};
use crate::error::{AppError, AppResult};
use crate::servers::routes::is_server_member;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/channels/{id}/polls", post(create))
        .route("/messages/{id}/poll", get(get_poll))
        .route("/messages/{id}/poll/votes", put(vote))
        .route("/messages/{id}/poll/close", post(close))
}

fn is_over() -> AppError {
    AppError::Conflict("this poll has ended".into())
}

/// The chat `me` may post a poll in, or vote in: as sending a message (a DM or group only for its members, a
/// server chat only for the server's members), and not a quotes or clips channel.
async fn poll_channel(s: &AppState, me: UserId, id: ChannelId) -> AppResult<Channel> {
    let ch = channel_for(&s.db, me, id).await?;
    if matches!(ch.kind, ChannelKind::Quotes | ChannelKind::Clips) {
        return Err(AppError::BadRequest(
            "polls go in text and voice chats, DMs and groups".into(),
        ));
    }
    if let Some(server) = ch.server_id
        && !is_server_member(&s.db, server, me).await?
    {
        return Err(AppError::Forbidden);
    }
    Ok(ch)
}

/// A live poll whose chat `me` can see (404 otherwise, as if there were none).
async fn visible(s: &AppState, me: UserId, id: MessageId) -> AppResult<Stored> {
    let mut conn = s.db.acquire().await?;
    let p = super::load(&mut conn, id)
        .await?
        .ok_or(AppError::NotFound)?;
    drop(conn);
    channel_for(&s.db, me, p.channel_id).await?;
    Ok(p)
}

async fn my_view(s: &AppState, me: UserId, id: MessageId) -> AppResult<Json<Poll>> {
    super::view_for(&s.db, Some(me), id)
        .await?
        .map(Json)
        .ok_or(AppError::NotFound)
}

async fn changed(s: &AppState, p: &Stored) {
    s.hub
        .publish(
            &s.db,
            Event::PollChanged {
                channel_id: p.channel_id,
                message_id: p.message_id,
            },
        )
        .await;
}

/// `POST /channels/{id}/polls`: post a poll. Returns its message with the poll as its creator sees it; the chat
/// gets it as a `MessageCreated`.
async fn create(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<CreatePollRequest>,
) -> AppResult<(StatusCode, Json<Message>)> {
    if !s.limits.send_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    crate::media::writes_ok(&s.cfg)?;
    let ch = poll_channel(&s, me, id).await?;
    let question = clean_question(&req.question)?;
    let options = clean_options(req.options)?;
    let at = chrono::Utc::now();
    let ends_at = match req.ends_in {
        PollDuration::Hour => Some(stamp(at + chrono::Duration::hours(1))),
        PollDuration::Day => Some(stamp(at + chrono::Duration::days(1))),
        PollDuration::Week => Some(stamp(at + chrono::Duration::weeks(1))),
        PollDuration::Never => None,
    };
    let mut msg = Message {
        id: MessageId::new(),
        channel_id: id,
        author_id: Some(me),
        kind: MessageKind::Normal,
        content: super::fallback_text(&question, &options),
        reply_to_id: None,
        created_at: now(),
        edited_at: None,
        deleted: false,
        mentions: vec![],
        reactions: vec![],
        thread_root_id: None,
        quote: None,
        clip: None,
        embeds: vec![],
        call: None,
        wheel: None,
        qoty_card: None,
        pin: None,
        pin_line: None,
        poll: None,
        poll_ended: None,
        stage_share_line: None,
        gif: None,
        attachments: vec![],
        caption_is_fallback: false,
    };
    let stored = Stored {
        message_id: msg.id,
        channel_id: id,
        author_id: Some(me),
        question,
        options,
        multi: req.multi,
        anonymous: req.anonymous,
        ends_at,
        closed_at: None,
    };
    // As a send: the message, the poll and your read point land together, and the read point's event goes out in
    // commit order with every other message.
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    crate::access::channel_live_in(&mut tx, id).await?; // channel deleted since (#109)
    // minted under the write lock, so ids keep commit order (as a send)
    msg.id = MessageId::new();
    msg.created_at = now();
    let stored = Stored {
        message_id: msg.id,
        ..stored
    };
    sqlx::query("INSERT INTO messages (id, channel_id, author_id, kind, content, created_at) VALUES (?, ?, ?, 'normal', ?, ?)")
        .bind(msg.id.to_string())
        .bind(id.to_string())
        .bind(me.to_string())
        .bind(&msg.content)
        .bind(&msg.created_at)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO polls (message_id, channel_id, question, options, multi, anonymous, ends_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(msg.id.to_string())
    .bind(id.to_string())
    .bind(&stored.question)
    .bind(serde_json::to_string(&stored.options).map_err(anyhow::Error::from)?)
    .bind(i64::from(stored.multi))
    .bind(i64::from(stored.anonymous))
    .bind(&stored.ends_at)
    .bind(&msg.created_at)
    .execute(&mut *tx)
    .await?;
    if ch.kind.is_private() {
        // A new message reopens closed conversations.
        sqlx::query("DELETE FROM dm_hidden WHERE channel_id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
    }
    // a poll counts as a message toward the server chat badges, as a send does (never in DMs or groups)
    let badged = ch.server_id.is_some();
    if badged {
        crate::badges::stats::add(&mut tx, me, crate::badges::catalogue::stat::MESSAGES, 1).await?;
    }
    let moved = crate::reads::advance(&mut tx, me, id, msg.id).await?;
    tx.commit().await?;
    let at = now();
    msg.poll = Some(Box::new(super::view(&stored, &[], None, false, &at)));
    s.hub
        .publish(
            &s.db,
            Event::MessageCreated {
                message: msg.clone(),
                nonce: None,
            },
        )
        .await;
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
    drop(_order);
    if badged {
        crate::badges::engine::after(&s, &[me]).await;
    }
    msg.poll = Some(Box::new(super::view(&stored, &[], Some(me), false, &at)));
    Ok((StatusCode::CREATED, Json(msg)))
}

/// `GET /messages/{id}/poll`: the poll as you see it now.
async fn get_poll(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<Json<Poll>> {
    visible(&s, me, id).await?;
    my_view(&s, me, id).await
}

/// `PUT /messages/{id}/poll/votes`: your choices, replacing any before; none takes your vote back.
async fn vote(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
    Json(req): Json<PollVoteRequest>,
) -> AppResult<Json<Poll>> {
    crate::media::writes_ok(&s.cfg)?;
    let p = visible(&s, me, id).await?;
    poll_channel(&s, me, p.channel_id).await?;
    let mut choices = req.options;
    choices.sort_unstable();
    choices.dedup();
    if choices.iter().any(|c| *c as usize >= p.options.len()) {
        return Err(AppError::BadRequest("no such option".into()));
    }
    if !p.multi && choices.len() > 1 {
        return Err(AppError::BadRequest("this poll takes one choice".into()));
    }
    if !s.limits.poll_vote_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(Some(
            "that's a lot of votes; try again in a moment".into(),
        )));
    }
    let mut tx = begin_write(&s.db).await?;
    // Read again under the write lock: a close (or a delete, of it or its channel) may have landed since.
    crate::access::channel_live_in(&mut tx, p.channel_id).await?;
    let at = now();
    match super::load(&mut tx, id).await? {
        None => return Err(AppError::NotFound),
        Some(p) if p.over(&at) => return Err(is_over()),
        Some(_) => {}
    }
    sqlx::query("DELETE FROM poll_votes WHERE message_id = ? AND voter_id = ?")
        .bind(id.to_string())
        .bind(me.to_string())
        .execute(&mut *tx)
        .await?;
    for c in &choices {
        sqlx::query(
            "INSERT INTO poll_votes (message_id, option, voter_id, voted_at) VALUES (?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(i64::from(*c))
        .bind(me.to_string())
        .bind(&at)
        .execute(&mut *tx)
        .await?;
    }
    if !choices.is_empty() {
        // results, once seen, stay: taking the vote back can't be used to peek without voting
        sqlx::query("INSERT OR IGNORE INTO poll_seen (message_id, user_id) VALUES (?, ?)")
            .bind(id.to_string())
            .bind(me.to_string())
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    changed(&s, &p).await;
    my_view(&s, me, id).await
}

/// `POST /messages/{id}/poll/close`: its creator ends it now. Posts the ended line.
async fn close(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<Json<Poll>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    crate::media::writes_ok(&s.cfg)?;
    let p = visible(&s, me, id).await?;
    if p.author_id != Some(me) {
        return Err(AppError::Forbidden);
    }
    let at = now();
    if p.over(&at) {
        return Err(is_over());
    }
    let who = creator_name(&s.db, &p).await?;
    // the ended line goes out in commit order with the chat's other messages (as a send)
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    crate::access::channel_live_in(&mut tx, p.channel_id).await?; // channel deleted since (#109)
    // a delete may have landed since the read: a deleted poll has no row
    if super::load(&mut tx, id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    let line = close_in(&mut tx, &p, &who, &at)
        .await?
        .ok_or_else(is_over)?;
    tx.commit().await?;
    announce_close(&s, &p, line).await;
    drop(_order);
    my_view(&s, me, id).await
}
