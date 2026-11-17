//! Invite to game (#436). An invite is an ordinary message in a DM or group (its `content` is the invite as text, for
//! clients that don't know invites) with a row in `game_invites`: the game as its sender's "Playing" showed it when
//! sent. Everyone in the chat sees the same card ([`GameInvite`]), so a change goes out whole, as `GameInviteChanged`.
//!
//! Where to join (a [`SteamJoin`]) is read from Steam when the invite is sent and on every Join round while it lasts
//! (`steam::join::tick`), whether or not the sender shares Join or hides what they're playing: sending an invite is
//! consent for that chat. It never leaves the server in a message: a member of the chat asks for it when they press
//! Join (`POST /messages/{id}/game-invite/join`), so nothing that copies or shows a message (notifications, search,
//! previews, forwards) can carry it.
//!
//! An invite ends, for good, when its sender stops playing that game ([`follow`], on every game change and
//! disconnect), when the lobby it offered is gone or Steam shows them in another one ([`refresh`]: an invite never
//! follows its sender to a lobby it didn't offer), or when its sender leaves or is removed from the chat
//! ([`end_for_member`]; and anything that reads an invite to hand it out or keep it going needs its sender still in the
//! chat, [`SENDER_IN`]). A server restart ends every open invite too (what
//! anyone plays lives in memory), but not for good: one whose sender reports the same game again within
//! [`RESTART_GRACE`] of startup opens again (#438). An ended invite keeps its card, greyed out, with nothing to join.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{GameArt, GameInvite, Message, MessageKind, Playing, SteamJoin};
use pulse_protocol::steam_join::valid_steam_id;
use sqlx::{SqliteConnection, SqlitePool};

use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::{begin_write, now};
use crate::error::{AppError, AppResult};

/// How long after startup an invite the restart ended can open again, when its sender reports the same game.
pub const RESTART_GRACE: chrono::Duration = chrono::Duration::minutes(10);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/channels/{id}/game-invite", post(create))
        .route("/messages/{id}/game-invite/join", post(join))
}

/// An invite as stored.
#[derive(Clone, Debug)]
pub(crate) struct Stored {
    pub message_id: MessageId,
    pub channel_id: ChannelId,
    pub sender_id: UserId,
    pub game: String,
    pub steam_app_id: Option<u32>,
    pub art: Option<GameArt>,
    pub join: Option<SteamJoin>,
    pub ended_at: Option<String>,
}

type Row = (
    String,
    String,
    String,
    String,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
);

const COLS: &str = "g.message_id, g.channel_id, g.sender_id, g.game, g.steam_app_id, g.art, g.join_info, g.ended_at \
     FROM game_invites g JOIN messages m ON m.id = g.message_id";

/// The invite's sender is still in its chat: an invite whose sender left hands out nothing and isn't kept going, even
/// if something left it open ([`end_for_member`] ends it when they go).
const SENDER_IN: &str = "EXISTS (SELECT 1 FROM channel_members cm \
     WHERE cm.channel_id = g.channel_id AND cm.user_id = g.sender_id)";

fn stored_from(r: Row) -> AppResult<Stored> {
    let (message_id, channel_id, sender_id, game, app, art, join, ended_at) = r;
    Ok(Stored {
        message_id: message_id.parse().map_err(anyhow::Error::from)?,
        channel_id: channel_id.parse().map_err(anyhow::Error::from)?,
        sender_id: sender_id.parse().map_err(anyhow::Error::from)?,
        game,
        steam_app_id: app.and_then(|a| u32::try_from(a).ok()),
        art: art.and_then(|a| serde_json::from_str(&a).ok()),
        join: join.and_then(|j| serde_json::from_str(&j).ok()),
        ended_at,
    })
}

impl Stored {
    /// The card, as everyone in the chat sees it: never where to join.
    pub(crate) fn view(&self) -> GameInvite {
        GameInvite {
            name: self.game.clone(),
            steam_app_id: self.steam_app_id,
            art: self.art.clone(),
            joinable: self.join.is_some() && self.ended_at.is_none(),
            ended: self.ended_at.is_some(),
        }
    }

