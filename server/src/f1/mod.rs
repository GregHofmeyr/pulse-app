//! F1 race weekends (#386), in each server's F1 channel (its owner picks one in Server settings; none by default):
//!
//! - a countdown banner to the next lights out, which clients draw from `GET /servers/{id}/f1`;
//! - the "race weekend" card as the weekend starts (the race, the circuit, every session's time), on which people
//!   predict the podium until lights out;
//! - a "lights out in 1 hour" line;
//! - after the race, a podium and results card (the top 10, the fastest lap, the standings after the round), then
//!   the predictions' scores (3 points a pick in its exact place, 1 a pick on the podium elsewhere) with the season's
//!   table, which feeds the Podium Prophet badge.
//!
//! Every card is a system line whose payload is an [`F1Card`]; a weekend card's predictions live in
//! `f1_predictions` and are attached when read ([`attach`]). Everyone sees the same predictions, so a change goes out
//! whole (`F1PredictionsChanged`). The data comes from Jolpica-F1 through [`jolpica`], cached hard; only the
//! [`worker`] and the banner's route ask it, and nothing is asked while no server has an F1 channel.

pub mod jolpica;
pub mod routes;
pub mod worker;

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{
    F1Card, F1Prediction, F1ResultsCard, F1ScoresCard, F1SeasonRow, F1Weekend, F1WeekendCard,
    Message, MessageKind, f1_limits,
};
use sqlx::{SqliteConnection, SqlitePool};

use crate::AppState;
use crate::error::AppResult;

pub use routes::router;

/// The payload `type` of every F1 card (the card's own `kind` tells them apart).
pub const F1_CARD: &str = "f1";

/// A card's stored payload.
pub fn payload(card: &F1Card) -> AppResult<serde_json::Value> {
    let mut v = serde_json::to_value(card).map_err(anyhow::Error::from)?;
    v["type"] = F1_CARD.into();
    Ok(v)
}

/// The F1 card in a stored payload, if it is one.
pub fn card(payload: &str) -> Option<F1Card> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    if v.get("type")?.as_str()? != F1_CARD {
        return None;
    }
    serde_json::from_value(v).ok()
}

/// A time as the fallback text says it: UTC, the one time everyone reads the same way.
fn utc_text(at: &str) -> String {
    DateTime::parse_from_rfc3339(at)
        .map(|t| {
            t.with_timezone(&Utc)
                .format("%a %-d %b %Y, %H:%M UTC")
                .to_string()
        })
        .unwrap_or_else(|_| at.to_string())
}

/// The weekend card as text, for clients that don't know F1 cards (and previews, search).
pub fn weekend_text(w: &F1Weekend) -> String {
    format!(
        "🏁 Race weekend: {} (round {}), {}, {}, {}. Lights out {}.",
        w.race_name,
        w.round,
        w.circuit,
        w.locality,
        w.country,
        utc_text(&w.race_at)
    )
}

pub fn lights_out_text(race_name: &str) -> String {
    format!("🚦 Lights out in 1 hour: {race_name}")
}

pub fn results_text(card: &F1ResultsCard) -> String {
    let podium: Vec<String> = card
        .results
        .iter()
        .take(f1_limits::PODIUM)
        .map(|r| format!("{}. {}", r.position, r.driver.name))
        .collect();
    format!("🏆 {}: {}", card.race_name, podium.join(", "))
}

/// The scores card as text; `names` are the scorers' display names in the card's order.
pub fn scores_text(card: &F1ScoresCard, names: &[String]) -> String {
    let each: Vec<String> = card
        .scores
        .iter()
        .zip(names)
        .map(|(s, n)| format!("{n} {}", s.points))
        .collect();
    format!(
        "🔮 Podium predictions, {}: {}",
        card.race_name,
        each.join(", ")
    )
}

/// Points for one prediction against the real podium: 3 a pick in its exact place, 1 a pick on the podium
/// elsewhere.
pub fn score(picks: &[String], podium: &[String]) -> u32 {
    picks
        .iter()
        .take(f1_limits::PODIUM)
        .enumerate()
        .map(|(i, p)| {
            if podium.get(i) == Some(p) {
                f1_limits::EXACT_POINTS
            } else if podium.iter().take(f1_limits::PODIUM).any(|d| d == p) {
                f1_limits::PODIUM_POINTS
            } else {
                0
            }
        })
        .sum()
}

type PredictionRow = (String, String, String, Option<i64>);

fn prediction_from(r: &PredictionRow) -> Option<F1Prediction> {
    Some(F1Prediction {
        user_id: r.1.parse().ok()?,
        picks: serde_json::from_str(&r.2).ok()?,
        points: r.3.and_then(|p| u32::try_from(p).ok()),
    })
}

