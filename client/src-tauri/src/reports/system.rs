//! The machine and this process, for bug reports (#605): memory, CPU, uptime and (on Windows) battery. Read from
//! the OS directly (`/proc` on Linux, a handful of Win32 calls on Windows): no system-info crate. Nothing here names
//! the machine or the person: no hostname, user name or path.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use serde::Serialize;

/// When the app started (forced early by `mark_started`; a report before that counts from the first ask).
static STARTED: LazyLock<Instant> = LazyLock::new(Instant::now);

/// How long the CPU sample runs: long enough to mean something, short enough not to hold up the report.
const CPU_SAMPLE: Duration = Duration::from_millis(250);

pub fn mark_started() {
    LazyLock::force(&STARTED);
}

pub fn uptime() -> Duration {
    STARTED.elapsed()
}

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct System {
    pub uptime_s: u64,
    pub logical_cpus: Option<usize>,
    pub cpu_model: Option<String>,
    pub ram_total_mb: Option<u64>,
    /// What the OS says is available (Linux `MemAvailable`, Windows `ullAvailPhys`).
    pub ram_free_mb: Option<u64>,
    pub process: Process,
    /// Windows only (none on a desktop without a battery).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power: Option<Power>,
}

/// This process only: on Windows the WebView2 processes (the page) are separate and not counted.
#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct Process {
    /// Resident set (Linux `VmRSS`) / working set (Windows).
    pub memory_mb: Option<u64>,
    /// Memory only this process holds (Linux `RssAnon`, Windows private bytes).
    pub private_mb: Option<u64>,
    /// CPU over a short sample just now, as a share of the whole machine (Task Manager's way: 100 = every core).
    pub cpu_pct: Option<f64>,
    /// The same averaged over the app's life so far.
    pub cpu_avg_pct: Option<f64>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct Power {
    pub on_battery: Option<bool>,
    pub battery_pct: Option<u8>,
    /// Windows' battery saver is on.
    pub battery_saver: bool,
}

/// Everything above, sampling the CPU for [`CPU_SAMPLE`].
pub async fn gather() -> System {
    let t0 = (Instant::now(), process_cpu_time());
    tokio::time::sleep(CPU_SAMPLE).await;
    let t1 = (Instant::now(), process_cpu_time());
    let cpus = std::thread::available_parallelism().ok().map(|n| n.get());
    let up = uptime();
    let cpu_pct = match (t0.1, t1.1, cpus) {
        (Some(a), Some(b), Some(n)) => cpu_share(b.saturating_sub(a), t1.0 - t0.0, n),
        _ => None,
    };
    let cpu_avg_pct = match (t1.1, cpus) {
        (Some(total), Some(n)) => cpu_share(total, up, n),
        _ => None,
    };
    // file reads and syscalls: off the async workers
    let rest = tokio::task::spawn_blocking(move || {
        let (memory, private) = process_memory();
        let (total, free) = ram();
        (memory, private, total, free, cpu_model(), power())
    })
    .await;
    let (memory, private, total, free, model, power) =
        rest.unwrap_or((None, None, None, None, None, None));
    System {
        uptime_s: up.as_secs(),
        logical_cpus: cpus,
        cpu_model: model,
        ram_total_mb: total.map(mb),
        ram_free_mb: free.map(mb),
        process: Process {
            memory_mb: memory.map(mb),
            private_mb: private.map(mb),
            cpu_pct,
            cpu_avg_pct,
        },
        power,
    }
}

fn mb(bytes: u64) -> u64 {
    bytes / (1024 * 1024)
}

/// `used` CPU time over `wall`, as a percentage of `cpus` cores' worth, to one decimal.
pub fn cpu_share(used: Duration, wall: Duration, cpus: usize) -> Option<f64> {
    if wall.is_zero() || cpus == 0 {
        return None;
    }
    let pct = used.as_secs_f64() / (wall.as_secs_f64() * cpus as f64) * 100.0;
    Some((pct.clamp(0.0, 100.0) * 10.0).round() / 10.0)
}

// ---- Linux: /proc ----

#[cfg(target_os = "linux")]
fn process_cpu_time() -> Option<Duration> {
    // SAFETY: sysconf only reads a configuration value.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    parse_stat_cpu(&stat, u64::try_from(ticks).ok()?)
}

