use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId, next_ulid};
use pulse_protocol::rest::{
    Channel, ChannelKind, CreateWheelRequest, Message, QuietSpin, SpinAgainRequest,
    SpinWheelRequest, UpdateWheelRequest, Wheel, WheelScope, WheelSpin, wheel_limits,
};

use super::{clean_name, clean_options, draw, for_owner, for_server, load, spinnable};
use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::{begin_write, now};
use crate::error::{AppError, AppResult};
use crate::messages::system;
use crate::servers::routes::{is_server_member, require_member};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/servers/{id}/wheels", get(list_server))
        .route("/wheels/mine", get(list_mine))
        .route("/wheels", post(create))
        .route("/wheels/{id}", patch(update).delete(delete))
        .route("/channels/{id}/wheel-spins", post(spin))
        .route("/channels/{id}/wheel-spins/quiet", post(spin_quietly))
        .route("/messages/{id}/spin-again", post(spin_again))
}

fn too_many_wheels(what: &str, cap: i64) -> AppError {
    AppError::BadRequest(format!(
        "{what} can hold {cap} wheels; delete one to make room"
    ))
}

/// A stored spin: its chat, saved wheel, name, options (JSON) and winner.
type SpinRow = (String, Option<String>, Option<String>, String, i64);

fn spin_too_soon() -> AppError {
    AppError::TooManyRequests(Some(
        "one spin every few seconds; try again in a moment".into(),
    ))
}

/// The wheel, if `me` may see and change it: a server wheel's server members, a Just me wheel's owner (anyone
/// else gets 404: it doesn't exist for them).
async fn editable(s: &AppState, me: UserId, id: &str) -> AppResult<Wheel> {
    let wheel = load(&s.db, id).await?.ok_or(AppError::NotFound)?;
    match (wheel.server_id, wheel.owner_id) {
        (_, Some(owner)) if owner != me => Err(AppError::NotFound),
        (Some(server), _) if !is_server_member(&s.db, server, me).await? => {
            Err(AppError::Forbidden)
        }
        _ => Ok(wheel),
    }
}

fn saved(wheel: Wheel) -> Event {
    Event::WheelSaved { wheel }
}

async fn list_server(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(server): Path<ServerId>,
) -> AppResult<Json<Vec<Wheel>>> {
    require_member(&s.db, server, me).await?;
    Ok(Json(for_server(&s.db, server).await?))
}

async fn list_mine(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
) -> AppResult<Json<Vec<Wheel>>> {
    Ok(Json(for_owner(&s.db, me).await?))
}

