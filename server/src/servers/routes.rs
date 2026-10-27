use std::collections::{BTreeSet, HashMap};

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
use crate::limits::caps;

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
    // Quotes channels start in one-reaction-per-person mode; everything else reacts freely.
    let reaction_mode = if kind == ChannelKind::Quotes {
        "one_per_person"
    } else {
        "normal"
    };
    sqlx::query("INSERT INTO channels (id, server_id, kind, name, position, created_at, reaction_mode) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(id.to_string())
        .bind(server.map(|s| s.to_string()))
        .bind(kind.as_str())
        .bind(name)
        .bind(position)
        .bind(now())
        .bind(reaction_mode)
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

/// 400 if any of `users` is already in [`caps::GROUPS_PER_USER`] groups. Run it in the write
/// transaction that adds them, so concurrent creates and adds can't push anyone past it.
pub async fn check_group_cap(tx: &mut sqlx::SqliteConnection, users: &[UserId]) -> AppResult<()> {
    for u in users {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM channel_members cm JOIN channels c ON c.id = cm.channel_id
             WHERE cm.user_id = ? AND c.kind = 'group'",
        )
        .bind(u.to_string())
        .fetch_one(&mut *tx)
        .await?;
        if n >= caps::GROUPS_PER_USER {
            return Err(AppError::BadRequest(format!(
                "someone here is already in {} groups, the most allowed",
                caps::GROUPS_PER_USER
            )));
        }
    }
    Ok(())
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
    if !s.limits.create_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    let name = clean_name(&req.name)?;
    let id = ServerId::new();
    let at = now();
    let mut tx = crate::db::begin_write(&s.db).await?;
    let mine: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM servers WHERE created_by = ?")
        .bind(me.to_string())
        .fetch_one(&mut *tx)
        .await?;
    if mine >= caps::SERVERS_PER_USER {
        return Err(AppError::BadRequest(format!(
            "you can create up to {} servers",
            caps::SERVERS_PER_USER
        )));
    }
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
    crate::reads::init_point(&mut tx, me, general.id).await?;
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
    // A join changes a live Quote of the Year's member count ("N of M"), which its events carry
    // as absolute state: publish in commit order (in_order, then the write lock).
    let _order = s.hub.in_order().await;
    // One transaction, under the write lock: a channel created concurrently is either seen here
    // (and gets a read point) or sees us as a member in create_channel.
    let mut tx = crate::db::begin_write(&s.db).await?;
    let added = sqlx::query(
        "INSERT OR IGNORE INTO server_members (server_id, user_id, joined_at) VALUES (?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(me.to_string())
    .bind(now())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if added > 0 {
        // Start at "all read": no wall of old unreads in a server you just joined.
        let chat: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM channels WHERE server_id = ? AND kind IN ('text', 'quotes')",
        )
        .bind(id.to_string())
        .fetch_all(&mut *tx)
        .await?;
        for ch in chat {
            crate::reads::init_point(&mut tx, me, ch.parse().map_err(anyhow::Error::from)?).await?;
        }
    }
    let qoty = if added > 0 {
        crate::qoty::live_in(&mut tx, id).await?
    } else {
        None
    };
    tx.commit().await?;
    // The join has landed: a failed read-back is logged, and the rest still goes out.
    if added > 0 {
        match member_of(&s.db, id, me).await {
            Ok(member) => {
                s.hub
                    .publish(
                        &s.db,
                        Event::MemberJoined {
                            server_id: id,
                            member,
                        },
                    )
                    .await;
            }
            Err(e) => {
                tracing::error!(error = ?e, server = %id, %me, "couldn't read back a join to publish");
            }
        }
    }
    if let Some(event) = qoty {
        crate::qoty::publish_updated(&s, event).await;
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

/// Every server's channels in one query, each list ordered as [`server_channels`] orders it.
pub async fn all_server_channels(db: &SqlitePool) -> AppResult<HashMap<ServerId, Vec<Channel>>> {
    let rows = sqlx::query_as(&format!(
        "SELECT {CHANNEL_COLS} FROM channels WHERE server_id IS NOT NULL
         ORDER BY server_id, kind, position, id"
    ))
    .fetch_all(db)
    .await?;
    let mut out: HashMap<ServerId, Vec<Channel>> = HashMap::new();
    for row in rows {
        let ch = channel_from_row(row)?;
        if let Some(sid) = ch.server_id {
            out.entry(sid).or_default().push(ch);
        }
    }
    Ok(out)
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
    if !s.limits.create_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    server_exists(&s.db, id).await?;
    if !is_server_member(&s.db, id, me).await? {
        return Err(AppError::Forbidden);
    }
    if req.kind.is_private() {
        return Err(AppError::BadRequest(
            "server channels are text, voice or quotes".into(),
        ));
    }
    let name = clean_name(&req.name)?;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channels WHERE server_id = ?")
        .bind(id.to_string())
        .fetch_one(&mut *tx)
        .await?;
    if count >= caps::CHANNELS_PER_SERVER {
        return Err(AppError::BadRequest(format!(
            "a server can have up to {} channels",
            caps::CHANNELS_PER_SERVER
        )));
    }
    let pos: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM channels WHERE server_id = ? AND kind = ?",
    )
    .bind(id.to_string())
    .bind(req.kind.as_str())
    .fetch_one(&mut *tx)
    .await?;
    let ch = insert_channel(&mut tx, Some(id), req.kind, Some(&name), pos).await?;
    if req.kind.is_server_chat() {
        // Everyone in the server starts at "all read" (the channel is empty).
        let members: Vec<String> =
            sqlx::query_scalar("SELECT user_id FROM server_members WHERE server_id = ?")
                .bind(id.to_string())
                .fetch_all(&mut *tx)
                .await?;
        for m in members {
            crate::reads::init_point(&mut tx, m.parse().map_err(anyhow::Error::from)?, ch.id)
                .await?;
        }
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

fn member_from_row(
    uid: String,
    username: String,
    avatar_hash: Option<String>,
    nickname: Option<String>,
) -> AppResult<Member> {
    Ok(Member {
        user: User {
            id: uid.parse().map_err(anyhow::Error::from)?,
            username,
            avatar_hash,
        },
        nickname,
    })
}

pub async fn server_members(db: &SqlitePool, id: ServerId) -> AppResult<Vec<Member>> {
    let rows: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT u.id, u.username, u.avatar_hash, m.nickname FROM server_members m JOIN users u ON u.id = m.user_id
         WHERE m.server_id = ? ORDER BY m.joined_at, m.user_id",
    )
    .bind(id.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|(uid, username, avatar_hash, nickname)| {
            member_from_row(uid, username, avatar_hash, nickname)
        })
        .collect()
}

/// server id, user id, username, avatar hash, nickname.
type ServerMemberRow = (String, String, String, Option<String>, Option<String>);

/// Every server's members in one query, each list ordered as [`server_members`] orders it.
pub async fn all_server_members(db: &SqlitePool) -> AppResult<HashMap<ServerId, Vec<Member>>> {
    let rows: Vec<ServerMemberRow> = sqlx::query_as(
        "SELECT m.server_id, u.id, u.username, u.avatar_hash, m.nickname
         FROM server_members m JOIN users u ON u.id = m.user_id
         ORDER BY m.server_id, m.joined_at, m.user_id",
    )
    .fetch_all(db)
    .await?;
    let mut out: HashMap<ServerId, Vec<Member>> = HashMap::new();
    for (sid, uid, username, avatar_hash, nickname) in rows {
        let sid: ServerId = sid.parse().map_err(anyhow::Error::from)?;
        out.entry(sid)
            .or_default()
            .push(member_from_row(uid, username, avatar_hash, nickname)?);
    }
    Ok(out)
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

/// The 1:1 DM between `a` and `b`, if there is one.
async fn existing_dm<'e>(
    db: impl sqlx::SqliteExecutor<'e>,
    a: UserId,
    b: UserId,
) -> AppResult<Option<String>> {
    Ok(sqlx::query_scalar(
        "SELECT c.id FROM channels c
         JOIN channel_members a ON a.channel_id = c.id AND a.user_id = ?
         JOIN channel_members b ON b.channel_id = c.id AND b.user_id = ?
         WHERE c.kind = 'dm' LIMIT 1",
    )
    .bind(a.to_string())
    .bind(b.to_string())
    .fetch_optional(db)
    .await?)
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
    // A group is always new, so it pays now; a 1:1 DM pays only if it doesn't exist yet.
    if others.len() > 1 && !s.limits.create_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    // Out of creates: refuse a new 1:1 here, before the lookups and the write lock (a plain
    // read; the spend below, under the lock, is what counts).
    if others.len() == 1
        && !s.limits.create_user.allowed(&me.to_string())
        && existing_dm(&s.db, me, *others.iter().next().unwrap())
            .await?
            .is_none()
    {
        return Err(AppError::TooManyRequests);
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

    let mut tx = crate::db::begin_write(&s.db).await?;
    if others.len() == 1 {
        let other = *others.iter().next().unwrap();
        let existing = existing_dm(&mut *tx, me, other).await?;
        if let Some(id) = existing {
            let channel_id: ChannelId = id.parse().map_err(anyhow::Error::from)?;
            // Starting a DM you had closed reopens it.
            let reopened =
                sqlx::query("DELETE FROM dm_hidden WHERE user_id = ? AND channel_id = ?")
                    .bind(me.to_string())
                    .bind(&id)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
            tx.commit().await?;
            if reopened > 0 {
                s.hub
                    .publish(
                        &s.db,
                        Event::ConversationVisibility {
                            user_id: me,
                            channel_id,
                            hidden: false,
                        },
                    )
                    .await;
            }
            let ch = crate::access::load_channel(&s.db, channel_id).await?;
            return ch.map(Json).ok_or(AppError::NotFound);
        }
        if !s.limits.create_user.hit(&me.to_string()) {
            return Err(AppError::TooManyRequests); // dropping `tx` rolls back
        }
    }
    let kind = if others.len() == 1 {
        ChannelKind::Dm
    } else {
        ChannelKind::Group
    };
    if kind == ChannelKind::Group {
        let everyone: Vec<UserId> = std::iter::once(me).chain(others.iter().copied()).collect();
        check_group_cap(&mut tx, &everyone).await?; // dropping `tx` rolls back
    }
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
        crate::reads::init_point(&mut tx, u, ch.id).await?;
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
    let members: Vec<UserId> = std::iter::once(me).chain(others.iter().copied()).collect();
    s.hub
        .publish(
            &s.db,
            Event::GroupMembersChanged {
                channel_id: ch.id,
                user_ids: members,
            },
        )
        .await;
    if kind == ChannelKind::Group {
        let me_name = crate::auth::routes::load_user(&s.db, me).await?.username;
        let mut names = Vec::new();
        for u in &others {
            names.push(crate::auth::routes::load_user(&s.db, *u).await?.username);
        }
        crate::messages::system::post(
            &s,
            ch.id,
            format!(
                "{me_name} created the group with {}",
                crate::messages::system::join_names(&names)
            ),
            serde_json::json!({"type": "group_created", "by": me, "with": others}),
        )
        .await?;
    }
    Ok(Json(ch))
}
