//! After the installer takes over (#460). Pulse ends the moment it hands over to the installer, so a failed install
//! would be invisible: the next launch would quietly find the same update and download ~80 MB again, every launch.
//!
//! - Just before the handover, `update-attempt.json` in the profile dir says "installing X (from Y) at T".
//! - The next launch reads it: on X (or newer), the update worked and the file goes. Still older: the install failed.
//!   That's logged, reported (as a crash would be: only with automatic reports on), shown as "Couldn't update" with a
//!   download link, and Pulse doesn't install anything by itself for [`INSTALL_BACKOFF`]; "Restart to update" still
//!   offers it.
//! - Leftover installers from the plugin's temp dirs are deleted at startup ([`stale_installers`]).

use std::path::{Path, PathBuf};
use std::time::Duration;

use semver::Version;
use serde::{Deserialize, Serialize};

const FILE: &str = "update-attempt.json";
/// After a failed install, how long Pulse waits before installing anything by itself again (at launch, or when idle).
pub const INSTALL_BACKOFF: Duration = Duration::from_secs(24 * 3600);

/// "Installing `to` (from `from`) at `at`", and once a launch found it failed, when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub from: String,
    pub to: String,
    pub at: u64,
    #[serde(default)]
    pub failed_at: Option<u64>,
}

/// A failed install, as this launch knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub attempt: Attempt,
    /// This launch is the first to find it (report it, once).
    pub fresh: bool,
    /// No installing by itself until then (unix seconds).
    pub backoff_until: u64,
}

/// Before the handover: what's being installed. Best effort (a write that fails only loses the detection).
pub fn record(dir: &Path, from: &str, to: &str, now: u64) {
    let a = Attempt {
        from: from.into(),
        to: to.into(),
        at: now,
        failed_at: None,
    };
    if let Err(e) = write(dir, &a) {
        tracing::warn!(error = %e, "update: couldn't note the install attempt");
    }
}

/// The installer didn't start after all (Pulse carries on and says why): nothing to detect next launch.
pub fn clear(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(FILE));
}

fn write(dir: &Path, a: &Attempt) -> std::io::Result<()> {
    let body = serde_json::to_vec(a).map_err(std::io::Error::other)?;
    crate::reports::write_atomic(dir, FILE, &body)
}

fn parse(v: &str) -> Option<Version> {
    Version::parse(v).ok()
}

/// At launch, on version `current`: what the last install attempt came to. `None`: there wasn't one, it worked, or its
/// back-off is over (the file goes in all three).
pub fn check(dir: &Path, current: &Version, now: u64) -> Option<Failure> {
    let raw = std::fs::read(dir.join(FILE)).ok()?;
    let Ok(mut a) = serde_json::from_slice::<Attempt>(&raw) else {
        clear(dir);
        return None;
    };
    let Some(to) = parse(&a.to) else {
        clear(dir);
        return None;
    };
    if *current >= to {
        tracing::info!(from = %a.from, to = %a.to, "update: installed");
        clear(dir);
        return None;
    }
    let fresh = a.failed_at.is_none();
    let failed_at = *a.failed_at.get_or_insert(now);
    let backoff_until = failed_at.saturating_add(INSTALL_BACKOFF.as_secs());
    // long enough: the next found update installs as usual again. A failure dated further ahead than the back-off
    // itself (the clock was ahead when it was noted, then put right) is over too: it would otherwise stretch the
    // back-off by however far the clock was off.
    if now >= backoff_until || failed_at > now.saturating_add(INSTALL_BACKOFF.as_secs()) {
        clear(dir);
        return None;
    }
    if fresh {
        tracing::warn!(from = %a.from, to = %a.to, current = %current, "update: the last install didn't finish; still on this version");
        if let Err(e) = write(dir, &a) {
            tracing::warn!(error = %e, "update: couldn't note the failed install");
        }
    }
    Some(Failure {
        attempt: a,
        fresh,
        backoff_until,
    })
}

/// After an install that failed while running, the first wait before installing that version by itself again; each
/// further failure of the same version doubles it, up to [`INSTALL_BACKOFF`].
pub const RETRY_FIRST: Duration = Duration::from_secs(3600);

/// How long to wait after the `count`-th failed install of one version in a row (1 h, 2 h, 4 h, … capped at
/// [`INSTALL_BACKOFF`]).
pub fn retry_after(count: u32) -> Duration {
    let doublings = count.saturating_sub(1).min(16);
    RETRY_FIRST
        .saturating_mul(1 << doublings)
        .min(INSTALL_BACKOFF)
}

/// An install that failed while running (in memory, this run only): the idle install leaves that version alone for a
/// while ([`retry_after`]), and for good after a bad signature (until another version is offered). "Restart to update"
/// still tries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFailure {
    pub version: String,
    /// Failed installs of this version in a row.
    pub count: u32,
    /// No installing it by itself until then (unix seconds); `None`: not this version at all (a bad signature).
    pub until: Option<u64>,
}

