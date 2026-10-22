//! THROWAWAY (screen-share spike): commands for the hidden debug panel. Results are `spike:` log lines,
//! so every bug report carries them.

pub mod loopback; // builds on every OS; only its WASAPI capture is Windows-only
pub mod onset;

use pulse_protocol::ids::ChannelId;
use pulse_protocol::rest::VoiceTokenResponse;
use serde::Serialize;
use tauri::State;

use crate::api::ApiError;
use crate::commands::Core;

/// wry's default WebView2 args plus whatever the spike run asks for (S1 HW off, S6 throttling, S7 decode).
pub fn webview_args(defaults: &str, extra: Option<&str>) -> String {
    let extra: Vec<&str> = extra.unwrap_or("").split_whitespace().collect();
    std::iter::once(defaults)
        .chain(extra)
        .collect::<Vec<_>>()
        .join(" ")
}

#[tauri::command]
pub fn spike_log(line: String) {
    tracing::info!(target: "spike", "spike: {line}");
}

#[tauri::command]
pub async fn spike_ticket(
    core: State<'_, Core>,
    channel_id: ChannelId,
    role: String,
) -> Result<VoiceTokenResponse, ApiError> {
    let (api, token) = core.current()?;
    api.spike_ticket(&token, channel_id, &role).await
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OsStats {
    pulse_private_mb: f64,
    webview_private_mb: f64,
    handles: u32,
    nonpaged_pool_mb: f64,
    dwm_ws_mb: f64,
    audiodg_ws_mb: f64,
    gpu_dedicated_mb: Option<f64>,
}

#[tauri::command]
pub fn spike_os_stats() -> OsStats {
    #[cfg(windows)]
    {
        win::os_stats()
    }
    #[cfg(not(windows))]
    {
        OsStats::default()
    }
}

#[tauri::command]
pub fn spike_window_pid(title: String) -> Option<u32> {
    #[cfg(windows)]
    {
        win::window_pid(&title)
    }
    #[cfg(not(windows))]
    {
        let _ = title;
        None
    }
}

#[cfg(windows)]
mod win {
    use super::OsStats;
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::ProcessStatus::{
        GetPerformanceInfo, GetProcessMemoryInfo, PERFORMANCE_INFORMATION,
        PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentProcessId, GetProcessHandleCount, OpenProcess,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextW, GetWindowThreadProcessId,
    };
    use windows::core::BOOL;

    const MB: f64 = 1024.0 * 1024.0;

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

    /// (private bytes, working set) of one process, in MB.
    fn mem(pid: u32) -> (f64, f64) {
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return (0.0, 0.0);
            };
            let mut c = PROCESS_MEMORY_COUNTERS_EX::default();
            let ok = GetProcessMemoryInfo(
                h,
                &mut c as *mut _ as *mut _,
                std::mem::size_of_val(&c) as u32,
            )
            .is_ok();
            let _ = CloseHandle(h);
            if ok {
                (c.PrivateUsage as f64 / MB, c.WorkingSetSize as f64 / MB)
            } else {
                (0.0, 0.0)
            }
        }
    }

    pub fn os_stats() -> OsStats {
        let me = unsafe { GetCurrentProcessId() };
        let procs = processes();
        // WebView2's browser process is our child; its renderers/GPU process are ITS children.
        let mut tree = vec![me];
        let mut i = 0;
        while i < tree.len() {
            let parent = tree[i];
            tree.extend(procs.iter().filter(|p| p.1 == parent).map(|p| p.0));
            i += 1;
        }
        let webview: f64 = tree[1..].iter().map(|pid| mem(*pid).0).sum();
        let by_name = |n: &str| {
            procs
                .iter()
                .find(|p| p.2.eq_ignore_ascii_case(n))
                .map_or(0.0, |p| mem(p.0).1)
        };
        let mut handles = 0u32;
        unsafe {
            let _ = GetProcessHandleCount(GetCurrentProcess(), &mut handles);
        }
        let mut perf = PERFORMANCE_INFORMATION::default();
        unsafe {
            let _ = GetPerformanceInfo(&mut perf, std::mem::size_of_val(&perf) as u32);
        }
        OsStats {
            pulse_private_mb: mem(me).0,
            webview_private_mb: webview,
            handles,
            nonpaged_pool_mb: perf.KernelNonpaged as f64 * perf.PageSize as f64 / MB,
            dwm_ws_mb: by_name("dwm.exe"),
            audiodg_ws_mb: by_name("audiodg.exe"),
            // DXGI per-adapter usage needs IDXGIAdapter3::QueryVideoMemoryInfo; the protocol has the owner read
            // Task Manager's "Dedicated GPU memory" instead (S3), so None here.
            gpu_dedicated_mb: None,
        }
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

    #[test]
    fn extra_args_are_appended_to_the_defaults() {
        assert_eq!(
            webview_args("--disable-features=A", Some("--enable-features=B  --x")),
            "--disable-features=A --enable-features=B --x"
        );
        assert_eq!(
            webview_args("--disable-features=A", None),
            "--disable-features=A"
        );
        assert_eq!(
            webview_args("--disable-features=A", Some("   ")),
            "--disable-features=A"
        );
    }
}
