//! Speaker links: "this plain name is that member", per server. Lines keep their raw speaker, so
//! linking, re-pointing and undoing never touch them; reads resolve through `effective_speaker`.
//! Anyone in the server may link, and linking is not an edit, so it works whatever a vote is doing.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, put};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{MessageId, ServerId, UserId};
use pulse_protocol::rest::{LinkSpeakerRequest, SpeakerLink};
use sqlx::{SqliteConnection, SqlitePool};

use super::{clean_name, name_key, parse_id};
use crate::AppState;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::messages::routes::load;

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
    limit(&s, me)?;
    crate::servers::routes::server_exists(&s.db, server).await?;
    let name = clean_name(&req.name)?;
    let key = name_key(name);

    // Events carry absolute state, so publish in commit order (in_order, then the write lock).
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    // A vote whose time ran out is over even if no sweep has ended it yet: end it first, so its
    // wins snapshot the speakers as they were at the end, not as this changes them.
    let overdue = crate::qoty::ending::end_if_overdue(&mut tx, server).await?;
    if !member_in(&mut tx, server, me).await? {
        return Err(AppError::Forbidden);
    }
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
    if !member_in(&mut tx, server, req.user_id).await? {
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
    let new = req.user_id.to_string();
    if current.as_deref() == Some(new.as_str()) {
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
    let moved: Vec<String> = current.into_iter().chain([new]).collect();
    let quotes = moved_quotes(&mut tx, server, &key, &moved).await?;
    tx.commit().await?;
    if let Some(ended) = overdue {
        crate::qoty::ending::announce(&s, ended).await;
    }
    publish(&s, server, links, quotes).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Undo a link: the lines go back to the plain name.
async fn unlink(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path((server, key)): Path<(ServerId, String)>,
) -> AppResult<StatusCode> {
    limit(&s, me)?;
    crate::servers::routes::server_exists(&s.db, server).await?;
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    // A vote whose time ran out is over even if no sweep has ended it yet: end it first, so its
    // wins snapshot the speakers as they were at the end, not as this changes them.
    let overdue = crate::qoty::ending::end_if_overdue(&mut tx, server).await?;
    if !member_in(&mut tx, server, me).await? {
        return Err(AppError::Forbidden);
    }
    let key = name_key(&key);
    let removed: Option<String> = sqlx::query_scalar(
        "DELETE FROM speaker_links WHERE server_id = ? AND name_key = ? RETURNING user_id",
    )
    .bind(server.to_string())
    .bind(&key)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(removed) = removed else {
        return Err(AppError::NotFound);
    };
    let links = links_of(&mut tx, server).await?;
    let quotes = moved_quotes(&mut tx, server, &key, &[removed]).await?;
    tx.commit().await?;
    if let Some(ended) = overdue {
        crate::qoty::ending::announce(&s, ended).await;
    }
    publish(&s, server, links, quotes).await;
    Ok(StatusCode::NO_CONTENT)
}

fn limit(s: &AppState, me: UserId) -> AppResult<()> {
    if s.limits.mutate_user.hit(&me.to_string()) {
        Ok(())
    } else {
        Err(AppError::TooManyRequests(None))
    }
}

async fn member_in(db: &mut SqliteConnection, server: ServerId, user: UserId) -> AppResult<bool> {
    let hit: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM server_members WHERE server_id = ? AND user_id = ?")
            .bind(server.to_string())
            .bind(user.to_string())
            .fetch_optional(db)
            .await?;
    Ok(hit.is_some())
}

/// Publish the server's links, then a fresh copy of each of `quotes` (see [`moved_quotes`]).
/// Called after commit, under the caller's `in_order` guard. The link has landed by then, so a
/// quote that can't be read back is logged and skipped rather than failing the request.
async fn publish(s: &AppState, server: ServerId, links: Vec<SpeakerLink>, quotes: Vec<MessageId>) {
    s.hub
        .publish(
            &s.db,
            Event::SpeakerLinksChanged {
                server_id: server,
                links,
            },
        )
        .await;
    for id in quotes {
        match load(&s.db, None, id).await {
            Ok(Some(message)) => {
                s.hub
                    .publish(&s.db, Event::MessageUpdated { message })
                    .await;
            }
            Ok(None) => {}
            Err(e) => {
                tracing::error!(error = ?e, %id, "couldn't load a quote a link change moved");
            }
        }
    }
}

/// The live quotes of `server` with a line under `key` that one of `users` awarded, oldest first:
/// their counted awards move with the link, and clients can't derive that.
async fn moved_quotes(
    db: &mut SqliteConnection,
    server: ServerId,
    key: &str,
    users: &[String],
) -> AppResult<Vec<MessageId>> {
    let mut q = sqlx::QueryBuilder::new(
        "SELECT DISTINCT msg.id FROM quote_lines l
         JOIN messages msg ON msg.id = l.message_id AND msg.deleted_at IS NULL
         JOIN channels c ON c.id = msg.channel_id
         JOIN quote_awards a ON a.line_id = l.id
         WHERE c.server_id = ",
    );
    q.push_bind(server.to_string());
    q.push(" AND l.speaker_key = ").push_bind(key);
    q.push(" AND a.giver_id IN (");
    let mut list = q.separated(", ");
    for u in users {
        list.push_bind(u);
    }
    q.push(") ORDER BY msg.id");
    let ids: Vec<String> = q.build_query_scalar().fetch_all(db).await?;
    ids.iter().map(|id| parse_id::<MessageId>(id)).collect()
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

/// Every server's links, for Ready: it returns all servers' links, matching `Audience::Everyone`
/// for `SpeakerLinksChanged` (Ready lists every server to every user too).
pub async fn for_ready(db: &SqlitePool) -> AppResult<Vec<SpeakerLink>> {
    let rows: Vec<Row> = sqlx::query_as(&format!("{COLUMNS} ORDER BY server_id, name_key"))
        .fetch_all(db)
        .await?;
    rows.into_iter().map(link_of).collect()
}
