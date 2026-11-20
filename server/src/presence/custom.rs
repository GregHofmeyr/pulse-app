//! Custom status (#152): a short line of plain text and an optional emoji beside your name, clearing after a while or
//! never. It rides on presence ([`super::view_for`]): only those you look online to see it, so Invisible, appearing
//! offline to someone and being offline all hide it. Your own sessions get it with its end and your recent ones.

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{CUSTOM_STATUS_MAX, CustomStatus, MyCustomStatus};
use sqlx::{SqliteConnection, SqlitePool};

use crate::AppState;
use crate::db::now;
use crate::error::{AppError, AppResult};

/// How many recent custom statuses are kept for picking again.
pub const RECENT: i64 = 5;

/// The longest custom status timer the server takes: "this week" is at most 7 days off, with a day to spare for
/// time zones.
pub const MAX_CUSTOM: jiff::SignedDuration = crate::times::days(8);

/// Characters that can reorder or disguise text around them: the bidi embeddings, overrides and isolates, and the
/// directional marks.
fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}'
    )
}

/// The text as kept and shown: control and bidi characters gone, any run of whitespace (line breaks and tabs
/// included) one space, trimmed. Too long after that is refused rather than cut.
pub fn clean_text(raw: &str) -> AppResult<String> {
    let kept: String = raw
        .chars()
        .filter_map(|c| {
            if c.is_whitespace() {
                Some(' ')
            } else if c.is_control() || is_bidi_control(c) {
                None
            } else {
                Some(c)
            }
        })
        .collect();
    let text = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > CUSTOM_STATUS_MAX {
        return Err(AppError::BadRequest(format!(
            "A custom status is up to {CUSTOM_STATUS_MAX} characters."
        )));
    }
    Ok(text)
}

/// The emoji as kept (a reaction key): a standard one, or a live custom one from a server `me` is in.
pub async fn clean_emoji(db: &SqlitePool, me: UserId, raw: &str) -> AppResult<String> {
    let key = crate::emoji::canonical_key(raw.trim());
    let ok = match key.strip_prefix("custom:") {
        Some(id) => match crate::emoji::load(db, id).await? {
            Some(e) if e.hash.is_some() => {
                if !crate::servers::routes::is_server_member(db, e.server_id, me).await? {
                    return Err(AppError::ForbiddenMsg(
                        "you can only use emoji from servers you're in".into(),
                    ));
                }
                true
            }
            _ => false,
        },
        None => crate::messages::reactions::is_standard(&key),
    };
    if !ok {
        return Err(AppError::BadRequest("unknown emoji".into()));
    }
    Ok(key)
}

/// `me`'s custom status, its end and recent ones, for their own sessions. A run-out one is none.
pub async fn mine(db: &SqlitePool, me: UserId, at: &str) -> anyhow::Result<MyCustomStatus> {
    let row: Option<(Option<String>, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT custom_text, custom_emoji, custom_until FROM users WHERE id = ?")
            .bind(me.to_string())
            .fetch_optional(db)
            .await?;
    let recent: Vec<(String, String)> = sqlx::query_as(
        "SELECT text, emoji FROM custom_status_recent WHERE user_id = ? ORDER BY used_at DESC, rowid DESC LIMIT ?",
    )
    .bind(me.to_string())
    .bind(RECENT)
    .fetch_all(db)
    .await?;
    let recent = recent
        .into_iter()
        .map(|(text, emoji)| CustomStatus {
            text,
            emoji: (!emoji.is_empty()).then_some(emoji),
        })
        .collect();
    let Some((Some(text), emoji, until)) = row else {
        return Ok(MyCustomStatus {
            recent,
            ..Default::default()
        });
    };
    if until.as_deref().is_some_and(|u| u <= at) {
        return Ok(MyCustomStatus {
            recent,
            ..Default::default()
        });
    }
    Ok(MyCustomStatus {
        current: Some(CustomStatus { text, emoji }),
        until,
        recent,
    })
}

/// Save `status` as `me`'s, and first among their recent ones (only the newest [`RECENT`] are kept).
pub(super) async fn save(
    tx: &mut SqliteConnection,
    me: UserId,
    status: &CustomStatus,
    until: Option<&str>,
    at: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE users SET custom_text = ?, custom_emoji = ?, custom_until = ? WHERE id = ?",
    )
    .bind(&status.text)
    .bind(&status.emoji)
    .bind(until)
    .bind(me.to_string())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO custom_status_recent (user_id, text, emoji, used_at) VALUES (?, ?, ?, ?) \
         ON CONFLICT (user_id, text, emoji) DO UPDATE SET used_at = excluded.used_at",
    )
    .bind(me.to_string())
    .bind(&status.text)
    .bind(status.emoji.as_deref().unwrap_or(""))
    .bind(at)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM custom_status_recent WHERE user_id = ?1 AND rowid NOT IN \
         (SELECT rowid FROM custom_status_recent WHERE user_id = ?1 ORDER BY used_at DESC, rowid DESC LIMIT ?2)",
    )
    .bind(me.to_string())
    .bind(RECENT)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Clear `me`'s custom status (their recent ones stay).
