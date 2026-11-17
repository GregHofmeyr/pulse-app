//! Sports routes (#385): your follows and "Sports score pings" switch, each league's teams for the follow search, and
//! each server's Sports channel.

use axum::extract::{Path, Query, State};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use pulse_protocol::rest::{
    ChannelKind, FollowTeamRequest, SetSportsChannelRequest, SportsChannel, SportsLeague,
    SportsSettingsRequest, SportsStatus, SportsTeam, SportsTeams,
};
use serde::Deserialize;

use crate::AppState;
use crate::access::load_channel;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::servers::routes::require_member;

/// The most teams one person follows.
pub const MAX_FOLLOWS: i64 = 12;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/sports", get(status).put(settings))
        .route("/me/sports/follows", post(follow))
        .route("/me/sports/follows/{league}/{team}", delete(unfollow))
        .route("/sports/teams", get(teams))
        .route(
            "/servers/{id}/sports-channel",
            get(get_channel).put(set_channel),
        )
}

fn not_set_up() -> AppError {
    AppError::BadRequest("Sports aren't set up on this server.".into())
}

pub async fn status_of(s: &AppState, me: UserId) -> AppResult<SportsStatus> {
    let pings: Option<bool> =
        sqlx::query_scalar("SELECT pings FROM sports_prefs WHERE user_id = ?")
            .bind(me.to_string())
            .fetch_optional(&s.db)
            .await?;
    let rows: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT league, team_id, team_name FROM sports_follows WHERE user_id = ? ORDER BY created_at, league, team_id",
    )
    .bind(me.to_string())
    .fetch_all(&s.db)
    .await?;
    Ok(SportsStatus {
        available: s.sports.enabled(),
        pings: pings.unwrap_or(true),
        follows: rows
            .into_iter()
            .filter_map(|(l, id, name)| {
                Some(SportsTeam {
                    league: SportsLeague::parse(&l)?,
                    team_id: id.clamp(0, u32::MAX as i64) as u32,
                    name,
                })
            })
            .collect(),
        leagues: SportsLeague::ALL.to_vec(),
        note: if s.sports.enabled() {
            s.sports.api().note()
        } else {
            None
        },
    })
}

/// `GET /me/sports`.
async fn status(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
) -> AppResult<Json<SportsStatus>> {
    Ok(Json(status_of(&s, me).await?))
}

/// `PUT /me/sports`: your "Sports score pings" switch.
async fn settings(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(req): Json<SportsSettingsRequest>,
) -> AppResult<Json<SportsStatus>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    if let Some(on) = req.pings {
        sqlx::query(
            "INSERT INTO sports_prefs (user_id, pings) VALUES (?, ?)
             ON CONFLICT (user_id) DO UPDATE SET pings = excluded.pings",
        )
        .bind(me.to_string())
        .bind(on)
        .execute(&s.db)
        .await?;
    }
    Ok(Json(status_of(&s, me).await?))
}

/// `POST /me/sports/follows`: follow a team from the league's list.
async fn follow(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(req): Json<FollowTeamRequest>,
) -> AppResult<Json<SportsStatus>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    if !s.sports.enabled() {
        return Err(not_set_up());
    }
    let name: Option<String> =
        sqlx::query_scalar("SELECT name FROM sports_teams WHERE league = ? AND team_id = ?")
            .bind(req.league.key())
            .bind(req.team_id)
            .fetch_optional(&s.db)
            .await?;
    let Some(name) = name else {
        return Err(AppError::BadRequest(
            "pick a team from the league's list".into(),
        ));
    };
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sports_follows WHERE user_id = ?")
        .bind(me.to_string())
        .fetch_one(&s.db)
        .await?;
    if n >= MAX_FOLLOWS {
        return Err(AppError::BadRequest(format!(
            "You can follow up to {MAX_FOLLOWS} teams."
        )));
    }
    sqlx::query(
        "INSERT OR IGNORE INTO sports_follows (user_id, league, team_id, team_name, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(me.to_string())
    .bind(req.league.key())
    .bind(req.team_id)
    .bind(&name)
    .bind(crate::db::now())
    .execute(&s.db)
    .await?;
    Ok(Json(status_of(&s, me).await?))
}