    /// Whether `p` is still the game this invite is for.
    fn same_game(&self, p: Option<&Playing>) -> bool {
        p.is_some_and(|p| {
            p.name == self.game
                && (self.steam_app_id.is_none()
                    || p.steam_app_id.is_none()
                    || p.steam_app_id == self.steam_app_id)
        })
    }

    /// Whether `p` is the very game this invite is for, after a restart: the same Steam app id when the invite has
    /// one, else the same name.
    fn same_game_again(&self, p: &Playing) -> bool {
        match self.steam_app_id {
            Some(app) => p.steam_app_id == Some(app),
            None => p.name == self.game,
        }
    }
}

/// Whether `st`'s sender is still in its chat.
async fn sender_in(db: &SqlitePool, st: &Stored) -> AppResult<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM channel_members WHERE channel_id = ? AND user_id = ?",
    )
    .bind(st.channel_id.to_string())
    .bind(st.sender_id.to_string())
    .fetch_optional(db)
    .await?
    .is_some())
}

/// A live (not deleted) message's invite.
async fn load(db: &mut SqliteConnection, id: MessageId) -> AppResult<Option<Stored>> {
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} WHERE g.message_id = ? AND m.deleted_at IS NULL"
    )))
    .bind(id.to_string())
    .fetch_optional(&mut *db)
    .await?;
    row.map(stored_from).transpose()
}

/// `sender`'s invites that haven't ended (on live messages, in chats they're still in).
async fn live_of(db: &SqlitePool, sender: UserId) -> AppResult<Vec<Stored>> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} WHERE g.sender_id = ? AND g.ended_at IS NULL AND m.deleted_at IS NULL AND {SENDER_IN}"
    )))
    .bind(sender.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter().map(stored_from).collect()
}

/// Everyone with an invite that hasn't ended, with the SteamID they're linked to: the Join round reads them too.
pub async fn live_senders(db: &SqlitePool) -> sqlx::Result<Vec<(String, String)>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT DISTINCT g.sender_id, l.steam_id FROM game_invites g
         JOIN steam_links l ON l.user_id = g.sender_id
         JOIN messages m ON m.id = g.message_id
         WHERE g.ended_at IS NULL AND m.deleted_at IS NULL AND {SENDER_IN}"
    )))
    .fetch_all(db)
    .await
}

/// Set each live invite's `game_invite` (and clear any other).
pub(crate) async fn attach(db: &SqlitePool, page: &mut [Message]) -> AppResult<()> {
    let ids: Vec<String> = page
        .iter()
        .filter(|m| !m.deleted && m.kind == MessageKind::Normal)
        .map(|m| m.id.to_string())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let mut q = sqlx::QueryBuilder::new(format!("SELECT {COLS} WHERE g.message_id IN ("));
    let mut list = q.separated(", ");
    for id in &ids {
        list.push_bind(id);
    }
    q.push(")");
    let rows: Vec<Row> = q.build_query_as().fetch_all(db).await?;
    if rows.is_empty() {
        return Ok(());
    }
    let mut by_id: std::collections::HashMap<MessageId, GameInvite> = rows
        .into_iter()
        .map(|r| stored_from(r).map(|s| (s.message_id, s.view())))
        .collect::<AppResult<_>>()?;
    for m in page.iter_mut() {
        if let Some(v) = by_id.remove(&m.id) {
            m.game_invite = Some(Box::new(v));
        }
    }
    Ok(())
}

