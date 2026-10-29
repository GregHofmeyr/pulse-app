//! The one place that turns a `users` row into a protocol [`User`].

use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{NameStyle, User};

use crate::error::AppResult;
use crate::profiles::colors;

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
                // Only well-formed colours: an empty column is no colour, not [""].
                colors: self
                    .name_color
                    .map(|c| {
                        c.split(',')
                            .filter(|p| colors::parse_hex(p).is_some())
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name_color: Option<&str>) -> UserRow {
        UserRow {
            id: "00000000000000000000000001".into(),
            username: "alex".into(),
            avatar_hash: None,
            still_hash: None,
            name_font: None,
            name_color: name_color.map(str::to_owned),
        }
    }

    #[test]
    fn only_well_formed_name_colours_are_kept() {
        let colors = |c| row(c).into_user().unwrap().style.colors;
        assert!(colors(None).is_empty());
        assert!(colors(Some("")).is_empty());
        assert_eq!(colors(Some("#ffc4a3")), ["#ffc4a3"]);
        assert_eq!(colors(Some("#ffc4a3,#ff7fae")), ["#ffc4a3", "#ff7fae"]);
        assert_eq!(colors(Some("#ffc4a3,,nope")), ["#ffc4a3"]);
    }
}
