//! Time helpers on `jiff` (#441: off chrono, which is unmaintained). The formats are chrono's, byte for byte: stored
//! and API times are RFC 3339 UTC with milliseconds and a `Z` ([`crate::db::stamp`]), and parsing takes RFC 3339 as
//! chrono's `parse_from_rfc3339` did (a full date and time with seconds, and an offset or `Z`).

use jiff::civil::Date;
use jiff::fmt::temporal::DateTimeParser;
use jiff::tz::Offset;
use jiff::{SignedDuration, Timestamp};

/// `n` days of 24 hours (chrono's `Duration::days`).
pub const fn days(n: i64) -> SignedDuration {
    SignedDuration::from_hours(n * 24)
}

/// `n` weeks of 7 × 24 hours (chrono's `Duration::weeks`).
pub const fn weeks(n: i64) -> SignedDuration {
    days(n * 7)
}

/// A malformed RFC 3339 time.
#[derive(Debug, thiserror::Error)]
#[error("not an RFC 3339 time")]
pub struct BadTime;

/// Whether `s` has RFC 3339's shape (`YYYY-MM-DD`, `T`/`t`/space, `HH:MM:SS`): what chrono insisted on and jiff's
/// wider ISO 8601 parser doesn't (it also takes basic format and times without seconds).
fn rfc3339_shape(s: &str) -> bool {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| {
        r.into_iter()
            .all(|i| b.get(i).is_some_and(u8::is_ascii_digit))
    };
    b.len() >= 20
        && digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && matches!(b[10], b'T' | b't' | b' ')
        && digits(11..13)
        && b[13] == b':'
        && digits(14..16)
        && b[16] == b':'
        && digits(17..19)
        // fractional seconds, then the offset: nothing else (no `[zone]` annotation)
        && !s.contains('[')
}

/// An RFC 3339 time with its offset (`Z` reads as UTC), as chrono's `DateTime::parse_from_rfc3339`.
pub fn parse_with_offset(s: &str) -> Result<(Timestamp, Offset), BadTime> {
    if !rfc3339_shape(s) {
        return Err(BadTime);
    }
    static PARSER: DateTimeParser = DateTimeParser::new();
    let pieces = PARSER.parse_pieces(s).map_err(|_| BadTime)?;
    let offset = match pieces.offset() {
        Some(o) => o.to_numeric_offset(),
        None => return Err(BadTime),
    };
    let time = pieces.time().ok_or(BadTime)?;
    let dt = pieces.date().to_datetime(time);
    let ts = offset.to_timestamp(dt).map_err(|_| BadTime)?;
    Ok((ts, offset))
}

/// An RFC 3339 time as an instant (chrono's `parse_from_rfc3339(..).with_timezone(&Utc)`).
pub fn parse(s: &str) -> Result<Timestamp, BadTime> {
    parse_with_offset(s).map(|(t, _)| t)
}

/// RFC 3339 UTC with whole seconds and a `Z` (chrono's `to_rfc3339_opts(SecondsFormat::Secs, true)`).
pub fn rfc3339_secs(t: Timestamp) -> String {
    t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// RFC 3339 UTC with `+00:00` and as many fractional digits as needed in groups of three (chrono's `to_rfc3339`).
pub fn rfc3339(t: Timestamp) -> String {
    let fmt = match t.subsec_nanosecond() {
        0 => "%Y-%m-%dT%H:%M:%S+00:00",
        n if n % 1_000_000 == 0 => "%Y-%m-%dT%H:%M:%S%.3f+00:00",
        n if n % 1_000 == 0 => "%Y-%m-%dT%H:%M:%S%.6f+00:00",
        _ => "%Y-%m-%dT%H:%M:%S%.9f+00:00",
    };
    t.strftime(fmt).to_string()
}

/// The UTC calendar date of `t` (chrono's `date_naive()` on a UTC time).
pub fn utc_date(t: Timestamp) -> Date {
    Offset::UTC.to_datetime(t).date()
}

/// Midnight UTC at the start of `d` (chrono's `d.and_hms_opt(0, 0, 0).unwrap().and_utc()`).
pub fn utc_midnight(d: Date) -> Timestamp {
    Offset::UTC
        .to_timestamp(d.to_datetime(jiff::civil::Time::midnight()))
        .expect("a civil date is in range")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_are_chronos_byte_for_byte() {
        let t: Timestamp = "2026-11-19T20:52:52.123987654Z".parse().unwrap();
        // milliseconds truncated, never rounded, as chrono's SecondsFormat::Millis
        assert_eq!(crate::db::stamp(t), "2026-11-19T20:52:52.123Z");
        let whole: Timestamp = "2026-01-02T03:04:05Z".parse().unwrap();
        assert_eq!(crate::db::stamp(whole), "2026-01-02T03:04:05.000Z");
        assert_eq!(rfc3339_secs(t), "2026-11-19T20:52:52Z");
        assert_eq!(rfc3339(t), "2026-11-19T20:52:52.123987654+00:00");
        assert_eq!(rfc3339(whole), "2026-01-02T03:04:05+00:00");
        let ms: Timestamp = "2026-01-02T03:04:05.120Z".parse().unwrap();
        assert_eq!(rfc3339(ms), "2026-01-02T03:04:05.120+00:00");
        let before_epoch: Timestamp = "1969-12-31T23:59:59.5Z".parse().unwrap();
        assert_eq!(crate::db::stamp(before_epoch), "1969-12-31T23:59:59.500Z");
    }

    #[test]
    fn parses_what_chronos_rfc3339_parser_took() {
        let utc = |s: &str| parse(s).map(crate::db::stamp).ok();
        assert_eq!(
            utc("2026-11-19T20:52:52Z").as_deref(),
            Some("2026-11-19T20:52:52.000Z")
        );
        assert_eq!(
            utc("2026-11-19t20:52:52z").as_deref(),
            Some("2026-11-19T20:52:52.000Z")
        );
        assert_eq!(
            utc("2026-11-19 20:52:52Z").as_deref(),
            Some("2026-11-19T20:52:52.000Z")
        );
        assert_eq!(
            utc("2026-11-19T22:52:52.5+02:00").as_deref(),
            Some("2026-11-19T20:52:52.500Z")
        );
        assert_eq!(
            utc("2026-11-19T20:52:52.123456789-00:00").as_deref(),
            Some("2026-11-19T20:52:52.123Z")
        );
        let (_, off) = parse_with_offset("2026-11-19T22:52:52+02:00").unwrap();
        assert_eq!(off.seconds(), 7200);
        // and refuses what it refused
        for bad in [
            "",
            "2026-11-19",
            "2026-11-19T20:52Z",
            "2026-11-19T20:52:52",
            "20261119T205252Z",
            "2026-11-19T20:52:52Z[Africa/Johannesburg]",
            "2026-13-19T20:52:52Z",
            "2026-11-19T25:52:52Z",
            "not a time at all, no",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn days_weeks_and_dates() {
        assert_eq!(days(2), SignedDuration::from_hours(48));
        assert_eq!(weeks(1), SignedDuration::from_hours(168));
        let t: Timestamp = "2026-11-19T23:59:59.999Z".parse().unwrap();
        assert_eq!(utc_date(t), jiff::civil::date(2026, 11, 19));
        assert_eq!(
            crate::db::stamp(utc_midnight(utc_date(t))),
            "2026-11-19T00:00:00.000Z"
        );
    }
}
