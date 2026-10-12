//! The end of the newest log file, for attaching to a report.

use std::path::Path;

/// Last `max` bytes of the newest `pulse.*.log` in `dir`, cut to start at a whole line and
/// decoded lossily (a cut can land mid-character). Empty if there's no log.
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
    String::from_utf8_lossy(&buf).into_owned()
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
    fn no_log_is_empty() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(log_tail(d.path(), 1024), "");
        assert_eq!(log_tail(&d.path().join("missing"), 1024), "");
    }
}
