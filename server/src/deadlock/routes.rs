//! Deadlock's routes (#390): each server's Deadlock channel, the member list's ranks, and a member's Deadlock section.
//! Your own switches live with your Steam link (`PUT /me/steam/settings`).

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use pulse_protocol::rest::{
    ChannelKind, DeadlockChannel, DeadlockProfile, DeadlockRankEntry, DeadlockRanks,
    SetDeadlockChannelRequest,
};

use super::assets;
use crate::AppState;
use crate::access::load_channel;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::servers::routes::require_member;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/servers/{id}/deadlock-channel",
            get(get_channel).put(set_channel),
        )
        .route("/servers/{id}/deadlock-ranks", get(ranks))
        .route("/servers/{id}/members/{user}/deadlock", get(member))
}

async fn channel_of(s: &AppState, server: ServerId) -> AppResult<DeadlockChannel> {
    let row: Option<(String, bool, bool, bool)> = sqlx::query_as(
        "SELECT d.channel_id, d.match_cards, d.rank_movers, d.patch_notes FROM server_deadlock_channels d
         JOIN channels c ON c.id = d.channel_id AND c.deleted_at IS NULL
         WHERE d.server_id = ?",
    )
    .bind(server.to_string())
    .fetch_optional(&s.db)
    .await?;
    Ok(match row {
        Some((ch, m, r, p)) => DeadlockChannel {
            channel_id: ch.parse().ok(),
            match_cards: m,
            rank_movers: r,
            patch_notes: p,
        },
        None => DeadlockChannel {
            channel_id: None,
            match_cards: true,
            rank_movers: true,
            patch_notes: true,
        },
    })
}

async fn get_channel(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
) -> AppResult<Json<DeadlockChannel>> {
    require_member(&s.db, id, me).await?;
    Ok(Json(channel_of(&s, id).await?))
}

/// `PUT /servers/{id}/deadlock-channel`: the server's owner picks one of its text channels (or none) and which posts
/// go there.
async fn set_channel(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
    Json(req): Json<SetDeadlockChannelRequest>,
) -> AppResult<Json<DeadlockChannel>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    require_member(&s.db, id, me).await?;
    let owner: String = sqlx::query_scalar("SELECT created_by FROM servers WHERE id = ?")
        .bind(id.to_string())
        .fetch_one(&s.db)
        .await?;
    if owner != me.to_string() {
        return Err(AppError::ForbiddenMsg(
            "Only the server's owner can pick its Deadlock channel.".into(),
        ));
    }
    let current = channel_of(&s, id).await?;
    match req.channel_id {
        None => {
            sqlx::query("DELETE FROM server_deadlock_channels WHERE server_id = ?")
                .bind(id.to_string())
                .execute(&s.db)
                .await?;
        }
        Some(ch) => {
            check_channel(&s, id, ch).await?;
            sqlx::query(
                "INSERT INTO server_deadlock_channels (server_id, channel_id, match_cards, rank_movers, patch_notes)
                 VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT (server_id) DO UPDATE SET channel_id = excluded.channel_id,
                     match_cards = excluded.match_cards, rank_movers = excluded.rank_movers,
                     patch_notes = excluded.patch_notes",
            )
            .bind(id.to_string())
            .bind(ch.to_string())
            .bind(req.match_cards.unwrap_or(current.match_cards))
            .bind(req.rank_movers.unwrap_or(current.rank_movers))
            .bind(req.patch_notes.unwrap_or(current.patch_notes))
            .execute(&s.db)
            .await?;
        }
    }
    Ok(Json(channel_of(&s, id).await?))
}

async fn check_channel(s: &AppState, server: ServerId, ch: ChannelId) -> AppResult<()> {
    let channel = load_channel(&s.db, ch).await?.ok_or(AppError::NotFound)?;
    if channel.server_id != Some(server) || channel.kind != ChannelKind::Text {
        return Err(AppError::BadRequest(
            "pick one of this server's text channels".into(),
        ));
    }
    crate::access::require_open(&channel) // an archived one posts nothing (#303)
}

/// `GET /servers/{id}/deadlock-ranks`: the members' ranks for the member list; empty while deadlock-api is dark.
async fn ranks(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
) -> AppResult<Json<DeadlockRanks>> {
    require_member(&s.db, id, me).await?;
    if !s.deadlock.api().available() {
        return Ok(Json(DeadlockRanks { entries: vec![] }));
    }
    let rows = super::ranks::of_server(&s.db, &id.to_string(), me).await?;
    let cat = if rows.is_empty() {
        None
    } else {
        assets::catalog(&s).await
    };
    let mut entries = Vec::new();
    for (user, badge) in rows {
        entries.push(DeadlockRankEntry {
            user_id: user,
            rank: assets::rank(&s, me, cat.as_deref(), badge).await,
        });
    }
    Ok(Json(DeadlockRanks { entries }))
}

/// `GET /servers/{id}/members/{user}/deadlock`: a member's Deadlock section, for another member.
async fn member(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path((id, user)): Path<(ServerId, UserId)>,
) -> AppResult<Json<DeadlockProfile>> {
    require_member(&s.db, id, me).await?;
    if !crate::servers::routes::is_server_member(&s.db, id, user).await? {
        return Err(AppError::NotFound);
    }
    Ok(Json(super::squad::profile(&s, id, me, user).await?))
}
