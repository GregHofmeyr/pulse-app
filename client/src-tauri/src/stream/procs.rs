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
    let mut out: Vec<u32> = Vec::new();
    for (pid, _) in audio_sessions() {
        if !out.contains(&pid) {
            out.push(pid);
        }
    }
    out
}

/// Each Active render session's process and its peak now (0.0–1.0), as [`audio_session_pids`] finds them; a
/// process can have several.
pub fn audio_sessions() -> Vec<(u32, f32)> {
    #[cfg(windows)]
    return win::audio_sessions().unwrap_or_default();
    #[cfg(not(windows))]
    Vec::new()
}

/// The app's name for the stream controls (#229): from the exe's version info on Windows (read once per path),
/// else from its file name (`plan::app_name`).
pub fn app_name(exe: &str) -> String {
    #[cfg(windows)]
    {
        use std::collections::HashMap;
        use std::sync::{Mutex, OnceLock};
        static NAMES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
        let names = NAMES.get_or_init(Default::default);
        if let Some(n) = names.lock().unwrap().get(exe) {
            return n.clone();
        }
        let (description, product) = win::version_names(exe);
        let n = super::plan::friendly_name(description.as_deref(), product.as_deref(), exe);
        let mut names = names.lock().unwrap();
        // bounded: the apps seen while choosing and streaming
        if names.len() >= 256 {
            names.clear();
        }
        names.insert(exe.to_string(), n.clone());
        n
    }
    #[cfg(not(windows))]
    super::plan::app_name(exe)
}

/// Every process, with its image path where it can be read (else its exe name).
pub fn processes() -> Vec<Proc> {
    #[cfg(windows)]
    return win::processes_with_paths();
    #[cfg(not(windows))]
    Vec::new()
}

/// Every process by exe name only (one snapshot, no process opened): enough to tell Discord apart for Everything
/// (#458) on every rescan.
pub fn process_names() -> Vec<Proc> {
    #[cfg(windows)]
    return win::process_names();
    #[cfg(not(windows))]
    Vec::new()
}