async fn create(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(req): Json<CreateWheelRequest>,
) -> AppResult<(StatusCode, Json<Wheel>)> {
    if !s.limits.wheel_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let name = clean_name(&req.name)?;
    let options = clean_options(req.options)?;
    if let Some(server) = req.server_id {
        require_member(&s.db, server, me).await?;
    }
    let id = next_ulid().to_string();
    let at = now();
    let mut tx = begin_write(&s.db).await?;
    // Counted under the write lock, so two creates at once can't both slip under the cap.
    let (count, cap, whose): (i64, i64, &str) = match req.server_id {
        Some(server) => (
            sqlx::query_scalar("SELECT COUNT(*) FROM wheels WHERE server_id = ?")
                .bind(server.to_string())
                .fetch_one(&mut *tx)
                .await?,
            wheel_limits::WHEELS_PER_SERVER,
            "A server",
        ),
        None => (
            sqlx::query_scalar("SELECT COUNT(*) FROM wheels WHERE owner_id = ?")
                .bind(me.to_string())
                .fetch_one(&mut *tx)
                .await?,
            wheel_limits::WHEELS_PER_PERSON,
            "Just me",
        ),
    };
    if count >= cap {
        return Err(too_many_wheels(whose, cap));
    }
    sqlx::query(
        "INSERT INTO wheels (id, server_id, owner_id, name, options, created_by, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(req.server_id.map(|s| s.to_string()))
    .bind(req.server_id.is_none().then(|| me.to_string()))
    .bind(&name)
    .bind(serde_json::to_string(&options).map_err(anyhow::Error::from)?)
    .bind(me.to_string())
    .bind(&at)
    .bind(&at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let wheel = load(&s.db, &id).await?.ok_or(AppError::NotFound)?;
    s.hub.publish(&s.db, saved(wheel.clone())).await;
    Ok((StatusCode::CREATED, Json(wheel)))
}

async fn update(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateWheelRequest>,
) -> AppResult<Json<Wheel>> {
    if !s.limits.wheel_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let old = editable(&s, me, &id).await?;
    let name = match &req.name {
        Some(n) => clean_name(n)?,
        None => old.name,
    };
    let options = match req.options {
        Some(o) => clean_options(o)?,
        None => old.options,
    };
    let changed =
        sqlx::query("UPDATE wheels SET name = ?, options = ?, updated_at = ? WHERE id = ?")
            .bind(&name)
            .bind(serde_json::to_string(&options).map_err(anyhow::Error::from)?)
            .bind(now())
            .bind(&id)
            .execute(&s.db)
            .await?
            .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    let wheel = load(&s.db, &id).await?.ok_or(AppError::NotFound)?;
    s.hub.publish(&s.db, saved(wheel.clone())).await;
    Ok(Json(wheel))
}

async fn delete(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    if !s.limits.wheel_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let wheel = editable(&s, me, &id).await?;
    let gone = sqlx::query("DELETE FROM wheels WHERE id = ?")
        .bind(&id)
        .execute(&s.db)
        .await?
        .rows_affected();
    if gone == 0 {
        return Err(AppError::NotFound);
    }
    s.hub
        .publish(
            &s.db,
            Event::WheelDeleted {
                wheel_id: id,
                server_id: wheel.server_id,
                owner_id: wheel.owner_id,
            },
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}

/// The chat `me` may spin in: as sending a message (a DM or group only for its members, a server chat only for
/// the server's members), and not a quotes or clips channel.
async fn spin_channel(s: &AppState, me: UserId, id: ChannelId) -> AppResult<Channel> {
    let ch = channel_for(&s.db, me, id).await?;
    if matches!(ch.kind, ChannelKind::Quotes | ChannelKind::Clips) {
        return Err(AppError::BadRequest(
            "wheels spin in text and voice chats, DMs and groups".into(),
        ));
    }
    if let Some(server) = ch.server_id
        && !is_server_member(&s.db, server, me).await?
    {
        return Err(AppError::Forbidden);
    }
    Ok(ch)
}

/// `POST /channels/{id}/wheel-spins`: spin a saved wheel or a quick one. Returns the spin's line, which reaches
/// the chat as a `MessageCreated`.
async fn spin(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<SpinWheelRequest>,
) -> AppResult<(StatusCode, Json<Message>)> {
    crate::media::writes_ok(&s.cfg)?;
    let ch = spin_channel(&s, me, id).await?;
    let (wheel_id, name, options) = what_to_spin(&s, me, &ch, req).await?;
    // Spent once the spin is sure to go ahead: a refused one (too few options, the wrong chat) costs nothing.
    if !s.limits.wheel_spin_user.hit(&me.to_string()) {
        return Err(spin_too_soon());
    }
    let line = post_spin(&s, me, &ch, wheel_id, name, options).await?;
    Ok((StatusCode::CREATED, Json(line)))
}

/// `POST /channels/{id}/wheel-spins/quiet` (#270): a spin the chat watches live that leaves nothing behind. Drawn
/// and checked as [`spin`] (the same rate limit), but nothing is stored: the spin reaches whoever is in the chat
/// now as a `WheelSpunQuietly`, and the answer.
async fn spin_quietly(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<SpinWheelRequest>,
) -> AppResult<(StatusCode, Json<QuietSpin>)> {
    let ch = spin_channel(&s, me, id).await?;
    let (wheel_id, name, options) = what_to_spin(&s, me, &ch, req).await?;
    if !s.limits.wheel_spin_user.hit(&me.to_string()) {
        return Err(spin_too_soon());
    }
    let (winner, angle) = draw(options.len(), &mut rand::rng());
    let quiet = QuietSpin {
        id: next_ulid().to_string(),
        channel_id: ch.id,
        spin: WheelSpin {
            spinner_id: Some(me),
            wheel_id,
            name,
            options,
            winner: u32::try_from(winner).map_err(anyhow::Error::from)?,
            angle,
        },
    };
    s.hub
        .publish(
            &s.db,
            Event::WheelSpunQuietly {
                spin: quiet.clone(),
            },
        )
        .await;
    Ok((StatusCode::CREATED, Json(quiet)))
}

/// What a spin request spins: a saved wheel (its id, name and options), a quick wheel (its options), or a saved
/// wheel with only some of its options (named; each option at most as often as the wheel has it). At least two.
async fn what_to_spin(
    s: &AppState,
    me: UserId,
    ch: &Channel,
    req: SpinWheelRequest,
) -> AppResult<(Option<String>, Option<String>, Vec<String>)> {
    let picked = match (req.wheel_id, req.options) {
        (Some(w), only) => {
            let wheel = load(&s.db, &w).await?.ok_or(AppError::NotFound)?;
            match wheel.scope {
                WheelScope::Personal if wheel.owner_id != Some(me) => {
                    return Err(AppError::NotFound);
                }
                WheelScope::Server if wheel.server_id != ch.server_id => {
                    return Err(AppError::BadRequest(
                        "a server's wheels spin in that server's chats".into(),
                    ));
                }
                _ => {}
            }
            let options = match only {
                None => wheel.options,
                Some(only) => {
                    let only = clean_options(only)?;
                    let mut left = wheel.options;
                    for o in &only {
                        let Some(i) = left.iter().position(|x| x == o) else {
                            return Err(AppError::BadRequest(
                                "that wheel has changed; open it again to spin".into(),
                            ));
                        };
                        left.swap_remove(i);
                    }
                    only
                }
            };
            (Some(wheel.id), Some(wheel.name), options)
        }
        (None, Some(options)) => (None, None, clean_options(options)?),
        (None, None) => {
            return Err(AppError::BadRequest(
                "spin a saved wheel or a list of options".into(),
            ));
        }
    };
    spinnable(&picked.2)?;
    Ok(picked)
}

/// `POST /messages/{id}/spin-again`: Spin again (the same options) or Remove winner & spin, from a spin's line.
/// Anyone who may spin in its chat may.
async fn spin_again(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
    Json(req): Json<SpinAgainRequest>,
) -> AppResult<(StatusCode, Json<Message>)> {
    crate::media::writes_ok(&s.cfg)?;
    let row: Option<SpinRow> = sqlx::query_as(
        // a deleted line (soft-deleted, so its spin row stays) spins no more
        "SELECT w.channel_id, w.wheel_id, w.name, w.options, w.winner FROM wheel_spins w
         JOIN messages m ON m.id = w.message_id
         WHERE w.message_id = ? AND m.deleted_at IS NULL",
    )
    .bind(id.to_string())
    .fetch_optional(&s.db)
    .await?;
    let (channel, wheel_id, name, options, winner) = row.ok_or(AppError::NotFound)?;
    let channel: ChannelId = channel.parse().map_err(anyhow::Error::from)?;
    // 404 for anyone who can't see the chat, as if there were no such spin.
    let ch = spin_channel(&s, me, channel).await?;
    let mut options = super::parse_options(&options)?;
    if req.remove_winner {
        let i = usize::try_from(winner).map_err(anyhow::Error::from)?;
        if i < options.len() {
            options.remove(i);
        }
        if options.len() < wheel_limits::MIN_OPTIONS {
            return Err(AppError::BadRequest(
                "that would leave fewer than 2 options".into(),
            ));
        }
    }
    if !s.limits.wheel_spin_user.hit(&me.to_string()) {
        return Err(spin_too_soon());
    }
    let line = post_spin(&s, me, &ch, wheel_id, name, options).await?;
    Ok((StatusCode::CREATED, Json(line)))
}

/// Draw, store the line and the spin together, then tell the chat.
async fn post_spin(
    s: &AppState,
    me: UserId,
    ch: &Channel,
    wheel_id: Option<String>,
    name: Option<String>,
    options: Vec<String>,
) -> AppResult<Message> {
    let (winner, angle) = draw(options.len(), &mut rand::rng());
    let who = crate::dms::routes::shown_name(&s.db, me).await?;
    let content = format!(
        "🎡 {who} spun {} → {}",
        name.as_deref().unwrap_or("a quick wheel"),
        options[winner]
    );
    let spin = WheelSpin {
        spinner_id: Some(me),
        wheel_id,
        name,
        options,
        winner: u32::try_from(winner).map_err(anyhow::Error::from)?,
        angle,
    };
    let payload = system::wheel_spin_payload(&spin)?;
    let channel = ch.id;
    let mut tx = begin_write(&s.db).await?;
    let line = system::insert(&mut tx, channel, content, payload).await?;
    let inserted = sqlx::query(
        "INSERT INTO wheel_spins (message_id, channel_id, spinner_id, wheel_id, name, options, winner, angle, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(line.id.to_string())
    .bind(channel.to_string())
    .bind(me.to_string())
    .bind(&spin.wheel_id)
    .bind(&spin.name)
    .bind(serde_json::to_string(&spin.options).map_err(anyhow::Error::from)?)
    .bind(i64::from(spin.winner))
    .bind(spin.angle)
    .bind(&line.created_at)
    .execute(&mut *tx)
    .await;
    match inserted {
        // The saved wheel (or the chat) went between the read and the write.
        Err(sqlx::Error::Database(d)) if d.is_foreign_key_violation() => {
            return Err(AppError::NotFound);
        }
        r => {
            r?;
        }
    }
    if ch.kind.is_private() {
        // Like a new message or a call line, a spin reopens the conversation for anyone who closed it.
        sqlx::query("DELETE FROM dm_hidden WHERE channel_id = ?")
            .bind(channel.to_string())
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    s.hub
        .publish(
            &s.db,
            Event::MessageCreated {
                message: line.clone(),
                nonce: None,
            },
        )
        .await;
    Ok(line)
}
