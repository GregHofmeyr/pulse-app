//! Server invites (#80, step 1): links and direct invites into a server.
//!
//! Any member makes invites (`POST /servers/{id}/invites`); members within [`server_invites::MEMBER_MAX_AGE_SECS`] and
//! [`server_invites::MEMBER_MAX_USES`], the owner with any expiry or use limit, "never" and "no limit" included. A
//! direct invite (`for_user`) is for that one person, single use, and also arrives as a card in its maker's DM with
//! them, mentioning them so it reaches their inbox. Its maker or the owner revokes one (`DELETE /invites/{code}`).
//!
//! A signed-in person previews a code (`GET /invites/{code}`): what its card shows, and whether it would let them in.
//! An unknown and a revoked code answer the same 404, so a guessed code is never confirmed. Accepting
//! (`POST /invites/{code}/accept`) checks everything again inside one write transaction, puts them in the server the
//! way Join does (`routes::add_member_in`) and counts the use there, so two people racing a single-use invite give
//! exactly one member. Code lookups are rate limited per address and per account, every lookup of a code that doesn't
//! work is logged (`invite_failures`), and too many unknown codes block lookups from that address or account for a
//! while ([`lookups_blocked`]).
//!
//! Servers stay open in this step: Join still works, and nothing changes for anyone until an invite is used. Invites
//! that stopped working are kept a while so old cards can still say why, then swept ([`prune`], with the failure log).

use std::collections::BTreeSet;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, TimeDelta, Utc};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{
    ChannelKind, CreateServerInviteRequest, INVITE_JOIN_TEXT_CHARS, InvitePreview, InviteState,
    Message, MessageServerInvite, Server, ServerInvite,
};
use sqlx::{SqliteConnection, SqlitePool};

use crate::AppState;
use crate::auth::AuthUser;
use crate::db::{begin_write, now, stamp};
use crate::error::{AppError, AppResult};
use crate::limits::{ClientIp, caps, rate_key, server_invites as rules};
use crate::servers::routes::{require_member, server_exists, server_live_in};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/servers/{id}/invites", get(list).post(create))
        .route("/invites/{code}", get(preview).delete(revoke))
        .route("/invites/{code}/accept", post(accept))
}

/// What a code is made of: no 0/O, 1/l/I, so a code read aloud or retyped can't be mistaken.
const ALPHABET: &[u8] = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// Characters in a code.
pub const CODE_LEN: usize = 10;
/// How much of a looked-up code the failure log keeps (it is whatever was typed).
const LOGGED_CODE_CHARS: usize = 64;
/// How much of a user agent the failure log keeps.
const LOGGED_AGENT_CHARS: usize = 256;

/// A fresh random code, from the OS-seeded CSPRNG.
pub fn new_code() -> String {
    // ThreadRng is !Send: keep it out of the async state machine.
    let mut rng = rand::rng();
    (0..CODE_LEN)
        .map(|_| ALPHABET[rand::Rng::random_range(&mut rng, 0..ALPHABET.len())] as char)
        .collect()
}

/// Whether `code` could be one of ours at all.
fn well_formed(code: &str) -> bool {
    code.len() == CODE_LEN && code.bytes().all(|b| ALPHABET.contains(&b))
}

#[derive(sqlx::FromRow, Clone, Debug)]
struct Row {
    code: String,
    server_id: String,
    created_by: String,
    for_user: Option<String>,
    join_text: Option<String>,
    created_at: String,
    expires_at: Option<String>,
    max_uses: Option<i64>,
    uses: i64,
    revoked_at: Option<String>,
}

const COLS: &str = "code, server_id, created_by, for_user, join_text, created_at, expires_at, max_uses, uses, revoked_at";

/// An invite that still lets someone in, as a `WHERE` on `server_invites` binding the time now.
const LIVE: &str = "revoked_at IS NULL AND (expires_at IS NULL OR expires_at > ?) \
     AND (max_uses IS NULL OR uses < max_uses)";

impl Row {
    /// Valid, or why not, at `now` (stored-time format).
    fn state(&self, now: &str) -> InviteState {
        if self.revoked_at.is_some() {
            InviteState::Revoked
        } else if self.expires_at.as_deref().is_some_and(|e| e <= now) {
            InviteState::Expired
        } else if self.max_uses.is_some_and(|m| self.uses >= m) {
            InviteState::UsedUp
        } else {
            InviteState::Valid
        }
    }

    fn server(&self) -> AppResult<ServerId> {
        Ok(self.server_id.parse().map_err(anyhow::Error::from)?)
    }

    fn maker(&self) -> AppResult<UserId> {
        Ok(self.created_by.parse().map_err(anyhow::Error::from)?)
    }