/// Each card's predictions, in the order they were first made.
async fn predictions_for(
    db: &mut SqliteConnection,
    ids: &[String],
) -> AppResult<HashMap<String, Vec<F1Prediction>>> {
    let mut out: HashMap<String, Vec<F1Prediction>> = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let mut q = sqlx::QueryBuilder::new(
        "SELECT message_id, user_id, picks, points FROM f1_predictions WHERE message_id IN (",
    );
    let mut list = q.separated(", ");
    for id in ids {
        list.push_bind(id);
    }
    q.push(") ORDER BY rowid");
    let rows: Vec<PredictionRow> = q.build_query_as().fetch_all(&mut *db).await?;
    for r in &rows {
        if let Some(p) = prediction_from(r) {
            out.entry(r.0.clone()).or_default().push(p);
        }
    }
    Ok(out)
}

/// One card's predictions.
pub(crate) async fn predictions_of(
    db: &mut SqliteConnection,
    id: MessageId,
) -> AppResult<Vec<F1Prediction>> {
    Ok(predictions_for(db, &[id.to_string()])
        .await?
        .remove(&id.to_string())
        .unwrap_or_default())
}

/// Fill each weekend card's predictions.
pub(crate) async fn attach(db: &SqlitePool, page: &mut [Message]) -> AppResult<()> {
    let ids: Vec<String> = page
        .iter()
        .filter(|m| {
            !m.deleted
                && m.kind == MessageKind::System
                && matches!(m.f1.as_deref(), Some(F1Card::Weekend(_)))
        })
        .map(|m| m.id.to_string())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let mut conn = db.acquire().await?;
    let mut all = predictions_for(&mut conn, &ids).await?;
    for m in page.iter_mut() {
        if let Some(F1Card::Weekend(c)) = m.f1.as_deref_mut() {
            c.predictions = all.remove(&m.id.to_string()).unwrap_or_default();
        }
    }
    Ok(())
}

/// A live weekend card: (its channel, its server, the card).
pub(crate) async fn weekend_card(
    db: &mut SqliteConnection,
    id: MessageId,
) -> AppResult<Option<(ChannelId, ServerId, F1WeekendCard)>> {
    let row: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT m.channel_id, c.server_id, m.system_payload FROM messages m JOIN channels c ON c.id = m.channel_id
         WHERE m.id = ? AND m.kind = 'system' AND m.deleted_at IS NULL",
    )
    .bind(id.to_string())
    .fetch_optional(&mut *db)
    .await?;
    let Some((channel, Some(server), Some(stored))) = row else {
        return Ok(None);
    };
    let Some(F1Card::Weekend(c)) = card(&stored) else {
        return Ok(None);
    };
    Ok(Some((
        channel.parse().map_err(anyhow::Error::from)?,
        server.parse().map_err(anyhow::Error::from)?,
        c,
    )))
}

/// The season's prediction table in a server: the points over its scored weekend cards, highest first (then the
/// fewer races, then who got there first).
pub(crate) async fn season_table(
    db: &mut SqliteConnection,
    server: ServerId,
    season: u16,
) -> AppResult<Vec<F1SeasonRow>> {
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT p.user_id, SUM(p.points), COUNT(*) FROM f1_predictions p
         JOIN f1_posts f ON f.message_id = p.message_id AND f.kind = 'weekend'
         JOIN server_members sm ON sm.server_id = f.server_id AND sm.user_id = p.user_id
         WHERE f.server_id = ? AND f.season = ? AND p.points IS NOT NULL
         GROUP BY p.user_id ORDER BY SUM(p.points) DESC, COUNT(*), MIN(p.rowid)",
    )
    .bind(server.to_string())
    .bind(i64::from(season))
    .fetch_all(&mut *db)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(u, points, races)| {
            Some(F1SeasonRow {
                user_id: u.parse().ok()?,
                points: u32::try_from(points).ok()?,
                races: u32::try_from(races).ok()?,
            })
        })
        .collect())
}

/// Tell the channel a card's predictions as they are now (after a commit).
pub(crate) async fn announce(s: &AppState, channel_id: ChannelId, message_id: MessageId) {
    let predictions = match s.db.acquire().await {
        Ok(mut conn) => predictions_of(&mut conn, message_id).await,
        Err(e) => Err(e.into()),
    };
    match predictions {
        Ok(predictions) => {
            s.hub
                .publish(
                    &s.db,
                    Event::F1PredictionsChanged {
                        channel_id,
                        message_id,
                        predictions,
                    },
                )
                .await;
        }
        Err(e) => tracing::warn!(error = %e, "couldn't read F1 predictions to announce them"),
    }
}

