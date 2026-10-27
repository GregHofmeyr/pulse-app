//! The end of the newest log file, for attaching to a report.

use std::path::Path;

/// Last `max` bytes of the newest `pulse.*.log` in `dir`, cut to start at a whole line and
/// decoded lossily (a cut can land mid-character), and cut further to start at the last session
/// marker line (`logging::is_session_marker_line`) if one is in that window. Empty if there's no log.
pub fn log_tail(dir: &Path, max: usize) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(entries) = std::fs::read_dir(dir) else {
        return String::new();
    };
    let newest = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("pulse.") && n.ends_with(".log"))
        })
        .max(); // names are pulse.YYYY-MM-DD.log: the greatest is the newest
    let Some(path) = newest else {
        return String::new();
    };
    let Ok(mut f) = std::fs::File::open(&path) else {
        return String::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(max as u64);
    let mut buf = Vec::new();
    if f.seek(SeekFrom::Start(start)).is_err() || f.read_to_end(&mut buf).is_err() {
        return String::new();
    }
    if start > 0 {
        // drop the partial first line
        match buf.iter().position(|b| *b == b'\n') {
            Some(i) => {
                buf.drain(..=i);
            }
            None => buf.clear(),
        }
    }
    let text = String::from_utf8_lossy(&buf);
    // Start at the current session: the last marker line (if it's in the window). Only the
    // marker as the log writes it counts, not a line that merely contains its text.
    let mut at = 0;
    let mut start = None;
    for line in text.split_inclusive('\n') {
        if crate::logging::is_session_marker_line(line) {
            start = Some(at);
        }
        at += line.len();
    }
    match start {
        Some(i) => text[i..].to_string(),
        None => text.into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_newest_file() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("pulse.2026-10-11.log"), "old\n").unwrap();
        std::fs::write(d.path().join("pulse.2026-10-12.log"), "new\n").unwrap();
        std::fs::write(d.path().join("crash.json"), "{}").unwrap();
        assert_eq!(log_tail(d.path(), 1024), "new\n");
    }

    #[test]
    fn tail_starts_at_a_whole_line() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("pulse.2026-10-12.log"),
            "first line\nsecond é line\nthird\n",
        )
        .unwrap();
        // 20 bytes from the end lands inside "second é line"
        assert_eq!(log_tail(d.path(), 20), "third\n");
    }

    #[test]
    fn tail_starts_at_the_last_session_marker() {
        let d = tempfile::tempdir().unwrap();
        let m = crate::logging::SESSION_MARKER;
        std::fs::write(
            d.path().join("pulse.2026-10-12.log"),
            format!(
                "A one\n INFO pulse_session: {m}\nA two\n INFO pulse_session: {m}\nB one\nB two\n"
            ),
        )
        .unwrap();
        let t = log_tail(d.path(), 1024);
        assert!(
            t.starts_with(" INFO pulse_session: pulse-session-start\nB one"),
            "{t}"
        );
        assert!(!t.contains("A one") && !t.contains("A two"), "{t}");
        assert_eq!(t.matches(m).count(), 1, "{t}");
    }

    /// Only the marker line as the logger writes it starts a session: a line that merely
    /// contains the marker text doesn't cut the tail.
    #[test]
    fn a_line_merely_containing_the_marker_does_not_cut_the_tail() {
        let d = tempfile::tempdir().unwrap();
        let m = crate::logging::SESSION_MARKER;
        let real = format!("2026-10-12T10:00:00.000000Z  INFO pulse_session: {m}\n");
        std::fs::write(
            d.path().join("pulse.2026-10-12.log"),
            format!(
                "A one\n{real}B one\n\
                 2026-10-12T10:00:01.000000Z  WARN pulse_client::api: server said {m}\n\
                 {m}\n\
                 2026-10-12T10:00:02.000000Z  INFO pulse_client: x pulse_session: {m}\n\
                 B two\n"
            ),
        )
        .unwrap();
        let t = log_tail(d.path(), 1024);
        assert!(t.starts_with(&format!("{real}B one\n")), "{t}");
        assert!(t.ends_with("B two\n") && !t.contains("A one"), "{t}");
    }

    #[test]
    fn a_marker_outside_the_window_changes_nothing() {
        let d = tempfile::tempdir().unwrap();
        let m = crate::logging::SESSION_MARKER;
        std::fs::write(
            d.path().join("pulse.2026-10-12.log"),
            format!("{m}\nfirst line\nsecond line\nthird\n"),
        )
        .unwrap();
        assert_eq!(log_tail(d.path(), 14), "third\n");
    }

    #[test]
    fn no_log_is_empty() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(log_tail(d.path(), 1024), "");
        assert_eq!(log_tail(&d.path().join("missing"), 1024), "");
    }
}
