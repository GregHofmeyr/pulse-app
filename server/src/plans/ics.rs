//! A plan as a calendar file (#387): one VEVENT in a VCALENDAR (RFC 5545), times in UTC, text escaped and long lines
//! folded. Its UID is the plan's message id, so downloading it again after an edit updates the same event in a
//! calendar instead of adding another.
//!
//! DTEND is the plan's end, or (#399) an hour after its start when it has none: an event with DTSTART alone is a
//! zero-length point that calendar apps draw as a sliver or not at all, so a plan shows as an hour's block, as a new
//! event does when it's added in a calendar app.

use jiff::{SignedDuration, Timestamp};
use pulse_protocol::ids::MessageId;
use pulse_protocol::rest::{Plan, plan_limits};

fn parse(t: &str) -> Option<Timestamp> {
    crate::times::parse(t).ok()
}

fn ical(t: Timestamp) -> String {
    t.strftime("%Y%m%dT%H%M%SZ").to_string()
}

/// A stored time (RFC 3339) in iCalendar's UTC form, `20261120T163000Z`.
fn utc(t: &str) -> Option<String> {
    parse(t).map(ical)
}

/// A TEXT value escaped (RFC 5545 §3.3.11): backslash, semicolon, comma and line breaks.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str("\\n");
            }
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// One content line, folded at 75 octets (§3.1) without splitting a character, ending in CRLF.
pub fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + line.len() / 70 * 3 + 2);
    let mut width = 0;
    for c in line.chars() {
        let n = c.len_utf8();
        // a continuation line starts with a space, which counts toward its 75
        if width + n > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += n;
    }
    out.push_str("\r\n");
    out
}

/// The calendar file for plan `id`, stamped `now`. `host` names the server in the UID (`<id>@<host>`).
pub fn calendar(id: MessageId, plan: &Plan, host: &str, now: Timestamp) -> String {
    let mut lines: Vec<String> = vec![
        "BEGIN:VCALENDAR".into(),
        "VERSION:2.0".into(),
        "PRODID:-//Pulse//Plan something//EN".into(),
        "CALSCALE:GREGORIAN".into(),
        "METHOD:PUBLISH".into(),
        "BEGIN:VEVENT".into(),
        format!("UID:{id}@{host}"),
        format!("DTSTAMP:{}", now.strftime("%Y%m%dT%H%M%SZ")),
    ];
    if let Some(start) = parse(&plan.starts_at) {
        lines.push(format!("DTSTART:{}", ical(start)));
        let end = plan
            .ends_at
            .as_deref()
            .and_then(parse)
            .filter(|e| *e > start)
            .unwrap_or(start + SignedDuration::from_mins(plan_limits::DEFAULT_MINUTES));
        lines.push(format!("DTEND:{}", ical(end)));
    }
    lines.push(format!("SUMMARY:{}", escape(&plan.title)));
    if let Some(place) = &plan.place {
        lines.push(format!("LOCATION:{}", escape(place)));
    }
    if let Some(note) = &plan.note {
        lines.push(format!("DESCRIPTION:{}", escape(note)));
    }
    if let Some(edited) = plan.edited_at.as_deref().and_then(utc) {
        lines.push(format!("LAST-MODIFIED:{edited}"));
    }
    lines.push(
        if plan.cancelled_at.is_some() {
            "STATUS:CANCELLED"
        } else {
            "STATUS:CONFIRMED"
        }
        .into(),
    );
    lines.push("END:VEVENT".into());
    lines.push("END:VCALENDAR".into());
    lines.iter().map(|l| fold(l)).collect()
}

