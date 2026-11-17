//! Your own order of a server's channel list (#304): `PUT|DELETE /me/channel-orders/{server_id}`. Drawn over the
//! server's layout for you only, so it never touches the layout everyone shares (nor its revision: a drag of yours
//! can't be refused over, or get in the way of, someone else's layout save). Kept per person and server
//! (`channel_orders`) so it follows you to every device; sent only to you (Ready's `channel_orders` and
//! `ChannelOrderChanged`). Delete it to go back to the server's order.
//!
//! What's kept is cleaned against the server as it is when you save: channels and categories of that server only,
//! live and not archived, each once. Later changes to the server (new channels, deleted ones) aren't written back:
//! the client draws whatever the order doesn't name in the server's order, after what it does.

use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{ChannelOrder, ChannelOrderRequest, LayoutCategory};
use sqlx::SqlitePool;

use crate::AppState;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};

/// The most channels (and, separately, categories) one order may name: far more than any list holds.
pub const MAX_NAMED: usize = 500;

pub fn router() -> Router<AppState> {
    Router::new().route("/me/channel-orders/{server_id}", put(set).delete(reset))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Stored {
    uncategorised: Vec<pulse_protocol::ids::ChannelId>,
    categories: Vec<LayoutCategory>,
}

/// `user`'s orders, for servers they're still in (and that aren't deleted).
pub async fn list_for(db: &SqlitePool, user: UserId) -> AppResult<Vec<ChannelOrder>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT o.server_id, o.body FROM channel_orders o
         JOIN server_members m ON m.server_id = o.server_id AND m.user_id = o.user_id
         JOIN servers s ON s.id = o.server_id AND s.deleted_at IS NULL
         WHERE o.user_id = ? ORDER BY o.server_id",
    )
    .bind(user.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|(server, body)| {
            let b: Stored = serde_json::from_str(&body).map_err(anyhow::Error::from)?;
            Ok(ChannelOrder {
                server_id: server.parse().map_err(anyhow::Error::from)?,
                uncategorised: b.uncategorised,
                categories: b.categories,
            })
        })
        .collect()
}

async fn member(s: &AppState, me: UserId, server: ServerId) -> AppResult<()> {
    if !crate::servers::routes::is_server_member(&s.db, server, me).await? {
        return Err(AppError::NotFound);
    }
    if !s.limits.layout_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(Some(
            "You're moving things too fast. Try again in a minute.".into(),
        )));
    }
    Ok(())
}

/// `PUT /me/channel-orders/{server_id}`: your order, cleaned against the server (see the module). Answers with it.
async fn set(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(server): Path<ServerId>,
    Json(req): Json<ChannelOrderRequest>,
) -> AppResult<Json<ChannelOrder>> {
    let named = req.uncategorised.len()
        + req
            .categories
            .iter()
            .map(|c| c.channels.len())
            .sum::<usize>();
    if named > MAX_NAMED || req.categories.len() > MAX_NAMED {
        return Err(AppError::BadRequest("that's too many channels".into()));
    }
    member(&s, me, server).await?;
    let channels: HashSet<String> = sqlx::query_scalar(
        "SELECT id FROM channels WHERE server_id = ? AND deleted_at IS NULL AND archived_at IS NULL",
    )
    .bind(server.to_string())
    .fetch_all(&s.db)
    .await?
    .into_iter()
    .collect();
    let cats: HashSet<String> = sqlx::query_scalar("SELECT id FROM categories WHERE server_id = ?")
        .bind(server.to_string())
        .fetch_all(&s.db)
        .await?
        .into_iter()
        .collect();
    let mut seen = HashSet::new();
    let mut keep = |ids: Vec<pulse_protocol::ids::ChannelId>| -> Vec<_> {
        ids.into_iter()
            .filter(|c| channels.contains(&c.to_string()) && seen.insert(*c))
            .collect()
    };
    let uncategorised = keep(req.uncategorised);
    let mut seen_cats = HashSet::new();
    let mut categories = Vec::new();
    for c in req.categories {
        if cats.contains(&c.id.to_string()) && seen_cats.insert(c.id) {
            categories.push(LayoutCategory {
                id: c.id,
                channels: keep(c.channels),
            });
        }
    }
    let body = serde_json::to_string(&Stored {
        uncategorised: uncategorised.clone(),
        categories: categories.clone(),
    })
    .map_err(anyhow::Error::from)?;
    sqlx::query(
        "INSERT INTO channel_orders (user_id, server_id, body, updated_at) VALUES (?, ?, ?, ?)
         ON CONFLICT (user_id, server_id) DO UPDATE SET body = excluded.body, updated_at = excluded.updated_at",
    )
    .bind(me.to_string())
    .bind(server.to_string())
    .bind(body)
    .bind(now())
    .execute(&s.db)
    .await?;
    let order = ChannelOrder {
        server_id: server,
        uncategorised,
        categories,
    };
    tell(&s, me, server, Some(order.clone())).await;
    Ok(Json(order))
}

/// `DELETE /me/channel-orders/{server_id}`: back to the server's order. `204`, also when you had none.
async fn reset(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(server): Path<ServerId>,
) -> AppResult<axum::http::StatusCode> {
    member(&s, me, server).await?;
    sqlx::query("DELETE FROM channel_orders WHERE user_id = ? AND server_id = ?")
        .bind(me.to_string())
        .bind(server.to_string())
        .execute(&s.db)
        .await?;
    tell(&s, me, server, None).await;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn tell(s: &AppState, me: UserId, server: ServerId, order: Option<ChannelOrder>) {
    s.hub
        .publish(
            &s.db,
            Event::ChannelOrderChanged {
                user_id: me,
                server_id: server,
                order,
            },
        )
        .await;
}
