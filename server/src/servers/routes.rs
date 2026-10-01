use std::collections::BTreeSet;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use pulse_protocol::rest::{
    Channel, ChannelKind, CreateChannelRequest, CreateDmRequest, CreateServerRequest, Member,
    Server, User,
};
use sqlx::SqlitePool;

use crate::AppState;
use crate::access::{CHANNEL_COLS, channel_from_row};
use crate::auth::AuthUser;
use crate::auth::routes::load_user;
use crate::db::now;
use crate::error::{AppError, AppResult};

const MAX_GROUP_OTHERS: usize = 9;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/servers", get(list_servers).post(create_server))
        .route("/servers/{id}/join", post(join))
        .route(
            "/servers/{id}/channels",
            get(list_channels).post(create_channel),
        )
        .route("/servers/{id}/members", get(list_members))
        .route("/dms", get(list_dms).post(create_dm))
}

fn clean_name(name: &str) -> AppResult<String> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > 64 {
        return Err(AppError::BadRequest("name must be 1-64 characters".into()));
    }
    Ok(n.to_string())
}

async fn server_exists(db: &SqlitePool, id: ServerId) -> AppResult<()> {
    let hit: Option<i64> = sqlx::query_scalar("SELECT 1 FROM servers WHERE id = ?")
        .bind(id.to_string())
        .fetch_optional(db)
        .await?;
    hit.map(|_| ()).ok_or(AppError::NotFound)
}

pub async fn is_server_member(db: &SqlitePool, server: ServerId, user: UserId) -> AppResult<bool> {
    let hit: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM server_members WHERE server_id = ? AND user_id = ?")
            .bind(server.to_string())
            .bind(user.to_string())
            .fetch_optional(db)
            .await?;
    Ok(hit.is_some())
}

