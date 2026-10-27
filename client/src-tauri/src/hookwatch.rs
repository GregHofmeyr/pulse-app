//! Windows silently unhooks a low-level hook whose callback overruns LowLevelHooksTimeout (and on
//! some versions after repeated overruns), with no notification: global binds just stop working.
//! The only symptom is input the hooks never saw, so a watchdog compares the session's last-input
//! tick (`GetLastInputInfo`) with the last tick a hook saw and reinstalls when they disagree.
//! Some input the hooks can never see (an elevated window in front, a remapper earlier in the
//! chain eating keys), so a reinstall the hooks stay blind after isn't repeated until they see
//! something again. The decision lives here, free of Windows, so it's tested everywhere.
//!
//! Hooks exist only while they're wanted (`hooks_wanted`): the user's switch (Settings → Keybinds)
//! is on AND something is bound. Otherwise they're removed, not just ignored. `hook_step` is what
//! the hook thread does with each message it gets, given what's wanted as it reads at that moment.

use std::time::{Duration, Instant};

/// What the hook thread does on a message: "the switch or the binds changed" or the watchdog's
/// "reinstall".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookStep {
    /// Wanted with none in (or asked to reinstall while there are none): install them.
    Install,
    /// The watchdog thinks Windows dropped them: unhook and install again.
    Reinstall,
    /// Not wanted any more (switched off, last bind removed): unhook both.
    Unhook,
    Nothing,
}

/// `wanted` is `hooks_wanted` as read when the message is handled (not when it was sent), so a
/// burst of changes ends with the hooks matching the last one, and a watchdog request that raced
/// an "off" (or the last bind going) can't put the hooks back.
pub fn hook_step(wanted: bool, installed: bool, reinstall_asked: bool) -> HookStep {
    match (wanted, installed) {
        (true, false) => HookStep::Install,
        (true, true) if reinstall_asked => HookStep::Reinstall,
        (false, true) => HookStep::Unhook,
        _ => HookStep::Nothing,
    }
}

/// Whether there should be hooks at all: the switch is on AND something is bound. With nothing
/// bound there's nothing to match, so the keyboard and mouse aren't watched (first run included).
/// Recording a bind doesn't need them: the Settings window records it from its own key events.
pub fn hooks_wanted(enabled: bool, has_binds: bool) -> bool {
    enabled && has_binds
}

/// What the hook thread holds, as one atomic so a reader never sees a half-made change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Hooks {
    /// No hooks.
    None = 0,
    /// Both hooks installed.
    In = 1,
    /// The watchdog's reinstall removed them and is (re)trying to put them back: no hooks yet.
    Retrying = 2,
}

impl Hooks {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::In,
            2 => Self::Retrying,
            _ => Self::None,
        }
    }
}

/// A snapshot of the hook machinery, for the decisions below.
#[derive(Debug, Clone, Copy)]
pub struct Seen<'a> {
    /// `hooks_wanted`, as it reads now.
    pub wanted: bool,
    pub hooks: Hooks,
    /// The hook thread started and is still running its message loop.
    pub thread_alive: bool,
    /// Why the last install failed, if it did (cleared by the next attempt).
    pub install_failed: Option<&'a str>,
    /// Failed attempts in the current watchdog reinstall (0 outside one).
    pub reinstall_failures: u32,
}

/// A reinstall failing this many times in a row (10 s apart) is shown in Settings: by then it's
/// not a blip, and the user's binds have been dead for a while.
pub const SHOW_REINSTALL_FAILURES_AFTER: u32 = 3;

/// What the watchdog does on a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Hooks are in and should be: check they still see input.
    Watch,
    /// Hooks and switch disagree (a WM_SWITCH was lost, or is still on its way): send it again.
    /// Harmless if the original is merely late: the hook thread reads the switch when it gets
    /// there, and a second one finds nothing to do.
    Resend,
    /// Nothing to watch: not wanted with no hooks, the install failed (reported; the user's
    /// off/on retries it), or there's no hook thread to talk to (reported). Forget the backoff.
    Idle,
    /// The hook thread is retrying a reinstall: no input polled, nothing sent (it isn't reading
    /// messages), and the backoff kept for the hooks it puts back.
    Wait,
}

pub fn watch_gate(s: &Seen) -> Gate {
    if !s.thread_alive {
        return Gate::Idle;
    }
    match (s.wanted, s.hooks) {
        (_, Hooks::Retrying) => Gate::Wait,
        (true, Hooks::In) => Gate::Watch,
        (false, Hooks::In) => Gate::Resend,
        (true, Hooks::None) if s.install_failed.is_none() => Gate::Resend,
        _ => Gate::Idle,
    }
}

