//! Real-time scheduling for our own audio threads. On Windows they join the multimedia class
//! scheduler (MMCSS) as "Pro Audio" (or "Audio" if that's refused), so a game pinning the CPU
//! can't starve the mic processor or Strong's worker. cpal ≥ 0.18 with `realtime` does the same
//! for its own WASAPI threads. Elsewhere this is a no-op.

use std::sync::Mutex;

/// Keeps the calling thread registered; dropping it (on that same thread) unregisters.
#[must_use = "the thread leaves MMCSS when this is dropped"]
pub struct Registration {
    #[cfg(windows)]
    handle: Option<windows::Win32::Foundation::HANDLE>,
    /// The MMCSS task we got ("Pro Audio" / "Audio"), or None.
    pub task: Option<&'static str>,
}

/// Register the calling thread; `what` names it in the log. Success is logged once per kind per
/// run (the processor thread is respawned for every call); a refusal every time, since one that
/// comes after an earlier success is what a "choppy under load" report needs to show.
pub fn promote(what: &'static str) -> Registration {
    let (r, err) = register();
    match (r.task, err) {
        (Some(task), _) if first_time(what) => {
            tracing::info!(
                thread = what,
                task,
                "audio thread: real-time priority (MMCSS)"
            )
        }
        (None, Some(error)) => tracing::warn!(
            thread = what,
            %error,
            "audio thread: MMCSS refused; normal priority"
        ),
        (None, None) if first_time(what) => {
            tracing::debug!(thread = what, "audio thread: no MMCSS on this OS")
        }
        _ => {}
    }
    r
}

static LOGGED: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

fn first_time(what: &'static str) -> bool {
    let mut seen = LOGGED.lock().unwrap_or_else(|e| e.into_inner());
    if seen.contains(&what) {
        false
    } else {
        seen.push(what);
        true
    }
}

#[cfg(windows)]
fn register() -> (Registration, Option<String>) {
    use windows::Win32::System::Threading::AvSetMmThreadCharacteristicsW;
    use windows::core::w;
    let mut last_err = None;
    for (name, task) in [("Pro Audio", w!("Pro Audio")), ("Audio", w!("Audio"))] {
        let mut index = 0u32;
        // SAFETY: `task` is a static NUL-terminated wide string and `index` a live out-pointer;
        // the returned handle is released by `Drop` on this same thread.
        match unsafe { AvSetMmThreadCharacteristicsW(task, &mut index) } {
            Ok(h) => {
                return (
                    Registration {
                        handle: Some(h),
                        task: Some(name),
                    },
                    None,
                );
            }
            Err(e) => last_err = Some(e.to_string()),
        }
    }
    (
        Registration {
            handle: None,
            task: None,
        },
        last_err,
    )
}

#[cfg(not(windows))]
fn register() -> (Registration, Option<String>) {
    (Registration { task: None }, None)
}

#[cfg(windows)]
impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            // SAFETY: `h` came from AvSetMmThreadCharacteristicsW on this thread (HANDLE is !Send,
            // so the Registration can't have moved threads).
            let _ =
                unsafe { windows::Win32::System::Threading::AvRevertMmThreadCharacteristics(h) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// cpal 0.15 passed a thread id where a handle belongs, so its WASAPI threads were never
    /// raised; 0.18 with `realtime` registers them with MMCSS. Guard against a downgrade.
    #[test]
    fn cpal_raises_its_own_wasapi_threads() {
        let manifest = include_str!("../../Cargo.toml");
        let line = manifest
            .lines()
            .find(|l| l.starts_with("cpal") && l.contains("realtime"))
            .expect("cpal with the `realtime` feature (Windows target dependency)");
        assert!(line.contains("\"0.18"), "{line}");
    }

    #[test]
    fn each_thread_kind_is_logged_once() {
        assert!(first_time("test-kind-a"));
        assert!(!first_time("test-kind-a"));
        assert!(first_time("test-kind-b"));
    }

    #[cfg(not(windows))]
    #[test]
    fn elsewhere_promotion_is_a_harmless_no_op() {
        let r = promote("test-noop");
        assert_eq!(r.task, None);
    }

    /// Runs only on Windows (no Windows CI: see Step 5 for the check on the owner's machine).
    #[cfg(windows)]
    #[test]
    fn windows_threads_join_mmcss() {
        let r = promote("test-mmcss");
        assert!(r.task.is_some(), "MMCSS refused this thread");
    }
}
