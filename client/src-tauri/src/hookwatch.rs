//! Windows silently unhooks a low-level hook whose callback overruns LowLevelHooksTimeout (and on
//! some versions after repeated overruns), with no notification: global binds just stop working.
//! The only symptom is input the hooks never saw, so a watchdog compares the session's last-input
//! tick (`GetLastInputInfo`) with the last tick a hook saw and reinstalls when they disagree.
//! Some input the hooks can never see (an elevated window in front, a remapper earlier in the
//! chain eating keys), so a reinstall the hooks stay blind after isn't repeated until they see
//! something again. The decision lives here, free of Windows, so it's tested everywhere.

use std::time::{Duration, Instant};

/// Whether the system saw input more than `margin_ms` after the hooks last did. Ticks are
/// `GetTickCount`-style milliseconds that wrap every ~49.7 days; the difference is read as signed,
/// so a wrap between the two ticks is fine and hooks newer than the system (a race) are alive.
pub fn hooks_look_dead(last_input_ms: u32, last_hook_ms: u32, margin_ms: u32) -> bool {
    let ahead = last_input_ms.wrapping_sub(last_hook_ms) as i32;
    i64::from(ahead) > i64::from(margin_ms)
}

/// What the watchdog should do after a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// Hooks look alive, or it's too soon to tell.
    Fine,
    /// Hooks look dead: reinstall them.
    Reinstall,
    /// Input kept coming after a reinstall and the hooks saw none of it: input they can't see,
    /// not a drop. No more reinstalls until a hook event comes in. Returned once per episode.
    GaveUp,
}

/// Where a reinstall left the hooks' and the system's ticks, to tell what happened since.
#[derive(Clone, Copy)]
struct Reinstalled {
    hook_ms: u32,
    input_ms: u32,
    gave_up: bool,
}

/// `hooks_look_dead` plus a rate limit (at most one reinstall per `min_gap`) and a backoff: input
/// the hooks never see costs one reinstall, not one every `min_gap`.
pub struct HookWatch {
    margin_ms: u32,
    min_gap: Duration,
    last_reinstall: Option<Instant>,
    /// Set by a reinstall, cleared once a hook event shows the hooks are seeing input.
    pending: Option<Reinstalled>,
}

impl HookWatch {
    pub fn new(margin_ms: u32, min_gap: Duration) -> Self {
        Self {
            margin_ms,
            min_gap,
            last_reinstall: None,
            pending: None,
        }
    }

    /// `last_hook_ms` must only move on real hook events (not on the reinstall itself), or
    /// the backoff can't tell a working reinstall from a blind one.
    pub fn check(&mut self, last_input_ms: u32, last_hook_ms: u32, now: Instant) -> Check {
        if let Some(p) = &mut self.pending {
            if last_hook_ms != p.hook_ms {
                // the hooks saw something since the reinstall: re-armed
                self.pending = None;
            } else if p.gave_up {
                return Check::Fine;
            } else if last_input_ms != p.input_ms {
                p.gave_up = true;
                return Check::GaveUp;
            } else {
                // no input since the reinstall yet: can't tell
                return Check::Fine;
            }
        }
        if !hooks_look_dead(last_input_ms, last_hook_ms, self.margin_ms) {
            return Check::Fine;
        }
        if let Some(at) = self.last_reinstall
            && now.saturating_duration_since(at) < self.min_gap
        {
            return Check::Fine;
        }
        self.last_reinstall = Some(now);
        self.pending = Some(Reinstalled {
            hook_ms: last_hook_ms,
            input_ms: last_input_ms,
            gave_up: false,
        });
        Check::Reinstall
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
        assert_eq!(w.check(50_000, 50_000, t), Check::Fine);
        assert_eq!(w.check(60_000, 50_000, t), Check::Reinstall);
    }

    #[test]
    fn watch_reinstalls_at_most_once_per_gap() {
        let mut w = HookWatch::new(MARGIN, GAP);
        let t = Instant::now();
        assert_eq!(w.check(60_000, 50_000, t), Check::Reinstall);
        // the reinstall worked (hooks saw 61_000), then they were dropped again
        let s = Duration::from_secs;
        assert_eq!(w.check(65_000, 61_000, t + s(5)), Check::Fine);
        assert_eq!(
            w.check(69_000, 61_000, t + Duration::from_millis(9_999)),
            Check::Fine
        );
        assert_eq!(w.check(70_000, 61_000, t + GAP), Check::Reinstall);
    }

    #[test]
    fn alive_checks_dont_use_up_the_rate_limit() {
        let mut w = HookWatch::new(MARGIN, GAP);
        let t = Instant::now();
        assert_eq!(w.check(50_000, 50_000, t), Check::Fine);
        assert_eq!(
            w.check(60_000, 50_000, t + Duration::from_secs(1)),
            Check::Reinstall
        );
    }

    #[test]
    fn input_the_hooks_never_see_costs_one_reinstall_then_quiet() {
        let mut w = HookWatch::new(MARGIN, GAP);
        let t = Instant::now();
        let s = Duration::from_secs;
        assert_eq!(w.check(60_000, 50_000, t), Check::Reinstall);
        assert_eq!(w.check(63_000, 50_000, t + s(3)), Check::GaveUp);
        for n in 2..20 {
            let input = 60_000 + n * 3_000;
            assert_eq!(w.check(input, 50_000, t + s(3 * n as u64)), Check::Fine);
        }
    }

    #[test]
    fn no_new_input_after_a_reinstall_is_waiting_not_giving_up() {
        let mut w = HookWatch::new(MARGIN, GAP);
        let t = Instant::now();
        let s = Duration::from_secs;
        assert_eq!(w.check(60_000, 50_000, t), Check::Reinstall);
        assert_eq!(w.check(60_000, 50_000, t + s(3)), Check::Fine);
        assert_eq!(w.check(60_000, 50_000, t + s(30)), Check::Fine);
        // input arrives and the hooks miss it: now it's blind
        assert_eq!(w.check(91_000, 50_000, t + s(33)), Check::GaveUp);
    }

    #[test]
    fn a_hook_event_rearms_after_giving_up() {
        let mut w = HookWatch::new(MARGIN, GAP);
        let t = Instant::now();
        let s = Duration::from_secs;
        assert_eq!(w.check(60_000, 50_000, t), Check::Reinstall);
        assert_eq!(w.check(63_000, 50_000, t + s(3)), Check::GaveUp);
        assert_eq!(w.check(66_000, 50_000, t + s(6)), Check::Fine);
        // hooks see input again (the elevated window went away)
        assert_eq!(w.check(70_000, 70_000, t + s(12)), Check::Fine);
        // later Windows really drops them: recovered
        assert_eq!(w.check(90_000, 75_000, t + s(30)), Check::Reinstall);
    }

    #[test]
    fn a_real_drop_recovers_and_a_later_drop_recovers_again() {
        let mut w = HookWatch::new(MARGIN, GAP);
        let t = Instant::now();
        let s = Duration::from_secs;
        assert_eq!(w.check(60_000, 50_000, t), Check::Reinstall);
        // the reinstalled hooks see the next input
        assert_eq!(w.check(63_000, 63_000, t + s(3)), Check::Fine);
        assert_eq!(w.check(80_000, 79_500, t + s(20)), Check::Fine);
        assert_eq!(w.check(90_000, 80_000, t + s(30)), Check::Reinstall);
        assert_eq!(w.check(93_000, 93_000, t + s(33)), Check::Fine);
    }
}
