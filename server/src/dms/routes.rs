//! Conversation management: groups, read points, mutes, closing.

use std::collections::BTreeSet;

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
    let in_channel: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM messages WHERE id = ? AND channel_id = ?")
            .bind(req.message_id.to_string())
            .bind(id.to_string())
            .fetch_optional(&s.db)
            .await?;
    if in_channel.is_none() {
        return Err(AppError::BadRequest("message not in this channel".into()));
    }
    if crate::reads::advance(&s.db, me, id, req.message_id).await? {
        s.hub
            .publish(
                &s.db,
                Event::ReadStateUpdated {
                    user_id: me,
                    channel_id: id,
                    last_read_message_id: Some(req.message_id),
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
    // Write first, then count: the first INSERT takes SQLite's write lock, so the count below
    // sees every committed add, and two adds racing past the check above can't overfill the group.
    let mut tx = s.db.begin().await?;
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
    let removed = sqlx::query("DELETE FROM channel_members WHERE channel_id = ? AND user_id = ?")
        .bind(id.to_string())
        .bind(user.to_string())
        .execute(&s.db)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(AppError::NotFound);
    }
    for t in ["read_states", "dm_hidden"] {
        sqlx::query(&format!(
            "DELETE FROM {t} WHERE channel_id = ? AND user_id = ?"
        ))
        .bind(id.to_string())
        .bind(user.to_string())
        .execute(&s.db)
        .await?;
    }
    let unmuted = sqlx::query(
        "DELETE FROM notification_prefs WHERE user_id = ? AND target_kind = 'channel' AND target_id = ?",
    )
    .bind(user.to_string())
    .bind(id.to_string())
    .execute(&s.db)
    .await?
    .rows_affected();
    if unmuted > 0 {
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
    let left = members_of(&s.db, id).await?;
    if left.is_empty() {
        sqlx::query("DELETE FROM channels WHERE id = ?")
            .bind(id.to_string())
            .execute(&s.db)
            .await?;
        return Ok(StatusCode::NO_CONTENT);
    }
    s.hub
        .publish(
            &s.db,
            Event::GroupMembersChanged {
                channel_id: id,
                user_ids: left,
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
    crate::messages::system::post(&s, id, content, payload).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn rename(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<RenameChannelRequest>,
) -> AppResult<Json<Channel>> {
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
