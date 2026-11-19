//! Your status in one server (#154): `PUT /servers/{id}/my-status`, "My status here" on a server's right-click menu.
//!
//! Same as everywhere (no row), Online, Do Not Disturb or Invisible, for that server only. Its members see it in that
//! server (its member list and voice rows): each viewer's Ready people and `PresenceChanged` carry `in_servers`, one
//! entry per server they share with the person where it's set, built here per viewer ([`in_server`]) from what they
//! may see of the person's main presence. Everywhere else (DMs, groups, Home) they see the main status.
//!
//! What it shows, against the main presence a viewer gets:
//! - Invisible: offline in that server, with the last seen frozen at when it was set (as Appear offline does).
//! - Do Not Disturb: DND there whenever they'd look online.
//! - Online: online there whenever they're connected, even while DND or Invisible everywhere else, unless they appear
//!   offline to that viewer, or have blocked them.
//!
//! Kept per person (`server_status`), sent only to them (Ready's `server_statuses` and `MyServerStatusesChanged`,
//! always the whole list). Their own client applies it to their own alerts (DND there holds them back).

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{
    PresenceStatus, ServerPresence, ServerStatus, SetServerStatusRequest, Status,
};
use sqlx::SqlitePool;

use crate::AppState;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::presence::{PresenceView, Subject};

pub fn router() -> Router<AppState> {
    Router::new().route("/servers/{id}/my-status", put(set))
}

/// `owner`'s statuses, by server.
pub async fn list(db: &SqlitePool, owner: UserId) -> AppResult<Vec<ServerStatus>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT server_id, status FROM server_status WHERE user_id = ? ORDER BY server_id",
    )
    .bind(owner.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .filter_map(|(id, st)| Some((id, Status::parse(&st)?)))
        .map(|(id, status)| {
            Ok(ServerStatus {
                server_id: id
                    .parse()
                    .map_err(|e| AppError::from(anyhow::Error::from(e)))?,
                status,
            })
        })
        .collect()
}

/// `PUT /servers/{id}/my-status`: your status in that server, or Same as everywhere (`status: null`). 404 unless you
/// are in it (#80). Answers with your whole list.
async fn set(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
    Json(req): Json<SetServerStatusRequest>,
) -> AppResult<Json<Vec<ServerStatus>>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    crate::servers::routes::visible_to(&s.db, id, me).await?;
    match req.status {
        Some(status) => {
            sqlx::query(
                "INSERT INTO server_status (user_id, server_id, status, set_at) VALUES (?, ?, ?, ?)
                 ON CONFLICT (user_id, server_id) DO UPDATE SET status = excluded.status, set_at = excluded.set_at
                 WHERE server_status.status != excluded.status",
            )
            .bind(me.to_string())
            .bind(id.to_string())
            .bind(status.as_str())
            .bind(now())
            .execute(&s.db)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM server_status WHERE user_id = ? AND server_id = ?")
                .bind(me.to_string())
                .bind(id.to_string())
                .execute(&s.db)
                .await?;
        }
    }
    let list = list(&s.db, me).await?;
    s.hub
        .publish(
            &s.db,
            Event::MyServerStatusesChanged {
                user_id: me,
                list: list.clone(),
            },
        )
        .await;
    s.hub.sync_presence(&s.db, me).await;
    Ok(Json(list))
}

/// Everyone's statuses: person → server → (status, set at). For Ready and the presence loads.
pub async fn load_all(
    db: &SqlitePool,
) -> anyhow::Result<HashMap<UserId, HashMap<ServerId, (Status, String)>>> {
    let rows: Vec<(String, String, String, String)> =
        sqlx::query_as("SELECT user_id, server_id, status, set_at FROM server_status")
            .fetch_all(db)
            .await?;
    let mut all: HashMap<UserId, HashMap<ServerId, (Status, String)>> = HashMap::new();
    for (u, sv, st, at) in rows {
        if let Some(st) = Status::parse(&st) {
            all.entry(u.parse()?)
                .or_default()
                .insert(sv.parse()?, (st, at));
        }
    }
    Ok(all)
}

/// One person's statuses, by server.
pub async fn load_for(
    db: &SqlitePool,
    user: UserId,
) -> anyhow::Result<HashMap<ServerId, (Status, String)>> {
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT server_id, status, set_at FROM server_status WHERE user_id = ?")
            .bind(user.to_string())
            .fetch_all(db)
            .await?;
    let mut out = HashMap::new();
    for (sv, st, at) in rows {
        if let Some(st) = Status::parse(&st) {
            out.insert(sv.parse()?, (st, at));
        }
    }
    Ok(out)
}

/// The members of each of `servers`, for telling who shares which with a person.
pub async fn members_of(
    db: &SqlitePool,
    servers: impl IntoIterator<Item = ServerId>,
) -> anyhow::Result<HashMap<ServerId, HashSet<UserId>>> {
    let mut out = HashMap::new();
    for sv in servers {
        let ids: Vec<String> =
            sqlx::query_scalar("SELECT user_id FROM server_members WHERE server_id = ?")
                .bind(sv.to_string())
                .fetch_all(db)
                .await?;
        out.insert(sv, ids.iter().map(|i| i.parse()).collect::<Result<_, _>>()?);
    }
    Ok(out)
}

/// How `s` looks to `viewer` inside a server where they've set `status` (set at `set_at`), given `main`, what the
/// viewer may see of them anywhere ([`crate::presence::view_for`]). See the module docs.
pub fn in_server(
    s: &Subject,
    viewer: UserId,
    main: &PresenceView,
    status: Status,
    set_at: &str,
    connected: bool,
    idle: bool,
) -> (PresenceStatus, Option<String>, bool) {
    match status {
        Status::Invisible => {
            if main.online() {
                (PresenceStatus::Offline, Some(set_at.to_string()), false)
            } else {
                (main.status, main.last_seen_at.clone(), false)
            }
        }
        Status::Dnd if main.online() => (PresenceStatus::Dnd, None, false),
        Status::Online if connected && s.may_show_online_to(viewer) => {
            (PresenceStatus::Online, None, idle)
        }
        _ => (main.status, main.last_seen_at.clone(), main.idle),
    }
}

/// The `in_servers` of one viewer's view of `s`: the servers among `shared` (the ones they share) where `s` has a
/// status of its own, in server order.
pub fn views_for(
    s: &Subject,
    viewer: UserId,
    main: &PresenceView,
    shared: impl Fn(ServerId) -> bool,
    connected: bool,
    idle: bool,
) -> Vec<ServerPresence> {
    if viewer == s.id {
        return vec![];
    }
    let mut out: Vec<ServerPresence> = s
        .server_status
        .iter()
        .filter(|(sv, _)| shared(**sv))
        .map(|(sv, (st, at))| {
            let (status, last_seen_at, idle) = in_server(s, viewer, main, *st, at, connected, idle);
            ServerPresence {
                server_id: *sv,
                status,
                last_seen_at,
                idle,
            }
        })
        .collect();
    out.sort_by_key(|p| p.server_id.to_string());
    out
}
