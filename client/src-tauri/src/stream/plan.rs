//! Which processes the stream-audio capture follows. Pure, so the rules are testable anywhere:
//! processes, audio sessions and windows come in as plain rows (`procs` gathers the real ones).

use std::collections::HashSet;

/// One WASAPI process-loopback capture: take only this process tree, or everything but it.
/// Logged as `{"include":pid}` / `{"exclude":pid}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Capture {
    Include(u32),
    Exclude(u32),
}

/// The sharer's sound choice. "No sound" never reaches here: then nothing is published.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sound {
    /// The shared window's app (a screen share treats it as `Everything`).
    App,
    /// Everything except Pulse.
    Everything,
}

/// What the capture is doing now; the stream controls say it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Tier {
    /// The window's own app (its process trees).
    App,
    /// The app, plus processes of its family that are playing (same exe, its launcher).
    Related,
    /// Everything except Pulse, as chosen (or a screen share).
    Everything,
    /// This app was chosen but nobody could be found: everything except Pulse and the
    /// always-on apps in `SKIP_ALWAYS_ON`.
    LastResort,
}

/// One process: `exe` is its image path when it could be read, else its exe name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub parent: u32,
    pub exe: String,
}

/// What is being shared. A window share carries the processes owning the window (more than one
/// only when they are the same app); empty when nobody could be identified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Screen,
    Window(Vec<u32>),
}

/// Always-on apps the last-resort tier leaves out, so the stream is mostly the shared app:
/// music players, and Discord (often running beside Pulse). Exe names, any case.
pub const SKIP_ALWAYS_ON: &[&str] = &[
    "spotify.exe",
    "applemusic.exe",
    "itunes.exe",
    "foobar2000.exe",
    "musicbee.exe",
    "aimp.exe",
    "winamp.exe",
    "tidal.exe",
    "deezer.exe",
    "amazon music.exe",
    "discord.exe",
    "discordptb.exe",
    "discordcanary.exe",
];

/// Shells and system hosts: never an app's "launcher". Capturing one (tree mode) would take
/// every app it started.
const NOT_A_LAUNCHER: &[&str] = &[
    "explorer.exe",
    "svchost.exe",
    "services.exe",
    "wininit.exe",
    "winlogon.exe",
    "smss.exe",
    "csrss.exe",
    "sihost.exe",
    "runtimebroker.exe",
    "applicationframehost.exe",
    "dllhost.exe",
    "taskhostw.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "conhost.exe",
    "openconsole.exe",
    "windowsterminal.exe",
];

/// UWP windows belong to this host; the app is the process owning the frame's child window.
const FRAME_HOST: &str = "applicationframehost.exe";

pub struct PlanInput<'a> {
    pub own_pid: u32,
    /// Every Pulse process, own included, with WebView2 children.
    pub pulse_pids: &'a [u32],
    /// The main Pulse processes (exe name equals ours), own included.
    pub pulse_roots: &'a [u32],
    /// Processes with an active render session.
    pub session_pids: &'a [u32],
    /// Every process (for families, launchers and the skip list).
    pub procs: &'a [Proc],
    pub source: Source,
    pub sound: Sound,
}

/// `exe`'s file name: the part after the last `\` or `/`.
pub fn exe_name(exe: &str) -> &str {
    exe.rsplit(['\\', '/']).next().unwrap_or(exe)
}

fn named_in(exe: &str, list: &[&str]) -> bool {
    let n = exe_name(exe);
    list.iter().any(|l| n.eq_ignore_ascii_case(l))
}

