//! Windows silently unhooks a low-level hook whose callback overruns LowLevelHooksTimeout (and on
//! some versions after repeated overruns), with no notification: global binds just stop working.
//! The only symptom is input the hooks never saw, so a watchdog compares the session's last-input
//! tick (`GetLastInputInfo`) with the last tick a hook saw and reinstalls when they disagree.
//! The decision lives here, free of Windows, so it's tested everywhere.

use std::time::{Duration, Instant};

/// Whether the system saw input more than `margin_ms` after the hooks last did. Ticks are
/// `GetTickCount`-style milliseconds that wrap every ~49.7 days; the difference is read as signed,
/// so a wrap between the two ticks is fine and hooks newer than the system (a race) are alive.
pub fn hooks_look_dead(last_input_ms: u32, last_hook_ms: u32, margin_ms: u32) -> bool {
    let ahead = last_input_ms.wrapping_sub(last_hook_ms) as i32;
    i64::from(ahead) > i64::from(margin_ms)
}

/// `hooks_look_dead` plus a rate limit, so input the hooks can never see (if some source like
/// that exists) costs one reinstall per `min_gap`, not a reinstall loop.
pub struct HookWatch {
    margin_ms: u32,
    min_gap: Duration,
    last_reinstall: Option<Instant>,
}

impl HookWatch {
    pub fn new(margin_ms: u32, min_gap: Duration) -> Self {
        Self {
            margin_ms,
            min_gap,
            last_reinstall: None,
        }
    }

    /// Whether to reinstall now; a `true` counts as a reinstall for the rate limit.
    pub fn check(&mut self, last_input_ms: u32, last_hook_ms: u32, now: Instant) -> bool {
        if !hooks_look_dead(last_input_ms, last_hook_ms, self.margin_ms) {
            return false;
        }
        if let Some(at) = self.last_reinstall
            && now.saturating_duration_since(at) < self.min_gap
        {
            return false;
        }
        self.last_reinstall = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARGIN: u32 = 1000;
    const GAP: Duration = Duration::from_secs(10);

    #[test]
    fn hooks_that_saw_the_last_input_are_alive() {
        assert!(!hooks_look_dead(50_000, 50_000, MARGIN));
    }

    #[test]
    fn input_well_after_the_hooks_last_saw_any_means_dead() {
        assert!(hooks_look_dead(60_000, 50_000, MARGIN));
    }

    #[test]
    fn input_within_the_margin_is_a_race_not_death() {
        assert!(!hooks_look_dead(51_000, 50_000, MARGIN));
        assert!(hooks_look_dead(51_001, 50_000, MARGIN));
    }

    #[test]
    fn hooks_newer_than_the_system_tick_are_alive() {
        assert!(!hooks_look_dead(50_000, 52_000, MARGIN));
    }

    #[test]
    fn the_tick_wrapping_between_hook_and_input_still_reads_right() {
        // hook saw input just before the wrap, system input 3 s later, after it: dead
        assert!(hooks_look_dead(2_000, u32::MAX - 999, MARGIN));
        // hook saw input just before the wrap, system input 0.5 s later, after it: alive
        assert!(!hooks_look_dead(400, u32::MAX - 99, MARGIN));
        // hook saw input just after the wrap, system tick still before it: alive
        assert!(!hooks_look_dead(u32::MAX - 10, 5, MARGIN));
    }

    #[test]
    fn watch_reinstalls_when_dead_and_not_when_alive() {
        let mut w = HookWatch::new(MARGIN, GAP);
        let t = Instant::now();
        assert!(!w.check(50_000, 50_000, t));
        assert!(w.check(60_000, 50_000, t));
    }

    #[test]
    fn watch_reinstalls_at_most_once_per_gap() {
        let mut w = HookWatch::new(MARGIN, GAP);
        let t = Instant::now();
        assert!(w.check(60_000, 50_000, t));
        assert!(!w.check(65_000, 50_000, t + Duration::from_secs(5)));
        assert!(!w.check(69_000, 50_000, t + Duration::from_millis(9_999)));
        assert!(w.check(70_000, 50_000, t + GAP));
    }

    #[test]
    fn alive_checks_dont_use_up_the_rate_limit() {
        let mut w = HookWatch::new(MARGIN, GAP);
        let t = Instant::now();
        assert!(!w.check(50_000, 50_000, t));
        assert!(w.check(60_000, 50_000, t + Duration::from_secs(1)));
    }
}
