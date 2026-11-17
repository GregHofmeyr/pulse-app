//! The F1 worker (#386): every [`TICK_EVERY`] (and at boot) it reads the calendar (cached; nothing at all while no
//! server has an F1 channel) and posts what's due in each server's F1 channel:
//!
//! - the weekend card from [`CARD_LEAD_HOURS`] before the first session until lights out (it waits for the field,
//!   so people have drivers to pick, until qualifying starts);
//! - the "lights out in 1 hour" line in the hour before lights out (one the server slept through is skipped);
//! - the results card, then the predictions' scores, once the results are in: they're only asked for from a while
//!   after lights out, and only until a server has its card, so the results are polled around the race's end and
//!   never otherwise.
//!
//! Each line goes out with its `f1_posts` row in one transaction, so a restart neither loses nor repeats one.
//!
//! The scores line mentions everyone who predicted and is still in the server (#403), as a plan's reminder does, so
//! it reaches their inbox and notifications the way a typed `@name` does.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{
    F1Card, F1Driver, F1LightsOut, F1ResultsCard, F1Score, F1ScoresCard, F1Weekend, F1WeekendCard,
    Message, f1_limits,
};
use sqlx::SqliteConnection;

use super::jolpica;
use crate::AppState;
use crate::badges::catalogue::stat;
use crate::db::{begin_write, now as now_stamp};
use crate::error::AppResult;
use crate::messages::system;

pub const TICK_EVERY: Duration = Duration::from_secs(60);
/// The weekend card goes this long before the first session.
pub const CARD_LEAD_HOURS: i64 = 12;

/// What's posted for a round, once per server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Post {
    Weekend,
    LightsOut,
    Results,
}

impl Post {
    fn as_str(self) -> &'static str {
        match self {
            Self::Weekend => "weekend",
            Self::LightsOut => "lights_out",
            Self::Results => "results",
        }
    }
}

/// A server and its F1 channel.
#[derive(Clone, Copy, Debug)]
struct Target {
    server: ServerId,
    channel: ChannelId,
}

/// Every live server's F1 channel (live text channels only).
async fn targets(s: &AppState) -> AppResult<Vec<Target>> {
    let rows: Vec<(String, String)> = sqlx::query_as(&format!(
        "SELECT f.server_id, f.channel_id FROM server_f1_channels f
         JOIN channels c ON c.id = f.channel_id AND c.deleted_at IS NULL AND c.archived_at IS NULL AND c.kind = 'text'
         WHERE f.server_id NOT IN {} ORDER BY f.server_id",
        crate::access::DELETED_SERVERS
    ))
    .fetch_all(&s.db)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(sv, ch)| {
            Some(Target {
                server: sv.parse().ok()?,
                channel: ch.parse().ok()?,
            })
        })
        .collect())
}

async fn posted(s: &AppState, t: Target, w: &F1Weekend, kind: Post) -> AppResult<bool> {
    let hit: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM f1_posts WHERE server_id = ? AND season = ? AND round = ? AND kind = ?",
    )
    .bind(t.server.to_string())
    .bind(i64::from(w.season))
    .bind(i64::from(w.round))
    .bind(kind.as_str())
    .fetch_optional(&s.db)
    .await?;
    Ok(hit.is_some())
}

/// The server's weekend card for the round, if it went out and is still there.
async fn weekend_message(
    db: &mut SqliteConnection,
    server: ServerId,
    w: &F1Weekend,
) -> AppResult<Option<MessageId>> {
    let id: Option<String> = sqlx::query_scalar(
        "SELECT f.message_id FROM f1_posts f JOIN messages m ON m.id = f.message_id AND m.deleted_at IS NULL
         WHERE f.server_id = ? AND f.season = ? AND f.round = ? AND f.kind = 'weekend'",
    )
    .bind(server.to_string())
    .bind(i64::from(w.season))
    .bind(i64::from(w.round))
    .fetch_optional(&mut *db)
    .await?;
    Ok(id.and_then(|i| i.parse().ok()))
}

/// Claim `kind` for the round in `tx`: false when it went out already.
async fn claim(tx: &mut SqliteConnection, t: Target, w: &F1Weekend, kind: Post) -> AppResult<bool> {
    Ok(sqlx::query(
        "INSERT OR IGNORE INTO f1_posts (server_id, season, round, kind, message_id, posted_at)
         VALUES (?, ?, ?, ?, NULL, ?)",
    )
    .bind(t.server.to_string())
    .bind(i64::from(w.season))
    .bind(i64::from(w.round))
    .bind(kind.as_str())
    .bind(now_stamp())
    .execute(&mut *tx)
    .await?
    .rows_affected()
        == 1)
}

