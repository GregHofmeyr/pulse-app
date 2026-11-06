//! Which processes are Pulse, which are playing audio, and who owns a window. Windows only: every
//! function returns empty / `None` elsewhere.

use super::plan::Proc;
#[cfg(windows)]
use super::plan::{is_frame_host, title_owner};

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

/// Every process, with its image path where it can be read (else its exe name).
pub fn processes() -> Vec<Proc> {
    #[cfg(windows)]
    return win::processes_with_paths();
    #[cfg(not(windows))]
    Vec::new()
}

/// How the shared window's app was found (logged; never the title itself).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum How {
    /// By the window's own handle, from the capture source.
    Handle,
    /// By its title (no usable handle).
    Title,
    /// Not identified.
    None,
}

/// The processes owning the shared window: by `hwnd` (the picker's capture source) when it is
/// still a window, else by `title` (see `plan::title_owner`). Empty when nobody can be told apart.
pub fn resolve_window(
    hwnd: Option<u64>,
    title: Option<&str>,
    pulse: &[u32],
    procs: &[Proc],
) -> (Vec<u32>, How) {
    #[cfg(windows)]
    {
        let by_handle = hwnd.and_then(|h| win::window_owner(h, procs));
        choose(by_handle, pulse, || match title {
            // a UWP frame found by title is its host, not the app: not identified
            Some(t) => title_owner(&win::windows_titled(t), t, pulse, procs)
                .into_iter()
                .filter(|o| !procs.iter().any(|p| p.pid == *o && is_frame_host(&p.exe)))
                .collect(),
            None => Vec::new(),
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd, title, procs);
        choose(None, pulse, Vec::new)
    }
}

/// `by_handle`: the owner of the handle's window, `Some(0)` when it is a window whose app can't
/// be told (an unresolved UWP frame), `None` when there is no such window. A live window's
/// answer is final (Pulse's own is not identified); only without one does the title count.
fn choose(
    by_handle: Option<u32>,
    pulse: &[u32],
    by_title: impl FnOnce() -> Vec<u32>,
) -> (Vec<u32>, How) {
    match by_handle {
        Some(p) if p != 0 && !pulse.contains(&p) => return (vec![p], How::Handle),
        Some(_) => return (Vec::new(), How::None),
        None => {}
    }
    let t = by_title();
    let how = if t.is_empty() { How::None } else { How::Title };
    (t, how)
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
    use super::Proc;
    use crate::stream::plan::{frame_host_app, is_frame_host};
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindow,
        IsWindowVisible,
    };
    use windows::core::{BOOL, PWSTR};

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

    /// `pid`'s image path, when we may read it (not for elevated or protected processes).
    fn image_path(pid: u32) -> Option<String> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let r = QueryFullProcessImageNameW(
                h,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            );
            let _ = CloseHandle(h);
            r.ok()?;
            Some(String::from_utf16_lossy(&buf[..len as usize]))
        }
    }

    /// Every process, with its image path where readable (two apps can share an exe name).
    pub fn processes_with_paths() -> Vec<Proc> {
        processes()
            .into_iter()
            .map(|(pid, parent, name)| Proc {
                pid,
                parent,
                exe: image_path(pid).unwrap_or(name),
            })
            .collect()
    }

    /// The process owning top-level window `hwnd`, if it is still a window. For a UWP window
    /// (ApplicationFrameHost's), the app behind the frame, or `Some(0)` when the frame has none
    /// (minimised): a window, but no app to follow (see `choose`).
    pub fn window_owner(hwnd: u64, procs: &[Proc]) -> Option<u32> {
        let h = HWND(hwnd as usize as *mut core::ffi::c_void);
        unsafe {
            if !IsWindow(Some(h)).as_bool() {
                return None;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, Some(&mut pid));
            if pid == 0 {
                return None;
            }
            let exe = procs.iter().find(|p| p.pid == pid).map(|p| p.exe.as_str());
            match exe {
                Some(exe) if is_frame_host(exe) => {
                    Some(frame_host_app(pid, exe, &child_window_pids(h)).unwrap_or(0))
                }
                _ => Some(pid),
            }
        }
    }

    /// The owners of `parent`'s child windows.
    fn child_window_pids(parent: HWND) -> Vec<u32> {
        unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> BOOL {
            let out = unsafe { &mut *(lp.0 as *mut Vec<u32>) };
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            if !out.contains(&pid) {
                out.push(pid);
            }
            BOOL(1)
        }
        let mut out: Vec<u32> = Vec::new();
        unsafe {
            let _ = EnumChildWindows(
                Some(parent),
                Some(each),
                LPARAM(&mut out as *mut Vec<u32> as isize),
            );
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

    /// (title, pid, visible) for each top-level window titled exactly `title`.
    pub fn windows_titled(title: &str) -> Vec<(String, u32, bool)> {
        struct Ctx<'a> {
            want: &'a str,
            rows: Vec<(String, u32, bool)>,
        }
        unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> BOOL {
            let ctx = unsafe { &mut *(lp.0 as *mut Ctx) };
            let mut buf = [0u16; 512];
            let n = unsafe { GetWindowTextW(hwnd, &mut buf) } as usize;
            if n > 0 {
                let t = String::from_utf16_lossy(&buf[..n]);
                if t == ctx.want {
                    let mut pid = 0u32;
                    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
                    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
                    ctx.rows.push((t, pid, visible));
                }
            }
            BOOL(1) // every window: a second match makes the title ambiguous
        }
        let mut ctx = Ctx {
            want: title,
            rows: Vec::new(),
        };
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&mut ctx as *mut Ctx as isize));
        }
        ctx.rows
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

    #[test]
    fn the_handle_wins_over_the_title() {
        assert_eq!(
            choose(Some(40), &[10], || vec![50]),
            (vec![40], How::Handle)
        );
    }

    #[test]
    fn a_live_window_without_an_app_is_final() {
        // Pulse's own window, or a UWP frame with no app behind it: not another app by title
        assert_eq!(choose(Some(10), &[10], || vec![50]), (vec![], How::None));
        assert_eq!(choose(Some(0), &[10], || vec![50]), (vec![], How::None));
    }

    #[test]
    fn no_window_for_the_handle_falls_back_to_the_title() {
        assert_eq!(
            choose(None, &[10], || vec![50, 51]),
            (vec![50, 51], How::Title)
        );
        assert_eq!(choose(None, &[10], Vec::new), (vec![], How::None));
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
        assert!(processes().is_empty());
        assert_eq!(
            resolve_window(Some(1), Some("anything"), &[], &[]),
            (vec![], How::None)
        );
    }
}