/// What Settings shows. Off (or nothing bound) isn't a problem, but hooks still in then is (until
/// the watchdog's resend removes them), unless `switch_on_its_way`: the change was just delivered
/// and the hook thread hasn't got to it yet. Wanted, it's a missing hook thread or the install
/// failure, if any.
pub fn switch_problem(s: &Seen, switch_on_its_way: bool) -> Option<String> {
    if s.wanted && !s.thread_alive {
        return Some("Keybinds aren't running (their background thread couldn't start or stopped); restart Pulse to try again".into());
    }
    match (s.wanted, s.hooks) {
        (false, Hooks::In) if switch_on_its_way => None,
        (false, Hooks::In) => Some(
            "Global keybinds are off but the keyboard and mouse hooks are still in; Pulse keeps trying to remove them (restart Pulse if this stays)".into(),
        ),
        (false, _) => None,
        // the watchdog is putting them back; said so once it keeps failing
        (true, Hooks::Retrying) if s.reinstall_failures >= SHOW_REINSTALL_FAILURES_AFTER => Some(
            "Global keybinds stopped (Windows dropped the hooks) and couldn't be reinstalled; Pulse keeps retrying (restart Pulse if this stays)".into(),
        ),
        (true, Hooks::Retrying) => None,
        (true, _) => s.install_failed.map(str::to_owned),
    }
}

/// Whether the hook thread has caught up with a change: the hooks match what's wanted, or it
/// can't get there (the install failed, no thread). A settings change waits for this, briefly,
/// so its reply says what really happened.
pub fn settled(s: &Seen) -> bool {
    if !s.thread_alive {
        return true;
    }
    match s.hooks {
        Hooks::Retrying => false,
        Hooks::In => s.wanted,
        Hooks::None => !s.wanted || s.install_failed.is_some(),
    }
}

/// The reinstall retry's pause: `total` in `slice`s, stopping as soon as `wanted` says hooks
/// aren't wanted any more (switched off, last bind removed). Returns whether they still are.
pub fn retry_wait(
    total: Duration,
    slice: Duration,
    mut wanted: impl FnMut() -> bool,
    mut sleep: impl FnMut(Duration),
) -> bool {
    let mut left = total;
    while !left.is_zero() {
        let d = slice.min(left);
        sleep(d);
        left -= d;
        if !wanted() {
            return false;
        }
    }
    true
}

/// A run of failures to log once (and its end once), not on every retry.
#[derive(Debug, Default)]
pub struct Episode {
    failing: bool,
}

impl Episode {
    /// A failure: true if it starts a new run (log it).
    pub fn failed(&mut self) -> bool {
        !std::mem::replace(&mut self.failing, true)
    }

