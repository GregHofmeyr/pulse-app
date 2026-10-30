//! Conversation management: groups, read points, mutes, closing.

use std::collections::{BTreeSet, HashMap};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, patch, post, put};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{
    AddMembersRequest, Channel, ChannelKind, MarkReadRequest, Mute, MuteTarget,
    RenameChannelRequest, SetMuteRequest,
};
use sqlx::SqlitePool;

use crate::AppState;
use crate::access::{channel_for, load_channel};
use crate::auth::AuthUser;
use crate::auth::routes::load_user;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::voice::state::Left;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/channels/{id}/read", post(mark_read))
        .route("/channels/{id}/members", post(add_members))
        .route("/channels/{id}/members/{user_id}", delete(remove_member))
        .route("/channels/{id}", patch(rename))
        .route("/mutes", put(set_mute))
        .route("/mutes/{kind}/{id}", delete(clear_mute))
        .route("/channels/{id}/close", post(close))
}

async fn mark_read(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<MarkReadRequest>,
) -> AppResult<StatusCode> {
    channel_for(&s.db, me, id).await?;
    // A reply lives in its quote's thread, not the main list the read point measures.
    let in_channel: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM messages WHERE id = ? AND channel_id = ? AND thread_root_id IS NULL",
    )
    .bind(req.message_id.to_string())
    .bind(id.to_string())
    .fetch_optional(&s.db)
    .await?;
    if in_channel.is_none() {
        return Err(AppError::BadRequest("message not in this channel".into()));
    }
    // The event carries an absolute count: take it in publish order, with messages that may mention you.
    let _order = s.hub.in_order().await;
    let moved = {
        let mut c = s.db.acquire().await?;
        crate::reads::advance(&mut c, me, id, req.message_id).await?
    };
    if moved
        && let Some((unread, mentions)) = crate::reads::counts_after_commit(&s.db, me, id).await
    {
        s.hub
            .publish(
                &s.db,
                Event::ReadStateUpdated {
                    user_id: me,
                    channel_id: id,
                    last_read_message_id: Some(req.message_id),
                    mentions: Some(mentions),
                    unread: Some(unread),
                },
            )
            .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

const MAX_GROUP: usize = 10;

pub async fn members_of(db: &SqlitePool, channel: ChannelId) -> AppResult<Vec<UserId>> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT user_id FROM channel_members WHERE channel_id = ? ORDER BY user_id",
    )
    .bind(channel.to_string())
    .fetch_all(db)
    .await?;
    Ok(ids
        .iter()
        .map(|i| i.parse())
        .collect::<Result<_, _>>()
        .map_err(anyhow::Error::from)?)
}

/// The members of every DM and group `me` is in, in one query, each list ordered as
/// [`members_of`] orders it.
pub async fn members_of_my_conversations(
    db: &SqlitePool,
    me: UserId,
) -> AppResult<HashMap<ChannelId, Vec<UserId>>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT cm.channel_id, cm.user_id FROM channel_members cm
         JOIN channel_members mine ON mine.channel_id = cm.channel_id AND mine.user_id = ?
         ORDER BY cm.channel_id, cm.user_id",
    )
    .bind(me.to_string())
    .fetch_all(db)
    .await?;
    let mut out: HashMap<ChannelId, Vec<UserId>> = HashMap::new();
    for (ch, u) in rows {
        let ch: ChannelId = ch.parse().map_err(anyhow::Error::from)?;
        out.entry(ch)
            .or_default()
            .push(u.parse().map_err(anyhow::Error::from)?);
    }
    Ok(out)
}

async fn username(db: &SqlitePool, u: UserId) -> AppResult<String> {
    Ok(load_user(db, u).await?.username)
}

async fn group_for(s: &AppState, me: UserId, id: ChannelId) -> AppResult<Channel> {
    let ch = channel_for(&s.db, me, id).await?; // 404 for outsiders
    if ch.kind != ChannelKind::Group {
        return Err(AppError::BadRequest(
            "only group conversations can be changed".into(),
        ));
    }
    Ok(ch)
}