async fn mark(
    tx: &mut SqliteConnection,
    t: Target,
    w: &F1Weekend,
    kind: Post,
    msg: MessageId,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE f1_posts SET message_id = ? WHERE server_id = ? AND season = ? AND round = ? AND kind = ?",
    )
    .bind(msg.to_string())
    .bind(t.server.to_string())
    .bind(i64::from(w.season))
    .bind(i64::from(w.round))
    .bind(kind.as_str())
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Post one line for the round, once. The card is made in the transaction (it may name the weekend card).
async fn post_once(
    s: &AppState,
    t: Target,
    w: &F1Weekend,
    kind: Post,
    make: impl FnOnce(Option<MessageId>) -> (String, F1Card),
) -> AppResult<Option<Message>> {
    // each line goes out in commit order with the chat's other messages (as a send)
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    if !crate::access::channel_is_live_in(&mut tx, t.channel).await?
        || !claim(&mut tx, t, w, kind).await?
    {
        return Ok(None);
    }
    let weekend = weekend_message(&mut tx, t.server, w).await?;
    let (text, card) = make(weekend);
    let msg = system::insert(&mut tx, t.channel, text, super::payload(&card)?).await?;
    mark(&mut tx, t, w, kind, msg.id).await?;
    tx.commit().await?;
    s.hub
        .publish(
            &s.db,
            Event::MessageCreated {
                message: msg.clone(),
                nonce: None,
            },
        )
        .await;
    Ok(Some(msg))
}

/// The results card, then the predictions' scores (when anyone predicted) and the season's table, in one go.
async fn post_results(
    s: &AppState,
    t: Target,
    w: &F1Weekend,
    card: &F1ResultsCard,
    podium: &[String],
) -> AppResult<bool> {
    let podium_drivers: Vec<F1Driver> = card
        .results
        .iter()
        .take(f1_limits::PODIUM)
        .map(|r| r.driver.clone())
        .collect();
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    if !crate::access::channel_is_live_in(&mut tx, t.channel).await?
        || !claim(&mut tx, t, w, Post::Results).await?
    {
        return Ok(false);
    }
    let results = system::insert(
        &mut tx,
        t.channel,
        super::results_text(card),
        super::payload(&F1Card::Results(card.clone()))?,
    )
    .await?;
    mark(&mut tx, t, w, Post::Results, results.id).await?;
    let weekend = weekend_message(&mut tx, t.server, w).await?;
    let mut scorers: Vec<UserId> = vec![];
    let mut scores_msg = None;
    if let Some(wid) = weekend {
        let field = super::weekend_card(&mut tx, wid)
            .await?
            .map(|(_, _, c)| c.drivers)
            .unwrap_or_default();
        let predictions = super::predictions_of(&mut tx, wid).await?;
        let mut scores: Vec<F1Score> = Vec::with_capacity(predictions.len());
        for p in predictions {
            let points = super::score(&p.picks, podium);
            sqlx::query(
                "UPDATE f1_predictions SET points = ? WHERE message_id = ? AND user_id = ?",
            )
            .bind(i64::from(points))
            .bind(wid.to_string())
            .bind(p.user_id.to_string())
            .execute(&mut *tx)
            .await?;
            scorers.push(p.user_id);
            scores.push(F1Score {
                user_id: p.user_id,
                picks: p.picks,
                points,
            });
        }
        if !scores.is_empty() {
            // highest first; a tie keeps the order they predicted in
            scores.sort_by_key(|a| std::cmp::Reverse(a.points));
            let season_table = super::season_table(&mut tx, t.server, w.season).await?;
            // Podium Prophet: the best season anyone's had in any server
            for row in &season_table {
                crate::badges::stats::set_max(
                    &mut tx,
                    row.user_id,
                    stat::F1_SEASON_POINTS,
                    i64::from(row.points),
                )
                .await?;
            }
            let users: Vec<UserId> = scores.iter().map(|x| x.user_id).collect();
            let names = super::display_names(&s.db, &users).await;
            let card = F1ScoresCard {
                season: w.season,
                round: w.round,
                race_name: w.race_name.clone(),
                podium: podium_drivers,
                drivers: field,
                scores,
                season_table,
                weekend_message_id: Some(wid),
            };
            let text = super::scores_text(&card, &names);
            let mut msg = system::insert(
                &mut tx,
                t.channel,
                text,
                super::payload(&F1Card::Scores(card))?,
            )
            .await?;
            // a typed mention each, for the predictors still in the server: the inbox and notifications take it
            let told = still_members(&mut tx, t.server, &users).await?;
            crate::messages::mentions::store(&mut tx, msg.id, &told, true).await?;
            msg.mentions = told;
            scores_msg = Some(msg);
        }
    }
    tx.commit().await?;
    for m in std::iter::once(results).chain(scores_msg) {
        s.hub
            .publish(
                &s.db,
                Event::MessageCreated {
                    message: m,
                    nonce: None,
                },
            )
            .await;
    }
    drop(_order);
    if let Some(wid) = weekend
        && !scorers.is_empty()
    {
        super::announce(s, t.channel, wid).await;
        crate::badges::engine::after(s, &scorers).await;
    }
    Ok(true)
}

