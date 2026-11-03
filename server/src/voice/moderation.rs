//! Server mute, server deafen and disconnect (#140), like Discord: anyone in a server may do them
//! to anyone else in its voice channels, and anyone (but the target) may undo a mute or deafen.
//!
//! Mute and deafen are a row per (server, user) in `voice_moderation`, kept across leaving and
//! rejoining until undone, and carried in `Ready` and `ServerVoiceChanged`. Both take away
//! publishing at LiveKit (deafen implies mute): the voice token is minted without it, a live
//! participant's permission is updated, and every join is reconciled. Not hearing anything while
//! deafened is the target's client muting the call. Disconnect removes the participant from the
//! voice and stream rooms now and stores nothing; they can rejoin.

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use pulse_protocol::rest::{ServerVoiceMod, SetServerVoiceRequest};
use sqlx::SqlitePool;

use crate::AppState;
use crate::access::load_channel;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::servers::routes::{is_server_member, require_member};

pub fn router() -> Router<AppState> {
    Router::new().route("/servers/{server}/voice/{user}", post(set))
}

type Row = (String, String, Option<String>, Option<String>);

fn from_row((server, user, muted, deafened): Row) -> AppResult<ServerVoiceMod> {
    let id = |s: &str| {
        s.parse::<UserId>()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("bad user id {s}: {e}")))
    };
    Ok(ServerVoiceMod {
        server_id: server
            .parse()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("bad server id {server}: {e}")))?,
        user_id: id(&user)?,
        muted_by: muted.as_deref().map(id).transpose()?,
        deafened_by: deafened.as_deref().map(id).transpose()?,
    })
}

/// `user`'s server mute / deafen in `server`, if any is in force.
pub async fn get(
    db: &SqlitePool,
    server: ServerId,
    user: UserId,
) -> AppResult<Option<ServerVoiceMod>> {
    let row: Option<Row> = sqlx::query_as(
        "SELECT server_id, user_id, muted_by, deafened_by FROM voice_moderation
         WHERE server_id = ? AND user_id = ? AND (muted_by IS NOT NULL OR deafened_by IS NOT NULL)",
    )
    .bind(server.to_string())
    .bind(user.to_string())
    .fetch_optional(db)
    .await?;
    row.map(from_row).transpose()
}

/// Every server's mutes / deafens in force (servers are public, like their members).
pub async fn for_ready(db: &SqlitePool) -> AppResult<Vec<ServerVoiceMod>> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT server_id, user_id, muted_by, deafened_by FROM voice_moderation
         WHERE muted_by IS NOT NULL OR deafened_by IS NOT NULL ORDER BY server_id, user_id",
    )
    .fetch_all(db)
    .await?;
    rows.into_iter().map(from_row).collect()
}

/// Whether someone with this server mute / deafen may publish their mic.
pub fn publish_allowed(m: Option<&ServerVoiceMod>) -> bool {
    m.is_none_or(|m| m.muted_by.is_none() && m.deafened_by.is_none())
}

/// The server voice channel `user` is in, if it belongs to `server`.
pub(crate) async fn call_in(
    s: &AppState,
    server: ServerId,
    user: UserId,
) -> AppResult<Option<ChannelId>> {
    let Some((channel, _)) = s.voice.room_flags(user) else {
        return Ok(None);
    };
    let ch = load_channel(&s.db, channel).await?;
    Ok(ch.filter(|c| c.server_id == Some(server)).map(|c| c.id))
}

/// Who may act on `target` in `server`: a member, not on themselves, on a member.
async fn check_actor(s: &AppState, server: ServerId, me: UserId, target: UserId) -> AppResult<()> {
    require_member(&s.db, server, me).await?;
    if me == target {
        return Err(AppError::BadRequest(
            "someone else has to do this to you".into(),
        ));
    }
    if !is_server_member(&s.db, server, target).await? {
        return Err(AppError::NotFound);
    }
    Ok(())
}

async fn set(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path((server, target)): Path<(ServerId, UserId)>,
    Json(req): Json<SetServerVoiceRequest>,
) -> AppResult<Json<ServerVoiceMod>> {
    check_actor(&s, server, me, target).await?;
    let call = call_in(&s, server, target).await?;
    if (req.mute == Some(true) || req.deafen == Some(true)) && call.is_none() {
        return Err(AppError::Conflict(
            "they aren't in this server's voice".into(),
        ));
    }
    let _order = s.hub.in_order().await;
    let before = get(&s.db, server, target).await?;
    let pick = |want: Option<bool>, had: Option<UserId>| match want {
        Some(true) => had.or(Some(me)),
        Some(false) => None,
        None => had,
    };
    let after = ServerVoiceMod {
        server_id: server,
        user_id: target,
        muted_by: pick(req.mute, before.as_ref().and_then(|b| b.muted_by)),
        deafened_by: pick(req.deafen, before.as_ref().and_then(|b| b.deafened_by)),
    };
    let unchanged = match &before {
        Some(b) => *b == after,
        None => after.muted_by.is_none() && after.deafened_by.is_none(),
    };
    if unchanged {
        return Ok(Json(after));
    }
    if after.muted_by.is_none() && after.deafened_by.is_none() {
        sqlx::query("DELETE FROM voice_moderation WHERE server_id = ? AND user_id = ?")
            .bind(server.to_string())
            .bind(target.to_string())
            .execute(&s.db)
            .await?;
    } else {
        sqlx::query(
            "INSERT INTO voice_moderation (server_id, user_id, muted_by, deafened_by, updated_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (server_id, user_id) DO UPDATE SET
               muted_by = excluded.muted_by, deafened_by = excluded.deafened_by,
               updated_at = excluded.updated_at",
        )
        .bind(server.to_string())
        .bind(target.to_string())
        .bind(after.muted_by.map(|u| u.to_string()))
        .bind(after.deafened_by.map(|u| u.to_string()))
        .bind(now())
        .execute(&s.db)
        .await?;
    }
    tracing::info!(server = %server, target = %target, by = %me, muted = after.muted_by.is_some(), deafened = after.deafened_by.is_some(), "server voice changed");
    s.hub
        .publish(
            &s.db,
            Event::ServerVoiceChanged {
                state: after.clone(),
            },
        )
        .await;
    Ok(Json(after))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_publish_follows_mute_or_deafen() {
        let (server, user, by) = (ServerId::new(), UserId::new(), UserId::new());
        let m = |muted: bool, deafened: bool| ServerVoiceMod {
            server_id: server,
            user_id: user,
            muted_by: muted.then_some(by),
            deafened_by: deafened.then_some(by),
        };
        assert!(publish_allowed(None));
        assert!(publish_allowed(Some(&m(false, false))));
        assert!(!publish_allowed(Some(&m(true, false))));
        assert!(
            !publish_allowed(Some(&m(false, true))),
            "deafen implies mute"
        );
        assert!(!publish_allowed(Some(&m(true, true))));
    }
}
