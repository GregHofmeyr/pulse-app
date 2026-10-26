//! Log file in the app's log folder (per profile), so a Windows test session can be read back
//! afterwards from Linux. Panics are logged too.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// Logged once per sign-in / restore. Report log tails start at the last one, so a report holds
/// the current session only.
pub const SESSION_MARKER: &str = "pulse-session-start";

pub fn session_start() {
    tracing::info!("{SESSION_MARKER}");
}

/// Libraries that must never log below info, whatever `PULSE_LOG` says: tungstenite trace-logs
/// frame payloads (the gateway Hello carries the session token); the others log headers/bytes.
const CAPPED_TARGETS: [&str; 8] = [
    "tungstenite",
    "tokio_tungstenite",
    "hyper",
    "hyper_util",
    "h2",
    "reqwest",
    "rustls",
    "tower_http",
];

/// The log filter: `user` (the `PULSE_LOG` value) or the default, then the library caps. The caps
/// come last so they win: for the same target a later directive replaces an earlier one, and a
/// target directive also covers its child modules.
pub fn build_filter(user: Option<&str>) -> EnvFilter {
    let mut filter = user
        .and_then(|s| EnvFilter::try_new(s).ok())
        .unwrap_or_else(|| EnvFilter::new("info,livekit=warn,libwebrtc=warn"));
    for target in CAPPED_TARGETS {
        if let Ok(d) = format!("{target}=info").parse() {
            filter = filter.add_directive(d);
        }
    }
    filter
}

/// `path` for a log line: the home directory prefix becomes `~`.
pub fn redact_home(path: &Path) -> String {
    redact_home_in(path, home_dir().as_deref())
}

fn home_dir() -> Option<std::path::PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .map(std::path::PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

fn redact_home_in(path: &Path, home: Option<&Path>) -> String {
    if let Some(rest) = home.and_then(|h| path.strip_prefix(h).ok()) {
        if rest.as_os_str().is_empty() {
            return "~".into();
        }
        return format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display());
    }
    path.display().to_string()
}

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
    let filter = build_filter(std::env::var("PULSE_LOG").ok().as_deref());
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
    use tracing::Level;
    use tracing_subscriber::layer::SubscriberExt;

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

    /// Is `target` enabled at `level` under a subscriber filtered by `filter`?
    fn enabled(filter: EnvFilter, target: &'static str, level: Level) -> bool {
        let sub = tracing_subscriber::registry().with(filter);
        tracing::subscriber::with_default(sub, || probe(target, level))
    }

    fn probe(target: &'static str, level: Level) -> bool {
        // one callsite per (target, level): `enabled!` needs literals
        match (target, level) {
            ("tungstenite", Level::TRACE) => tracing::enabled!(target: "tungstenite", Level::TRACE),
            ("tungstenite", Level::DEBUG) => tracing::enabled!(target: "tungstenite", Level::DEBUG),
            ("tungstenite", Level::INFO) => tracing::enabled!(target: "tungstenite", Level::INFO),
            ("hyper_util", Level::TRACE) => tracing::enabled!(target: "hyper_util", Level::TRACE),
            ("hyper_util::client", Level::TRACE) => {
                tracing::enabled!(target: "hyper_util::client", Level::TRACE)
            }
            ("rustls", Level::DEBUG) => tracing::enabled!(target: "rustls", Level::DEBUG),
            ("pulse_client::gateway", Level::TRACE) => {
                tracing::enabled!(target: "pulse_client::gateway", Level::TRACE)
            }
            _ => panic!("unprobed {target} {level}"),
        }
    }

    #[test]
    fn pulse_log_cannot_raise_library_targets_above_info() {
        let f = || build_filter(Some("trace"));
        assert!(!enabled(f(), "tungstenite", Level::TRACE));
        assert!(!enabled(f(), "tungstenite", Level::DEBUG));
        assert!(enabled(f(), "tungstenite", Level::INFO));
        assert!(!enabled(f(), "hyper_util", Level::TRACE));
        assert!(!enabled(f(), "hyper_util::client", Level::TRACE)); // child modules too
        assert!(!enabled(f(), "rustls", Level::DEBUG));
        // the app's own targets still follow PULSE_LOG
        assert!(enabled(f(), "pulse_client::gateway", Level::TRACE));
        // an explicit user directive for the same target loses to the cap
        let g = build_filter(Some("tungstenite=trace,pulse_client=trace"));
        assert!(!enabled(g, "tungstenite", Level::TRACE));
    }

    #[test]
    fn unset_or_invalid_pulse_log_falls_back_to_the_default() {
        for user in [None, Some("=== not a filter ===[")] {
            assert!(!enabled(
                build_filter(user),
                "pulse_client::gateway",
                Level::TRACE
            ));
            assert!(!enabled(build_filter(user), "tungstenite", Level::DEBUG));
        }
    }

    #[test]
    fn redact_home_replaces_the_home_prefix_only() {
        let home = Path::new("/home/alex");
        assert_eq!(
            redact_home_in(Path::new("/home/alex/.local/share/pulse"), Some(home)),
            "~/.local/share/pulse"
        );
        assert_eq!(redact_home_in(Path::new("/home/alex"), Some(home)), "~");
        // a sibling with the same string prefix is not under home
        assert_eq!(
            redact_home_in(Path::new("/home/alexander/x"), Some(home)),
            "/home/alexander/x"
        );
        assert_eq!(
            redact_home_in(Path::new("/run/user/1000/pulse.sock"), Some(home)),
            "/run/user/1000/pulse.sock"
        );
        assert_eq!(redact_home_in(Path::new("/a/b"), None), "/a/b");
    }

    #[test]
    fn session_marker_is_a_fixed_string() {
        assert_eq!(SESSION_MARKER, "pulse-session-start");
    }
}
