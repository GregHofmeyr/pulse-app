//! Small counter bumps the routes make inside their own write transactions, for things that need
//! more than one `stats::add`. Callers only call these for server channels (never DMs or groups)
//! and pass the users returned here to [`super::engine::after`] once they have committed.

use pulse_protocol::ids::{MessageId, UserId};
use sqlx::SqliteConnection;

use super::catalogue::stat;
use super::stats;
use crate::error::AppResult;

/// Uses of one custom emoji that make it count toward Emoji Smith.
const SMITH_USES: i64 = 10;
/// Distinct reactors (other than the author) that make a clip a Highlight Reel.
const HIGHLIGHT_REACTORS: i64 = 5;

/// `user` used these emoji keys (a message's live keys, or one reaction): each `custom:<id>`
/// someone else uploaded is one more use for its uploader, and the use that makes ten counts the
/// emoji toward Emoji Smith. Returns the uploaders, to check.
pub async fn emoji_used(
    conn: &mut SqliteConnection,
    user: UserId,
    keys: &[String],
) -> AppResult<Vec<UserId>> {
    emoji_uses(conn, user, keys, 1).await
}

/// A use taken back (a reaction taken off), so toggling one can't farm uses. Dropping below ten
/// takes the emoji back off the Emoji Smith count; the badge itself stays.
pub async fn emoji_unused(
    conn: &mut SqliteConnection,
    user: UserId,
    keys: &[String],
) -> AppResult<()> {
    emoji_uses(conn, user, keys, -1).await.map(drop)
}

async fn emoji_uses(
    conn: &mut SqliteConnection,
    user: UserId,
    keys: &[String],
    delta: i64,
) -> AppResult<Vec<UserId>> {
    let mut uploaders = Vec::new();
    for id in keys.iter().filter_map(|k| k.strip_prefix("custom:")) {
        let added_by: Option<Option<String>> =
            sqlx::query_scalar("SELECT added_by FROM custom_emoji WHERE id = ?")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?;
        let Some(uploader) = added_by.flatten().and_then(|u| u.parse::<UserId>().ok()) else {
            continue;
        };
        if uploader == user {
            continue;
        }
        let key = format!("{}{id}", stat::EMOJI_USES_PREFIX);
        let n = stats::add(conn, uploader, &key, delta).await?;
        // Crossing ten either way; the floor at 0 means a removal at 0 crosses nothing.
        if delta > 0 && n == SMITH_USES {
            stats::add(conn, uploader, stat::EMOJI_SMITH, 1).await?;
        } else if delta < 0 && n == SMITH_USES - 1 {
            stats::add(conn, uploader, stat::EMOJI_SMITH, -1).await?;
        }
        if !uploaders.contains(&uploader) {
            uploaders.push(uploader);
        }
    }
    Ok(uploaders)
}

/// `reactor` put `emoji` on (`added`) or took it off `msg`. Someone else's reaction is one more
/// (or one fewer) received by the author, so toggling nets nothing; custom emoji count as uses;
/// and a clip with five other people reacting earns its author Highlight Reel. Returns the users
/// to check.
pub async fn reacted(
    conn: &mut SqliteConnection,
    reactor: UserId,
    msg: MessageId,
    emoji: &str,
    added: bool,
) -> AppResult<Vec<UserId>> {
    let keys = [emoji.to_string()];
    let mut check = if added {
        emoji_used(conn, reactor, &keys).await?
    } else {
        emoji_unused(conn, reactor, &keys).await?;
        Vec::new()
    };
    let author: Option<Option<String>> =
        sqlx::query_scalar("SELECT author_id FROM messages WHERE id = ?")
            .bind(msg.to_string())
            .fetch_optional(&mut *conn)
            .await?;
    let Some(author) = author.flatten().and_then(|a| a.parse::<UserId>().ok()) else {
        return Ok(check);
    };
    if author == reactor {
        return Ok(check);
    }
    stats::add(
        conn,
        author,
        stat::REACTIONS_RECEIVED,
        if added { 1 } else { -1 },
    )
    .await?;
    if added {
        let reactors: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT r.user_id) FROM reactions r
             JOIN clips c ON c.message_id = r.message_id
             WHERE r.message_id = ? AND r.user_id != ?",
        )
        .bind(msg.to_string())
        .bind(author.to_string())
        .fetch_one(&mut *conn)
        .await?;
        if reactors >= HIGHLIGHT_REACTORS {
            stats::set_max(conn, author, stat::HIGHLIGHT_REEL, 1).await?;
        }
    }
    if !check.contains(&author) {
        check.push(author);
    }
    Ok(check)
}
