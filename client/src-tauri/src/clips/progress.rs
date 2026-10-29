//! ffmpeg's `-progress pipe:1` lines.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressLine {
    /// How far the output has got, in microseconds of output time.
    OutTimeUs(u64),
    /// `progress=end`: ffmpeg is done writing.
    End,
    /// `frame=`: video frames written so far.
    Frame(u64),
}

/// Read one `-progress` line. Everything else, and `out_time_us=N/A` or a negative time (ffmpeg's way of
/// saying "not yet"), is `None`.
pub fn parse(line: &str) -> Option<ProgressLine> {
    let line = line.trim();
    if let Some(us) = line.strip_prefix("out_time_us=") {
        return us.trim().parse().ok().map(ProgressLine::OutTimeUs);
    }
    if let Some(n) = line.strip_prefix("frame=") {
        return n.trim().parse().ok().map(ProgressLine::Frame);
    }
    (line == "progress=end").then_some(ProgressLine::End)
}

/// Reads one ffmpeg run's `-progress` lines for real progress: output time that moved while video frames had
/// been written. Output time alone is not enough: with audio, ffmpeg reports the audio's time while a GPU
/// video encoder (opened lazily, on the first filtered frame) has not yet opened, or never will.
#[derive(Debug, Default)]
pub struct Tracker {
    frames: u64,
    advanced: bool,
}

impl Tracker {
    /// Take one line; the output time in microseconds when it is real progress.
    pub fn feed(&mut self, line: &str) -> Option<u64> {
        match parse(line)? {
            // `frame=` comes before `out_time_us=` in each block
            ProgressLine::Frame(n) => {
                self.frames = n;
                None
            }
            ProgressLine::OutTimeUs(us @ 1..) if self.frames > 0 => {
                self.advanced = true;
                Some(us)
            }
            ProgressLine::OutTimeUs(_) | ProgressLine::End => None,
        }
    }

    /// Whether the run has made real progress (frames written and output time moved).
    pub fn advanced(&self) -> bool {
        self.advanced
    }
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
        assert_eq!(parse(""), None);
    }

    #[test]
    fn frame_lines_parse() {
        assert_eq!(parse("frame=197"), Some(ProgressLine::Frame(197)));
        assert_eq!(parse("frame=0"), Some(ProgressLine::Frame(0)));
        assert_eq!(parse("frame=N/A"), None);
    }

    /// The block ffmpeg printed when h264_nvenc could not load libcuda: the audio had been encoded, so
    /// `out_time_us` moved, but no video frame was ever written.
    const NVENC_NEVER_OPENED: &str = "frame=0\nfps=0.00\nstream_0_0_q=0.0\nbitrate=   0.0kbits/s\n\
total_size=0\nout_time_us=369625\nout_time_ms=369625\nout_time=00:00:00.369625\ndup_frames=0\n\
drop_frames=0\nspeed=3.08x\nprogress=end";

    #[test]
    fn out_time_without_a_frame_written_is_not_an_advance() {
        let mut t = Tracker::default();
        let advances: Vec<_> = NVENC_NEVER_OPENED
            .lines()
            .filter_map(|l| t.feed(l))
            .collect();
        assert!(advances.is_empty(), "{advances:?}");
        assert!(!t.advanced());
    }

    #[test]
    fn out_time_with_frames_written_is_an_advance() {
        let mut t = Tracker::default();
        let lines = [
            "frame=0",
            "out_time_us=0",
            "progress=continue",
            "frame=87",
            "total_size=524336",
            "out_time_us=2944000",
            "progress=continue",
            "frame=300",
            "out_time_us=N/A",
            "out_time_us=5000000",
            "progress=end",
        ];
        let advances: Vec<_> = lines.iter().filter_map(|l| t.feed(l)).collect();
        assert_eq!(advances, [2_944_000, 5_000_000]);
        assert!(t.advanced());
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
