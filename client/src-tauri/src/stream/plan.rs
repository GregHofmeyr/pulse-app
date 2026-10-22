//! Which processes the stream-audio capture follows. Pure, so the rules are testable anywhere.

/// One WASAPI process-loopback capture: take only this process tree, or everything but it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Capture {
    Include(u32),
    Exclude(u32),
}

pub struct PlanInput<'a> {
    pub own_pid: u32,
    /// Every Pulse process, own included, with WebView2 children.
    pub pulse_pids: &'a [u32],
    /// Processes with an active render session.
    pub session_pids: &'a [u32],
    pub window_pid: Option<u32>,
}

/// The captures to run now.
///
/// Another Pulse instance is any Pulse pid that is not `own_pid`, so the caller passes this
/// instance's own WebView2 children as part of the own tree, not as extra pids.
pub fn capture_plan(i: &PlanInput) -> Vec<Capture> {
    if let Some(p) = i.window_pid.filter(|p| !i.pulse_pids.contains(p)) {
        return vec![Capture::Include(p)];
    }
    if i.pulse_pids.iter().all(|&p| p == i.own_pid) {
        // Alone: exclude our own tree, capture the rest of the machine.
        return vec![Capture::Exclude(i.own_pid)];
    }
    // EXCLUDE takes one process tree, so excluding ours would capture the other instance's
    // playback. Follow each non-Pulse audio process instead.
    let mut plan: Vec<Capture> = i
        .session_pids
        .iter()
        .filter(|p| !i.pulse_pids.contains(p))
        .map(|&p| Capture::Include(p))
        .collect();
    plan.sort_by_key(|c| match *c {
        Capture::Include(p) | Capture::Exclude(p) => p,
    });
    plan.dedup();
    plan
}

/// What to start and what to stop to get from `old` to `new`.
pub fn plan_diff(old: &[Capture], new: &[Capture]) -> (Vec<Capture>, Vec<Capture>) {
    let start = new.iter().filter(|c| !old.contains(c)).copied().collect();
    let stop = old.iter().filter(|c| !new.contains(c)).copied().collect();
    (start, stop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Capture::*;

    fn plan(own: u32, pulse: &[u32], sessions: &[u32], window: Option<u32>) -> Vec<Capture> {
        capture_plan(&PlanInput {
            own_pid: own,
            pulse_pids: pulse,
            session_pids: sessions,
            window_pid: window,
        })
    }

    #[test]
    fn window_share_includes_the_window_process() {
        assert_eq!(plan(10, &[10], &[30], Some(40)), vec![Include(40)]);
    }

    #[test]
    fn window_owned_by_pulse_falls_back_to_screen_rules() {
        assert_eq!(plan(10, &[10], &[30], Some(10)), vec![Exclude(10)]);
    }

    #[test]
    fn alone_excludes_own_tree() {
        assert_eq!(plan(10, &[10], &[10, 30], None), vec![Exclude(10)]);
    }

    #[test]
    fn another_pulse_switches_to_per_session_include() {
        let got = plan(10, &[10, 20, 21], &[21, 30, 31, 30], None);
        assert_eq!(got, vec![Include(30), Include(31)]);
    }

    #[test]
    fn another_pulse_with_no_other_audio_is_an_empty_plan() {
        assert!(plan(10, &[10, 20, 21], &[10, 21], None).is_empty());
    }

    #[test]
    fn diff_starts_new_and_stops_gone() {
        let old = [Include(1), Include(2)];
        let new = [Include(2), Include(3)];
        assert_eq!(plan_diff(&old, &new), (vec![Include(3)], vec![Include(1)]));
    }
}