impl RunFailure {
    /// After `prev`, installing `version` failed at `now` (`signature`: its signature didn't check out).
    pub fn after(
        prev: Option<&RunFailure>,
        version: &str,
        signature: bool,
        now: u64,
    ) -> RunFailure {
        let count = match prev {
            Some(p) if p.version == version => p.count.saturating_add(1),
            _ => 1,
        };
        RunFailure {
            version: version.into(),
            count,
            until: (!signature).then(|| now.saturating_add(retry_after(count).as_secs())),
        }
    }

    /// Whether the idle install leaves `version` alone at `now`. Another version is never held back; a wait further
    /// ahead than [`INSTALL_BACKOFF`] (the clock went back) is over.
    pub fn blocks(&self, version: &str, now: u64) -> bool {
        self.version == version
            && self
                .until
                .is_none_or(|u| now < u && u <= now.saturating_add(INSTALL_BACKOFF.as_secs()))
    }
}

/// The plugin's leftovers in `temp`: `{app}-{X.Y.Z}-updater-*` dirs holding nothing but `{app}-{X.Y.Z}-installer.exe`
/// (or `.msi`). The files to delete, then the dirs (deleted only once empty). Anything else is left alone.
pub fn stale_installers(temp: &Path, app: &str) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let (mut files, mut dirs) = (vec![], vec![]);
    let Ok(entries) = std::fs::read_dir(temp) else {
        return (files, dirs);
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(version) = name
            .strip_prefix(&format!("{app}-"))
            .and_then(|r| r.split_once("-updater-"))
            .map(|(v, _)| v.to_string())
        else {
            continue;
        };
        if parse(&version).is_none() || !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(inner) = std::fs::read_dir(e.path()) else {
            continue;
        };
        let installer = |n: &str| {
            n == format!("{app}-{version}-installer.exe")
                || n == format!("{app}-{version}-installer.msi")
        };
        let mut mine = vec![];
        let mut other = false;
        for f in inner.flatten() {
            let n = f.file_name().to_string_lossy().into_owned();
            if installer(&n) && f.file_type().is_ok_and(|t| t.is_file()) {
                mine.push(f.path());
            } else {
                other = true;
            }
        }
        if !other {
            files.extend(mine);
            dirs.push(e.path());
        }
    }
    (files, dirs)
}

