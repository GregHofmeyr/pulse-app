//! Leaving a server, and its owner removing someone from it (#371).
//!
//! Anyone but the owner may leave; the owner may remove anyone else. The owner can't leave while they own it: they hand
//! it to another member first ([`transfer`], #418), or delete it (#184). Leaving and removing take the same way out
//! ([`take_out`]): the membership row goes, with their read points, replies seen, inbox reads and reminders in its
//! channels (their inbox drops the server's mentions, which only count for members), and everyone hears `MemberLeft`.
//! Anyone in one of its calls is taken out of it (voice state, their stream, watching, stage share and watch party
//! place), and their LiveKit connections for the server go (#149): the voice room, and every stream room of its
//! channels (ghost and named, `streams::ghosts::member_removed`), and then any participant still carrying one of their
//! grants for its rooms, as a logout removes them (`voice::revoke::remove_granted`, #418). A join they try later is
//! refused anyway (`voice::revoke::admit` checks membership). Their messages stay. Everything that checks membership as
//! it acts (posting, reacting, voice and stream tokens, the posting workers, scheduled messages at their time) refuses
//! them from then on, and mentions in its channels no longer reach them (`messages::mentions`). A system line says so
//! in its first text channel ("alex left the server", "sam was removed", #418). A custom status showing one of its
//! emoji loses the emoji (#418).
//!
//! A removed person may rejoin: servers are public to the group, and Join server works as before, starting at "all
//! read". But the third time the owner removes the same person within [`REMOVAL_WINDOW_HOURS`] (#418) they're blocked
//! ([`record_removal`]): Join server refuses them ([`BLOCKED_JOIN`]) until the owner allows them back
//! (`DELETE /servers/{id}/blocked/{user}`, [`allow_back`]), which clears the block and the removals it counted. The
//! block has no expiry. Only the owner sees who's blocked (`GET /servers/{id}/blocked`, Server settings › Members);
//! a block or an Allow back tells the owner's devices (`ServerBlocksChanged`, so an open Members pane asks again) and
//! the person blocked (Ready's `rejoin_blocked` and `RejoinBlockChanged`, so they see why before they press Join,
//! #431).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{ServerBlock, TransferServerRequest};

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::servers::routes::{server_exists, server_live_in};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/servers/{id}/leave", post(leave))
        .route("/servers/{id}/members/{user}", delete(remove))
        .route("/servers/{id}/owner", put(transfer))
        .route("/servers/{id}/blocked", get(blocked))
        .route("/servers/{id}/blocked/{user}", delete(allow_back))
}

/// How many removals of the same person from the same server, within [`REMOVAL_WINDOW_HOURS`], block their rejoin.
pub(crate) const REMOVALS_TO_BLOCK: i64 = 3;
/// The window [`REMOVALS_TO_BLOCK`] counts removals in.
pub(crate) const REMOVAL_WINDOW_HOURS: i64 = 48;
/// What Join server says to someone blocked from rejoining.
pub(crate) const BLOCKED_JOIN: &str = "You can't rejoin this server right now. Ask the owner.";
const ONLY_OWNER_BLOCKS: &str = "Only the server's owner can see or lift rejoin blocks.";

const OWNER_CANT_LEAVE: &str = "The owner can't leave their own server: hand it to someone else first (Server settings, Transfer ownership), or delete it.";
const ONLY_OWNER_REMOVES: &str = "Only the server's owner can remove people.";

/// The owner (creator) of `server`.
async fn owner_of(s: &AppState, server: ServerId) -> AppResult<UserId> {
    let owner: String = sqlx::query_scalar("SELECT created_by FROM servers WHERE id = ?")
        .bind(server.to_string())
        .fetch_one(&s.db)
        .await?;
    Ok(owner.parse().map_err(anyhow::Error::from)?)
}

