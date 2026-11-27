//! The picker wheel (#158): saved wheels (a server's, or one person's "Just me" wheels) and spins. A spin is a
//! system line whose payload carries the whole spin; `wheel_spins` keeps each one so Spin again and Remove winner
//! & spin run from the stored spin. The server draws the winner and the angle; clients only animate to them.

pub mod routes;

use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{Wheel, WheelScope, wheel_limits, wheel_palettes};
use rand::RngExt;
use sqlx::SqlitePool;

use crate::error::{AppError, AppResult};

type Row = (
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);

const COLS: &str = "id, server_id, owner_id, name, options, updated_at, weights, palette";

fn from_row(
    (id, server_id, owner_id, name, options, updated_at, weights, palette): Row,
) -> AppResult<Wheel> {
    let server_id = server_id
        .map(|s| s.parse::<ServerId>())
        .transpose()
        .map_err(anyhow::Error::from)?;
    let owner_id = owner_id
        .map(|s| s.parse::<UserId>())
        .transpose()
        .map_err(anyhow::Error::from)?;
    Ok(Wheel {
        id,
        scope: if owner_id.is_some() {
            WheelScope::Personal
        } else {
            WheelScope::Server
        },
        server_id,
        owner_id,
        name,
        options: parse_options(&options)?,
        updated_at,
        weights: parse_weights(weights.as_deref())?,
        palette,
    })
}

/// Stored weights (a JSON array), or none.
pub fn parse_weights(stored: Option<&str>) -> AppResult<Option<Vec<u32>>> {
    Ok(stored
        .map(serde_json::from_str)
        .transpose()
        .map_err(anyhow::Error::from)?)
}

/// Weights as stored: none for "all alike".
pub fn stored_weights(weights: &Option<Vec<u32>>) -> AppResult<Option<String>> {
    Ok(weights
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(anyhow::Error::from)?)
}

/// Weights for `n` options (#259): one each, 1 to [`wheel_limits::MAX_WEIGHT`]. All 1 (or none) is `None`: an
/// unweighted wheel, as older clients draw it.
pub fn clean_weights(raw: Option<Vec<u32>>, n: usize) -> AppResult<Option<Vec<u32>>> {
    let Some(w) = raw else { return Ok(None) };
    if w.len() != n {
        return Err(AppError::BadRequest(
            "a wheel's weights go one per option".into(),
        ));
    }
    if w.iter().any(|&x| x == 0 || x > wheel_limits::MAX_WEIGHT) {
        return Err(AppError::BadRequest(format!(
            "an option's weight is 1 to {}",
            wheel_limits::MAX_WEIGHT
        )));
    }
    Ok((!w.iter().all(|&x| x == 1)).then_some(w))
}

/// A palette's name (#259), or `None` for the neon spectrum.
pub fn clean_palette(raw: Option<String>) -> AppResult<Option<String>> {
    match raw.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(wheel_palettes::NEON) => Ok(None),
        Some(p) if wheel_palettes::ALL.contains(&p) => Ok(Some(p.to_string())),
        Some(_) => Err(AppError::BadRequest(
            "that isn't one of the palettes".into(),
        )),
    }
}

/// A stored JSON array of options.
pub fn parse_options(stored: &str) -> AppResult<Vec<String>> {
    Ok(serde_json::from_str(stored).map_err(anyhow::Error::from)?)
}

pub async fn load(db: &SqlitePool, id: &str) -> AppResult<Option<Wheel>> {
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM wheels WHERE id = ?"
    )))
    .bind(id)
    .fetch_optional(db)
    .await?;
    row.map(from_row).transpose()
}

/// A server's wheels, by name.
pub async fn for_server(db: &SqlitePool, server: ServerId) -> AppResult<Vec<Wheel>> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM wheels WHERE server_id = ? ORDER BY name COLLATE NOCASE, id"
    )))
    .bind(server.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter().map(from_row).collect()
}

/// One person's Just me wheels, by name.
pub async fn for_owner(db: &SqlitePool, owner: UserId) -> AppResult<Vec<Wheel>> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM wheels WHERE owner_id = ? ORDER BY name COLLATE NOCASE, id"
    )))
    .bind(owner.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter().map(from_row).collect()
}

