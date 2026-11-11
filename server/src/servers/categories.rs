//! Channel categories (#54): named headings that hold a server's channels of any kind, and the
//! order of everything in a server's channel list (#55).
//!
//! The server owns the layout. Each category has a position among the server's categories; each
//! live channel has a `category_id` (or none: the uncategorised area at the top) and a position
//! inside it. A per-server `layout_revision` goes up with every change to any of that (a category
//! made, renamed or deleted, a new order saved, a channel created, deleted or restored), and every
//! change publishes the server's whole layout (`LayoutChanged`, to everyone, as channels are
//! public). A save names the revision it was based on and the whole order; anything stale or
//! incomplete is refused rather than merged, so two people dragging at once can't leave a mix of
//! both.
//!
//! Anyone in the server may change it (the v1 trust model, as for channels). Every write here takes
//! `hub.in_order()` before its transaction and holds it until its layout is published, so layouts
//! reach clients in revision order.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{patch, post, put};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{CategoryId, ServerId, UserId};
use pulse_protocol::rest::{
    Category, ChannelKind, ChannelPlacement, CreateCategoryRequest, RenameCategoryRequest,
    SaveLayoutRequest, ServerLayout,
};
use sqlx::{SqliteConnection, SqlitePool};

use crate::AppState;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::limits::caps;
use crate::servers::routes::{clean_name, require_member};

/// What a refused save says. The client shows it as its draft snaps back to the order everyone
/// has, so it speaks to the person, not about revisions.
const STALE: &str = "The order changed just now, so it's back to how it is for everyone.";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/servers/{id}/categories", post(create))
        .route("/servers/{id}/layout", put(save_layout))
        .route("/categories/{id}", patch(rename).delete(delete))
}

/// Adds a category to `server` at `position`. `default_kind` is the kind a default category is
/// made for (a new server's), `None` for one people make.
pub async fn insert_category(
    tx: &mut SqliteConnection,
    server: ServerId,
    name: &str,
    position: i64,
    default_kind: Option<ChannelKind>,
) -> AppResult<Category> {
    let id = CategoryId::new();
    sqlx::query(
        "INSERT INTO categories (id, server_id, name, position, default_kind, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(server.to_string())
    .bind(name)
    .bind(position)
    .bind(default_kind.map(|k| k.as_str()))
    .bind(now())
    .execute(&mut *tx)
    .await?;
    Ok(Category {
        id,
        server_id: server,
        name: name.to_string(),
        position,
        default_kind,
    })
}

type CategoryRow = (String, String, String, i64, Option<String>);

const CATEGORY_COLS: &str = "id, server_id, name, position, default_kind";

fn category_from_row((id, server, name, position, kind): CategoryRow) -> AppResult<Category> {
    Ok(Category {
        id: id.parse().map_err(anyhow::Error::from)?,
        server_id: server.parse().map_err(anyhow::Error::from)?,
        name,
        position,
        default_kind: kind
            .map(|k| ChannelKind::parse(&k).ok_or_else(|| anyhow::anyhow!("bad category kind {k}")))
            .transpose()?,
    })
}

/// `server`'s whole layout: its categories in order and every live channel's placement (by
/// category, then position). 404 when there is no such server.
pub async fn layout_of(db: &mut SqliteConnection, server: ServerId) -> AppResult<ServerLayout> {
    let revision: i64 = sqlx::query_scalar("SELECT layout_revision FROM servers WHERE id = ?")
        .bind(server.to_string())
        .fetch_optional(&mut *db)
        .await?
        .ok_or(AppError::NotFound)?;
    let rows: Vec<CategoryRow> = sqlx::query_as(&format!(
        "SELECT {CATEGORY_COLS} FROM categories WHERE server_id = ? ORDER BY position, id"
    ))
    .bind(server.to_string())
    .fetch_all(&mut *db)
    .await?;
    let categories = rows
        .into_iter()
        .map(category_from_row)
        .collect::<AppResult<_>>()?;
    let placed: Vec<(String, Option<String>, i64)> = sqlx::query_as(
        "SELECT id, category_id, position FROM channels
         WHERE server_id = ? AND deleted_at IS NULL ORDER BY category_id, position, id",
    )
    .bind(server.to_string())
    .fetch_all(&mut *db)
    .await?;
    let channels = placed
        .into_iter()
        .map(|(id, category, position)| {
            Ok(ChannelPlacement {
                channel_id: id.parse().map_err(anyhow::Error::from)?,
                category_id: category
                    .map(|c| c.parse::<CategoryId>())
                    .transpose()
                    .map_err(anyhow::Error::from)?,
                position,
            })
        })
        .collect::<AppResult<_>>()?;
    Ok(ServerLayout {
        server_id: server,
        revision,
        categories,
        channels,
    })
}

/// Every server's layout for Ready: categories and revisions only (the channels in Ready carry
/// their own placement, so `channels` is empty). Two queries however many servers there are.
pub async fn all_layouts(db: &SqlitePool) -> AppResult<Vec<ServerLayout>> {
    let servers: Vec<(String, i64)> = sqlx::query_as(
        "SELECT id, layout_revision FROM servers WHERE deleted_at IS NULL ORDER BY id",
    )
    .fetch_all(db)
    .await?;
    let rows: Vec<CategoryRow> = sqlx::query_as(&format!(
        "SELECT {CATEGORY_COLS} FROM categories ORDER BY server_id, position, id"
    ))
    .fetch_all(db)
    .await?;
    let mut by_server: HashMap<ServerId, Vec<Category>> = HashMap::new();
    for row in rows {
        let c = category_from_row(row)?;
        by_server.entry(c.server_id).or_default().push(c);
    }
    servers
        .into_iter()
        .map(|(id, revision)| {
            let server_id: ServerId = id.parse().map_err(anyhow::Error::from)?;
            Ok(ServerLayout {
                server_id,
                revision,
                categories: by_server.remove(&server_id).unwrap_or_default(),
                channels: vec![],
            })
        })
        .collect()
}

/// Raise `server`'s layout revision by one, in the transaction that changes its layout, and
/// return the new one. Saves based on anything older are refused from then on.
pub async fn bump(tx: &mut SqliteConnection, server: ServerId) -> AppResult<i64> {
    Ok(sqlx::query_scalar(
        "UPDATE servers SET layout_revision = layout_revision + 1 WHERE id = ?
         RETURNING layout_revision",
    )
    .bind(server.to_string())
    .fetch_one(&mut *tx)
    .await?)
}

/// The position after the last live channel in `category` of `server` (`None`: the uncategorised
/// area), 0 when there is none. Deleted channels don't count: they come back at the bottom.
pub async fn bottom_of(
    tx: &mut SqliteConnection,
    server: ServerId,
    category: Option<CategoryId>,
) -> AppResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM channels
         WHERE server_id = ? AND deleted_at IS NULL AND category_id IS ?",
    )
    .bind(server.to_string())
    .bind(category.map(|c| c.to_string()))
    .fetch_one(&mut *tx)
    .await?)
}

