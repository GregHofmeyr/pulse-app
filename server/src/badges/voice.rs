//! Voice counters, bumped once a `voice_sessions` row has opened or closed, in a transaction of
//! their own: bookkeeping never fails the join, leave or delete that caused it (errors are
//! logged). Only server voice channels have rows, so DM and group calls never get here.

use chrono::{DateTime, Duration, Utc};
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use sqlx::SqliteConnection;

use super::catalogue::stat;
use super::time::{accompanied, covers_3am, parse, sast_day, sast_day_start, seconds};
use super::{engine, stats};
use crate::AppState;
use crate::db::{begin_write, stamp};
use crate::error::AppResult;

/// Shorter than this in voice is a Ghost visit (seconds)...
const GHOST_UNDER: i64 = 5;
/// ...unless it began this soon after you left the same channel: that's a reconnect (a network
/// blip, or spamming join), not a drop-in (seconds, #130).
const RECONNECT_WITHIN: i64 = 10;

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
        let early_bird = joined(&mut tx, server, user, session, at).await?;
        tx.commit().await?;
        AppResult::Ok(early_bird)
    }
    .await;
    match r {
        Ok(early_bird) => {
            let users: Vec<UserId> = std::iter::once(user).chain(early_bird).collect();
            engine::after(s, &users).await
        }
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

/// `user` just opened voice session `session` at `at` in a channel of `server`. Early Bird (#130):
/// whoever was first into voice anywhere in that server today (South African) gets the day once
/// someone else joins after them, so a day alone doesn't count; and nobody does if someone was
/// already in voice there across midnight. Once a day per person. "First" is by join time, then
/// id, so joins committed close together all agree on it. Returns the early bird credited.
pub async fn joined(
    conn: &mut SqliteConnection,
    server: ServerId,
    user: UserId,
    session: &str,
    at: DateTime<Utc>,
) -> AppResult<Option<UserId>> {
    let day_start = stamp(sast_day_start(at));
    let carried_over: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM voice_sessions v JOIN channels c ON c.id = v.channel_id
         WHERE c.server_id = ? AND v.joined_at < ? AND (v.left_at IS NULL OR v.left_at > ?) LIMIT 1",
    )
    .bind(server.to_string())
    .bind(&day_start)
    .bind(&day_start)
    .fetch_optional(&mut *conn)
    .await?;
    if carried_over.is_some() {
        return Ok(None);
    }
    let first: Option<String> = sqlx::query_scalar(
        "SELECT v.user_id FROM voice_sessions v JOIN channels c ON c.id = v.channel_id
         WHERE c.server_id = ? AND v.joined_at >= ? AND (v.joined_at, v.id) <= (?, ?)
         ORDER BY v.joined_at, v.id LIMIT 1",
    )
    .bind(server.to_string())
    .bind(&day_start)
    .bind(stamp(at))
    .bind(session)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(first) = first.and_then(|f| f.parse::<UserId>().ok()) else {
        return Ok(None);
    };
    let today = sast_day(at);
    if first == user || stats::get(conn, first, stat::EARLY_BIRD_LAST_DAY).await? >= today {
        return Ok(None);
    }
    stats::add(conn, first, stat::EARLY_BIRD_DAYS, 1).await?;
    stats::set(conn, first, stat::EARLY_BIRD_LAST_DAY, today).await?;
    Ok(Some(first))
}

/// `user`'s session in `channel` from `joined` to `left` just closed. The time someone else was
/// there too is voice time (and Night Owl if that held 03:00); under five seconds is a Ghost
/// visit, unless it was a reconnect. Others still in the call count as there until `left`.
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
        let rejoined: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM voice_sessions
             WHERE user_id = ? AND channel_id = ? AND joined_at < ? AND (left_at IS NULL OR left_at >= ?)
             LIMIT 1",
        )
        .bind(user.to_string())
        .bind(channel.to_string())
        .bind(joined)
        .bind(stamp(mine.0 - Duration::seconds(RECONNECT_WITHIN)))
        .fetch_optional(&mut *conn)
        .await?;
        if rejoined.is_none() {
            stats::add(conn, user, stat::GHOST, 1).await?;
        }
    }
    Ok(())
}
