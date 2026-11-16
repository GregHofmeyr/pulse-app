use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use chrono::{DateTime, Utc};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{
    Channel, ChannelKind, CreatePlanRequest, Message, MessageKind, Plan, PlanForecast,
    PlanRemindersRequest, PlanRsvpRequest,
};

use super::{Cleaned, Stored, announce, clean, fallback_text, reminded_at_for, rsvp_str};
use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::{begin_write, now, stamp};
use crate::error::{AppError, AppResult};
use crate::servers::routes::is_server_member;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/channels/{id}/plans", post(create))
        .route("/messages/{id}/plan", get(get_plan).put(edit))
        .route("/messages/{id}/plan/rsvp", put(rsvp))
        .route("/messages/{id}/plan/cancel", post(cancel))
        .route("/messages/{id}/plan/forecast", get(forecast))
        .route("/messages/{id}/plan/ics", get(ics))
        .route("/me/plan-reminders", put(set_reminders))
}

/// Whether `user` gets plans' reminder pings (#399): on unless they turned it off.
pub async fn reminders_on(db: &sqlx::SqlitePool, user: UserId) -> AppResult<bool> {
    let on: Option<i64> = sqlx::query_scalar("SELECT plan_reminders FROM users WHERE id = ?")
        .bind(user.to_string())
        .fetch_optional(db)
        .await?;
    Ok(on.is_none_or(|v| v != 0))
}