/// `DELETE /me/sports/follows/{league}/{team}`.
async fn unfollow(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path((league, team)): Path<(String, u32)>,
) -> AppResult<Json<SportsStatus>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let league = SportsLeague::parse(&league).ok_or(AppError::NotFound)?;
    sqlx::query("DELETE FROM sports_follows WHERE user_id = ? AND league = ? AND team_id = ?")
        .bind(me.to_string())
        .bind(league.key())
        .bind(team)
        .execute(&s.db)
        .await?;
    Ok(Json(status_of(&s, me).await?))
}

#[derive(Deserialize)]
struct TeamsQuery {
    league: String,
}

/// `GET /sports/teams?league=psl`: the league's teams this season (apps filter them as people type).
async fn teams(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Query(q): Query<TeamsQuery>,
) -> AppResult<Json<SportsTeams>> {
    let league = SportsLeague::parse(&q.league)
        .ok_or_else(|| AppError::BadRequest("unknown league".into()))?;
    if !s.sports.enabled() {
        return Err(not_set_up());
    }
    // a read may spend a request: the search counts as a mutation for the rate limit
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    Ok(Json(SportsTeams {
        teams: super::teams::list(&s, league).await?,
    }))
}

async fn channel_of(s: &AppState, server: ServerId) -> AppResult<SportsChannel> {
    let ch: Option<String> = sqlx::query_scalar(
        "SELECT d.channel_id FROM server_sports_channels d
         JOIN channels c ON c.id = d.channel_id AND c.deleted_at IS NULL
         WHERE d.server_id = ?",
    )
    .bind(server.to_string())
    .fetch_optional(&s.db)
    .await?;
    Ok(SportsChannel {
        channel_id: ch.and_then(|c| c.parse().ok()),
    })
}

async fn get_channel(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
) -> AppResult<Json<SportsChannel>> {
    require_member(&s.db, id, me).await?;
    Ok(Json(channel_of(&s, id).await?))
}

/// `PUT /servers/{id}/sports-channel`: the server's owner picks one of its text channels, or none.
async fn set_channel(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
    Json(req): Json<SetSportsChannelRequest>,
) -> AppResult<Json<SportsChannel>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    require_member(&s.db, id, me).await?;
    let owner: String = sqlx::query_scalar("SELECT created_by FROM servers WHERE id = ?")
        .bind(id.to_string())
        .fetch_one(&s.db)
        .await?;
    if owner != me.to_string() {
        return Err(AppError::ForbiddenMsg(
            "Only the server's owner can pick its Sports channel.".into(),
        ));
    }
    match req.channel_id {
        None => {
            sqlx::query("DELETE FROM server_sports_channels WHERE server_id = ?")
                .bind(id.to_string())
                .execute(&s.db)
                .await?;
        }
        Some(ch) => {
            check_channel(&s, id, ch).await?;
            sqlx::query(
                "INSERT INTO server_sports_channels (server_id, channel_id) VALUES (?, ?)
                 ON CONFLICT (server_id) DO UPDATE SET channel_id = excluded.channel_id",
            )
            .bind(id.to_string())
            .bind(ch.to_string())
            .execute(&s.db)
            .await?;
        }
    }
    Ok(Json(channel_of(&s, id).await?))
}

async fn check_channel(s: &AppState, server: ServerId, ch: ChannelId) -> AppResult<()> {
    let channel = load_channel(&s.db, ch).await?.ok_or(AppError::NotFound)?;
    if channel.server_id != Some(server) || channel.kind != ChannelKind::Text {
        return Err(AppError::BadRequest(
            "pick one of this server's text channels".into(),
        ));
    }
    crate::access::require_open(&channel) // an archived one posts nothing (#303)
}
