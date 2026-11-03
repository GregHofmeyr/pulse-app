//! Voice counters, bumped once a `voice_sessions` row has opened or closed, in a transaction of
//! their own: bookkeeping never fails the join, leave or delete that caused it (errors are
//! logged). Only server voice channels have rows, so DM and group calls never get here.

use chrono::{DateTime, Utc};
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use sqlx::SqliteConnection;

use super::catalogue::stat;
use super::time::{accompanied, covers_3am, parse, sast_day, sast_day_start, seconds};
use super::{engine, stats};
use crate::AppState;
use crate::db::{begin_write, stamp};
use crate::error::AppResult;

/// Shorter than this in voice is a Ghost visit (seconds).
const GHOST_UNDER: i64 = 5;

/// `user` opened voice session `session` (committed) at `at` in a channel of `server`.
pub async fn opened(
    s: &AppState,
    server: ServerId,
    user: UserId,
    session: &str,
    at: DateTime<Utc>,
) {
    let r = async {
        let mut tx = begin_write(&s.db).await?;
        joined(&mut tx, server, user, session, at).await?;
        tx.commit().await?;
        AppResult::Ok(())
    }
    .await;
    match r {
        Ok(()) => engine::after(s, &[user]).await,
        Err(e) => tracing::warn!(error = ?e, %user, "couldn't count a voice join toward badges"),
    }
}

/// These sessions in `channel` (user, joined_at, left_at) just closed (committed).
pub async fn closed(s: &AppState, channel: ChannelId, sessions: &[(UserId, String, String)]) {
    if sessions.is_empty() {
        return;
    }
    let r = async {
        let mut tx = begin_write(&s.db).await?;
        for (user, joined, gone) in sessions {
            left(&mut tx, *user, channel, joined, gone).await?;
        }
        tx.commit().await?;
        AppResult::Ok(())
    }
    .await;
    let users: Vec<UserId> = sessions.iter().map(|(u, ..)| *u).collect();
    match r {
        Ok(()) => engine::after(s, &users).await,
        Err(e) => tracing::warn!(error = ?e, %channel, "couldn't count voice time toward badges"),
    }
}

/// `user` just opened voice session `session` at `at` in a channel of `server`: if nobody had been
/// into voice anywhere in that server earlier today (South African), that's one more Early Bird
/// day, once a day. "Earlier" is by join time, then id, so two joins committed close together
/// never both see the other and both miss out.
pub async fn joined(
    conn: &mut SqliteConnection,
    server: ServerId,
    user: UserId,
    session: &str,
    at: DateTime<Utc>,
) -> AppResult<()> {
    let earlier: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM voice_sessions v JOIN channels c ON c.id = v.channel_id
         WHERE c.server_id = ? AND v.joined_at >= ? AND (v.joined_at, v.id) < (?, ?) LIMIT 1",
    )
    .bind(server.to_string())
    .bind(stamp(sast_day_start(at)))
    .bind(stamp(at))
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