    fn for_user(&self) -> AppResult<Option<UserId>> {
        Ok(self
            .for_user
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(anyhow::Error::from)?)
    }

    fn view(&self, now: &str) -> AppResult<ServerInvite> {
        Ok(ServerInvite {
            code: self.code.clone(),
            server_id: self.server()?,
            created_by: self.maker()?,
            for_user: self.for_user()?,
            join_text: self.join_text.clone(),
            created_at: self.created_at.clone(),
            expires_at: self.expires_at.clone(),
            max_uses: self.max_uses,
            uses: self.uses,
            revoked_at: self.revoked_at.clone(),
            state: self.state(now),
        })
    }
}

async fn load<'e>(db: impl sqlx::SqliteExecutor<'e>, code: &str) -> AppResult<Option<Row>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM server_invites WHERE code = ?"
    )))
    .bind(code)
    .fetch_optional(db)
    .await?)
}

async fn owner_of<'e>(db: impl sqlx::SqliteExecutor<'e>, server: ServerId) -> AppResult<UserId> {
    let owner: String = sqlx::query_scalar("SELECT created_by FROM servers WHERE id = ?")
        .bind(server.to_string())
        .fetch_one(db)
        .await?;
    Ok(owner.parse().map_err(anyhow::Error::from)?)
}

async fn member_in(tx: &mut SqliteConnection, server: ServerId, user: UserId) -> AppResult<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM server_members WHERE server_id = ? AND user_id = ?",
    )
    .bind(server.to_string())
    .bind(user.to_string())
    .fetch_optional(&mut *tx)
    .await?
    .is_some())
}

/// Whether `user` is banned from `server` (#80).
pub(crate) async fn banned<'e>(
    db: impl sqlx::SqliteExecutor<'e>,
    server: ServerId,
    user: UserId,
) -> AppResult<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM server_bans WHERE server_id = ? AND user_id = ?",
    )
    .bind(server.to_string())
    .bind(user.to_string())
    .fetch_optional(db)
    .await?
    .is_some())
}

/// Tell the people whose Invites list for `server` changed: each of `makers`, and its owner (#80).
pub(crate) async fn invites_changed(
    s: &AppState,
    server: ServerId,
    makers: impl IntoIterator<Item = UserId>,
) {
    let mut people: BTreeSet<UserId> = makers.into_iter().collect();
    match owner_of(&s.db, server).await {
        Ok(o) => {
            people.insert(o);
        }
        Err(e) => {
            tracing::warn!(error = %e, server = %server, "no owner to tell of an invite change")
        }
    }
    let events = people
        .into_iter()
        .map(|user_id| Event::InvitesChanged {
            user_id,
            server_id: server,
        })
        .collect();
    s.hub.publish_all(&s.db, events).await;
}

/// A custom join button text: trimmed, one line, at most [`INVITE_JOIN_TEXT_CHARS`]; blank is none.
fn clean_join_text(raw: Option<&str>) -> AppResult<Option<String>> {
    let Some(t) = raw.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    if t.chars().any(char::is_control) {
        return Err(AppError::BadRequest(
            "the join button text must be one line".into(),
        ));
    }
    if t.chars().count() > INVITE_JOIN_TEXT_CHARS {
        return Err(AppError::BadRequest(format!(
            "the join button text can be up to {INVITE_JOIN_TEXT_CHARS} characters"
        )));
    }
    Ok(Some(t.to_string()))
}

/// The expiry and use limit a request asks for, checked against what `owner` (or a member) may make: `(expires_at,
/// max_uses)`, `None` meaning never and no limit.
fn limits_for(
    req: &CreateServerInviteRequest,
    owner: bool,
    direct: bool,
    now: DateTime<Utc>,
) -> AppResult<(Option<String>, Option<i64>)> {
    let age = req.max_age_secs.unwrap_or(rules::DEFAULT_MAX_AGE_SECS);
    if !(0..=rules::MAX_AGE_SECS).contains(&age) {
        return Err(AppError::BadRequest("that expiry is out of range".into()));
    }
    if !owner && (age == 0 || age > rules::MEMBER_MAX_AGE_SECS) {
        return Err(AppError::ForbiddenMsg(format!(
            "Only the owner can make an invite that lasts longer than {} days.",
            rules::MEMBER_MAX_AGE_SECS / 86_400
        )));
    }
    let expires_at = (age > 0).then(|| stamp(now + TimeDelta::seconds(age)));
    if direct {
        // only that person, once
        return Ok((expires_at, Some(1)));
    }
    let uses = req.max_uses.unwrap_or(rules::DEFAULT_MAX_USES);
    if uses < 0 {
        return Err(AppError::BadRequest(
            "that use limit is out of range".into(),
        ));
    }
    if !owner && (uses == 0 || uses > rules::MEMBER_MAX_USES) {
        return Err(AppError::ForbiddenMsg(format!(
            "Only the owner can make an invite for more than {} people.",
            rules::MEMBER_MAX_USES
        )));
    }
    Ok((expires_at, (uses > 0).then_some(uses)))
}

