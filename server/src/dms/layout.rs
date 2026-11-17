//! Your conversation list's layout (#305): conversations pinned to the top, and your own categories (create, rename,
//! fold, reorder (#419), delete, move a conversation into one). Kept per person (`dm_categories`, `dm_placements`) so it follows you
//! to every device, and sent only to you (Ready's `dm_layout` and `DmLayoutChanged`, always the whole layout). A
//! placement only counts while you're in the conversation: one you left reads as not there (its row goes with the
//! chat when that's purged, or comes back if you're added again).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{patch, post, put};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, DmCategoryId, UserId};
use pulse_protocol::rest::{
    CreateDmCategoryRequest, DmCategory, DmLayout, DmPlacement, DmPlacementRequest,
    UpdateDmCategoryRequest, dm_layout_limits,
};
use sqlx::SqlitePool;

use crate::AppState;
use crate::auth::AuthUser;
use crate::db::{begin_write, now};
use crate::error::{AppError, AppResult};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/dm-categories", post(create))
        .route("/me/dm-categories/{id}", patch(update).delete(remove))
        .route("/me/dm-placements/{channel_id}", put(place))
}

/// `user`'s layout: their categories in order, and their placements in conversations they're still in.
pub async fn of(db: &SqlitePool, user: UserId) -> AppResult<DmLayout> {
    let cats: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT id, name, collapsed FROM dm_categories WHERE user_id = ? ORDER BY position, id",
    )
    .bind(user.to_string())
    .fetch_all(db)
    .await?;
    let places: Vec<(String, i64, Option<String>)> = sqlx::query_as(
        "SELECT p.channel_id, p.pinned, p.category_id FROM dm_placements p
         JOIN channel_members cm ON cm.channel_id = p.channel_id AND cm.user_id = p.user_id
         JOIN channels c ON c.id = p.channel_id AND c.server_id IS NULL AND c.deleted_at IS NULL
         WHERE p.user_id = ? ORDER BY p.channel_id",
    )
    .bind(user.to_string())
    .fetch_all(db)
    .await?;
    Ok(DmLayout {
        categories: cats
            .into_iter()
            .map(|(id, name, collapsed)| {
                Ok(DmCategory {
                    id: id.parse().map_err(anyhow::Error::from)?,
                    name,
                    collapsed: collapsed != 0,
                })
            })
            .collect::<AppResult<_>>()?,
        placements: places
            .into_iter()
            .map(|(channel, pinned, category)| {
                Ok(DmPlacement {
                    channel_id: channel.parse().map_err(anyhow::Error::from)?,
                    pinned: pinned != 0,
                    category_id: category
                        .map(|c| c.parse())
                        .transpose()
                        .map_err(anyhow::Error::from)?,
                })
            })
            .collect::<AppResult<_>>()?,
    })
}

/// A category's name, trimmed: 1-32 characters on one line, and not one of the Move flyout's own entries (#419).
pub fn clean_name(raw: &str) -> AppResult<String> {
    let n = raw.trim();
    let len = n.chars().count();
    if len == 0 || len > dm_layout_limits::NAME_CHARS || n.chars().any(char::is_control) {
        return Err(AppError::BadRequest(format!(
            "a category's name is 1-{} characters on one line",
            dm_layout_limits::NAME_CHARS
        )));
    }
    if dm_layout_limits::is_reserved(n) {
        return Err(AppError::BadRequest(format!(
            "\"{n}\" is a menu entry: pick another name"
        )));
    }
    Ok(n.to_string())
}

fn too_fast() -> AppError {
    AppError::TooManyRequests(Some(
        "You're moving things too fast. Try again in a minute.".into(),
    ))
}

/// Tell `me`'s own sessions their layout as it is now.
async fn announce(s: &AppState, me: UserId) -> AppResult<DmLayout> {
    let layout = of(&s.db, me).await?;
    s.hub
        .publish(
            &s.db,
            Event::DmLayoutChanged {
                user_id: me,
                layout: layout.clone(),
            },
        )
        .await;
    Ok(layout)
}

