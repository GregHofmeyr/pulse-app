//! Speaker links: "this plain name is that member", per server. Lines keep their raw speaker, so
//! linking, re-pointing and undoing never touch them; reads resolve through `effective_speaker`.
//! Anyone in the server may link, and linking is not an edit, so it works whatever a vote is doing.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, put};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{LinkSpeakerRequest, SpeakerLink};
use sqlx::{SqliteConnection, SqlitePool};

use super::{MAX_NAME, name_key, parse_id};
use crate::AppState;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::servers::routes::is_server_member;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/servers/{id}/speaker-links", put(link))
        .route("/servers/{id}/speaker-links/{name_key}", delete(unlink))
}

/// Link a plain name to a member, or re-point an existing link. The name must be on a live line in
/// this server. A repeat of the current link changes nothing and publishes nothing.
async fn link(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(server): Path<ServerId>,
    Json(req): Json<LinkSpeakerRequest>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    if !is_server_member(&s.db, server, me).await? {
        return Err(AppError::Forbidden);
    }
    let name = req.name.trim();
    let key = name_key(name);
    if key.is_empty() || name.chars().count() > MAX_NAME {
        return Err(AppError::BadRequest(format!(
            "a name must be 1-{MAX_NAME} characters"
        )));
    }

    // Events carry absolute state, so publish in commit order (in_order, then the write lock).
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let on_a_line: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM quote_lines l
         JOIN messages msg ON msg.id = l.message_id AND msg.deleted_at IS NULL
         JOIN channels c ON c.id = msg.channel_id
         WHERE c.server_id = ? AND l.speaker_key = ? LIMIT 1",
    )
    .bind(server.to_string())
    .bind(&key)
    .fetch_optional(&mut *tx)
    .await?;
    if on_a_line.is_none() {
        return Err(AppError::BadRequest(
            "no quote in this server has that name".into(),
        ));
    }
    let member: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM server_members WHERE server_id = ? AND user_id = ?")
            .bind(server.to_string())
            .bind(req.user_id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    if member.is_none() {
        return Err(AppError::BadRequest(
            "that person isn't in this server".into(),
        ));
    }
    let current: Option<String> = sqlx::query_scalar(
        "SELECT user_id FROM speaker_links WHERE server_id = ? AND name_key = ?",
    )
    .bind(server.to_string())
    .bind(&key)
    .fetch_optional(&mut *tx)
    .await?;
    if current.as_deref() == Some(req.user_id.to_string().as_str()) {
        return Ok(StatusCode::NO_CONTENT);
    }
    sqlx::query(
        "INSERT INTO speaker_links (server_id, name_key, name, user_id, linked_by, linked_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (server_id, name_key) DO UPDATE SET
           name = excluded.name, user_id = excluded.user_id,
           linked_by = excluded.linked_by, linked_at = excluded.linked_at",
    )
    .bind(server.to_string())
    .bind(&key)
    .bind(name)
    .bind(req.user_id.to_string())
    .bind(me.to_string())
    .bind(now())
    .execute(&mut *tx)
    .await?;
    let links = links_of(&mut tx, server).await?;
    tx.commit().await?;
    publish(&s, server, links).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Undo a link: the lines go back to the plain name.
async fn unlink(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path((server, key)): Path<(ServerId, String)>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    if !is_server_member(&s.db, server, me).await? {
        return Err(AppError::Forbidden);
    }
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let removed = sqlx::query("DELETE FROM speaker_links WHERE server_id = ? AND name_key = ?")
        .bind(server.to_string())
        .bind(name_key(&key))
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(AppError::NotFound);
    }
    let links = links_of(&mut tx, server).await?;
    tx.commit().await?;
    publish(&s, server, links).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn publish(s: &AppState, server: ServerId, links: Vec<SpeakerLink>) {
    s.hub
        .publish(
            &s.db,
            Event::SpeakerLinksChanged {
                server_id: server,
                links,
            },
        )
        .await;
}

type Row = (String, String, String, String, String, String);

fn link_of((server, key, name, user, by, at): Row) -> AppResult<SpeakerLink> {
    Ok(SpeakerLink {
        server_id: parse_id::<ServerId>(&server)?,
        name_key: key,
        name,
        user_id: parse_id::<UserId>(&user)?,
        linked_by: parse_id::<UserId>(&by)?,
        linked_at: at,
    })
}

const COLUMNS: &str =
    "SELECT server_id, name_key, name, user_id, linked_by, linked_at FROM speaker_links";

/// The server's full list, in a stable order.
async fn links_of(db: &mut SqliteConnection, server: ServerId) -> AppResult<Vec<SpeakerLink>> {
    let rows: Vec<Row> =
        sqlx::query_as(&format!("{COLUMNS} WHERE server_id = ? ORDER BY name_key"))
            .bind(server.to_string())
            .fetch_all(db)
            .await?;
    rows.into_iter().map(link_of).collect()
}

/// Every server's links, for Ready. Same audience as the event: server-wide, so everyone gets them
/// (Ready also lists every server).
pub async fn for_ready(db: &SqlitePool) -> AppResult<Vec<SpeakerLink>> {
    let rows: Vec<Row> = sqlx::query_as(&format!("{COLUMNS} ORDER BY server_id, name_key"))
        .fetch_all(db)
        .await?;
    rows.into_iter().map(link_of).collect()
}
