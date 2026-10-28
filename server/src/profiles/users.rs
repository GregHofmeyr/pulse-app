//! The one place that turns a `users` row into a protocol [`User`].

use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{NameStyle, User};

use crate::error::AppResult;

/// What [`UserRow`] reads, for a query that selects from `users u` with [`USER_JOIN`].
pub const USER_COLS: &str =
    "u.id, u.username, u.avatar_hash, av.still_hash, u.name_font, u.name_color";
/// Brings in the avatar's still (`av.still_hash`).
pub const USER_JOIN: &str = "LEFT JOIN media av ON av.hash = u.avatar_hash";

#[derive(sqlx::FromRow)]
pub struct UserRow {
    pub id: String,
    pub username: String,
    pub avatar_hash: Option<String>,
    pub still_hash: Option<String>,
    pub name_font: Option<String>,
    pub name_color: Option<String>,
}

impl UserRow {
    pub fn into_user(self) -> AppResult<User> {
        Ok(User {
            id: self.id.parse::<UserId>().map_err(anyhow::Error::from)?,
            username: self.username,
            avatar_hash: self.avatar_hash,
            avatar_still: self.still_hash,
            style: NameStyle {
                font: self.name_font,
                colors: self
                    .name_color
                    .map(|c| c.split(',').map(str::to_owned).collect())
                    .unwrap_or_default(),
            },
        })
    }
}
