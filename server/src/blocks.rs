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
//!   Your messages in servers you share stay readable to them. That holds for your `User` inside member payloads too
//!   (Ready, the members list, joins, changes and a joined server's snapshot: [`member_views`]).
//! - **Nothing of theirs reopens a DM you closed**: their calls, polls, plans, spins, pins and game invites there are
//!   withheld like a message ([`dm_post`], [`withhold_line`]), and reopen it only for them ([`reopen`]).
//! - **Reactions say who reacted**, so your client leaves the people you hide out of the counts.
//! - **They can't put you in a group** (a new one, or an add to theirs: [`may_group_with`]); groups from before stay.
//! - **No shared DM call**: you can't call or join them in your DM (as you can't message them), and they can't join a
//!   call you're in there; blocking them mid-call takes them out of it (`voice::routes::may_join_now`).
//! - **Their edits in your DM never reach you**: an edit withholds the message from you, as a send would.
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

/// Before `me` puts `people` in a group with them (a new group, or an add, #474): refused when any of them has blocked
/// `me`, with the same answer as for someone who isn't there, so it doesn't tell them they're blocked. Groups made
/// before the block stay (the blocker's client hides them there).
pub async fn may_group_with(
    tx: &mut sqlx::SqliteConnection,
    me: UserId,
    people: impl IntoIterator<Item = UserId>,
) -> AppResult<()> {
    for u in people {
        if u != me && has_blocked(&mut *tx, u, me).await? {
            return Err(AppError::BadRequest(UNKNOWN_USER.into()));
        }
    }
    Ok(())
}

/// The answer for a person who can't be put in a conversation: the same whether they don't exist or have blocked you.
pub const UNKNOWN_USER: &str = "unknown user";

/// In a 1:1 DM between `author` and `other`: who a message from `author` is withheld from (`other`, when they've
/// blocked `author`).
pub async fn withhold_for<'e>(
    db: impl sqlx::SqliteExecutor<'e>,
    author: UserId,
    other: UserId,
) -> AppResult<Option<UserId>> {
    Ok(has_blocked(db, other, author).await?.then_some(other))
}

/// Before `me` posts anything in `ch` (a message, poll, plan, spin, game invite): in a 1:1 DM with someone they've
/// blocked, refused (unblock them first); with someone who has blocked them, who it's withheld from (#474).
pub async fn dm_post(
    tx: &mut sqlx::SqliteConnection,
    ch: &pulse_protocol::rest::Channel,
    me: UserId,
) -> AppResult<Option<UserId>> {
    match crate::messages::routes::dm_other(&mut *tx, ch, me).await? {
        Some(other) if has_blocked(&mut *tx, me, other).await? => Err(AppError::ForbiddenMsg(
            "You've blocked them. Unblock them to send a message.".into(),
        )),
        Some(other) => withhold_for(&mut *tx, me, other).await,
        None => Ok(None),
    }
}

