//! Bans (#80, step 1): a server's owner bans someone, which refuses their invites (`invites::accept`).
//!
//! `PUT /servers/{id}/bans/{user}` bans: the person is taken out of the server if they're in it (the same way out as a
//! removal, `membership::take_out_banned`, except that it doesn't count towards a rejoin block), and any open direct
//! invite to them is revoked. `DELETE` lifts it; `GET /servers/{id}/bans` lists them. The owner only, and only the
//! owner's devices hear `BansChanged`. Servers are still open in this step, but a ban stops the open Join too
//! (`routes::join`).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::ServerBan;
use sqlx::SqliteConnection;

use crate::AppState;
use crate::auth::AuthUser;
use crate::db::{begin_write, now};
use crate::error::{AppError, AppResult};
use crate::servers::routes::{server_exists, server_live_in};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/servers/{id}/bans", get(list))
        .route("/servers/{id}/bans/{user}", put(ban).delete(unban))
}

const ONLY_OWNER: &str = "Only the server's owner can ban people or see who's banned.";

/// 403 unless `me` owns `server` (read where the caller is: in a write transaction, under the lock).
async fn require_owner(db: &mut SqliteConnection, server: ServerId, me: UserId) -> AppResult<()> {
    let owner: String = sqlx::query_scalar("SELECT created_by FROM servers WHERE id = ?")
        .bind(server.to_string())
        .fetch_one(&mut *db)
        .await?;
    if owner == me.to_string() {
        Ok(())
    } else {
        Err(AppError::ForbiddenMsg(ONLY_OWNER.into()))
    }
}

async fn bans_changed(s: &AppState, server: ServerId, owner: UserId) {
    s.hub
        .publish(
            &s.db,
            Event::BansChanged {
                user_id: owner,
                server_id: server,
            },
        )
        .await;
}

/// `PUT /servers/{id}/bans/{user}`: ban someone (see the module docs). 403 for anyone but the owner, 400 on yourself,
/// 404 for someone unknown. Banning someone already banned changes nothing more.
async fn ban(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path((id, user)): Path<(ServerId, UserId)>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    server_exists(&s.db, id).await?;
    if user == me {
        return Err(AppError::BadRequest("you can't ban yourself".into()));
    }
    let known: Option<i64> = sqlx::query_scalar("SELECT 1 FROM users WHERE id = ?")
        .bind(user.to_string())
        .fetch_optional(&s.db)
        .await?;
    if known.is_none() {
        return Err(AppError::NotFound);
    }
    let mut tx = begin_write(&s.db).await?;
    server_live_in(&mut tx, id).await?;
    require_owner(&mut tx, id, me).await?;
    let added = sqlx::query(
        "INSERT OR IGNORE INTO server_bans (server_id, user_id, banned_by, banned_at) VALUES (?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(user.to_string())
    .bind(me.to_string())
    .bind(now())
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    let makers = crate::servers::invites::revoke_direct_in(&mut tx, id, user).await?;
    tx.commit().await?;
    tracing::info!(server = %id, user = %user, by = %me, "banned");
    // Out of the server if they're in it. The ban has landed, so an invite they accept from now on is refused.
    if crate::servers::routes::is_server_member(&s.db, id, user).await? {
        match crate::servers::membership::take_out_banned(&s, id, user, me).await {
            Ok(()) | Err(AppError::NotFound) => {} // a leave won the race
            Err(e) => return Err(e),
        }
    }
    if added {
        bans_changed(&s, id, me).await;
    }
    if !makers.is_empty() {
        crate::servers::invites::invites_changed(&s, id, makers).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /servers/{id}/bans/{user}`: lift a ban. 403 for anyone but the owner, 404 when they aren't banned.
async fn unban(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path((id, user)): Path<(ServerId, UserId)>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    server_exists(&s.db, id).await?;
    let mut tx = begin_write(&s.db).await?;
    server_live_in(&mut tx, id).await?;
    require_owner(&mut tx, id, me).await?;
    let gone = sqlx::query("DELETE FROM server_bans WHERE server_id = ? AND user_id = ?")
        .bind(id.to_string())
        .bind(user.to_string())
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if gone == 0 {
        return Err(AppError::NotFound);
    }
    tx.commit().await?;
    tracing::info!(server = %id, user = %user, by = %me, "unbanned");
    bans_changed(&s, id, me).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /servers/{id}/bans`: who's banned, newest first. The owner only (403 for anyone else).
async fn list(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
) -> AppResult<Json<Vec<ServerBan>>> {
    server_exists(&s.db, id).await?;
    require_owner(&mut *s.db.acquire().await?, id, me).await?;
    let rows: Vec<(String, Option<String>, String)> = sqlx::query_as(
        "SELECT user_id, banned_by, banned_at FROM server_bans WHERE server_id = ?
         ORDER BY banned_at DESC, user_id",
    )
    .bind(id.to_string())
    .fetch_all(&s.db)
    .await?;
    rows.into_iter()
        .map(|(user, by, banned_at)| {
            Ok(ServerBan {
                user_id: user.parse().map_err(anyhow::Error::from)?,
                banned_by: by
                    .map(|b| b.parse())
                    .transpose()
                    .map_err(anyhow::Error::from)?,
                banned_at,
            })
        })
        .collect::<AppResult<Vec<_>>>()
        .map(Json)
}