/// Delete [`stale_installers`] (best effort: one still in use stays for next time).
pub fn delete_stale_installers(temp: &Path, app: &str) {
    let (files, dirs) = stale_installers(temp, app);
    let mut gone = 0;
    for f in &files {
        if std::fs::remove_file(f).is_ok() {
            gone += 1;
        }
    }
    for d in &dirs {
        let _ = std::fs::remove_dir(d);
    }
    if gone > 0 {
        tracing::info!(count = gone, "update: deleted leftover installers");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn no_attempt_is_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(check(dir.path(), &v("0.2.18"), 100), None);
    }

    #[test]
    fn reaching_the_version_is_success_and_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        record(dir.path(), "0.2.17", "0.2.18", 100);
        assert_eq!(check(dir.path(), &v("0.2.18"), 200), None);
        assert!(!dir.path().join(FILE).exists());
        // a newer one (installed by hand) counts too
        record(dir.path(), "0.2.17", "0.2.18", 100);
        assert_eq!(check(dir.path(), &v("0.2.19"), 200), None);
    }

    #[test]
    fn still_old_is_a_failure_reported_once_then_backed_off() {
        let dir = tempfile::tempdir().unwrap();
        record(dir.path(), "0.2.17", "0.2.18", 100);
        let f = check(dir.path(), &v("0.2.17"), 200).expect("failed");
        assert!(f.fresh);
        assert_eq!(f.attempt.to, "0.2.18");
        assert_eq!(f.backoff_until, 200 + INSTALL_BACKOFF.as_secs());
        // the next launch still backs off, but it isn't news
        let f2 = check(dir.path(), &v("0.2.17"), 300).expect("still backing off");
        assert!(!f2.fresh);
        assert_eq!(f2.backoff_until, f.backoff_until);
        // and after the back-off it's forgotten
        assert_eq!(check(dir.path(), &v("0.2.17"), f.backoff_until), None);
        assert!(!dir.path().join(FILE).exists());
    }

    #[test]
    fn a_failure_dated_in_the_future_doesnt_stretch_the_back_off() {
        let dir = tempfile::tempdir().unwrap();
        let day = INSTALL_BACKOFF.as_secs();
        let now = 10 * day;
        // noted while the clock was a week ahead
        let a = Attempt {
            from: "0.2.17".into(),
            to: "0.2.18".into(),
            at: now + 7 * day,
            failed_at: Some(now + 7 * day),
        };
        write(dir.path(), &a).unwrap();
        assert_eq!(check(dir.path(), &v("0.2.17"), now), None);
        assert!(!dir.path().join(FILE).exists());
        // a little ahead (within the back-off) still backs off, until its own end
        let a = Attempt {
            failed_at: Some(now + 60),
            ..a
        };
        write(dir.path(), &a).unwrap();
        let f = check(dir.path(), &v("0.2.17"), now).expect("backing off");
        assert_eq!(f.backoff_until, now + 60 + day);
    }

    #[test]
    fn run_failures_back_off_doubling_up_to_a_day() {
        let hours: Vec<u64> = (1..=7).map(|n| retry_after(n).as_secs() / 3600).collect();
        assert_eq!(hours, [1, 2, 4, 8, 16, 24, 24]);
        assert_eq!(retry_after(0), RETRY_FIRST);
        assert_eq!(retry_after(u32::MAX), INSTALL_BACKOFF);

        let f1 = RunFailure::after(None, "0.2.19", false, 1000);
        assert_eq!((f1.count, f1.until), (1, Some(1000 + 3600)));
        assert!(f1.blocks("0.2.19", 1000));
        assert!(f1.blocks("0.2.19", 1000 + 3599));
        assert!(!f1.blocks("0.2.19", 1000 + 3600), "the wait is over");
        assert!(
            !f1.blocks("0.2.20", 1000),
            "a newer version isn't held back"
        );

        let f2 = RunFailure::after(Some(&f1), "0.2.19", false, 5000);
        assert_eq!((f2.count, f2.until), (2, Some(5000 + 7200)));
        // another version starts over
        let other = RunFailure::after(Some(&f2), "0.2.20", false, 6000);
        assert_eq!((other.count, other.until), (1, Some(6000 + 3600)));
    }

    #[test]
    fn a_bad_signature_holds_that_version_back_until_another_is_offered() {
        let f = RunFailure::after(None, "0.2.19", true, 1000);
        assert_eq!(f.until, None);
        assert!(f.blocks("0.2.19", 1000 + 10 * INSTALL_BACKOFF.as_secs()));
        assert!(!f.blocks("0.2.20", 1000));
    }

    #[test]
    fn a_run_failure_wait_beyond_a_day_is_over() {
        // noted with the clock far ahead, then put right
        let f = RunFailure::after(None, "0.2.19", false, 100 * INSTALL_BACKOFF.as_secs());
        assert!(!f.blocks("0.2.19", 1000));
    }

    #[test]
    fn a_cleared_attempt_is_nothing() {
        let dir = tempfile::tempdir().unwrap();
        record(dir.path(), "0.2.17", "0.2.18", 100);
        clear(dir.path());
        assert_eq!(check(dir.path(), &v("0.2.17"), 200), None);
    }

    #[test]
    fn a_garbled_file_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), b"{nope").unwrap();
        assert_eq!(check(dir.path(), &v("0.2.17"), 200), None);
        assert!(!dir.path().join(FILE).exists());
    }

    #[test]
    fn only_our_installer_dirs_are_cleaned() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path();
        let mk = |dir: &str, files: &[&str]| {
            std::fs::create_dir(p.join(dir)).unwrap();
            for f in files {
                std::fs::write(p.join(dir).join(f), b"x").unwrap();
            }
        };
        mk(
            "Pulse-0.2.17-updater-AbC123",
            &["Pulse-0.2.17-installer.exe"],
        );
        mk("Pulse-0.2.16-updater-zzzzzz", &[]);
        // something else in there: left alone
        mk(
            "Pulse-0.2.15-updater-q1w2e3",
            &["Pulse-0.2.15-installer.exe", "notes.txt"],
        );
        // not ours
        mk(
            "Other-0.2.17-updater-AbC123",
            &["Other-0.2.17-installer.exe"],
        );
        mk(
            "Pulse-latest-updater-AbC123",
            &["Pulse-latest-installer.exe"],
        );
        std::fs::write(p.join("Pulse-0.2.17-installer.exe"), b"x").unwrap();

        delete_stale_installers(p, "Pulse");
        assert!(!p.join("Pulse-0.2.17-updater-AbC123").exists());
        assert!(!p.join("Pulse-0.2.16-updater-zzzzzz").exists());
        assert!(
            p.join("Pulse-0.2.15-updater-q1w2e3/Pulse-0.2.15-installer.exe")
                .exists()
        );
        assert!(
            p.join("Other-0.2.17-updater-AbC123/Other-0.2.17-installer.exe")
                .exists()
        );
        assert!(
            p.join("Pulse-latest-updater-AbC123/Pulse-latest-installer.exe")
                .exists()
        );
        assert!(
            p.join("Pulse-0.2.17-installer.exe").exists(),
            "loose files aren't touched"
        );
    }
}
