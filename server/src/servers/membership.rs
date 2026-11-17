//! Leaving a server, and its owner removing someone from it (#371).
//!
//! Anyone but the owner may leave; the owner may remove anyone else. The owner can't leave while they own it: they hand
//! it to another member first ([`transfer`], #418), or delete it (#184). Leaving and removing take the same way out ([`take_out`]): the membership row
//! goes, with their read points, replies seen, inbox reads and reminders in its channels (their inbox drops the
//! server's mentions, which only count for members), and everyone hears `MemberLeft`. Anyone in one of its calls is
//! taken out of it (voice state, their stream, watching, stage share and watch party place), and their LiveKit
//! connections for the server go (#149): the voice room, and every stream room of its channels (ghost and named,
//! `streams::ghosts::member_removed`). A join they try later is refused anyway (`voice::revoke::admit` checks
//! membership). Their messages stay. Everything that checks membership as it acts (posting, reacting, voice and stream
//! tokens, the posting workers, scheduled messages at their time) refuses them from then on, and mentions in its
//! channels no longer reach them (`messages::mentions`). A system line says so in its first text channel ("alex left
//! the server", "sam was removed", #418). A custom status showing one of its emoji loses the emoji (#418).
//!
//! Nothing stops a rejoin: servers are public to the group, and Join server works as before, starting at "all read".

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, post, put};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::TransferServerRequest;

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::servers::routes::{server_exists, server_live_in};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/servers/{id}/leave", post(leave))
        .route("/servers/{id}/members/{user}", delete(remove))
        .route("/servers/{id}/owner", put(transfer))
}

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
        return Err(AppError::ForbiddenMsg(
            "The owner can't leave their own server: hand it to someone else first (Server settings, Transfer ownership), or delete it.".into(),
        ));
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
        return Err(AppError::ForbiddenMsg(
            "Only the server's owner can remove people.".into(),
        ));
    }
    if user == me {
        return Err(AppError::BadRequest(
            "you can't remove yourself from your own server".into(),
        ));
    }
    take_out(&s, id, user, Some(me)).await?;
    Ok(StatusCode::NO_CONTENT)
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
    tx.commit().await?;
    tracing::info!(server = %server, user = %user, by = ?by, "left server");
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