/// `POST /servers/{id}/invites`: make an invite link, or a direct invite (see the module docs). 403 for a non-member
/// or a member asking for more than members may make; 429 past the rate or the day's direct invites; 400 past the
/// server's live invites. A direct invite: 400 for yourself or someone unknown, 409 when they're already in, 403 when
/// they're banned.
async fn create(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
    Json(req): Json<CreateServerInviteRequest>,
) -> AppResult<Json<ServerInvite>> {
    if !s.limits.server_invite_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    require_member(&s.db, id, me).await?;
    let join_text = clean_join_text(req.join_text.as_deref())?;
    let direct = req.for_user;
    if let Some(other) = direct {
        if other == me {
            return Err(AppError::BadRequest("you can't invite yourself".into()));
        }
        let known: Option<i64> = sqlx::query_scalar("SELECT 1 FROM users WHERE id = ?")
            .bind(other.to_string())
            .fetch_optional(&s.db)
            .await?;
        if known.is_none() {
            return Err(AppError::BadRequest("unknown user".into()));
        }
    }
    let at = Utc::now();
    let created_at = stamp(at);
    // A direct invite's card goes out in order with the DM's other messages.
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    server_live_in(&mut tx, id).await?;
    // again under the write lock: a leave, removal or hand-over may have landed since
    if !member_in(&mut tx, id, me).await? {
        return Err(AppError::Forbidden);
    }
    let owner = owner_of(&mut *tx, id).await? == me;
    let (expires_at, max_uses) = limits_for(&req, owner, direct.is_some(), at)?;
    let live: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT COUNT(*) FROM server_invites WHERE server_id = ? AND {LIVE}"
    )))
    .bind(id.to_string())
    .bind(&created_at)
    .fetch_one(&mut *tx)
    .await?;
    if live >= caps::LIVE_INVITES_PER_SERVER {
        return Err(AppError::BadRequest(format!(
            "a server can have up to {} invites at once: revoke some first",
            caps::LIVE_INVITES_PER_SERVER
        )));
    }
    if let Some(other) = direct {
        if member_in(&mut tx, id, other).await? {
            return Err(AppError::ConflictCode(
                "already_member",
                "They're already in this server.".into(),
            ));
        }
        if banned(&mut *tx, id, other).await? {
            return Err(AppError::ForbiddenMsg(
                "They're banned from this server.".into(),
            ));
        }
        let today: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM server_invites WHERE created_by = ? AND for_user IS NOT NULL AND created_at > ?",
        )
        .bind(me.to_string())
        .bind(stamp(at - TimeDelta::days(1)))
        .fetch_one(&mut *tx)
        .await?;
        if today >= rules::DIRECT_PER_DAY {
            return Err(AppError::TooManyRequests(Some(
                "You've sent a lot of invites today. Try again tomorrow.".into(),
            )));
        }
    }
    let code = new_code();
    sqlx::query(
        "INSERT INTO server_invites (code, server_id, created_by, for_user, join_text, created_at, expires_at, max_uses, signup_ok)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&code)
    .bind(id.to_string())
    .bind(me.to_string())
    .bind(direct.map(|u| u.to_string()))
    .bind(&join_text)
    .bind(&created_at)
    .bind(&expires_at)
    .bind(max_uses)
    // the owner's links may double as sign-up codes (step 3); members' never do
    .bind(owner && direct.is_none())
    .execute(&mut *tx)
    .await?;
    let card = match direct {
        Some(other) => Some((
            other,
            post_card_in(&mut tx, &s, id, me, other, &code).await?,
        )),
        None => None,
    };
    tx.commit().await?;
    tracing::info!(server = %id, by = %me, direct = direct.is_some(), "server invite made");
    if let Some((other, card)) = card {
        publish_card(&s, me, other, card).await;
    }
    drop(_order);
    invites_changed(&s, id, [me]).await;
    let row = load(&s.db, &code).await?.ok_or(AppError::NotFound)?;
    Ok(Json(row.view(&now())?))
}

/// A direct invite's card, stored in the transaction; published by [`publish_card`] after the commit.
struct Card {
    channel: ChannelId,
    message: MessageId,
    /// The DM was made for it (nobody had one yet).
    new_dm: bool,
    moved: bool,
}

