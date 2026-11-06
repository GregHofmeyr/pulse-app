//! Server nicknames and the Name Archive (#48). Anyone in a server can set anyone's nickname
//! there, their own included, or reset it. Each change is a `nickname_history` row written in the
//! same transaction, and it reaches everyone live as `MemberUpdated`; no line is posted anywhere.

use axum::extract::{Path, Query, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ServerId, UserId, next_ulid};
use pulse_protocol::rest::{Member, NameArchivePage, NameChange, SetNicknameRequest};

use super::routes::{member_of, require_member};
use crate::AppState;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::profiles::names;
use crate::profiles::private::too_fast;

/// Name Archive rows per page.
pub const ARCHIVE_PAGE: usize = 50;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/servers/{id}/members/{user}/nickname", put(rename))
        .route("/servers/{id}/members/{user}/names", get(archive))
}

/// A malformed user id names no one: 404 like an unknown one.
fn user_id(s: &str) -> AppResult<UserId> {
    s.parse().map_err(|_| AppError::NotFound)
}

async fn rename(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path((server, user)): Path<(ServerId, String)>,
    Json(req): Json<SetNicknameRequest>,
) -> AppResult<Json<Member>> {
    let target = user_id(&user)?;
    let nickname = names::clean_nickname(req.nickname.as_deref())?;
    require_member(&s.db, server, me).await?;
    if !s.limits.rename_user.hit(&me.to_string()) {
        return Err(too_fast());
    }

    // Hold the order guard so two quick renames publish in commit order.
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let current: Option<Option<String>> = sqlx::query_scalar(
        "SELECT nickname FROM server_members WHERE server_id = ? AND user_id = ?",
    )
    .bind(server.to_string())
    .bind(target.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    let Some(current) = current else {
        return Err(AppError::NotFound);
    };
    if current == nickname {
        // Nothing changed: nothing to archive or announce.
        tx.rollback().await?;
        return Ok(Json(member_of(&s.db, server, target).await?));
    }
    sqlx::query("UPDATE server_members SET nickname = ? WHERE server_id = ? AND user_id = ?")
        .bind(&nickname)
        .bind(server.to_string())
        .bind(target.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO nickname_history \
         (id, server_id, target_user_id, changed_by, old_nick, new_nick, changed_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(next_ulid().to_string())
    .bind(server.to_string())
    .bind(target.to_string())
    .bind(me.to_string())
    .bind(&current)
    .bind(&nickname)
    .bind(now())
    .execute(&mut *tx)
    .await?;
    // Someone else giving you a name counts toward Identity Crisis; resets and your own don't.
    let renamed = target != me && nickname.is_some();
    if renamed {
        let stat = crate::badges::catalogue::stat::RENAMED_BY_OTHERS;
        crate::badges::stats::add(&mut tx, target, stat, 1).await?;
    }
    // Whoever saves a Zalgo nickname earns the hidden Zalgo badge (#210). Private nicknames
    // don't: private activity never earns a badge.
    let zalgo = nickname.as_deref().is_some_and(names::is_zalgo);
    if zalgo {
        let stat = crate::badges::catalogue::stat::ZALGO_NAME;
        crate::badges::stats::set_max(&mut tx, me, stat, 1).await?;
    }
    tx.commit().await?;

    let member = member_of(&s.db, server, target).await?;
    s.hub
        .publish(
            &s.db,
            Event::MemberUpdated {
                server_id: server,
                member: member.clone(),
            },
        )
        .await;
    let check: Vec<UserId> = [(renamed, target), (zalgo, me)]
        .into_iter()
        .filter_map(|(yes, who)| yes.then_some(who))
        .collect();
    crate::badges::engine::after(&s, &check).await;
    Ok(Json(member))
}

#[derive(serde::Deserialize)]
struct ArchiveQuery {
    cursor: Option<String>,
}

/// A page of someone's nicknames in one server, newest first. Members only.
async fn archive(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path((server, user)): Path<(ServerId, String)>,
    Query(q): Query<ArchiveQuery>,
) -> AppResult<Json<NameArchivePage>> {
    let target = user_id(&user)?;
    require_member(&s.db, server, me).await?;
    // The cursor is the last row's `changed_at|id`.
    let after = match q.cursor.as_deref() {
        Some(c) => Some(
            c.split_once('|')
                .ok_or_else(|| AppError::BadRequest("bad cursor".into()))?,
        ),
        None => None,
    };
    let mut tx = s.db.begin().await?;
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM nickname_history WHERE server_id = ? AND target_user_id = ?",
    )
    .bind(server.to_string())
    .bind(target.to_string())
    .fetch_one(&mut *tx)
    .await?;
    // No cursor: start above every timestamp.
    let (at, id) = after.unwrap_or(("\u{10ffff}", ""));
    let rows: Vec<(String, Option<String>, String, String)> = sqlx::query_as(
        "SELECT id, new_nick, changed_by, changed_at FROM nickname_history \
         WHERE server_id = ? AND target_user_id = ? \
           AND (changed_at < ? OR (changed_at = ? AND id < ?)) \
         ORDER BY changed_at DESC, id DESC LIMIT ?",
    )
    .bind(server.to_string())
    .bind(target.to_string())
    .bind(at)
    .bind(at)
    .bind(id)
    .bind(ARCHIVE_PAGE as i64 + 1)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    let more = rows.len() > ARCHIVE_PAGE;
    let rows = &rows[..rows.len().min(ARCHIVE_PAGE)];
    let next_cursor = more
        .then(|| rows.last().map(|(id, _, _, at)| format!("{at}|{id}")))
        .flatten();
    let entries = rows
        .iter()
        .map(|(_, name, by, at)| {
            Ok(NameChange {
                name: name.clone(),
                changed_by: by.parse().map_err(anyhow::Error::from)?,
                changed_at: at.clone(),
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    Ok(Json(NameArchivePage {
        entries,
        total: total as u32,
        next_cursor,
    }))
}