/// The app's icon for the Go Live sound picker (#458), as a `data:image/png` URL: the exe's own icon on Windows (read
/// once per path), else none.
pub fn app_icon(exe: &str) -> Option<String> {
    #[cfg(windows)]
    {
        use std::collections::HashMap;
        use std::sync::{Mutex, OnceLock};
        static ICONS: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
        let icons = ICONS.get_or_init(Default::default);
        if let Some(i) = icons.lock().unwrap().get(exe) {
            return i.clone();
        }
        let icon = win::exe_icon_png(exe).map(|png| {
            use base64::Engine as _;
            format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(png)
            )
        });
        let mut icons = icons.lock().unwrap();
        // bounded: the apps seen while choosing
        if icons.len() >= 64 {
            icons.clear();
        }
        icons.insert(exe.to_string(), icon.clone());
        icon
    }
    #[cfg(not(windows))]
    {
        let _ = exe;
        None
    }
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

    /// Every process by exe name, nothing opened.
    pub fn process_names() -> Vec<Proc> {
        processes()
            .into_iter()
            .map(|(pid, parent, exe)| Proc { pid, parent, exe })
            .collect()
    }

    /// The exe's first icon (32 px) as PNG bytes (#458). None when it has none or can't be read.
    pub fn exe_icon_png(exe: &str) -> Option<Vec<u8>> {
        use windows::Win32::Graphics::Gdi::{
            BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC,
            GetDIBits, GetObjectW, HGDIOBJ, ReleaseDC,
        };
        use windows::Win32::UI::Shell::ExtractIconExW;
        use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};
        use windows::core::HSTRING;

        let path = HSTRING::from(exe);
        let mut large = HICON::default();
        // SAFETY: path is a valid wide string; one icon handle is asked for, into `large`.
        let n = unsafe { ExtractIconExW(&path, 0, Some(&mut large as *mut HICON), None, 1) };
        if n == 0 || large.is_invalid() {
            return None;
        }
        let mut info = ICONINFO::default();
        // SAFETY: `large` is a valid icon; GetIconInfo fills `info` with bitmaps we must delete.
        let got = unsafe { GetIconInfo(large, &mut info) };
        let png = got.ok().and_then(|_| {
            let mut bm = BITMAP::default();
            // SAFETY: hbmColor is a bitmap handle from GetIconInfo; bm is sized for BITMAP.
            let ok = unsafe {
                GetObjectW(
                    HGDIOBJ(info.hbmColor.0),
                    std::mem::size_of::<BITMAP>() as i32,
                    Some((&mut bm as *mut BITMAP).cast()),
                )
            };
            let (w, h) = (bm.bmWidth, bm.bmHeight);
            if ok == 0 || info.hbmColor.is_invalid() || w <= 0 || h <= 0 || w > 256 || h > 256 {
                return None;
            }
            let mut bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    // top-down rows
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut px = vec![0u8; (w * h * 4) as usize];
            // SAFETY: a screen DC, released below; px holds w*h 32-bit pixels as bi describes.
            let rows = unsafe {
                let dc = GetDC(None);
                let rows = GetDIBits(
                    dc,
                    info.hbmColor,
                    0,
                    h as u32,
                    Some(px.as_mut_ptr().cast()),
                    &mut bi,
                    DIB_RGB_COLORS,
                );
                ReleaseDC(None, dc);
                rows
            };
            if rows == 0 {
                return None;
            }
            // BGRA to RGBA; an icon with no alpha at all (old style) is drawn opaque
            let has_alpha = px.chunks_exact(4).any(|p| p[3] != 0);
            for p in px.chunks_exact_mut(4) {
                p.swap(0, 2);
                if !has_alpha {
                    p[3] = 255;
                }
            }
            let img = image::RgbaImage::from_raw(w as u32, h as u32, px)?;
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Png).ok()?;
            Some(out.into_inner())
        });
        // SAFETY: the bitmaps GetIconInfo made and the icon ExtractIconExW made are ours to free.
        unsafe {
            if !info.hbmColor.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
            }
            if !info.hbmMask.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
            }
            let _ = DestroyIcon(large);
        }
        png
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

    pub fn audio_sessions() -> Result<Vec<(u32, f32)>, wasapi::WasapiError> {
        // Fine on a thread that already has COM (either apartment); balanced below when we
        // initialised it.
        let com = wasapi::initialize_mta();
        let r = sessions();
        if com.is_ok() {
            wasapi::deinitialize();
        }
        r
    }

    fn sessions() -> Result<Vec<(u32, f32)>, wasapi::WasapiError> {
        let device =
            wasapi::DeviceEnumerator::new()?.get_default_device(wasapi::Direction::Render)?;
        let list = device
            .get_iaudiosessionmanager()?
            .get_audiosessionenumerator()?;
        let mut out = Vec::new();
        for i in 0..list.get_count()? {
            let Ok(s) = list.get_session(i) else { continue };
            if !matches!(s.get_state(), Ok(wasapi::SessionState::Active)) {
                continue;
            }
            let Ok(pid) = s.get_process_id() else {
                continue;
            };
            if pid == 0 {
                continue;
            }
            // the session's peak (IAudioMeterInformation); a session without a meter reads as silent
            let peak = s
                .get_audiometerinformation()
                .and_then(|m| m.get_peak_value())
                .unwrap_or(0.0);
            out.push((pid, peak));
        }
        Ok(out)
    }

    /// The exe's version info names (FileDescription, ProductName), in its first listed language.
    pub fn version_names(exe: &str) -> (Option<String>, Option<String>) {
        use windows::Win32::Storage::FileSystem::{
            GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
        };
        use windows::core::HSTRING;
        let path = HSTRING::from(exe);
        // SAFETY: path is a valid wide string; the handle argument is unused.
        let size = unsafe { GetFileVersionInfoSizeW(&path, None) };
        if size == 0 {
            return (None, None);
        }
        let mut data = vec![0u8; size as usize];
        // SAFETY: data has `size` bytes, as asked for.
        if unsafe { GetFileVersionInfoW(&path, None, size, data.as_mut_ptr().cast()) }.is_err() {
            return (None, None);
        }
        let query = |sub: &str| -> Option<(*const u8, usize)> {
            let mut ptr: *mut core::ffi::c_void = std::ptr::null_mut();
            let mut len = 0u32;
            // SAFETY: data is the version block just read; ptr and len receive a pointer into it.
            let ok = unsafe {
                VerQueryValueW(
                    data.as_ptr().cast(),
                    &HSTRING::from(sub),
                    &mut ptr,
                    &mut len,
                )
            };
            (ok.as_bool() && !ptr.is_null() && len > 0).then_some((ptr as *const u8, len as usize))
        };
        // the first (language, code page) pair
        let Some((ptr, len)) = query(r"\VarFileInfo\Translation") else {
            return (None, None);
        };
        if len < 4 {
            return (None, None);
        }
        // SAFETY: at least 4 bytes at ptr, inside data.
        let (lang, cp) = unsafe {
            (
                u16::from_le_bytes([*ptr, *ptr.add(1)]),
                u16::from_le_bytes([*ptr.add(2), *ptr.add(3)]),
            )
        };
        let string = |name: &str| -> Option<String> {
            let (ptr, len) = query(&format!(r"\StringFileInfo\{lang:04x}{cp:04x}\{name}"))?;
            // SAFETY: `len` UTF-16 units at ptr, inside data (VerQueryValueW counts characters for strings).
            let units = unsafe { std::slice::from_raw_parts(ptr as *const u16, len) };
            let end = units.iter().position(|c| *c == 0).unwrap_or(units.len());
            Some(String::from_utf16_lossy(&units[..end]))
        };
        (string("FileDescription"), string("ProductName"))
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