async fn add_members(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<AddMembersRequest>,
) -> AppResult<StatusCode> {
    if !s.limits.create_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    group_for(&s, me, id).await?;
    let current = members_of(&s.db, id).await?;
    let new: Vec<UserId> = req
        .user_ids
        .into_iter()
        .filter(|u| !current.contains(u))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if new.is_empty() {
        return Ok(StatusCode::NO_CONTENT);
    }
    if current.len() + new.len() > MAX_GROUP {
        return Err(AppError::BadRequest(format!(
            "groups can have up to {MAX_GROUP} people"
        )));
    }
    for u in &new {
        let hit: Option<i64> = sqlx::query_scalar("SELECT 1 FROM users WHERE id = ?")
            .bind(u.to_string())
            .fetch_optional(&s.db)
            .await?;
        if hit.is_none() {
            return Err(AppError::BadRequest("unknown user".into()));
        }
    }
    // Write first, then count, under the write lock: two adds racing past the check above
    // can't overfill the group.
    let mut tx = crate::db::begin_write(&s.db).await?;
    crate::servers::routes::check_group_cap(&mut tx, &new).await?; // dropping `tx` rolls back
    for u in &new {
        sqlx::query("INSERT INTO channel_members (channel_id, user_id, added_by, added_at) VALUES (?, ?, ?, ?)")
            .bind(id.to_string()).bind(u.to_string()).bind(me.to_string()).bind(now())
            .execute(&mut *tx).await?;
    }
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM channel_members WHERE channel_id = ?")
            .bind(id.to_string())
            .fetch_one(&mut *tx)
            .await?;
    if count as usize > MAX_GROUP {
        return Err(AppError::BadRequest(format!(
            "groups can have up to {MAX_GROUP} people"
        ))); // dropping `tx` rolls the inserts back
    }
    for u in &new {
        crate::reads::init_point(&mut tx, *u, id).await?;
    }
    tx.commit().await?;
    let channel = load_channel(&s.db, id).await?.ok_or(AppError::NotFound)?;
    s.hub
        .publish(&s.db, Event::ChannelCreated { channel })
        .await; // reaches the newcomers too
    s.hub
        .publish(
            &s.db,
            Event::GroupMembersChanged {
                channel_id: id,
                user_ids: members_of(&s.db, id).await?,
            },
        )
        .await;
    let mut names = Vec::new();
    for u in &new {
        names.push(username(&s.db, *u).await?);
    }
    let content = format!(
        "{} added {}",
        username(&s.db, me).await?,
        crate::messages::system::join_names(&names)
    );
    crate::messages::system::post(
        &s,
        id,
        content,
        serde_json::json!({"type": "members_added", "by": me, "users": new}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_member(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path((id, user)): Path<(ChannelId, UserId)>,
) -> AppResult<StatusCode> {
    group_for(&s, me, id).await?;
    // One transaction: a crash or a concurrent add can't leave a half-removed member or an
    // empty group behind.
    let mut tx = crate::db::begin_write(&s.db).await?;
    let removed = sqlx::query("DELETE FROM channel_members WHERE channel_id = ? AND user_id = ?")
        .bind(id.to_string())
        .bind(user.to_string())
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(AppError::NotFound); // dropping `tx` rolls back
    }
    for t in ["read_states", "dm_hidden"] {
        sqlx::query(&format!(
            "DELETE FROM {t} WHERE channel_id = ? AND user_id = ?"
        ))
        .bind(id.to_string())
        .bind(user.to_string())
        .execute(&mut *tx)
        .await?;
    }
    let unmuted = sqlx::query(
        "DELETE FROM notification_prefs WHERE user_id = ? AND target_kind = 'channel' AND target_id = ?",
    )
    .bind(user.to_string())
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM channel_members WHERE channel_id = ?")
            .bind(id.to_string())
            .fetch_one(&mut *tx)
            .await?;
    if remaining == 0 {
        sqlx::query("DELETE FROM channels WHERE id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
    }
    // Their LiveKit tokens for this call go with their membership (a rejoin with one is also
    // refused by the join check, `voice::revoke::admit`).
    let had_grants =
        crate::auth::session::drop_grants(&mut tx, &id.to_string(), &user.to_string()).await?;
    tx.commit().await?;
    // Out of the group, out of its call: LiveKit removes them (best-effort, bounded wait) and
    // they leave the voice state. Their leave webhook then finds nothing to do.
    let left_call = s.voice.leave(id, user, "") == Left::Removed;
    if had_grants > 0 || left_call {
        crate::voice::revoke::remove_identities(&s, &id.to_string(), &[user.to_string()]).await;
    }
    if left_call {
        s.hub
            .publish(
                &s.db,
                Event::VoiceLeft {
                    channel_id: id,
                    user_id: user,
                },
            )
            .await;
    }
    // Committed: the rest only tells people. A failure there is logged, not returned: the removal
    // stands, and the client's retry would only get a 404.
    if let Err(e) = announce_removal(&s, me, id, user, unmuted > 0, remaining == 0).await {
        tracing::warn!(error = %e, channel = %id, "member removed, but announcing it failed");
    }
    Ok(StatusCode::NO_CONTENT)
}

/// The events and the system line for a committed [`remove_member`].
async fn announce_removal(
    s: &AppState,
    me: UserId,
    id: ChannelId,
    user: UserId,
    unmuted: bool,
    emptied: bool,
) -> AppResult<()> {
    if unmuted {
        s.hub
            .publish(
                &s.db,
                Event::MutesChanged {
                    user_id: user,
                    mutes: mutes_of(&s.db, user).await?,
                },
            )
            .await;
    }
    s.hub
        .publish(
            &s.db,
            Event::ChannelRemoved {
                channel_id: id,
                user_id: user,
            },
        )
        .await;
    if emptied {
        return Ok(());
    }
    s.hub
        .publish(
            &s.db,
            Event::GroupMembersChanged {
                channel_id: id,
                user_ids: members_of(&s.db, id).await?,
            },
        )
        .await;
    let (content, payload) = if user == me {
        (
            format!("{} left", username(&s.db, me).await?),
            serde_json::json!({"type": "member_left", "user": me}),
        )
    } else {
        (
            format!(
                "{} removed {}",
                username(&s.db, me).await?,
                username(&s.db, user).await?
            ),
            serde_json::json!({"type": "member_removed", "by": me, "user": user}),
        )
    };
    crate::messages::system::post(s, id, content, payload).await?;
    Ok(())
}

async fn rename(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<RenameChannelRequest>,
) -> AppResult<Json<Channel>> {
    // Each rename reaches every member and adds a system message: limited and disk-guarded.
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    crate::media::writes_ok(&s.cfg)?;
    group_for(&s, me, id).await?;
    let name = req
        .name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());
    if name.as_ref().is_some_and(|n| n.chars().count() > 64) {
        return Err(AppError::BadRequest(
            "group names are up to 64 characters".into(),
        ));
    }
    sqlx::query("UPDATE channels SET name = ? WHERE id = ?")
        .bind(&name)
        .bind(id.to_string())
        .execute(&s.db)
        .await?;
    let channel = load_channel(&s.db, id).await?.ok_or(AppError::NotFound)?;
    s.hub
        .publish(
            &s.db,
            Event::ChannelUpdated {
                channel: channel.clone(),
            },
        )
        .await;
    let who = username(&s.db, me).await?;
    let content = match &name {
        Some(n) => format!("{who} renamed the group to {n}"),
        None => format!("{who} removed the group name"),
    };
    crate::messages::system::post(
        &s,
        id,
        content,
        serde_json::json!({"type": "group_renamed", "by": me, "name": name}),
    )
    .await?;
    Ok(Json(channel))
}

pub async fn mutes_of(db: &SqlitePool, me: UserId) -> AppResult<Vec<Mute>> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT target_kind, target_id, muted_until FROM notification_prefs
         WHERE user_id = ? AND muted = 1 AND (muted_until IS NULL OR muted_until > ?)",
    )
    .bind(me.to_string())
    .bind(now())
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(k, id, until)| {
            Some(Mute {
                target_kind: MuteTarget::parse(&k)?,
                target_id: id,
                until,
            })
        })
        .collect())
}