async fn insert_channel(
    tx: &mut sqlx::SqliteConnection,
    server: Option<ServerId>,
    kind: ChannelKind,
    name: Option<&str>,
    position: i64,
) -> AppResult<Channel> {
    let id = ChannelId::new();
    sqlx::query("INSERT INTO channels (id, server_id, kind, name, position, created_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(id.to_string())
        .bind(server.map(|s| s.to_string()))
        .bind(kind.as_str())
        .bind(name)
        .bind(position)
        .bind(now())
        .execute(&mut *tx)
        .await?;
    Ok(Channel {
        id,
        server_id: server,
        kind,
        name: name.map(str::to_string),
        position,
    })
}

pub async fn member_of(db: &SqlitePool, server: ServerId, user: UserId) -> AppResult<Member> {
    let nickname: Option<String> = sqlx::query_scalar(
        "SELECT nickname FROM server_members WHERE server_id = ? AND user_id = ?",
    )
    .bind(server.to_string())
    .bind(user.to_string())
    .fetch_one(db)
    .await?;
    Ok(Member {
        user: load_user(db, user).await?,
        nickname,
    })
}

async fn create_server(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(req): Json<CreateServerRequest>,
) -> AppResult<Json<Server>> {
    let name = clean_name(&req.name)?;
    let id = ServerId::new();
    let at = now();
    let mut tx = s.db.begin().await?;
    sqlx::query("INSERT INTO servers (id, name, created_by, created_at) VALUES (?, ?, ?, ?)")
        .bind(id.to_string())
        .bind(&name)
        .bind(me.to_string())
        .bind(&at)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO server_members (server_id, user_id, joined_at) VALUES (?, ?, ?)")
        .bind(id.to_string())
        .bind(me.to_string())
        .bind(&at)
        .execute(&mut *tx)
        .await?;
    let general = insert_channel(&mut tx, Some(id), ChannelKind::Text, Some("general"), 0).await?;
    let lounge = insert_channel(&mut tx, Some(id), ChannelKind::Voice, Some("Lounge"), 0).await?;
    tx.commit().await?;

    let server = Server {
        id,
        name,
        icon_hash: None,
    };
    s.hub
        .publish(
            &s.db,
            Event::ServerCreated {
                server: server.clone(),
            },
        )
        .await;
    s.hub
        .publish(&s.db, Event::ChannelCreated { channel: general })
        .await;
    s.hub
        .publish(&s.db, Event::ChannelCreated { channel: lounge })
        .await;
    s.hub
        .publish(
            &s.db,
            Event::MemberJoined {
                server_id: id,
                member: member_of(&s.db, id, me).await?,
            },
        )
        .await;
    Ok(Json(server))
}

pub async fn all_servers(db: &SqlitePool) -> AppResult<Vec<Server>> {
    let rows: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT id, name, icon_hash FROM servers ORDER BY id")
            .fetch_all(db)
            .await?;
    rows.into_iter()
        .map(|(id, name, icon_hash)| {
            Ok(Server {
                id: id.parse().map_err(anyhow::Error::from)?,
                name,
                icon_hash,
            })
        })
        .collect()
}

async fn list_servers(State(s): State<AppState>, _: AuthUser) -> AppResult<Json<Vec<Server>>> {
    Ok(Json(all_servers(&s.db).await?))
}

async fn join(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
) -> AppResult<StatusCode> {
    server_exists(&s.db, id).await?;
    let added = sqlx::query(
        "INSERT OR IGNORE INTO server_members (server_id, user_id, joined_at) VALUES (?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(me.to_string())
    .bind(now())
    .execute(&s.db)
    .await?
    .rows_affected();
    if added > 0 {
        s.hub
            .publish(
                &s.db,
                Event::MemberJoined {
                    server_id: id,
                    member: member_of(&s.db, id, me).await?,
                },
            )
            .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn server_channels(db: &SqlitePool, id: ServerId) -> AppResult<Vec<Channel>> {
    let rows = sqlx::query_as(&format!(
        "SELECT {CHANNEL_COLS} FROM channels WHERE server_id = ? ORDER BY kind, position, id"
    ))
    .bind(id.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter().map(channel_from_row).collect()
}

async fn list_channels(
    State(s): State<AppState>,
    _: AuthUser,
    Path(id): Path<ServerId>,
) -> AppResult<Json<Vec<Channel>>> {
    server_exists(&s.db, id).await?;
    Ok(Json(server_channels(&s.db, id).await?))
}

async fn create_channel(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
    Json(req): Json<CreateChannelRequest>,
) -> AppResult<Json<Channel>> {
    server_exists(&s.db, id).await?;
    if !is_server_member(&s.db, id, me).await? {
        return Err(AppError::Forbidden);
    }
    if req.kind.is_private() {
        return Err(AppError::BadRequest(
            "server channels are text or voice".into(),
        ));
    }
    let name = clean_name(&req.name)?;
    let mut tx = s.db.begin().await?;
    let pos: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM channels WHERE server_id = ? AND kind = ?",
    )
    .bind(id.to_string())
    .bind(req.kind.as_str())
    .fetch_one(&mut *tx)
    .await?;
    let ch = insert_channel(&mut tx, Some(id), req.kind, Some(&name), pos).await?;
    tx.commit().await?;
    s.hub
        .publish(
            &s.db,
            Event::ChannelCreated {
                channel: ch.clone(),
            },
        )
        .await;
    Ok(Json(ch))
}

pub async fn server_members(db: &SqlitePool, id: ServerId) -> AppResult<Vec<Member>> {
    let rows: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT u.id, u.username, u.avatar_hash, m.nickname FROM server_members m JOIN users u ON u.id = m.user_id
         WHERE m.server_id = ? ORDER BY m.joined_at",
    )
    .bind(id.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|(uid, username, avatar_hash, nickname)| {
            Ok(Member {
                user: User {
                    id: uid.parse().map_err(anyhow::Error::from)?,
                    username,
                    avatar_hash,
                },
                nickname,
            })
        })
        .collect()
}

async fn list_members(
    State(s): State<AppState>,
    _: AuthUser,
    Path(id): Path<ServerId>,
) -> AppResult<Json<Vec<Member>>> {
    server_exists(&s.db, id).await?;
    Ok(Json(server_members(&s.db, id).await?))
}

pub async fn dms_of(db: &SqlitePool, user: UserId) -> AppResult<Vec<Channel>> {
    let rows = sqlx::query_as(&format!(
        "SELECT {} FROM channels c JOIN channel_members cm ON cm.channel_id = c.id WHERE cm.user_id = ? ORDER BY c.id",
        CHANNEL_COLS.split(", ").map(|c| format!("c.{c}")).collect::<Vec<_>>().join(", ")
    ))
    .bind(user.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter().map(channel_from_row).collect()
}

async fn list_dms(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
) -> AppResult<Json<Vec<Channel>>> {
    Ok(Json(dms_of(&s.db, me).await?))
}

async fn create_dm(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(req): Json<CreateDmRequest>,
) -> AppResult<Json<Channel>> {
    let others: BTreeSet<UserId> = req.user_ids.into_iter().filter(|u| *u != me).collect();
    if others.is_empty() || others.len() > MAX_GROUP_OTHERS {
        return Err(AppError::BadRequest(format!(
            "pick 1-{MAX_GROUP_OTHERS} other people"
        )));
    }
    for u in &others {
        let hit: Option<i64> = sqlx::query_scalar("SELECT 1 FROM users WHERE id = ?")
            .bind(u.to_string())
            .fetch_optional(&s.db)
            .await?;
        if hit.is_none() {
            return Err(AppError::BadRequest("unknown user".into()));
        }
    }

    let mut tx = s.db.begin().await?;
    if others.len() == 1 {
        let other = others.iter().next().unwrap().to_string();
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT c.id FROM channels c
             JOIN channel_members a ON a.channel_id = c.id AND a.user_id = ?
             JOIN channel_members b ON b.channel_id = c.id AND b.user_id = ?
             WHERE c.kind = 'dm' LIMIT 1",
        )
        .bind(me.to_string())
        .bind(&other)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(id) = existing {
            tx.rollback().await?;
            let ch = crate::access::load_channel(&s.db, id.parse().map_err(anyhow::Error::from)?)
                .await?;
            return ch.map(Json).ok_or(AppError::NotFound);
        }
    }
    let kind = if others.len() == 1 {
        ChannelKind::Dm
    } else {
        ChannelKind::Group
    };
    let ch = insert_channel(&mut tx, None, kind, None, 0).await?;
    let at = now();
    for u in std::iter::once(me).chain(others.iter().copied()) {
        sqlx::query("INSERT INTO channel_members (channel_id, user_id, added_by, added_at) VALUES (?, ?, ?, ?)")
            .bind(ch.id.to_string())
            .bind(u.to_string())
            .bind(me.to_string())
            .bind(&at)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    s.hub
        .publish(
            &s.db,
            Event::ChannelCreated {
                channel: ch.clone(),
            },
        )
        .await;
    Ok(Json(ch))
}
