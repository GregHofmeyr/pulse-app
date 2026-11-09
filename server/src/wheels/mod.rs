//! The picker wheel (#158): saved wheels (a server's, or one person's "Just me" wheels) and spins. A spin is a
//! system line whose payload carries the whole spin; `wheel_spins` keeps each one so Spin again and Remove winner
//! & spin run from the stored spin. The server draws the winner and the angle; clients only animate to them.

pub mod routes;

use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{Wheel, WheelScope, wheel_limits};
use rand::Rng;
use sqlx::SqlitePool;

use crate::error::{AppError, AppResult};

type Row = (
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    String,
);

const COLS: &str = "id, server_id, owner_id, name, options, updated_at";

fn from_row((id, server_id, owner_id, name, options, updated_at): Row) -> AppResult<Wheel> {
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
    })
}

/// A stored JSON array of options.
pub fn parse_options(stored: &str) -> AppResult<Vec<String>> {
    Ok(serde_json::from_str(stored).map_err(anyhow::Error::from)?)
}

pub async fn load(db: &SqlitePool, id: &str) -> AppResult<Option<Wheel>> {
    let row: Option<Row> = sqlx::query_as(&format!("SELECT {COLS} FROM wheels WHERE id = ?"))
        .bind(id)
        .fetch_optional(db)
        .await?;
    row.map(from_row).transpose()
}

/// A server's wheels, by name.
pub async fn for_server(db: &SqlitePool, server: ServerId) -> AppResult<Vec<Wheel>> {
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM wheels WHERE server_id = ? ORDER BY name COLLATE NOCASE, id"
    ))
    .bind(server.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter().map(from_row).collect()
}

/// One person's Just me wheels, by name.
pub async fn for_owner(db: &SqlitePool, owner: UserId) -> AppResult<Vec<Wheel>> {
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM wheels WHERE owner_id = ? ORDER BY name COLLATE NOCASE, id"
    ))
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
pub fn draw(n: usize, rng: &mut impl Rng) -> (usize, f64) {
    let winner = rng.random_range(0..n);
    let seg = 360.0 / n as f64;
    let at = seg * (winner as f64 + rng.random_range(0.1..0.9));
    let angle = (360.0 - at).rem_euclid(360.0);
    (winner, angle)
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
