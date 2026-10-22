//! Which processes are Pulse, which are playing audio, and who owns a window. Windows only: every
//! function returns empty / `None` elsewhere.

/// The main Pulse processes (exe name equals ours, own included).
pub fn pulse_roots() -> Vec<u32> {
    #[cfg(windows)]
    return win::pulse().0;
    #[cfg(not(windows))]
    Vec::new()
}

/// Every Pulse process: the main ones plus their descendants (WebView2's processes).
pub fn pulse_pids() -> Vec<u32> {
    #[cfg(windows)]
    return win::pulse().1;
    #[cfg(not(windows))]
    Vec::new()
}

/// Processes with an Active render session on the default output device (pid 0, the system
/// sounds session, left out).
pub fn audio_session_pids() -> Vec<u32> {
    #[cfg(windows)]
    return win::audio_session_pids().unwrap_or_default();
    #[cfg(not(windows))]
    Vec::new()
}

/// The process owning the top-level window titled exactly `title`.
pub fn window_pid(title: &str) -> Option<u32> {
    #[cfg(windows)]
    return win::window_pid(title);
    #[cfg(not(windows))]
    {
        let _ = title;
        None
    }
}

/// (roots, roots plus all their descendants) from (pid, parent pid, exe name) rows. Exe names
/// compare case-insensitively, as Windows does.
#[cfg_attr(not(windows), allow(dead_code))]
fn pulse_tree(procs: &[(u32, u32, String)], exe: &str) -> (Vec<u32>, Vec<u32>) {
    let roots: Vec<u32> = procs
        .iter()
        .filter(|p| p.2.eq_ignore_ascii_case(exe))
        .map(|p| p.0)
        .collect();
    let mut all = roots.clone();
    let mut i = 0;
    while i < all.len() {
        let parent = all[i];
        for p in procs.iter().filter(|p| p.1 == parent) {
            // Pids are reused, so a stale parent id can loop back into the tree.
            if !all.contains(&p.0) {
                all.push(p.0);
            }
        }
        i += 1;
    }
    (roots, all)
}

#[cfg(windows)]
mod win {
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextW, GetWindowThreadProcessId,
    };
    use windows::core::BOOL;

    /// (pid, parent pid, exe name) for every process.
    fn processes() -> Vec<(u32, u32, String)> {
        let mut out = Vec::new();
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                return out;
            };
            let mut e = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            if Process32FirstW(snap, &mut e).is_ok() {
                loop {
                    let n = e
                        .szExeFile
                        .iter()
                        .position(|c| *c == 0)
                        .unwrap_or(e.szExeFile.len());
                    out.push((
                        e.th32ProcessID,
                        e.th32ParentProcessID,
                        String::from_utf16_lossy(&e.szExeFile[..n]),
                    ));
                    if Process32NextW(snap, &mut e).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
        }
        out
    }

    /// (roots, all) as in `pulse_tree`, for our own exe name.
    pub fn pulse() -> (Vec<u32>, Vec<u32>) {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
        match exe {
            Some(exe) => super::pulse_tree(&processes(), &exe),
            // Without our own name, at least we are Pulse.
            None => (vec![std::process::id()], vec![std::process::id()]),
        }
    }

    pub fn audio_session_pids() -> Result<Vec<u32>, wasapi::WasapiError> {
        // Fine on a thread that already has COM (either apartment); balanced below when we
        // initialised it.
        let com = wasapi::initialize_mta();
        let r = sessions();
        if com.is_ok() {
            wasapi::deinitialize();
        }
        r
    }

    fn sessions() -> Result<Vec<u32>, wasapi::WasapiError> {
        let device =
            wasapi::DeviceEnumerator::new()?.get_default_device(&wasapi::Direction::Render)?;
        let list = device
            .get_iaudiosessionmanager()?
            .get_audiosessionenumerator()?;
        let mut out = Vec::new();
        for i in 0..list.get_count()? {
            let Ok(s) = list.get_session(i) else { continue };
            if !matches!(s.get_state(), Ok(wasapi::SessionState::Active)) {
                continue;
            }
            match s.get_process_id() {
                Ok(pid) if pid != 0 && !out.contains(&pid) => out.push(pid),
                _ => {}
            }
        }
        Ok(out)
    }

    pub fn window_pid(title: &str) -> Option<u32> {
        struct Ctx<'a> {
            want: &'a str,
            found: Option<u32>,
        }
        unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> BOOL {
            let ctx = unsafe { &mut *(lp.0 as *mut Ctx) };
            let mut buf = [0u16; 512];
            let n = unsafe { GetWindowTextW(hwnd, &mut buf) } as usize;
            if n > 0 && String::from_utf16_lossy(&buf[..n]) == ctx.want {
                let mut pid = 0u32;
                unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
                ctx.found = Some(pid);
                return BOOL(0);
            }
            BOOL(1)
        }
        let mut ctx = Ctx {
            want: title,
            found: None,
        };
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&mut ctx as *mut Ctx as isize));
        }
        ctx.found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, parent: u32, exe: &str) -> (u32, u32, String) {
        (pid, parent, exe.to_string())
    }

    #[test]
    fn pulse_tree_takes_every_instance_and_its_descendants() {
        let procs = [
            row(10, 1, "pulse-app.exe"),
            row(11, 10, "msedgewebview2.exe"),
            row(12, 11, "msedgewebview2.exe"), // renderer: a grandchild
            row(20, 1, "Pulse-App.EXE"),       // a second instance
            row(21, 20, "msedgewebview2.exe"),
            row(30, 1, "game.exe"),
            row(31, 30, "helper.exe"),
        ];
        let (roots, mut all) = pulse_tree(&procs, "pulse-app.exe");
        assert_eq!(roots, vec![10, 20]);
        all.sort_unstable();
        assert_eq!(all, vec![10, 11, 12, 20, 21]);
    }

    #[test]
    fn pulse_tree_survives_a_parent_cycle() {
        // Windows reuses pids, so a stale parent id can point back into the tree.
        let procs = [row(10, 11, "pulse-app.exe"), row(11, 10, "x.exe")];
        let (roots, mut all) = pulse_tree(&procs, "pulse-app.exe");
        assert_eq!(roots, vec![10]);
        all.sort_unstable();
        assert_eq!(all, vec![10, 11]);
    }

    #[cfg(windows)]
    #[test]
    fn pulse_pids_includes_ourselves() {
        let me = std::process::id();
        assert!(pulse_pids().contains(&me));
        assert!(pulse_roots().contains(&me));
    }

    #[cfg(not(windows))]
    #[test]
    fn off_windows_discovery_is_empty() {
        assert!(pulse_roots().is_empty());
        assert!(pulse_pids().is_empty());
        assert!(audio_session_pids().is_empty());
        assert_eq!(window_pid("anything"), None);
    }
}