/// A saved wheel's name, trimmed: 1 to 40 characters on one line.
pub fn clean_name(raw: &str) -> AppResult<String> {
    let name = raw.trim();
    let n = name.chars().count();
    if n == 0 || n > wheel_limits::NAME_CHARS || name.chars().any(char::is_control) {
        return Err(AppError::BadRequest(format!(
            "wheel names are 1-{} characters",
            wheel_limits::NAME_CHARS
        )));
    }
    Ok(name.to_string())
}

/// Options, each trimmed: none blank, each up to 60 characters on one line, at most 100 of them. A saved wheel may
/// hold fewer than two while it's being filled; a spin needs two ([`spinnable`]).
pub fn clean_options(raw: Vec<String>) -> AppResult<Vec<String>> {
    if raw.len() > wheel_limits::MAX_OPTIONS {
        return Err(AppError::BadRequest(format!(
            "a wheel has at most {} options",
            wheel_limits::MAX_OPTIONS
        )));
    }
    raw.into_iter()
        .map(|o| {
            let o = o.trim();
            if o.is_empty() {
                return Err(AppError::BadRequest("options can't be blank".into()));
            }
            if o.chars().count() > wheel_limits::OPTION_CHARS || o.chars().any(char::is_control) {
                return Err(AppError::BadRequest(format!(
                    "options are up to {} characters on one line",
                    wheel_limits::OPTION_CHARS
                )));
            }
            Ok(o.to_string())
        })
        .collect()
}

/// Enough options to spin.
pub fn spinnable(options: &[String]) -> AppResult<()> {
    if options.len() < wheel_limits::MIN_OPTIONS {
        return Err(AppError::BadRequest(
            "a wheel needs at least 2 options to spin".into(),
        ));
    }
    Ok(())
}

/// The server's draw for a wheel of `n` slices: the winner, uniformly, and the wheel's final clockwise rotation
/// (degrees in [0, 360)) that puts the pointer 10-90 % of the way across the winner's slice, never on an edge.
pub fn draw(n: usize, rng: &mut impl RngExt) -> (usize, f64) {
    let winner = rng.random_range(0..n);
    let seg = 360.0 / n as f64;
    let at = seg * (winner as f64 + rng.random_range(0.1..0.9));
    let angle = (360.0 - at).rem_euclid(360.0);
    (winner, angle)
}