pub async fn hidden_of(db: &SqlitePool, me: UserId) -> AppResult<Vec<ChannelId>> {
    let ids: Vec<String> = sqlx::query_scalar("SELECT channel_id FROM dm_hidden WHERE user_id = ?")
        .bind(me.to_string())
        .fetch_all(db)
        .await?;
    Ok(ids
        .iter()
        .map(|i| i.parse())
        .collect::<Result<_, _>>()
        .map_err(anyhow::Error::from)?)
}

/// 404 unless `me` can see the target (servers are public; channels via channel_for).
async fn check_target(s: &AppState, me: UserId, kind: MuteTarget, id: &str) -> AppResult<()> {
    match kind {
        MuteTarget::Server => {
            let hit: Option<i64> = sqlx::query_scalar("SELECT 1 FROM servers WHERE id = ?")
                .bind(id)
                .fetch_optional(&s.db)
                .await?;
            hit.map(|_| ()).ok_or(AppError::NotFound)
        }
        MuteTarget::Channel => {
            let ch: ChannelId = id.parse().map_err(|_| AppError::NotFound)?;
            channel_for(&s.db, me, ch).await.map(|_| ())
        }
    }
}

async fn set_mute(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(req): Json<SetMuteRequest>,
) -> AppResult<StatusCode> {
    check_target(&s, me, req.target_kind, &req.target_id).await?;
    sqlx::query(
        "INSERT INTO notification_prefs (user_id, target_kind, target_id, muted, muted_until) VALUES (?, ?, ?, 1, ?)
         ON CONFLICT (user_id, target_kind, target_id) DO UPDATE SET muted = 1, muted_until = excluded.muted_until",
    )
    .bind(me.to_string()).bind(req.target_kind.as_str()).bind(&req.target_id).bind(&req.until)
    .execute(&s.db).await?;
    s.hub
        .publish(
            &s.db,
            Event::MutesChanged {
                user_id: me,
                mutes: mutes_of(&s.db, me).await?,
            },
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn clear_mute(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path((kind, id)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    let kind = MuteTarget::parse(&kind).ok_or(AppError::NotFound)?;
    sqlx::query(
        "DELETE FROM notification_prefs WHERE user_id = ? AND target_kind = ? AND target_id = ?",
    )
    .bind(me.to_string())
    .bind(kind.as_str())
    .bind(&id)
    .execute(&s.db)
    .await?;
    s.hub
        .publish(
            &s.db,
            Event::MutesChanged {
                user_id: me,
                mutes: mutes_of(&s.db, me).await?,
            },
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn close(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
) -> AppResult<StatusCode> {
    let ch = channel_for(&s.db, me, id).await?;
    if !ch.kind.is_private() {
        return Err(AppError::BadRequest(
            "only DMs and groups can be closed".into(),
        ));
    }
    sqlx::query(
        "INSERT OR REPLACE INTO dm_hidden (user_id, channel_id, hidden_at) VALUES (?, ?, ?)",
    )
    .bind(me.to_string())
    .bind(id.to_string())
    .bind(now())
    .execute(&s.db)
    .await?;
    s.hub
        .publish(
            &s.db,
            Event::ConversationVisibility {
                user_id: me,
                channel_id: id,
                hidden: true,
            },
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}
