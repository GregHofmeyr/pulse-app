//! Log file in the app's log folder (per profile), so a Windows test session can be read back
//! afterwards from Linux. Panics are logged too.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// Start logging to `<dir>/pulse.log.<date>` (daily files, last 7 kept). Keep the guard alive for
/// the app's lifetime: dropping it flushes and stops the writer.
pub fn init(dir: &Path) -> anyhow::Result<WorkerGuard> {
    std::fs::create_dir_all(dir)?;
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("pulse")
        .filename_suffix("log")
        .max_log_files(7)
        .build(dir)?;
    let (writer, guard) =
        tracing_appender::non_blocking(Capped::new(appender, LOG_CAP_BYTES, utc_day));
    let filter = EnvFilter::try_from_env("PULSE_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,livekit=warn,libwebrtc=warn"));
    // try_init: tests / a second call must not panic on an already-set global subscriber
    let _ = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(filter)
        .try_init();

    install_panic_hook(dir.to_path_buf());
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        "Pulse starting"
    );
    Ok(guard)
}

/// The crash marker + `PANIC` log line, then the previous hook (so the panic still prints).
/// Once per process: a second `init` must not stack another wrapper and log each panic twice.
fn install_panic_hook(marker_dir: std::path::PathBuf) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // Chain to the previous hook so the panic still prints (a console run, and test output).
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // marker first: it's what reports the crash, and logging could itself fail mid-panic
            let message = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".into());
            let location = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_default();
            crate::reports::crash::write_panic_marker(&marker_dir, &message, &location);
            tracing::error!(panic = %info, "PANIC");
            previous(info);
        }));
    });
}

/// Stops writing after `cap` bytes per UTC day (one notice line marks the cut), so a log flood
/// can't fill the disk. `day` returns the current UTC day number; injectable for tests.
pub struct Capped<W> {
    inner: W,
    cap: u64,
    day: fn() -> u64,
    today: u64,
    written: u64,
}

pub const LOG_CAP_BYTES: u64 = 50 * 1024 * 1024;

pub fn utc_day() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0)
}

impl<W: std::io::Write> Capped<W> {
    pub fn new(inner: W, cap: u64, day: fn() -> u64) -> Self {
        Self {
            inner,
            cap,
            day,
            today: day(),
            written: 0,
        }
    }
}

impl<W: std::io::Write> std::io::Write for Capped<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let day = (self.day)();
        if day != self.today {
            self.today = day;
            self.written = 0;
        }
        if self.written >= self.cap {
            return Ok(buf.len()); // already cut today: swallow
        }
        if self.written + buf.len() as u64 > self.cap {
            self.written = self.cap;
            self.inner
                .write_all(b"WARN pulse: log size cap reached; nothing more is logged today\n")?;
            return Ok(buf.len());
        }
        self.written += buf.len() as u64;
        self.inner.write_all(buf)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static DAY: AtomicU64 = AtomicU64::new(1);
    fn fake_day() -> u64 {
        DAY.load(Ordering::SeqCst)
    }

    #[test]
    fn capped_writer_stops_at_the_cap_with_one_notice_and_resets_next_day() {
        DAY.store(1, Ordering::SeqCst);
        let mut w = Capped::new(Vec::new(), 10, fake_day);
        w.write_all(b"12345\n").unwrap();
        w.write_all(b"67890\n").unwrap(); // crosses the cap: dropped, notice written
        w.write_all(b"more\n").unwrap(); // dropped silently
        let text = String::from_utf8(w.inner.clone()).unwrap();
        assert_eq!(text.matches("log size cap reached").count(), 1, "{text}");
        assert!(text.starts_with("12345\n"));
        assert!(!text.contains("67890") && !text.contains("\nmore\n"));
        DAY.store(2, Ordering::SeqCst);
        w.write_all(b"fresh\n").unwrap();
        assert!(
            String::from_utf8(w.inner.clone())
                .unwrap()
                .ends_with("fresh\n")
        );
    }

    const PANIC_CHILD: &str = "PULSE_TEST_PANIC_HOOK_CHILD";

    /// Runs only as the child of `a_panic_after_init_still_prints_and_leaves_a_marker`.
    #[test]
    fn panic_hook_child() {
        let Some(dir) = std::env::var_os(PANIC_CHILD) else {
            return;
        };
        let _guard = init(Path::new(&dir)).unwrap();
        let _again = init(Path::new(&dir)).unwrap(); // a second init must not stack the hook
        panic!("child-panic-xyzzy");
    }

    /// The hook is process-wide, so it's installed in a child test process: here it can't hide
    /// other tests' panics. After init a panic must still print (the default hook runs too), and a
    /// second init must not stack a second hook (the panic would be logged twice).
    #[test]
    fn a_panic_after_init_still_prints_and_leaves_a_marker() {
        let dir = tempfile::tempdir().unwrap();
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "logging::tests::panic_hook_child", "--nocapture"])
            .args(["--test-threads", "1"])
            .env(PANIC_CHILD, dir.path())
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(!out.status.success(), "the child panics: {stdout}");
        assert!(
            stderr.contains("child-panic-xyzzy"),
            "panic text reaches stderr: {stderr}"
        );
        let log: String = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|f| f.file_name().to_string_lossy().starts_with("pulse"))
            .map(|f| std::fs::read_to_string(f.path()).unwrap_or_default())
            .collect();
        assert_eq!(
            log.matches("child-panic-xyzzy").count(),
            1,
            "logged once although init ran twice: {log}"
        );
        assert!(
            dir.path().join("crash.json").exists(),
            "crash marker written"
        );
    }

    #[test]
    fn writes_messages_to_a_file_in_the_dir() {
        let dir = tempfile::tempdir().unwrap();
        let guard = init(dir.path()).unwrap();
        tracing::info!(marker = "xyzzy-123", "hello from the test");
        drop(guard); // flush
        let files: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().collect();
        assert!(!files.is_empty(), "no log file written");
        let all: String = files
            .iter()
            .map(|f| std::fs::read_to_string(f.path()).unwrap_or_default())
            .collect();
        assert!(all.contains("xyzzy-123"), "log content: {all}");
    }
}
