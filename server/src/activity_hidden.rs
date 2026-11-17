//! Hide what I'm playing per person (#436): `PUT|DELETE /me/activity-hidden/{user_id}`. Someone on your list never
//! gets your game activity: your "Playing" (and the Join inside it), your Steam hours (profile card and a server's
//! board), and your Steam completion and 100 % games on your profile card (Steam badges still show to everyone: one vanishing would be a clearer tell than it ticking up). They see you online with no game, as if you played nothing. "Listening to" isn't game activity and isn't
//! touched.
//!
//! Enforced where game activity leaves the server: the hub's per-viewer pass (`presence::Subject::shows_game_to`,
//! read by `Hub::sync_presence` and Ready's people), and the Steam hours and stat reads. A change re-runs the pass, so a newly
//! hidden viewer's clients drop your game at once and an un-hidden one gets it back.
//!
//! Posts that show someone's games to a whole channel (Steam achievement cards, Deadlock match cards and rank movers)
//! are one message for everyone, so they can't be filtered per viewer: a post about you skips every server where
//! anyone you hide your games from is a member ([`hides_in_server`]). Privacy wins over the post (#438).
//!
//! Kept per person (`activity_hidden`) so it follows you to every device, sent only to you (Ready's `activity_hidden`
//! and `ActivityHiddenChanged`, always the whole list). The person newly hidden (or shown again) gets
//! `GameStatsChanged`, naming only you, so a card of yours they have open asks again and drops (or gets back) your
//! Steam hours, Steam stats and Deadlock section (#438). It goes out only when you have something there
//! that hiding changes for them (a game now, shared Steam hours or completion, or shared Deadlock
//! data), and like the re-run pass it says no more than that your game activity changed.

use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::UserId;
use sqlx::SqlitePool;

use crate::AppState;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};

/// People one person may have on the list.
pub const MAX_PEOPLE: i64 = 200;

pub fn router() -> Router<AppState> {
    Router::new().route("/me/activity-hidden/{id}", put(set).delete(clear))
}

/// `owner`'s list, in the order they were added.
pub async fn list(db: &SqlitePool, owner: UserId) -> AppResult<Vec<UserId>> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT target_id FROM activity_hidden WHERE user_id = ? ORDER BY created_at, rowid",
    )
    .bind(owner.to_string())
    .fetch_all(db)
    .await?;
    ids.iter()
        .map(|i| {
            i.parse()
                .map_err(|e| AppError::from(anyhow::Error::from(e)))
        })
        .collect()
}

/// The person named: real and not you. Also the rate limit.
async fn target(s: &AppState, me: UserId, id: &str) -> AppResult<UserId> {
    // a malformed id names no user, so it is a 404 like an unknown one
    let target: UserId = id.parse().map_err(|_| AppError::NotFound)?;
    if target == me {
        return Err(AppError::BadRequest(
            "You always see what you're playing.".into(),
        ));
    }
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM users WHERE id = ?")
        .bind(target.to_string())
        .fetch_optional(&s.db)
        .await?;
    if exists.is_none() {
        return Err(AppError::NotFound);
    }
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(Some(
            "You're changing who sees your games too fast. Try again in a minute.".into(),
        )));
    }
    Ok(target)
}

/// `PUT /me/activity-hidden/{id}`: hide your game activity from them. Answers with your whole list.
async fn set(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Vec<UserId>>> {
    let target = target(&s, me, &id).await?;
    let mut tx = crate::db::begin_write(&s.db).await?;
    let (count, has): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(target_id = ?), 0) FROM activity_hidden WHERE user_id = ?",
    )
    .bind(target.to_string())
    .bind(me.to_string())
    .fetch_one(&mut *tx)
    .await?;
    if has == 0 && count >= MAX_PEOPLE {
        return Err(AppError::BadRequest(format!(
            "What you're playing can be hidden from up to {MAX_PEOPLE} people."
        )));
    }
    let added = sqlx::query(
        "INSERT INTO activity_hidden (user_id, target_id, created_at) VALUES (?, ?, ?)
         ON CONFLICT (user_id, target_id) DO NOTHING",
    )
    .bind(me.to_string())
    .bind(target.to_string())
    .bind(now())
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    tx.commit().await?;
    let list = announce(&s, me).await?;
    if added {
        stats_changed(&s, me, target).await?;
    }
    Ok(Json(list))
}

