//! Profiles: avatars, banners, name styles and profile cards.

pub mod colors;
pub mod names;
pub mod private;
pub mod routes;
pub mod users;

use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{
    CardProfile, NameArchiveEntry, Profile, ProfileStats, UpdateProfileRequest,
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
    card_glow: bool,
    created_at: String,
    card_stats: Option<String>,
}

/// The most rename rows a profile carries.
const PROFILE_ARCHIVE_MAX: i64 = 50;

/// A user's full profile as `viewer` sees it (see [`crate::badges::view::profile_badges`]), or
/// `None` if there is no such user. Stats are totals across all servers (plan reconciliation 16).
pub async fn load_profile(
    db: &SqlitePool,
    user: UserId,
    viewer: UserId,
) -> AppResult<Option<Profile>> {
    let id = user.to_string();
    // One read transaction, so the card, stats, badges and archive are a single snapshot.
    let mut tx = db.begin().await?;
    let Some(row) = sqlx::query_as::<_, ProfileRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {USER_COLS}, u.banner_hash, bn.still_hash AS banner_still, u.theme_top, \
         u.theme_bottom, u.pronouns, u.bio, u.card_glow, u.created_at, u.card_stats \
         FROM users u {USER_JOIN} LEFT JOIN media bn ON bn.hash = u.banner_hash WHERE u.id = ?"
    )))
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

    // Every rename counts, resets included; the list here is only the newest (a server's full
    // archive is paged by `GET /servers/{id}/members/{user}/names`).
    let times_renamed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM nickname_history WHERE target_user_id = ?")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await?;
    let times_renamed = times_renamed as u32;
    // Only names given in servers `viewer` is in (#80): a server's nicknames are its own.
    let archive: Vec<(String, Option<String>, String, String)> = sqlx::query_as(
        "SELECT server_id, new_nick, changed_by, changed_at FROM nickname_history \
         WHERE target_user_id = ? AND server_id IN \
           (SELECT m.server_id FROM server_members m JOIN servers s ON s.id = m.server_id \
            WHERE m.user_id = ? AND s.deleted_at IS NULL) \
         ORDER BY changed_at DESC, id DESC LIMIT ?",
    )
    .bind(&id)
    .bind(viewer.to_string())
    .bind(PROFILE_ARCHIVE_MAX)
    .fetch_all(&mut *tx)
    .await?;
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

    let (badges, showcase, showcase_chosen) =
        crate::badges::view::profile_badges(&mut tx, user, viewer).await?;
    let (steam_completion, steam_perfect_games) =
        crate::steam::store::profile_stat(&mut tx, user).await?;
    let steam_recent_minutes = crate::steam::store::profile_hours(&mut tx, user).await?;
    tx.commit().await?;

    // What the card shows (#727). Someone else's view never carries a hidden stat's value; your own does.
    let chosen = stored_card_stats(row.card_stats.as_deref());
    let visible =
        |key: &str| viewer == user || chosen.as_ref().is_none_or(|c| c.iter().any(|k| k == key));
    let shown = chosen.clone();
    let hidden_from_viewer = crate::blocks::has_blocked(db, user, viewer).await?;
    let profile = Profile {
        user: row.user.into_user()?,
        card: CardProfile {
            banner_hash: row.banner_hash,
            banner_still: row.banner_still,
            theme_top: row.theme_top,
            theme_bottom: row.theme_bottom,
            pronouns: row.pronouns,
            bio: row.bio,
            glow: row.card_glow,
        },
        stats: ProfileStats {
            // required for older clients, so a hidden one is sent neutral rather than omitted
            voice_seconds: if visible("voice") {
                voice.max(0.0).round() as u64
            } else {
                0
            },
            member_since: if visible("member_since") {
                row.created_at
            } else {
                String::new()
            },
            top_reaction,
            times_renamed,
            steam_completion: steam_completion.filter(|_| visible("steam_completion")),
            steam_perfect_games: steam_perfect_games.filter(|_| visible("steam_completion")),
            steam_recent_minutes: steam_recent_minutes.filter(|_| visible("steam_recent")),
            shown,
        },
        badges,
        showcase,
        showcase_chosen,
        name_archive,
    };
    // someone they've blocked sees it emptied (#474)
    Ok(Some(if hidden_from_viewer {
        crate::blocks::emptied_profile(profile)
    } else {
        profile
    }))
}

/// The card stats (#727), stored by key in `users.card_stats` as a JSON array of the shown ones, in order (NULL:
/// all four, in this order). A literal copy is in the client test.
pub const CARD_STATS: [&str; 4] = ["member_since", "voice", "steam_completion", "steam_recent"];

/// The saved card stats, if any: keys this server knows, as stored (a newer server's key is dropped).
fn stored_card_stats(raw: Option<&str>) -> Option<Vec<String>> {
    let keys: Vec<String> = serde_json::from_str(raw?).ok()?;
    Some(
        keys.into_iter()
            .filter(|k| CARD_STATS.contains(&k.as_str()))
            .collect(),
    )
}

/// The bundled name fonts (plan reconciliation 9; the last six from #142).
pub const NAME_FONTS: [&str; 14] = [
    "cormorant-garamond",
    "fredoka",
    "press-start-2p",
    "pacifico",
    "caveat",
    "space-grotesk",
    "bricolage-grotesque",
    "orbitron",
    "bungee",
    "silkscreen",
    "audiowide",
    "righteous",
    "kaushan-script",
    "black-ops-one",
];

