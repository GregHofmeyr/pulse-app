//! Block a person (#474): `PUT|DELETE /me/blocks/{user_id}`. Yours only; they're never told.
//!
//! What a block does, and where the server enforces it (the rest, what you see of them in servers you share, is your
//! client's, from Ready's `blocks` and `BlocksChanged`):
//! - **Their DMs never reach you.** A message they send you in your DM is kept with `withheld_from` set to you: it
//!   looks sent on their side (no error, no read state) and nothing about it reaches you, now or after an unblock
//!   (history, unread counts, search, the inbox and every event about it skip it). Their typing, calls and game
//!   invites in that DM don't reach you either ([`dm_drop`], read by the audience).
//! - **Their pings never reach you**, anywhere: their mentions of you aren't inbox items or mention counts.
//! - **Their direct server invites skip you** (#80): the invite is made and looks sent, and its card in your DM is
//!   withheld like a message.
//! - **You always look offline to them, with an emptied profile**: the presence rule ([`crate::presence`]) treats them
//!   as someone you appear offline to, and every copy of your profile they get (Ready, introductions, profile
//!   changes, your profile card) has the default avatar and no banner, bio, status, badges, games or name style.
//!   Your messages in servers you share stay readable to them.
//!
//! You can't message someone you've blocked (unblock them first).

use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{Block, BlockMode, BlockRequest};
use sqlx::SqlitePool;

use crate::AppState;
use crate::auth::AuthUser;
use crate::db::now;
use crate::error::{AppError, AppResult};

/// People one person may block.
pub const MAX_PEOPLE: i64 = 1000;

pub fn router() -> Router<AppState> {
    Router::new().route("/me/blocks/{id}", put(set).delete(clear))
}

fn mode_str(m: BlockMode) -> &'static str {
    match m {
        BlockMode::Line => "line",
        BlockMode::Hide => "hide",
    }
}

/// `owner`'s blocks, in the order they were made.
pub async fn list(db: &SqlitePool, owner: UserId) -> AppResult<Vec<Block>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT target_id, mode FROM blocks WHERE blocker_id = ? ORDER BY created_at, rowid",
    )
    .bind(owner.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|(id, mode)| {
            Ok(Block {
                user_id: id
                    .parse()
                    .map_err(|e| AppError::from(anyhow::Error::from(e)))?,
                mode: if mode == "hide" {
                    BlockMode::Hide
                } else {
                    BlockMode::Line
                },
            })
        })
        .collect()
}

/// Whether `blocker` has blocked `target`.
pub async fn has_blocked<'e>(
    db: impl sqlx::SqliteExecutor<'e>,
    blocker: UserId,
    target: UserId,
) -> AppResult<bool> {
    let found: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM blocks WHERE blocker_id = ? AND target_id = ?")
            .bind(blocker.to_string())
            .bind(target.to_string())
            .fetch_optional(db)
            .await?;
    Ok(found.is_some())
}

/// The people who have blocked `target`.
pub async fn blocked_by(db: &SqlitePool, target: UserId) -> anyhow::Result<HashSet<UserId>> {
    let ids: Vec<String> = sqlx::query_scalar("SELECT blocker_id FROM blocks WHERE target_id = ?")
        .bind(target.to_string())
        .fetch_all(db)
        .await?;
    Ok(ids.iter().map(|i| i.parse()).collect::<Result<_, _>>()?)
}

/// SQL true when the user in column `author` is someone `viewer` (a column or placeholder) has blocked: their mentions
/// of the viewer aren't pings (#474). Both are pasted in as they are, so they are `'static` SQL, never request data.
pub fn blocked_sql(viewer: &'static str, author: &'static str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM blocks bk WHERE bk.blocker_id = {viewer} AND bk.target_id = {author})"
    )
}

/// SQL true when the message whose `withheld_from` column is `col` may be seen by `viewer` (a column or placeholder):
/// it wasn't withheld from them. `'static` SQL only, as above.
pub fn visible_sql(col: &'static str, viewer: &'static str) -> String {
    format!("({col} IS NULL OR {col} != {viewer})")
}

