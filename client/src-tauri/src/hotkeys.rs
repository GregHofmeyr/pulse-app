//! Global binds (Windows; on Linux the compositor binds the CLI, see ipc.rs). Low-level keyboard and
//! mouse hooks see every key and button system-wide and always pass it on: a bind toggles mute or
//! deafen and the key still works in the focused app, like Discord. Which press fires what is
//! decided by `bindmatch::Matcher`. Paused while a bind is being recorded. Windows can drop the
//! hooks silently; a watchdog notices (`hookwatch`) and reinstalls them.
//!
//! The user can switch global binds off: then both hooks are removed (not just ignored), the
//! watchdog stands down and stops polling input, and at startup they're never installed. The hook
//! thread (and the watchdog) still run, idle, so switching back on is instant.

#[cfg(windows)]
mod platform {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{LazyLock, Mutex, OnceLock, mpsc};
    use std::time::{Duration, Instant};

    use tauri::AppHandle;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::StationsAndDesktops::{
        CloseDesktop, DESKTOP_CONTROL_FLAGS, DESKTOP_READOBJECTS, OpenInputDesktop,
    };
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentThread, GetCurrentThreadId, OpenProcess, OpenProcessToken,
        PROCESS_QUERY_LIMITED_INFORMATION, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, GetLastInputInfo, LASTINPUTINFO, MAPVK_VK_TO_VSC_EX, MapVirtualKeyW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, HHOOK,
        KBDLLHOOKSTRUCT, LLKHF_EXTENDED, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, PeekMessageW,
        PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL,
        WM_APP, WM_KEYDOWN, WM_KEYUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
        WM_XBUTTONDOWN, WM_XBUTTONUP,
    };

    use crate::bindmatch::{Matcher, key_code};
    use crate::hookwatch::{
        Check, Gate, HookStep, HookWatch, hook_step, switch_problem, watch_gate,
    };
    use crate::keybinds::{Action, Keybinds};

    /// Locked by the hook procedures once per event and briefly by apply/pause; nothing else.
    static MATCHER: LazyLock<Mutex<Matcher>> = LazyLock::new(Default::default);
    /// Fired actions go to a forwarder thread: the hook procedure must return at once, or Windows
    /// silently drops the hook (LowLevelHooksTimeout).
    static FIRED: OnceLock<mpsc::Sender<Action>> = OnceLock::new();
    /// Set if the hooks couldn't be installed, cleared when they are (or switched off); reported
    /// in Settings while the switch is on.
    static BROKEN: Mutex<Option<String>> = Mutex::new(None);
    /// The user's switch. Written by `install`/`set_enabled` (then a WM_SWITCH is posted), read by
    /// the hook thread when it handles a message: the switch as it is then, not as it was.
    static ENABLED: AtomicBool = AtomicBool::new(true);
    /// Whether the hook thread holds hooks (it alone writes this). The watchdog only checks while
    /// it's set, so it never polls input, nor asks for a reinstall, while switched off.
    static HOOKED: AtomicBool = AtomicBool::new(false);
    /// The hook thread, for posting to.
    static HOOK_THREAD: OnceLock<u32> = OnceLock::new();
    /// `GetTickCount` when a hook last ran: the watchdog's evidence that they're still installed.
    static LAST_HOOK_TICK: AtomicU32 = AtomicU32::new(0);

    /// Posted to the hook thread to reinstall its hooks (hooks belong to the installing thread).
    const WM_REINSTALL: u32 = WM_APP + 1;
    /// Posted to the hook thread after ENABLED changed: install or remove the hooks to match.
    const WM_SWITCH: u32 = WM_APP + 2;
    /// How often the watchdog looks.
    const WATCH_EVERY: Duration = Duration::from_secs(3);
    /// System input this much newer than the hooks' last event means they're gone; less is the
    /// race between an event reaching the hook and the watchdog reading the system tick. A hook
    /// thread stalled for longer than this reads as dead too (REINSTALL_GAP caps the cost).
    const DEAD_MARGIN_MS: u32 = 1000;
    /// At most one reinstall per this long.
    const REINSTALL_GAP: Duration = Duration::from_secs(10);

    /// Called first thing in each hook procedure: one atomic store, nothing that can block.
    fn saw_input() {
        // SAFETY: GetTickCount has no preconditions.
        LAST_HOOK_TICK.store(unsafe { GetTickCount() }, Ordering::Relaxed);
    }

    fn matcher() -> std::sync::MutexGuard<'static, Matcher> {
        MATCHER.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether Windows thinks a modifier is down (it tracks key-ups we never saw).
    fn still_down(code: &str) -> bool {
        let vk = match code {
            "ControlLeft" => 0xA2,
            "ControlRight" => 0xA3,
            "ShiftLeft" => 0xA0,
            "ShiftRight" => 0xA1,
            "AltLeft" => 0xA4,
            "AltRight" => 0xA5,
            "MetaLeft" => 0x5B,
            "MetaRight" => 0x5C,
            _ => return true,
        };
        (unsafe { GetAsyncKeyState(vk) }) < 0
    }

    fn event(code: &'static str, down: bool) {
        if !down {
            return matcher().up(code);
        }
        let fired = {
            let mut m = matcher();
            m.forget_released(still_down);
            m.down(code, Instant::now())
        };
        if let Some(tx) = FIRED.get() {
            for a in fired {
                let _ = tx.send(a);
            }
        }
    }

    /// Keys by physical position (scan code), so a layout switch doesn't move binds. Injected keys
    /// (gaming-keyboard software, remappers) count like real ones; injected text (VK_PACKET)
    /// doesn't.
    unsafe extern "system" fn keyboard(n: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if n >= 0 {
            saw_input();
            // SAFETY: for WH_KEYBOARD_LL with n >= 0, lParam points to a KBDLLHOOKSTRUCT.
            let k = unsafe { &*(l.0 as *const KBDLLHOOKSTRUCT) };
            let ext = k.flags.0 & LLKHF_EXTENDED.0 != 0;
            // SAFETY: MapVirtualKeyW only reads its arguments.
            let to_scan = |vk| unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC_EX) };
            if let Some(code) = key_code(k.vkCode, k.scanCode, ext, to_scan) {
                match w.0 as u32 {
                    WM_KEYDOWN | WM_SYSKEYDOWN => event(code, true),
                    WM_KEYUP | WM_SYSKEYUP => event(code, false),
                    _ => {}
                }
            }
        }
        unsafe { CallNextHookEx(None, n, w, l) }
    }

    /// Wheel click and the side buttons; everything else (moves, left/right clicks) passes untouched.
    unsafe extern "system" fn mouse(n: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if n >= 0 {
            // every event, moves included: they update the system's last-input tick too
            saw_input();
            let msg = w.0 as u32;
            let button = match msg {
                WM_MBUTTONDOWN | WM_MBUTTONUP => Some("Mouse3"),
                WM_XBUTTONDOWN | WM_XBUTTONUP => {
                    // SAFETY: for WH_MOUSE_LL with n >= 0, lParam points to an MSLLHOOKSTRUCT.
                    let m = unsafe { &*(l.0 as *const MSLLHOOKSTRUCT) };
                    match (m.mouseData >> 16) & 0xFFFF {
                        1 => Some("Mouse4"),
                        2 => Some("Mouse5"),
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some(code) = button {
                event(code, matches!(msg, WM_MBUTTONDOWN | WM_XBUTTONDOWN));
            }
        }
        unsafe { CallNextHookEx(None, n, w, l) }
    }

    fn hook() -> Result<[HHOOK; 2], String> {
        // SAFETY: plain Win32 calls; the procedures are 'static functions.
        unsafe {
            let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            let keys = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), Some(module.into()), 0)
                .map_err(|e| e.to_string())?;
            match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), Some(module.into()), 0) {
                Ok(buttons) => Ok([keys, buttons]),
                Err(e) => {
                    let _ = UnhookWindowsHookEx(keys);
                    Err(e.to_string())
                }
            }
        }
    }

    fn unhook(hooks: [HHOOK; 2]) {
        for h in hooks {
            // Windows may already have removed it; nothing to do if so.
            // SAFETY: h came from SetWindowsHookExW on this thread and isn't used again.
            let _ = unsafe { UnhookWindowsHookEx(h) };
        }
    }

    /// Windows times a hook callback (LowLevelHooksTimeout) from the input event, waiting to be
    /// scheduled included, so a normal-priority hook thread on a machine a game is loading can
    /// overrun it and lose its hooks. TIME_CRITICAL is the top of the normal priority class (not
    /// real-time): ahead of any app's own threads, ABOVE_NORMAL isn't, as games often raise
    /// theirs. It costs nothing: the thread is blocked in GetMessageW but for a few microseconds
    /// per event.
    fn raise_priority() -> Result<(), String> {
        // SAFETY: GetCurrentThread's pseudo-handle is always valid on the calling thread.
        unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) }
            .map_err(|e| e.to_string())
    }

    fn set_broken(problem: Option<String>) {
        *BROKEN.lock().unwrap_or_else(|e| e.into_inner()) = problem;
    }

    /// Hooks are delivered through the installing thread's message loop, so they get their own
    /// thread that does nothing else (but add, remove or reinstall them when asked). It starts
    /// with no hooks and installs them only if the switch is on.
    fn run_hooks(ready: mpsc::Sender<u32>) {
        if let Err(e) = raise_priority() {
            tracing::warn!(error = %e, "keybinds: hook thread priority couldn't be raised");
        }
        // Make sure this thread has a message queue before anyone posts to it.
        let mut msg = MSG::default();
        // SAFETY: msg is a valid MSG to write into.
        let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
        let mut hooks = step(None, false);
        // SAFETY: GetCurrentThreadId has no preconditions.
        let _ = ready.send(unsafe { GetCurrentThreadId() });
        loop {
            // SAFETY: msg is a valid MSG to write into.
            let got = unsafe { GetMessageW(&mut msg, None, 0, 0) }.0;
            if got == 0 || got == -1 {
                break;
            }
            let reinstall_asked = match msg.message {
                WM_REINSTALL => true,
                WM_SWITCH => false,
                _ => continue,
            };
            hooks = step(hooks, reinstall_asked);
        }
        if let Some(h) = hooks {
            unhook(h);
        }
        HOOKED.store(false, Ordering::SeqCst);
    }

    /// One message's worth of work on the hook thread: make the hooks match the switch, or
    /// reinstall them for the watchdog (`hookwatch::hook_step` decides which).
    fn step(hooks: Option<[HHOOK; 2]>, reinstall_asked: bool) -> Option<[HHOOK; 2]> {
        let enabled = ENABLED.load(Ordering::SeqCst);
        match hook_step(enabled, hooks.is_some(), reinstall_asked) {
            HookStep::Nothing => hooks,
            HookStep::Unhook => {
                if let Some(h) = hooks {
                    unhook(h);
                }
                HOOKED.store(false, Ordering::SeqCst);
                tracing::info!("keybinds: switched off; hooks removed");
                None
            }
            HookStep::Install => match hook() {
                Ok(h) => {
                    // Fresh hooks: their tick starts now (before HOOKED, which the watchdog reads
                    // first), and keys released while there were none are forgotten.
                    // SAFETY: GetTickCount has no preconditions.
                    LAST_HOOK_TICK.store(unsafe { GetTickCount() }, Ordering::Relaxed);
                    matcher().forget_released(still_down);
                    set_broken(None);
                    HOOKED.store(true, Ordering::SeqCst);
                    tracing::info!("keybinds: hooks installed");
                    Some(h)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "keybinds: hooks couldn't be installed");
                    set_broken(Some(format!(
                        "Keybinds couldn't start ({e}); switch them off and on, or restart Pulse, to try again"
                    )));
                    None
                }
            },
            HookStep::Reinstall => reinstall(hooks),
        }
    }

    /// The watchdog's reinstall: unhook, then hook again until it works or the switch goes off.
    fn reinstall(hooks: Option<[HHOOK; 2]>) -> Option<[HHOOK; 2]> {
        if let Some(h) = hooks {
            unhook(h);
        }
        // LAST_HOOK_TICK is left alone: only a real event moving it tells the watchdog the
        // new hooks work.
        let mut failures = 0u32;
        let hooks = loop {
            match hook() {
                Ok(h) => break h,
                Err(e) => {
                    if failures == 0 {
                        tracing::warn!(error = %e, "keybinds: hooks couldn't be reinstalled; retrying");
                    } else {
                        tracing::debug!(error = %e, failures, "keybinds: hook reinstall failed again");
                    }
                    failures += 1;
                    // No hooks to service, so blocking this thread is harmless.
                    std::thread::sleep(REINSTALL_GAP);
                    if !ENABLED.load(Ordering::SeqCst) {
                        // switched off meanwhile: there are no hooks, and that's now right
                        HOOKED.store(false, Ordering::SeqCst);
                        tracing::info!("keybinds: switched off; hooks removed");
                        return None;
                    }
                }
            }
        };
        // Key-ups missed while the hooks were gone: drop modifiers Windows says are up (a
        // stale plain key is already treated as a fresh press, REPEAT_WINDOW).
        matcher().forget_released(still_down);
        tracing::info!("keybinds: hooks reinstalled");
        Some(hooks)
    }

    fn token_elevated(process: HANDLE) -> Option<bool> {
        let mut token = HANDLE::default();
        // SAFETY: process is a valid process handle; token is written on success.
        unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.ok()?;
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        // SAFETY: the buffer is a TOKEN_ELEVATION of the size passed.
        let got = unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut _),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            )
        };
        // SAFETY: token was opened above and isn't used again.
        let _ = unsafe { CloseHandle(token) };
        got.ok()?;
        Some(elevation.TokenIsElevated != 0)
    }

    /// Whether the hooks can see input right now. They can't while the secure desktop (UAC, lock
    /// screen) has the input, nor (UIPI) while a window more privileged than us is in front, yet
    /// the system's last-input tick still moves: a check then would read as a drop.
    fn hooks_can_see_input(self_elevated: bool) -> bool {
        // SAFETY: plain Win32 calls; every handle opened here is closed here.
        unsafe {
            match OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS) {
                Ok(desk) => {
                    let _ = CloseDesktop(desk);
                }
                Err(_) => return false,
            }
            if self_elevated {
                return true;
            }
            let window = GetForegroundWindow();
            if window.is_invalid() {
                return false;
            }
            let mut pid = 0u32;
            if GetWindowThreadProcessId(window, Some(&mut pid)) == 0 || pid == 0 {
                return false;
            }
            let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return false;
            };
            let elevated = token_elevated(process);
            let _ = CloseHandle(process);
            elevated == Some(false)
        }
    }

    /// Asks the hook thread to do something; a failure is logged (the watchdog tries again).
    fn post(thread: u32, message: u32) -> bool {
        // SAFETY: posting a thread message with no pointers in it.
        match unsafe { PostThreadMessageW(thread, message, WPARAM(0), LPARAM(0)) } {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(error = %e, message, "keybinds: couldn't reach the hook thread");
                false
            }
        }
    }

    /// Low-level hooks get no notice when Windows drops them, so compare what they saw with what
    /// the system saw and have the hook thread reinstall them when they've gone quiet.
    fn watch_hooks(hook_thread: u32) {
        // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing.
        let self_elevated = token_elevated(unsafe { GetCurrentProcess() }).unwrap_or(false);
        let mut watch = HookWatch::new(DEAD_MARGIN_MS, REINSTALL_GAP);
        loop {
            std::thread::sleep(WATCH_EVERY);
            let enabled = ENABLED.load(Ordering::SeqCst);
            let hooked = HOOKED.load(Ordering::SeqCst);
            let failed = BROKEN.lock().unwrap_or_else(|e| e.into_inner()).is_some();
            match watch_gate(enabled, hooked, failed) {
                Gate::Watch => {}
                Gate::Resend => {
                    // A lost (or late) WM_SWITCH: hooks and switch disagree. The next hooks get
                    // a fresh watch either way.
                    watch.reset();
                    tracing::debug!(
                        enabled,
                        hooked,
                        "keybinds: hooks don't match the switch; resending"
                    );
                    post(hook_thread, WM_SWITCH);
                    continue;
                }
                Gate::Idle => {
                    // switched off (or the install failed): nothing to watch, and no input
                    // polled; the next hooks get a fresh watch
                    watch.reset();
                    continue;
                }
            }
            if !hooks_can_see_input(self_elevated) {
                continue;
            }
            let mut info = LASTINPUTINFO {
                cbSize: size_of::<LASTINPUTINFO>() as u32,
                dwTime: 0,
            };
            // SAFETY: info is a valid LASTINPUTINFO with cbSize set.
            if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
                continue;
            }
            let hook_ms = LAST_HOOK_TICK.load(Ordering::Relaxed);
            let behind_ms = info.dwTime.wrapping_sub(hook_ms);
            match watch.check(info.dwTime, hook_ms, Instant::now()) {
                Check::Fine => continue,
                Check::GaveUp => {
                    tracing::debug!(
                        behind_ms,
                        "keybinds: hooks still see no input after reinstalling; not retrying until they do"
                    );
                    continue;
                }
                Check::Reinstall => tracing::warn!(
                    behind_ms,
                    "keybinds: hooks stopped seeing input (dropped by Windows?); reinstalling"
                ),
            }
            post(hook_thread, WM_REINSTALL);
        }
    }

    /// Returns the binds' problems (only "couldn't start", if the hooks failed). With `enabled`
    /// off the hooks aren't installed; the threads start idle so switching on later works.
    pub fn install(
        app: &tauri::App,
        binds: &Keybinds,
        enabled: bool,
    ) -> tauri::Result<Vec<String>> {
        matcher().set_binds(binds);
        ENABLED.store(enabled, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel::<Action>();
        let _ = FIRED.set(tx);
        let handle = app.handle().clone();
        std::thread::Builder::new()
            .name("pulse-keys-fire".into())
            .spawn(move || {
                for a in rx {
                    crate::commands::hotkey(&handle, a.command());
                }
            })?;
        let (ready_tx, ready) = mpsc::channel();
        std::thread::Builder::new()
            .name("pulse-keys".into())
            .spawn(move || run_hooks(ready_tx))?;
        match ready.recv() {
            Ok(hook_thread) => {
                let _ = HOOK_THREAD.set(hook_thread);
                if !enabled {
                    tracing::info!("keybinds: switched off; no hooks installed");
                }
                std::thread::Builder::new()
                    .name("pulse-keys-watch".into())
                    .spawn(move || watch_hooks(hook_thread))?;
            }
            Err(_) => set_broken(Some(
                "Keybinds couldn't start; restart Pulse to try again".into(),
            )),
        }
        Ok(problems())
    }

    /// Why binds aren't working, if they aren't. Switched off isn't a problem; switched off with
    /// the hooks still in is.
    pub fn problems() -> Vec<String> {
        let broken = BROKEN.lock().unwrap_or_else(|e| e.into_inner()).clone();
        switch_problem(
            ENABLED.load(Ordering::SeqCst),
            HOOKED.load(Ordering::SeqCst),
            broken.as_deref(),
        )
        .into_iter()
        .collect()
    }

    pub fn apply(_app: &AppHandle, binds: &Keybinds) -> Vec<String> {
        matcher().set_binds(binds);
        problems()
    }

    pub fn pause(_app: &AppHandle, paused: bool) -> Vec<String> {
        matcher().set_paused(paused);
        problems()
    }

    /// The master switch. The hook thread owns the hooks, so it's told to add or remove them;
    /// it reads ENABLED when it gets to the message, so a burst of toggles ends where the last
    /// one left the switch. Switching on retries a failed install, so the old failure is cleared
    /// (a new one shows the next time Settings asks). A post that fails is retried briefly here,
    /// and after that by the watchdog (`watch_gate`) until the hooks match the switch.
    pub fn set_enabled(_app: &AppHandle, enabled: bool) -> Vec<String> {
        if enabled {
            set_broken(None);
        }
        ENABLED.store(enabled, Ordering::SeqCst);
        let Some(&thread) = HOOK_THREAD.get() else {
            return problems();
        };
        for attempt in 0..3 {
            if post(thread, WM_SWITCH) {
                break;
            }
            if attempt < 2 {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        // a post that never got through shows here while off (hooks still in) and the watchdog
        // keeps resending
        problems()
    }

    #[cfg(test)]
    mod tests {
        use windows::Win32::System::Threading::{
            GetCurrentThread, GetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
        };

        #[test]
        fn the_hook_thread_runs_time_critical() {
            std::thread::spawn(|| {
                super::raise_priority().unwrap();
                // SAFETY: GetCurrentThread's pseudo-handle is always valid on its own thread.
                let p = unsafe { GetThreadPriority(GetCurrentThread()) };
                assert_eq!(p, THREAD_PRIORITY_TIME_CRITICAL.0);
            })
            .join()
            .unwrap();
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use crate::keybinds::Keybinds;

    pub fn install(
        _app: &tauri::App,
        _binds: &Keybinds,
        _enabled: bool,
    ) -> tauri::Result<Vec<String>> {
        Ok(Vec::new())
    }

    pub fn problems() -> Vec<String> {
        Vec::new()
    }

    /// Stored and shown, but nothing to switch: the compositor binds the CLI (ipc.rs).
    pub fn set_enabled(_app: &tauri::AppHandle, _enabled: bool) -> Vec<String> {
        Vec::new()
    }

    pub fn apply(_app: &tauri::AppHandle, _binds: &Keybinds) -> Vec<String> {
        Vec::new()
    }

    pub fn pause(_app: &tauri::AppHandle, _paused: bool) -> Vec<String> {
        Vec::new()
    }
}

pub use platform::{apply, install, pause, problems, set_enabled};