/// Post the card for direct invite `code` in `me`'s DM with `other` (made now if they have none), mentioning them so
/// it reaches their inbox, as an invite to play posts its card (#436). The card's text is the invite for clients that
/// don't know invite cards.
async fn post_card_in(
    tx: &mut SqliteConnection,
    s: &AppState,
    server: ServerId,
    me: UserId,
    other: UserId,
    code: &str,
) -> AppResult<Card> {
    let name: String = sqlx::query_scalar("SELECT name FROM servers WHERE id = ?")
        .bind(server.to_string())
        .fetch_one(&mut *tx)
        .await?;
    let (channel, new_dm) = match crate::servers::routes::existing_dm(&mut *tx, me, other).await? {
        Some(id) => (id.parse().map_err(anyhow::Error::from)?, false),
        None => {
            let ch =
                crate::servers::routes::insert_channel(tx, None, ChannelKind::Dm, None, 0, None)
                    .await?;
            let at = now();
            for u in [me, other] {
                sqlx::query(
                    "INSERT INTO channel_members (channel_id, user_id, added_by, added_at) VALUES (?, ?, ?, ?)",
                )
                .bind(ch.id.to_string())
                .bind(u.to_string())
                .bind(me.to_string())
                .bind(&at)
                .execute(&mut *tx)
                .await?;
                crate::reads::init_point(tx, u, ch.id).await?;
            }
            (ch.id, true)
        }
    };
    let content = match &s.cfg.embeds.public_url {
        Some(base) => format!(
            "📨 Invite to join {name}: {}/i/{code}",
            base.as_str().trim_end_matches('/')
        ),
        None => format!("📨 Invite to join {name} (invite code {code})"),
    };
    // minted under the write lock, so ids keep commit order (as a send)
    let message = MessageId::new();
    sqlx::query("INSERT INTO messages (id, channel_id, author_id, kind, content, created_at) VALUES (?, ?, ?, 'normal', ?, ?)")
        .bind(message.to_string())
        .bind(channel.to_string())
        .bind(me.to_string())
        .bind(&content)
        .bind(now())
        .execute(&mut *tx)
        .await?;
    crate::messages::mentions::store(tx, message, &[other], true).await?;
    sqlx::query("UPDATE server_invites SET message_id = ? WHERE code = ?")
        .bind(message.to_string())
        .bind(code)
        .execute(&mut *tx)
        .await?;
    // A new message reopens closed conversations.
    sqlx::query("DELETE FROM dm_hidden WHERE channel_id = ?")
        .bind(channel.to_string())
        .execute(&mut *tx)
        .await?;
    let moved = crate::reads::advance(tx, me, channel, message).await?;
    Ok(Card {
        channel,
        message,
        new_dm,
        moved,
    })
}

/// After the commit (under `in_order`): the new DM, if one was made, then the card and the sender's read point.
async fn publish_card(s: &AppState, me: UserId, other: UserId, card: Card) {
    if card.new_dm {
        match crate::access::load_channel(&s.db, card.channel).await {
            Ok(Some(channel)) => {
                s.hub
                    .publish(&s.db, Event::ChannelCreated { channel })
                    .await;
                s.hub
                    .publish(
                        &s.db,
                        Event::GroupMembersChanged {
                            channel_id: card.channel,
                            user_ids: vec![me, other],
                        },
                    )
                    .await;
                // they share a conversation now: presence both ways
                s.hub.resync(&s.db, [me, other]).await;
            }
            Ok(None) => return,
            Err(e) => tracing::error!(error = ?e, "couldn't read back an invite's new DM"),
        }
    }
    match crate::messages::routes::load(&s.db, Some(me), card.message).await {
        Ok(Some(message)) => {
            s.hub
                .publish(
                    &s.db,
                    Event::MessageCreated {
                        message,
                        nonce: None,
                    },
                )
                .await;
        }
        Ok(None) => {}
        Err(e) => tracing::error!(error = ?e, "couldn't read back an invite card"),
    }
    if card.moved
        && let Some((unread, mentions)) =
            crate::reads::counts_after_commit(&s.db, me, card.channel).await
    {
        s.hub
            .publish(
                &s.db,
                Event::ReadStateUpdated {
                    user_id: me,
                    channel_id: card.channel,
                    last_read_message_id: Some(card.message),
                    mentions: Some(mentions),
                    unread: Some(unread),
                },
            )
            .await;
    }
}

