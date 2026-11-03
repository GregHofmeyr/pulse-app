//! Voice counters, bumped in the transaction that opens or closes a `voice_sessions` row. Only
//! server voice channels have rows, so DM and group calls never get here.

use chrono::{DateTime, Utc};
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use sqlx::SqliteConnection;

use super::catalogue::stat;
use super::stats;
use super::time::{accompanied, covers_3am, parse, sast_day, sast_day_start, seconds};
use crate::db::stamp;
use crate::error::AppResult;

/// Shorter than this in voice is a Ghost visit (seconds).
const GHOST_UNDER: i64 = 5;

/// `user` just opened voice session `session` in a channel of `server`: if nobody had been into
/// voice anywhere in that server yet today (South African), that's one more Early Bird day, once
/// a day.
pub async fn joined(
    conn: &mut SqliteConnection,
    server: ServerId,
    user: UserId,
    session: &str,
    at: DateTime<Utc>,
) -> AppResult<()> {
    let earlier: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM voice_sessions v JOIN channels c ON c.id = v.channel_id
         WHERE c.server_id = ? AND v.joined_at >= ? AND v.id != ? LIMIT 1",
    )
    .bind(server.to_string())
    .bind(stamp(sast_day_start(at)))
    .bind(session)
    .fetch_optional(&mut *conn)
    .await?;
    let today = sast_day(at);
    if earlier.is_none() && stats::get(conn, user, stat::EARLY_BIRD_LAST_DAY).await? < today {
        stats::add(conn, user, stat::EARLY_BIRD_DAYS, 1).await?;
        stats::set(conn, user, stat::EARLY_BIRD_LAST_DAY, today).await?;
    }
    Ok(())
}

/// `user`'s session in `channel` from `joined` to `left` just closed. The time someone else was
/// there too is voice time (and Night Owl if that held 03:00); under five seconds is a Ghost
/// visit. Others still in the call count as there until `left`.
pub async fn left(
    conn: &mut SqliteConnection,
    user: UserId,
    channel: ChannelId,
    joined: &str,
    left: &str,
) -> AppResult<()> {
    let others: Vec<(String, String)> = sqlx::query_as(
        "SELECT joined_at, COALESCE(left_at, ?) FROM voice_sessions
         WHERE channel_id = ? AND user_id != ? AND joined_at < ? AND (left_at IS NULL OR left_at > ?)",
    )
    .bind(left)
    .bind(channel.to_string())
    .bind(user.to_string())
    .bind(left)
    .bind(joined)
    .fetch_all(&mut *conn)
    .await?;
    let others = others
        .iter()
        .map(|(a, b)| Ok((parse(a)?, parse(b)?)))
        .collect::<AppResult<Vec<_>>>()?;
    let mine = (parse(joined)?, parse(left)?);
    let together = accompanied(mine, &others);
    let secs = seconds(&together);
    if secs > 0 {
        stats::add(conn, user, stat::VOICE_SECS, secs).await?;
    }
    if covers_3am(&together) {
        stats::add(conn, user, stat::NIGHT_OWL, 1).await?;
    }
    if (mine.1 - mine.0).num_seconds() < GHOST_UNDER {
        stats::add(conn, user, stat::GHOST, 1).await?;
    }
    Ok(())
}
