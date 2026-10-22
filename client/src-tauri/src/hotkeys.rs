//! Global binds (Windows; on Linux the compositor binds the CLI, see ipc.rs). Low-level keyboard and
//! mouse hooks see every key and button system-wide and always pass it on: a bind toggles mute or
//! deafen and the key still works in the focused app, like Discord. Which press fires what is
//! decided by `bindmatch::Matcher`. Always on; paused while a bind is being recorded. Windows can
//! drop the hooks silently; a watchdog notices (`hookwatch`) and reinstalls them.

#[cfg(windows)]
mod platform {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{LazyLock, Mutex, OnceLock, mpsc};
    use std::time::{Duration, Instant};

    use tauri::AppHandle;
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, GetLastInputInfo, LASTINPUTINFO, MAPVK_VK_TO_VSC_EX, MapVirtualKeyW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, HHOOK, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, MSG, MSLLHOOKSTRUCT,
        PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
        WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP, WM_KEYDOWN, WM_KEYUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
        WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
    };

    use crate::bindmatch::{Matcher, key_code};
    use crate::hookwatch::HookWatch;
    use crate::keybinds::{Action, Keybinds};

    /// Locked by the hook procedures once per event and briefly by apply/pause; nothing else.
    static MATCHER: LazyLock<Mutex<Matcher>> = LazyLock::new(Default::default);
    /// Fired actions go to a forwarder thread: the hook procedure must return at once, or Windows
    /// silently drops the hook (LowLevelHooksTimeout).
    static FIRED: OnceLock<mpsc::Sender<Action>> = OnceLock::new();
    /// Set if the hooks couldn't be installed; reported in Settings until restart.
    static BROKEN: OnceLock<String> = OnceLock::new();
    /// `GetTickCount` when a hook last ran: the watchdog's evidence that they're still installed.
    static LAST_HOOK_TICK: AtomicU32 = AtomicU32::new(0);

    /// Posted to the hook thread to reinstall its hooks (hooks belong to the installing thread).
    const WM_REINSTALL: u32 = WM_APP + 1;
    /// How often the watchdog looks.
    const WATCH_EVERY: Duration = Duration::from_secs(3);
    /// System input this much newer than the hooks' last event means they're gone; less is the
    /// race between an event reaching the hook and the watchdog reading the system tick.
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

    /// Hooks are delivered through the installing thread's message loop, so they get their own
    /// thread that does nothing else (but reinstall them when the watchdog asks).
    fn run_hooks(ready: mpsc::Sender<Result<u32, String>>) {
        // Make sure this thread has a message queue before anyone posts to it.
        let mut msg = MSG::default();
        // SAFETY: msg is a valid MSG to write into.
        let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
        let mut hooks = match hook() {
            Ok(h) => h,
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        };
        // SAFETY: GetTickCount has no preconditions.
        LAST_HOOK_TICK.store(unsafe { GetTickCount() }, Ordering::Relaxed);
        // SAFETY: GetCurrentThreadId has no preconditions.
        let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
        while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
            if msg.message != WM_REINSTALL {
                continue;
            }
            for h in hooks {
                // Windows may already have removed it; nothing to do if so.
                // SAFETY: h came from SetWindowsHookExW on this thread.
                let _ = unsafe { UnhookWindowsHookEx(h) };
            }
            match hook() {
                Ok(h) => {
                    hooks = h;
                    // SAFETY: GetTickCount has no preconditions.
                    LAST_HOOK_TICK.store(unsafe { GetTickCount() }, Ordering::Relaxed);
                    // Key-ups missed while the hooks were gone: drop modifiers Windows says are
                    // up (a stale plain key is already treated as a fresh press, REPEAT_WINDOW).
                    matcher().forget_released(still_down);
                    tracing::info!("keybinds: hooks reinstalled");
                }
                // The watchdog asks again after REINSTALL_GAP.
                Err(e) => tracing::warn!(error = %e, "keybinds: hooks couldn't be reinstalled"),
            }
        }
    }

    /// Low-level hooks get no notice when Windows drops them, so compare what they saw with what
    /// the system saw and have the hook thread reinstall them when they've gone quiet.
    fn watch_hooks(hook_thread: u32) {
        let mut watch = HookWatch::new(DEAD_MARGIN_MS, REINSTALL_GAP);
        loop {
            std::thread::sleep(WATCH_EVERY);
            let mut info = LASTINPUTINFO {
                cbSize: size_of::<LASTINPUTINFO>() as u32,
                dwTime: 0,
            };
            // SAFETY: info is a valid LASTINPUTINFO with cbSize set.
            if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
                continue;
            }
            let hook_ms = LAST_HOOK_TICK.load(Ordering::Relaxed);
            if !watch.check(info.dwTime, hook_ms, Instant::now()) {
                continue;
            }
            tracing::warn!(
                behind_ms = info.dwTime.wrapping_sub(hook_ms),
                "keybinds: hooks stopped seeing input (dropped by Windows?); reinstalling"
            );
            // SAFETY: posting a thread message with no pointers in it.
            if let Err(e) =
                unsafe { PostThreadMessageW(hook_thread, WM_REINSTALL, WPARAM(0), LPARAM(0)) }
            {
                tracing::warn!(error = %e, "keybinds: couldn't ask for a hook reinstall");
            }
        }
    }

    /// Returns the binds' problems (only "couldn't start", if the hooks failed).
    pub fn install(app: &tauri::App, binds: &Keybinds) -> tauri::Result<Vec<String>> {
        matcher().set_binds(binds);
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
            Ok(Ok(hook_thread)) => {
                tracing::info!("keybinds: hooks installed");
                std::thread::Builder::new()
                    .name("pulse-keys-watch".into())
                    .spawn(move || watch_hooks(hook_thread))?;
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "keybinds: hooks couldn't be installed");
                let _ = BROKEN.set(format!(
                    "Keybinds couldn't start ({e}); restart Pulse to try again"
                ));
            }
            Err(_) => {
                let _ = BROKEN.set("Keybinds couldn't start; restart Pulse to try again".into());
            }
        }
        Ok(problems())
    }

    fn problems() -> Vec<String> {
        BROKEN.get().cloned().into_iter().collect()
    }

    pub fn apply(_app: &AppHandle, binds: &Keybinds) -> Vec<String> {
        matcher().set_binds(binds);
        problems()
    }

    pub fn pause(_app: &AppHandle, paused: bool) -> Vec<String> {
        matcher().set_paused(paused);
        problems()
    }
}

#[cfg(not(windows))]
mod platform {
    use crate::keybinds::Keybinds;

    pub fn install(_app: &tauri::App, _binds: &Keybinds) -> tauri::Result<Vec<String>> {
        Ok(Vec::new())
    }

    pub fn apply(_app: &tauri::AppHandle, _binds: &Keybinds) -> Vec<String> {
        Vec::new()
    }

    pub fn pause(_app: &tauri::AppHandle, _paused: bool) -> Vec<String> {
        Vec::new()
    }
}

pub use platform::{apply, install, pause};