/// Set each invite card's `server_invite` (and clear any other), for a page of messages.
pub(crate) async fn attach(db: &SqlitePool, page: &mut [Message]) -> AppResult<()> {
    let ids: Vec<String> = page
        .iter()
        .filter(|m| !m.deleted)
        .map(|m| m.id.to_string())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let mut q = sqlx::QueryBuilder::new(
        "SELECT message_id, code, server_id FROM server_invites WHERE message_id IN (",
    );
    let mut list = q.separated(", ");
    for id in &ids {
        list.push_bind(id);
    }
    q.push(")");
    let rows: Vec<(String, String, String)> = q.build_query_as().fetch_all(db).await?;
    if rows.is_empty() {
        return Ok(());
    }
    let mut by_id = std::collections::HashMap::new();
    for (message, code, server) in rows {
        let message: MessageId = message.parse().map_err(anyhow::Error::from)?;
        let server_id: ServerId = server.parse().map_err(anyhow::Error::from)?;
        by_id.insert(message, MessageServerInvite { code, server_id });
    }
    for m in page.iter_mut() {
        if let Some(card) = by_id.remove(&m.id) {
            m.server_invite = Some(card);
        }
    }
    Ok(())
}

/// `GET /servers/{id}/invites`: the server's invites, newest first, including ones that stopped working (until they're
/// swept). The owner sees all of them, a member their own; 403 for anyone else.
async fn list(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
) -> AppResult<Json<Vec<ServerInvite>>> {
    require_member(&s.db, id, me).await?;
    let owner = owner_of(&s.db, id).await? == me;
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM server_invites WHERE server_id = ?1 AND (?2 OR created_by = ?3)
         ORDER BY created_at DESC, code"
    )))
    .bind(id.to_string())
    .bind(owner)
    .bind(me.to_string())
    .fetch_all(&s.db)
    .await?;
    let at = now();
    rows.iter()
        .map(|r| r.view(&at))
        .collect::<AppResult<Vec<_>>>()
        .map(Json)
}

/// `DELETE /invites/{code}`: revoke an invite; its maker or the server's owner. 404 for anyone else, as for a code that
/// doesn't exist. Revoking one already revoked changes nothing.
async fn revoke(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(code): Path<String>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    if !well_formed(&code) {
        return Err(AppError::NotFound);
    }
    let mut tx = begin_write(&s.db).await?;
    let row = load(&mut *tx, &code).await?.ok_or(AppError::NotFound)?;
    let server = row.server()?;
    let maker = row.maker()?;
    if maker != me && owner_of(&mut *tx, server).await? != me {
        return Err(AppError::NotFound);
    }
    let changed = sqlx::query(
        "UPDATE server_invites SET revoked_at = ? WHERE code = ? AND revoked_at IS NULL",
    )
    .bind(now())
    .bind(&code)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    tx.commit().await?;
    if changed {
        tracing::info!(server = %server, by = %me, "server invite revoked");
        invites_changed(&s, server, [maker]).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Why a looked-up code didn't work, as the failure log records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Unknown,
    Expired,
    Revoked,
    UsedUp,
}

impl Failure {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
            Self::UsedUp => "used_up",
        }
    }

    fn of(state: InviteState) -> Option<Self> {
        match state {
            InviteState::Expired => Some(Self::Expired),
            InviteState::Revoked => Some(Self::Revoked),
            InviteState::UsedUp => Some(Self::UsedUp),
            _ => None,
        }
    }
}

/// Who is looking a code up, for the limits and the failure log.
struct Seeker {
    user: UserId,
    /// The address as the rate limits count it.
    ip: String,
    agent: Option<String>,
}

impl Seeker {
    fn new(user: UserId, ip: std::net::IpAddr, headers: &HeaderMap) -> Self {
        Self {
            user,
            ip: rate_key(ip).to_string(),
            agent: headers
                .get(axum::http::header::USER_AGENT)
                .and_then(|v| v.to_str().ok())
                .map(|a| a.chars().take(LOGGED_AGENT_CHARS).collect()),
        }
    }
}

/// Note a lookup of `code` that didn't work. Never fails the request: a failure to log is logged.
async fn log_failure(db: &SqlitePool, who: &Seeker, code: &str, kind: Failure) {
    let code: String = code.chars().take(LOGGED_CODE_CHARS).collect();
    if let Err(e) = sqlx::query(
        "INSERT INTO invite_failures (at, ip, user_id, user_agent, code, kind) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(now())
    .bind(&who.ip)
    .bind(who.user.to_string())
    .bind(&who.agent)
    .bind(code)
    .bind(kind.as_str())
    .execute(db)
    .await
    {
        tracing::warn!(error = %e, "an invite lookup failure wasn't logged");
    }
}

/// Whether too many unknown codes, recently, from this address or this account block its lookups now: any
/// [`rules::UNKNOWN_CODES_TO_BLOCK`] of them within [`rules::UNKNOWN_WINDOW_SECS`] block for [`rules::BLOCK_SECS`] from
/// the last of them. Read from the failure log, so a restart forgets nothing.
pub async fn lookups_blocked(
    db: &SqlitePool,
    ip: &str,
    user: UserId,
    now: DateTime<Utc>,
) -> AppResult<bool> {
    let since = stamp(
        now - TimeDelta::seconds(rules::BLOCK_SECS)
            - TimeDelta::seconds(rules::UNKNOWN_WINDOW_SECS),
    );
    for (col, key) in [("ip", ip.to_string()), ("user_id", user.to_string())] {
        let times: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT at FROM invite_failures WHERE kind = 'unknown' AND {col} = ? AND at > ? ORDER BY at"
        )))
        .bind(&key)
        .bind(&since)
        .fetch_all(db)
        .await?;
        if blocks(&times, now) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// [`lookups_blocked`]'s rule over one source's unknown-code times (sorted).