/// Those of `users` still in `server`, in order.
async fn still_members(
    tx: &mut SqliteConnection,
    server: ServerId,
    users: &[UserId],
) -> AppResult<Vec<UserId>> {
    let mut out = Vec::with_capacity(users.len());
    for u in users {
        let member: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM server_members WHERE server_id = ? AND user_id = ?")
                .bind(server.to_string())
                .bind(u.to_string())
                .fetch_optional(&mut *tx)
                .await?;
        if member.is_some() && !out.contains(u) {
            out.push(*u);
        }
    }
    Ok(out)
}

fn t(at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Post everything due at `now`. Returns how many lines went out.
pub async fn tick(s: &AppState, now: DateTime<Utc>) -> AppResult<usize> {
    let targets = targets(s).await?;
    if targets.is_empty() {
        return Ok(0);
    }
    let schedule = jolpica::schedule(&s.db, &s.cfg.f1, now).await?;
    let mut sent = 0;
    for w in &schedule {
        let Some(race_at) = t(&w.race_at) else {
            continue;
        };
        // the weekend card
        if now >= jolpica::weekend_start(w) - TimeDelta::hours(CARD_LEAD_HOURS) && now < race_at {
            let mut waiting = vec![];
            for &tg in &targets {
                if !posted(s, tg, w, Post::Weekend).await? {
                    waiting.push(tg);
                }
            }
            if !waiting.is_empty() {
                let drivers = jolpica::field(&s.db, &s.cfg.f1, w, now).await?;
                // with no field to pick from yet, wait for one until qualifying
                if !drivers.is_empty() || now >= jolpica::qualifying_start(w) {
                    for tg in waiting {
                        let card = F1WeekendCard {
                            weekend: w.clone(),
                            drivers: drivers.clone(),
                            predictions: vec![],
                        };
                        let text = super::weekend_text(w);
                        if post_once(s, tg, w, Post::Weekend, |_| (text, F1Card::Weekend(card)))
                            .await?
                            .is_some()
                        {
                            sent += 1;
                        }
                    }
                }
            }
        }
        // lights out in an hour
        if now >= race_at - TimeDelta::minutes(f1_limits::LIGHTS_OUT_MINUTES) && now < race_at {
            for &tg in &targets {
                if posted(s, tg, w, Post::LightsOut).await? {
                    continue;
                }
                let make = |weekend_message_id| {
                    (
                        super::lights_out_text(&w.race_name),
                        F1Card::LightsOut(F1LightsOut {
                            season: w.season,
                            round: w.round,
                            race_name: w.race_name.clone(),
                            race_at: w.race_at.clone(),
                            weekend_message_id,
                        }),
                    )
                };
                if post_once(s, tg, w, Post::LightsOut, make).await?.is_some() {
                    sent += 1;
                }
            }
        }
        // the results, around the race's end only
        if now >= race_at + TimeDelta::minutes(jolpica::RESULTS_AFTER_MINUTES)
            && now < race_at + TimeDelta::hours(jolpica::RESULTS_GIVE_UP_HOURS)
        {
            let mut waiting = vec![];
            for &tg in &targets {
                if !posted(s, tg, w, Post::Results).await? {
                    waiting.push(tg);
                }
            }
            if waiting.is_empty() {
                continue;
            }
            let Some((card, podium)) = jolpica::results_card(&s.db, &s.cfg.f1, w, now).await?
            else {
                continue;
            };
            for tg in waiting {
                if post_results(s, tg, w, &card, &podium).await? {
                    sent += 1;
                }
            }
        }
    }
    Ok(sent)
}

/// Run [`tick`] every [`TICK_EVERY`] (the first at once: the boot pass). Call after binding the listener.
pub fn spawn(s: AppState) {
    tokio::spawn(async move {
        let mut every = tokio::time::interval(TICK_EVERY);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            every.tick().await;
            match tick(&s, Utc::now()).await {
                Ok(0) => {}
                Ok(n) => tracing::debug!(n, "F1 lines posted"),
                Err(e) => tracing::warn!(error = %e, "F1 tick failed"),
            }
        }
    });
}