/// The name effects (#468), stored by key in `users.name_effect`. A literal copy is in
/// client/ui/src/lib/nameEffects.test.ts.
pub const NAME_EFFECTS: [&str; 8] = [
    "neon", "toon", "pop", "gummy", "prism", "shimmer", "glitch", "ember",
];

/// The speaking ring styles (#651), stored by key in `users.ring_style` (NULL: `"classic"`). A
/// literal copy is in client/ui/src/lib/speakingRing.test.ts.
pub const RING_STYLES: [&str; 11] = [
    "classic",
    "glow",
    "pulse",
    "ripple",
    "sweep",
    "orbit",
    "halo",
    "viewfinder",
    "liquid",
    "dots",
    "bars",
];

/// The speaking ring colours (#651), stored by key in `users.ring_color` (NULL: `"green"`). A
/// literal copy is in client/ui/src/lib/speakingRing.test.ts.
pub const RING_COLORS: [&str; 9] = [
    "green", "blue", "gold", "pink", "cyan", "orange", "white", "purple", "profile",
];

/// The default ring style and colour, stored as NULL (#651).
const RING_DEFAULTS: [&str; 2] = ["classic", "green"];

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
    /// One of [`NAME_EFFECTS`], or none (#468).
    pub effect: Option<String>,
    pub pronouns: Option<String>,
    pub bio: Option<String>,
    /// `None`: keep the current display name.
    pub display_name: Option<String>,
    /// `None`: keep the current card glow (#156).
    pub glow: Option<bool>,
    /// `None`: keep the current typing text; `Some(None)`: back to "is typing" (#652).
    pub typing_text: Option<Option<String>>,
    /// `None`: keep the current ring style; `Some(None)`: back to Classic (#651).
    pub ring_style: Option<Option<String>>,
    /// `None`: keep the current ring colour; `Some(None)`: back to green (#651).
    pub ring_color: Option<Option<String>>,
    /// `None`: keep the current card stats; `Some(None)`: the default (all four) (#727). Else the JSON array.
    pub card_stats: Option<Option<String>>,
}

/// A card stat list from a save (#727): absent keeps the current one, the default list stores NULL, an unknown or
/// repeated key is refused.
fn card_stats(keys: Option<Vec<String>>) -> AppResult<Option<Option<String>>> {
    let Some(keys) = keys else { return Ok(None) };
    for (i, k) in keys.iter().enumerate() {
        if !CARD_STATS.contains(&k.as_str()) {
            return Err(bad("Unknown card stat."));
        }
        if keys[..i].contains(k) {
            return Err(bad("A card stat can be listed once."));
        }
    }
    if keys == CARD_STATS {
        return Ok(Some(None));
    }
    let json = serde_json::to_string(&keys).map_err(anyhow::Error::from)?;
    Ok(Some(Some(json)))
}

/// A ring key from a save (#651): absent keeps the current one, a default key stores NULL and a
/// key not in `keys` is refused with `msg`.
fn ring_key(key: Option<String>, keys: &[&str], msg: &str) -> AppResult<Option<Option<String>>> {
    match key {
        None => Ok(None),
        Some(k) if !keys.contains(&k.as_str()) => Err(bad(msg)),
        Some(k) if RING_DEFAULTS.contains(&k.as_str()) => Ok(Some(None)),
        Some(k) => Ok(Some(Some(k))),
    }
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
    let effect = match req.style.effect {
        Some(e) if NAME_EFFECTS.contains(&e.as_str()) => Some(e),
        Some(_) => return Err(bad("Unknown name effect.")),
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

    let display_name = req.display_name.as_deref().map(names::clean).transpose()?;
    let typing_text = req
        .typing_text
        .as_deref()
        .map(names::clean_typing_text)
        .transpose()?;
    let ring_style = ring_key(req.ring_style, &RING_STYLES, "Unknown speaking ring.")?;
    let ring_color = ring_key(req.ring_color, &RING_COLORS, "Unknown ring colour.")?;
    let card_stats = card_stats(req.card_stats)?;
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
        effect,
        pronouns,
        bio,
        display_name,
        glow: req.glow,
        typing_text,
        ring_style,
        ring_color,
        card_stats,
    })
}

#[cfg(test)]
mod name_effect_tests {
    use super::NAME_EFFECTS;

    /// A literal copy of `NAME_EFFECTS` in client/ui/src/lib/nameEffects.test.ts: change one side
    /// and both tests fail.
    #[test]
    fn keys_match_the_ui_list() {
        assert_eq!(
            NAME_EFFECTS,
            [
                "neon", "toon", "pop", "gummy", "prism", "shimmer", "glitch", "ember"
            ]
        );
    }
}

#[cfg(test)]
mod ring_tests {
    use super::{RING_COLORS, RING_STYLES};

    /// A literal copy in client/ui/src/lib/speakingRing.test.ts: change one side and both tests
    /// fail.
    #[test]
    fn ring_keys_match_the_ui_list() {
        assert_eq!(
            RING_STYLES,
            [
                "classic",
                "glow",
                "pulse",
                "ripple",
                "sweep",
                "orbit",
                "halo",
                "viewfinder",
                "liquid",
                "dots",
                "bars"
            ]
        );
        assert_eq!(
            RING_COLORS,
            [
                "green", "blue", "gold", "pink", "cyan", "orange", "white", "purple", "profile"
            ]
        );
    }
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
                "bungee",
                "silkscreen",
                "audiowide",
                "righteous",
                "kaushan-script",
                "black-ops-one",
            ]
        );
    }
}