fn blocks(times: &[String], now: DateTime<Utc>) -> bool {
    let n = rules::UNKNOWN_CODES_TO_BLOCK;
    let times: Vec<DateTime<Utc>> = times
        .iter()
        .filter_map(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))
        .collect();
    times.windows(n).any(|w| {
        let (first, last) = (w[0], w[n - 1]);
        last - first < TimeDelta::seconds(rules::UNKNOWN_WINDOW_SECS)
            && last + TimeDelta::seconds(rules::BLOCK_SECS) > now
    })
}

/// The guard in front of every code lookup: 429 while blocked, or past the lookups per address or per account.
async fn guard(s: &AppState, who: &Seeker) -> AppResult<()> {
    if lookups_blocked(&s.db, &who.ip, who.user, Utc::now()).await? {
        return Err(AppError::TooManyRequests(None));
    }
    if !s.limits.invite_lookup_ip.hit(&who.ip)
        || !s.limits.invite_lookup_user.hit(&who.user.to_string())
    {
        return Err(AppError::TooManyRequests(None));
    }
    Ok(())
}

/// The invite behind `code`, or 404 (logged): unknown and revoked look the same.
async fn found(s: &AppState, who: &Seeker, code: &str) -> AppResult<Row> {
    let row = if well_formed(code) {
        load(&s.db, code).await?
    } else {
        None
    };
    let Some(row) = row else {
        log_failure(&s.db, who, code, Failure::Unknown).await;
        return Err(AppError::NotFound);
    };
    if row.revoked_at.is_some() {
        log_failure(&s.db, who, code, Failure::Revoked).await;
        return Err(AppError::NotFound);
    }
    Ok(row)
}

/// `GET /invites/{code}`: what the invite's card shows, and whether it would let the viewer in (see
/// [`InvitePreview`]). 404 for an unknown or revoked code, or one whose server is in Recently deleted.
async fn preview(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    Path(code): Path<String>,
) -> AppResult<Json<InvitePreview>> {
    let who = Seeker::new(me, ip, &headers);
    guard(&s, &who).await?;
    let row = found(&s, &who, &code).await?;
    let server = row.server()?;
    server_exists(&s.db, server).await?;
    let base = row.state(&now());
    let for_user = row.for_user()?;
    let state = if crate::servers::routes::is_server_member(&s.db, server, me).await? {
        InviteState::AlreadyMember
    } else if banned(&s.db, server, me).await? {
        InviteState::Banned
    } else if for_user.is_some_and(|u| u != me) {
        InviteState::ForSomeoneElse
    } else {
        base
    };
    // a code that doesn't work for them (one they're already in through isn't a failed try)
    if let Some(f) = Failure::of(state) {
        log_failure(&s.db, &who, &code, f).await;
    }
    let (name, icon_hash): (String, Option<String>) =
        sqlx::query_as("SELECT name, icon_hash FROM servers WHERE id = ?")
            .bind(server.to_string())
            .fetch_one(&s.db)
            .await?;
    let member_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM server_members WHERE server_id = ?")
            .bind(server.to_string())
            .fetch_one(&s.db)
            .await?;
    let inviter = row.maker()?;
    Ok(Json(InvitePreview {
        code: row.code.clone(),
        server_id: server,
        name,
        icon_hash,
        banner_hash: None,
        member_count,
        online_count: online_count(&s, server, me).await?,
        faces: faces(&s.db, server, me).await?,
        inviter_id: inviter,
        inviter_name: crate::dms::routes::shown_name(&s.db, inviter).await?,
        join_text: row.join_text.clone(),
        for_user,
        for_user_name: match for_user {
            Some(u) => Some(crate::dms::routes::shown_name(&s.db, u).await?),
            None => None,
        },
        state,
    }))
}