#[cfg(target_os = "linux")]
fn process_memory() -> (Option<u64>, Option<u64>) {
    match std::fs::read_to_string("/proc/self/status") {
        Ok(s) => (kb_field(&s, "VmRSS:"), kb_field(&s, "RssAnon:")),
        Err(_) => (None, None),
    }
}

#[cfg(target_os = "linux")]
fn ram() -> (Option<u64>, Option<u64>) {
    match std::fs::read_to_string("/proc/meminfo") {
        Ok(s) => (kb_field(&s, "MemTotal:"), kb_field(&s, "MemAvailable:")),
        Err(_) => (None, None),
    }
}

#[cfg(target_os = "linux")]
fn cpu_model() -> Option<String> {
    parse_cpu_model(&std::fs::read_to_string("/proc/cpuinfo").ok()?)
}

#[cfg(target_os = "linux")]
fn power() -> Option<Power> {
    None
}

/// utime + stime from `/proc/<pid>/stat` (fields 14 and 15, counted after the `(comm)`, which may hold spaces).
#[cfg(any(target_os = "linux", test))]
pub fn parse_stat_cpu(stat: &str, ticks_per_sec: u64) -> Option<Duration> {
    if ticks_per_sec == 0 {
        return None;
    }
    let after = &stat[stat.rfind(')')? + 1..];
    // after the comm: state is field 3, so utime (14) is the 12th from here
    let mut f = after.split_whitespace().skip(11);
    let utime: u64 = f.next()?.parse().ok()?;
    let stime: u64 = f.next()?.parse().ok()?;
    Some(Duration::from_secs_f64(
        (utime + stime) as f64 / ticks_per_sec as f64,
    ))
}

/// A `Name:   1234 kB` line's value in bytes.
#[cfg(any(target_os = "linux", test))]
pub fn kb_field(text: &str, name: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with(name))?;
    let kb: u64 = line[name.len()..].split_whitespace().next()?.parse().ok()?;
    Some(kb * 1024)
}

#[cfg(any(target_os = "linux", test))]
pub fn parse_cpu_model(cpuinfo: &str) -> Option<String> {
    cpuinfo
        .lines()
        .find(|l| l.starts_with("model name"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

// ---- Windows ----

#[cfg(windows)]
fn filetime(t: windows::Win32::Foundation::FILETIME) -> Duration {
    // 100 ns units
    let n = (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    Duration::from_nanos(n.saturating_mul(100))
}

#[cfg(windows)]
fn process_cpu_time() -> Option<Duration> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: the pseudo-handle of our own process; four FILETIMEs to fill.
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    }
    .ok()?;
    Some(filetime(kernel) + filetime(user))
}

#[cfg(windows)]
fn process_memory() -> (Option<u64>, Option<u64>) {
    use windows::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut c = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    // SAFETY: our own process's pseudo-handle; `c` is the _EX struct and `cb` says so (the API takes it via the
    // base struct's pointer).
    let r = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut c as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            c.cb,
        )
    };
    match r {
        Ok(()) => (Some(c.WorkingSetSize as u64), Some(c.PrivateUsage as u64)),
        Err(_) => (None, None),
    }
}

#[cfg(windows)]
fn ram() -> (Option<u64>, Option<u64>) {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: `m` is a MEMORYSTATUSEX with its length set, as the call requires.
    match unsafe { GlobalMemoryStatusEx(&mut m) } {
        Ok(()) => (Some(m.ullTotalPhys), Some(m.ullAvailPhys)),
        Err(_) => (None, None),
    }
}

#[cfg(windows)]
fn cpu_model() -> Option<String> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    use windows::core::w;
    let mut buf = vec![0u16; 256];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: static wide key/value names; `buf` holds `size` bytes.
    let r = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0"),
            w!("ProcessorNameString"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if r != ERROR_SUCCESS {
        return None;
    }
    let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    let s = String::from_utf16_lossy(&buf[..n]).trim().to_string();
    (!s.is_empty()).then_some(s)
}

#[cfg(windows)]
fn power() -> Option<Power> {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut p = SYSTEM_POWER_STATUS::default();
    // SAFETY: `p` is the struct the call fills.
    unsafe { GetSystemPowerStatus(&mut p) }.ok()?;
    power_from(
        p.ACLineStatus,
        p.BatteryFlag,
        p.BatteryLifePercent,
        p.SystemStatusFlag,
    )
}

