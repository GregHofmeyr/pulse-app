//! ffmpeg's `-progress pipe:1` lines.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressLine {
    /// How far the output has got, in microseconds of output time.
    OutTimeUs(u64),
    /// `progress=end`: ffmpeg is done writing.
    End,
}

/// Read one `-progress` line. Everything else, and `out_time_us=N/A` or a negative time (ffmpeg's way of
/// saying "not yet"), is `None`.
pub fn parse(line: &str) -> Option<ProgressLine> {
    let line = line.trim();
    if let Some(us) = line.strip_prefix("out_time_us=") {
        return us.trim().parse().ok().map(ProgressLine::OutTimeUs);
    }
    (line == "progress=end").then_some(ProgressLine::End)
}

/// A fraction of the work to a whole percent for the UI: 0 to 99 (100 is for the caller once the job is done).
pub fn percent(fraction: f32) -> u8 {
    (fraction * 100.0).clamp(0.0, 99.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines_parse() {
        assert_eq!(
            parse("out_time_us=5000000"),
            Some(ProgressLine::OutTimeUs(5_000_000))
        );
        assert_eq!(parse("out_time_us=0"), Some(ProgressLine::OutTimeUs(0)));
        assert_eq!(parse("progress=end\n"), Some(ProgressLine::End));
        // not yet known, negative, other keys and junk are not progress
        assert_eq!(parse("out_time_us=N/A"), None);
        assert_eq!(parse("out_time_us=-9223372036854775808"), None);
        assert_eq!(parse("out_time_ms=5000000"), None);
        assert_eq!(parse("progress=continue"), None);
        assert_eq!(parse("frame=12"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn percent_stops_at_99() {
        assert_eq!(percent(0.0), 0);
        assert_eq!(percent(0.5), 50);
        assert_eq!(percent(1.0), 99);
        assert_eq!(percent(7.0), 99);
        assert_eq!(percent(-1.0), 0);
    }
}
