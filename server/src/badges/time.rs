//! Pure time helpers for the voice badges. South African time is a fixed UTC+2 (no daylight
//! saving), so it is a constant offset rather than a time zone database.

use chrono::{DateTime, Duration, FixedOffset, Utc};

use crate::error::{AppError, AppResult};

/// South African Standard Time.
pub const SAST: FixedOffset = match FixedOffset::east_opt(2 * 3600) {
    Some(o) => o,
    None => panic!("a valid offset"),
};

const DAY: i64 = 24 * 3600;

/// A counted muted-talk report needs this long since the last counted one (seconds)...
pub const DOOS_MIN_GAP: i64 = 60;
/// ...and at most this many count in one South African day.
pub const DOOS_DAY_CAP: i64 = 5;

/// A stored time (see [`crate::db::stamp`]).
pub fn parse(t: &str) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(t)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| AppError::Internal(e.into()))
}

/// The South African day `t` falls on, as days since the epoch.
pub fn sast_day(t: DateTime<Utc>) -> i64 {
    unix_sast_day(t.timestamp())
}

fn unix_sast_day(unix: i64) -> i64 {
    (unix + i64::from(SAST.local_minus_utc())).div_euclid(DAY)
}

/// When the South African day `t` falls on began.
pub fn sast_day_start(t: DateTime<Utc>) -> DateTime<Utc> {
    let start = sast_day(t) * DAY - i64::from(SAST.local_minus_utc());
    DateTime::from_timestamp(start, 0).expect("in range")
}

/// The parts of `mine` when someone else was there too: the union of `others` clipped to `mine`,
/// merged and in order.
pub fn accompanied(
    mine: (DateTime<Utc>, DateTime<Utc>),
    others: &[(DateTime<Utc>, DateTime<Utc>)],
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let mut clipped: Vec<_> = others
        .iter()
        .map(|&(a, b)| (a.max(mine.0), b.min(mine.1)))
        .filter(|(a, b)| a < b)
        .collect();
    clipped.sort();
    let mut out: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();
    for (a, b) in clipped {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Whole seconds covered by `intervals` (which don't overlap).
pub fn seconds(intervals: &[(DateTime<Utc>, DateTime<Utc>)]) -> i64 {
    intervals.iter().map(|(a, b)| (*b - *a).num_seconds()).sum()
}

/// Whether one of `intervals` (start inclusive, end exclusive) holds 03:00 South African time.
pub fn covers_3am(intervals: &[(DateTime<Utc>, DateTime<Utc>)]) -> bool {
    intervals.iter().any(|&(a, b)| {
        // the first 03:00 SAST at or after `a`
        let three = sast_day_start(a) + Duration::hours(3);
        let three = if three < a {
            three + Duration::days(1)
        } else {
            three
        };
        three < b
    })
}

/// Whether a muted-talk report at `now_unix` counts, given the last counted one (`last_at`, unix
/// seconds) and the day (`day`, see [`sast_day`]) and number (`day_count`) of the last counted.
pub fn doos_counts(now_unix: i64, last_at: i64, day: i64, day_count: i64) -> bool {
    now_unix - last_at >= DOOS_MIN_GAP
        && (day != unix_sast_day(now_unix) || day_count < DOOS_DAY_CAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn sast_day_turns_at_22_utc() {
        let before = t("2026-11-03T21:59:59Z");
        let after = t("2026-11-03T22:00:00Z");
        assert_eq!(sast_day(after), sast_day(before) + 1);
        assert_eq!(sast_day_start(after), after);
        assert_eq!(sast_day_start(before), t("2026-11-02T22:00:00Z"));
        // 1970-01-01 02:00 SAST is day 0
        assert_eq!(sast_day(t("1970-01-01T00:00:00Z")), 0);
        assert_eq!(sast_day(t("1969-12-31T21:59:59Z")), -1);
    }

    #[test]
    fn accompanied_unions_overlaps() {
        let mine = (t("2026-11-03T10:00:00Z"), t("2026-11-03T12:00:00Z"));
        let others = [
            (t("2026-11-03T11:30:00Z"), t("2026-11-03T13:00:00Z")),
            (t("2026-11-03T09:00:00Z"), t("2026-11-03T10:30:00Z")),
            (t("2026-11-03T10:15:00Z"), t("2026-11-03T11:00:00Z")),
        ];
        let got = accompanied(mine, &others);
        assert_eq!(
            got,
            vec![
                (t("2026-11-03T10:00:00Z"), t("2026-11-03T11:00:00Z")),
                (t("2026-11-03T11:30:00Z"), t("2026-11-03T12:00:00Z")),
            ]
        );
        assert_eq!(seconds(&got), 5400);
    }

    #[test]
    fn alone_is_zero() {
        let mine = (t("2026-11-03T10:00:00Z"), t("2026-11-03T12:00:00Z"));
        assert!(accompanied(mine, &[]).is_empty());
        // someone there before and after, never during
        let others = [
            (t("2026-11-03T08:00:00Z"), t("2026-11-03T10:00:00Z")),
            (t("2026-11-03T12:00:00Z"), t("2026-11-03T13:00:00Z")),
        ];
        assert_eq!(seconds(&accompanied(mine, &others)), 0);
    }

    #[test]
    fn covers_3am_only_with_company_at_0100_utc() {
        let mine = (t("2026-11-03T00:00:00Z"), t("2026-11-03T02:00:00Z"));
        // company only after 03:00 SAST
        let late = accompanied(
            mine,
            &[(t("2026-11-03T01:00:01Z"), t("2026-11-03T03:00:00Z"))],
        );
        assert!(!covers_3am(&late));
        // company only before it
        let early = accompanied(
            mine,
            &[(t("2026-11-02T23:00:00Z"), t("2026-11-03T01:00:00Z"))],
        );
        assert!(!covers_3am(&early));
        // company from exactly 03:00 SAST (inclusive start)
        let on = accompanied(
            mine,
            &[(t("2026-11-03T01:00:00Z"), t("2026-11-03T01:30:00Z"))],
        );
        assert!(covers_3am(&on));
        // a long night across midnight UTC
        assert!(covers_3am(&[(
            t("2026-11-02T20:00:00Z"),
            t("2026-11-03T04:00:00Z")
        )]));
        assert!(!covers_3am(&[]));
    }

    #[test]
    fn doos_gap_and_daily_cap() {
        let now = t("2026-11-03T10:00:00Z").timestamp();
        let today = sast_day(t("2026-11-03T10:00:00Z"));
        // never counted before
        assert!(doos_counts(now, 0, 0, 0));
        // within a minute of the last counted one
        assert!(!doos_counts(now, now - 30, today, 1));
        assert!(doos_counts(now, now - 60, today, 1));
        // five already today: the sixth doesn't count
        assert!(doos_counts(now, now - 600, today, 4));
        assert!(!doos_counts(now, now - 600, today, 5));
        // the next South African day starts again
        let tomorrow = t("2026-11-03T22:00:00Z").timestamp();
        assert!(doos_counts(tomorrow, now, today, 5));
    }
}