    /// A success: true if it ends a run (log the recovery).
    pub fn succeeded(&mut self) -> bool {
        std::mem::replace(&mut self.failing, false)
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

    /// Forget the rate limit and backoff: the hooks were removed (not wanted, or gone), and new ones
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

// ---- "my PC froze": what the hook machinery can measure ----
//
// Windows holds each input event, for the whole PC, until every low-level hook has answered (up
// to LowLevelHooksTimeout; past it the event goes on without that hook, which Windows then
// silently removes). So a freeze can be a slow hook chain (ours or one ahead of ours), our hook
// thread not getting the CPU, or the whole machine stalling. Each is measured separately.

/// The worst value noted since the last take, and how many reached `threshold`. From a hook
/// procedure: two relaxed atomics, nothing that can block.
pub struct MaxMeter {
    max: std::sync::atomic::AtomicU32,
    bad: std::sync::atomic::AtomicU32,
    threshold: u32,
}

impl MaxMeter {
    pub const fn new(threshold: u32) -> Self {
        Self {
            max: std::sync::atomic::AtomicU32::new(0),
            bad: std::sync::atomic::AtomicU32::new(0),
            threshold,
        }
    }

    pub fn note(&self, v: u32) {
        use std::sync::atomic::Ordering::Relaxed;
        self.max.fetch_max(v, Relaxed);
        if v >= self.threshold {
            self.bad.fetch_add(1, Relaxed);
        }
    }

    /// (worst, how many reached the threshold), and start over.
    pub fn take(&self) -> (u32, u32) {
        use std::sync::atomic::Ordering::Relaxed;
        (self.max.swap(0, Relaxed), self.bad.swap(0, Relaxed))
    }
}

/// A delay people feel as a stuck mouse or keyboard.
pub const LATE_INPUT_MS: u32 = 250;
/// Our own hook procedure should take microseconds; this long means it waited on something.
pub const OWN_SLOW_US: u32 = 20_000;
/// A longer "delay" is a stamp we can't believe (a tool's own `time`, or from before a sleep).
pub const MAX_PLAUSIBLE_MS: u32 = 60_000;

/// How late `now_ms` is after `stamp_ms` (GetTickCount-style, wrapping). A stamp just after `now`
/// (read in the other order) is on time; an implausible delay is `None`.
pub fn tick_delay(stamp_ms: u32, now_ms: u32) -> Option<u32> {
    let d = now_ms.wrapping_sub(stamp_ms);
    if d >= 1 << 31 {
        Some(0)
    } else if d > MAX_PLAUSIBLE_MS {
        None
    } else {
        Some(d)
    }
}

/// How long a sleep overran, in ms; an overrun past `MAX_PLAUSIBLE_MS` was the PC suspending,
/// not stalling.
pub fn overslept_ms(over: Duration) -> u32 {
    let ms = u32::try_from(over.as_millis()).unwrap_or(u32::MAX);
    if ms > MAX_PLAUSIBLE_MS { 0 } else { ms }
}

/// One watchdog tick's worst of each.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stall {
    /// Input reaching our hook late: the hook chain ahead of us, our thread waking, or the PC.
    pub input_late_ms: u32,
    /// How many input events were that late.
    pub late_events: u32,
    /// Our hook procedure's own run time.
    pub own_us: u32,
    /// The hook thread picking up a message (no input involved): its scheduling delay.
    pub hook_thread_ms: u32,
    /// The watchdog's own sleep running long: the whole machine stalled.
    pub overslept_ms: u32,
}

impl Stall {
    pub fn felt(&self) -> bool {
        self.input_late_ms >= LATE_INPUT_MS
            || self.own_us >= OWN_SLOW_US
            || self.hook_thread_ms >= LATE_INPUT_MS
            || self.overslept_ms >= LATE_INPUT_MS
    }