pub(super) async fn clear(tx: &mut SqliteConnection, me: UserId) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE users SET custom_text = NULL, custom_emoji = NULL, custom_until = NULL WHERE id = ?",
    )
    .bind(me.to_string())
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// On `tx`, clear every custom status whose timer ran out by `now` (only `who`'s if given); whose, for announcing
/// after commit.
pub(super) async fn end_expired(
    tx: &mut SqliteConnection,
    now: &str,
    who: Option<UserId>,
) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar(
        "UPDATE users SET custom_text = NULL, custom_emoji = NULL, custom_until = NULL \
         WHERE custom_until IS NOT NULL AND custom_until <= ?1 AND (?2 IS NULL OR id = ?2) RETURNING id",
    )
    .bind(now)
    .bind(who.map(|u| u.to_string()))
    .fetch_all(&mut *tx)
    .await
}

/// `user` left `server` (#418): a custom status showing one of its emoji drops the emoji and keeps the text (one that
/// was only that emoji goes), as they can't use its emoji any more; recent ones with its emoji go too, since picking
/// them again would be refused. Whether their status changed, for announcing after commit (`announce_own`, then
/// `Hub::sync_presence`). A run-out status changes like any other: the sweep clears it anyway.
pub(crate) async fn drop_server_emoji(
    tx: &mut SqliteConnection,
    user: UserId,
    server: pulse_protocol::ids::ServerId,
) -> sqlx::Result<bool> {
    const THEIRS: &str = "(SELECT 'custom:' || id FROM custom_emoji WHERE server_id = ?)";
    let (uid, sid) = (user.to_string(), server.to_string());
    let changed = sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE users SET
           custom_text = CASE WHEN custom_text = '' THEN NULL ELSE custom_text END,
           custom_until = CASE WHEN custom_text = '' THEN NULL ELSE custom_until END,
           custom_emoji = NULL
         WHERE id = ? AND custom_emoji IN {THEIRS}"
    )))
    .bind(&uid)
    .bind(&sid)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DELETE FROM custom_status_recent WHERE user_id = ? AND emoji IN {THEIRS}"
    )))
    .bind(&uid)
    .bind(&sid)
    .execute(&mut *tx)
    .await?;
    Ok(changed > 0)
}

/// Tell `me`'s own sessions their custom status as it now stands.
pub(crate) async fn announce_own(s: &AppState, me: UserId) {
    match mine(&s.db, me, &now()).await {
        Ok(status) => {
            s.hub
                .publish(
                    &s.db,
                    Event::MyCustomStatusChanged {
                        user_id: me,
                        status,
                    },
                )
                .await
        }
        Err(e) => tracing::error!(error = ?e, "custom status not announced to its owner"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_trimmed_collapsed_and_stripped() {
        assert_eq!(clean_text("  Gaming \n\t now  ").unwrap(), "Gaming now");
        // control characters go; a line break is whitespace, so words stay apart
        assert_eq!(clean_text("a\u{0007}b\r\nc").unwrap(), "ab c");
        // bidi overrides, isolates and marks go
        assert_eq!(
            clean_text("\u{202E}evil\u{202C} \u{2067}x\u{2069}\u{200F}").unwrap(),
            "evil x"
        );
        // emoji sequences keep their joiners
        assert_eq!(clean_text("👩‍💻 coding").unwrap(), "👩‍💻 coding");
        assert_eq!(clean_text("   ").unwrap(), "");
    }

    #[test]
    fn text_is_capped_in_characters() {
        let max = "é".repeat(CUSTOM_STATUS_MAX);
        assert_eq!(clean_text(&max).unwrap(), max);
        assert!(clean_text(&format!("{max}x")).is_err());
        // whitespace that collapses doesn't count against it
        assert!(clean_text(&format!("  {max}  ")).is_ok());
    }
}
