//! Channel categories (#54): named headings that hold a server's channels of any kind.

use pulse_protocol::ids::{CategoryId, ServerId};
use pulse_protocol::rest::{Category, ChannelKind};

use crate::db::now;
use crate::error::AppResult;

/// Adds a category to `server` at `position`. `default_kind` is the kind a default category is
/// made for (a new server's), `None` for one people make.
pub async fn insert_category(
    tx: &mut sqlx::SqliteConnection,
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
