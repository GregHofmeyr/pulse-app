//! Each league's teams this season (#385), for the follow search: read from API-Sports when someone first searches a
//! league, then kept for [`crate::config::SportsConfig::teams_every`] (a request a league a week). A failed read is
//! tried again no sooner than [`RETRY`], so typing in the search can't spend the day's allowance.

use std::time::Duration;

use pulse_protocol::rest::{SportsLeague, SportsTeam};

use super::{due, mark, season_for, sport_of};
use crate::AppState;
use crate::error::AppResult;

/// The shortest gap between tries after a failed team list read.
pub const RETRY: Duration = Duration::from_secs(1800);

fn fetched_key(league: SportsLeague) -> String {
    format!("teams_at:{}", league.key())
}

fn season_key(league: SportsLeague) -> String {
    format!("teams_season:{}", league.key())
}

fn tried_key(league: SportsLeague) -> String {
    format!("teams_tried:{}", league.key())
}

/// The cached teams of `league`, by name.
pub async fn cached(s: &AppState, league: SportsLeague) -> AppResult<Vec<SportsTeam>> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT team_id, name FROM sports_teams WHERE league = ? ORDER BY name COLLATE NOCASE",
    )
    .bind(league.key())
    .fetch_all(&s.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name)| SportsTeam {
            league,
            team_id: id.clamp(0, u32::MAX as i64) as u32,
            name,
        })
        .collect())
}

/// `league`'s teams, read first when the list is missing, a week old or from last season (and a read is allowed).
pub async fn list(s: &AppState, league: SportsLeague) -> AppResult<Vec<SportsTeam>> {
    if s.sports.enabled() {
        refresh(s, league).await?;
    }
    cached(s, league).await
}

async fn refresh(s: &AppState, league: SportsLeague) -> AppResult<()> {
    let season = season_for(chrono::Utc::now().date_naive());
    let stale = || async {
        due(&s.db, &fetched_key(league), s.cfg.sports.teams_every).await
            || super::state_get(&s.db, &season_key(league)).await != Some(season.to_string())
    };
    if !stale().await {
        return Ok(());
    }
    let _one = s.sports.teams_lock().lock().await;
    // someone else's read may have landed while this one waited
    if !stale().await || !due(&s.db, &tried_key(league), RETRY).await {
        return Ok(());
    }
    mark(&s.db, &tried_key(league)).await?;
    let got = match s
        .sports
        .api()
        .teams(&s.db, sport_of(league), super::league_id(league), season)
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::debug!(league = league.key(), error = ?e, "sports teams not read");
            return Ok(());
        }
    };
    if got.is_empty() {
        // a season not filed yet, or not on the plan: keep what's there
        return Ok(());
    }
    let mut tx = s.db.begin().await?;
    sqlx::query("DELETE FROM sports_teams WHERE league = ?")
        .bind(league.key())
        .execute(&mut *tx)
        .await?;
    for t in got {
        sqlx::query("INSERT OR REPLACE INTO sports_teams (league, team_id, name) VALUES (?, ?, ?)")
            .bind(league.key())
            .bind(t.id)
            .bind(t.name.trim())
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    mark(&s.db, &fetched_key(league)).await?;
    super::state_set(&s.db, &season_key(league), &season.to_string()).await?;
    Ok(())
}