/// Tell everyone `server`'s layout as it is now. Called after the change has committed (and with
/// `in_order` still held), so a failed read is only logged: the change has landed, and the next
/// one (or a reconnect's Ready) brings clients up to date.
pub async fn publish_layout(s: &AppState, server: ServerId) {
    let layout = match s.db.acquire().await {
        Ok(mut c) => layout_of(&mut c, server).await,
        Err(e) => Err(e.into()),
    };
    match layout {
        Ok(layout) => s.hub.publish(&s.db, Event::LayoutChanged { layout }).await,
        Err(e) => tracing::error!(error = ?e, %server, "couldn't read a layout to publish"),
    }
}

/// 429 when `me` has made too many layout changes just now.
fn spend(s: &AppState, me: UserId) -> AppResult<()> {
    if s.limits.layout_user.hit(&me.to_string()) {
        Ok(())
    } else {
        Err(AppError::TooManyRequests(None))
    }
}

/// The server category `id` is in: 404 when there is no such category.
async fn server_of(db: &SqlitePool, id: CategoryId) -> AppResult<ServerId> {
    let server: String = sqlx::query_scalar("SELECT server_id FROM categories WHERE id = ?")
        .bind(id.to_string())
        .fetch_optional(db)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(server.parse().map_err(anyhow::Error::from)?)
}

/// `POST /servers/{id}/categories`: a new, empty category at the bottom of the list. 400 once the
/// server has [`caps::CATEGORIES_PER_SERVER`].
async fn create(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(server): Path<ServerId>,
    Json(req): Json<CreateCategoryRequest>,
) -> AppResult<Json<Category>> {
    spend(&s, me)?;
    require_member(&s.db, server, me).await?;
    let name = clean_name(&req.name)?;
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    crate::servers::routes::server_live_in(&mut tx, server).await?; // deleted since the check (#287)
    let (count, next): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(MAX(position) + 1, 0) FROM categories WHERE server_id = ?",
    )
    .bind(server.to_string())
    .fetch_one(&mut *tx)
    .await?;
    if count >= caps::CATEGORIES_PER_SERVER {
        return Err(AppError::BadRequest(format!(
            "a server can have up to {} categories",
            caps::CATEGORIES_PER_SERVER
        )));
    }
    let category = insert_category(&mut tx, server, &name, next, None).await?;
    bump(&mut tx, server).await?;
    tx.commit().await?;
    publish_layout(&s, server).await;
    Ok(Json(category))
}