/// A message is being deleted (in `tx`): its invite goes with it, and with it where to join.
pub(crate) async fn on_delete(tx: &mut SqliteConnection, id: MessageId) -> AppResult<()> {
    sqlx::query("DELETE FROM game_invites WHERE message_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// At boot nobody is playing anything (it lives in memory): every invite still open ends, for [`RESTART_GRACE`] only
/// ([`reopen_after_restart`]).
pub async fn end_all_on_startup(db: &SqlitePool) -> sqlx::Result<u64> {
    let grace = crate::db::stamp(chrono::Utc::now() + RESTART_GRACE);
    Ok(sqlx::query(
        "UPDATE game_invites SET ended_at = ?, join_info = NULL, restart_grace_until = ? WHERE ended_at IS NULL",
    )
    .bind(now())
    .bind(grace)
    .execute(db)
    .await?
    .rows_affected())
}

/// `user` plays `p`: each of their invites a restart ended less than [`RESTART_GRACE`] ago, for that same game,
/// opens again (and reads where to join at once, as when sent).
async fn reopen_after_restart(s: &AppState, user: UserId, p: &Playing) -> AppResult<()> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} WHERE g.sender_id = ? AND g.ended_at IS NOT NULL AND g.restart_grace_until > ?
           AND m.deleted_at IS NULL AND {SENDER_IN}"
    )))
    .bind(user.to_string())
    .bind(now())
    .fetch_all(&s.db)
    .await?;
    let mut join = None;
    for st in rows.into_iter().map(stored_from) {
        let st = st?;
        if !st.same_game_again(p) {
            continue;
        }
        let reopened = sqlx::query(
            "UPDATE game_invites SET ended_at = NULL, restart_grace_until = NULL
             WHERE message_id = ? AND ended_at IS NOT NULL AND restart_grace_until > ?",
        )
        .bind(st.message_id.to_string())
        .bind(now())
        .execute(&s.db)
        .await?
        .rows_affected()
            > 0;
        if !reopened {
            continue;
        }
        announce(s, st.channel_id, st.message_id).await;
        if join.is_none() {
            join = Some(read_join(s, user, p).await);
        }
        if let Some(Some(j)) = &join {
            let open = Stored {
                ended_at: None,
                join: None,
                ..st
            };
            set_join(s, &open, j).await?;
        }
    }
    Ok(())
}

/// The text of an invite to play `game`, for clients that don't know invites (and for search, previews and forwards).
pub fn fallback_text(game: &str) -> String {
    format!("🎮 Invite to play {game}")
}

/// Announce an invite as it is now (after a commit).
async fn announce(s: &AppState, channel_id: ChannelId, message_id: MessageId) {
    let stored = match s.db.acquire().await {
        Ok(mut conn) => load(&mut conn, message_id).await,
        Err(e) => Err(e.into()),
    };
    match stored {
        Ok(Some(st)) => {
            s.hub
                .publish(
                    &s.db,
                    Event::GameInviteChanged {
                        channel_id,
                        message_id,
                        invite: Box::new(st.view()),
                    },
                )
                .await;
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, "couldn't read a game invite to announce it"),
    }
}

/// End invite `id` (if it hasn't ended) and tell its chat.
async fn end(s: &AppState, st: &Stored) -> AppResult<()> {
    let ended = sqlx::query(
        "UPDATE game_invites SET ended_at = ?, join_info = NULL WHERE message_id = ? AND ended_at IS NULL",
    )
    .bind(now())
    .bind(st.message_id.to_string())
    .execute(&s.db)
    .await?
    .rows_affected()
        > 0;
    if ended {
        announce(s, st.channel_id, st.message_id).await;
    }
    Ok(())
}

/// `user` left group `channel` or was removed from it (committed): each of their invites there ends, live, and one a
/// restart ended can't open again.
pub async fn end_for_member(s: &AppState, channel: ChannelId, user: UserId) {
    let r: Result<Vec<String>, sqlx::Error> = async {
        sqlx::query(
            "UPDATE game_invites SET restart_grace_until = NULL
             WHERE channel_id = ? AND sender_id = ? AND restart_grace_until IS NOT NULL",
        )
        .bind(channel.to_string())
        .bind(user.to_string())
        .execute(&s.db)
        .await?;
        sqlx::query_scalar(
            "UPDATE game_invites SET ended_at = ?, join_info = NULL
             WHERE channel_id = ? AND sender_id = ? AND ended_at IS NULL RETURNING message_id",
        )
        .bind(now())
        .bind(channel.to_string())
        .bind(user.to_string())
        .fetch_all(&s.db)
        .await
    }
    .await;
    match r {
        Ok(ended) => {
            for id in ended {
                if let Ok(id) = id.parse() {
                    announce(s, channel, id).await;
                }
            }
        }
        Err(e) => tracing::warn!(error = %e, "game invites not ended for a member who left"),
    }
}