/// A file name for the plan's calendar file: its title, with anything a file system minds dropped.
pub fn file_name(title: &str) -> String {
    let stem: String = title
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let stem = stem.split_whitespace().collect::<Vec<_>>().join(" ");
    let stem = stem.trim_matches('.');
    if stem.is_empty() {
        "plan.ics".into()
    } else {
        format!("{stem}.ics")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        Plan {
            title: "Braai, games; drinks".into(),
            starts_at: "2026-11-20T16:30:00.000Z".into(),
            ends_at: None,
            place: Some("kim's place\\back yard".into()),
            note: Some("bring ice\nand chairs".into()),
            going: vec![],
            maybe: vec![],
            cant: vec![],
            cancelled_at: None,
            edited_at: None,
        }
    }

    #[test]
    fn text_is_escaped() {
        assert_eq!(escape("a,b;c\\d\ne\r\nf"), "a\\,b\\;c\\\\d\\ne\\nf");
        assert_eq!(escape("tab\there"), "tabhere");
    }

    #[test]
    fn long_lines_fold_at_75_octets_without_splitting_a_character() {
        let line = format!("SUMMARY:{}", "é".repeat(60));
        let folded = fold(&line);
        assert!(folded.ends_with("\r\n"));
        for part in folded.trim_end_matches("\r\n").split("\r\n") {
            assert!(part.len() <= 75, "{} octets: {part}", part.len());
        }
        // unfolding gives the line back
        assert_eq!(folded.trim_end_matches("\r\n").replace("\r\n ", ""), line);
        assert_eq!(fold("SHORT:x"), "SHORT:x\r\n");
    }

    #[test]
    fn the_calendar_has_one_event_in_utc_with_a_stable_uid() {
        let id = MessageId::new();
        let now = crate::times::parse("2026-11-16T10:00:00Z").unwrap();
        let ics = calendar(id, &plan(), "pulse.test", now);
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:"));
        assert!(ics.ends_with("END:VEVENT\r\nEND:VCALENDAR\r\n"));
        assert!(ics.contains(&format!("\r\nUID:{id}@pulse.test\r\n")));
        assert!(ics.contains("\r\nDTSTAMP:20261116T100000Z\r\n"));
        assert!(ics.contains("\r\nDTSTART:20261120T163000Z\r\n"));
        // no end: an hour's block
        assert!(ics.contains("\r\nDTEND:20261120T173000Z\r\n"));
        assert!(ics.contains("\r\nSUMMARY:Braai\\, games\\; drinks\r\n"));
        assert!(ics.contains("\r\nLOCATION:kim's place\\\\back yard\r\n"));
        assert!(ics.contains("\r\nDESCRIPTION:bring ice\\nand chairs\r\n"));
        assert!(ics.contains("\r\nSTATUS:CONFIRMED\r\n"));
        assert_eq!(ics.matches("BEGIN:VEVENT").count(), 1);
        // every line ends in CRLF: no bare LF anywhere
        assert!(!ics.replace("\r\n", "").contains('\n'));

        let mut cancelled = plan();
        cancelled.cancelled_at = Some("2026-11-17T08:00:00.000Z".into());
        cancelled.edited_at = Some("2026-11-17T08:00:00.000Z".into());
        cancelled.place = None;
        let ics = calendar(id, &cancelled, "pulse.test", now);
        assert!(ics.contains("\r\nSTATUS:CANCELLED\r\n"));
        assert!(ics.contains("\r\nLAST-MODIFIED:20261117T080000Z\r\n"));
        assert!(!ics.contains("LOCATION"));

        // its own end, when it has one
        let mut ends = plan();
        ends.ends_at = Some("2026-11-20T21:00:00.000Z".into());
        let ics = calendar(id, &ends, "pulse.test", now);
        assert!(ics.contains("\r\nDTEND:20261120T210000Z\r\n"));
        assert_eq!(ics.matches("DTEND").count(), 1);
    }

    #[test]
    fn the_file_is_named_after_the_title() {
        assert_eq!(file_name("Braai at kim's"), "Braai at kim's.ics");
        assert_eq!(file_name("a/b: c?"), "a b c.ics");
        assert_eq!(file_name(" ... "), "plan.ics");
    }
}