/// A new post by `me` in conversation `channel` reopens it for anyone who had closed it; one `withheld` from
/// someone (#474) reopens it only for `me`, so nothing of theirs brings back a DM the blocker closed.
pub async fn reopen(
    tx: &mut sqlx::SqliteConnection,
    channel: pulse_protocol::ids::ChannelId,
    withheld: Option<UserId>,
    me: UserId,
) -> AppResult<()> {
    sqlx::query("DELETE FROM dm_hidden WHERE channel_id = ? AND (? IS NULL OR user_id = ?)")
        .bind(channel.to_string())
        .bind(withheld.map(|_| me.to_string()))
        .bind(me.to_string())
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Withhold the line `line` from `from`, when someone: a line about something withheld (a call, a pin, a poll's end,
/// a plan's reminder) is withheld from the same person.
pub async fn withhold_line(
    tx: &mut sqlx::SqliteConnection,
    line: pulse_protocol::ids::MessageId,
    from: Option<UserId>,
) -> AppResult<()> {
    if let Some(from) = from {
        sqlx::query("UPDATE messages SET withheld_from = ? WHERE id = ?")
            .bind(from.to_string())
            .bind(line.to_string())
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

/// Who the message `id` was withheld from, if anyone.
pub async fn withheld_from(
    tx: &mut sqlx::SqliteConnection,
    id: pulse_protocol::ids::MessageId,
) -> AppResult<Option<UserId>> {
    let from: Option<Option<String>> =
        sqlx::query_scalar("SELECT withheld_from FROM messages WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    from.flatten()
        .map(|f| {
            f.parse()
                .map_err(|e| AppError::from(anyhow::Error::from(e)))
        })
        .transpose()
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
    if has == 0 {
        end_shared_call(&s, me, target).await?;
    }
    Ok(Json(announce(&s, me, target).await?))
}

/// `me` just blocked `target`: if they're both in their 1:1 DM's call, `target` is taken out of it (out of its voice
/// room and its stream room, in the background), as they couldn't join it now (`voice::routes::may_join_now`). A call
/// `target` is in alone (one that rings out for them) is left alone, so nothing tells them.
async fn end_shared_call(s: &AppState, me: UserId, target: UserId) -> AppResult<()> {
    let Some(dm) = crate::servers::routes::existing_dm(&s.db, me, target).await? else {
        return Ok(());
    };
    let dm: pulse_protocol::ids::ChannelId = dm.parse().map_err(anyhow::Error::from)?;
    let room = s.voice.members(dm);
    if !(room.contains(&me) && room.contains(&target)) {
        return Ok(());
    }
    let Some(ch) = crate::access::load_channel(&s.db, dm).await? else {
        return Ok(());
    };
    tracing::info!(channel = %dm, "taking a blocked person out of a DM call");
    let s = s.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::voice::routes::left_call(&s, &ch, target, "", false).await {
            tracing::error!(error = ?e, channel = %dm, "couldn't take a blocked person out of a DM call");
        }
        crate::voice::revoke::remove_from_call(&s, dm, &[target]).await;
    });
    Ok(())
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

/// Just the [`pulse_protocol::rest::User`] part of [`emptied`]: the default avatar, no name style
/// and the plain "is typing" (#652).
pub fn emptied_user(mut user: pulse_protocol::rest::User) -> pulse_protocol::rest::User {
    user.avatar_hash = None;
    user.avatar_still = None;
    user.style = Default::default();
    user.typing_text = None;
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

/// `members` as someone sees them who was blocked by each of `blockers` (#508): those people's `User` emptied, as
/// everywhere else they show.
pub fn empty_members(members: &mut [pulse_protocol::rest::Member], blockers: &HashSet<UserId>) {
    if blockers.is_empty() {
        return;
    }
    for m in members.iter_mut() {
        if blockers.contains(&m.user.id) {
            m.user = emptied_user(m.user.clone());
        }
    }
}

/// The members an event carries a `User` for: a join or change, or a whole server's.
fn members_of(event: &mut Event) -> Option<&mut [pulse_protocol::rest::Member]> {
    match event {
        Event::MemberJoined { member, .. } | Event::MemberUpdated { member, .. } => {
            Some(std::slice::from_mut(member))
        }
        Event::ServerJoined { snapshot, .. } | Event::ServerRestored { snapshot } => {
            Some(&mut snapshot.members)
        }
        _ => None,
    }
}

/// For an event that carries members (#508), the copy each person blocked by any of them gets instead: those
/// members' `User` emptied. Everyone else gets `event` as it is (and so does everyone, for any other event).
pub async fn member_views(db: &SqlitePool, event: &Event) -> anyhow::Result<Vec<(UserId, Event)>> {
    let mut probe = event.clone();
    let Some(members) = members_of(&mut probe) else {
        return Ok(vec![]);
    };
    if members.is_empty() {
        return Ok(vec![]);
    }
    let mut q =
        sqlx::QueryBuilder::new("SELECT blocker_id, target_id FROM blocks WHERE blocker_id IN (");
    let mut list = q.separated(", ");
    for m in members.iter() {
        list.push_bind(m.user.id.to_string());
    }
    q.push(")");
    let pairs: Vec<(String, String)> = q.build_query_as().fetch_all(db).await?;
    let mut by_viewer: std::collections::HashMap<UserId, HashSet<UserId>> = Default::default();
    for (blocker, target) in pairs {
        by_viewer
            .entry(target.parse()?)
            .or_default()
            .insert(blocker.parse()?);
    }
    Ok(by_viewer
        .into_iter()
        .map(|(viewer, blockers)| {
            let mut copy = event.clone();
            if let Some(members) = members_of(&mut copy) {
                empty_members(members, &blockers);
            }
            (viewer, copy)
        })
        .collect())
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