/// `DELETE /me/activity-hidden/{id}`: show them your game activity again. Answers with your whole list.
async fn clear(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Vec<UserId>>> {
    let target = target(&s, me, &id).await?;
    let removed = sqlx::query("DELETE FROM activity_hidden WHERE user_id = ? AND target_id = ?")
        .bind(me.to_string())
        .bind(target.to_string())
        .execute(&s.db)
        .await?
        .rows_affected()
        > 0;
    let list = announce(&s, me).await?;
    if removed {
        stats_changed(&s, me, target).await?;
    }
    Ok(Json(list))
}

/// Tell `viewer`'s sessions that what they may see of `me`'s game stats changed, when `me` has any that hiding takes
/// away ([`has_game_stats`]): otherwise nothing they could see changed, and nothing goes out.
async fn stats_changed(s: &AppState, me: UserId, viewer: UserId) -> AppResult<()> {
    if !has_game_stats(s, me).await? {
        return Ok(());
    }
    s.hub
        .publish(
            &s.db,
            Event::GameStatsChanged {
                user_id: viewer,
                person_id: me,
            },
        )
        .await;
    Ok(())
}

/// Whether `me` has game activity that hiding it from someone takes off what they see: a game now, Steam hours or a
/// Steam completion they share, or Deadlock data they share.
async fn has_game_stats(s: &AppState, me: UserId) -> AppResult<bool> {
    if s.hub.playing_of(me).is_some() {
        return Ok(true);
    }
    let steam: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM steam_links l WHERE l.user_id = ? AND (
           (l.share_hours = 1 AND l.recent_minutes IS NOT NULL)
           OR (l.share_achievements = 1 AND l.completion_rate IS NOT NULL)
           OR ((l.deadlock_matches = 1 OR l.deadlock_rank = 1 OR l.deadlock_squad = 1)
               AND (EXISTS (SELECT 1 FROM deadlock_players p WHERE p.user_id = l.user_id)
                    OR EXISTS (SELECT 1 FROM deadlock_matches dm WHERE dm.user_id = l.user_id))))",
    )
    .bind(me.to_string())
    .fetch_optional(&s.db)
    .await?;
    Ok(steam.is_some())
}

/// Tell `me`'s own sessions their list as it is now, and every viewer what they may now see of `me`'s game.
async fn announce(s: &AppState, me: UserId) -> AppResult<Vec<UserId>> {
    let list = list(&s.db, me).await?;
    s.hub
        .publish(
            &s.db,
            Event::ActivityHiddenChanged {
                user_id: me,
                list: list.clone(),
            },
        )
        .await;
    s.hub.sync_presence(&s.db, me).await;
    Ok(list)
}

/// SQL that is true when the user in column `user` hides their game activity from any member of the server in column
/// `server` (#438): a channel post about their games there would reach someone they hide them from. Both are pasted
/// into the SQL as they are, so they are `'static` column names or placeholders, never request data.
pub fn hides_in_server(user: &'static str, server: &'static str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM activity_hidden h JOIN server_members hm ON hm.user_id = h.target_id
           AND hm.server_id = {server} WHERE h.user_id = {user})"
    )
}

/// Whether `owner` hides their game activity from `viewer` (never from themselves).
pub async fn hides<'e>(
    db: impl sqlx::SqliteExecutor<'e>,
    owner: UserId,
    viewer: UserId,
) -> sqlx::Result<bool> {
    if owner == viewer {
        return Ok(false);
    }
    let hit: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM activity_hidden WHERE user_id = ? AND target_id = ?")
            .bind(owner.to_string())
            .bind(viewer.to_string())
            .fetch_optional(db)
            .await?;
    Ok(hit.is_some())
}