/// `POST /servers/{id}/leave`: leave a server. 404 when it isn't there or you aren't in it; 403 for its owner.
async fn leave(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    server_exists(&s.db, id).await?;
    if owner_of(&s, id).await? == me {
        return Err(AppError::ForbiddenMsg(OWNER_CANT_LEAVE.into()));
    }
    take_out(&s, id, me, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /servers/{id}/members/{user}`: the owner removes someone. 403 for anyone else, 400 on yourself, 404 when
/// they aren't in it.
async fn remove(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path((id, user)): Path<(ServerId, UserId)>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    server_exists(&s.db, id).await?;
    if owner_of(&s, id).await? != me {
        return Err(AppError::ForbiddenMsg(ONLY_OWNER_REMOVES.into()));
    }
    if user == me {
        return Err(AppError::BadRequest(
            "you can't remove yourself from your own server".into(),
        ));
    }
    take_out(&s, id, user, Some(me)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /servers/{id}/blocked` (#418): the people blocked from rejoining, newest block first. The owner only (403 for
/// anyone else).
async fn blocked(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
) -> AppResult<Json<Vec<ServerBlock>>> {
    server_exists(&s.db, id).await?;
    if owner_of(&s, id).await? != me {
        return Err(AppError::ForbiddenMsg(ONLY_OWNER_BLOCKS.into()));
    }
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT user_id, blocked_at FROM server_blocks WHERE server_id = ? ORDER BY blocked_at DESC, user_id",
    )
    .bind(id.to_string())
    .fetch_all(&s.db)
    .await?;
    rows.into_iter()
        .map(|(user, blocked_at)| {
            Ok(ServerBlock {
                user_id: user.parse().map_err(anyhow::Error::from)?,
                blocked_at,
            })
        })
        .collect::<AppResult<Vec<_>>>()
        .map(Json)
}

/// `DELETE /servers/{id}/blocked/{user}` (#418): the owner allows a blocked person back; their removals so far stop
/// counting too. 403 for anyone but the owner, 404 when they aren't blocked. The owner is re-checked under the write
/// lock, as a hand-over may have landed since.
async fn allow_back(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path((id, user)): Path<(ServerId, UserId)>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    server_exists(&s.db, id).await?;
    let mut tx = crate::db::begin_write(&s.db).await?;
    server_live_in(&mut tx, id).await?;
    let (sid, uid) = (id.to_string(), user.to_string());
    let owner: String = sqlx::query_scalar("SELECT created_by FROM servers WHERE id = ?")
        .bind(&sid)
        .fetch_one(&mut *tx)
        .await?;
    if owner != me.to_string() {
        return Err(AppError::ForbiddenMsg(ONLY_OWNER_BLOCKS.into()));
    }
    let gone = sqlx::query("DELETE FROM server_blocks WHERE server_id = ? AND user_id = ?")
        .bind(&sid)
        .bind(&uid)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if gone == 0 {
        return Err(AppError::NotFound);
    }
    sqlx::query("DELETE FROM server_removals WHERE server_id = ? AND user_id = ?")
        .bind(&sid)
        .bind(&uid)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!(server = %id, user = %user, by = %me, "allowed back");
    blocks_changed(&s, id, me, user, false).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Tell `owner`'s devices that who's blocked from rejoining `server` changed, and `user` that they are (`blocked`) or
/// aren't any more (#431).
async fn blocks_changed(
    s: &AppState,
    server: ServerId,
    owner: UserId,
    user: UserId,
    blocked: bool,
) {
    s.hub
        .publish_all(
            &s.db,
            vec![
                Event::ServerBlocksChanged {
                    user_id: owner,
                    server_id: server,
                },
                Event::RejoinBlockChanged {
                    user_id: user,
                    server_id: server,
                    blocked,
                },
            ],
        )
        .await;
}

/// The live servers `user` is blocked from rejoining (#431), for their Ready.
pub(crate) async fn blocked_servers(
    db: &sqlx::SqlitePool,
    user: UserId,
) -> anyhow::Result<Vec<ServerId>> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT b.server_id FROM server_blocks b JOIN servers sv ON sv.id = b.server_id AND sv.deleted_at IS NULL
         WHERE b.user_id = ? ORDER BY b.server_id",
    )
    .bind(user.to_string())
    .fetch_all(db)
    .await?;
    ids.iter().map(|i| Ok(i.parse()?)).collect()
}

/// Whether `user` is blocked from rejoining `server` (#418), inside the join's write transaction.
pub(crate) async fn blocked_in(
    tx: &mut sqlx::SqliteConnection,
    server: ServerId,
    user: UserId,
) -> AppResult<bool> {
    let hit: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM server_blocks WHERE server_id = ? AND user_id = ?")
            .bind(server.to_string())
            .bind(user.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    Ok(hit.is_some())
}

/// Note that `by` removed `user` from `server` (#418), in the removal's transaction: removals older than the window go,
/// and the [`REMOVALS_TO_BLOCK`]th within it blocks their rejoin. True when this one blocked them.
async fn record_removal(
    tx: &mut sqlx::SqliteConnection,
    server: ServerId,
    user: UserId,
    by: UserId,
) -> AppResult<bool> {
    let (sid, uid) = (server.to_string(), user.to_string());
    let at = crate::db::now();
    let since =
        crate::db::stamp(chrono::Utc::now() - chrono::TimeDelta::hours(REMOVAL_WINDOW_HOURS));
    sqlx::query(
        "DELETE FROM server_removals WHERE server_id = ? AND user_id = ? AND removed_at <= ?",
    )
    .bind(&sid)
    .bind(&uid)
    .bind(&since)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO server_removals (server_id, user_id, removed_by, removed_at) VALUES (?, ?, ?, ?)",
    )
    .bind(&sid)
    .bind(&uid)
    .bind(by.to_string())
    .bind(&at)
    .execute(&mut *tx)
    .await?;
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM server_removals WHERE server_id = ? AND user_id = ?",
    )
    .bind(&sid)
    .bind(&uid)
    .fetch_one(&mut *tx)
    .await?;
    if count < REMOVALS_TO_BLOCK {
        return Ok(false);
    }
    let added = sqlx::query(
        "INSERT OR IGNORE INTO server_blocks (server_id, user_id, blocked_at) VALUES (?, ?, ?)",
    )
    .bind(&sid)
    .bind(&uid)
    .bind(&at)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    Ok(added > 0)
}