/// Display names, as cards print them.
pub(crate) async fn display_names(db: &SqlitePool, users: &[UserId]) -> Vec<String> {
    let mut out = Vec::with_capacity(users.len());
    for u in users {
        let n = sqlx::query_scalar::<_, String>(
            "SELECT CASE WHEN display_name != '' THEN display_name ELSE username END FROM users WHERE id = ?",
        )
        .bind(u.to_string())
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "Someone".into());
        out.push(n);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_protocol::rest::{
        F1Driver, F1LightsOut, F1ResultRow, F1Score, F1Session, F1SessionKind,
    };

    fn d(id: &str) -> F1Driver {
        F1Driver {
            id: id.into(),
            code: Some(id.to_uppercase()),
            name: format!("Driver {id}"),
            team: None,
        }
    }

    fn picks(p: &[&str]) -> Vec<String> {
        p.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn three_for_the_exact_place_one_for_the_podium() {
        let podium = picks(&["a", "b", "c"]);
        assert_eq!(score(&picks(&["a", "b", "c"]), &podium), 9);
        assert_eq!(score(&picks(&["b", "a", "c"]), &podium), 1 + 1 + 3);
        assert_eq!(score(&picks(&["c", "a", "b"]), &podium), 3);
        assert_eq!(score(&picks(&["a", "x", "y"]), &podium), 3);
        assert_eq!(score(&picks(&["x", "y", "z"]), &podium), 0);
        // a podium short of three (a red-flagged race, say) still scores what there is
        assert_eq!(score(&picks(&["a", "b", "c"]), &picks(&["a", "b"])), 6);
    }

    #[test]
    fn every_card_round_trips_through_its_payload() {
        let weekend = F1Weekend {
            season: 2030,
            round: 1,
            race_name: "Desert Grand Prix".into(),
            circuit: "Desert Park".into(),
            locality: "Dune City".into(),
            country: "Australia".into(),
            sessions: vec![F1Session {
                kind: F1SessionKind::Race,
                starts_at: "2030-03-08T04:00:00.000Z".into(),
            }],
            race_at: "2030-03-08T04:00:00.000Z".into(),
        };
        let cards = [
            F1Card::Weekend(F1WeekendCard {
                weekend: weekend.clone(),
                drivers: vec![d("a")],
                predictions: vec![],
            }),
            F1Card::LightsOut(F1LightsOut {
                season: 2030,
                round: 1,
                race_name: "Desert Grand Prix".into(),
                race_at: weekend.race_at.clone(),
                weekend_message_id: Some(MessageId::new()),
            }),
            F1Card::Results(F1ResultsCard {
                season: 2030,
                round: 1,
                race_name: "Desert Grand Prix".into(),
                country: "Australia".into(),
                results: vec![F1ResultRow {
                    position: 1,
                    driver: d("a"),
                    time: "1:30:00.000".into(),
                    points: 25.0,
                }],
                fastest_lap: None,
                standings: vec![],
            }),
            F1Card::Scores(F1ScoresCard {
                season: 2030,
                round: 1,
                race_name: "Desert Grand Prix".into(),
                podium: vec![d("a")],
                drivers: vec![d("a")],
                scores: vec![F1Score {
                    user_id: UserId::new(),
                    picks: picks(&["a", "b", "c"]),
                    points: 3,
                }],
                season_table: vec![],
                weekend_message_id: None,
            }),
        ];
        for c in cards {
            let stored = payload(&c).unwrap().to_string();
            assert_eq!(card(&stored), Some(c));
        }
        assert_eq!(card(r#"{"type":"pin"}"#), None);
        assert_eq!(card("nope"), None);
        assert_eq!(
            weekend_text(&weekend),
            "🏁 Race weekend: Desert Grand Prix (round 1), Desert Park, Dune City, Australia. Lights out Fri 8 Mar \
             2030, 04:00 UTC."
        );
    }

    #[test]
    fn the_fallback_texts() {
        assert_eq!(
            lights_out_text("Desert GP"),
            "🚦 Lights out in 1 hour: Desert GP"
        );
        let r = F1ResultsCard {
            season: 2030,
            round: 1,
            race_name: "Desert GP".into(),
            country: "X".into(),
            results: ["a", "b", "c", "e"]
                .iter()
                .enumerate()
                .map(|(i, id)| F1ResultRow {
                    position: i as u16 + 1,
                    driver: d(id),
                    time: String::new(),
                    points: 0.0,
                })
                .collect(),
            fastest_lap: None,
            standings: vec![],
        };
        assert_eq!(
            results_text(&r),
            "🏆 Desert GP: 1. Driver a, 2. Driver b, 3. Driver c"
        );
        let s = F1ScoresCard {
            season: 2030,
            round: 1,
            race_name: "Desert GP".into(),
            podium: vec![],
            drivers: vec![],
            scores: vec![
                F1Score {
                    user_id: UserId::new(),
                    picks: vec![],
                    points: 4,
                },
                F1Score {
                    user_id: UserId::new(),
                    picks: vec![],
                    points: 1,
                },
            ],
            season_table: vec![],
            weekend_message_id: None,
        };
        assert_eq!(
            scores_text(&s, &["alex".into(), "sam".into()]),
            "🔮 Podium predictions, Desert GP: alex 4, sam 1"
        );
    }
}
