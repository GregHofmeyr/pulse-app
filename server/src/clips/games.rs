//! The per-server games registry that clips are tagged with. Games are never deleted.

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{GameId, ServerId, UserId};
use pulse_protocol::rest::{AddGameRequest, Game};
use sqlx::SqlitePool;

use crate::AppState;
use crate::auth::AuthUser;
use crate::db::{begin_write, now};
use crate::error::{AppError, AppResult};
use crate::quotes::name_key;
use crate::servers::routes::is_server_member;

pub const MAX_NAME: usize = 40;
pub const MAX_PER_SERVER: i64 = 300;

pub fn router() -> Router<AppState> {
    Router::new().route("/servers/{id}/games", post(add))
}

type Row = (String, String, String, Option<String>, String);

const SELECT: &str = "SELECT id, server_id, name, created_by, created_at FROM games";

fn from_row((id, server_id, name, created_by, created_at): Row) -> AppResult<Game> {
    Ok(Game {
        id: id.parse::<GameId>().map_err(anyhow::Error::from)?,
        server_id: server_id.parse::<ServerId>().map_err(anyhow::Error::from)?,
        name,
        created_by: created_by
            .map(|u| u.parse::<UserId>())
            .transpose()
            .map_err(anyhow::Error::from)?,
        created_at,
    })
}

/// Every server's games, by name (the client filters by server).
pub async fn for_ready(db: &SqlitePool) -> AppResult<Vec<Game>> {
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} ORDER BY name_key, id"))
        .fetch_all(db)
        .await?;
    rows.into_iter().map(from_row).collect()
}

/// Whether `game` is in `server`'s registry.
pub async fn in_server(db: &SqlitePool, server: ServerId, game: GameId) -> AppResult<bool> {
    let found: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM games WHERE id = ? AND server_id = ?")
            .bind(game.to_string())
            .bind(server.to_string())
            .fetch_optional(db)
            .await?;
    Ok(found.is_some())
}

/// The name as stored: trimmed, inner whitespace collapsed, 1..=[`MAX_NAME`] characters.
fn clean_name(name: &str) -> AppResult<String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(AppError::BadRequest(format!(
            "a game name is 1 to {MAX_NAME} characters"
        )));
    }
    Ok(name)
}

async fn add(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(server): Path<ServerId>,
    Json(req): Json<AddGameRequest>,
) -> AppResult<Json<Game>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    crate::media::writes_ok(&s.cfg)?;
    if !is_server_member(&s.db, server, me).await? {
        return Err(AppError::Forbidden);
    }
    let name = clean_name(&req.name)?;
    let key = name_key(&name);
    let mut tx = begin_write(&s.db).await?;
    let existing: Option<Row> =
        sqlx::query_as(&format!("{SELECT} WHERE server_id = ? AND name_key = ?"))
            .bind(server.to_string())
            .bind(&key)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(row) = existing {
        return Ok(Json(from_row(row)?));
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM games WHERE server_id = ?")
        .bind(server.to_string())
        .fetch_one(&mut *tx)
        .await?;
    if count >= MAX_PER_SERVER {
        return Err(AppError::BadRequest(
            "This server has too many games.".into(),
        ));
    }
    let id = GameId::new();
    sqlx::query(
        "INSERT INTO games (id, server_id, name, name_key, created_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(server.to_string())
    .bind(&name)
    .bind(&key)
    .bind(me.to_string())
    .bind(now())
    .execute(&mut *tx)
    .await?;
    let row: Row = sqlx::query_as(&format!("{SELECT} WHERE id = ?"))
        .bind(id.to_string())
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    let game = from_row(row)?;
    s.hub
        .publish(&s.db, Event::GameAdded { game: game.clone() })
        .await;
    Ok(Json(game))
}