/// `PUT /servers/{id}/owner`: its owner hands the server to another of its members (#418). 403 for anyone but the
/// owner, 400 for themselves or someone who already owns as many servers as anyone may create, 404 when the new owner
/// isn't in it. One write transaction: the owner is re-checked under the write lock, so of two racing transfers only
/// the first lands (the second is no longer the owner's). Everyone hears `ServerOwnerChanged`; from then on the old
/// owner is a member like any other, and may leave.
async fn transfer(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
    Json(req): Json<TransferServerRequest>,
) -> AppResult<StatusCode> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    server_exists(&s.db, id).await?;
    let to = req.user_id;
    if to == me {
        return Err(AppError::BadRequest("you already own this server".into()));
    }
    let mut tx = crate::db::begin_write(&s.db).await?;
    server_live_in(&mut tx, id).await?; // deleted since the check above (#287)
    let owner: String = sqlx::query_scalar("SELECT created_by FROM servers WHERE id = ?")
        .bind(id.to_string())
        .fetch_one(&mut *tx)
        .await?;
    if owner != me.to_string() {
        return Err(AppError::ForbiddenMsg(
            "Only the server's owner can hand it over.".into(),
        ));
    }
    let member: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM server_members WHERE server_id = ? AND user_id = ?")
            .bind(id.to_string())
            .bind(to.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    if member.is_none() {
        return Err(AppError::NotFound);
    }
    // Owning counts against the servers they may create (deleted ones too, as creating counts them).
    let theirs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM servers WHERE created_by = ?")
        .bind(to.to_string())
        .fetch_one(&mut *tx)
        .await?;
    if theirs >= crate::limits::caps::SERVERS_PER_USER {
        return Err(AppError::BadRequest(format!(
            "they already own {} servers, the most anyone may",
            crate::limits::caps::SERVERS_PER_USER
        )));
    }
    sqlx::query("UPDATE servers SET created_by = ? WHERE id = ? AND created_by = ?")
        .bind(to.to_string())
        .bind(id.to_string())
        .bind(me.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!(server = %id, from = %me, to = %to, "server handed over");
    s.hub
        .publish(
            &s.db,
            Event::ServerOwnerChanged {
                server_id: id,
                owner_id: to,
                previous_owner_id: me,
            },
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}

/// Take `user` out of `server` (they left: `by` is `None`; or `by` removed them). See the module docs.
pub(crate) async fn take_out(
    s: &AppState,
    server: ServerId,
    user: UserId,
    by: Option<UserId>,
) -> AppResult<()> {
    // A leave changes a live Quote of the Year's member count, which its events carry as absolute state: in commit
    // order, as a join publishes it.
    // their name as it is now, for the line's text (clients name them as they're called when they show it)
    let name = crate::dms::routes::shown_name(&s.db, user).await?;
    let order = s.hub.in_order().await;
    let mut tx = crate::db::begin_write(&s.db).await?;
    server_live_in(&mut tx, server).await?; // deleted since the route's check (#287)
    let (sid, uid) = (server.to_string(), user.to_string());
    // The owner again, under the write lock: a hand-over (`transfer`) that committed after the route's check wins, so
    // the new owner can't leave and the old one can no longer remove anyone.
    let owner: String = sqlx::query_scalar("SELECT created_by FROM servers WHERE id = ?")
        .bind(&sid)
        .fetch_one(&mut *tx)
        .await?;
    if owner == uid {
        return Err(AppError::ForbiddenMsg(OWNER_CANT_LEAVE.into()));
    }
    if by.is_some_and(|by| by.to_string() != owner) {
        return Err(AppError::ForbiddenMsg(ONLY_OWNER_REMOVES.into()));
    }
    let gone = sqlx::query("DELETE FROM server_members WHERE server_id = ? AND user_id = ?")
        .bind(&sid)
        .bind(&uid)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if gone == 0 {
        return Err(AppError::NotFound); // not in it (or a racing leave won); dropping `tx` rolls back
    }
    // Their place in its chats: read points (the inbox only lists chats you have one in), replies seen and items
    // opened from the inbox. A rejoin starts at "all read" again.
    sqlx::query(
        "DELETE FROM read_states WHERE user_id = ?
         AND channel_id IN (SELECT id FROM channels WHERE server_id = ?)",
    )
    .bind(&uid)
    .bind(&sid)
    .execute(&mut *tx)
    .await?;
    for table in ["thread_reads", "inbox_reads"] {
        let col = if table == "thread_reads" {
            "root_id"
        } else {
            "message_id"
        };
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE user_id = ? AND {col} IN
               (SELECT m.id FROM messages m JOIN channels c ON c.id = m.channel_id WHERE c.server_id = ?)"
        ))
        .bind(&uid)
        .bind(&sid)
        .execute(&mut *tx)
        .await?;
    }
    // Their reminders on its messages (#300) leave their inbox with it.
    let reminders: Vec<String> = sqlx::query_scalar(
        "DELETE FROM message_reminders WHERE user_id = ? AND message_id IN
           (SELECT m.id FROM messages m JOIN channels c ON c.id = m.channel_id WHERE c.server_id = ?)
         RETURNING id",
    )
    .bind(&uid)
    .bind(&sid)
    .fetch_all(&mut *tx)
    .await?;
    let qoty = crate::qoty::live_in(&mut tx, server).await?;
    let line = member_line_in(&mut tx, server, user, by, &name).await?;
    // a custom status showing one of its emoji drops it, keeping the text (#418)
    let status_changed = crate::presence::custom::drop_server_emoji(&mut tx, user, server).await?;
    // Their LiveKit grants for its rooms go with the membership (a join with one is refused anyway); the participants
    // still carrying one are removed below, whatever identity they connected under, as a logout does (#418).
    let grants = crate::auth::session::drop_server_grants(&mut tx, user, server).await?;
    // a removal counts towards blocking their rejoin (#418); a leave doesn't
    let blocked = match by {
        Some(by) => record_removal(&mut tx, server, user, by).await?,
        None => false,
    };
    tx.commit().await?;
    tracing::info!(server = %server, user = %user, by = ?by, blocked, "left server");
    if let Some(message) = line {
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
    s.hub
        .publish(
            &s.db,
            Event::MemberLeft {
                server_id: server,
                user_id: user,
                removed_by: by,
            },
        )
        .await;
    if blocked && let Some(by) = by {
        blocks_changed(s, server, by, user, true).await;
    }
    for id in reminders {
        match id.parse() {
            Ok(id) => {
                s.hub
                    .publish(&s.db, Event::ReminderRemoved { user_id: user, id })
                    .await
            }
            Err(e) => tracing::error!(error = %e, "bad reminder id"),
        }
    }
    if let Some(event) = qoty {
        crate::qoty::publish_updated(s, event).await;
    }
    drop(order);
    if status_changed {
        crate::presence::custom::announce_own(s, user).await;
        s.hub.sync_presence(&s.db, user).await;
    }
    out_of_calls(s, server, user, by).await;
    // Their stream-room connections in its channels go too (#149): a ghost viewer isn't in the call, and a named
    // viewer's `#v` / `#a` may outlive their voice connection.
    crate::streams::ghosts::member_removed(s, server, user).await;
    // and any stray connection by grant: one in another of its rooms (a lost join webhook), or under an identity the
    // calls above don't name
    crate::voice::revoke::remove_granted(s, &grants).await;
    Ok(())
}

/// Store the line saying `user` left `server` (or `by` removed them) in its first text channel, the one at the top of
/// its channel list (uncategorised channels first, then each category in order; deleted and archived ones skipped),
/// where a new server's #general sits. None when it has no such channel. It carries their id, so clients show today's
/// name; the text is for clients from before it. Servers have no join lines to match.
async fn member_line_in(
    tx: &mut sqlx::SqliteConnection,
    server: ServerId,
    user: UserId,
    by: Option<UserId>,
    name: &str,
) -> AppResult<Option<pulse_protocol::rest::Message>> {
    let channel: Option<String> = sqlx::query_scalar(
        "SELECT c.id FROM channels c LEFT JOIN categories k ON k.id = c.category_id
         WHERE c.server_id = ? AND c.kind = 'text' AND c.deleted_at IS NULL AND c.archived_at IS NULL
         ORDER BY k.id IS NOT NULL, k.position, c.position, c.id LIMIT 1",
    )
    .bind(server.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    let Some(channel) = channel else {
        return Ok(None);
    };
    let line = pulse_protocol::rest::MemberLine {
        user_id: user,
        removed_by: by,
    };
    let content = match by {
        None => format!("{name} left the server"),
        Some(_) => format!("{name} was removed"),
    };
    let payload = crate::messages::system::member_line_payload(&line)?;
    let channel = channel.parse().map_err(anyhow::Error::from)?;
    Ok(Some(
        crate::messages::system::insert(tx, channel, content, payload).await?,
    ))
}

/// `user` is no longer in `server`: out of its call, if they're in one (told who did it first, when removed, as a
/// voice disconnect tells them). Never fails; problems are logged.
async fn out_of_calls(s: &AppState, server: ServerId, user: UserId, by: Option<UserId>) {
    let channel = match crate::voice::moderation::call_in(s, server, user).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = ?e, server = %server, user = %user, "couldn't look up a leaver's call");
            None
        }
    };
    let Some(channel) = channel else {
        return;
    };
    let ch = match crate::access::load_channel(&s.db, channel).await {
        Ok(Some(ch)) => ch,
        Ok(None) => return, // deleted meanwhile: its delete ended the call
        Err(e) => {
            tracing::error!(error = ?e, channel = %channel, "couldn't load a leaver's call");
            return;
        }
    };
    if let Some(by) = by {
        s.hub
            .publish(
                &s.db,
                Event::VoiceDisconnected {
                    channel_id: channel,
                    user_id: user,
                    by,
                },
            )
            .await;
    }
    if let Err(e) = crate::voice::routes::left_call(s, &ch, user, "", false).await {
        tracing::error!(error = ?e, channel = %channel, user = %user, "couldn't take a leaver out of their call");
    }
    // out of its LiveKit voice room, and their connections out of its stream room
    crate::voice::revoke::remove_from_call(s, channel, &[user]).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{create_server, post_json, register, spawn};

    /// alex owns "Main", sam and kim are in it, and then (as if a hand-over landed after the route's owner check, before
    /// `take_out`'s transaction) the server is handed to sam.
    async fn handed_to_sam() -> (crate::testing::TestApp, ServerId, UserId, UserId, UserId) {
        let app = spawn().await;
        let (alex, a) = register(&app, "alex").await;
        let (sam, s) = register(&app, "sam").await;
        let (kim, k) = register(&app, "kim").await;
        let server = create_server(&app, &a, "Main").await.id;
        for t in [&s, &k] {
            let r = post_json(
                &app,
                t,
                &format!("/servers/{server}/join"),
                serde_json::json!({}),
            )
            .await;
            assert_eq!(r.status(), 204);
        }
        sqlx::query("UPDATE servers SET created_by = ? WHERE id = ?")
            .bind(sam.to_string())
            .bind(server.to_string())
            .execute(&app.db)
            .await
            .unwrap();
        (app, server, alex, sam, kim)
    }

    async fn is_member(app: &crate::testing::TestApp, server: ServerId, user: UserId) -> bool {
        crate::servers::routes::is_server_member(&app.db, server, user)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_leave_racing_a_hand_over_to_the_leaver_is_refused() {
        let (app, server, _, sam, _) = handed_to_sam().await;
        let r = take_out(&app.state(), server, sam, None).await;
        assert!(matches!(r, Err(AppError::ForbiddenMsg(_))), "{r:?}");
        assert!(
            is_member(&app, server, sam).await,
            "the new owner is still in it"
        );
    }

    #[tokio::test]
    async fn a_removal_by_someone_who_just_handed_the_server_over_is_refused() {
        let (app, server, alex, _, kim) = handed_to_sam().await;
        let r = take_out(&app.state(), server, kim, Some(alex)).await;
        assert!(matches!(r, Err(AppError::ForbiddenMsg(_))), "{r:?}");
        assert!(is_member(&app, server, kim).await);
    }

    #[tokio::test]
    async fn the_owner_still_removes_and_members_still_leave() {
        let (app, server, alex, sam, kim) = handed_to_sam().await;
        take_out(&app.state(), server, kim, Some(sam))
            .await
            .unwrap();
        take_out(&app.state(), server, alex, None).await.unwrap();
        assert!(!is_member(&app, server, kim).await);
        assert!(!is_member(&app, server, alex).await);
    }
}