/// In a 1:1 DM between `author` and `other`: who a message from `author` is withheld from (`other`, when they've
/// blocked `author`).
pub async fn withhold_for<'e>(
    db: impl sqlx::SqliteExecutor<'e>,
    author: UserId,
    other: UserId,
) -> AppResult<Option<UserId>> {
    Ok(has_blocked(db, other, author).await?.then_some(other))
}

/// The person named: real and not you. Also the rate limit.
async fn target(s: &AppState, me: UserId, id: &str) -> AppResult<UserId> {
    let target: UserId = id.parse().map_err(|_| AppError::NotFound)?;
    if target == me {
        return Err(AppError::BadRequest("You can't block yourself.".into()));
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
            "You're changing who you block too fast. Try again in a minute.".into(),
        )));
    }
    Ok(target)
}

/// `PUT /me/blocks/{id}`: block them, or change how the block shows. Answers with your whole list.
async fn set(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
    body: Option<Json<BlockRequest>>,
) -> AppResult<Json<Vec<Block>>> {
    let target = target(&s, me, &id).await?;
    let mode = body.map(|Json(b)| b.mode).unwrap_or_default();
    let mut tx = crate::db::begin_write(&s.db).await?;
    let (count, has): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(target_id = ?), 0) FROM blocks WHERE blocker_id = ?",
    )
    .bind(target.to_string())
    .bind(me.to_string())
    .fetch_one(&mut *tx)
    .await?;
    if has == 0 && count >= MAX_PEOPLE {
        return Err(AppError::BadRequest(format!(
            "You can block up to {MAX_PEOPLE} people."
        )));
    }
    sqlx::query(
        "INSERT INTO blocks (blocker_id, target_id, mode, created_at) VALUES (?, ?, ?, ?)
         ON CONFLICT (blocker_id, target_id) DO UPDATE SET mode = excluded.mode",
    )
    .bind(me.to_string())
    .bind(target.to_string())
    .bind(mode_str(mode))
    .bind(now())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    tracing::info!(new = has == 0, "block set");
    Ok(Json(announce(&s, me, target).await?))
}