async fn owned(tx: &mut sqlx::SqliteConnection, me: UserId, id: DmCategoryId) -> AppResult<()> {
    let hit: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM dm_categories WHERE id = ? AND user_id = ?")
            .bind(id.to_string())
            .bind(me.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    hit.map(|_| ()).ok_or(AppError::NotFound)
}

/// Refuse a name another of `me`'s categories has (ignoring case): two headings you can't tell apart.
async fn name_free(
    tx: &mut sqlx::SqliteConnection,
    me: UserId,
    name: &str,
    except: Option<DmCategoryId>,
) -> AppResult<()> {
    let names: Vec<(String, String)> =
        sqlx::query_as("SELECT id, name FROM dm_categories WHERE user_id = ?")
            .bind(me.to_string())
            .fetch_all(&mut *tx)
            .await?;
    let except = except.map(|e| e.to_string());
    if names
        .iter()
        .any(|(id, n)| Some(id) != except.as_ref() && n.to_lowercase() == name.to_lowercase())
    {
        return Err(AppError::Conflict(
            "you already have a category with that name".into(),
        ));
    }
    Ok(())
}

/// `POST /me/dm-categories`: a new category, last. Answers with the whole layout (the new one is the last category).
async fn create(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(req): Json<CreateDmCategoryRequest>,
) -> AppResult<(StatusCode, Json<DmLayout>)> {
    let name = clean_name(&req.name)?;
    if !s.limits.layout_user.hit(&me.to_string()) {
        return Err(too_fast());
    }
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    let (count, last): (i64, Option<i64>) =
        sqlx::query_as("SELECT COUNT(*), MAX(position) FROM dm_categories WHERE user_id = ?")
            .bind(me.to_string())
            .fetch_one(&mut *tx)
            .await?;
    if count as usize >= dm_layout_limits::MAX_CATEGORIES {
        return Err(AppError::BadRequest(format!(
            "you can have up to {} categories",
            dm_layout_limits::MAX_CATEGORIES
        )));
    }
    name_free(&mut tx, me, &name, None).await?;
    sqlx::query(
        "INSERT INTO dm_categories (id, user_id, name, position, collapsed, created_at) VALUES (?, ?, ?, ?, 0, ?)",
    )
    .bind(DmCategoryId::new().to_string())
    .bind(me.to_string())
    .bind(&name)
    .bind(last.map_or(0, |l| l + 1))
    .bind(now())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let layout = announce(&s, me).await?;
    Ok((StatusCode::CREATED, Json(layout)))
}

/// `PATCH /me/dm-categories/{id}`: rename it, fold it open or shut, or move it to `position` among your categories
/// (#419).
async fn update(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<DmCategoryId>,
    Json(req): Json<UpdateDmCategoryRequest>,
) -> AppResult<Json<DmLayout>> {
    let name = req.name.as_deref().map(clean_name).transpose()?;
    if !s.limits.layout_user.hit(&me.to_string()) {
        return Err(too_fast());
    }
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    owned(&mut tx, me, id).await?;
    if let Some(n) = &name {
        name_free(&mut tx, me, n, Some(id)).await?;
    }
    sqlx::query(
        "UPDATE dm_categories SET name = COALESCE(?, name), collapsed = COALESCE(?, collapsed) WHERE id = ?",
    )
    .bind(name)
    .bind(req.collapsed.map(i64::from))
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?;
    if let Some(to) = req.position {
        let mut order: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM dm_categories WHERE user_id = ? ORDER BY position, id",
        )
        .bind(me.to_string())
        .fetch_all(&mut *tx)
        .await?;
        let id = id.to_string();
        order.retain(|c| *c != id);
        order.insert((to as usize).min(order.len()), id);
        for (position, c) in order.iter().enumerate() {
            sqlx::query("UPDATE dm_categories SET position = ? WHERE id = ?")
                .bind(position as i64)
                .bind(c)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    Ok(Json(announce(&s, me).await?))
}

/// `DELETE /me/dm-categories/{id}`: its conversations go back to the main list (pinned ones stay pinned).
async fn remove(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<DmCategoryId>,
) -> AppResult<Json<DmLayout>> {
    if !s.limits.layout_user.hit(&me.to_string()) {
        return Err(too_fast());
    }
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    owned(&mut tx, me, id).await?;
    sqlx::query("DELETE FROM dm_placements WHERE user_id = ? AND category_id = ? AND pinned = 0")
        .bind(me.to_string())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE dm_placements SET category_id = NULL WHERE user_id = ? AND category_id = ?",
    )
    .bind(me.to_string())
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM dm_categories WHERE id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(announce(&s, me).await?))
}

/// `PUT /me/dm-placements/{channel_id}`: pin or unpin one of your conversations (a DM, group or your notes) and put
/// it in one of your categories, or none.
async fn place(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(channel): Path<ChannelId>,
    Json(req): Json<DmPlacementRequest>,
) -> AppResult<Json<DmLayout>> {
    let ch = crate::access::channel_for(&s.db, me, channel).await?;
    if ch.server_id.is_some() {
        return Err(AppError::BadRequest(
            "only conversations go in your conversation list".into(),
        ));
    }
    if !s.limits.layout_user.hit(&me.to_string()) {
        return Err(too_fast());
    }
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    if let Some(c) = req.category_id {
        owned(&mut tx, me, c).await?;
    }
    if !req.pinned && req.category_id.is_none() {
        sqlx::query("DELETE FROM dm_placements WHERE user_id = ? AND channel_id = ?")
            .bind(me.to_string())
            .bind(channel.to_string())
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query(
            "INSERT INTO dm_placements (user_id, channel_id, pinned, category_id) VALUES (?, ?, ?, ?)
             ON CONFLICT (user_id, channel_id) DO UPDATE SET pinned = excluded.pinned, category_id = excluded.category_id",
        )
        .bind(me.to_string())
        .bind(channel.to_string())
        .bind(i64::from(req.pinned))
        .bind(req.category_id.map(|c| c.to_string()))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(announce(&s, me).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_limited() {
        assert_eq!(clean_name("  Gaming ").unwrap(), "Gaming");
        assert!(clean_name(" ").is_err());
        assert!(clean_name(&"x".repeat(33)).is_err());
        assert!(clean_name(&"é".repeat(32)).is_ok());
        assert!(clean_name("a\nb").is_err());
        // the Move flyout's own entries (#419)
        for reserved in ["New category…", " new category... ", "REMOVE FROM CATEGORY"] {
            assert!(clean_name(reserved).is_err(), "{reserved}");
        }
        assert!(clean_name("New category").is_ok());
    }
}
