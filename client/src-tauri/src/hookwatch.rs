//! Windows silently unhooks a low-level hook whose callback overruns LowLevelHooksTimeout (and on
//! some versions after repeated overruns), with no notification: global binds just stop working.
//! The only symptom is input the hooks never saw, so a watchdog compares the session's last-input
//! tick (`GetLastInputInfo`) with the last tick a hook saw and reinstalls when they disagree.
//! Some input the hooks can never see (an elevated window in front, a remapper earlier in the
//! chain eating keys), so a reinstall the hooks stay blind after isn't repeated until they see
//! something again. The decision lives here, free of Windows, so it's tested everywhere.
//!
//! The user can switch global binds off (Settings → Keybinds): then the hooks are removed, not just
//! ignored. `hook_step` is what the hook thread does with each message it gets, given the switch
//! as it reads at that moment.

use std::time::{Duration, Instant};

/// What the hook thread does on a message: "the switch changed" or the watchdog's "reinstall".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookStep {
    /// Switched on (or asked to reinstall while there are none): install them.
    Install,
    /// The watchdog thinks Windows dropped them: unhook and install again.
    Reinstall,
    /// Switched off: unhook both.
    Unhook,
    Nothing,
}

/// `enabled` is the switch as read when the message is handled (not when it was sent), so a burst
/// of on/off ends with the hooks matching the switch, and a watchdog request that raced an "off"
/// can't put the hooks back.
pub fn hook_step(enabled: bool, installed: bool, reinstall_asked: bool) -> HookStep {
    match (enabled, installed) {
        (true, false) => HookStep::Install,
        (true, true) if reinstall_asked => HookStep::Reinstall,
        (false, true) => HookStep::Unhook,
        _ => HookStep::Nothing,
    }
}

/// What the watchdog does on a tick, given the switch, whether the hook thread holds hooks, and
/// whether their install failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Hooks are in and should be: check they still see input.
    Watch,
    /// Hooks and switch disagree (a WM_SWITCH was lost, or is still on its way): send it again.
    /// Harmless if the original is merely late: the hook thread reads the switch when it gets
    /// there, and a second one finds nothing to do.
    Resend,
    /// Nothing to watch: switched off with no hooks, or the install failed (reported; the user's
    /// off/on retries it).
    Idle,
}

pub fn watch_gate(enabled: bool, hooked: bool, install_failed: bool) -> Gate {
    match (enabled, hooked) {
        (true, true) => Gate::Watch,
        (false, true) => Gate::Resend,
        (true, false) if !install_failed => Gate::Resend,
        _ => Gate::Idle,
    }
}