/// Members of `server` who look online to `viewer`: connected, not Invisible, not appearing offline to them.
async fn online_count(s: &AppState, server: ServerId, viewer: UserId) -> AppResult<i64> {
    let visible: Vec<String> = sqlx::query_scalar(
        "SELECT m.user_id FROM server_members m JOIN users u ON u.id = m.user_id
         WHERE m.server_id = ?1 AND u.presence <> 'invisible'
           AND NOT EXISTS (SELECT 1 FROM presence_hidden h WHERE h.owner_id = m.user_id AND h.target_id = ?2)",
    )
    .bind(server.to_string())
    .bind(viewer.to_string())
    .fetch_all(&s.db)
    .await?;
    let online = s.hub.online_users();
    Ok(visible
        .iter()
        .filter_map(|u| u.parse::<UserId>().ok())
        .filter(|u| online.contains(u))
        .count() as i64)
}

/// How many faces a preview shows.
const FACES: i64 = 3;

/// Up to [`FACES`] members of `server` whom `viewer` already knows: they share another live server or a DM or group.
async fn faces(
    db: &SqlitePool,
    server: ServerId,
    viewer: UserId,
) -> AppResult<Vec<pulse_protocol::rest::User>> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT m.user_id FROM server_members m
         WHERE m.server_id = ?1 AND m.user_id <> ?2 AND (
           EXISTS (SELECT 1 FROM server_members a
                   JOIN server_members b ON b.server_id = a.server_id AND b.user_id = ?2
                   JOIN servers sv ON sv.id = a.server_id AND sv.deleted_at IS NULL
                   WHERE a.user_id = m.user_id AND a.server_id <> ?1)
           OR EXISTS (SELECT 1 FROM channel_members x
                      JOIN channel_members y ON y.channel_id = x.channel_id AND y.user_id = ?2
                      WHERE x.user_id = m.user_id))
         ORDER BY m.joined_at, m.user_id LIMIT ?3",
    )
    .bind(server.to_string())
    .bind(viewer.to_string())
    .bind(FACES)
    .fetch_all(db)
    .await?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        out.push(
            crate::auth::routes::load_user(db, id.parse().map_err(anyhow::Error::from)?).await?,
        );
    }
    Ok(out)
}

/// What an accept found, inside its transaction.
enum Outcome {
    Joined(crate::servers::routes::Joined),
    Refused(AppError, Option<Failure>),
}

/// `POST /invites/{code}/accept`: join the invite's server. Everything is checked again inside one write transaction
/// (see the module docs). 404 for an unknown or revoked code; 409 (`already_member`, `invite_expired`,
/// `invite_used_up`); 403 for someone else's direct invite or a banned person. Returns the server.
async fn accept(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    Path(code): Path<String>,
) -> AppResult<Json<Server>> {
    let who = Seeker::new(me, ip, &headers);
    guard(&s, &who).await?;
    // the early look: an unknown or revoked code is logged and refused before the write lock
    let early = found(&s, &who, &code).await?;
    let server = early.server()?;
    server_exists(&s.db, server).await?;
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    server_live_in(&mut tx, server).await?;
    let outcome = accept_in(&mut tx, &code, server, me).await?;
    let (joined, maker) = match outcome {
        Outcome::Joined(j) => {
            tx.commit().await?;
            (j, early.maker()?)
        }
        Outcome::Refused(e, failure) => {
            drop(tx); // rolls back
            if let Some(f) = failure {
                log_failure(&s.db, &who, &code, f).await;
            }
            return Err(e);
        }
    };
    tracing::info!(server = %server, user = %me, "joined by invite");
    crate::servers::routes::announce_joined(&s, server, me, joined).await;
    drop(_order);
    invites_changed(&s, server, [maker]).await;
    let row: crate::servers::routes::ServerRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {} FROM servers WHERE id = ?",
        crate::servers::routes::SERVER_COLS
    )))
    .bind(server.to_string())
    .fetch_one(&s.db)
    .await?;
    Ok(Json(crate::servers::routes::server_from_row(row)?))
}

/// [`accept`]'s checks and writes, under the write lock.
async fn accept_in(
    tx: &mut SqliteConnection,
    code: &str,
    server: ServerId,
    me: UserId,
) -> AppResult<Outcome> {
    let Some(row) = load(&mut *tx, code).await? else {
        // swept since the early look
        return Ok(Outcome::Refused(AppError::NotFound, Some(Failure::Unknown)));
    };
    let state = row.state(&now());
    if state == InviteState::Revoked {
        return Ok(Outcome::Refused(AppError::NotFound, Some(Failure::Revoked)));
    }
    if member_in(tx, server, me).await? {
        return Ok(Outcome::Refused(
            AppError::ConflictCode("already_member", "You're already in this server.".into()),
            None,
        ));
    }
    if banned(&mut *tx, server, me).await? {
        return Ok(Outcome::Refused(
            AppError::ForbiddenMsg("You can't join this server.".into()),
            None,
        ));
    }
    if row.for_user()?.is_some_and(|u| u != me) {
        return Ok(Outcome::Refused(
            AppError::ForbiddenMsg("This invite is for someone else.".into()),
            None,
        ));
    }
    match state {
        InviteState::Expired => {
            return Ok(Outcome::Refused(
                AppError::ConflictCode("invite_expired", "This invite has expired.".into()),
                Some(Failure::Expired),
            ));
        }
        InviteState::UsedUp => {
            return Ok(Outcome::Refused(
                AppError::ConflictCode("invite_used_up", "This invite has been used up.".into()),
                Some(Failure::UsedUp),
            ));
        }
        _ => {}
    }
    let joined = crate::servers::routes::add_member_in(tx, server, me).await?;
    sqlx::query("UPDATE server_invites SET uses = uses + 1 WHERE code = ?")
        .bind(code)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT OR IGNORE INTO server_invite_uses (code, user_id, used_at, created_account) VALUES (?, ?, ?, 0)",
    )
    .bind(code)
    .bind(me.to_string())
    .bind(now())
    .execute(&mut *tx)
    .await?;
    Ok(Outcome::Joined(joined))
}

