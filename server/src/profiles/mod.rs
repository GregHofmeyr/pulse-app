//! Profiles: avatars, banners, name styles and profile cards.

pub mod colors;
pub mod routes;
pub mod users;

use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{
    Badge, CardProfile, NameArchiveEntry, Profile, ProfileStats, UpdateProfileRequest,
};
use sqlx::SqlitePool;

use crate::db::now;
use crate::error::{AppError, AppResult};
use users::{USER_COLS, USER_JOIN};

#[derive(sqlx::FromRow)]
struct ProfileRow {
    #[sqlx(flatten)]
    user: users::UserRow,
    banner_hash: Option<String>,
    banner_still: Option<String>,
    theme_top: String,
    theme_bottom: String,
    pronouns: Option<String>,
    bio: Option<String>,
    created_at: String,
}

/// A user's full profile, or `None` if there is no such user. Stats are totals across all
/// servers (plan reconciliation 16).
pub async fn load_profile(db: &SqlitePool, user: UserId) -> AppResult<Option<Profile>> {
    let id = user.to_string();
    // One read transaction, so the card, stats, badges and archive are a single snapshot.
    let mut tx = db.begin().await?;
    let Some(row) = sqlx::query_as::<_, ProfileRow>(&format!(
        "SELECT {USER_COLS}, u.banner_hash, bn.still_hash AS banner_still, u.theme_top, \
         u.theme_bottom, u.pronouns, u.bio, u.created_at \
         FROM users u {USER_JOIN} LEFT JOIN media bn ON bn.hash = u.banner_hash WHERE u.id = ?"
    ))
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(None);
    };

    let voice: f64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM((julianday(COALESCE(left_at, ?)) - julianday(joined_at)) * 86400.0), 0.0) \
         FROM voice_sessions WHERE user_id = ?",
    )
    .bind(now())
    .bind(&id)
    .fetch_one(&mut *tx)
    .await?;

    let top_reaction: Option<String> = sqlx::query_scalar(
        "SELECT r.emoji FROM reactions r JOIN messages m ON m.id = r.message_id \
         WHERE r.user_id = ? AND m.deleted_at IS NULL \
         GROUP BY r.emoji ORDER BY COUNT(*) DESC, MAX(r.created_at) DESC LIMIT 1",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;

    let archive: Vec<(String, Option<String>, String, String)> = sqlx::query_as(
        "SELECT server_id, new_nick, changed_by, changed_at FROM nickname_history \
         WHERE target_user_id = ? ORDER BY changed_at DESC, id DESC",
    )
    .bind(&id)
    .fetch_all(&mut *tx)
    .await?;
    let times_renamed = archive.len() as u32;
    let name_archive = archive
        .into_iter()
        // A cleared nickname has no name to archive.
        .filter_map(|(server, name, by, at)| Some((server, name?, by, at)))
        .map(|(server, name, by, at)| {
            Ok(NameArchiveEntry {
                server_id: server.parse::<ServerId>().map_err(anyhow::Error::from)?,
                name,
                changed_by: by.parse::<UserId>().map_err(anyhow::Error::from)?,
                changed_at: at,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;

    let badges = sqlx::query_as::<_, (String, String, bool)>(
        "SELECT badge_key, earned_at, first_earner FROM user_badges WHERE user_id = ? \
         ORDER BY earned_at, badge_key",
    )
    .bind(&id)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|(key, earned_at, first_earner)| Badge {
        key,
        earned_at,
        first_earner,
    })
    .collect();
    tx.commit().await?;

    Ok(Some(Profile {
        user: row.user.into_user()?,
        card: CardProfile {
            banner_hash: row.banner_hash,
            banner_still: row.banner_still,
            theme_top: row.theme_top,
            theme_bottom: row.theme_bottom,
            pronouns: row.pronouns,
            bio: row.bio,
        },
        stats: ProfileStats {
            voice_seconds: voice.max(0.0).round() as u64,
            member_since: row.created_at,
            top_reaction,
            times_renamed,
        },
        badges,
        name_archive,
    }))
}

/// The bundled name fonts (plan reconciliation 9).
pub const NAME_FONTS: [&str; 8] = [
    "cormorant-garamond",
    "fredoka",
    "press-start-2p",
    "pacifico",
    "caveat",
    "space-grotesk",
    "bricolage-grotesque",
    "orbitron",
];

const PRONOUNS_MAX: usize = 40;
const BIO_MAX: usize = 190;
const BIO_LINE_BREAKS_MAX: usize = 4;

/// A profile edit that passed every text and colour rule, ready to store. The two picture
/// hashes still need their database check.
#[derive(Debug, PartialEq, Eq)]
pub struct Validated {
    pub avatar_hash: Option<String>,
    pub banner_hash: Option<String>,
    pub theme_top: String,
    pub theme_bottom: String,
    pub font: Option<String>,
    /// Lightened, lowercase `#rrggbb`, at most two.
    pub colors: Vec<String>,
    pub pronouns: Option<String>,
    pub bio: Option<String>,
}

fn bad(msg: &str) -> AppError {
    AppError::BadRequest(msg.into())
}

fn bad_colour() -> AppError {
    bad("Colours must look like #a1b2c3.")
}

/// Trimmed text, or `None` if nothing is left.
fn text(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

/// A bio with `\r\n` and lone `\r` as `\n`, its blank leading lines dropped (so they don't count
/// as line breaks; the first text line keeps its leading space) and its end trimmed, or `None` if
/// nothing is left.
fn bio_text(s: Option<String>) -> Option<String> {
    s.map(|s| {
        let s = s.replace("\r\n", "\n").replace('\r', "\n");
        let start = s
            .find(|c: char| !c.is_whitespace())
            .map_or(s.len(), |i| s[..i].rfind('\n').map_or(0, |n| n + 1));
        s[start..].trim_end().to_owned()
    })
    .filter(|s| !s.is_empty())
}

/// The text and colour rules (the server enforces every rule; the client only previews).
pub fn validate(req: UpdateProfileRequest) -> AppResult<Validated> {
    let font = match req.style.font {
        Some(f) if NAME_FONTS.contains(&f.as_str()) => Some(f),
        Some(_) => return Err(bad("Unknown name font.")),
        None => None,
    };
    if req.style.colors.len() > 2 {
        return Err(bad_colour());
    }
    let colors = req
        .style
        .colors
        .iter()
        .map(|c| {
            colors::parse_hex(c)
                .map(|rgb| colors::to_hex(colors::lighten_for_name(rgb)))
                .ok_or_else(bad_colour)
        })
        .collect::<AppResult<Vec<_>>>()?;
    // Theme colours are taken as chosen (spec: a custom pair is accepted as is).
    let theme = |c: &str| {
        colors::parse_hex(c)
            .map(colors::to_hex)
            .ok_or_else(bad_colour)
    };

    let pronouns = text(req.pronouns);
    if let Some(p) = &pronouns {
        if p.chars().count() > PRONOUNS_MAX {
            return Err(bad("Pronouns can be up to 40 characters."));
        }
        if p.chars().any(char::is_control) {
            return Err(bad("Pronouns can't contain control characters."));
        }
    }
    let bio = bio_text(req.bio);
    if let Some(b) = &bio {
        if b.chars().count() > BIO_MAX {
            return Err(bad("Your bio can be up to 190 characters."));
        }
        if b.matches('\n').count() > BIO_LINE_BREAKS_MAX {
            return Err(bad("Your bio can have at most 4 line breaks."));
        }
        if b.chars().any(|c| c.is_control() && c != '\n') {
            return Err(bad("Your bio can't contain control characters."));
        }
    }
    Ok(Validated {
        avatar_hash: req.avatar_hash,
        banner_hash: req.banner_hash,
        theme_top: theme(&req.theme_top)?,
        theme_bottom: theme(&req.theme_bottom)?,
        font,
        colors,
        pronouns,
        bio,
    })
}

#[cfg(test)]
mod name_font_tests {
    use super::NAME_FONTS;

    /// A literal copy of `NAME_FONTS` in client/ui/src/lib/nameFonts.test.ts: change one side and
    /// both tests fail.
    #[test]
    fn keys_match_the_ui_list() {
        assert_eq!(
            NAME_FONTS,
            [
                "cormorant-garamond",
                "fredoka",
                "press-start-2p",
                "pacifico",
                "caveat",
                "space-grotesk",
                "bricolage-grotesque",
                "orbitron",
            ]
        );
    }
}