/// `user`'s game changed (or they went): each of their invites for another game than the one they play now ends, and
/// one a restart ended for the game they play now opens again ([`reopen_after_restart`]).
pub async fn follow(s: &AppState, user: UserId) {
    if let Some(p) = s.hub.playing_of(user)
        && let Err(e) = reopen_after_restart(s, user, &p).await
    {
        tracing::warn!(error = %e, "game invites not reopened");
    }
    let live = match live_of(&s.db, user).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "game invites not read");
            return;
        }
    };
    if live.is_empty() {
        return;
    }
    let playing = s.hub.playing_of(user);
    for st in live {
        if !st.same_game(playing.as_ref())
            && let Err(e) = end(s, &st).await
        {
            tracing::warn!(error = %e, "game invite not ended");
        }
    }
}

/// A Join round read `user` from Steam: `found` is where they can be joined now (`None`: nowhere). Each of their live
/// invites for the game they play that offers nothing yet takes it; one that offered a lobby or server that's gone, or
/// that Steam now shows them somewhere else than, ends (they left it: the invite was to that one).
pub async fn refresh(s: &AppState, user: UserId, found: Option<SteamJoin>) {
    let live = match live_of(&s.db, user).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "game invites not read");
            return;
        }
    };
    let playing = s.hub.playing_of(user);
    for st in live {
        if !st.same_game(playing.as_ref()) {
            continue; // `follow` ends it
        }
        // once it offers a lobby or server, it offers that one only: gone, or another one, ends it
        let r = match (&st.join, &found) {
            (Some(was), Some(j)) if was != j => end(s, &st).await,
            (Some(_), None) => end(s, &st).await,
            (None, Some(j)) => set_join(s, &st, j).await.map(|_| ()),
            _ => Ok(()),
        };
        if let Err(e) = r {
            tracing::warn!(error = %e, "game invite not refreshed");
        }
    }
}

/// Store `j` as where invite `st` can be joined (while it hasn't ended); tell its chat when it just became joinable.
async fn set_join(s: &AppState, st: &Stored, j: &SteamJoin) -> AppResult<bool> {
    let json = serde_json::to_string(j).map_err(anyhow::Error::from)?;
    let set = sqlx::query(
        "UPDATE game_invites SET join_info = ? WHERE message_id = ? AND ended_at IS NULL",
    )
    .bind(json)
    .bind(st.message_id.to_string())
    .execute(&s.db)
    .await?
    .rows_affected()
        > 0;
    let became = set && st.join.is_none();
    if became {
        announce(s, st.channel_id, st.message_id).await;
    }
    Ok(became)
}

/// Where `user`, playing `p`, can be joined now: the Join their "Playing" already carries (they share Join), else read
/// from Steam once for this invite (their link, whatever their Join switch says). `None` without a link, a key, or a
/// lobby or server Steam shows for that game.
async fn read_join(s: &AppState, user: UserId, p: &Playing) -> Option<SteamJoin> {
    if let Some(j) = p.join.clone() {
        return Some(j);
    }
    let link = crate::steam::store::link_of(&s.db, user).await.ok()??;
    if !valid_steam_id(&link.steam_id) {
        return None;
    }
    let app = s.steam.app_of(user, Some(p));
    match s.steam.api().in_game(&[link.steam_id.as_str()]).await {
        Ok(list) => list
            .iter()
            .find_map(|g| crate::steam::join::join_of(g, &link.steam_id, app)),
        Err(e) => {
            tracing::debug!(error = ?e, "Steam summaries unavailable for a game invite");
            None
        }
    }
}

