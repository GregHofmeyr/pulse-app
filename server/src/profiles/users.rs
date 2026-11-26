//! The one place that turns a `users` row into a protocol [`User`].

use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{NameStyle, User};

use crate::error::AppResult;
use crate::profiles::colors;

/// What [`UserRow`] reads, for a query that selects from `users u` with [`USER_JOIN`].
pub const USER_COLS: &str = "u.id, u.username, u.display_name, u.avatar_hash, av.still_hash, u.name_font, u.name_color, \
     u.name_effect, u.typing_text";
/// Brings in the avatar's still (`av.still_hash`).
pub const USER_JOIN: &str = "LEFT JOIN media av ON av.hash = u.avatar_hash";

#[derive(sqlx::FromRow)]
pub struct UserRow {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub avatar_hash: Option<String>,
    pub still_hash: Option<String>,
    pub name_font: Option<String>,
    pub name_color: Option<String>,
    pub name_effect: Option<String>,
    pub typing_text: Option<String>,
}

impl UserRow {
    pub fn into_user(self) -> AppResult<User> {
        Ok(User {
            id: self.id.parse::<UserId>().map_err(anyhow::Error::from)?,
            username: self.username,
            display_name: self.display_name,
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
                // Only an effect this server knows (#468).
                effect: self
                    .name_effect
                    .filter(|e| super::NAME_EFFECTS.contains(&e.as_str())),
            },
            // Stored clean (#652); the name rules again in case a row predates them.
            typing_text: self
                .typing_text
                .and_then(|t| super::names::clean_typing_text(&t).ok().flatten()),
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
            display_name: "alex".into(),
            avatar_hash: None,
            still_hash: None,
            name_font: None,
            name_color: name_color.map(str::to_owned),
            name_effect: None,
            typing_text: None,
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

    #[test]
    fn only_known_name_effects_are_kept() {
        let effect = |e: Option<&str>| {
            let mut r = row(None);
            r.name_effect = e.map(str::to_owned);
            r.into_user().unwrap().style.effect
        };
        assert_eq!(effect(None), None);
        assert_eq!(effect(Some("ember")).as_deref(), Some("ember"));
        assert_eq!(effect(Some("sparkle")), None);
    }

    #[test]
    fn typing_text_is_read_back_and_a_bad_row_is_the_default() {
        let text = |t: Option<&str>| {
            let mut r = row(None);
            r.typing_text = t.map(str::to_owned);
            r.into_user().unwrap().typing_text
        };
        assert_eq!(text(None), None);
        assert_eq!(text(Some("cooking")).as_deref(), Some("cooking"));
        assert_eq!(text(Some("")), None);
        assert_eq!(text(Some("go to pulse.app")), None);
    }
}