/// `DELETE /me/blocks/{id}`: unblock them. Answers with your whole list. What they sent while blocked stays withheld.
async fn clear(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Vec<Block>>> {
    let target = target(&s, me, &id).await?;
    sqlx::query("DELETE FROM blocks WHERE blocker_id = ? AND target_id = ?")
        .bind(me.to_string())
        .bind(target.to_string())
        .execute(&s.db)
        .await?;
    Ok(Json(announce(&s, me, target).await?))
}

/// Tell `me`'s own sessions their list as it is now; then what `target` may now see of `me` (presence, and their copy
/// of `me`'s profile: emptied while blocked, the real one again after).
async fn announce(s: &AppState, me: UserId, target: UserId) -> AppResult<Vec<Block>> {
    let list = list(&s.db, me).await?;
    s.hub
        .publish(
            &s.db,
            Event::BlocksChanged {
                user_id: me,
                list: list.clone(),
            },
        )
        .await;
    s.hub.sync_presence(&s.db, me).await;
    if crate::presence::known_to(&s.db, me)
        .await?
        .contains(&target)
    {
        // load_profile empties it for them while they're blocked
        if let Some(p) = crate::profiles::load_profile(&s.db, me, target).await? {
            s.hub
                .publish_to(
                    &s.db,
                    Event::ProfileUpdated {
                        user: p.user,
                        card: p.card,
                    },
                    &[target].into_iter().collect(),
                )
                .await;
        }
    }
    Ok(list)
}

/// The default card theme (as a new account's).
const THEME_TOP: &str = "#3a3d46";
const THEME_BOTTOM: &str = "#16171b";

/// What someone blocked sees of the person who blocked them (#474): the default avatar, no banner, bio, pronouns or
/// name style, the default card. Their names stay (they're how anyone finds them).
pub fn emptied(
    user: pulse_protocol::rest::User,
    card: pulse_protocol::rest::CardProfile,
) -> (
    pulse_protocol::rest::User,
    pulse_protocol::rest::CardProfile,
) {
    let user = emptied_user(user);
    let card = pulse_protocol::rest::CardProfile {
        banner_hash: None,
        banner_still: None,
        theme_top: THEME_TOP.into(),
        theme_bottom: THEME_BOTTOM.into(),
        pronouns: None,
        bio: None,
        glow: card.glow,
    };
    (user, card)
}

/// Just the [`pulse_protocol::rest::User`] part of [`emptied`]: the default avatar and no name style.
pub fn emptied_user(mut user: pulse_protocol::rest::User) -> pulse_protocol::rest::User {
    user.avatar_hash = None;
    user.avatar_still = None;
    user.style = Default::default();
    user
}

/// A whole profile as someone blocked sees it: [`emptied`], and no badges or games.
pub fn emptied_profile(p: pulse_protocol::rest::Profile) -> pulse_protocol::rest::Profile {
    let (user, card) = emptied(p.user, p.card);
    pulse_protocol::rest::Profile {
        user,
        card,
        stats: pulse_protocol::rest::ProfileStats {
            top_reaction: None,
            steam_completion: None,
            steam_perfect_games: None,
            steam_recent_minutes: None,
            ..p.stats
        },
        badges: vec![],
        showcase: vec![],
        showcase_chosen: false,
        ..p
    }
}

/// The message an event in a conversation is about, if any (its withholding decides who hears of it).
fn message_of(event: &Event) -> Option<pulse_protocol::ids::MessageId> {
    match event {
        Event::MessageCreated { message, .. } | Event::MessageUpdated { message } => {
            Some(message.id)
        }
        Event::MessageDeleted { message_id, .. }
        | Event::ReactionAdded { message_id, .. }
        | Event::ReactionRemoved { message_id, .. }
        | Event::MessageUnpinned { message_id, .. }
        | Event::PollChanged { message_id, .. }
        | Event::PlanChanged { message_id, .. }
        | Event::GameInviteChanged { message_id, .. }
        | Event::QuoteAwardChanged { message_id, .. } => Some(*message_id),
        Event::MessagePinned { pinned, .. } => Some(pinned.message.id),
        _ => None,
    }
}

/// Who did what an event in a conversation tells of, when someone did.
fn actor_of(event: &Event) -> Option<UserId> {
    match event {
        Event::MessageCreated { message, .. } => message.author_id,
        Event::Typing { user_id, .. }
        | Event::ReactionAdded { user_id, .. }
        | Event::ReactionRemoved { user_id, .. }
        | Event::VoiceJoined { user_id, .. }
        | Event::SoundPlayed { user_id, .. } => Some(*user_id),
        Event::CallStarted { call } => call.started_by,
        Event::CallRinging { ring } => Some(ring.caller_id),
        _ => None,
    }
}

/// Of the `members` of a 1:1 DM, who must not hear `event` (#474): the person a message it's about was withheld from,
/// and anyone who has blocked whoever did what it tells of (their typing, their call, their reaction).
pub async fn dm_drop(
    db: &SqlitePool,
    event: &Event,
    members: &HashSet<UserId>,
) -> anyhow::Result<HashSet<UserId>> {
    let mut drop = HashSet::new();
    if let Some(id) = message_of(event) {
        let withheld: Option<Option<String>> =
            sqlx::query_scalar("SELECT withheld_from FROM messages WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(db)
                .await?;
        if let Some(Some(from)) = withheld {
            drop.insert(from.parse()?);
        }
    }
    if let Some(actor) = actor_of(event) {
        for b in blocked_by(db, actor).await? {
            if members.contains(&b) {
                drop.insert(b);
            }
        }
    }
    Ok(drop)
}
