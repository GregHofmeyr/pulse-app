//! F1's routes (#386): each server's F1 channel and countdown, and podium predictions on a weekend card.

use axum::extract::{Path, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use chrono::Utc;
use pulse_protocol::ids::{ChannelId, MessageId, ServerId};
use pulse_protocol::rest::{
    ChannelKind, F1Prediction, F1PredictionRequest, F1Status, SetF1ChannelRequest, f1_limits,
};

use super::jolpica;
use crate::AppState;
use crate::access::{channel_for, load_channel};
use crate::auth::AuthUser;
use crate::db::{begin_write, now, stamp};
use crate::error::{AppError, AppResult};
use crate::servers::routes::{is_server_member, require_member};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/servers/{id}/f1", get(status))
        .route("/servers/{id}/f1-channel", put(set_channel))
        .route("/messages/{id}/f1/prediction", put(predict))
}

async fn channel_of(s: &AppState, server: ServerId) -> AppResult<Option<ChannelId>> {
    let id: Option<String> = sqlx::query_scalar(
        "SELECT f.channel_id FROM server_f1_channels f
         JOIN channels c ON c.id = f.channel_id AND c.deleted_at IS NULL
         WHERE f.server_id = ?",
    )
    .bind(server.to_string())
    .fetch_optional(&s.db)
    .await?;
    Ok(id.and_then(|i| i.parse().ok()))
}

/// The server's F1 status: its channel, and (only when it has one, so nothing is fetched for servers without) the
/// next race weekend and the season's prediction table.
async fn current(s: &AppState, server: ServerId) -> AppResult<F1Status> {
    let channel_id = channel_of(s, server).await?;
    if channel_id.is_none() {
        return Ok(F1Status {
            channel_id,
            next: None,
            season_table: vec![],
        });
    }
    let at = Utc::now();
    let schedule = jolpica::schedule(&s.db, &s.cfg.f1, at).await?;
    let next = jolpica::next_weekend(&schedule, at).cloned();
    let season = next
        .as_ref()
        .map(|w| w.season)
        .or_else(|| schedule.last().map(|w| w.season));
    let season_table = match season {
        Some(season) => {
            let mut conn = s.db.acquire().await?;
            super::season_table(&mut conn, server, season).await?
        }
        None => vec![],
    };
    Ok(F1Status {
        channel_id,
        next,
        season_table,
    })
}

/// `GET /servers/{id}/f1`: for its members.
async fn status(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
) -> AppResult<Json<F1Status>> {
    require_member(&s.db, id, me).await?;
    Ok(Json(current(&s, id).await?))
}

/// `PUT /servers/{id}/f1-channel`: the server's owner picks one of its text channels, or none.
async fn set_channel(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ServerId>,
    Json(req): Json<SetF1ChannelRequest>,
) -> AppResult<Json<F1Status>> {
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
            "Only the server's owner can pick its F1 channel.".into(),
        ));
    }
    match req.channel_id {
        None => {
            sqlx::query("DELETE FROM server_f1_channels WHERE server_id = ?")
                .bind(id.to_string())
                .execute(&s.db)
                .await?;
        }
        Some(ch) => {
            let channel = load_channel(&s.db, ch).await?.ok_or(AppError::NotFound)?;
            if channel.server_id != Some(id) || channel.kind != ChannelKind::Text {
                return Err(AppError::BadRequest(
                    "pick one of this server's text channels".into(),
                ));
            }
            crate::access::require_open(&channel)?; // an archived one posts nothing (#303)
            sqlx::query(
                "INSERT INTO server_f1_channels (server_id, channel_id) VALUES (?, ?)
                 ON CONFLICT (server_id) DO UPDATE SET channel_id = excluded.channel_id",
            )
            .bind(id.to_string())
            .bind(ch.to_string())
            .execute(&s.db)
            .await?;
        }
    }
    Ok(Json(current(&s, id).await?))
}

