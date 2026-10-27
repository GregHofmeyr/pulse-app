//! Log file in the app's log folder (per profile), so a Windows test session can be read back
//! afterwards from Linux. Panics are logged too.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// Logged at process start and on each sign-in / restore. Report log tails start at the last one, so a report holds
/// the current session only.
pub const SESSION_MARKER: &str = "pulse-session-start";

/// Its own log target, which `build_filter` always enables, so `PULSE_LOG` can't drop the marker.
pub const SESSION_TARGET: &str = "pulse_session";

pub fn session_start() {
    // A root event (no parent span), so the line has a fixed shape whatever span the caller is
    // in: `<timestamp>  INFO pulse_session: pulse-session-start`. See `is_session_marker_line`.
    tracing::info!(target: SESSION_TARGET, parent: None, "{SESSION_MARKER}");
}

/// How long one `ui_log` line can be (bytes).
pub const UI_LINE_MAX: usize = 1024;

/// Characters that would let a line pass for something else: control characters (a newline could
/// forge a whole line, even the session marker report tails start at), line/paragraph separators,
/// and bidi controls (they reorder what a viewer shows).
fn misleading(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{2028}' | '\u{2029}' | '\u{200E}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Text from the webview as one log line: `misleading` characters become spaces, and it's cut to
/// `max` bytes on a char boundary.
pub fn one_line(line: &str, max: usize) -> String {
    let mut out = String::with_capacity(line.len().min(max));
    for c in line.chars() {
        let c = if misleading(c) { ' ' } else { c };
        if out.len() + c.len_utf8() > max {
            break;
        }
        out.push(c);
    }
    out
}

/// A token bucket for lines the webview asks us to log: a loop there mustn't use up the day's
/// log budget (`Capped`) and push real lines out.
pub struct LineBudget {
    burst: f64,
    per_sec: f64,
    tokens: f64,
    last: std::time::Instant,
    dropped: u64,
}

impl LineBudget {
    pub fn new(burst: u32, per_sec: f64, now: std::time::Instant) -> Self {
        Self {
            burst: f64::from(burst),
            per_sec,
            tokens: f64::from(burst),
            last: now,
            dropped: 0,
        }
    }

    /// `Some(n)`: log this line, after saying `n` were dropped since the last one that was. `None`: drop it.
    pub fn take(&mut self, now: std::time::Instant) -> Option<u64> {
        let gained = now.saturating_duration_since(self.last).as_secs_f64() * self.per_sec;
        self.tokens = (self.tokens + gained).min(self.burst);
        self.last = now;
        if self.tokens < 1.0 {
            self.dropped += 1;
            return None;
        }
        self.tokens -= 1.0;
        Some(std::mem::take(&mut self.dropped))
    }
}

/// Logs `line` (already made one line) at warn under `budget`, first saying how many were dropped.
pub fn log_budgeted(
    budget: &std::sync::Mutex<Option<LineBudget>>,
    burst: u32,
    per_sec: f64,
    emit: impl FnOnce(),
    dropped: impl FnOnce(u64),
) {
    let now = std::time::Instant::now();
    let verdict = budget
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(|| LineBudget::new(burst, per_sec, now))
        .take(now);
    match verdict {
        None => {}
        Some(0) => emit(),
        Some(n) => {
            dropped(n);
            emit();
        }
    }
}

static UI_BUDGET: std::sync::Mutex<Option<LineBudget>> = std::sync::Mutex::new(None);

/// Something the UI decided or handled on its own (a refused emoji name at warn, why a message
/// did or didn't ding at `info`), so the log shows it too. The page says why, never what was
/// typed. Up to 20 at once, then 2 a second.
#[tauri::command]
pub fn ui_log(line: String, info: Option<bool>) {
    let line = one_line(&line, UI_LINE_MAX);
    log_budgeted(
        &UI_BUDGET,
        20,
        2.0,
        || {
            if info == Some(true) {
                tracing::info!(target: "ui", "ui: {line}")
            } else {
                tracing::warn!(target: "ui", "ui: {line}")
            }
        },
        |n| tracing::warn!(target: "ui", "ui: {n} lines dropped (too many)"),
    );
}

/// Whether `line` (one log line, with or without its line ending) is the session marker exactly
/// as the log writes it: an optional timestamp, then `INFO pulse_session: pulse-session-start`
/// and nothing else. Any other line that merely contains the marker text is not.
pub fn is_session_marker_line(line: &str) -> bool {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line.contains(['\n', '\r']) {
        return false;
    }
    let target = format!("{SESSION_TARGET}:");
    let words: Vec<&str> = line.split_whitespace().collect();
    let tail = ["INFO", target.as_str(), SESSION_MARKER];
    match words.as_slice() {
        [_, rest @ ..] if rest == tail => true, // with the timestamp
        rest => rest == tail,
    }
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

/// The log filter: `user` (the `PULSE_LOG` value) or the default, with the library caps applied
/// and the session marker's target always on. A cap only lowers verbosity to info: a stricter
/// level the user asked for (`warn`, `hyper=warn`, `off`) is kept.
pub fn build_filter(user: Option<&str>) -> EnvFilter {
    let capped = user
        .map(cap_libraries)
        .and_then(|s| EnvFilter::try_new(s).ok());
    let mut filter = capped.unwrap_or_else(|| EnvFilter::new("info,livekit=warn,libwebrtc=warn"));
    // appended last: a target directive beats a global `off`/`warn`, and nothing else names it
    if let Ok(d) = format!("{SESSION_TARGET}=info").parse() {
        filter = filter.add_directive(d);
    }
    filter
}

fn is_capped(target: &str) -> bool {
    CAPPED_TARGETS
        .iter()
        .any(|t| target == *t || target.strip_prefix(t).is_some_and(|r| r.starts_with("::")))
}

/// Rewrites a `PULSE_LOG` value so no capped library (or one of its modules) is above info, and
/// no span/field directive is above info (see `cap_span_directive`).
fn cap_libraries(spec: &str) -> String {
    use tracing_subscriber::filter::LevelFilter;
    let info = LevelFilter::INFO;
    let mut parts: Vec<String> = Vec::new();
    let mut global_above_info = false;
    let mut named: Vec<&str> = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if part.contains(['[', '{']) {
            parts.push(cap_span_directive(part));
            continue;
        }
        let (target, level) = match part.split_once('=') {
            Some((t, l)) => (Some(t.trim()), l.trim().parse::<LevelFilter>().ok()),
            None => match part.parse::<LevelFilter>() {
                Ok(l) => (None, Some(l)), // a bare level is the global default
                Err(_) => (Some(part), Some(LevelFilter::TRACE)), // a bare target means trace
            },
        };
        match (target, level) {
            (None, Some(l)) => {
                global_above_info |= l > info;
                parts.push(part.to_string());
            }
            (Some(t), Some(l)) if is_capped(t) => {
                named.push(t);
                let l = if l > info { info } else { l };
                parts.push(format!("{t}={l}"));
            }
            _ => parts.push(part.to_string()),
        }
    }
    if global_above_info {
        for t in CAPPED_TARGETS {
            if !named.contains(&t) {
                parts.push(format!("{t}=info"));
            }
        }
    }
    parts.join(",")
}

/// A span/field directive (`target[span{field}]=level`) clamped to at most info. Inside a
/// matching span such a directive enables events of *every* target (EnvFilter checks the span
/// scope before the per-target directives), so a target cap can't hold against it: the only
/// safe level is info. A malformed one is left as written (and the whole value is then rejected).
fn cap_span_directive(part: &str) -> String {
    use tracing_subscriber::filter::LevelFilter;
    let Some(close) = part.rfind(']') else {
        return part.to_string();
    };
    let (head, rest) = part.split_at(close + 1);
    let level = match rest.trim() {
        "" => Some(LevelFilter::TRACE), // no level means trace
        r => r.strip_prefix('=').and_then(|l| l.trim().parse().ok()),
    };
    match level {
        Some(l) if l > LevelFilter::INFO => format!("{head}=info"),
        _ => part.to_string(),
    }
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
    let Some(rest) = home.and_then(|h| strip_home(path, h)) else {
        return path.display().to_string();
    };
    Path::new("~")
        .join(rest)
        .display()
        .to_string()
        .trim_end_matches(std::path::MAIN_SEPARATOR)
        .to_string()
}

/// `path` below `home`, compared component by component (case-insensitively on Windows, where
/// paths are), or `None` if it isn't below it.
fn strip_home(path: &Path, home: &Path) -> Option<std::path::PathBuf> {
    let same = |a: &std::ffi::OsStr, b: &std::ffi::OsStr| {
        if cfg!(windows) {
            a.to_string_lossy()
                .eq_ignore_ascii_case(&b.to_string_lossy())
        } else {
            a == b
        }
    };
    let mut p = path.components();
    for h in home.components() {
        if !same(p.next()?.as_os_str(), h.as_os_str()) {
            return None;
        }
    }
    Some(p.as_path().to_path_buf())
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
    session_start(); // a report sent before sign-in must not include the previous run
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
        // the one-line session marker is always kept: report tails are cut at it
        if buf.len() < 256 && std::str::from_utf8(buf).is_ok_and(is_session_marker_line) {
            self.inner.write_all(buf)?;
            return Ok(buf.len());
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

    #[test]
    fn a_ui_line_is_one_capped_line() {
        // a newline could forge a line (even the session marker that report tails cut at)
        assert_eq!(
            one_line(
                "emoji: save failed\n2026 INFO pulse_session: pulse-session-start\r\x1b[31m",
                UI_LINE_MAX
            ),
            "emoji: save failed 2026 INFO pulse_session: pulse-session-start  [31m"
        );
        // line/paragraph separators and bidi controls can't reorder or split it in a viewer either
        assert_eq!(
            one_line(
                "a\u{2028}b\u{2029}c\u{200E}\u{200F}\u{202A}\u{202E}\u{2066}\u{2069}d",
                100
            ),
            "a b c      d"
        );
        let long = "é".repeat(UI_LINE_MAX);
        let cut = one_line(&long, UI_LINE_MAX);
        assert!(cut.len() <= UI_LINE_MAX && cut.chars().all(|c| c == 'é'));
        assert_eq!(cut.len(), UI_LINE_MAX);
        let odd = format!("a{}", "é".repeat(10));
        assert_eq!(one_line(&odd, 4), "aé"); // byte 4 is mid-char: back off to 3
    }

    #[test]
    fn a_line_budget_allows_a_burst_then_a_steady_rate_and_counts_what_it_drops() {
        let t0 = std::time::Instant::now();
        let mut b = LineBudget::new(3, 1.0, t0);
        assert_eq!(b.take(t0), Some(0));
        assert_eq!(b.take(t0), Some(0));
        assert_eq!(b.take(t0), Some(0));
        assert_eq!(b.take(t0), None); // burst spent
        assert_eq!(b.take(t0), None);
        let t1 = t0 + std::time::Duration::from_millis(1000);
        assert_eq!(b.take(t1), Some(2)); // one refilled; says two were dropped
        assert_eq!(b.take(t1), None);
        // never more than the burst, however long it was quiet
        let t2 = t1 + std::time::Duration::from_secs(3600);
        assert_eq!(b.take(t2), Some(1));
        assert_eq!(b.take(t2), Some(0));
        assert_eq!(b.take(t2), Some(0));
        assert_eq!(b.take(t2), None);
    }

    #[test]
    fn ui_lines_reach_the_log_at_warn_by_default() {
        assert!(enabled(build_filter(None), "ui", Level::WARN));
    }

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
            ("pulse_session", Level::INFO) => {
                tracing::enabled!(target: "pulse_session", Level::INFO)
            }
            ("hyper", Level::TRACE) => tracing::enabled!(target: "hyper", Level::TRACE),
            ("hyper", Level::DEBUG) => tracing::enabled!(target: "hyper", Level::DEBUG),
            ("hyper", Level::INFO) => tracing::enabled!(target: "hyper", Level::INFO),
            ("hyper", Level::WARN) => tracing::enabled!(target: "hyper", Level::WARN),
            ("hyper_util", Level::TRACE) => tracing::enabled!(target: "hyper_util", Level::TRACE),
            ("hyper_util::client", Level::TRACE) => {
                tracing::enabled!(target: "hyper_util::client", Level::TRACE)
            }
            ("rustls", Level::DEBUG) => tracing::enabled!(target: "rustls", Level::DEBUG),
            ("ui", Level::WARN) => tracing::enabled!(target: "ui", Level::WARN),
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
        // an explicit user directive for the same target, or a child module, loses to the cap
        let g = build_filter(Some("tungstenite=trace,pulse_client=trace"));
        assert!(!enabled(g, "tungstenite", Level::TRACE));
        let h = build_filter(Some("hyper_util::client=trace"));
        assert!(!enabled(h, "hyper_util::client", Level::TRACE));
    }

    /// Like `enabled`, but checked inside an entered span named `span` (target `span_target`),
    /// so span directives (`[span]=level`, `target[span]=level`) take part.
    fn enabled_in_span(
        filter: EnvFilter,
        span_target: &'static str,
        target: &'static str,
        level: Level,
    ) -> bool {
        let sub = tracing_subscriber::registry().with(filter);
        tracing::subscriber::with_default(sub, || {
            let span = match span_target {
                "hyper" => tracing::info_span!(target: "hyper", "x"),
                "pulse_client" => tracing::info_span!(target: "pulse_client", "gateway"),
                _ => panic!("unprobed span target {span_target}"),
            };
            let _in = span.enter();
            probe(target, level)
        })
    }

    #[test]
    fn span_directives_cannot_raise_library_targets_above_info() {
        // A span directive naming a capped target: events inside its span stay at most info.
        for spec in ["hyper[x]=trace", "hyper[x]", "hyper[x]=debug"] {
            let f = || build_filter(Some(spec));
            assert!(
                !enabled_in_span(f(), "hyper", "hyper", Level::TRACE),
                "{spec}"
            );
            assert!(
                !enabled_in_span(f(), "hyper", "hyper", Level::DEBUG),
                "{spec}"
            );
            assert!(
                !enabled_in_span(f(), "hyper", "tungstenite", Level::TRACE),
                "{spec}"
            );
        }
        // Still honoured up to info.
        let f = build_filter(Some("hyper[x]=trace"));
        assert!(enabled_in_span(f, "hyper", "hyper", Level::INFO));
        // A target-less span directive applies to every target inside the span: clamped too.
        for spec in [
            "[gateway]=trace",
            "[gateway]",
            "info,[gateway]=trace",
            "[gateway{id}]=trace",
        ] {
            let f = || build_filter(Some(spec));
            for (target, level) in [
                ("tungstenite", Level::TRACE),
                ("tungstenite", Level::DEBUG),
                ("hyper", Level::TRACE),
                ("hyper_util::client", Level::TRACE),
                ("rustls", Level::DEBUG),
            ] {
                assert!(
                    !enabled_in_span(f(), "pulse_client", target, level),
                    "{spec}: {target} {level}"
                );
            }
        }
        // A stricter span level is kept.
        let f = build_filter(Some("[gateway]=warn"));
        assert!(!enabled_in_span(
            f,
            "pulse_client",
            "tungstenite",
            Level::INFO
        ));
    }

    #[test]
    fn the_caps_never_raise_a_stricter_level() {
        for spec in ["warn", "error", "off"] {
            assert!(
                !enabled(build_filter(Some(spec)), "tungstenite", Level::INFO),
                "{spec}: tungstenite must stay below info"
            );
            assert!(!enabled(
                build_filter(Some(spec)),
                "hyper_util",
                Level::TRACE
            ));
        }
        assert!(!enabled(
            build_filter(Some("hyper=warn")),
            "hyper",
            Level::INFO
        ));
        assert!(enabled(
            build_filter(Some("hyper=warn")),
            "hyper",
            Level::WARN
        ));
        // a stricter library level next to a verbose global one
        let f = build_filter(Some("trace,tungstenite=warn"));
        assert!(!enabled(f, "tungstenite", Level::INFO));
    }

    #[test]
    fn the_session_marker_survives_any_pulse_log() {
        for spec in [None, Some("warn"), Some("error"), Some("off")] {
            assert!(
                enabled(build_filter(spec), SESSION_TARGET, Level::INFO),
                "{spec:?}"
            );
        }
        assert_eq!(SESSION_TARGET, "pulse_session");
        assert!(!SESSION_TARGET.contains(SESSION_MARKER));
    }

    /// What `session_start` writes through the real fmt layer (as `init` sets it up), called
    /// inside a span: the marker line must not depend on the caller's span context.
    fn formatted_marker() -> String {
        use std::sync::{Arc, Mutex};
        let out = Arc::new(Mutex::new(Vec::new()));
        let sink = out.clone();
        let sub = tracing_subscriber::fmt()
            .with_writer(move || Sink(sink.clone()))
            .with_ansi(false)
            .with_env_filter(build_filter(None))
            .finish();
        tracing::subscriber::with_default(sub, || {
            let span = tracing::info_span!("sign_in", note = "pulse_session: x");
            let _in = span.enter();
            session_start();
        });
        String::from_utf8(out.lock().unwrap().clone()).unwrap()
    }

    struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl Write for Sink {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_size_cap_lets_the_session_marker_through() {
        DAY.store(1, Ordering::SeqCst);
        let marker = formatted_marker();
        assert!(marker.contains(SESSION_MARKER), "{marker}");
        let mut w = Capped::new(Vec::new(), 10, fake_day);
        w.write_all(b"12345\n").unwrap();
        w.write_all(b"67890\n").unwrap(); // cap reached
        w.write_all(marker.as_bytes()).unwrap();
        w.write_all(b"after\n").unwrap();
        let text = String::from_utf8(w.inner.clone()).unwrap();
        assert!(text.ends_with(&marker), "{text}");
        assert!(!text.contains("after"), "{text}");
    }

    /// Any other short line that merely contains the marker text (a logged server message, a
    /// file name, an error string) is capped like everything else.
    #[test]
    fn a_line_merely_containing_the_marker_does_not_bypass_the_cap() {
        DAY.store(1, Ordering::SeqCst);
        let mut w = Capped::new(Vec::new(), 10, fake_day);
        w.write_all(b"12345\n").unwrap();
        w.write_all(b"67890\n").unwrap(); // cap reached
        for spoof in [
            "pulse-session-start\n",
            "2026-10-26T10:00:00.000000Z  INFO pulse_client::gateway: pulse-session-start\n",
            "2026-10-26T10:00:00.000000Z  INFO pulse_client: said pulse_session: pulse-session-start\n",
            "2026-10-26T10:00:00.000000Z  WARN pulse_session: pulse-session-start\n",
            "2026-10-26T10:00:00.000000Z  INFO pulse_session: pulse-session-start and more\n",
            "x\n2026-10-26T10:00:00.000000Z  INFO pulse_session: pulse-session-start\n",
        ] {
            w.write_all(spoof.as_bytes()).unwrap();
        }
        let text = String::from_utf8(w.inner.clone()).unwrap();
        assert!(!text.contains(SESSION_MARKER), "{text}");
    }

    const MARKER_CHILD: &str = "PULSE_TEST_MARKER_CHILD";

    /// Runs only as the child of `init_writes_a_session_marker_even_under_pulse_log_error`.
    #[test]
    fn marker_child() {
        let Some(dir) = std::env::var_os(MARKER_CHILD) else {
            return;
        };
        drop(init(Path::new(&dir)).unwrap());
    }

    #[test]
    fn init_writes_a_session_marker_even_under_pulse_log_error() {
        let dir = tempfile::tempdir().unwrap();
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "logging::tests::marker_child",
                "--test-threads",
                "1",
            ])
            .env(MARKER_CHILD, dir.path())
            .env("PULSE_LOG", "error")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        let log: String = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|f| f.file_name().to_string_lossy().starts_with("pulse"))
            .map(|f| std::fs::read_to_string(f.path()).unwrap_or_default())
            .collect();
        assert_eq!(log.matches(SESSION_MARKER).count(), 1, "{log}");
        assert!(
            !log.contains("Pulse starting"),
            "info is filtered out: {log}"
        );
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
        let home = Path::new(if cfg!(windows) {
            "C:\\Users\\alex"
        } else {
            "/home/alex"
        });
        let tilde = |rest: &[&str]| {
            rest.iter()
                .fold(std::path::PathBuf::from("~"), |p, s| p.join(s))
                .display()
                .to_string()
        };
        let under = |rest: &[&str]| rest.iter().fold(home.to_path_buf(), |p, s| p.join(s));
        assert_eq!(
            redact_home_in(&under(&[".local", "share", "pulse"]), Some(home)),
            tilde(&[".local", "share", "pulse"])
        );
        assert_eq!(redact_home_in(home, Some(home)), "~");
        // a sibling with the same string prefix is not under home
        let sibling = home.with_file_name("alexander").join("x");
        assert_eq!(
            redact_home_in(&sibling, Some(home)),
            sibling.display().to_string()
        );
        let elsewhere = Path::new(if cfg!(windows) {
            "D:\\data\\p"
        } else {
            "/run/user/1000/p"
        });
        assert_eq!(
            redact_home_in(elsewhere, Some(home)),
            elsewhere.display().to_string()
        );
        assert_eq!(redact_home_in(Path::new("a"), None), "a");
    }

    #[cfg(windows)]
    #[test]
    fn redact_home_ignores_case_on_windows() {
        let home = Path::new("C:\\Users\\alex");
        assert_eq!(
            redact_home_in(Path::new("c:\\users\\ALEX\\AppData"), Some(home)),
            "~\\AppData"
        );
    }

    #[test]
    fn session_marker_is_a_fixed_string() {
        assert_eq!(SESSION_MARKER, "pulse-session-start");
    }
}