/// `PUT /me/plan-reminders` (#399): turn plans' reminder pings on or off for you, on every device. Your own sessions
/// hear it (`PlanRemindersChanged`).
async fn set_reminders(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(req): Json<PlanRemindersRequest>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    sqlx::query("UPDATE users SET plan_reminders = ? WHERE id = ?")
        .bind(i64::from(req.on))
        .bind(me.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    s.hub
        .publish(
            &s.db,
            Event::PlanRemindersChanged {
                user_id: me,
                on: req.on,
            },
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}

fn is_cancelled() -> AppError {
    AppError::Conflict("this plan was cancelled".into())
}

/// The chat `me` may post a plan in, or answer one in: as sending a message (a DM or group only for its members, a
/// server chat only for the server's members), and not a quotes or clips channel. As polls (#157).
async fn plan_channel(s: &AppState, me: UserId, id: ChannelId) -> AppResult<Channel> {
    let ch = channel_for(&s.db, me, id).await?;
    if matches!(ch.kind, ChannelKind::Quotes | ChannelKind::Clips) {
        return Err(AppError::BadRequest(
            "plans go in text and voice chats, DMs and groups".into(),
        ));
    }
    if let Some(server) = ch.server_id
        && !is_server_member(&s.db, server, me).await?
    {
        return Err(AppError::Forbidden);
    }
    Ok(ch)
}

/// A live plan whose chat `me` can see (404 otherwise, as if there were none).
async fn visible(s: &AppState, me: UserId, id: MessageId) -> AppResult<Stored> {
    let mut conn = s.db.acquire().await?;
    let p = super::load(&mut conn, id)
        .await?
        .ok_or(AppError::NotFound)?;
    drop(conn);
    channel_for(&s.db, me, p.channel_id).await?;
    Ok(p)
}

async fn current(s: &AppState, id: MessageId) -> AppResult<Plan> {
    let mut conn = s.db.acquire().await?;
    super::view_of(&mut conn, id)
        .await?
        .ok_or(AppError::NotFound)
}

/// `POST /channels/{id}/plans`: post a plan. Returns its message with the plan; the chat gets it as a
/// `MessageCreated`.
async fn create(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<CreatePlanRequest>,
) -> AppResult<(StatusCode, Json<Message>)> {
    if !s.limits.send_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    crate::media::writes_ok(&s.cfg)?;
    let ch = plan_channel(&s, me, id).await?;
    let at = Utc::now();
    let c = clean(&req, at)?;
    let mut msg = Message {
        id: MessageId::new(),
        channel_id: id,
        author_id: Some(me),
        kind: MessageKind::Normal,
        content: fallback_text(&c),
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
        embeds_hidden: false,
        gif: None,
        attachments: vec![],
        caption_is_fallback: false,
        achievement: None,
        deadlock: None,
        plan: None,
        plan_reminder: None,
        f1: None,
    };
    // As a send: the message, the plan and your read point land together, and the read point's event goes out in
    // commit order with every other message.
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    crate::access::channel_live_in(&mut tx, id).await?; // channel deleted since (#109)
    // minted under the write lock, so ids keep commit order (as a send)
    msg.id = MessageId::new();
    msg.created_at = now();
    sqlx::query("INSERT INTO messages (id, channel_id, author_id, kind, content, created_at) VALUES (?, ?, ?, 'normal', ?, ?)")
        .bind(msg.id.to_string())
        .bind(id.to_string())
        .bind(me.to_string())
        .bind(&msg.content)
        .bind(&msg.created_at)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO plans (message_id, channel_id, title, starts_at, ends_at, place, note, reminded_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(msg.id.to_string())
    .bind(id.to_string())
    .bind(&c.title)
    .bind(&c.starts_at)
    .bind(&c.ends_at)
    .bind(&c.place)
    .bind(&c.note)
    .bind(reminded_at_for(&c.starts_at, at))
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
    // a plan counts as a message toward the server chat badges, as a send does (never in DMs or groups)
    let badged = ch.server_id.is_some();
    if badged {
        crate::badges::stats::add(&mut tx, me, crate::badges::catalogue::stat::MESSAGES, 1).await?;
    }
    let moved = crate::reads::advance(&mut tx, me, id, msg.id).await?;
    tx.commit().await?;
    msg.plan = Some(Box::new(Plan {
        title: c.title,
        starts_at: c.starts_at,
        ends_at: c.ends_at,
        place: c.place,
        note: c.note,
        going: vec![],
        maybe: vec![],
        cant: vec![],
        cancelled_at: None,
        edited_at: None,
    }));
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
    Ok((StatusCode::CREATED, Json(msg)))
}

/// `GET /messages/{id}/plan`: the plan as it is now.
async fn get_plan(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<Json<Plan>> {
    visible(&s, me, id).await?;
    current(&s, id).await.map(Json)
}

/// Load the plan for its creator's change, under the write lock in `tx`: there, theirs, not cancelled.
async fn mine_open(
    tx: &mut sqlx::SqliteConnection,
    me: UserId,
    id: MessageId,
) -> AppResult<Stored> {
    let p = super::load(tx, id).await?.ok_or(AppError::NotFound)?;
    if p.author_id != Some(me) {
        return Err(AppError::Forbidden);
    }
    if p.cancelled_at.is_some() {
        return Err(is_cancelled());
    }
    Ok(p)
}

/// `PUT /messages/{id}/plan`: its creator changes it (every field, as a new post would set them). A new time sets
/// its reminder up again.
async fn edit(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
    Json(req): Json<CreatePlanRequest>,
) -> AppResult<Json<Plan>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    crate::media::writes_ok(&s.cfg)?;
    let p = visible(&s, me, id).await?;
    let at = Utc::now();
    let c: Cleaned = clean(&req, at)?;
    let mut tx = begin_write(&s.db).await?;
    crate::access::channel_live_in(&mut tx, p.channel_id).await?;
    let p = mine_open(&mut tx, me, id).await?;
    let edited = stamp(at);
    sqlx::query(
        "UPDATE plans SET title = ?, starts_at = ?, ends_at = ?, place = ?, note = ?, edited_at = ?,
           reminded_at = CASE WHEN starts_at = ? THEN reminded_at ELSE ? END
         WHERE message_id = ?",
    )
    .bind(&c.title)
    .bind(&c.starts_at)
    .bind(&c.ends_at)
    .bind(&c.place)
    .bind(&c.note)
    .bind(&edited)
    .bind(&c.starts_at)
    .bind(reminded_at_for(&c.starts_at, at))
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?;
    // the text for clients that don't know plans (and search) follows
    sqlx::query("UPDATE messages SET content = ? WHERE id = ?")
        .bind(fallback_text(&c))
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    announce(&s, p.channel_id, id).await;
    current(&s, id).await.map(Json)
}

/// `PUT /messages/{id}/plan/rsvp`: your answer, replacing any before; none takes it back.
async fn rsvp(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
    Json(req): Json<PlanRsvpRequest>,
) -> AppResult<Json<Plan>> {
    crate::media::writes_ok(&s.cfg)?;
    let p = visible(&s, me, id).await?;
    plan_channel(&s, me, p.channel_id).await?;
    if !s.limits.poll_vote_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(Some(
            "that's a lot of answers; try again in a moment".into(),
        )));
    }
    let mut tx = begin_write(&s.db).await?;
    // Read again under the write lock: a cancel (or a delete, of it or its channel) may have landed since.
    crate::access::channel_live_in(&mut tx, p.channel_id).await?;
    match super::load(&mut tx, id).await? {
        None => return Err(AppError::NotFound),
        Some(p) if p.cancelled_at.is_some() => return Err(is_cancelled()),
        Some(_) => {}
    }
    match req.rsvp {
        // the same answer again keeps its place in the list
        Some(r) => {
            sqlx::query(
                "INSERT INTO plan_rsvps (message_id, user_id, rsvp, at) VALUES (?, ?, ?, ?)
                 ON CONFLICT (message_id, user_id) DO UPDATE SET rsvp = excluded.rsvp, at = excluded.at
                 WHERE plan_rsvps.rsvp != excluded.rsvp",
            )
            .bind(id.to_string())
            .bind(me.to_string())
            .bind(rsvp_str(r))
            .bind(now())
            .execute(&mut *tx)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM plan_rsvps WHERE message_id = ? AND user_id = ?")
                .bind(id.to_string())
                .bind(me.to_string())
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    announce(&s, p.channel_id, id).await;
    current(&s, id).await.map(Json)
}

/// `POST /messages/{id}/plan/cancel`: its creator calls it off. It stays in the chat, marked cancelled; no more
/// answers, no reminder.
async fn cancel(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<Json<Plan>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    crate::media::writes_ok(&s.cfg)?;
    let p = visible(&s, me, id).await?;
    let mut tx = begin_write(&s.db).await?;
    crate::access::channel_live_in(&mut tx, p.channel_id).await?;
    mine_open(&mut tx, me, id).await?;
    sqlx::query("UPDATE plans SET cancelled_at = ? WHERE message_id = ? AND cancelled_at IS NULL")
        .bind(now())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    announce(&s, p.channel_id, id).await;
    current(&s, id).await.map(Json)
}

/// `GET /messages/{id}/plan/forecast`: the forecast for its place on its day (`null` when there's none). The server
/// asks Open-Meteo and caches the answer; apps never call it themselves.
async fn forecast(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<Json<Option<PlanForecast>>> {
    let p = visible(&s, me, id).await?;
    let (Some(place), None) = (&p.place, &p.cancelled_at) else {
        return Ok(Json(None));
    };
    let Ok(starts) = DateTime::parse_from_rfc3339(&p.starts_at) else {
        return Ok(Json(None));
    };
    let f = super::weather::forecast(
        &s.db,
        &s.cfg.plans,
        place,
        starts.with_timezone(&Utc),
        Utc::now(),
    )
    .await?;
    Ok(Json(f))
}

/// `GET /messages/{id}/plan/ics`: the plan as a calendar file (`text/calendar`), named after its title.
async fn ics(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<Response> {
    visible(&s, me, id).await?;
    let plan = current(&s, id).await?;
    let body = super::ics::calendar(id, &plan, "pulse", Utc::now());
    let name = super::ics::file_name(&plan.title);
    // the name as an RFC 5987 `filename*`, so any title survives; the plain one is ASCII only
    let ascii: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .filter(|c| *c != '"' && *c != '\\')
        .collect();
    let encoded: String = name
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    Ok((
        [
            (
                header::CONTENT_TYPE,
                "text/calendar; charset=utf-8".to_string(),
            ),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}"),
            ),
        ],
        body,
    )
        .into_response())
}