/// An app's name for the stream controls, from its exe: `C:\…\firefox.exe` → `Firefox`.
pub fn app_name(exe: &str) -> String {
    let n = exe_name(exe);
    let stem = match n.rfind('.') {
        Some(i) if i > 0 => &n[..i],
        _ => n,
    };
    match stem.to_ascii_lowercase().as_str() {
        "msedge" => return "Edge".into(),
        "vlc" => return "VLC".into(),
        _ => {}
    }
    let mut c = stem.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// For a UWP window (owned by ApplicationFrameHost): the app, the first other process owning
/// one of its child windows. `None` for any other owner.
pub fn frame_host_app(owner_pid: u32, owner_exe: &str, child_pids: &[u32]) -> Option<u32> {
    if !named_in(owner_exe, &[FRAME_HOST]) {
        return None;
    }
    child_pids
        .iter()
        .copied()
        .find(|&p| p != owner_pid && p != 0)
}

/// From (title, pid, visible) rows for top-level windows: the processes owning a visible window
/// titled exactly `title`, outside `pulse`. Several only when they are one app (the same exe):
/// two different apps with the same title are ambiguous, and guessing could capture the wrong one.
pub fn title_owner(
    rows: &[(String, u32, bool)],
    title: &str,
    pulse: &[u32],
    procs: &[Proc],
) -> Vec<u32> {
    let mut pids: Vec<u32> = rows
        .iter()
        .filter(|r| r.2 && r.0 == title && !pulse.contains(&r.1))
        .map(|r| r.1)
        .collect();
    pids.sort_unstable();
    pids.dedup();
    if pids.len() <= 1 {
        return pids;
    }
    let exe = |pid: u32| procs.iter().find(|p| p.pid == pid).map(|p| p.exe.as_str());
    let first = exe(pids[0]);
    let one_app = first.is_some_and(|f| {
        pids.iter()
            .all(|&p| exe(p).is_some_and(|e| e.eq_ignore_ascii_case(f)))
    });
    if one_app { pids } else { Vec::new() }
}

/// `roots` and all their descendants. Pids are reused, so a stale parent id can loop back.
fn trees(roots: &[u32], procs: &[Proc]) -> HashSet<u32> {
    let mut all: Vec<u32> = roots.to_vec();
    let mut seen: HashSet<u32> = all.iter().copied().collect();
    let mut i = 0;
    while i < all.len() {
        let parent = all[i];
        for p in procs.iter().filter(|p| p.parent == parent && p.pid != 0) {
            if seen.insert(p.pid) {
                all.push(p.pid);
            }
        }
        i += 1;
    }
    seen
}

/// `pid` or one of its ancestors is an always-on app (Spotify's helpers play its sound).
fn always_on(pid: u32, procs: &[Proc]) -> bool {
    let mut at = pid;
    // bounded: a parent cycle can't spin
    for _ in 0..=procs.len() {
        let Some(p) = procs.iter().find(|p| p.pid == at) else {
            return false;
        };
        if named_in(&p.exe, SKIP_ALWAYS_ON) {
            return true;
        }
        if p.parent == 0 || p.parent == p.pid {
            return false;
        }
        at = p.parent;
    }
    false
}

fn sorted(mut plan: Vec<Capture>) -> Vec<Capture> {
    plan.sort_by_key(|c| match *c {
        Capture::Include(p) | Capture::Exclude(p) => p,
    });
    plan.dedup();
    plan
}

/// Each non-Pulse audio process, minus the always-on apps when `skip`. A capture takes the
/// whole tree, so a process whose tree holds Pulse (or, when `skip`, an always-on app) is left
/// out too: it would bring them back in.
fn per_session(i: &PlanInput, skip: bool) -> Vec<Capture> {
    sorted(
        i.session_pids
            .iter()
            .filter(|p| !i.pulse_pids.contains(p))
            .filter(|&&p| !(skip && always_on(p, i.procs)))
            .filter(|&&p| {
                let tree = trees(&[p], i.procs);
                !tree.iter().any(|t| {
                    i.pulse_pids.contains(t) || (skip && is_named(*t, i.procs, SKIP_ALWAYS_ON))
                })
            })
            .map(|&p| Capture::Include(p))
            .collect(),
    )
}

/// `pid`'s exe is one of `list`.
fn is_named(pid: u32, procs: &[Proc], list: &[&str]) -> bool {
    procs.iter().any(|p| p.pid == pid && named_in(&p.exe, list))
}

/// UWP windows' host: an owner that is it was not resolved to the app behind the frame.
pub fn is_frame_host(exe: &str) -> bool {
    named_in(exe, &[FRAME_HOST])
}

fn everything(i: &PlanInput) -> Vec<Capture> {
    if i.pulse_roots.iter().all(|&p| p == i.own_pid) {
        // Alone: exclude our own tree, capture the rest of the machine.
        return vec![Capture::Exclude(i.own_pid)];
    }
    // EXCLUDE takes one process tree, so excluding ours would capture the other instance's
    // playback. Follow each non-Pulse audio process instead.
    per_session(i, false)
}

/// The window's owners, Pulse left out.
fn owners(i: &PlanInput) -> Vec<u32> {
    match &i.source {
        Source::Window(o) => o
            .iter()
            .copied()
            .filter(|p| !i.pulse_pids.contains(p))
            .collect(),
        Source::Screen => Vec::new(),
    }
}

/// An owner's parent counts as its launcher only when capturing it (a capture takes the whole
/// tree) brings in nothing else: not a shell or system host, not an always-on app, and nothing
/// in its tree outside the owners' trees but itself and processes of its own or the owners' exe
/// (no sibling app, no Pulse).
fn launcher_ok(parent: &Proc, i: &PlanInput, own: &HashSet<u32>, owner_procs: &[&Proc]) -> bool {
    if named_in(&parent.exe, NOT_A_LAUNCHER)
        || named_in(&parent.exe, SKIP_ALWAYS_ON)
        || i.pulse_pids.contains(&parent.pid)
    {
        return false;
    }
    let ours = |exe: &str| {
        exe.eq_ignore_ascii_case(&parent.exe)
            || owner_procs.iter().any(|o| o.exe.eq_ignore_ascii_case(exe))
    };
    trees(&[parent.pid], i.procs)
        .iter()
        .filter(|t| **t != parent.pid && !own.contains(t))
        .all(|t| !i.pulse_pids.contains(t) && i.procs.iter().any(|p| p.pid == *t && ours(&p.exe)))
}

/// Audio processes of the owners' family, when the owners' own trees are silent: processes of
/// the same exe and their descendants (a browser's other processes), and an owner's parent (its
/// launcher) when `launcher_ok`.
fn related(i: &PlanInput, owners: &[u32]) -> Vec<u32> {
    let own = trees(owners, i.procs);
    if i.session_pids.iter().any(|p| own.contains(p)) {
        return Vec::new();
    }
    let owner_procs: Vec<&Proc> = i.procs.iter().filter(|p| owners.contains(&p.pid)).collect();
    let same: Vec<u32> = i
        .procs
        .iter()
        .filter(|p| {
            owner_procs
                .iter()
                .any(|o| !o.exe.is_empty() && o.exe.eq_ignore_ascii_case(&p.exe))
        })
        .map(|p| p.pid)
        .collect();
    let mut family = trees(&same, i.procs);
    for o in &owner_procs {
        if let Some(parent) = i.procs.iter().find(|p| p.pid == o.parent && p.pid != 0)
            && launcher_ok(parent, i, &own, &owner_procs)
        {
            family.insert(parent.pid);
        }
    }
    let mut out: Vec<u32> = i
        .session_pids
        .iter()
        .copied()
        .filter(|p| family.contains(p) && !own.contains(p) && !i.pulse_pids.contains(p))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The captures to run now, and which tier they are.
///
/// Another Pulse instance is a root in `pulse_roots` other than `own_pid`.
pub fn capture_plan(i: &PlanInput) -> (Vec<Capture>, Tier) {
    if i.sound == Sound::Everything || i.source == Source::Screen {
        return (everything(i), Tier::Everything);
    }
    let owners = owners(i);
    if owners.is_empty() {
        return (per_session(i, true), Tier::LastResort);
    }
    let rel = related(i, &owners);
    let tier = if rel.is_empty() {
        Tier::App
    } else {
        Tier::Related
    };
    let plan = owners
        .iter()
        .chain(rel.iter())
        .map(|&p| Capture::Include(p))
        .collect();
    (sorted(plan), tier)
}

/// Whether the window's app (its trees or its family) has an active audio session: the
/// picker's "is it playing" hint. False when it can't be identified.
pub fn playing(i: &PlanInput) -> bool {
    let owners = owners(i);
    if owners.is_empty() {
        return false;
    }
    let own = trees(&owners, i.procs);
    i.session_pids.iter().any(|p| own.contains(p)) || !related(i, &owners).is_empty()
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

    fn p(pid: u32, parent: u32, exe: &str) -> Proc {
        Proc {
            pid,
            parent,
            exe: exe.to_string(),
        }
    }

    /// A machine: explorer (2) started a browser (40, with audio helper 41), a launcher (50)
    /// that started one game (51), a store (55) that started two games (56, 57), Spotify (60,
    /// helper 61), a second browser window process of the same exe (45, a sibling under
    /// explorer, with its own helper 46), a video player (70), and Pulse (10, WebView2 11).
    fn machine() -> Vec<Proc> {
        vec![
            p(2, 1, r"C:\Windows\explorer.exe"),
            p(10, 2, r"C:\Pulse\pulse-app.exe"),
            p(11, 10, "msedgewebview2.exe"),
            p(40, 2, r"C:\Firefox\firefox.exe"),
            p(41, 40, r"C:\Firefox\firefox.exe"),
            p(45, 2, r"C:\Firefox\firefox.exe"),
            p(46, 45, r"C:\Firefox\firefox.exe"),
            p(50, 2, r"C:\Launcher\launcher.exe"),
            p(51, 50, r"C:\Games\game.exe"),
            p(55, 2, r"C:\Store\store.exe"),
            p(56, 55, r"C:\Games\shared.exe"),
            p(57, 55, r"C:\Games\other.exe"),
            p(60, 2, r"C:\Spotify\Spotify.exe"),
            p(61, 60, r"C:\Spotify\Spotify.exe"),
            p(70, 2, r"C:\Video\player.exe"),
        ]
    }

    struct Case {
        pulse: Vec<u32>,
        roots: Vec<u32>,
        sessions: Vec<u32>,
        procs: Vec<Proc>,
    }

    impl Case {
        fn new(sessions: &[u32]) -> Self {
            Case {
                pulse: vec![10, 11],
                roots: vec![10],
                sessions: sessions.to_vec(),
                procs: machine(),
            }
        }
        fn input(&self, source: Source, sound: Sound) -> PlanInput<'_> {
            PlanInput {
                own_pid: 10,
                pulse_pids: &self.pulse,
                pulse_roots: &self.roots,
                session_pids: &self.sessions,
                procs: &self.procs,
                source,
                sound,
            }
        }
        fn plan(&self, source: Source, sound: Sound) -> (Vec<Capture>, Tier) {
            capture_plan(&self.input(source, sound))
        }
    }

    fn window(pids: &[u32]) -> Source {
        Source::Window(pids.to_vec())
    }

    #[test]
    fn app_tier_includes_the_owner() {
        let c = Case::new(&[41, 60]);
        assert_eq!(
            c.plan(window(&[40]), Sound::App),
            (vec![Include(40)], Tier::App)
        );
    }

    #[test]
    fn a_silent_app_with_a_silent_family_is_still_the_app() {
        // silent now is fine: a paused video starts later, and the rescan keeps watching
        let c = Case::new(&[60, 70]);
        assert_eq!(
            c.plan(window(&[51]), Sound::App),
            (vec![Include(51)], Tier::App)
        );
    }

    #[test]
    fn related_adds_same_exe_session() {
        // the window is 45's, but the sound plays from the other browser process' helper
        let c = Case::new(&[41, 60]);
        assert_eq!(
            c.plan(window(&[45]), Sound::App),
            (vec![Include(41), Include(45)], Tier::Related)
        );
    }

    #[test]
    fn related_adds_launcher_parent() {
        let c = Case::new(&[50, 60]);
        assert_eq!(
            c.plan(window(&[51]), Sound::App),
            (vec![Include(50), Include(51)], Tier::Related)
        );
    }

    #[test]
    fn related_never_takes_a_sibling_by_itself() {
        // the store's other game is not the shared game's family
        let c = Case::new(&[57]);
        assert_eq!(
            c.plan(window(&[56]), Sound::App),
            (vec![Include(56)], Tier::App)
        );
    }

    #[test]
    fn a_launcher_with_another_app_under_it_is_not_family() {
        // capturing the store's tree would take the other game too
        let c = Case::new(&[55]);
        assert_eq!(
            c.plan(window(&[56]), Sound::App),
            (vec![Include(56)], Tier::App)
        );
    }

    #[test]
    fn a_launcher_that_started_pulse_is_not_family() {
        // its tree would carry the call's own voices to the viewers
        let mut c = Case::new(&[50]);
        c.procs.iter_mut().find(|p| p.pid == 10).unwrap().parent = 50;
        assert_eq!(
            c.plan(window(&[51]), Sound::App),
            (vec![Include(51)], Tier::App)
        );
    }

    #[test]
    fn an_always_on_app_is_never_a_launcher() {
        let mut c = Case::new(&[90]);
        c.procs.push(p(90, 2, r"C:\Discord\Discord.exe"));
        c.procs.push(p(91, 90, r"C:\Games\activity.exe"));
        assert_eq!(
            c.plan(window(&[91]), Sound::App),
            (vec![Include(91)], Tier::App)
        );
    }

    #[test]
    fn last_resort_leaves_out_a_session_whose_tree_holds_pulse_or_music() {
        // explorer's tree is Pulse's and Spotify's too; the store's is not
        let mut c = Case::new(&[2, 55, 70]);
        c.procs.push(p(62, 55, r"C:\Spotify\Spotify.exe"));
        assert_eq!(
            c.plan(window(&[]), Sound::App),
            (vec![Include(70)], Tier::LastResort)
        );
    }

    #[test]
    fn related_never_takes_the_shell_as_a_launcher() {
        // explorer started nearly everything: as a tree it would be the whole machine
        let c = Case::new(&[2]);
        assert_eq!(
            c.plan(window(&[70]), Sound::App),
            (vec![Include(70)], Tier::App)
        );
    }

    #[test]
    fn once_the_app_plays_the_family_is_dropped() {
        let c = Case::new(&[46, 41]);
        assert_eq!(
            c.plan(window(&[45]), Sound::App),
            (vec![Include(45)], Tier::App)
        );
    }

    #[test]
    fn last_resort_skips_pulse_and_always_on_apps() {
        let c = Case::new(&[11, 41, 51, 60]);
        assert_eq!(
            c.plan(window(&[]), Sound::App),
            (vec![Include(41), Include(51)], Tier::LastResort)
        );
    }

    #[test]
    fn last_resort_skips_music_app_helpers() {
        let c = Case::new(&[61, 70]);
        assert_eq!(
            c.plan(window(&[]), Sound::App),
            (vec![Include(70)], Tier::LastResort)
        );
    }

    #[test]
    fn a_window_owned_by_pulse_is_not_identified() {
        let c = Case::new(&[41]);
        assert_eq!(c.plan(window(&[10]), Sound::App).1, Tier::LastResort);
    }

    #[test]
    fn everything_ignores_the_owner() {
        let c = Case::new(&[41]);
        assert_eq!(
            c.plan(window(&[40]), Sound::Everything),
            (vec![Exclude(10)], Tier::Everything)
        );
    }

    #[test]
    fn a_screen_share_is_everything_whatever_the_choice() {
        let c = Case::new(&[41]);
        assert_eq!(
            c.plan(Source::Screen, Sound::App),
            (vec![Exclude(10)], Tier::Everything)
        );
    }

    #[test]
    fn another_pulse_switches_everything_to_per_session_include() {
        let mut c = Case::new(&[21, 30, 31, 30]);
        c.pulse = vec![10, 20, 21];
        c.roots = vec![10, 20];
        assert_eq!(
            c.plan(Source::Screen, Sound::Everything),
            (vec![Include(30), Include(31)], Tier::Everything)
        );
    }

    #[test]
    fn another_pulse_with_no_other_audio_is_an_empty_plan() {
        let mut c = Case::new(&[10, 21]);
        c.pulse = vec![10, 20, 21];
        c.roots = vec![10, 20];
        assert!(c.plan(Source::Screen, Sound::Everything).0.is_empty());
    }

    #[test]
    fn everything_keeps_music_apps() {
        // the skip list is the last resort's only
        let mut c = Case::new(&[60]);
        c.roots = vec![10, 20];
        assert_eq!(
            c.plan(Source::Screen, Sound::Everything).0,
            vec![Include(60)]
        );
    }

    #[test]
    fn playing_follows_the_app_and_its_family() {
        let c = Case::new(&[41]);
        assert!(playing(&c.input(window(&[40]), Sound::App)));
        assert!(playing(&c.input(window(&[45]), Sound::App)), "family");
        assert!(!playing(&c.input(window(&[51]), Sound::App)));
        assert!(!playing(&c.input(window(&[]), Sound::App)));
    }

    #[test]
    fn skip_list_is_named_and_case_insensitive() {
        for exe in [
            r"C:\x\Spotify.exe",
            "DISCORD.EXE",
            "DiscordCanary.exe",
            "foobar2000.exe",
        ] {
            assert!(named_in(exe, SKIP_ALWAYS_ON), "{exe}");
        }
        assert!(!named_in("firefox.exe", SKIP_ALWAYS_ON));
        assert!(!named_in("pulse-app.exe", SKIP_ALWAYS_ON));
    }

    #[test]
    fn app_name_from_path() {
        assert_eq!(
            app_name(r"C:\Program Files\Mozilla Firefox\firefox.exe"),
            "Firefox"
        );
        assert_eq!(app_name(r"C:\x\msedge.exe"), "Edge");
        assert_eq!(app_name("Game.EXE"), "Game");
        assert_eq!(app_name("/usr/bin/mpv"), "Mpv");
        assert_eq!(app_name(""), "");
    }

    #[test]
    fn frame_host_window_resolves_to_its_app() {
        assert_eq!(
            frame_host_app(
                5,
                r"C:\Windows\System32\ApplicationFrameHost.exe",
                &[5, 0, 77]
            ),
            Some(77)
        );
        assert_eq!(frame_host_app(5, "game.exe", &[77]), None);
        assert_eq!(frame_host_app(5, "ApplicationFrameHost.exe", &[5]), None);
    }

    fn row(title: &str, pid: u32, visible: bool) -> (String, u32, bool) {
        (title.to_string(), pid, visible)
    }

    #[test]
    fn title_owner_finds_the_one_visible_match() {
        let procs = machine();
        let rows = [
            row("Other", 70, true),
            row("Game", 51, true),
            row("Game", 51, true),
            row("Game - Settings", 57, true),
            row("Game", 60, false), // hidden windows don't count
        ];
        assert_eq!(title_owner(&rows, "Game", &[10], &procs), vec![51]);
        assert!(title_owner(&rows, "Nope", &[10], &procs).is_empty());
    }

    #[test]
    fn title_owner_is_none_for_two_different_apps() {
        let rows = [row("Game", 51, true), row("Game", 57, true)];
        assert!(title_owner(&rows, "Game", &[10], &machine()).is_empty());
    }

    #[test]
    fn title_owner_takes_one_app_with_two_processes() {
        let rows = [row("Video", 45, true), row("Video", 40, true)];
        assert_eq!(title_owner(&rows, "Video", &[10], &machine()), vec![40, 45]);
    }

    #[test]
    fn title_owner_ignores_pulse_windows() {
        let rows = [
            row("Game", 10, true),
            row("Game", 11, true),
            row("Game", 51, true),
        ];
        assert_eq!(title_owner(&rows, "Game", &[10, 11], &machine()), vec![51]);
        assert!(title_owner(&rows[..2], "Game", &[10, 11], &machine()).is_empty());
    }

    #[test]
    fn trees_survive_a_parent_cycle() {
        let procs = [p(10, 11, "a.exe"), p(11, 10, "b.exe")];
        let t = trees(&[10], &procs);
        assert_eq!(t.len(), 2);
        assert!(!always_on(10, &procs));
    }

    #[test]
    fn diff_starts_new_and_stops_gone() {
        let old = [Include(1), Include(2)];
        let new = [Include(2), Include(3)];
        assert_eq!(plan_diff(&old, &new), (vec![Include(3)], vec![Include(1)]));
    }
}