/// Revoke `server`'s open direct invites to `user` (a ban), in the ban's transaction. Returns their makers.
pub(crate) async fn revoke_direct_in(
    tx: &mut SqliteConnection,
    server: ServerId,
    user: UserId,
) -> AppResult<Vec<UserId>> {
    let makers: Vec<String> = sqlx::query_scalar(
        "UPDATE server_invites SET revoked_at = ? WHERE server_id = ? AND for_user = ? AND revoked_at IS NULL
         RETURNING created_by",
    )
    .bind(now())
    .bind(server.to_string())
    .bind(user.to_string())
    .fetch_all(&mut *tx)
    .await?;
    makers
        .iter()
        .map(|m| Ok(m.parse().map_err(anyhow::Error::from)?))
        .collect()
}

/// How many rows one [`prune`] removed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pruned {
    pub invites: u64,
    pub failures: u64,
}

/// Delete invites [`rules::DEAD_RETENTION_DAYS`] after they stopped working (revoked, expired, or used up at their last
/// use), and failed lookups older than [`rules::FAILURE_RETENTION_DAYS`]. Run hourly with the other cleanup
/// (`hygiene::prune`).
pub async fn prune(db: &SqlitePool, now: DateTime<Utc>) -> AppResult<Pruned> {
    let dead = stamp(now - TimeDelta::days(rules::DEAD_RETENTION_DAYS));
    let invites = sqlx::query(
        "DELETE FROM server_invites WHERE revoked_at <= ?1 OR expires_at <= ?1
           OR (max_uses IS NOT NULL AND uses >= max_uses
               AND (SELECT MAX(u.used_at) FROM server_invite_uses u WHERE u.code = server_invites.code) <= ?1)",
    )
    .bind(&dead)
    .execute(db)
    .await?
    .rows_affected();
    let failures = sqlx::query("DELETE FROM invite_failures WHERE at <= ?")
        .bind(stamp(now - TimeDelta::days(rules::FAILURE_RETENTION_DAYS)))
        .execute(db)
        .await?
        .rows_affected();
    Ok(Pruned { invites, failures })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_ten_unambiguous_characters() {
        assert_eq!(ALPHABET.len(), 57);
        for look_alike in b"0O1lI" {
            assert!(!ALPHABET.contains(look_alike));
        }
        let a = new_code();
        assert_eq!(a.len(), CODE_LEN);
        assert!(well_formed(&a));
        assert_ne!(a, new_code());
        assert!(!well_formed("abc"));
        assert!(!well_formed("abcdefghi0"));
    }

    fn at(t: DateTime<Utc>) -> String {
        stamp(t)
    }

    #[test]
    fn unknown_codes_block_only_when_bunched_and_only_for_a_while() {
        let now = Utc::now();
        let n = rules::UNKNOWN_CODES_TO_BLOCK as i64;
        let bunched: Vec<String> = (0..n)
            .map(|i| at(now - TimeDelta::minutes(n - i)))
            .collect();
        assert!(blocks(&bunched, now));
        assert!(!blocks(&bunched[1..], now), "one fewer doesn't");
        // spread over more than the window: no block
        let spread: Vec<String> = (0..n)
            .map(|i| {
                at(now - TimeDelta::seconds(rules::UNKNOWN_WINDOW_SECS / (n - 1) * (n - i) + 60))
            })
            .collect();
        assert!(!blocks(&spread, now));
        // bunched, but the block has run out
        let old = now - TimeDelta::seconds(rules::BLOCK_SECS) - TimeDelta::minutes(1);
        let stale: Vec<String> = (0..n)
            .map(|i| at(old - TimeDelta::minutes(n - i)))
            .collect();
        assert!(!blocks(&stale, now));
    }
}
