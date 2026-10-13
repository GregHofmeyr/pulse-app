//! Startup timeline: one log line per phase, the first time it's reached, with the time since the
//! process started. For finding where a slow launch goes (bug reports carry the log).

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();
static SEEN: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();

/// Call first thing in `run()`.
pub fn begin() {
    START.get_or_init(Instant::now);
}

/// Milliseconds since `begin`, the first time `phase` is reached; `None` after that (a reconnect
/// isn't startup).
pub fn first(phase: &'static str) -> Option<u64> {
    let start = *START.get_or_init(Instant::now);
    let seen = SEEN.get_or_init(Default::default);
    seen.lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(phase)
        .then(|| start.elapsed().as_millis() as u64)
}

/// Log `phase` if it's the first time.
pub fn mark(phase: &'static str) {
    if let Some(ms) = first(phase) {
        tracing::info!(phase, since_start_ms = ms, "startup");
    }
}

/// The UI calls this once it has painted its first screen.
#[tauri::command]
pub fn ui_painted(screen: String) {
    if let Some(ms) = first("ui_painted") {
        tracing::info!(phase = "ui_painted", %screen, since_start_ms = ms, "startup");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_phase_is_reported_once() {
        begin();
        assert!(first("test_phase").is_some());
        assert!(first("test_phase").is_none(), "a reconnect isn't startup");
        assert!(first("test_other_phase").is_some());
    }
}