/// `SYSTEM_POWER_STATUS`'s fields as a [`Power`]; `None` for a PC with no battery (battery flag 128).
#[cfg(any(windows, test))]
pub fn power_from(ac_line: u8, battery_flag: u8, pct: u8, status_flag: u8) -> Option<Power> {
    if battery_flag == 128 {
        return None;
    }
    Some(Power {
        on_battery: match ac_line {
            0 => Some(true),
            1 => Some(false),
            _ => None,
        },
        battery_pct: (pct <= 100).then_some(pct),
        battery_saver: status_flag == 1,
    })
}

#[cfg(not(any(target_os = "linux", windows)))]
fn process_cpu_time() -> Option<Duration> {
    None
}

#[cfg(not(any(target_os = "linux", windows)))]
fn process_memory() -> (Option<u64>, Option<u64>) {
    (None, None)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn ram() -> (Option<u64>, Option<u64>) {
    (None, None)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn cpu_model() -> Option<String> {
    None
}

#[cfg(not(any(target_os = "linux", windows)))]
fn power() -> Option<Power> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_cpu_skips_a_comm_with_spaces_and_parens() {
        // pid (comm) state ppid pgrp session tty tpgid flags minflt cminflt majflt cmajflt utime stime ...
        let stat = "4242 (pulse (app) x) S 1 4242 4242 0 -1 4194560 100 0 0 0 250 50 0 0 20 0 30 0";
        assert_eq!(parse_stat_cpu(stat, 100), Some(Duration::from_secs(3)));
        assert_eq!(parse_stat_cpu(stat, 0), None);
        assert_eq!(parse_stat_cpu("garbage", 100), None);
    }

    #[test]
    fn kb_fields_read_in_bytes() {
        let status = "Name:\tpulse\nVmRSS:\t  204800 kB\nRssAnon:\t  102400 kB\n";
        assert_eq!(kb_field(status, "VmRSS:"), Some(200 * 1024 * 1024));
        assert_eq!(kb_field(status, "RssAnon:"), Some(100 * 1024 * 1024));
        assert_eq!(kb_field(status, "VmSwap:"), None);
    }

    #[test]
    fn cpu_model_is_the_first_model_name() {
        let info = "processor\t: 0\nvendor_id\t: AuthenticAMD\nmodel name\t: AMD Ryzen 7 7840U\nprocessor\t: 1\nmodel name\t: AMD Ryzen 7 7840U\n";
        assert_eq!(parse_cpu_model(info).as_deref(), Some("AMD Ryzen 7 7840U"));
        assert_eq!(parse_cpu_model("processor\t: 0\n"), None);
    }

    #[test]
    fn cpu_share_is_of_the_whole_machine() {
        let s = |used_ms, wall_ms, cpus| {
            cpu_share(
                Duration::from_millis(used_ms),
                Duration::from_millis(wall_ms),
                cpus,
            )
        };
        assert_eq!(s(250, 250, 1), Some(100.0));
        assert_eq!(s(250, 250, 8), Some(12.5));
        assert_eq!(s(1, 3, 1), Some(33.3));
        assert_eq!(s(10, 0, 4), None);
        // timer slop can't make it more than everything
        assert_eq!(s(300, 250, 1), Some(100.0));
    }

    #[test]
    fn power_reads_the_battery_and_saver() {
        assert_eq!(power_from(1, 128, 255, 0), None, "no battery");
        assert_eq!(
            power_from(0, 0, 42, 1),
            Some(Power {
                on_battery: Some(true),
                battery_pct: Some(42),
                battery_saver: true,
            })
        );
        let unknown = power_from(255, 0, 255, 0).unwrap();
        assert_eq!(unknown.on_battery, None);
        assert_eq!(unknown.battery_pct, None);
    }

    #[tokio::test]
    async fn gather_fills_what_this_os_has() {
        let s = gather().await;
        assert!(s.logical_cpus.is_some_and(|n| n > 0));
        #[cfg(any(target_os = "linux", windows))]
        {
            assert!(s.ram_total_mb.is_some_and(|t| t > 0));
            assert!(s.process.memory_mb.is_some());
            assert!(s.process.cpu_pct.is_some());
        }
        let v = serde_json::to_value(&s).unwrap();
        assert!(v["process"].is_object());
    }
}
