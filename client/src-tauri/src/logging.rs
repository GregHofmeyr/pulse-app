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
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_env("PULSE_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,livekit=warn,libwebrtc=warn"));
    // try_init: tests / a second call must not panic on an already-set global subscriber
    let _ = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(filter)
        .try_init();

    std::panic::set_hook(Box::new(|info| {
        tracing::error!(panic = %info, "PANIC");
    }));
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        "Pulse starting"
    );
    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;

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
