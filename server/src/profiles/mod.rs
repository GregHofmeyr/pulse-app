//! Profiles: avatars, banners, name styles and profile cards.

pub mod routes;
pub mod users;

use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{Badge, CardProfile, NameArchiveEntry, Profile, ProfileStats};
use sqlx::SqlitePool;

use crate::db::now;
use crate::error::AppResult;
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
    let Some(row) = sqlx::query_as::<_, ProfileRow>(&format!(
        "SELECT {USER_COLS}, u.banner_hash, bn.still_hash AS banner_still, u.theme_top, \
         u.theme_bottom, u.pronouns, u.bio, u.created_at \
         FROM users u {USER_JOIN} LEFT JOIN media bn ON bn.hash = u.banner_hash WHERE u.id = ?"
    ))
    .bind(&id)
    .fetch_optional(db)
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
    .fetch_one(db)
    .await?;

    let top_reaction: Option<String> = sqlx::query_scalar(
        "SELECT r.emoji FROM reactions r JOIN messages m ON m.id = r.message_id \
         WHERE r.user_id = ? AND m.deleted_at IS NULL \
         GROUP BY r.emoji ORDER BY COUNT(*) DESC, MAX(r.created_at) DESC LIMIT 1",
    )
    .bind(&id)
    .fetch_optional(db)
    .await?;

    let archive: Vec<(String, Option<String>, String, String)> = sqlx::query_as(
        "SELECT server_id, new_nick, changed_by, changed_at FROM nickname_history \
         WHERE target_user_id = ? ORDER BY changed_at DESC, id DESC",
    )
    .bind(&id)
    .fetch_all(db)
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
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|(key, earned_at, first_earner)| Badge {
        key,
        earned_at,
        first_earner,
    })
    .collect();

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