    fn merge(&mut self, o: Stall) {
        self.input_late_ms = self.input_late_ms.max(o.input_late_ms);
        self.late_events = self.late_events.saturating_add(o.late_events);
        self.own_us = self.own_us.max(o.own_us);
        self.hook_thread_ms = self.hook_thread_ms.max(o.hook_thread_ms);
        self.overslept_ms = self.overslept_ms.max(o.overslept_ms);
    }
}

/// What to log after a tick.
#[derive(Debug, PartialEq, Eq)]
pub enum Emit {
    Nothing,
    /// The first bad tick of an episode, as it was.
    Started(Stall),
    /// Still going: the worst since the last line.
    Ongoing {
        since_last: Stall,
        lasted: Duration,
    },
    /// Over: the worst since the last line, and how long it lasted.
    Ended {
        since_last: Stall,
        lasted: Duration,
    },
}

/// How often an ongoing stall gets a summary line.
pub const STALL_SUMMARY_EVERY: Duration = Duration::from_secs(60);

/// A line when a stall starts, a summary a minute while it lasts, one when it ends: a long one
/// mustn't push the rest out of the log a report carries.
#[derive(Default)]
pub struct StallLog {
    episode: Option<StallEpisode>,
}

struct StallEpisode {
    started: Instant,
    last_line: Instant,
    since_last: Stall,
}

impl StallLog {
    pub fn tick(&mut self, s: Stall, now: Instant) -> Emit {
        match (&mut self.episode, s.felt()) {
            (None, false) => Emit::Nothing,
            (None, true) => {
                self.episode = Some(StallEpisode {
                    started: now,
                    last_line: now,
                    since_last: Stall::default(),
                });
                Emit::Started(s)
            }
            (Some(e), true) => {
                e.since_last.merge(s);
                if now.saturating_duration_since(e.last_line) < STALL_SUMMARY_EVERY {
                    return Emit::Nothing;
                }
                e.last_line = now;
                Emit::Ongoing {
                    since_last: std::mem::take(&mut e.since_last),
                    lasted: now.saturating_duration_since(e.started),
                }
            }
            (Some(_), false) => {
                let e = self.episode.take().expect("matched Some");
                Emit::Ended {
                    since_last: e.since_last,
                    lasted: now.saturating_duration_since(e.started),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARGIN: u32 = 1000;
    const GAP: Duration = Duration::from_secs(10);

    #[test]
    fn a_meter_keeps_the_worst_and_counts_the_bad_until_taken() {
        let m = MaxMeter::new(250);
        assert_eq!(m.take(), (0, 0));
        m.note(40);
        m.note(900);
        m.note(300);
        assert_eq!(m.take(), (900, 2));
        assert_eq!(m.take(), (0, 0)); // taken: starts over
    }

    #[test]
    fn tick_delay_wraps_and_drops_stamps_it_cant_believe() {
        assert_eq!(tick_delay(1_000, 1_040), Some(40));
        // the tick count wraps after ~49.7 days
        assert_eq!(tick_delay(u32::MAX - 10, 290), Some(301));
        // stamped a moment after "now" (the clock read before the stamp): on time
        assert_eq!(tick_delay(5_000, 4_990), Some(0));
        // a remapper's own stamp, or one from before a sleep: not a real delay
        assert_eq!(tick_delay(0, 61_000), None);
        assert_eq!(
            tick_delay(1_000, 1_000 + MAX_PLAUSIBLE_MS),
            Some(MAX_PLAUSIBLE_MS)
        );
    }

    fn stall(input_late_ms: u32) -> Stall {
        Stall {
            input_late_ms,
            late_events: u32::from(input_late_ms >= LATE_INPUT_MS),
            ..Stall::default()
        }
    }

    #[test]
    fn oversleeping_counts_unless_the_pc_was_asleep() {
        assert_eq!(overslept_ms(Duration::from_millis(400)), 400);
        assert_eq!(
            overslept_ms(Duration::from_millis(u64::from(MAX_PLAUSIBLE_MS))),
            MAX_PLAUSIBLE_MS
        );
        assert_eq!(overslept_ms(Duration::from_secs(3600)), 0); // suspended, not stalled
    }

    #[test]
    fn a_stall_is_anything_people_would_feel_or_our_own_slowness() {
        assert!(!Stall::default().felt());
        assert!(!stall(LATE_INPUT_MS - 1).felt());
        assert!(stall(LATE_INPUT_MS).felt());
        assert!(
            Stall {
                own_us: OWN_SLOW_US,
                ..Stall::default()
            }
            .felt()
        );
        assert!(
            !Stall {
                own_us: OWN_SLOW_US - 1,
                ..Stall::default()
            }
            .felt()
        );
        assert!(
            Stall {
                hook_thread_ms: LATE_INPUT_MS,
                ..Stall::default()
            }
            .felt()
        );
        assert!(
            Stall {
                overslept_ms: LATE_INPUT_MS,
                ..Stall::default()
            }
            .felt()
        );
    }

    #[test]
    fn stall_log_says_it_once_then_a_summary_a_minute_then_that_it_ended() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut log = StallLog::default();
        assert_eq!(log.tick(Stall::default(), at(0)), Emit::Nothing);
        assert_eq!(log.tick(stall(400), at(3)), Emit::Started(stall(400)));
        // more of it: quiet until a minute after the first line
        assert_eq!(log.tick(stall(900), at(6)), Emit::Nothing);
        assert_eq!(log.tick(stall(300), at(9)), Emit::Nothing);
        let mut both = stall(900);
        both.late_events = 2;
        assert_eq!(
            log.tick(Stall::default(), at(12)),
            Emit::Ended {
                since_last: both,
                lasted: Duration::from_secs(9)
            }
        );
        // a long one: a summary each minute
        assert_eq!(log.tick(stall(500), at(100)), Emit::Started(stall(500)));
        for s in (103..160).step_by(3) {
            assert_eq!(log.tick(stall(300), at(s)), Emit::Nothing, "{s}");
        }
        match log.tick(stall(300), at(160)) {
            Emit::Ongoing { since_last, lasted } => {
                assert_eq!(since_last.input_late_ms, 300);
                assert_eq!(since_last.late_events, 20);
                assert_eq!(lasted, Duration::from_secs(60));
            }
            other => panic!("{other:?}"),
        }
    }

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

    fn failed(wanted: bool, hooks: Hooks) -> Seen<'static> {
        Seen {
            install_failed: Some("boom"),
            ..seen(wanted, hooks)
        }
    }

    /// A lost "off" (the post to the hook thread failed) mustn't leave hooks running while the
    /// switch says off: the watchdog re-sends the switch until the hooks match.
    #[test]
    fn hooked_but_switched_off_is_resent_until_the_hooks_go() {
        assert_eq!(watch_gate(&seen(false, Hooks::In)), Gate::Resend);
        assert_eq!(watch_gate(&failed(false, Hooks::In)), Gate::Resend);
        // and handling that resend unhooks
        assert_eq!(hook_step(false, true, false), HookStep::Unhook);
    }

    /// A lost "on" heals the same way; a failed install isn't retried every few seconds (the
    /// user is told, and off/on retries it).
    #[test]
    fn switched_on_without_hooks_is_resent_unless_the_install_failed() {
        assert_eq!(watch_gate(&seen(true, Hooks::None)), Gate::Resend);
        assert_eq!(watch_gate(&failed(true, Hooks::None)), Gate::Idle);
    }

    #[test]
    fn the_watchdog_only_watches_hooks_that_should_be_there() {
        assert_eq!(watch_gate(&seen(true, Hooks::In)), Gate::Watch);
        assert_eq!(watch_gate(&seen(false, Hooks::None)), Gate::Idle);
        assert_eq!(watch_gate(&failed(false, Hooks::None)), Gate::Idle);
    }

    /// Off isn't a problem, but off with the hooks still in is, and stays visible.
    #[test]
    fn hooks_still_in_while_off_is_a_problem() {
        assert_eq!(switch_problem(&seen(false, Hooks::None), false), None);
        assert_eq!(switch_problem(&failed(false, Hooks::None), false), None);
        assert!(
            switch_problem(&seen(false, Hooks::In), false)
                .unwrap()
                .contains("still")
        );
        assert_eq!(
            switch_problem(&failed(true, Hooks::None), false),
            Some("boom".into())
        );
        assert_eq!(switch_problem(&seen(true, Hooks::In), false), None);
    }

    /// A normal "off": the message reached the hook thread, which hasn't unhooked yet. That's
    /// the switch on its way, not a problem; if it really gets stuck, Settings and the watchdog
    /// see it later.
    #[test]
    fn hooks_still_in_right_after_a_delivered_off_is_not_a_problem() {
        assert_eq!(switch_problem(&seen(false, Hooks::In), true), None);
        // an install failure while switching on is still shown
        assert_eq!(
            switch_problem(&failed(true, Hooks::None), true),
            Some("boom".into())
        );
    }

    fn seen(wanted: bool, hooks: Hooks) -> Seen<'static> {
        Seen {
            wanted,
            hooks,
            thread_alive: true,
            install_failed: None,
            reinstall_failures: 0,
        }
    }

    /// Hooks only when the switch is on AND something is bound: nothing to match, nothing to
    /// watch the keyboard for.
    #[test]
    fn hooks_are_wanted_only_when_on_with_something_bound() {
        assert!(hooks_wanted(true, true));
        assert!(!hooks_wanted(true, false), "on, nothing bound");
        assert!(!hooks_wanted(false, true), "off");
        assert!(!hooks_wanted(false, false));
    }

    /// Adding the first bind installs, removing the last unhooks: the same step as the switch.
    #[test]
    fn the_first_bind_installs_and_the_last_one_removed_unhooks() {
        assert_eq!(
            hook_step(hooks_wanted(true, true), false, false),
            HookStep::Install
        );
        assert_eq!(
            hook_step(hooks_wanted(true, false), true, false),
            HookStep::Unhook
        );
        // a watchdog request with nothing bound can't put them back
        assert_eq!(
            hook_step(hooks_wanted(true, false), false, true),
            HookStep::Nothing
        );
    }

    /// The watchdog treats "nothing bound" like "off": no input polled, the hooks resent away.
    #[test]
    fn the_watchdog_treats_nothing_bound_like_off() {
        let none = hooks_wanted(true, false);
        assert_eq!(watch_gate(&seen(none, Hooks::None)), Gate::Idle);
        assert_eq!(watch_gate(&seen(none, Hooks::In)), Gate::Resend);
    }

    /// While the hook thread is between hooks retrying a reinstall, the watchdog neither polls
    /// input nor resends nor forgets its backoff (that would cost an extra reinstall after).
    #[test]
    fn the_watchdog_waits_out_a_reinstall() {
        assert_eq!(watch_gate(&seen(true, Hooks::Retrying)), Gate::Wait);
        assert_eq!(watch_gate(&seen(false, Hooks::Retrying)), Gate::Wait);
    }

    /// A hook thread that never started, or stopped, can't be posted to: nothing to resend
    /// (no warning every tick), and if hooks are wanted it's shown, whatever an earlier
    /// install failure said or didn't.
    #[test]
    fn a_missing_hook_thread_is_shown_and_not_posted_to() {
        let dead = Seen {
            wanted: true,
            hooks: Hooks::None,
            thread_alive: false,
            install_failed: None,
            reinstall_failures: 0,
        };
        assert_eq!(watch_gate(&dead), Gate::Idle);
        assert!(
            switch_problem(&dead, false)
                .unwrap()
                .contains("restart Pulse")
        );
        assert!(switch_problem(&dead, true).is_some(), "not 'on its way'");
        // not wanted: nothing should run anyway
        let off = Seen {
            wanted: false,
            ..dead
        };
        assert_eq!(switch_problem(&off, false), None);
        assert_eq!(watch_gate(&off), Gate::Idle);
    }

    /// Off (or nothing bound) mid-retry: the loop stops by itself within a moment; not a
    /// problem. On mid-retry: the watchdog is fixing it.
    #[test]
    fn retrying_is_not_a_problem() {
        assert_eq!(switch_problem(&seen(false, Hooks::Retrying), false), None);
        assert_eq!(switch_problem(&seen(true, Hooks::Retrying), false), None);
    }

    #[test]
    fn the_hook_state_round_trips_through_its_atomic() {
        for h in [Hooks::None, Hooks::In, Hooks::Retrying] {
            assert_eq!(Hooks::from_u8(h as u8), h);
        }
        assert_eq!(Hooks::from_u8(99), Hooks::None);
    }

    /// A settings change waits (briefly) for the hook thread to settle, so its reply is true.
    #[test]
    fn settled_means_the_hooks_match_or_the_install_failed() {
        assert!(settled(&seen(true, Hooks::In)));
        assert!(settled(&seen(false, Hooks::None)));
        assert!(!settled(&seen(true, Hooks::None)), "install on its way");
        assert!(!settled(&seen(false, Hooks::In)), "unhook on its way");
        assert!(!settled(&seen(true, Hooks::Retrying)));
        assert!(!settled(&seen(false, Hooks::Retrying)));
        let failed = Seen {
            install_failed: Some("boom"),
            ..seen(true, Hooks::None)
        };
        assert!(settled(&failed));
        let dead = Seen {
            thread_alive: false,
            ..seen(true, Hooks::None)
        };
        assert!(settled(&dead), "nothing will change: report now");
    }

    /// The reinstall retry loop sleeps in slices and stops as soon as hooks aren't wanted.
    #[test]
    fn the_retry_wait_stops_as_soon_as_hooks_arent_wanted() {
        let mut checks = 0;
        let mut slept = 0;
        let kept_wanting = retry_wait(
            Duration::from_secs(10),
            Duration::from_millis(100),
            || {
                checks += 1;
                checks < 3
            },
            |d| slept += d.as_millis(),
        );
        assert!(!kept_wanting);
        assert_eq!(slept, 300, "stopped on the third slice, not after 10 s");
        let mut slept = 0;
        assert!(retry_wait(
            Duration::from_secs(10),
            Duration::from_millis(100),
            || true,
            |d| slept += d.as_millis(),
        ));
        assert_eq!(slept, 10_000);
    }

    /// Failing to reach the hook thread is logged once per run of failures, not every tick.
    #[test]
    fn failures_log_once_per_episode() {
        let mut e = Episode::default();
        assert!(e.failed(), "first failure: log");
        assert!(!e.failed());
        assert!(!e.failed());
        assert!(e.succeeded(), "recovered: log that once too");
        assert!(!e.succeeded());
        assert!(e.failed(), "a new episode logs again");
    }

    /// A reinstall that keeps failing is shown once it's clearly not a blip; before that, and
    /// once it works or hooks aren't wanted, it isn't.
    #[test]
    fn a_reinstall_that_keeps_failing_is_shown() {
        let retrying = |wanted, n| Seen {
            reinstall_failures: n,
            ..seen(wanted, Hooks::Retrying)
        };
        for n in 0..SHOW_REINSTALL_FAILURES_AFTER {
            assert_eq!(switch_problem(&retrying(true, n), false), None, "{n}");
        }
        let shown = switch_problem(&retrying(true, SHOW_REINSTALL_FAILURES_AFTER), false).unwrap();
        assert!(shown.contains("couldn't be reinstalled"), "{shown}");
        assert!(
            switch_problem(&retrying(true, 50), true).is_some(),
            "not 'on its way'"
        );
        assert_eq!(
            switch_problem(&retrying(false, 50), false),
            None,
            "stopping"
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
