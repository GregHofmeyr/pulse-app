//! Which processes the stream-audio capture follows. Pure, so the rules are testable anywhere.

/// One WASAPI process-loopback capture: take only this process tree, or everything but it.
/// Logged as `{"include":pid}` / `{"exclude":pid}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Capture {
    Include(u32),
    Exclude(u32),
}

/// What is being shared. A window share carries the window's process, if one was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Screen,
    Window(Option<u32>),
}

pub struct PlanInput<'a> {
    pub own_pid: u32,
    /// Every Pulse process, own included, with WebView2 children.
    pub pulse_pids: &'a [u32],
    /// The main Pulse processes (exe name equals ours), own included.
    pub pulse_roots: &'a [u32],
    /// Processes with an active render session.
    pub session_pids: &'a [u32],
    pub source: Source,
}

/// The captures to run now.
///
/// Another Pulse instance is a root in `pulse_roots` other than `own_pid`.
pub fn capture_plan(i: &PlanInput) -> Vec<Capture> {
    if let Source::Window(pid) = i.source {
        // Only the window's own process, or nothing: never the rest of the machine.
        return match pid.filter(|p| !i.pulse_pids.contains(p)) {
            Some(p) => vec![Capture::Include(p)],
            None => Vec::new(),
        };
    }
    if i.pulse_roots.iter().all(|&p| p == i.own_pid) {
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

    fn plan(
        own: u32,
        pulse: &[u32],
        roots: &[u32],
        sessions: &[u32],
        source: Source,
    ) -> Vec<Capture> {
        capture_plan(&PlanInput {
            own_pid: own,
            pulse_pids: pulse,
            pulse_roots: roots,
            session_pids: sessions,
            source,
        })
    }

    const SCREEN: Source = Source::Screen;

    #[test]
    fn window_share_includes_the_window_process() {
        assert_eq!(
            plan(10, &[10], &[10], &[30], Source::Window(Some(40))),
            vec![Include(40)]
        );
    }

    #[test]
    fn window_share_with_no_owner_captures_nothing() {
        // Not everything-but-Pulse: a window share promises that app's audio only.
        assert!(plan(10, &[10], &[10], &[30], Source::Window(None)).is_empty());
        assert!(plan(10, &[10, 20], &[10, 20], &[30], Source::Window(None)).is_empty());
    }

    #[test]
    fn window_owned_by_pulse_captures_nothing() {
        assert!(plan(10, &[10], &[10], &[30], Source::Window(Some(10))).is_empty());
    }

    #[test]
    fn alone_excludes_own_tree() {
        assert_eq!(
            plan(10, &[10, 11], &[10], &[10, 11, 30], SCREEN),
            vec![Exclude(10)]
        );
    }

    #[test]
    fn another_pulse_switches_to_per_session_include() {
        let got = plan(10, &[10, 20, 21], &[10, 20], &[21, 30, 31, 30], SCREEN);
        assert_eq!(got, vec![Include(30), Include(31)]);
    }

    #[test]
    fn another_pulse_with_no_other_audio_is_an_empty_plan() {
        assert!(plan(10, &[10, 20, 21], &[10, 20], &[10, 21], SCREEN).is_empty());
    }

    #[test]
    fn own_child_session_is_not_included_next_to_another_instance() {
        let got = plan(10, &[10, 11, 20], &[10, 20], &[11, 30], SCREEN);
        assert_eq!(got, vec![Include(30)]);
    }

    #[test]
    fn diff_starts_new_and_stops_gone() {
        let old = [Include(1), Include(2)];
        let new = [Include(2), Include(3)];
        assert_eq!(plan_diff(&old, &new), (vec![Include(3)], vec![Include(1)]));
    }
}