/// Keep the art's files from the media sweep, as achievement cards' are: a message shows them for good.
async fn keep_art(s: &AppState, art: &GameArt) {
    for t in [&art.hero, &art.header, &art.logo].into_iter().flatten() {
        if let Err(e) = sqlx::query("INSERT OR IGNORE INTO steam_card_media (hash) VALUES (?)")
            .bind(&t.hash)
            .execute(&s.db)
            .await
        {
            tracing::warn!(error = %e, "game invite art not kept");
        }
    }
}

/// `POST /channels/{id}/game-invite`: invite the DM or group to the game you're playing now. Returns the message with
/// its card; the chat gets it as a `MessageCreated`. 409 when you aren't playing anything.
async fn create(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
) -> AppResult<(StatusCode, Json<Message>)> {
    if !s.limits.send_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    crate::media::writes_ok(&s.cfg)?;
    let ch = channel_for(&s.db, me, id).await?;
    if !ch.kind.is_private() {
        return Err(AppError::BadRequest(
            "game invites go in DMs and groups".into(),
        ));
    }
    crate::access::require_open(&ch)?;
    let Some(playing) = s.hub.playing_of(me) else {
        return Err(AppError::Conflict("You're not in a game right now.".into()));
    };
    let join = read_join(&s, me, &playing).await;
    if let Some(art) = &playing.art {
        keep_art(&s, art).await;
    }
    let content = fallback_text(&playing.name);
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    crate::access::channel_live_in(&mut tx, id).await?; // channel deleted since (#109)
    crate::access::open_in(&mut tx, id).await?; // archived (#303): read-only
    // minted under the write lock, so ids keep commit order (as a send)
    let msg_id = MessageId::new();
    let at = now();
    sqlx::query("INSERT INTO messages (id, channel_id, author_id, kind, content, created_at) VALUES (?, ?, ?, 'normal', ?, ?)")
        .bind(msg_id.to_string())
        .bind(id.to_string())
        .bind(me.to_string())
        .bind(&content)
        .bind(&at)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO game_invites (message_id, channel_id, sender_id, game, steam_app_id, art, join_info, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(msg_id.to_string())
    .bind(id.to_string())
    .bind(me.to_string())
    .bind(&playing.name)
    .bind(playing.steam_app_id.map(i64::from))
    .bind(
        playing
            .art
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(anyhow::Error::from)?,
    )
    .bind(
        join.as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(anyhow::Error::from)?,
    )
    .bind(&at)
    .execute(&mut *tx)
    .await?;
    // A new message reopens closed conversations.
    sqlx::query("DELETE FROM dm_hidden WHERE channel_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    let moved = crate::reads::advance(&mut tx, me, id, msg_id).await?;
    tx.commit().await?;
    let msg = crate::messages::routes::load(&s.db, Some(me), msg_id)
        .await?
        .ok_or(AppError::NotFound)?;
    s.hub
        .publish(
            &s.db,
            Event::MessageCreated {
                message: msg.clone(),
                nonce: None,
            },
        )
        .await;
    if moved
        && let Some((unread, mentions)) = crate::reads::counts_after_commit(&s.db, me, id).await
    {
        s.hub
            .publish(
                &s.db,
                Event::ReadStateUpdated {
                    user_id: me,
                    channel_id: id,
                    last_read_message_id: Some(msg_id),
                    mentions: Some(mentions),
                    unread: Some(unread),
                },
            )
            .await;
    }
    drop(_order);
    // they may have stopped playing while it was sent
    follow(&s, me).await;
    Ok((StatusCode::CREATED, Json(msg)))
}

/// `POST /messages/{id}/game-invite/join`: where to join the invite's game, for a member of its chat (404 for anyone
/// else, as if there were none). 409 once it ended or while there's nothing to join.
async fn join(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<Json<SteamJoin>> {
    let st = {
        let mut conn = s.db.acquire().await?;
        load(&mut conn, id).await?.ok_or(AppError::NotFound)?
    };
    channel_for(&s.db, me, st.channel_id).await?;
    if st.ended_at.is_some() || !sender_in(&s.db, &st).await? {
        return Err(AppError::Conflict("No longer in this game.".into()));
    }
    st.join
        .map(Json)
        .ok_or_else(|| AppError::Conflict("There's nothing to join right now.".into()))
}