/// What Settings shows. Off isn't a problem, but off with the hooks still in is (until the
/// watchdog's resend removes them), unless `switch_on_its_way`: the "off" was just delivered and
/// the hook thread hasn't got to it yet. On, it's the install failure, if any.
pub fn switch_problem(
    enabled: bool,
    hooked: bool,
    install_failed: Option<&str>,
    switch_on_its_way: bool,
) -> Option<String> {
    match (enabled, hooked) {
        (false, true) if switch_on_its_way => None,
        (false, true) => Some(
            "Keybinds are switched off but the keyboard and mouse hooks are still in; Pulse keeps trying to remove them (restart Pulse if this stays)".into(),
        ),
        (false, false) => None,
        (true, _) => install_failed.map(str::to_owned),
    }
}

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

    /// Forget the rate limit and backoff: the hooks were removed (switched off), and new ones
    /// start with a clean slate.
    pub fn reset(&mut self) {
        self.last_reinstall = None;
        self.pending = None;
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
    fn on_with_no_hooks_installs_them() {
        assert_eq!(hook_step(true, false, false), HookStep::Install);
        // a reinstall asked for while they're gone (raced an off/on) installs too
        assert_eq!(hook_step(true, false, true), HookStep::Install);
    }

    #[test]
    fn on_with_hooks_only_reinstalls_when_asked() {
        assert_eq!(hook_step(true, true, false), HookStep::Nothing);
        assert_eq!(hook_step(true, true, true), HookStep::Reinstall);
    }

    #[test]
    fn off_removes_hooks_and_a_watchdog_request_cant_bring_them_back() {
        assert_eq!(hook_step(false, true, false), HookStep::Unhook);
        // the watchdog saw hooks, then the switch went off before its request arrived
        assert_eq!(hook_step(false, true, true), HookStep::Unhook);
        assert_eq!(hook_step(false, false, true), HookStep::Nothing);
        assert_eq!(hook_step(false, false, false), HookStep::Nothing);
    }

    /// Rapid on/off/on: each change posts a message and every message reads the switch as it is
    /// when handled, so the last message leaves the hooks matching the last switch position.
    #[test]
    fn rapid_toggles_end_where_the_switch_ends() {
        for flips in [
            vec![true],
            vec![false],
            vec![true, false],
            vec![false, true],
            vec![true, false, true, false, true],
        ] {
            let last = *flips.last().unwrap();
            // messages may be handled after any number of later flips: worst case, all after
            let mut installed = false;
            for _ in &flips {
                match hook_step(last, installed, false) {
                    HookStep::Install => installed = true,
                    HookStep::Unhook => installed = false,
                    HookStep::Reinstall | HookStep::Nothing => {}
                }
            }
            assert_eq!(installed, last, "{flips:?}");
        }
    }

    /// A lost "off" (the post to the hook thread failed) mustn't leave hooks running while the
    /// switch says off: the watchdog re-sends the switch until the hooks match.
    #[test]
    fn hooked_but_switched_off_is_resent_until_the_hooks_go() {
        assert_eq!(watch_gate(false, true, false), Gate::Resend);
        assert_eq!(watch_gate(false, true, true), Gate::Resend);
        // and handling that resend unhooks
        assert_eq!(hook_step(false, true, false), HookStep::Unhook);
    }

    /// A lost "on" heals the same way; a failed install isn't retried every few seconds (the
    /// user is told, and off/on retries it).
    #[test]
    fn switched_on_without_hooks_is_resent_unless_the_install_failed() {
        assert_eq!(watch_gate(true, false, false), Gate::Resend);
        assert_eq!(watch_gate(true, false, true), Gate::Idle);
    }

    #[test]
    fn the_watchdog_only_watches_hooks_that_should_be_there() {
        assert_eq!(watch_gate(true, true, false), Gate::Watch);
        assert_eq!(watch_gate(false, false, false), Gate::Idle);
        assert_eq!(watch_gate(false, false, true), Gate::Idle);
    }

    /// Off isn't a problem, but off with the hooks still in is, and stays visible.
    #[test]
    fn hooks_still_in_while_off_is_a_problem() {
        assert_eq!(switch_problem(false, false, None, false), None);
        assert_eq!(switch_problem(false, false, Some("boom"), false), None);
        assert!(
            switch_problem(false, true, None, false)
                .unwrap()
                .contains("still")
        );
        assert_eq!(
            switch_problem(true, false, Some("boom"), false),
            Some("boom".into())
        );
        assert_eq!(switch_problem(true, true, None, false), None);
    }

    /// A normal "off": the message reached the hook thread, which hasn't unhooked yet. That's
    /// the switch on its way, not a problem; if it really gets stuck, Settings and the watchdog
    /// see it later.
    #[test]
    fn hooks_still_in_right_after_a_delivered_off_is_not_a_problem() {
        assert_eq!(switch_problem(false, true, None, true), None);
        // an install failure while switching on is still shown
        assert_eq!(
            switch_problem(true, false, Some("boom"), true),
            Some("boom".into())
        );
    }

    #[test]
    fn a_reset_watch_forgets_its_backoff() {
        let mut w = HookWatch::new(MARGIN, GAP);
        let t = Instant::now();
        assert_eq!(w.check(60_000, 50_000, t), Check::Reinstall);
        assert_eq!(
            w.check(63_000, 50_000, t + Duration::from_secs(3)),
            Check::GaveUp
        );
        // switched off and on: fresh hooks get a fresh watch (no rate limit, no backoff)
        w.reset();
        assert_eq!(
            w.check(90_000, 70_000, t + Duration::from_secs(4)),
            Check::Reinstall
        );
    }

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