/// `PATCH /categories/{id}`: a new name, stored as typed (the list upper-cases it). A default
/// category keeps its `default_kind`, so a collapsed section stays collapsed after a rename.
async fn rename(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<CategoryId>,
    Json(req): Json<RenameCategoryRequest>,
) -> AppResult<Json<Category>> {
    spend(&s, me)?;
    let server = server_of(&s.db, id).await?;
    require_member(&s.db, server, me).await?;
    let name = clean_name(&req.name)?;
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let row: Option<CategoryRow> = sqlx::query_as(&format!(
        "UPDATE categories SET name = ? WHERE id = ? RETURNING {CATEGORY_COLS}"
    ))
    .bind(&name)
    .bind(id.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    // Deleted in the meantime.
    let category = category_from_row(row.ok_or(AppError::NotFound)?)?;
    bump(&mut tx, server).await?;
    tx.commit().await?;
    publish_layout(&s, server).await;
    Ok(Json(category))
}

/// `DELETE /categories/{id}`: the category goes, never its channels. Its live channels move, in
/// their order, to the bottom of the uncategorised area; its deleted ones lose it (`ON DELETE SET
/// NULL`), so a restore puts them there too.
async fn delete(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<CategoryId>,
) -> AppResult<StatusCode> {
    spend(&s, me)?;
    let server = server_of(&s.db, id).await?;
    require_member(&s.db, server, me).await?;
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let held: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM channels WHERE category_id = ? AND deleted_at IS NULL
         ORDER BY position, id",
    )
    .bind(id.to_string())
    .fetch_all(&mut *tx)
    .await?;
    let start = bottom_of(&mut tx, server, None).await?;
    for (i, ch) in held.iter().enumerate() {
        sqlx::query("UPDATE channels SET category_id = NULL, position = ? WHERE id = ?")
            .bind(start + i as i64)
            .bind(ch)
            .execute(&mut *tx)
            .await?;
    }
    let gone = sqlx::query("DELETE FROM categories WHERE id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if gone == 0 {
        return Err(AppError::NotFound); // deleted in the meantime; dropping `tx` rolls back
    }
    bump(&mut tx, server).await?;
    tx.commit().await?;
    publish_layout(&s, server).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Whether `named` is exactly `have`, each once.
fn exactly<'a>(named: impl IntoIterator<Item = &'a String>, have: &HashSet<String>) -> bool {
    let mut seen = HashSet::new();
    for id in named {
        if !have.contains(id) || !seen.insert(id) {
            return false;
        }
    }
    seen.len() == have.len()
}

/// `PUT /servers/{id}/layout`: save a whole new order (a drag or a Move up / Move down). 409 with
/// [`STALE`] unless it is based on the current revision and names exactly the server's categories
/// and exactly its live channels, each once: anything else would be a guess at what the person
/// meant on top of someone else's change, so the client snaps back to the order everyone has and
/// they try again. Nothing is half-applied: it is all one transaction.
async fn save_layout(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(server): Path<ServerId>,
    Json(req): Json<SaveLayoutRequest>,
) -> AppResult<Json<ServerLayout>> {
    spend(&s, me)?;
    require_member(&s.db, server, me).await?;
    let _order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let revision: i64 = sqlx::query_scalar("SELECT layout_revision FROM servers WHERE id = ?")
        .bind(server.to_string())
        .fetch_one(&mut *tx)
        .await?;
    let categories: HashSet<String> =
        sqlx::query_scalar("SELECT id FROM categories WHERE server_id = ?")
            .bind(server.to_string())
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .collect();
    let channels: HashSet<String> =
        sqlx::query_scalar("SELECT id FROM channels WHERE server_id = ? AND deleted_at IS NULL")
            .bind(server.to_string())
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .collect();
    let named_categories: Vec<String> = req.categories.iter().map(|c| c.id.to_string()).collect();
    let named_channels: Vec<String> = req
        .uncategorised
        .iter()
        .chain(req.categories.iter().flat_map(|c| &c.channels))
        .map(|c| c.to_string())
        .collect();
    if req.revision != revision
        || !exactly(&named_categories, &categories)
        || !exactly(&named_channels, &channels)
    {
        return Err(AppError::Conflict(STALE.into()));
    }
    let lists = std::iter::once((None, &req.uncategorised))
        .chain(req.categories.iter().map(|c| (Some(c.id), &c.channels)));
    for (category, list) in lists {
        for (i, ch) in list.iter().enumerate() {
            sqlx::query("UPDATE channels SET category_id = ?, position = ? WHERE id = ?")
                .bind(category.map(|c| c.to_string()))
                .bind(i as i64)
                .bind(ch.to_string())
                .execute(&mut *tx)
                .await?;
        }
    }
    for (i, c) in req.categories.iter().enumerate() {
        sqlx::query("UPDATE categories SET position = ? WHERE id = ?")
            .bind(i as i64)
            .bind(c.id.to_string())
            .execute(&mut *tx)
            .await?;
    }
    bump(&mut tx, server).await?;
    let layout = layout_of(&mut tx, server).await?;
    tx.commit().await?;
    s.hub
        .publish(
            &s.db,
            Event::LayoutChanged {
                layout: layout.clone(),
            },
        )
        .await;
    Ok(Json(layout))
}