/// [`draw`] for a weighted wheel (#259): option `i` wins with probability `weights[i]` / their sum (one whole-number
/// draw over the sum, so it is exact), and the angle lands 10-90 % of the way across its slice, which covers the
/// same share of the turn. No weights: [`draw`].
pub fn draw_weighted(n: usize, weights: Option<&[u32]>, rng: &mut impl RngExt) -> (usize, f64) {
    let Some(w) = weights.filter(|w| w.len() == n) else {
        return draw(n, rng);
    };
    let total: u64 = w.iter().map(|&x| u64::from(x)).sum();
    if total == 0 {
        return draw(n, rng);
    }
    let pick = rng.random_range(0..total);
    let (mut before, mut winner) = (0u64, n - 1);
    for (i, &x) in w.iter().enumerate() {
        if pick < before + u64::from(x) {
            winner = i;
            break;
        }
        before += u64::from(x);
    }
    let start = 360.0 * before as f64 / total as f64;
    let span = 360.0 * f64::from(w[winner]) / total as f64;
    let at = start + span * rng.random_range(0.1..0.9);
    (winner, (360.0 - at).rem_euclid(360.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_protocol::rest::wheel_winner_at;

    #[test]
    fn a_draw_lands_inside_its_winner() {
        let mut rng = rand::rng();
        for n in 2..=100 {
            for _ in 0..50 {
                let (winner, angle) = draw(n, &mut rng);
                assert!(winner < n);
                assert!((0.0..360.0).contains(&angle), "{angle}");
                assert_eq!(wheel_winner_at(angle, n), winner, "n={n} angle={angle}");
                // clear of the slice's edges by a tenth of a slice
                let seg = 360.0 / n as f64;
                let off = ((360.0 - angle).rem_euclid(360.0) - seg * winner as f64) / seg;
                assert!((0.099..=0.901).contains(&off), "n={n} off={off}");
            }
        }
    }

    #[test]
    fn every_option_can_win() {
        let mut rng = rand::rng();
        let mut seen = [false; 5];
        for _ in 0..500 {
            seen[draw(5, &mut rng).0] = true;
        }
        assert_eq!(seen, [true; 5]);
    }

    /// #259: a weighted draw is fair to its weights (a seeded run of 40 000 draws stays within 1.5 % of each
    /// option's share) and lands inside its winner's (weighted) slice; missing weights draw evenly.
    #[test]
    fn a_weighted_draw_wins_in_proportion_and_lands_in_its_slice() {
        use pulse_protocol::rest::wheel_winner_at_weighted;
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(259);
        let weights = [1u32, 2, 3, 4];
        let mut wins = [0u32; 4];
        let runs = 40_000;
        for _ in 0..runs {
            let (winner, angle) = draw_weighted(4, Some(&weights), &mut rng);
            assert!((0.0..360.0).contains(&angle));
            assert_eq!(
                wheel_winner_at_weighted(angle, &weights),
                winner,
                "angle={angle}"
            );
            wins[winner] += 1;
        }
        for (i, &w) in weights.iter().enumerate() {
            let share = f64::from(wins[i]) / f64::from(runs);
            let want = f64::from(w) / 10.0;
            assert!(
                (share - want).abs() < 0.015,
                "option {i}: {share} vs {want}"
            );
        }
        // a heavy option against a light one
        let mut heavy = 0;
        for _ in 0..10_000 {
            if draw_weighted(2, Some(&[1, 9]), &mut rng).0 == 1 {
                heavy += 1;
            }
        }
        assert!((8_800..=9_200).contains(&heavy), "{heavy}");
        // clear of the slice's edges by a tenth of the slice
        for _ in 0..2_000 {
            let w = [3u32, 1, 6];
            let (winner, angle) = draw_weighted(3, Some(&w), &mut rng);
            let at = (360.0 - angle).rem_euclid(360.0);
            let before: u32 = w[..winner].iter().sum();
            let off = (at - 36.0 * f64::from(before)) / (36.0 * f64::from(w[winner]));
            assert!((0.099..=0.901).contains(&off), "off={off}");
        }
        // no weights, or the wrong count: an even draw that still lands inside its winner
        for _ in 0..200 {
            let (winner, angle) = draw_weighted(5, None, &mut rng);
            assert_eq!(wheel_winner_at(angle, 5), winner);
            let (winner, angle) = draw_weighted(5, Some(&[1, 2]), &mut rng);
            assert_eq!(wheel_winner_at(angle, 5), winner);
        }
    }

    #[test]
    fn weights_and_palettes_are_cleaned() {
        assert_eq!(clean_weights(None, 3).unwrap(), None);
        assert_eq!(clean_weights(Some(vec![1, 1, 1]), 3).unwrap(), None);
        assert_eq!(
            clean_weights(Some(vec![1, 5, 10]), 3).unwrap(),
            Some(vec![1, 5, 10])
        );
        assert!(clean_weights(Some(vec![1, 2]), 3).is_err());
        assert!(clean_weights(Some(vec![0, 1, 1]), 3).is_err());
        assert!(clean_weights(Some(vec![11, 1, 1]), 3).is_err());
        assert_eq!(clean_palette(None).unwrap(), None);
        assert_eq!(clean_palette(Some("neon".into())).unwrap(), None);
        assert_eq!(
            clean_palette(Some("gems".into())).unwrap(),
            Some("gems".into())
        );
        assert!(clean_palette(Some("rainbow".into())).is_err());
    }

    #[test]
    fn options_and_names_are_cleaned() {
        assert_eq!(
            clean_options(vec![" a ".into(), "b".into()]).unwrap(),
            vec!["a", "b"]
        );
        assert!(clean_options(vec!["a".into(), "  ".into()]).is_err());
        assert!(clean_options(vec!["x".repeat(61)]).is_err());
        assert!(clean_options(vec!["é".repeat(60)]).is_ok());
        assert!(clean_options(vec!["a\nb".into()]).is_err());
        assert!(clean_options(vec!["o".into(); 101]).is_err());
        assert!(clean_options(vec!["o".into(); 100]).is_ok());
        assert!(clean_options(vec![]).is_ok());
        assert!(spinnable(&["a".into()]).is_err());
        assert!(spinnable(&["a".into(), "a".into()]).is_ok());
        assert_eq!(clean_name("  Games tonight ").unwrap(), "Games tonight");
        assert!(clean_name(" ").is_err());
        assert!(clean_name(&"n".repeat(41)).is_err());
    }
}