/// Checks a prediction against the card's field: three different drivers, each on the card.
pub fn clean_picks(picks: &[String], field: &[String]) -> AppResult<Vec<String>> {
    let picks: Vec<String> = picks.iter().map(|p| p.trim().to_string()).collect();
    let distinct = picks
        .iter()
        .enumerate()
        .all(|(i, p)| !picks[..i].contains(p));
    if picks.len() != f1_limits::PODIUM || !distinct {
        return Err(AppError::BadRequest(
            "pick three different drivers for P1, P2 and P3".into(),
        ));
    }
    if picks.iter().any(|p| !field.contains(p)) {
        return Err(AppError::BadRequest(
            "pick drivers from this race's field".into(),
        ));
    }
    Ok(picks)
}

/// `PUT /messages/{id}/f1/prediction`: your podium for a weekend card, replacing any before (`None` takes it back),
/// until lights out. The channel gets everyone's as `F1PredictionsChanged`.
async fn predict(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
    Json(req): Json<F1PredictionRequest>,
) -> AppResult<Json<Vec<F1Prediction>>> {
    crate::media::writes_ok(&s.cfg)?;
    let mut conn = s.db.acquire().await?;
    let (channel, server, card) = super::weekend_card(&mut conn, id)
        .await?
        .ok_or(AppError::NotFound)?;
    drop(conn);
    let ch = channel_for(&s.db, me, channel).await?;
    if !is_server_member(&s.db, server, me).await? {
        return Err(AppError::Forbidden);
    }
    crate::access::require_open(&ch)?; // archived (#303): read-only
    if !s.limits.poll_vote_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(Some(
            "that's a lot of picks; try again in a moment".into(),
        )));
    }
    let closed = || AppError::Conflict("predictions closed at lights out".into());
    if stamp(Utc::now()) >= card.weekend.race_at {
        return Err(closed());
    }
    let picks = match &req.picks {
        Some(p) => {
            let field: Vec<String> = card.drivers.iter().map(|d| d.id.clone()).collect();
            Some(clean_picks(p, &field)?)
        }
        None => None,
    };
    let mut tx = begin_write(&s.db).await?;
    crate::access::channel_live_in(&mut tx, channel).await?;
    crate::access::open_in(&mut tx, channel).await?; // archived since the check
    // read again under the write lock: deleted since, or lights out came while we checked
    if super::weekend_card(&mut tx, id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    if stamp(Utc::now()) >= card.weekend.race_at {
        return Err(closed());
    }
    match picks {
        Some(picks) => {
            // a change keeps its place in the list
            sqlx::query(
                "INSERT INTO f1_predictions (message_id, user_id, picks, at) VALUES (?, ?, ?, ?)
                 ON CONFLICT (message_id, user_id) DO UPDATE SET picks = excluded.picks, at = excluded.at",
            )
            .bind(id.to_string())
            .bind(me.to_string())
            .bind(serde_json::to_string(&picks).map_err(anyhow::Error::from)?)
            .bind(now())
            .execute(&mut *tx)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM f1_predictions WHERE message_id = ? AND user_id = ?")
                .bind(id.to_string())
                .bind(me.to_string())
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    super::announce(&s, channel, id).await;
    let mut conn = s.db.acquire().await?;
    Ok(Json(super::predictions_of(&mut conn, id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(p: &[&str]) -> Vec<String> {
        p.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_prediction_is_three_different_drivers_from_the_field() {
        let field = v(&["a", "b", "c", "d"]);
        assert_eq!(
            clean_picks(&v(&[" a", "b ", "c"]), &field).unwrap(),
            v(&["a", "b", "c"])
        );
        assert!(clean_picks(&v(&["a", "b"]), &field).is_err());
        assert!(clean_picks(&v(&["a", "b", "c", "d"]), &field).is_err());
        assert!(clean_picks(&v(&["a", "a", "c"]), &field).is_err());
        assert!(clean_picks(&v(&["a", "b", "x"]), &field).is_err());
    }
}
